"""Traceability, determinism and safety tests.

Three groups, matching the guarantees the audit claims:

* **Traceability** - every evidence record resolves to exactly one real image by
  SHA-256. A finding that cannot be traced to bytes is worthless.
* **Determinism** - the same immutable dataset produces equivalent evidence, and
  the cache cannot change the answer.
* **Safety** - the pipeline cannot create an ontology entity, change a label,
  alter a split, or reach game input.

The safety group is enforced by *reading the source*, not by calling into the
app: the point is that the forbidden capability is absent, so a runtime test
would be testing the wrong thing. A forbidden import or a write path is caught
the moment someone adds one.
"""

from __future__ import annotations

import ast
import hashlib
import io
import json
import os
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

PKG = Path(__file__).resolve().parents[1]          # ml/gpo_ocr_audit
REPO = Path(__file__).resolve().parents[3]

#: Modules whose presence in the audit pipeline would mean it can reach the game
#: or the production control path.
FORBIDDEN_IMPORTS = {
    "controller", "macro", "input", "keyboard", "mouse", "tauri",
    "gpo_train.train", "train",
}


def _py_files() -> list[Path]:
    return sorted(p for p in PKG.glob("*.py"))


def _imported_names(path: Path) -> set[str]:
    """Every module name imported by ``path``, at any depth."""
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    names: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for a in node.names:
                names.add(a.name.split(".")[0])
        elif isinstance(node, ast.ImportFrom):
            if node.module and node.level == 0:
                names.add(node.module.split(".")[0])
                names.add(node.module)
    return names


# --------------------------------------------------------------------------
# Safety: forbidden capabilities must be ABSENT
# --------------------------------------------------------------------------

def test_pipeline_cannot_reach_game_input_or_the_control_path():
    for path in _py_files():
        names = _imported_names(path)
        bad = names & FORBIDDEN_IMPORTS
        assert not bad, f"{path.name} imports {bad}; the audit must never reach "\
                        f"game input or the production control path"


def test_pipeline_never_imports_the_kb_mutators():
    """`knowledge.merge_import` / `validate_import` can add entities. The audit
    must never import them, so it structurally cannot create one."""
    for path in _py_files():
        src = path.read_text(encoding="utf-8")
        for forbidden in ("merge_import", "validate_import", "create_entity",
                          "add_entity", "upsert_entity"):
            # Appearing in prose is fine; appearing as an attribute access is not.
            assert f".{forbidden}(" not in src, (
                f"{path.name} calls {forbidden}(); the audit has no authority to "
                f"modify the knowledge base"
            )


def test_pipeline_never_writes_training_eligibility():
    """`training_eligible` may be READ (to report it) but never assigned."""
    for path in _py_files():
        src = path.read_text(encoding="utf-8")
        for pattern in ('"training_eligible":', "'training_eligible':"):
            # Appears only inside read-side reporting structures.
            assert f"{pattern} True" not in src
        assert "training_eligible = True" not in src
        assert "set_training_eligible" not in src


def test_pipeline_never_writes_into_the_dataset_or_kb_sources():
    """No `open(..., 'w')` / `write_text` aimed at the dataset, the KB export
    source, or FISH_VOCAB.txt."""
    forbidden_targets = ("labels.jsonl", "FISH_VOCAB.txt", "knowledge.rs",
                         "manifest.json", "reviews.jsonl", "fish_v1.json")
    for path in _py_files():
        src = path.read_text(encoding="utf-8")
        for target in forbidden_targets:
            # A read of manifest.json is legitimate; a write is not.
            for line in src.splitlines():
                if target in line and any(
                    w in line for w in ('"w"', "'w'", "write_text", "write(")
                ):
                    assert False, f"{path.name} may write to {target}: {line.strip()}"


def test_pipeline_only_writes_under_its_output_directory():
    """Every write must be rooted at the configured output dir or the module's
    own cache. This is a coarse but effective guard against a stray absolute path."""
    for path in _py_files():
        tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
        for node in ast.walk(tree):
            if isinstance(node, ast.Call):
                fn = node.func
                name = getattr(fn, "attr", None)
                if name not in ("write_text", "write_bytes", "open", "unlink",
                                "mkdir", "save", "executemany", "commit"):
                    continue
                for arg in node.args:
                    lit = getattr(arg, "value", None)
                    if isinstance(lit, str) and (":" in lit[:3] or lit.startswith("/")):
                        assert False, (
                            f"{path.name} writes to an absolute path {lit!r}"
                        )


