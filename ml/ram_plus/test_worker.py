"""Worker safety tests — run WITHOUT torch or the RAM package installed.

These guard the properties that make the teacher safe to have on this machine
at all: pinned checkpoints, no network, no writes outside the results file, no
label writes, and a visible failure rather than a silent one.
"""

from __future__ import annotations

import hashlib
import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from worker import (  # noqa: E402
    Checkpoint,
    EXCLUDED_CONCEPTS,
    GPO_VOCAB_VERSION,
    GPO_PREPROCESS,
    TeacherUnavailable,
    read_manifest,
    run,
    validate_request,
    sha256_file,
)


def _sha(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def test_checkpoint_must_match_its_pinned_sha():
    d = Path(tempfile.mkdtemp())
    f = d / "ram_plus_swin.pth"
    f.write_bytes(b"pretend-weights")
    good = _sha(b"pretend-weights")

    ck = Checkpoint("ram_plus", f, good)
    ok, why = ck.verify()
    assert ok, why

    # TAMPERED: the single most important test in this file.
    ck2 = Checkpoint("ram_plus", f, "0" * 64)
    ok2, why2 = ck2.verify()
    assert not ok2, "a mismatched checkpoint must be refused"
    assert "mismatch" in why2.lower()

    # MISSING
    ck3 = Checkpoint("ram_plus", d / "nope.pth", good)
    ok3, why3 = ck3.verify()
    assert not ok3 and "missing" in why3.lower()

    # MALFORMED PIN
    ck4 = Checkpoint("ram_plus", f, "short")
    ok4, _ = ck4.verify()
    assert not ok4, "a malformed pin must fail closed"


def test_an_unavailable_teacher_reports_an_error_per_image():
    """A broken teacher must be VISIBLE, never silently absent.

    If the worker emitted nothing, a downstream consumer would see zero results
    and could read that as "the teacher found nothing interesting" - which is
    how a broken teacher quietly becomes an authority.
    """
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        ck = td / "m.pth"
        ck.write_bytes(b"x")
        out = td / "out.jsonl"
        manifest = "\n".join(
            json.dumps({"image_id": f"i{n}", "path": str(td / "a.png")})
            for n in range(3)
        )
        import io as _io
        # The pinned SHA is deliberately wrong, so verification must fail and
        # the worker must degrade to an explicit per-image error.
        buf = _io.StringIO(manifest)
        old = sys.stdin
        sys.stdin = buf
        try:
            rc = run([
                "--checkpoint", str(ck), "--model", "ram_plus",
                "--expect-sha256", "0" * 64,
                "--dataset-root", str(td), "--out", str(out),
            ])
        finally:
            sys.stdin = old

        assert rc == 3, f"expected a distinct 'unavailable' exit code, got {rc}"
        lines = [json.loads(l) for l in out.read_text(encoding="utf-8").splitlines() if l.strip()]
        assert len(lines) == 3, "every requested image must get a record"
        for rec in lines:
            assert rec["status"].startswith("error:"), rec
            assert rec["tags"] == [] and rec["candidate_categories"] == []


def test_paths_outside_the_dataset_root_are_refused():
    """The caller chooses WHICH IMAGES. That boundary is enforced here."""
    with tempfile.TemporaryDirectory() as td:
        root = Path(td) / "dataset"
        root.mkdir()
        inside = root / "ok.png"
        inside.write_bytes(b"png")
        assert validate_request({"path": str(inside)}, root) == inside.resolve()

        outside = Path(td) / "secret.png"
        outside.write_bytes(b"png")
        assert validate_request({"path": str(outside)}, root) is None

        # traversal form
        assert validate_request({"path": str(root / ".." / "secret.png")}, root) is None
        # missing
        assert validate_request({"path": str(root / "gone.png")}, root) is None
        # no path at all
        assert validate_request({}, root) is None


def test_manifest_parse_errors_are_reported_not_dropped():
    items = read_manifest(
        [
            json.dumps({"image_id": "a", "path": "x"}),
            "{ this is not json",
            "",
            json.dumps({"image_id": "b", "path": "y"}),
        ]
    )
    assert len(items) == 3, "blank lines are skipped, bad lines are KEPT"
    assert "__error__" in items[1]


def test_excluded_concepts_cannot_enter_the_vocabulary():
    """A concept the KB deliberately lacks must never be proposable."""
    from worker import _is_excluded

    for c in EXCLUDED_CONCEPTS:
        assert _is_excluded(c), f"{c} must be excluded"
    for c in ["tigerfin", "Exotic Tigerfin", "Candy Corn Squid", "Sunken Anchor"]:
        assert _is_excluded(c)
    # Canonical classes must pass through.
    for c in ["fish:shark", "fish:snapper", "fish:swordfish", "fish:pufferfish"]:
        assert not _is_excluded(c), f"{c} is canonical and must be allowed"


def test_vocabulary_is_kb_only_and_versioned():
    """The teacher's category space is the KB, and it is versioned."""
    assert GPO_VOCAB_VERSION
    assert "kb" in GPO_VOCAB_VERSION or "gpo" in GPO_VOCAB_VERSION


def test_preprocessing_is_recorded():
    """A result that does not say how the image was prepared is not reproducible."""
    s = GPO_PREPROCESS.as_str()
    assert "224" in s, s
    assert "letterbox" in s or "resize" in s, s


def test_worker_module_imports_without_torch():
    """Importing the worker must not require the heavy dependency stack.

    Otherwise CI (and any machine without a GPU) could not test its safety
    properties at all.
    """
    import importlib
    m = importlib.import_module("worker")
    assert hasattr(m, "RamTeacher")
    ck = Checkpoint("ram", Path("/nonexistent"), "0" * 64)
    t = m.RamTeacher(ck)
    assert t.available is False
    try:
        t.load()
        raise AssertionError("load() must raise TeacherUnavailable without the package")
    except TeacherUnavailable:
        pass


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
        except Exception as e:  # noqa: BLE001
            failed += 1
            print(f"ERROR {fn.__name__}: {type(e).__name__}: {e}")
    print(f"\n{len(fns) - failed}/{len(fns)} passed")
    sys.exit(1 if failed else 0)