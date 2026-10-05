"""Final evaluation of a trained checkpoint (validation and/or TEST).

Reads ml/output/<run-id>/ (config + dataset_snapshot + best.pt), rebuilds
the session-pure splits from the CURRENT production dataset, and reports
honest metrics. The TEST split is evaluated only when --split test is
passed explicitly, once, after model selection is frozen.

Writes evaluation_<split>.json (+ calibration.json when fitting on val).

Usage: python -m gpo_train.evaluate <run-id> --split val|test
"""

from __future__ import annotations

import argparse
import json
import os

import numpy as np
import torch
from PIL import Image
from torch.utils.data import DataLoader, Dataset

from .dataset import INPUT_SIZE, STATE_LABELS, build_splits, load_snapshot
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
from .model import GpoVisionNet

BASE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUTPUT_DIR = os.path.join(BASE_DIR, "output")


class EvalDataset(Dataset):
    def __init__(self, snap, idxs, entity_vocab, mean, std):
        self.snap = snap
        self.idxs = idxs
        self.ent_index = {e: i for i, e in enumerate(entity_vocab)}
        self.mean = mean
        self.std = std

    def __len__(self):
        return len(self.idxs)

    def __getitem__(self, k):
        r = self.snap.rows[self.idxs[k]]
        img = Image.open(r.image_path).convert("RGB")
        side = max(img.size)
        canvas = Image.new("RGB", (side, side), (0, 0, 0))
        canvas.paste(img, ((side - img.size[0]) // 2, (side - img.size[1]) // 2))
        img = canvas.resize((INPUT_SIZE, INPUT_SIZE), Image.BILINEAR)
        t = torch.from_numpy(np.asarray(img, dtype=np.float32) / 255.0).permute(2, 0, 1)
        for c in range(3):
            t[c] = (t[c] - self.mean[c]) / self.std[c]
        state = STATE_LABELS.index(r.game_state)
        ent = self.ent_index.get(r.entity_id) if r.entity_id else None
        return t, state, (-1 if ent is None else ent)


def run_split(model, loader, device):
    model.eval()
    out = {"state_true": [], "state_pred": [], "ent_true": [], "ent_pred": [],
           "state_logits": [], "ent_logits": []}
    with torch.no_grad():
        for x, ys, ye in loader:
            sl, el = model(x.to(device))
            out["state_true"] += ys.tolist()
            out["state_pred"] += sl.argmax(1).cpu().tolist()
            out["state_logits"] += sl.cpu().tolist()
            m = ye >= 0
            for t, p, has in zip(ye.tolist(), el.argmax(1).cpu().tolist(), m.tolist()):
                if has:
                    out["ent_true"].append(t)
                    out["ent_pred"].append(p)
            out["ent_logits"] += [row for row, has in zip(el.cpu().tolist(), m.tolist()) if has]
    return out


def state_report(true, pred):
    prf = per_class_prf(true, pred, len(STATE_LABELS))
    return {
        "n": len(true),
        "accuracy": accuracy(true, pred),
        "macro_f1": macro_f1(true, pred, len(STATE_LABELS)),
        "balanced_accuracy": balanced_accuracy(true, pred, len(STATE_LABELS)),
        "per_class": {name: prf[i] for i, name in enumerate(STATE_LABELS)},
        "confusion": confusion(true, pred, len(STATE_LABELS)),
    }


def entity_report(true, pred, vocab):
    prf = per_class_prf(true, pred, len(vocab))
    return {
        "n": len(true),
        "accuracy": accuracy(true, pred),
        "macro_f1": macro_f1(true, pred, len(vocab)),
        "balanced_accuracy": balanced_accuracy(true, pred, len(vocab)),
        "per_entity": {vocab[i]: prf[i] for i in range(len(vocab))},
        "confusion": confusion(true, pred, len(vocab)),
    }


def majority_baseline(true, n_classes):
    from collections import Counter

    if not true:
        return {"accuracy": 0.0}
    maj = Counter(true).most_common(1)[0][0]
    pred = [maj] * len(true)
    return {"class": maj, "accuracy": accuracy(true, pred)}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("run_id")
    ap.add_argument("--split", choices=["val", "test"], default="val")
    args = ap.parse_args()
    outdir = os.path.join(OUTPUT_DIR, args.run_id)
    with open(os.path.join(outdir, "config.json"), encoding="utf-8") as f:
        config = json.load(f)
    with open(os.path.join(outdir, "dataset_snapshot.json"), encoding="utf-8") as f:
        snap_info = json.load(f)
    vocab = snap_info["entity_vocab"]
    mean = tuple(config["normalization"]["mean"])
    std = tuple(config["normalization"]["std"])

    torch.manual_seed(config.get("seed", 7))
    snap = load_snapshot()
    assert snap.version == snap_info.get("dataset_version", snap.version), "dataset version drift"
    splits = build_splits(snap)
    idxs = splits.validation if args.split == "val" else splits.test
    trans = (splits.transition_validation if args.split == "val" else splits.transition_test)

    device = "cpu"
    model = GpoVisionNet(n_entities=len(vocab))
    ckpt = torch.load(os.path.join(outdir, "best.pt"), map_location=device, weights_only=True)
    model.load_state_dict(ckpt["model"])
    loader = DataLoader(EvalDataset(snap, idxs, vocab, mean, std), batch_size=256, num_workers=0)
    res = run_split(model, loader, device)

    report: dict = {
        "split": args.split,
        "n": len(idxs),
        "state": state_report(res["state_true"], res["state_pred"]),
        "entity": entity_report(res["ent_true"], res["ent_pred"], vocab),
        "baseline_majority_state": majority_baseline(res["state_true"], len(STATE_LABELS)),
        "baseline_majority_entity": majority_baseline(res["ent_true"], len(vocab)),
    }
    # Calibration (probabilities): fit temperature on VAL only.
    if args.split == "val":
        cal = fit_temperature(res["state_logits"], res["state_true"])
        cal_probs = apply_temperature(res["state_logits"], cal["temperature"])
        raw_probs = apply_temperature(res["state_logits"], 1.0)
        report["calibration"] = {
            "temperature": cal["temperature"],
            "log_loss_before": log_loss(raw_probs, res["state_true"]),
            "log_loss_after": cal["log_loss"],
            "brier_before": brier_score(raw_probs, res["state_true"]),
            "brier_after": brier_score(cal_probs, res["state_true"]),
            "ece_before": expected_calibration_error(raw_probs, res["state_true"]),
            "ece_after": expected_calibration_error(cal_probs, res["state_true"]),
        }
        with open(os.path.join(outdir, "calibration.json"), "w", encoding="utf-8") as f:
            json.dump(report["calibration"], f, indent=2)
    else:
        cal_path = os.path.join(outdir, "calibration.json")
        if os.path.isfile(cal_path):
            with open(cal_path, encoding="utf-8") as f:
                cal = json.load(f)
            cal_probs = apply_temperature(res["state_logits"], cal["temperature"])
            report["calibration_applied"] = {
                "temperature": cal["temperature"],
                "log_loss": log_loss(cal_probs, res["state_true"]),
                "brier": brier_score(cal_probs, res["state_true"]),
                "ece": expected_calibration_error(cal_probs, res["state_true"]),
            }

    # Transition-adjacent diagnostic (same split only — never mixed).
    if trans:
        tloader = DataLoader(EvalDataset(snap, trans, vocab, mean, std), batch_size=256, num_workers=0)
        tres = run_split(model, tloader, device)
        report["transition"] = {
            "n": len(trans),
            "state_accuracy": accuracy(tres["state_true"], tres["state_pred"]),
            "note": "boundary frames excluded from training; reported, not selected on",
        }
    # Hard-example diagnostic (RESULT rows flagged hard in this split).
    hard_idx = [i for i in idxs if snap.rows[i].hard_example]
    if hard_idx:
        hloader = DataLoader(EvalDataset(snap, hard_idx, vocab, mean, std), batch_size=256, num_workers=0)
        hres = run_split(model, hloader, device)
        report["hard"] = {
            "n": len(hard_idx),
            "state_accuracy": accuracy(hres["state_true"], hres["state_pred"]),
        }
    # Rare-entity diagnostic: entities with <10 TRAIN rows.
    train_counts: dict[str, int] = {}
    for i in splits.train:
        e = snap.rows[i].entity_id
        if e:
            train_counts[e] = train_counts.get(e, 0) + 1
    rare = {e for e, c in train_counts.items() if c < 10}
    rare_idx = [i for i in idxs if (snap.rows[i].entity_id or "") in rare]
    if rare_idx:
        rloader = DataLoader(EvalDataset(snap, rare_idx, vocab, mean, std), batch_size=256, num_workers=0)
        rres = run_split(model, rloader, device)
        report["rare_entities"] = {
            "entities": sorted(rare),
            "n": len(rare_idx),
            "entity_accuracy": accuracy(rres["ent_true"], rres["ent_pred"]),
        }

    with open(os.path.join(outdir, f"evaluation_{args.split}.json"), "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2)
    s = report["state"]
    e = report["entity"]
    print(f"[{args.split}] n={report['n']} state_acc={s['accuracy']:.4f} state_macroF1={s['macro_f1']:.4f} "
          f"entity_acc={e['accuracy']:.4f} entity_macroF1={e['macro_f1']:.4f}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