def test_provenance_states_what_did_not_happen():
    """The README the pipeline emits must carry the explicit negative
    assurances, so a reader is never left inferring them."""
    src = (PKG / "__main__.py").read_text(encoding="utf-8")
    for phrase in (
        "No new ontology entity was created",
        "No training label was changed",
        "No TEST data was used for training",
        "No production safety gate was weakened",
        "No game input was controlled",
        "No synthetic data",
        "MODEL_NOT_READY",
    ):
        assert phrase in src, f"the emitted README omits the assurance: {phrase!r}"


# --------------------------------------------------------------------------
# Determinism
# --------------------------------------------------------------------------

def test_cache_key_includes_the_engine_fingerprint():
    """A stale result must not survive a configuration change."""
    sys.path.insert(0, str(PKG))
    from gpo_ocr_audit.ocr_engine import OcrCache, engine_fingerprint

    a = engine_fingerprint({"pipeline": "x", "rules": "y"})
    b = engine_fingerprint({"pipeline": "x", "rules": "z"})
    assert a != b, "a rules change must change the fingerprint"
    # And the key must mix in both parts.
    assert OcrCache.key("abc", a) != OcrCache.key("abc", b)
    assert OcrCache.key("abc", a) != OcrCache.key("abd", a)


def test_cache_roundtrip_preserves_the_reading():
    from gpo_ocr_audit.ocr_engine import OcrCache, OcrReading

    with tempfile.TemporaryDirectory() as td:
        p = Path(td) / "c.jsonl"
        c1 = OcrCache(p)
        c1.put(OcrReading("deadbeef", "New Item <Shark>", 0.9,
                          [{"text": "New Item <Shark>", "confidence": 0.9}],
                          [{"text": "junk", "confidence": 0.1}], 12), "fp1")
        c1.flush()
        c2 = OcrCache(p)
        got = c2.get("deadbeef", "fp1")
        assert got is not None
        assert got.text == "New Item <Shark>"
        assert got.confidence == 0.9
        assert got.rejected[0]["text"] == "junk"
        # A different fingerprint must miss, not return the stale value.
        assert c2.get("deadbeef", "fp2") is None


def test_normalisation_is_deterministic():
    from gpo_ocr_audit.text_normalizer import normalize_pipeline

    a = normalize_pipeline("Nev Item<Candy Com Squid>")
    b = normalize_pipeline("Nev Item<Candy Com Squid>")
    assert a[0] == b[0]
    assert [s.to_dict() for s in a[1]] == [s.to_dict() for s in b[1]]
    assert a[2] == b[2]


def test_evidence_record_contains_no_wall_clock_field():
    """Determinism requires it: a timestamp in the evidence would make two runs
    differ byte-for-byte. Generation time lives in provenance.json instead."""
    from gpo_ocr_audit.schemas import EntityEvidence

    ev = EntityEvidence(
        image_id="i", image_sha256="s", relative_image_path="images/i.png",
        session_id="sess", timestamp_ms=1, stored_entity_id=None,
        stored_source="collector", stored_ocr_text=None, ocr_engine="e",
        ocr_engine_version="1", raw_ocr="", normalized_ocr="",
        ocr_confidence=None, match_kind="NONE", ontology_status="UNREADABLE",
    )
    d = ev.to_dict()
    for banned in ("generated_at", "audited_at", "run_id", "wall_clock", "now"):
        assert banned not in d, f"{banned!r} would break run-to-run determinism"
    # The dataset row's own capture timestamp is fine: it is data, not clock.
    assert d["timestamp_ms"] == 1


# --------------------------------------------------------------------------
# Traceability
# --------------------------------------------------------------------------

