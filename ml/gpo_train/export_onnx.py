"""Export the best checkpoint to ONNX (opset 18, dynamic batch) and verify
numerical agreement with the PyTorch checkpoint via onnxruntime.

Usage: python -m gpo_train.export_onnx <run-id>
Writes: ml/output/<run-id>/gpo_vision_v1.onnx + onnx_check.json
"""

from __future__ import annotations

import argparse
import json
import os

import numpy as np
import torch

from .model import GpoVisionNet

BASE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUTPUT_DIR = os.path.join(BASE_DIR, "output")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("run_id")
    args = ap.parse_args()
    outdir = os.path.join(OUTPUT_DIR, args.run_id)
    with open(os.path.join(outdir, "config.json"), encoding="utf-8") as f:
        config = json.load(f)
    with open(os.path.join(outdir, "dataset_snapshot.json"), encoding="utf-8") as f:
        n_entities = len(json.load(f)["entity_vocab"])

    model = GpoVisionNet(n_entities=n_entities)
    ckpt = torch.load(os.path.join(outdir, "best.pt"), map_location="cpu", weights_only=True)
    model.load_state_dict(ckpt["model"])
    model.eval()
    dummy = torch.randn(1, 3, 96, 96)
    onnx_path = os.path.join(outdir, "gpo_vision_v1.onnx")
    torch.onnx.export(
        model, dummy, onnx_path,
        input_names=["pixels"], output_names=["state_logits", "entity_logits"],
        dynamic_axes={"pixels": {0: "batch"}, "state_logits": {0: "batch"}, "entity_logits": {0: "batch"}},
        opset_version=18,
    )

    import onnxruntime as ort

    torch.manual_seed(0)
    batches = {b: torch.randn(b, 3, 96, 96) for b in (1, 4)}
    sess = ort.InferenceSession(onnx_path, providers=["CPUExecutionProvider"])
    worst = 0.0
    with torch.no_grad():
        for b, x in batches.items():
            sl_t, el_t = model(x)
            sl_o, el_o = sess.run(None, {"pixels": x.numpy()})
            worst = max(
                worst,
                float(np.abs(sl_t.numpy() - sl_o).max()),
                float(np.abs(el_t.numpy() - el_o).max()),
            )
    ok = worst < 1e-4
    with open(os.path.join(outdir, "onnx_check.json"), "w", encoding="utf-8") as f:
        json.dump({"onnx_path": onnx_path, "max_abs_diff": worst, "tolerance": 1e-4, "pass": ok}, f, indent=2)
    print(f"max_abs_diff={worst:.2e} pass={ok}", flush=True)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
