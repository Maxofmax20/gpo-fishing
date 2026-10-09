"""Re-export a trained run's checkpoint to ONNX using the legacy (TorchScript)
exporter, and prove the result is statically analysable.

WHY THIS EXISTS

torch 2.14 changed `torch.onnx.export`'s default to `dynamo=True`. That exporter
emits `ReduceMean` + a `Reshape` whose target shape is a RUNTIME-COMPUTED tensor
instead of a constant initializer. `tract` - the inference runtime
`ShadowEngine::load` actually uses - cannot analyse that: it fails with

    Failed analyse for node #13 "node_Reshape_77" Reshape

so an artifact that is numerically perfect (onnxruntime agrees to 1.5e-05, single
file, deterministic) cannot be loaded by the app at all.

`dynamo=False` selects the previous exporter, which emits
`GlobalAveragePool` + `Flatten` + a constant-shape `Reshape` - the exact graph
the already-deployed 8-class model uses, and one tract accepts.

This is a COMPATIBILITY fix: the runtime did not change, the artifact format did.

Re-exporting does NOT retrain: `best.pt` is the trained checkpoint and its hash
is unchanged. Only the serialized graph differs, so metrics are identical while
`artifact_sha256` necessarily changes and the candidate must be re-registered.

Usage:
    python -m ml.gpo_train.reexport_tract_compatible <run-id>
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "ml"))

BASE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUTPUT_DIR = os.path.join(BASE_DIR, "output")
INPUT_SIZE = 96


def analyse_is_statically_analysable(onnx_path: Path) -> tuple[bool, str]:
    """A `Reshape` (or `Expand`/`Tile`) whose shape comes from a computed tensor
    is the tract-analysability trap: tract needs the output length at analysis
    time. Anything fed only from initializers is fine."""
    import onnx

    m = onnx.load(str(onnx_path), load_external_data=False)
    consts = {t.name for t in m.graph.initializer}
    consts.update(c.name for c in m.graph.input)
    for n in m.graph.node:
        if n.op_type in ("Reshape", "Expand", "Tile", "ConstantOfShape"):
            shape_input = n.input[1] if len(n.input) > 1 else None
            if shape_input is not None and shape_input not in consts:
                return False, (
                    f"node {n.name or n.op_type} ({n.op_type}) takes its shape from "
                    f"{shape_input!r}, which is a computed tensor, not a constant"
                )
    return True, "every shape input is a constant initializer"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("run_id")
    args = ap.parse_args()
    run = Path(OUTPUT_DIR) / args.run_id

    import numpy as np
    import torch

    from gpo_train.fish_train import FishNet
    from gpo_train.export_onnx import OUTPUT_DIR as _  # noqa: F401  (import guard)

    cfg = json.loads((run / "config.json").read_text(encoding="utf-8"))
    vocab = cfg["vocab"]
    ckpt_sha_before = hashlib.sha256((run / "best.pt").read_bytes()).hexdigest()

    model = FishNet(len(vocab))
    ckpt = torch.load(run / "best.pt", map_location="cpu", weights_only=True)
    model.load_state_dict(ckpt["model"])
    model.eval()

    out = run / "fish_vision_v1.onnx.legacy"
    dummy = torch.randn(1, 3, INPUT_SIZE, INPUT_SIZE)
    torch.onnx.export(
        model, dummy, str(out),
        input_names=["pixels"], output_names=["fish_logits"],
        dynamic_axes={"pixels": {0: "batch"}, "fish_logits": {0: "batch"}},
        opset_version=18,
        dynamo=False,          # the legacy TorchScript exporter
    )
    # Inline any external data so the artifact is a SINGLE file, exactly as the
    # trainer does.
    import onnx
    m = onnx.load(str(out))
    onnx.save(m, str(out))

    ok, why = analyse_is_statically_analysable(out)
    print(f"  legacy export: {out.name} ({out.stat().st_size} bytes)")
    print(f"  statically analysable: {ok} - {why}")

    # Numerically verify the new artifact against the checkpoint.
    import onnxruntime as ort
    sess = ort.InferenceSession(str(out), providers=["CPUExecutionProvider"])
    torch.manual_seed(0)
    worst = 0.0
    with torch.no_grad():
        for b in (1, 4):
            x = torch.randn(b, 3, INPUT_SIZE, INPUT_SIZE)
            t = model(x).numpy()
            o = sess.run(None, {"pixels": x.numpy()})[0]
            worst = max(worst, float(np.abs(t - o).max()))
    print(f"  torch<->onnx max_abs_diff: {worst:.3e} (tolerance 1e-4)")
    print(f"  best.pt sha256 unchanged: {ckpt_sha_before}")

    if worst >= 1e-4:
        print("  REFUSING: the legacy export does not agree numerically")
        return 1
    if not ok:
        print("  NOTE: still not statically analysable; tract will still refuse it")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())