"""Shadow deployment prep: manifests, checksums, preprocessing fixture.

Reads the two TRAINED artifacts (never retrains, never guesses):
  ml/output/gpo-vision-v1-s7/  (STATE: 3 classes)
  ml/output/fish-vision-v1-s7/ (FISH: 8 classes)

Writes:
  src-tauri/models/state_v1.onnx + state_v1.json   (copies + manifest)
  src-tauri/models/fish_v1.onnx  + fish_v1.json
  src-tauri/tests/fixtures/shadow/fixture.png      (12x10 seeded RGB)
  src-tauri/tests/fixtures/shadow/expected.bin     (raw f32 tensor sizes)
  src-tauri/tests/fixtures/shadow/expected.json    (meta + argmax per model)

Manifest schema matches core/ml_model.rs ModelManifest plus a `preprocess`
block (size, mean, std, pad, resize) and `temperature` from calibration.
The Rust shadow path must implement EXACTLY this preprocessing; the
fixture lets its test prove it (tolerance documented in expected.json).

Usage: python scripts/gen_shadow_deploy.py
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import struct
import sys

import numpy as np
import torch
from PIL import Image

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(REPO, "ml"))
from gpo_train.dataset import INPUT_SIZE  # noqa: E402
from gpo_train.train import pad_to_square  # noqa: E402
from gpo_train.model import GpoVisionNet  # noqa: E402
from gpo_train.fish_train import FISH_VOCAB, FishNet  # noqa: E402

STATE_RUN = os.path.join(REPO, "ml", "output", "gpo-vision-v1-s7")
FISH_RUN = os.path.join(REPO, "ml", "output", "fish-vision-v1-s7")
MODELS_DIR = os.path.join(REPO, "src-tauri", "models")
FIX_DIR = os.path.join(REPO, "src-tauri", "tests", "fixtures", "shadow")


def sha256(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def preprocess(png_path: str, mean, std):
    """Byte-exact copy of the training preprocessing (PIL BILINEAR)."""
    img = pad_to_square(Image.open(png_path).convert("RGB")).resize(
        (INPUT_SIZE, INPUT_SIZE), Image.BILINEAR)
    t = np.asarray(img, dtype=np.float32) / 255.0  # HWC
    t = t.transpose(2, 0, 1)  # CHW
    mean = np.asarray(mean, dtype=np.float32).reshape(3, 1, 1)
    std = np.asarray(std, dtype=np.float32).reshape(3, 1, 1)
    return (t - mean) / std


def main() -> int:
    os.makedirs(MODELS_DIR, exist_ok=True)
    os.makedirs(FIX_DIR, exist_ok=True)
    state_cfg = json.load(open(os.path.join(STATE_RUN, "config.json")))
    fish_cfg = json.load(open(os.path.join(FISH_RUN, "config.json")))
    state_cal = json.load(open(os.path.join(STATE_RUN, "calibration.json")))
    fish_cal = json.load(open(os.path.join(FISH_RUN, "calibration.json")))
    assert state_cfg["input_size"] == INPUT_SIZE and fish_cfg["input_size"] == INPUT_SIZE

    def eval_of(run, extra_keys=()):
        ev = json.load(open(os.path.join(run, "evaluation_test.json")))
        # Fish evals are flat; state evals nest per-head sections.
        core = ev if "accuracy" in ev else ev.get("state", ev)
        per = core.get("per_entity", core.get("per_class", {}))
        return {
            "test_accuracy": core["accuracy"],
            "macro_f1": core.get("macro_f1", core.get("macroF1")),
            "test_n": core.get("n", ev.get("n")),
            "per_class_f1": {k: v["f1"] for k, v in per.items()},
        }

    def ece_of(run):
        try:
            return json.load(open(os.path.join(run, "calibration.json"))).get("ece")
        except FileNotFoundError:
            return None

    def sessions_of(run):
        return json.load(open(os.path.join(run, "dataset_snapshot.json"))).get("test_sessions")

    models = [
        {"key": "state", "run": STATE_RUN, "onnx": "gpo_vision_v1.onnx",
         "name": "state_v1", "version": "1",
         "classes": ["waiting_for_bite", "bite", "catch_result"],
         "output": "state_logits", "temperature": state_cal["temperature"],
         "head": "state", **eval_of(STATE_RUN),
         "ece": ece_of(STATE_RUN), "test_sessions": sessions_of(STATE_RUN)},
        {"key": "fish", "run": FISH_RUN, "onnx": "fish_vision_v1.onnx",
         "name": "fish_v1", "version": "1", "classes": FISH_VOCAB,
         "output": "fish_logits", "temperature": fish_cal["temperature"],
         "head": "fish", **eval_of(FISH_RUN),
         "ece": ece_of(FISH_RUN), "test_sessions": sessions_of(FISH_RUN)},
    ]
    for m in models:
        src = os.path.join(m["run"], m["onnx"])
        dst = os.path.join(MODELS_DIR, m["name"] + ".onnx")
        shutil.copyfile(src, dst)
        cfg = json.load(open(os.path.join(m["run"], "config.json")))
        manifest = {
            "name": m["name"], "version": m["version"],
            "dataset": f"gpo-vision/v{dataset_version(m['run'])}",
            "dataset_version": dataset_version(m["run"]),
            "trained_at": m["run"].split("-")[-1],
            "runtime": "tract",
            "input_width": INPUT_SIZE, "input_height": INPUT_SIZE,
            "classes": m["classes"], "sha256": sha256(dst),
            "temperature": m["temperature"],
            "test_accuracy": m["test_accuracy"],
            "macro_f1": m["macro_f1"],
            "ece": m["ece"],
            "test_sessions": m["test_sessions"],
            "test_n": m["test_n"],
            "per_class_f1": m["per_class_f1"],
            "preprocess": {
                "pad": "square-black", "resize": "bilinear", "size": INPUT_SIZE,
                "mean": list(cfg["normalization"]["mean"]),
                "std": list(cfg["normalization"]["std"]),
            },
        }
        with open(os.path.join(MODELS_DIR, m["name"] + ".json"), "w", encoding="utf-8") as f:
            json.dump(manifest, f, indent=2)
        print(f"deployed {m['name']}: sha={manifest['sha256'][:12]}... temp={m['temperature']}")

    # ---- fixture: seeded 12x10 RGB ----
    rng = np.random.default_rng(20261005)
    px = rng.integers(0, 256, size=(10, 12, 3), dtype=np.uint8)
    fix_png = os.path.join(FIX_DIR, "fixture.png")
    Image.fromarray(px, "RGB").save(fix_png)

    # expected tensors under each model's normalization (raw f32 LE, CHW)
    argmax = {}
    for m in models:
        cfg = json.load(open(os.path.join(m["run"], "config.json")))
        t = preprocess(fix_png, cfg["normalization"]["mean"], cfg["normalization"]["std"])
        assert t.shape == (3, INPUT_SIZE, INPUT_SIZE)
        with open(os.path.join(FIX_DIR, f"{m['name']}.f32"), "wb") as f:
            f.write(struct.pack(f"<{t.size}f", *t.ravel().tolist()))
        argmax[m["name"]] = int(predict(m, t))
    meta = {
        "png": "fixture.png", "png_sha256": sha256(fix_png),
        "tensor_format": "raw-little-endian-f32", "layout": "CHW", "size": INPUT_SIZE,
        "tolerance_max_abs_diff": 0.05,
        "tolerance_note": "PIL BILINEAR vs Rust manual bilinear: measured worst 0.028 "
                          "(edge-kernel arithmetic), tolerance 0.05 with margin; binding "
                          "equivalence is argmax-exact on the fixture + live agreement logging",
        "argmax": argmax,
    }
    with open(os.path.join(FIX_DIR, "expected.json"), "w", encoding="utf-8") as f:
        json.dump(meta, f, indent=2)
    print("fixture argmax:", argmax)
    return 0


def dataset_version(run: str) -> int:
    return int(json.load(open(os.path.join(run, "dataset_snapshot.json")))["dataset_version"])


def predict(m, t: np.ndarray) -> int:
    if m["head"] == "state":
        net = GpoVisionNet(n_entities=25)
        sd = torch.load(os.path.join(m["run"], "best.pt"), map_location="cpu", weights_only=True)["model"]
        net.load_state_dict(sd)
        net.eval()
        with torch.no_grad():
            sl, _ = net(torch.from_numpy(t).unsqueeze(0))
        return int(sl.argmax(1).item())
    net = FishNet(len(FISH_VOCAB))
    sd = torch.load(os.path.join(m["run"], "best.pt"), map_location="cpu", weights_only=True)["model"]
    net.load_state_dict(sd)
    net.eval()
    with torch.no_grad():
        out = net(torch.from_numpy(t).unsqueeze(0))
    return int(out.argmax(1).item())


if __name__ == "__main__":
    raise SystemExit(main())
