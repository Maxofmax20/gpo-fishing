"""Real training run for gpo-vision v1 (PyTorch CPU, deterministic).

Reads the production dataset read-only via dataset.py (session-pure
splits + eligibility). Trains the multitask CNN (model.py) with
class-weighted losses (documented training-time policy), early stopping
on validation STATE macro-F1, and checkpointing (best + every epoch).

Writes ml/output/<run-id>/: config.json, dataset_snapshot.json,
training_log.jsonl, checkpoints/epoch_N.pt, best.pt.
TEST split is never loaded here (see evaluate.py).

Usage: python -m gpo_train.train [--seed 7] [--epochs 40]
"""

from __future__ import annotations

import argparse
import datetime
import json
import os
import random
import sys
import time

import numpy as np
import torch
import torch.nn.functional as F
from PIL import Image, ImageEnhance
from torch.utils.data import DataLoader, Dataset

from .dataset import INPUT_SIZE, STATE_LABELS, SplitSet, build_splits, load_snapshot
from .metrics import macro_f1
from .model import GpoVisionNet

BASE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUTPUT_DIR = os.path.join(BASE_DIR, "output")


def set_seed(seed: int) -> None:
    random.seed(seed)
    np.random.seed(seed)
    torch.manual_seed(seed)
    torch.use_deterministic_algorithms(True)
    torch.set_num_threads(max(1, (os.cpu_count() or 8) - 2))