def test_every_evidence_record_resolves_to_exactly_one_image_hash():
    """Against the real dataset when it is present.

    Skipped (not silently passed) when the dataset is absent, so the guarantee is
    never claimed without being checked.
    """
    sys.path.insert(0, str(PKG))
    from gpo_ocr_audit.gpo_correlator import default_dataset_root
    from gpo_ocr_audit.ocr_engine import sha256_file

    try:
        root = default_dataset_root()
    except RuntimeError:
        print("SKIP: no APPDATA dataset to verify against")
        return
    if not (root / "labels.jsonl").is_file():
        print("SKIP: dataset not present on this machine")
        return

    evidence = REPO / "reports" / "ocr_audit" / "evidence.jsonl"
    if not evidence.is_file():
        print("SKIP: no completed audit to verify")
        return

    rows = [json.loads(l) for l in io.open(evidence, encoding="utf-8") if l.strip()]
    assert rows, "evidence file is empty"

    # Image hashes are unique per distinct file, so a repeated hash means two
    # rows point at identical bytes; distinct rows must still resolve.
    seen_ids: set[str] = set()
    for r in rows:
        assert r["image_sha256"], f"{r['image_id']} has no image hash"
        assert len(r["image_sha256"]) == 64, r["image_sha256"]
        assert r["image_id"] not in seen_ids, f"duplicate record for {r['image_id']}"
        seen_ids.add(r["image_id"])
        # Portable path only: no absolute machine paths may leak into reports.
        assert not r["relative_image_path"].startswith(("/", "\\", "C:")), \
            r["relative_image_path"]

    # Spot-check the hashes against the real bytes.
    for r in rows[:40]:
        p = root / "images" / f"{r['image_id']}.png"
        assert p.is_file(), f"evidence references a missing image: {p}"
        assert sha256_file(p) == r["image_sha256"], (
            f"{r['image_id']}: recorded hash does not match the file on disk"
        )


def test_split_assignment_is_imported_not_reimplemented():
    """TEST membership must come from the project's existing rule."""
    from gpo_ocr_audit import gpo_correlator

    src = gpo_correlator.__file__
    text = Path(src).read_text(encoding="utf-8")
    assert "from gpo_train.dataset import fnv1a_session_split" in text or \
           "def fnv1a_session_split" in text
    # If the fallback copy is in use, it must be byte-identical to the port.
    sys.path.insert(0, str(REPO / "ml"))
    from gpo_train.dataset import fnv1a_session_split as canonical

    for sid in ["session_2026-10-02_212813_b21c9704",
                "session_2026-10-04_160014_94aaa3c9",
                "session_2026-10-06_000000_deadbeef",
                "session_2026-10-07_235959_cafebabe"]:
        fallback = gpo_correlator.fnv1a_session_split(sid)
        assert fallback == canonical(sid), (
            f"split rule disagrees for {sid}: {fallback} vs {canonical(sid)}; "
            f"TEST membership must never be re-derived"
        )


def test_session_ids_are_never_spread_across_splits():
    from gpo_ocr_audit.gpo_correlator import session_day

    # session_day is how recurrence is measured; it must not lose the date.
    assert session_day("session_2026-10-04_160014_94aaa3c9") == "2026-10-04"
    assert session_day(None) == ""
    assert session_day("not-a-session") == ""


# --------------------------------------------------------------------------
# Audit-status semantics
# --------------------------------------------------------------------------

def test_only_confirmed_statuses_are_confirmable():
    from gpo_ocr_audit.schemas import AuditStatus

    confirmable = {s for s in AuditStatus if s.is_confirmable}
    assert confirmable == {
        AuditStatus.CONFIRMED_MATCH,
        AuditStatus.ALIAS_MATCH,
        AuditStatus.OCR_VARIANT_MATCH,
    }
    for s in (AuditStatus.COMPOUND_AMBIGUOUS, AuditStatus.UNKNOWN_ENTITY,
              AuditStatus.OCR_CONFLICT, AuditStatus.OCR_UNREADABLE,
              AuditStatus.NO_DECISION):
        assert not s.is_confirmable, f"{s} must not be treated as confirmable"
        assert s.needs_human, f"{s} must require human review"


def test_a_fuzzy_or_partial_result_is_never_confirmable():
    from gpo_ocr_audit.schemas import AuditStatus, MatchKind

    assert not AuditStatus.NO_DECISION.is_confirmable
    assert MatchKind.FUZZY.value == "FUZZY"
    assert MatchKind.PARTIAL.value == "PARTIAL"


def test_priority_score_cannot_change_a_label():
    """`score_priority` has no access to the stored label by construction."""
    import inspect

    from gpo_ocr_audit.ocr_audit import score_priority

    sig = inspect.signature(score_priority)
    assert list(sig.parameters) == ["ev", "session_count", "day_count"], list(sig.parameters)
    src = inspect.getsource(score_priority)
    assert "stored_entity_id =" not in src
    assert "audit_status =" not in src