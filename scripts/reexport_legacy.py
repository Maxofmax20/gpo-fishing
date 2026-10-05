"""Re-export both trained models with the legacy (non-dynamo) ONNX exporter.

Why: the dynamo exporter emits a Conv formulation whose analysis tract 0.22
rejects (ConvHir). The legacy exporter produces a tract-compatible graph
with identical numerics (verified below against torch, same tolerance).

Overwrites in place:
  ml/output/gpo-vision-v1-s7/gpo_vision_v1.onnx
  ml/output/fish-vision-v1-s7/fish_vision_v1.onnx
and refreshes each onnx_check.json (same schema, exporter noted).

Usage: python scripts/reexport_legacy.py  (mlvenv python)
"""

from __future__ import annotations

import json
import os
import sys

import numpy as np
import torch

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(REPO, "ml"))
from gpo_train.dataset import INPUT_SIZE  # noqa: E402
from gpo_train.fish_train import FISH_VOCAB, FishNet  # noqa: E402
from gpo_train.model import GpoVisionNet  # noqa: E402


def export_state():
    outdir = os.path.join(REPO, "ml", "output", "gpo-vision-v1-s7")
    with open(os.path.join(outdir, "dataset_snapshot.json")) as f:
        n_entities = len(json.load(f)["entity_vocab"])
    model = GpoVisionNet(n_entities=n_entities)
    ckpt = torch.load(os.path.join(outdir, "best.pt"), map_location="cpu", weights_only=True)
    model.load_state_dict(ckpt["model"])
    model.eval()
    return (model, ["pixels"], ["state_logits", "entity_logits"], outdir, "gpo_vision_v1.onnx")


def export_fish():
    outdir = os.path.join(REPO, "ml", "output", "fish-vision-v1-s7")
    model = FishNet(len(FISH_VOCAB))
    ckpt = torch.load(os.path.join(outdir, "best.pt"), map_location="cpu", weights_only=True)
    model.load_state_dict(ckpt["model"])
    model.eval()
    return (model, ["pixels"], ["fish_logits"], outdir, "fish_vision_v1.onnx")


def main() -> int:
    import onnx
    import onnxruntime as ort

    ok_all = True
    for build in (export_state, export_fish):
        model, innames, outnames, outdir, fname = build()
        onnx_path = os.path.join(outdir, fname)
        dummy = torch.randn(1, 3, INPUT_SIZE, INPUT_SIZE)
        torch.onnx.export(
            model, dummy, onnx_path, input_names=innames, output_names=outnames,
            dynamic_axes={innames[0]: {0: "batch"}, **{o: {0: "batch"} for o in outnames}},
            opset_version=18, dynamo=False,
        )
        m = onnx.load(onnx_path)
        onnx.save(m, onnx_path)
        sess = ort.InferenceSession(onnx_path, providers=["CPUExecutionProvider"])
        torch.manual_seed(0)
        worst = 0.0
        model.eval()
        with torch.no_grad():
            for b in (1, 4):
                x = torch.randn(b, 3, INPUT_SIZE, INPUT_SIZE)
                ref = model(x)
                ref = ref if isinstance(ref, tuple) else (ref,)
                got = sess.run(None, {innames[0]: x.numpy()})
                for r, g in zip(ref, got):
                    worst = max(worst, float(np.abs(r.numpy() - g).max()))
        ok = worst < 1e-4
        ok_all &= ok
        with open(os.path.join(outdir, "onnx_check.json"), "w", encoding="utf-8") as f:
            json.dump({"onnx_path": onnx_path, "max_abs_diff": worst, "tolerance": 1e-4,
                       "pass": ok, "single_file": True, "exporter": "legacy-dynamoFalse-opset18"}, f, indent=2)
        print(f"{fname}: max_abs_diff={worst:.2e} pass={ok}", flush=True)
    return 0 if ok_all else 1


if __name__ == "__main__":
    raise SystemExit(main())
