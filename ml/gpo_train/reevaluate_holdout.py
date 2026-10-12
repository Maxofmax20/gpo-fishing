"""Evaluate deployed weights on a candidate's frozen holdout, without retraining.

By default writes a separate report. --write-manifest explicitly updates only
verified evaluation metadata; it refuses if either deployed file changed.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import numpy as np
from PIL import Image
from .dataset import STATE_LABELS, load_snapshot, build_splits
from .evaluation_contract import holdout_fingerprint
from .metrics import accuracy, macro_f1, per_class_prf, apply_temperature, expected_calibration_error


def evaluate_model(snapshot, indices, manifest, model_path, family):
    import onnxruntime as ort
    classes = manifest["classes"]
    if not classes or len(set(classes)) != len(classes):
        raise ValueError("invalid class map")
    if family == "state" and classes != STATE_LABELS:
        raise ValueError("state class order mismatch")
    if hashlib.sha256(Path(model_path).read_bytes()).hexdigest() != manifest["sha256"]:
        raise ValueError("deployed artifact checksum mismatch")
    spec = manifest["preprocess"]
    if spec["pad"] != "square-black" or spec["resize"] != "bilinear":
        raise ValueError("unsupported preprocessing")
    mean = np.asarray(spec["mean"], np.float32)[:, None, None]
    std = np.asarray(spec["std"], np.float32)[:, None, None]
    if mean.shape != (3,1,1) or std.shape != (3,1,1) or not np.isfinite(mean).all() or not np.isfinite(std).all() or (std <= 0).any():
        raise ValueError("invalid normalization")
    session = ort.InferenceSession(str(model_path), providers=["CPUExecutionProvider"])
    outputs = session.get_outputs()
    if len(outputs) == 1:
        output_name = outputs[0].name
    elif family == "state" and [o.name for o in outputs] == ["state_logits", "entity_logits"]:
        # The deployed v1 state artifact predates the single-head exporter.
        # Measure its named state head without interpreting entity logits.
        output_name = "state_logits"
    else:
        raise ValueError("expected one classification head or the legacy named state head")
    true, pred, logits = [], [], []
    for i in indices:
        row = snapshot.rows[i]
        label = row.entity_id if family == "fish" else row.game_state
        if label not in classes:
            raise ValueError(f"holdout class absent from deployed vocabulary: {label}")
        with Image.open(row.image_path) as source:
            image = source.convert("RGB")
        side = max(image.size)
        canvas = Image.new("RGB", (side,side), (0,0,0))
        canvas.paste(image, ((side-image.width)//2, (side-image.height)//2))
        image = canvas.resize((spec["size"],spec["size"]), Image.Resampling.BILINEAR)
        pixels = np.asarray(image, np.float32).transpose(2,0,1) / 255.0
        pixels = ((pixels - mean) / std)[None]
        output = session.run([output_name], {session.get_inputs()[0].name: pixels})[0]
        if output.shape != (1,len(classes)) or not np.isfinite(output).all():
            raise ValueError("invalid classifier output")
        values = output[0].tolist()
        logits.append(values)
        true.append(classes.index(label))
        pred.append(int(np.argmax(output[0])))
    if not true:
        raise ValueError("empty frozen holdout")
    temperature = manifest["temperature"]
    if not np.isfinite(temperature) or temperature <= 0:
        raise ValueError("invalid frozen temperature")
    per = per_class_prf(true, pred, len(classes))
    return {"accuracy": accuracy(true,pred), "macro_f1": macro_f1(true,pred,len(classes)),
            "ece": expected_calibration_error(apply_temperature(logits,temperature),true),
            "n": len(true), "test_sessions": len({snapshot.rows[i].session_id for i in indices}),
            "holdout_fingerprint": holdout_fingerprint(snapshot,indices,classes),
            "artifact_sha256": manifest["sha256"],
            "per_class": {c: per[i] for i,c in enumerate(classes)}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--family", choices=["fish","state"], required=True)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--snapshot", type=Path, required=True)
    parser.add_argument("--images", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--write-manifest", action="store_true")
    args = parser.parse_args()
    original = args.manifest.read_bytes()
    manifest = json.loads(original)
    os.environ["GPO_DATASET_DIR"] = str(args.snapshot.resolve())
    os.environ["GPO_IMAGES_DIR"] = str(args.images.resolve())
    snapshot = load_snapshot()
    indices = build_splits(snapshot).test
    if args.family == "fish":
        from .fish_train import FISH_VOCAB
        if manifest["classes"] != FISH_VOCAB:
            raise ValueError("incumbent must cover canonical fish classes in order")
        indices = [i for i in indices if snapshot.rows[i].game_state == "catch_result" and snapshot.rows[i].entity_id in FISH_VOCAB]
    report = evaluate_model(snapshot,indices,manifest,args.model,args.family)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x", encoding="utf-8") as handle:
        json.dump(report,handle,indent=2)
    if args.write_manifest:
        if args.manifest.read_bytes() != original or hashlib.sha256(args.model.read_bytes()).hexdigest() != manifest["sha256"]:
            raise RuntimeError("deployed files changed during evaluation; metadata update refused")
        manifest.update(test_accuracy=report["accuracy"], macro_f1=report["macro_f1"], ece=report["ece"],
                        test_n=report["n"], test_sessions=report["test_sessions"],
                        holdout_fingerprint=report["holdout_fingerprint"],
                        per_class_f1={c:v["f1"] for c,v in report["per_class"].items()},
                        test_support={c:v["support"] for c,v in report["per_class"].items()}, kept_accuracy=None)
        temp_name = None
        try:
            with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=args.manifest.parent, delete=False) as handle:
                temp_name = handle.name
                json.dump(manifest,handle,indent=2)
                handle.flush()
                os.fsync(handle.fileno())
            os.replace(temp_name,args.manifest)
        finally:
            if temp_name and Path(temp_name).exists():
                Path(temp_name).unlink()
    print(f"Frozen holdout evaluated: {report['n']} samples, {report['test_sessions']} sessions; {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
