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

Usage:
    python scripts/gen_shadow_deploy.py                     # regenerate both slots
    python scripts/gen_shadow_deploy.py --fish-run ml/output/fish-vision-v2-s7 \
        --fish-version 3 --fish-name fish_v1   # new fish weights into the slot
"""

# ---------------------------------------------------------------------------
# Why the fish run is parameterised
# ---------------------------------------------------------------------------
# This script used to hardcode `ml/output/fish-vision-v1-s7`, which made the
# deploy path single-use: after the vocabulary became the 15 confirmed species
# there is no way to put the new weights into the slot without editing the
# script. The state run stays as it is - state_v1 is solved, protected, and must
# not be retrained - so only the fish run takes arguments.
#
# The arguments do NOT relax anything: `classes` still comes from the run's own
# `config.json["vocab"]`, `vocab_sha` is recomputed over that list, and the
# manifest `version` is whatever the candidate registry records for the run. It
# is a plumbing change, not a contract change.

from __future__ import annotations

import argparse
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
from gpo_train.fish_train import FishNet  # noqa: E402

STATE_RUN = os.path.join(REPO, "ml", "output", "gpo-vision-v1-s7")
FISH_RUN_DEFAULT = os.path.join(REPO, "ml", "output", "fish-vision-v1-s7")
MODELS_DIR = os.path.join(REPO, "src-tauri", "models")
FIX_DIR = os.path.join(REPO, "src-tauri", "tests", "fixtures", "shadow")


def parse_args() -> argparse.Namespace:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--fish-run", default=FISH_RUN_DEFAULT,
                    help="trained fish run directory to deploy into the fish slot")
    ap.add_argument("--fish-name", default="fish_v1",
                    help="manifest/model name written into src-tauri/models")
    ap.add_argument("--fish-version", default="1",
                    help="manifest `version`; must be the registered candidate's "
                         "model_version so shadow telemetry is revision-scoped")
    ap.add_argument("--state-run", default=STATE_RUN)
    ap.add_argument("--skip-state", action="store_true",
                    help="only deploy the fish slot (state_v1 is already deployed)")
    return ap.parse_args()


def vocab_sha(classes) -> str:
    """Fingerprint of the index -> label mapping.

    Must match `crate::core::ml_model::sha256_hex(classes.join("\\n"))` on the
    Rust side, which is what `shadow_infer` verifies at load.
    """
    return hashlib.sha256("\n".join(classes).encode("utf-8")).hexdigest()


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
    global FISH_RUN_DEFAULT
    args = parse_args()
    os.makedirs(MODELS_DIR, exist_ok=True)
    os.makedirs(FIX_DIR, exist_ok=True)
    FISH_RUN_DEFAULT = os.path.abspath(args.fish_run)

    models = []
    if not args.skip_state:
        # state_v1 is SOLVED AND PROTECTED. It is regenerated here for one reason
        # only: the shadow FIXTURE needs its expected tensors, and that needs the
        # model present. Its MANIFEST IS NOT REWRITTEN.
        #
        # Rewriting it added `test_support` and moved `vocab_sha`, which changes
        # what the promotion gate can compare for the state family - a silent
        # behaviour change to a signed-off subsystem that is out of scope. The
        # weights are byte-identical either way, so nothing is gained.
        #
        # Its class list is the three game states, not a fish vocabulary: the
        # state trainer's config.json has no `vocab` key, and inventing one would
        # couple a protected model to an unrelated vocabulary file.
        models.append({
            "key": "state", "run": os.path.abspath(args.state_run),
            "onnx": "gpo_vision_v1.onnx", "name": "state_v1", "version": "1",
            "classes": ["waiting_for_bite", "bite", "catch_result"],
            "output": "state_logits", "head": "state",
            # Leave an existing manifest alone. Absent (fresh checkout) still
            # synthesises one.
            "reuse_manifest": True,
        })
    models.append({
        "key": "fish", "run": FISH_RUN_DEFAULT,
        "onnx": "fish_vision_v1.onnx", "name": args.fish_name,
        "version": str(args.fish_version), "head": "fish", "from_run_config": True,
    })

    for m in models:
        run = m["run"]
        cfg = json.load(open(os.path.join(run, "config.json")))
        cal = json.load(open(os.path.join(run, "calibration.json")))
        if m.get("from_run_config"):
            # The fish class map comes from the TRAINER's own config, never from
            # FISH_VOCAB.txt here: this script deploys a trained artifact, and
            # the artifact states what it was trained on. Re-deriving it from a
            # file would let a manifest and its weights describe different heads.
            m["classes"] = cfg["vocab"]
        m["temperature"] = cal["temperature"]
        assert cfg["input_size"] == INPUT_SIZE, f"{run}: unexpected input_size"
        ev = json.load(open(os.path.join(run, "evaluation_test.json")))
        core = ev if "accuracy" in ev else ev.get("state", ev)
        per = core.get("per_entity", core.get("per_class", {}))
        m.update({
            "test_accuracy": core["accuracy"],
            "macro_f1": core.get("macro_f1", core.get("macroF1")),
            "test_n": core.get("n", ev.get("n")),
            "per_class_f1": {k: v["f1"] for k, v in per.items()},
            "per_class_support": {k: int(v.get("support", 0)) for k, v in per.items()},
            # Test support per class. `compare()` refuses to let a class with
            # very few held-out examples decide a promotion, and it needs the
            # count to do that.
        })
        m["ece"] = cal.get("ece")
        m["test_sessions"] = json.load(
            open(os.path.join(run, "dataset_snapshot.json")))["test_sessions"]

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
            # Fingerprint of the index -> label mapping. The .onnx checksum
            # cannot catch a permuted `classes` array (the map lives here, not in
            # the weights), so it is pinned and VERIFIED at load.
            "vocab_sha": vocab_sha(m["classes"]),
            "temperature": m["temperature"],
            "test_accuracy": m["test_accuracy"],
            "macro_f1": m["macro_f1"],
            "ece": m["ece"],
            "test_sessions": m["test_sessions"],
            "test_n": m["test_n"],
            "per_class_f1": m["per_class_f1"],
            "test_support": m.get("per_class_support", {}),
            "preprocess": {
                "pad": "square-black", "resize": "bilinear", "size": INPUT_SIZE,
                "mean": list(cfg["normalization"]["mean"]),
                "std": list(cfg["normalization"]["std"]),
            },
        }
        dest = os.path.join(MODELS_DIR, m["name"] + ".json")
        if m.get("reuse_manifest") and os.path.isfile(dest):
            with open(dest, encoding="utf-8") as f:
                existing = json.load(f)
            if existing.get("sha256") == manifest["sha256"]:
                print(f"kept protected {m['name']} manifest (sha {manifest['sha256'][:12]}..., "
                      f"{len(manifest['classes'])} classes)")
                continue
            # The weights under the manifest changed, so it is no longer the
            # signed-off artifact. Say so loudly rather than leaving a manifest
            # that describes different bytes.
            raise SystemExit(
                f"REFUSING: {dest} is protected but its sha256 "
                f"({existing.get('sha256', '?')[:12]}...) does not match the run's "
                f"weights ({manifest['sha256'][:12]}...). Redeploying state_v1 is a "
                "separate, explicit decision."
            )
        # Trailing newline, always. `json.dump` alone leaves the last byte as
        # `}`, so every regenerated manifest showed as a whole-file diff against
        # the committed one ("\\ No newline at end of file") even when nothing
        # semantic had changed, and half the committed manifests ended with a
        # newline while half did not - purely by which tool wrote them last.
        with open(dest, "w", encoding="utf-8") as f:
            json.dump(manifest, f, indent=2)
            f.write("\n")
        print(f"deployed {m['name']}: sha={manifest['sha256'][:12]}... "
              f"temp={m['temperature']} classes={len(manifest['classes'])}")

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
        f.write("\n")
    print("fixture argmax:", argmax)
    return 0


def dataset_version(run: str) -> int:
    return int(json.load(open(os.path.join(run, "dataset_snapshot.json")))["dataset_version"])


def predict(m, t: np.ndarray) -> int:
    sd = torch.load(os.path.join(m["run"], "best.pt"), map_location="cpu", weights_only=True)["model"]
    if m["head"] == "state":
        net = GpoVisionNet(n_entities=25)
        net.load_state_dict(sd)
        net.eval()
        with torch.no_grad():
            sl, _ = net(torch.from_numpy(t).unsqueeze(0))
        return int(sl.argmax(1).item())
    # The head width comes from the run's own class list, never from the file
    # on disk: a model trained on a 15-class map must be widened by its own
    # vocabulary, or a checkpoint's shapes will silently refuse to load.
    net = FishNet(len(m["classes"]))
    net.load_state_dict(sd)
    net.eval()
    with torch.no_grad():
        out = net(torch.from_numpy(t).unsqueeze(0))
    return int(out.argmax(1).item())


if __name__ == "__main__":
    raise SystemExit(main())