def pad_to_square(img: Image.Image) -> Image.Image:
    """Pad to square (black) BEFORE resize so crop aspect ratios (bar vs
    dialog) cannot leak class signal through resize distortion."""
    w, h = img.size
    side = max(w, h)
    canvas = Image.new("RGB", (side, side), (0, 0, 0))
    canvas.paste(img, ((side - w) // 2, (side - h) // 2))
    return canvas


class VisionDataset(Dataset):
    def __init__(self, snap, idxs: list[int], entity_vocab: list[str],
                 mean: tuple[float, float, float], std: tuple[float, float, float],
                 augment: bool, size: int = 96) -> None:
        self.snap = snap
        self.idxs = idxs
        self.ent_index = {e: i for i, e in enumerate(entity_vocab)}
        self.mean = mean
        self.std = std
        self.augment = augment
        self.size = size

    def __len__(self) -> int:
        return len(self.idxs)

    def __getitem__(self, k: int):
        r = self.snap.rows[self.idxs[k]]
        img = pad_to_square(Image.open(r.image_path).convert("RGB"))
        if self.augment:
            # Realistic capture variation only: brightness/contrast jitter +
            # tiny translate/scale. No flips/rotation (UI orientation matters).
            if torch.rand(1).item() < 0.8:
                f = 0.9 + 0.2 * torch.rand(1).item()
                img = ImageEnhance.Brightness(img).enhance(f)
            if torch.rand(1).item() < 0.8:
                f = 0.9 + 0.2 * torch.rand(1).item()
                img = ImageEnhance.Contrast(img).enhance(f)
            dx = int((torch.rand(1).item() - 0.5) * 8)
            dy = int((torch.rand(1).item() - 0.5) * 8)
            sc = 0.95 + 0.1 * torch.rand(1).item()
            w, h = img.size
            nw, nh = max(1, int(w * sc)), max(1, int(h * sc))
            img = img.resize((nw, nh), Image.BILINEAR)
            canvas = Image.new("RGB", (w, h), (0, 0, 0))
            canvas.paste(img, (dx, dy))
            img = canvas
        img = img.resize((self.size, self.size), Image.BILINEAR)
        t = torch.from_numpy(np.asarray(img, dtype=np.float32) / 255.0).permute(2, 0, 1)
        for c in range(3):
            t[c] = (t[c] - self.mean[c]) / self.std[c]
        state = STATE_LABELS.index(r.game_state)
        ent = self.ent_index.get(r.entity_id) if r.entity_id else None
        return t, state, (-1 if ent is None else ent)


def compute_mean_std(snap, idxs: list[int]) -> tuple[tuple[float, float, float], tuple[float, float, float]]:
    n = 0
    s = torch.zeros(3, dtype=torch.float64)
    s2 = torch.zeros(3, dtype=torch.float64)
    for i in idxs:
        img = pad_to_square(Image.open(snap.rows[i].image_path).convert("RGB")).resize(
            (INPUT_SIZE, INPUT_SIZE), Image.BILINEAR
        )
        t = torch.from_numpy(np.asarray(img, dtype=np.float64) / 255.0).permute(2, 0, 1)
        n += 1
        s += t.mean(dim=(1, 2))
        s2 += (t * t).mean(dim=(1, 2))
    mean = tuple((s / n).tolist())
    var = tuple(((s2 / n) - (s / n) ** 2).clamp_min(1e-6).tolist())
    import math

    std = tuple(math.sqrt(v) for v in var)
    return mean, std


def class_weights(labels: list[int], n_classes: int) -> torch.Tensor:
    counts = torch.zeros(n_classes)
    for y in labels:
        counts[y] += 1
    counts = counts.clamp_min(1)
    w = counts.sum() / (n_classes * counts)
    return w


def evaluate(model, loader, state_w, device: str) -> dict:
    model.eval()
    state_true, state_pred = [], []
    ent_true, ent_pred = [], []
    tot_s, tot_e = 0.0, 0.0
    n = 0
    with torch.no_grad():
        for x, ys, ye in loader:
            x = x.to(device)
            sl, el = model(x)
            tot_s += F.cross_entropy(sl, ys.to(device), weight=state_w.to(device),
                                     reduction="sum").item()
            m = ye >= 0
            if m.any():
                tot_e += F.cross_entropy(el[m], ye[m].to(device), reduction="sum").item()
            state_true += ys.tolist()
            state_pred += sl.argmax(1).cpu().tolist()
            for t, p, has in zip(ye.tolist(), el.argmax(1).cpu().tolist(), m.tolist()):
                if has:
                    ent_true.append(t)
                    ent_pred.append(p)
            n += len(x)
    return {
        "n": n,
        "state_loss": tot_s / max(1, n),
        "entity_loss": tot_e / max(1, len(ent_true)),
        "state_true": state_true,
        "state_pred": state_pred,
        "ent_true": ent_true,
        "ent_pred": ent_pred,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--epochs", type=int, default=40)
    ap.add_argument("--batch", type=int, default=128)
    ap.add_argument("--lr", type=float, default=3e-4)
    ap.add_argument("--patience", type=int, default=8)
    ap.add_argument("--run-id", type=str, default="")
    args = ap.parse_args()
    set_seed(args.seed)

    run_id = args.run_id or ("run-" + datetime.datetime.now().strftime("%Y%m%d-%H%M%S") + f"-s{args.seed}")
    outdir = os.path.join(OUTPUT_DIR, run_id)
    ckptdir = os.path.join(outdir, "checkpoints")
    os.makedirs(ckptdir, exist_ok=True)

    snap = load_snapshot()
    splits = build_splits(snap)
    assert splits.test, "TEST split must be non-empty"
    assert splits.validation, "validation split must be non-empty"
    assert splits.train, "train split must be non-empty"

    # Entity vocabulary: sorted project entity ids present in TRAIN ONLY
    # (test entities must not shape the vocabulary).
    vocab = sorted({snap.rows[i].entity_id for i in splits.train if snap.rows[i].entity_id})
    assert len(vocab) >= 2, "need entity vocabulary"

    mean, std = compute_mean_std(snap, splits.train)
    train_ds = VisionDataset(snap, splits.train, vocab, mean, std, augment=True)
    val_ds = VisionDataset(snap, splits.validation, vocab, mean, std, augment=False)
    g = torch.Generator().manual_seed(args.seed)
    train_loader = DataLoader(train_ds, batch_size=args.batch, shuffle=True, num_workers=0, generator=g)
    val_loader = DataLoader(val_ds, batch_size=256, shuffle=False, num_workers=0)

    state_labels = [STATE_LABELS.index(snap.rows[i].game_state) for i in splits.train]
    state_w = class_weights(state_labels, len(STATE_LABELS))
    ent_labels = [train_ds.ent_index[snap.rows[i].entity_id] for i in splits.train if snap.rows[i].entity_id]
    ent_w = class_weights(ent_labels, len(vocab))

    device = "cpu"
    model = GpoVisionNet(n_entities=len(vocab)).to(device)
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=1e-4)
    sched = torch.optim.lr_scheduler.ReduceLROnPlateau(opt, mode="max", factor=0.5, patience=3)

    config = {
        "run_id": run_id,
        "seed": args.seed,
        "epochs": args.epochs,
        "batch": args.batch,
        "lr": args.lr,
        "optimizer": "AdamW",
        "weight_decay": 1e-4,
        "scheduler": "ReduceLROnPlateau(max, factor=0.5, patience=3)",
        "patience": args.patience,
        "architecture": "GpoVisionNet custom CNN (3/32/64/128/256 + GAP + 2 heads)",
        "input_size": 96,
        "normalization": {"mean": list(mean), "std": list(std)},
        "loss": "class-weighted CE(state) + class-weighted CE(entity, masked)",
        "selection": "best validation STATE macro-F1; ENTITY macro-F1 breaks ties (harder head keeps improving after state plateau)",
        "augmentation": "brightness/contrast +-10%, translate +-4px, scale 0.95-1.05; no flip/rotation; val/test clean",
        "dataset_version": snap.version,
        "framework": f"torch {torch.__version__}",
        "python": sys.version.split()[0],
        "device": device,
    }
    with open(os.path.join(outdir, "config.json"), "w", encoding="utf-8") as f:
        json.dump(config, f, indent=2)
    snap_info = {
        "dataset_version": snap.version,
        "train": len(splits.train),
        "validation": len(splits.validation),
        "test": len(splits.test),
        "transition_eval": {
            "train": len(splits.transition_train),
            "validation": len(splits.transition_validation),
            "test": len(splits.transition_test),
        },
        "excluded": len(splits.excluded),
        "entity_vocab": vocab,
        "train_sessions": len({snap.rows[i].session_id for i in splits.train}),
        "val_sessions": len({snap.rows[i].session_id for i in splits.validation}),
        "test_sessions": len({snap.rows[i].session_id for i in splits.test}),
    }
    with open(os.path.join(outdir, "dataset_snapshot.json"), "w", encoding="utf-8") as f:
        json.dump(snap_info, f, indent=2)

    log_path = os.path.join(outdir, "training_log.jsonl")
    best_score = -1.0
    best_ent = -1.0
    best_epoch = -1
    stale = 0
    t_start = time.time()
    with open(log_path, "w", encoding="utf-8") as logf:
        for epoch in range(1, args.epochs + 1):
            model.train()
            tot_loss, nb = 0.0, 0
            for x, ys, ye in train_loader:
                x = x.to(device)
                ys = ys.to(device)
                opt.zero_grad()
                sl, el = model(x)
                loss = F.cross_entropy(sl, ys, weight=state_w.to(device))
                m = ye >= 0
                if m.any():
                    loss = loss + F.cross_entropy(el[m], ye[m].to(device), weight=ent_w.to(device))
                loss.backward()
                opt.step()
                tot_loss += loss.item() * len(x)
                nb += len(x)
            ev = evaluate(model, val_loader, state_w, device)
            s_f1 = macro_f1(ev["state_true"], ev["state_pred"], len(STATE_LABELS))
            e_f1 = macro_f1(ev["ent_true"], ev["ent_pred"], len(vocab)) if ev["ent_true"] else 0.0
            entry = {
                "epoch": epoch,
                "train_loss": tot_loss / max(1, nb),
                "val_state_loss": ev["state_loss"],
                "val_entity_loss": ev["entity_loss"],
                "val_state_macro_f1": s_f1,
                "val_entity_macro_f1": e_f1,
                "lr": opt.param_groups[0]["lr"],
                "seconds": round(time.time() - t_start, 1),
            }
            logf.write(json.dumps(entry) + "\n")
            logf.flush()
            print(
                f"epoch {epoch:3d} train_loss={entry['train_loss']:.4f} "
                f"val_state_loss={ev['state_loss']:.4f} val_state_F1={s_f1:.4f} "
                f"val_entity_F1={e_f1:.4f} lr={entry['lr']:.2e}",
                flush=True,
            )
            torch.save(
                {"epoch": epoch, "model": model.state_dict(), "opt": opt.state_dict(), "val": entry},
                os.path.join(ckptdir, f"epoch_{epoch:03d}.pt"),
            )
            sched.step(s_f1)
            improved = (s_f1 > best_score + 1e-6) or (
                abs(s_f1 - best_score) <= 1e-6 and e_f1 > best_ent + 1e-6
            )
            if improved:
                best_score = s_f1
                best_ent = e_f1
                best_epoch = epoch
                stale = 0
                torch.save(
                    {"epoch": epoch, "model": model.state_dict(), "val": entry},
                    os.path.join(outdir, "best.pt"),
                )
            else:
                stale += 1
                if stale >= args.patience:
                    print(f"early stopping at epoch {epoch} (best {best_epoch}, F1 {best_score:.4f})", flush=True)
                    break
    print(f"done: best epoch {best_epoch} val_state_macroF1 {best_score:.4f} in {time.time()-t_start:.0f}s", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
