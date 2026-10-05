"""Scoped fish-vision trainer: 8 well-supported fish, session-isolated.

Scope justification (v5.4.0 gate review): exactly the 8 fish entities with
>=20 real examples AND held-out TEST coverage are trained. Nothing else is
claimed. All other 33 KB fish + all fruits resolve to UNKNOWN at runtime
until their own gates pass. No synthetic data, no Wiki-as-pixels.

Pipeline (mirrors gpo-vision-v1-s7 conventions):
  session-pure FNV-1a splits (shared dataset.py) -> pad-to-square + 96px +
  train-only aug -> class-weighted CE -> val macro-F1 selection (entity
  tie-break not needed: single head) -> ONE test evaluation -> temperature
  calibration (val) -> rejection threshold (val) -> ONNX export + verify.

Writes ml/output/<run-id>/: config, dataset_snapshot, training_log.jsonl,
checkpoints, best.pt, evaluation_val/test.json, calibration.json,
rejection.json, fish_vision_v1.onnx, onnx_check.json.

Usage: python -m gpo_train.fish_train --epochs 40 --seed 7 --run-id fish-vision-v1-s7
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time

import numpy as np
import torch
import torch.nn.functional as F
from PIL import Image
from torch.utils.data import DataLoader, Dataset

from .dataset import INPUT_SIZE, build_splits, load_snapshot
from .metrics import (
    accuracy,
    apply_temperature,
    balanced_accuracy,
    brier_score,
    confusion,
    expected_calibration_error,
    fit_temperature,
    log_loss,
    macro_f1,
    per_class_prf,
)
from .model import GpoVisionNet  # backbone reference only; head below is fish-only
from .train import compute_mean_std, pad_to_square  # identical preprocessing/aug

BASE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUTPUT_DIR = os.path.join(BASE_DIR, "output")

FISH_VOCAB = [
    "fish:angelfish", "fish:crimson", "fish:golden", "fish:pufferfish",
    "fish:shark", "fish:snapper", "fish:squid", "fish:swordfish",
]
MIN_EXAMPLES = 20


class FishNet(torch.nn.Module):
    """Same backbone as GpoVisionNet (3/32/64/64? no: 32/64/128/256 + GAP),
    single 8-class fish head. Independent weights, independent provenance."""

    def __init__(self, n_classes: int):
        super().__init__()
        m = GpoVisionNet(n_entities=n_classes)
        self.backbone = m.backbone
        self.drop = torch.nn.Dropout(0.25)
        self.head = torch.nn.Linear(256, n_classes)

    def forward(self, x):
        z = self.drop(self.backbone(x).flatten(1))
        return self.head(z)


class FishDataset(Dataset):
    def __init__(self, snap, idxs, ent_index, mean, std, augment=False):
        self.snap = snap
        self.idxs = idxs
        self.ent_index = ent_index
        self.mean = mean
        self.std = std
        self.augment = augment
        self.rng = np.random.default_rng(7)

    def __len__(self):
        return len(self.idxs)

    def _augment(self, img: Image.Image) -> Image.Image:
        arr = np.asarray(img).astype(np.float32)
        b = 1.0 + float(self.rng.uniform(-0.10, 0.10))
        c = 1.0 + float(self.rng.uniform(-0.10, 0.10))
        arr = ((arr - 127.5) * c + 127.5) * b
        arr = np.clip(arr, 0, 255).astype(np.uint8)
        img = Image.fromarray(arr)
        w, h = img.size
        dx, dy = int(self.rng.integers(-4, 5)), int(self.rng.integers(-4, 5))
        canvas = Image.new("RGB", (w, h), (0, 0, 0))
        canvas.paste(img, (dx, dy))
        return canvas

    def __getitem__(self, k):
        r = self.snap.rows[self.idxs[k]]
        img = pad_to_square(Image.open(r.image_path).convert("RGB"))
        if self.augment:
            img = self._augment(img)
        img = img.resize((INPUT_SIZE, INPUT_SIZE), Image.BILINEAR)
        t = torch.from_numpy(np.asarray(img, dtype=np.float32) / 255.0).permute(2, 0, 1)
        for ch in range(3):
            t[ch] = (t[ch] - self.mean[ch]) / self.std[ch]
        return t, self.ent_index[r.entity_id]


def class_weights(labels, n):
    from collections import Counter

    c = Counter(labels)
    total = max(1, len(labels))
    return torch.tensor([total / max(1, c.get(i, 0)) / n for i in range(n)], dtype=torch.float32)


def evaluate_logits(model, loader, device):
    model.eval()
    true, logits = [], []
    with torch.no_grad():
        for x, y in loader:
            out = model(x.to(device))
            true += y.tolist()
            logits += out.cpu().tolist()
    return true, logits


def pick_rejection_threshold(val_logits, val_true):
    """Lowest max-prob threshold with kept-accuracy >= 0.90 on VAL.
    Documented, single-parameter, fit on validation only."""
    probs = apply_temperature(val_logits, 1.0)
    best = {"threshold": 0.0, "kept": 1.0, "kept_accuracy": accuracy(val_true, [p.index(max(p)) for p in probs])}
    t = 0.05
    while t <= 0.95:
        kept_t, kept_p = [], []
        for p, y in zip(probs, val_true):
            if max(p) >= t:
                kept_t.append(y)
                kept_p.append(p.index(max(p)))
        if kept_t:
            acc = accuracy(kept_t, kept_p)
            if acc >= 0.90:
                best = {"threshold": round(t, 2), "kept": len(kept_t) / len(val_true), "kept_accuracy": acc}
                break
        t += 0.05
    return best


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--epochs", type=int, default=40)
    ap.add_argument("--batch", type=int, default=128)
    ap.add_argument("--lr", type=float, default=3e-4)
    ap.add_argument("--patience", type=int, default=10)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--run-id", default="fish-vision-v1-s7")
    args = ap.parse_args()
    outdir = os.path.join(OUTPUT_DIR, args.run_id)
    os.makedirs(os.path.join(outdir, "checkpoints"), exist_ok=True)

    torch.manual_seed(args.seed)
    np.random.seed(args.seed)
    snap = load_snapshot()
    splits = build_splits(snap)
    ent_index = {e: i for i, e in enumerate(FISH_VOCAB)}

    def fish_only(idxs):
        return [i for i in idxs
                if snap.rows[i].game_state == "catch_result" and snap.rows[i].entity_id in ent_index]

    tr, va, te = fish_only(splits.train), fish_only(splits.validation), fish_only(splits.test)
    from collections import Counter
    ctr = Counter(snap.rows[i].entity_id for i in tr)
    assert all(ctr.get(e, 0) >= MIN_EXAMPLES for e in FISH_VOCAB), f"scope violated: {dict(ctr)}"
    assert va and te, "need held-out val and test fish rows"

    mean, std = compute_mean_std(snap, tr)
    train_ds = FishDataset(snap, tr, ent_index, mean, std, augment=True)
    val_ds = FishDataset(snap, va, ent_index, mean, std)
    test_ds = FishDataset(snap, te, ent_index, mean, std)
    train_loader = DataLoader(train_ds, batch_size=args.batch, shuffle=True, num_workers=0)
    val_loader = DataLoader(val_ds, batch_size=256, num_workers=0)
    test_loader = DataLoader(test_ds, batch_size=256, num_workers=0)

    device = "cpu"
    model = FishNet(len(FISH_VOCAB)).to(device)
    w = class_weights([train_ds[i][1] for i in range(len(train_ds))], len(FISH_VOCAB))
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=1e-4)
    sched = torch.optim.lr_scheduler.ReduceLROnPlateau(opt, mode="max", factor=0.5, patience=3)

    config = {
        "run_id": args.run_id, "seed": args.seed, "epochs": args.epochs,
        "scope": "8 fish with >=20 examples and TEST coverage; all else UNKNOWN",
        "vocab": FISH_VOCAB, "architecture": "GpoVisionNet backbone + single 8-class head",
        "input_size": INPUT_SIZE, "normalization": {"mean": list(mean), "std": list(std)},
        "loss": "class-weighted CE", "selection": "best validation macro-F1",
        "framework": f"torch {torch.__version__}", "device": device,
    }
    with open(os.path.join(outdir, "config.json"), "w", encoding="utf-8") as f:
        json.dump(config, f, indent=2)
    with open(os.path.join(outdir, "dataset_snapshot.json"), "w", encoding="utf-8") as f:
        json.dump({"dataset_version": snap.version, "train": len(tr), "validation": len(va),
                   "test": len(te), "train_support": dict(ctr),
                   "train_sessions": len({snap.rows[i].session_id for i in tr}),
                   "val_sessions": len({snap.rows[i].session_id for i in va}),
                   "test_sessions": len({snap.rows[i].session_id for i in te})}, f, indent=2)

    log_path = os.path.join(outdir, "training_log.jsonl")
    best, best_epoch, stale = -1.0, -1, 0
    t_start = time.time()
    with open(log_path, "w", encoding="utf-8") as logf:
        for epoch in range(1, args.epochs + 1):
            model.train()
            tot, nb = 0.0, 0
            for x, y in train_loader:
                x, y = x.to(device), y.to(device)
                opt.zero_grad()
                loss = F.cross_entropy(model(x), y, weight=w.to(device))
                loss.backward()
                opt.step()
                tot += loss.item() * len(x)
                nb += len(x)
            vt, vl = evaluate_logits(model, val_loader, device)
            vp = [r.index(max(r)) for r in apply_temperature(vl, 1.0)]
            f1 = macro_f1(vt, vp, len(FISH_VOCAB))
            entry = {"epoch": epoch, "train_loss": tot / max(1, nb), "val_macro_f1": f1,
                     "lr": opt.param_groups[0]["lr"], "seconds": round(time.time() - t_start, 1)}
            logf.write(json.dumps(entry) + "\n")
            logf.flush()
            print(f"epoch {epoch:3d} train_loss={entry['train_loss']:.4f} val_F1={f1:.4f} lr={entry['lr']:.2e}", flush=True)
            torch.save({"epoch": epoch, "model": model.state_dict()},
                       os.path.join(outdir, "checkpoints", f"epoch_{epoch:03d}.pt"))
            sched.step(f1)
            if f1 > best + 1e-6:
                best, best_epoch, stale = f1, epoch, 0
                torch.save({"epoch": epoch, "model": model.state_dict(), "val": entry},
                           os.path.join(outdir, "best.pt"))
            else:
                stale += 1
                if stale >= args.patience:
                    print(f"early stopping at epoch {epoch} (best {best_epoch}, F1 {best:.4f})", flush=True)
                    break
    print(f"done: best epoch {best_epoch} val_macroF1 {best:.4f} in {time.time()-t_start:.0f}s", flush=True)

    # ---- frozen evaluation: VAL (calibration + threshold) then TEST once ----
    model.load_state_dict(torch.load(os.path.join(outdir, "best.pt"), map_location=device, weights_only=True)["model"])
    results = {}
    for split, loader in (("val", val_loader), ("test", test_loader)):
        yt, yl = evaluate_logits(model, loader, device)
        yp = [r.index(max(r)) for r in apply_temperature(yl, 1.0)]
        prf = per_class_prf(yt, yp, len(FISH_VOCAB))
        results[split] = {
            "n": len(yt), "accuracy": accuracy(yt, yp), "macro_f1": macro_f1(yt, yp, len(FISH_VOCAB)),
            "balanced_accuracy": balanced_accuracy(yt, yp, len(FISH_VOCAB)),
            "per_entity": {FISH_VOCAB[i]: prf[i] for i in range(len(FISH_VOCAB))},
            "confusion": confusion(yt, yp, len(FISH_VOCAB)),
        }
        with open(os.path.join(outdir, f"evaluation_{split}.json"), "w", encoding="utf-8") as f:
            json.dump(results[split], f, indent=2)
    yt_v, yl_v = evaluate_logits(model, val_loader, device)
    cal = fit_temperature(yl_v, yt_v)
    with open(os.path.join(outdir, "calibration.json"), "w", encoding="utf-8") as f:
        json.dump({**cal,
                   "brier": brier_score(apply_temperature(yl_v, cal["temperature"]), yt_v),
                   "ece": expected_calibration_error(apply_temperature(yl_v, cal["temperature"]), yt_v)}, f, indent=2)
    rej = pick_rejection_threshold(apply_temperature(yl_v, cal["temperature"]), yt_v)
    # apply frozen threshold to TEST once
    yt_t, yl_t = evaluate_logits(model, test_loader, device)
    probs_t = apply_temperature(yl_t, cal["temperature"])
    kt, kp = [], []
    for p, y in zip(probs_t, yt_t):
        if max(p) >= rej["threshold"]:
            kt.append(y)
            kp.append(p.index(max(p)))
    rej["test_kept"] = len(kt) / max(1, len(yt_t))
    rej["test_kept_accuracy"] = accuracy(kt, kp) if kt else 0.0
    with open(os.path.join(outdir, "rejection.json"), "w", encoding="utf-8") as f:
        json.dump(rej, f, indent=2)

    # ---- OCR-empty independence slice (vision value where OCR is blind) ----
    # Frozen-snapshot aware: same root the snapshot was loaded from.
    from .dataset import dataset_root as _dataset_root
    ocr_empty_hit, ocr_empty_n, ver_hit, ver_n = 0, 0, 0, 0
    with open(os.path.join(_dataset_root(), "labels.jsonl"), encoding="utf-8") as f:
        raw = [json.loads(l) for l in f if l.strip()]
    assert len(raw) == len(snap.rows)
    for i, y, p in zip(te, yt_t, [r.index(max(r)) for r in probs_t]):
        o = (raw[i].get("ocr_text") or "").strip()
        if not o:
            ocr_empty_n += 1
            ocr_empty_hit += (y == p)
        if o and not snap.rows[i].hard_example:
            ver_n += 1
            ver_hit += (y == p)
    results["test"]["ocr_empty_accuracy"] = {"n": ocr_empty_n, "accuracy": ocr_empty_hit / max(1, ocr_empty_n)}
    results["test"]["verified_accuracy"] = {"n": ver_n, "accuracy": ver_hit / max(1, ver_n)}
    with open(os.path.join(outdir, "evaluation_test.json"), "w", encoding="utf-8") as f:
        json.dump(results["test"], f, indent=2)
    print(f"[val] F1={results['val']['macro_f1']:.4f} [test] acc={results['test']['macro_f1']:.4f} "
          f"rej_thr={rej['threshold']} kept={rej['test_kept']:.2f}@{rej['test_kept_accuracy']:.3f} "
          f"ocr_empty={results['test']['ocr_empty_accuracy']} verified={results['test']['verified_accuracy']}", flush=True)

    # ---- ONNX export + verify ----
    model.eval()
    dummy = torch.randn(1, 3, INPUT_SIZE, INPUT_SIZE)
    onnx_path = os.path.join(outdir, "fish_vision_v1.onnx")
    torch.onnx.export(model, dummy, onnx_path, input_names=["pixels"], output_names=["fish_logits"],
                      dynamic_axes={"pixels": {0: "batch"}, "fish_logits": {0: "batch"}}, opset_version=18)
    import onnx
    m = onnx.load(onnx_path)
    onnx.save(m, onnx_path)  # single-file (inline any external data)
    import onnxruntime as ort
    sess = ort.InferenceSession(onnx_path, providers=["CPUExecutionProvider"])
    torch.manual_seed(0)
    worst = 0.0
    with torch.no_grad():
        for b in (1, 4):
            x = torch.randn(b, 3, INPUT_SIZE, INPUT_SIZE)
            o = sess.run(None, {"pixels": x.numpy()})[0]
            worst = max(worst, float(np.abs(model(x).numpy() - o).max()))
    ok = worst < 1e-4
    with open(os.path.join(outdir, "onnx_check.json"), "w", encoding="utf-8") as f:
        json.dump({"max_abs_diff": worst, "tolerance": 1e-4, "pass": ok, "single_file": True}, f, indent=2)
    print(f"onnx max_abs_diff={worst:.2e} pass={ok}", flush=True)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
