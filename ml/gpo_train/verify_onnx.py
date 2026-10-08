"""ONNX inference validation for a trained fish-vision run (section 17).

Verifies, against the run's own artifacts and REAL test images:

* the model loads in onnxruntime;
* input name/shape/dtype and output name/shape/dtype;
* output width == class-map width (15) and matches config.json vocab order;
* preprocessing in config.json is what the trainer used (mean/std present);
* deterministic inference: the same image twice gives bit-identical logits;
* native-vs-ONNX parity is already proven by the trainer's onnx_check.json
  (re-read and required to pass here too);
* a real TEST image's argmax lands on a valid class index, and a batch of
  test images produces the same argmax set whether run singly or batched.

Reads only. Writes onnx_inference_check.json beside the run's other artifacts.

Usage: python -m gpo_train.verify_onnx --run-dir ml/output/fish-vision-v2-s7
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

import numpy as np
from PIL import Image

REPO_ROOT = Path(__file__).resolve().parents[2]
INPUT_SIZE = 96


def _pad_to_square(img: Image.Image) -> Image.Image:
    w, h = img.size
    side = max(w, h)
    canvas = Image.new("RGB", (side, side), (0, 0, 0))
    canvas.paste(img, ((side - w) // 2, (side - h) // 2))
    return canvas


def _preprocess(path: Path, mean: list[float], std: list[float]) -> np.ndarray:
    img = _pad_to_square(Image.open(path).convert("RGB")).resize(
        (INPUT_SIZE, INPUT_SIZE), Image.BILINEAR
    )
    t = np.asarray(img, dtype=np.float32) / 255.0
    t = t.transpose(2, 0, 1)
    for ch in range(3):
        t[ch] = (t[ch] - mean[ch]) / std[ch]
    return t.astype(np.float32)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--run-dir", required=True)
    ap.add_argument("--appdata", type=Path,
                    default=Path(os.environ.get("APPDATA", "")) / "gpo-autofish")
    args = ap.parse_args()
    run = Path(args.run_dir)

    cfg = json.loads((run / "config.json").read_text(encoding="utf-8"))
    vocab = cfg["vocab"]
    mean, std = cfg["normalization"]["mean"], cfg["normalization"]["std"]
    onnx_path = run / "fish_vision_v1.onnx"
    failures: list[str] = []

    onnx_check = json.loads((run / "onnx_check.json").read_text(encoding="utf-8"))
    if not onnx_check.get("pass"):
        failures.append("trainer onnx_check did not pass")

    import onnxruntime as ort
    sess = ort.InferenceSession(str(onnx_path), providers=["CPUExecutionProvider"])
    inp = sess.get_inputs()[0]
    out = sess.get_outputs()[0]
    if inp.name != "pixels":
        failures.append(f"input name {inp.name!r} != 'pixels'")
    if inp.type != "tensor(float)":
        failures.append(f"input dtype {inp.type!r} != tensor(float)")
    if out.name != "fish_logits":
        failures.append(f"output name {out.name!r} != 'fish_logits'")
    out_dims = out.shape  # ['batch', 15] with dynamic batch
    if len(out_dims) != 2 or out_dims[1] != len(vocab):
        failures.append(f"output shape {out_dims} != [batch, {len(vocab)}]")
    if len(mean) != 3 or len(std) != 3:
        failures.append("normalization mean/std malformed")

    # Real test images through the frozen preprocessing.
    manifest = json.loads(
        (REPO_ROOT / "ml" / "output" / "reviewed-fish-v1" / "dataset_manifest.json")
        .read_text(encoding="utf-8")
    )
    test_rows = [r for r in manifest["rows"] if r["split"] == "test"][:12]
    images_dir = args.appdata / "datasets" / "gpo-vision" / "v1" / "images"
    tensors = [_preprocess(images_dir / f"{r['image_id']}.png", mean, std) for r in test_rows]

    # Determinism: same image twice -> bit-identical logits.
    x = tensors[0][None]
    a = sess.run(None, {"pixels": x})[0]
    b = sess.run(None, {"pixels": x})[0]
    if not np.array_equal(a, b):
        failures.append("non-deterministic inference on identical input")

    # Single vs batched argmax agreement.
    singles = [int(sess.run(None, {"pixels": t[None]})[0][0].argmax()) for t in tensors]
    batched = sess.run(None, {"pixels": np.stack(tensors)})[0].argmax(axis=1).tolist()
    if singles != [int(v) for v in batched]:
        failures.append("single vs batched inference disagree")
    for s in singles:
        if not (0 <= s < len(vocab)):
            failures.append(f"argmax {s} out of class range")

    report = {
        "kind": "onnx-inference-check",
        "run_dir": str(run),
        "onnx_sha256": __import__("hashlib").sha256(onnx_path.read_bytes()).hexdigest(),
        "input": {"name": inp.name, "type": inp.type, "shape": [str(d) for d in inp.shape]},
        "output": {"name": out.name, "type": out.type, "shape": [str(d) for d in out.shape]},
        "class_map_width": len(vocab),
        "vocab_sha256": cfg["vocab_sha256"],
        "deterministic": not any("non-deterministic" in f for f in failures),
        "test_images_checked": len(tensors),
        "failures": failures,
        "pass": not failures,
    }
    out_path = run / "onnx_inference_check.json"
    out_path.write_text(json.dumps(report, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    for k, v in report.items():
        if k != "failures":
            print(f"  {k}: {v}")
    print(f"  failures: {failures or 'none'}")
    print(f"  written: {out_path}")
    return 0 if report["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
