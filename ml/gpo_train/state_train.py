"""Complete state training, VAL calibration, frozen TEST evaluation and ONNX export.

Run from the checkout root: python -m ml.gpo_train.state_train --run-id NAME.
The supervisor supplies the frozen dataset and image roots through the environment.
"""
from __future__ import annotations
import argparse
import json
import sys
from pathlib import Path
import numpy as np
import torch
from torch.utils.data import DataLoader
from . import train
from .dataset import STATE_LABELS, INPUT_SIZE, load_snapshot, build_splits
from .evaluate import EvalDataset, run_split, state_report
from .evaluation_contract import holdout_fingerprint
from .metrics import apply_temperature, expected_calibration_error, fit_temperature
from .model import GpoVisionNet


class StateHead(torch.nn.Module):
    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, pixels):
        return self.model(pixels)[0]


def finish_run(run_id):
    out = Path(train.OUTPUT_DIR) / run_id
    config = json.loads((out / "config.json").read_text(encoding="utf-8"))
    info = json.loads((out / "dataset_snapshot.json").read_text(encoding="utf-8"))
    vocab = info["entity_vocab"]
    snap = load_snapshot()
    splits = build_splits(snap)
    model = GpoVisionNet(n_entities=len(vocab))
    model.load_state_dict(torch.load(out / "best.pt", map_location="cpu", weights_only=True)["model"])
    model.eval()
    norm = config["normalization"]

    def evaluate(indices):
        loader = DataLoader(EvalDataset(snap, indices, vocab, norm["mean"], norm["std"]),
                            batch_size=256, num_workers=0)
        result = run_split(model, loader, "cpu")
        return result, state_report(result["state_true"], result["state_pred"])

    val, val_report = evaluate(splits.validation)
    calibration = fit_temperature(val["state_logits"], val["state_true"])
    calibration["ece"] = expected_calibration_error(
        apply_temperature(val["state_logits"], calibration["temperature"]), val["state_true"])
    test, test_report = evaluate(splits.test)
    test_report["ece"] = expected_calibration_error(
        apply_temperature(test["state_logits"], calibration["temperature"]), test["state_true"])
    test_report["holdout_fingerprint"] = holdout_fingerprint(snap, splits.test, STATE_LABELS)
    config["vocab"] = list(STATE_LABELS)
    for filename, value in (("config.json", config), ("calibration.json", calibration),
                            ("evaluation_val.json", val_report), ("evaluation_test.json", test_report)):
        (out / filename).write_text(json.dumps(value, indent=2), encoding="utf-8")
    head = StateHead(model).eval()
    artifact = out / "gpo_vision_v1.onnx"
    torch.onnx.export(head, torch.randn(1, 3, INPUT_SIZE, INPUT_SIZE), str(artifact),
                      input_names=["pixels"], output_names=["state_logits"],
                      dynamic_axes={"pixels": {0: "batch"}, "state_logits": {0: "batch"}},
                      opset_version=18, dynamo=False)
    import onnx
    import onnxruntime as ort
    graph = onnx.load(str(artifact))
    onnx.checker.check_model(graph)
    onnx.save(graph, str(artifact))
    session = ort.InferenceSession(str(artifact), providers=["CPUExecutionProvider"])
    worst = 0.0
    with torch.no_grad():
        for batch in (1, 4):
            pixels = torch.randn(batch, 3, INPUT_SIZE, INPUT_SIZE)
            actual = session.run(None, {"pixels": pixels.numpy()})[0]
            worst = max(worst, float(np.abs(head(pixels).numpy() - actual).max()))
    passed = worst < 1e-4
    (out / "onnx_check.json").write_text(json.dumps({"max_abs_diff": worst, "tolerance": 1e-4,
                                                   "pass": passed, "single_file": True}), encoding="utf-8")
    if not passed:
        raise RuntimeError(f"ONNX output mismatch: {worst}")
    return 0


def main():
    if "--help" in sys.argv or "-h" in sys.argv:
        return train.main()
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--run-id", required=True)
    args, _ = parser.parse_known_args()
    status = train.main()
    return status if status else finish_run(args.run_id)


if __name__ == "__main__":
    raise SystemExit(main())
