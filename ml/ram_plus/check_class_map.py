"""Canonical-vocabulary contract, checked against the REAL exported model.

The Rust side (`tests/class_map_contract.rs`) proves FISH_VOCAB.txt agrees with
the deployed manifest. This proves the thing that actually matters at runtime:
the number of output logits the shipped network has.

A mismatch here is the failure the whole single-source-of-truth change exists
to prevent - readiness would count N classes while the model can only ever
emit M.
"""
import json
import os
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
sys.path.insert(0, str(REPO / "ml"))


def canonical_vocab():
    """Read FISH_VOCAB.txt exactly as fish_train.py does."""
    path = REPO / "FISH_VOCAB.txt"
    out, seen = [], set()
    for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        t = line.strip()
        if not t or t.startswith("#"):
            continue
        if not t.startswith("fish:"):
            raise AssertionError(f"FISH_VOCAB.txt:{n}: {t!r} is not a fish class id")
        if t in seen:
            raise AssertionError(f"FISH_VOCAB.txt:{n}: duplicate {t!r}")
        seen.add(t)
        out.append(t)
    assert out, "FISH_VOCAB.txt defines no classes"
    return out


def test_manifest_matches_canonical():
    vocab = canonical_vocab()
    man = json.loads((REPO / "src-tauri" / "models" / "fish_v1.json").read_text(encoding="utf-8"))
    assert man["classes"] == vocab, "deployed manifest class set/order diverges from FISH_VOCAB.txt"
    assert len(man["classes"]) == len(vocab)


def test_onnx_output_dimension_matches_canonical():
    """The trainer output dimension must equal the canonical vocabulary."""
    vocab = canonical_vocab()
    onnx = REPO / "src-tauri" / "models" / "fish_v1.onnx"
    if not onnx.is_file():
        raise AssertionError(f"missing {onnx}")
    try:
        import onnxruntime as ort
    except ImportError:
        print("SKIP: onnxruntime unavailable; cannot inspect the exported head")
        return
    sess = ort.InferenceSession(str(onnx), providers=["CPUExecutionProvider"])
    out = sess.get_outputs()[0]
    shape = out.shape
    width = None
    # Batch axis may be dynamic ('batch'); the class axis is the last one.
    for d in reversed(shape):
        if isinstance(d, int):
            width = d
            break
    assert width == len(vocab), (
        f"exported head emits {width} logits but FISH_VOCAB.txt defines "
        f"{len(vocab)} classes - every index after the first divergence is mislabelled"
    )
    print(f"OK: exported head width {width} == len(FISH_VOCAB) {len(vocab)}")


def test_vocab_sha_matches_manifest():
    import hashlib

    vocab = canonical_vocab()
    sha = hashlib.sha256("\n".join(vocab).encode("utf-8")).hexdigest()
    man = json.loads((REPO / "src-tauri" / "models" / "fish_v1.json").read_text(encoding="utf-8"))
    assert man.get("vocab_sha") == sha, "manifest vocab_sha does not match the canonical list"


def test_trainer_exposes_no_hardcoded_list():
    src = (REPO / "ml" / "gpo_train" / "fish_train.py").read_text(encoding="utf-8")
    assert "FISH_VOCAB.txt" in src, "trainer must read the canonical file"
    assert "FISH_VOCAB = [" not in src, "trainer must not define its own literal list"


def test_no_excluded_entity_was_sneaked_into_the_vocabulary():
    """The do-not-invent list must stay out, whatever the pressure to reach a count."""
    forbidden = [
        "tigerfin", "tiserfin", "skeletal", "candy", "jack-o", "fangfish",
        "anglerfish", "polka", "zebra", "ribbon", "sunken", "grouper", "tiger",
    ]
    for c in canonical_vocab():
        low = c.lower()
        for bad in forbidden:
            assert bad not in low, (
                f"{c!r} entered the canonical vocabulary; these entities were "
                f"deliberately excluded for lack of authoritative evidence"
            )


if __name__ == "__main__":
    fns = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    failed = 0
    for fn in fns:
        try:
            fn()
            print(f"PASS {fn.__name__}")
        except AssertionError as e:
            failed += 1
            print(f"FAIL {fn.__name__}: {e}")
    print(f"\n{len(fns) - failed}/{len(fns)} passed")
    sys.exit(1 if failed else 0)
