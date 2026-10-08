"""Fish-vision v2 trainer: 15 canonical, human-reviewed, wiki-documented fish.

WHY V2
======

fish_v1 trained on bare-stem labels (`fish:crimson`, `fish:snapper`, ...) that
the ontology investigation proved wrong: `fish:snapper` was really Crimson
Snapper, and there is no standalone "Angelfish". v2 trains on the HUMAN-
REVIEWED canonical labels via the rebuilt dataset (``reviewed_dataset.py``):

    15 classes, every one >= 20 eligible rows over >= 3 sessions
    labels = review record human_entity_id (never labels.jsonl.entity_id)
    splits = the same session-pure FNV-1a contract as v1 (70/15/15)
    Anglerfish is EXCLUDED by the class gate, automatically (12/20 rows)

WHAT IS REUSED (not a parallel architecture)
    FishNet (GpoVisionNet backbone + single head)      from fish_train
    FishDataset / augmentation / class_weights         from fish_train
    compute_mean_std / pad_to_square / set_seed        from train
    all metrics + temperature calibration              from metrics
    ONNX export + verify convention                    same as fish_train

The trainer emits the exact files the registry's candidate contract reads:
config.json (vocab + normalization), dataset_snapshot.json, training_log.jsonl,
checkpoints/, best.pt, evaluation_val.json, evaluation_test.json,
calibration.json, rejection.json, fish_vision_v1.onnx, onnx_check.json.

TEST is evaluated ONCE, after model selection on VALIDATION only.

Usage: python -m gpo_train.fish_train_v2 --epochs 40 --seed 7 --run-id fish-vision-v2-s7
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
import torch
import torch.nn.functional as F
from torch.utils.data import DataLoader

from .dataset import DatasetSnapshot, Row
from .fish_train import (
    FishDataset,
    FishNet,
    class_weights,
    evaluate_logits,
    pick_rejection_threshold,
)
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
from .train import compute_mean_std, set_seed

BASE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUTPUT_DIR = os.path.join(BASE_DIR, "output")
MANIFEST_DEFAULT = os.path.join(OUTPUT_DIR, "reviewed-fish-v1", "dataset_manifest.json")
INPUT_SIZE = 96
ONNX_NAME = "fish_vision_v1.onnx"  # the registry's fish-family artifact name


def _git_rev() -> str:
    try:
        return subprocess.run(
            ["git", "rev-parse", "HEAD"], capture_output=True, text=True, check=True
        ).stdout.strip()
    except Exception:
        return "unknown"


def _snapshot_from_manifest(manifest: dict, appdata: Path) -> DatasetSnapshot:
    """Adapt manifest rows to the dataset.Row shape FishDataset consumes."""
    images_dir = appdata / "datasets" / "gpo-vision" / "v1" / "images"
    rows = [
        Row(
            image_id=r["image_id"],
            session_id=r["session_id"],
            timestamp_ms=int(r.get("timestamp_ms", 0) or 0),
            game_state="catch_result",
            entity_id=r["entity_id"],
            hard_example=bool(r.get("hard_example", False)),
            image_path=str(images_dir / f"{r['image_id']}.png"),
        )
        for r in manifest["rows"]
    ]
    return DatasetSnapshot(version=int(manifest.get("dataset_version", 1)), rows=rows, overrides={})


def _weighted_f1(prf: list[dict]) -> float:
    total = sum(c["support"] for c in prf)
    return sum(c["f1"] * c["support"] for c in prf) / total if total else 0.0


def _macro_precision(prf: list[dict]) -> float:
    return sum(c["precision"] for c in prf) / len(prf) if prf else 0.0


def _macro_recall(prf: list[dict]) -> float:
    return sum(c["recall"] for c in prf) / len(prf) if prf else 0.0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--epochs", type=int, default=40)
    ap.add_argument("--batch", type=int, default=128)
    ap.add_argument("--lr", type=float, default=3e-4)
    ap.add_argument("--patience", type=int, default=10)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--run-id", default="fish-vision-v2-s7")
    ap.add_argument("--manifest", default=MANIFEST_DEFAULT)
    ap.add_argument("--appdata", type=Path,
                    default=Path(os.environ.get("APPDATA", "")) / "gpo-autofish")
    args = ap.parse_args()
    outdir = os.path.join(OUTPUT_DIR, args.run_id)
    os.makedirs(os.path.join(outdir, "checkpoints"), exist_ok=True)

    manifest = json.loads(Path(args.manifest).read_text(encoding="utf-8"))
    vocab: list[str] = manifest["class_map"]
    if len(vocab) != manifest["class_map_width"] or len(set(vocab)) != len(vocab):
        raise SystemExit("class map corrupt: width mismatch or duplicate classes")
    if len(vocab) != 15:
        raise SystemExit(f"expected the approved 15-class map, got {len(vocab)}")
    ent_index = {e: i for i, e in enumerate(vocab)}

    set_seed(args.seed)
    snap = _snapshot_from_manifest(manifest, args.appdata)
    idx_by_split = {
        s: [i for i, r in enumerate(manifest["rows"]) if r["split"] == s]
        for s in ("train", "validation", "test")
    }
    tr, va, te = idx_by_split["train"], idx_by_split["validation"], idx_by_split["test"]
    if not tr or not va or not te:
        raise SystemExit(f"empty split: train={len(tr)} val={len(va)} test={len(te)}")
    # The class gate is enforced at dataset build; belt here too.
    from collections import Counter
    ctr = Counter(snap.rows[i].entity_id for i in tr)
    th = manifest["thresholds"]
    for e in vocab:
        total = sum(1 for r in manifest["rows"] if r["entity_id"] == e)
        if total < th["entity_min_examples"]:
            raise SystemExit(f"{e} has {total} rows < gate {th['entity_min_examples']}")

    mean, std = compute_mean_std(snap, tr)
    train_ds = FishDataset(snap, tr, ent_index, mean, std, augment=True)
    val_ds = FishDataset(snap, va, ent_index, mean, std)
    test_ds = FishDataset(snap, te, ent_index, mean, std)
    train_loader = DataLoader(train_ds, batch_size=args.batch, shuffle=True, num_workers=0)
    val_loader = DataLoader(val_ds, batch_size=256, num_workers=0)
    test_loader = DataLoader(test_ds, batch_size=256, num_workers=0)

    device = "cpu"
    model = FishNet(len(vocab)).to(device)
    w = class_weights([train_ds[i][1] for i in range(len(train_ds))], len(vocab))
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=1e-4)
    sched = torch.optim.lr_scheduler.ReduceLROnPlateau(opt, mode="max", factor=0.5, patience=3)

    config = {
        "run_id": args.run_id, "seed": args.seed, "epochs": args.epochs,
        "scope": "15 canonical human-reviewed fish; Anglerfish gated out (12/20)",
        "vocab": vocab,
        "vocab_sha256": manifest["vocab_sha256"],
        "dataset_sha256": manifest["dataset_sha256"],
        "dataset_manifest": os.path.relpath(args.manifest, BASE_DIR),
        "created_from": manifest["created_from"],
        "blocked_classes": manifest["blocked_classes"],
        "source_commit": _git_rev(),
        "architecture": "GpoVisionNet backbone + single 15-class head",
        "input_size": INPUT_SIZE,
        "normalization": {"mean": list(mean), "std": list(std)},
        "loss": "class-weighted CE", "selection": "best validation macro-F1",
        "lr": args.lr, "weight_decay": 1e-4, "optimizer": "AdamW",
        "augmentation": "brightness/contrast + shift (train only)", "batch": args.batch,
        "early_stop_patience": args.patience,
        "framework": f"torch {torch.__version__}",
        "python": sys.version.split()[0],
        "onnxruntime_version": _ort_version(),
        "device": device,
    }
    with open(os.path.join(outdir, "config.json"), "w", encoding="utf-8") as f:
        json.dump(config, f, indent=2)
    with open(os.path.join(outdir, "dataset_snapshot.json"), "w", encoding="utf-8") as f:
        json.dump({
            "dataset_version": manifest["dataset_version"],
            "dataset_sha256": manifest["dataset_sha256"],
            "train": len(tr), "validation": len(va), "test": len(te),
            "train_support": dict(ctr),
            "entity_vocab": list(vocab),
            "val_support": {e: int(sum(1 for i in va if snap.rows[i].entity_id == e)) for e in vocab},
            "test_support": {e: int(sum(1 for i in te if snap.rows[i].entity_id == e)) for e in vocab},
            "train_sessions": len({snap.rows[i].session_id for i in tr}),
            "val_sessions": len({snap.rows[i].session_id for i in va}),
            "test_sessions": len({snap.rows[i].session_id for i in te}),
        }, f, indent=2)

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
            f1 = macro_f1(vt, vp, len(vocab))
            entry = {"epoch": epoch, "train_loss": tot / max(1, nb), "val_macro_f1": f1,
                     "lr": opt.param_groups[0]["lr"], "seconds": round(time.time() - t_start, 1)}
            logf.write(json.dumps(entry) + "\n")
            logf.flush()
            print(f"epoch {epoch:3d} train_loss={entry['train_loss']:.4f} val_F1={f1:.4f} "
                  f"lr={entry['lr']:.2e}", flush=True)
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
    duration_s = time.time() - t_start
    print(f"done: best epoch {best_epoch} val_macroF1 {best:.4f} in {duration_s:.0f}s", flush=True)

    # ---- frozen evaluation: VAL (calibration + threshold) then TEST once ----
    model.load_state_dict(torch.load(os.path.join(outdir, "best.pt"), map_location=device, weights_only=True)["model"])

    def full_eval(loader):
        yt, yl = evaluate_logits(model, loader, device)
        yp = [r.index(max(r)) for r in apply_temperature(yl, 1.0)]
        prf = per_class_prf(yt, yp, len(vocab))
        return yt, yl, yp, prf, {
            "n": len(yt),
            "accuracy": accuracy(yt, yp),
            "macro_precision": _macro_precision(prf),
            "macro_recall": _macro_recall(prf),
            "macro_f1": macro_f1(yt, yp, len(vocab)),
            "weighted_f1": _weighted_f1(prf),
            "balanced_accuracy": balanced_accuracy(yt, yp, len(vocab)),
            "per_entity": {vocab[i]: {**prf[i], "confusion_out": sum(confusion(yt, yp, len(vocab))[i]) - prf[i]["support"]} for i in range(len(vocab))},
            "confusion": confusion(yt, yp, len(vocab)),
        }

    yt_v, yl_v, _, _, val_eval = full_eval(val_loader)
    with open(os.path.join(outdir, "evaluation_val.json"), "w", encoding="utf-8") as f:
        json.dump(val_eval, f, indent=2)

    cal = fit_temperature(yl_v, yt_v)
    with open(os.path.join(outdir, "calibration.json"), "w", encoding="utf-8") as f:
        json.dump({**cal,
                   "brier": brier_score(apply_temperature(yl_v, cal["temperature"]), yt_v),
                   "ece": expected_calibration_error(apply_temperature(yl_v, cal["temperature"]), yt_v)}, f, indent=2)
    rej = pick_rejection_threshold(apply_temperature(yl_v, cal["temperature"]), yt_v)

    # TEST exactly once, with the frozen weights + frozen calibration.
    yt_t, yl_t, yp_t, _, test_eval = full_eval(test_loader)
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

    # OCR-empty independence slice: vision value where OCR had nothing to say.
    ocr_empty_hit, ocr_empty_n = 0, 0
    for i, y, p in zip(te, yt_t, yp_t):
        if not manifest["rows"][i].get("ocr_text_present", True):
            ocr_empty_n += 1
            ocr_empty_hit += int(y == p)
    test_eval["ocr_empty_accuracy"] = {"n": ocr_empty_n, "accuracy": ocr_empty_hit / max(1, ocr_empty_n)}
    test_eval["test_used_count"] = 1  # this run is the only time TEST was evaluated
    with open(os.path.join(outdir, "evaluation_test.json"), "w", encoding="utf-8") as f:
        json.dump(test_eval, f, indent=2)
    print(f"[val] F1={val_eval['macro_f1']:.4f} [test] acc={test_eval['accuracy']:.4f} "
          f"macroF1={test_eval['macro_f1']:.4f} wF1={test_eval['weighted_f1']:.4f} "
          f"balAcc={test_eval['balanced_accuracy']:.4f}", flush=True)

    # ---- ONNX export + verify ----
    model.eval()
    dummy = torch.randn(1, 3, INPUT_SIZE, INPUT_SIZE)
    onnx_path = os.path.join(outdir, ONNX_NAME)
    torch.onnx.export(model, dummy, onnx_path, input_names=["pixels"], output_names=["fish_logits"],
                      dynamic_axes={"pixels": {0: "batch"}, "fish_logits": {0: "batch"}}, opset_version=18)
    import onnx
    m = onnx.load(onnx_path)
    onnx.save(m, onnx_path)  # single-file
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

    # Record the true training duration in config now that it is known.
    config["training_duration_seconds"] = round(duration_s, 1)
    with open(os.path.join(outdir, "config.json"), "w", encoding="utf-8") as f:
        json.dump(config, f, indent=2)
    return 0 if ok else 1


def _ort_version() -> str:
    try:
        import onnxruntime  # noqa: PLC0415
        return onnxruntime.__version__
    except Exception:
        return "unknown"


if __name__ == "__main__":
    raise SystemExit(main())
