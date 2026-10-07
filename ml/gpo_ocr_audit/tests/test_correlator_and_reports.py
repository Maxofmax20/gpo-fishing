"""Tests for the dataset correlation layer and report aggregation.

Regression tests for two bugs found by running the audit against the real data:

* the shipped model is **NCHW** (`pixels: [batch, 3, 96, 96]`) but the pipeline
  fed it NHWC, and a bare ``except`` hid the error. The report then said
  "0 rows carry a model prediction" as though that were a finding.
* the stored-vs-fresh OCR comparison normalised one side and not the other,
  reporting **0.0%** agreement - a fabricated result that looked like data.

Both tests here would have failed before the fix.
"""

from __future__ import annotations

import inspect
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from gpo_ocr_audit import gpo_correlator  # noqa: E402
from gpo_ocr_audit.gpo_correlator import ModelRunner, session_day  # noqa: E402

REPO = Path(__file__).resolve().parents[3]
MODELS = REPO / "src-tauri" / "models"


# --------------------------------------------------------------------------
# Model preprocessing: the layout bug
# --------------------------------------------------------------------------

def test_model_preprocessing_is_nchw_matching_the_graph():
    """The exported graph declares the input; the preprocessing must match it."""
    runner = ModelRunner.load(MODELS, "fish_v1")
    if runner is None or not runner.available:
        print("SKIP: shipped model unavailable")
        return

    declared = runner._session.get_inputs()[0].shape
    # `['batch', 3, 96, 96]` -> channels-first.
    assert declared[1] == 3, f"unexpected graph input layout: {declared}"
    assert declared[2] == declared[3], f"non-square input: {declared}"

    mean = runner._mean
    std = runner._std
    assert mean.shape == (1, 3, 1, 1), f"mean is not NCHW-shaped: {mean.shape}"
    assert std.shape == (1, 3, 1, 1), f"std is not NCHW-shaped: {std.shape}"
    assert float(std.min()) > 0.0, "a zero std would divide by zero"


def test_a_prediction_failure_is_counted_not_swallowed():
    """The anti-pattern this guards: a broken model column must be loud.

    When the layout was wrong, every prediction raised InvalidArgument and a bare
    `except Exception` returned `(None, None)`. The audit then reported an empty
    model column with no explanation.
    """
    runner = ModelRunner.load(MODELS, "fish_v1")
    if runner is None or not runner.available:
        print("SKIP: shipped model unavailable")
        return

    before = runner.prediction_failures
    # A path that cannot be opened must increment the counter and record why.
    pred, conf = runner.predict(Path("definitely-missing-image.png"))
    assert pred is None and conf is None
    assert runner.prediction_failures == before + 1, (
        "a prediction failure must be counted so an empty model column is "
        "distinguishable from a model that had nothing to say"
    )
    assert runner.last_prediction_error, "the failure reason must be recorded"

    # And it must surface in provenance, which is what the report is built from.
    prov = runner.provenance()
    assert prov["prediction_failures"] == runner.prediction_failures
    assert prov["last_prediction_error"] == runner.last_prediction_error


def test_predict_never_raises():
    """A bad image must not abort a 15k-image run."""
    runner = ModelRunner.load(MODELS, "fish_v1")
    if runner is None:
        print("SKIP: shipped model unavailable")
        return
    # Even with no session at all, predict must return rather than raise.
    runner._session = None
    assert runner.predict(Path("x.png")) == (None, None)


# --------------------------------------------------------------------------
# Stored-vs-fresh comparison: both sides must be normalised the same way
# --------------------------------------------------------------------------

def test_stored_and_fresh_text_are_normalised_identically():
    """A one-sided normalisation fabricates a disagreement.

    `New Item <Shark>` (fresh) and `Nev Item<Shark>` (stored) are the same
    reading. Comparing a normalised string against a merely lowercased one made
    the audit report 0.0% agreement across 3,327 rows - a number that looked
    like a finding and was pure bug.
    """
    from gpo_ocr_audit.text_normalizer import normalize_pipeline

    stored = "Nev Item<Shark>"
    fresh = "New Item <Shark>"
    assert normalize_pipeline(stored)[0] == normalize_pipeline(fresh)[0], (
        "both sides of the comparison must go through the same chain"
    )

    # And the aggregation must use that function, not a hand-rolled lower() call.
    src = inspect.getsource(
        __import__("gpo_ocr_audit.report_generator", fromlist=["summarise"]).summarise
    )
    assert "normalize_pipeline" in src, (
        "summarise must normalise BOTH sides; a bare lower() on one side "
        "manufactures disagreement"
    )


def test_normalization_makes_the_measured_disagreement_visible():
    """Sanity: the pipeline's normalisation must actually reduce real noise.

    Scope note: only renderings a NAMED rule covers are repaired. A word mangled
    in eight different places (`csndyucom`) is deliberately left alone - inventing
    a rule for it would be deriving a correction from frequency, which is exactly
    what the task forbids. It stays UNKNOWN, which is the honest outcome.
    """
    from gpo_ocr_audit.text_normalizer import normalize_pipeline

    repairable = [
        ("Nev Item <crimson snapper*", "New Item <Crimson Snapper>"),
        ("uev.ltem•zebra Ribbon AnseLfish>", "Nev Item <zebra Ribbon Angelfish"),
        ("Nev Item<Candy Corn Squid>", "New Item <Candy Corn Squid>"),
        ("Nev Item csvordfish>", "New Item <Swordfish>"),
    ]
    for a, b in repairable:
        na = normalize_pipeline(a)[0]
        nb = normalize_pipeline(b)[0]
        assert na == nb, f"{a!r} -> {na!r} but {b!r} -> {nb!r}"

    # Heavily mangled input is NOT silently repaired into a known name.
    mangled = normalize_pipeline("<CSndyuCom Squid*")[0]
    assert mangled != "new item candy corn squid", (
        "a rule must not exist that reconstructs this from guesswork"
    )
    assert "squid" in mangled, mangled


def test_mangled_words_stay_unknown_rather_than_being_invented():
    """The pipeline's answer to garbage is UNKNOWN_ENTITY, not a best guess."""
    from gpo_ocr_audit.ontology_matcher import load_ontology
    from gpo_ocr_audit.schemas import MatchKind, OntologyStatus
    from gpo_ocr_audit.text_normalizer import normalize_pipeline

    ont = load_ontology()
    norm = normalize_pipeline("New Item <CSndyuCom Squid>")[0]
    m = ont.match("New Item <CSndyuCom Squid>", norm)
    # `squid` may or may not surface as a fragment; what must not happen is a
    # confident single-entity resolution presented as a match.
    assert m.matched == [], m.matched
    assert m.kind in (MatchKind.PARTIAL, MatchKind.NONE), m.kind
    assert m.status == OntologyStatus.UNKNOWN_ENTITY, m.status


# --------------------------------------------------------------------------
# Session / day derivation
# --------------------------------------------------------------------------

def test_session_day_extraction():
    assert session_day("session_2026-10-04_160014_94aaa3c9") == "2026-10-04"
    assert session_day("session_2026-10-06_235959_cafebabe") == "2026-10-06"
    assert session_day("") == ""
    assert session_day(None) == ""
    assert session_day("session") == ""


def test_correlator_exposes_no_write_capability():
    """The module that joins metadata must not be able to change any of it."""
    forbidden = ("def write", "def save", "def update", "def delete", "def insert",
                 "def set_", "training_eligible =", ".unlink(")
    src = inspect.getsource(gpo_correlator)
    for bad in forbidden:
        assert bad not in src, f"gpo_correlator contains {bad!r}; it must be read-only"


# --------------------------------------------------------------------------
# Report aggregation arithmetic
# --------------------------------------------------------------------------

def test_summary_counts_add_up():
    """Every per-status count must sum to the total, or a report is lying."""
    import tempfile

    from gpo_ocr_audit.report_generator import summarise
    from gpo_ocr_audit.schemas import EntityEvidence

    evs = []
    for i in range(7):
        evs.append(EntityEvidence(
            image_id=f"i{i}", image_sha256="0" * 64, relative_image_path="images/i.png",
            session_id="session_2026-10-04_160014_94aaa3c9", timestamp_ms=1,
            stored_entity_id="fish:shark", stored_source="collector", stored_ocr_text=None,
            ocr_engine="e", ocr_engine_version="1", raw_ocr="New Item <Shark>",
            normalized_ocr="new item shark", ocr_confidence=0.8,
            match_kind="EXACT", ontology_status="EXACT_ENTITY",
            residual_text="shark", audit_status="CONFIRMED_MATCH",
            image_category="FISH_DROP", split="train",
        ))
    s = summarise(evs)
    assert sum(s["by_status"].values()) == s["total"], s["by_status"]
    assert sum(s["by_category"].values()) == s["total"], s["by_category"]
    assert s["total"] == 7
    assert s["ocr_success"] == 7
    assert s["by_status"]["CONFIRMED_MATCH"] == 7


def test_per_class_clean_is_a_strict_subset_of_rows():
    """`effective clean` must never exceed the row count, and must exclude
    compound/unknown/conflict rows by construction."""
    from gpo_ocr_audit.report_generator import per_class_quality
    from gpo_ocr_audit.schemas import EntityEvidence

    def ev(status: str) -> EntityEvidence:
        return EntityEvidence(
            image_id=status, image_sha256="", relative_image_path="",
            session_id="session_2026-10-02_212813_b21c9704", timestamp_ms=1,
            stored_entity_id="fish:snapper", stored_source="collector",
            stored_ocr_text=None, ocr_engine="e", ocr_engine_version="1",
            raw_ocr="", normalized_ocr="", ocr_confidence=None,
            match_kind="NONE", ontology_status="COMPOUND", audit_status=status,
            split="train",
        )

    rows = [
        ev("CONFIRMED_MATCH"), ev("ALIAS_MATCH"), ev("OCR_VARIANT_MATCH"),
        ev("COMPOUND_AMBIGUOUS"), ev("UNKNOWN_ENTITY"), ev("OCR_CONFLICT"),
        ev("OCR_UNREADABLE"), ev("NO_DECISION"),
    ]
    q = per_class_quality(rows, ["fish:snapper"])["fish:snapper"]
    assert q["total"] == 8
    assert q["effective_clean"] == 3, q
    assert q["needs_human"] == 5, q
    assert q["effective_clean"] <= q["total"]


def test_a_class_with_no_rows_is_reported_as_zero_not_missing():
    from gpo_ocr_audit.report_generator import per_class_quality

    vocab = ["fish:shark", "fish:swordfish"]
    q = per_class_quality([], vocab)
    assert set(q) == set(vocab)
    assert all(v["total"] == 0 and v["effective_clean"] == 0 for v in q.values())
    assert all(v["clean_share"] == "n/a" for v in q.values())


def test_search_index_roundtrips_a_query(tmp=None):
    """`Crimson Snapper` must be reachable from the index with its full context."""
    import sqlite3
    import tempfile

    from gpo_ocr_audit.report_generator import build_concepts, query_index, write_search_index
    from gpo_ocr_audit.schemas import EntityEvidence

    evs = [
        EntityEvidence(
            image_id=f"img{n}", image_sha256="0" * 64,
            relative_image_path=f"images/img{n}.png",
            session_id=f"session_2026-10-0{2 + n % 3}_160014_aaaaaaa",
            timestamp_ms=n, stored_entity_id="fish:snapper",
            stored_source="collector", stored_ocr_text="Nev Item <crimson snapper*",
            ocr_engine="e", ocr_engine_version="1",
            raw_ocr="New Item <crimson Snapper>", normalized_ocr="new item crimson snapper",
            ocr_confidence=0.84, match_kind="AMBIGUOUS", ontology_status="COMPOUND",
            residual_text="crimson snapper",
            candidate_entities=["fish:crimson", "fish:snapper"],
            audit_status="COMPOUND_AMBIGUOUS", image_category="FISH_DROP",
            split="train", review_priority=60,
        )
        for n in range(4)
    ]
    concepts = build_concepts(evs)
    with tempfile.TemporaryDirectory() as td:
        path = Path(td) / "idx.sqlite"
        write_search_index(path, evs, concepts)
        hits = query_index(path, "crimson snapper")
        assert len(hits["images"]) == 4, hits["images"]
        assert len(hits["concepts"]) == 1
        c = hits["concepts"][0]
        assert c["concept"] == "crimson snapper"
        assert c["occurrences"] == 4
        # The candidates must survive, so the human sees both parents.
        assert set(json.loads(c["candidates"])) == {"fish:crimson", "fish:snapper"}
        # And the stored label must still be visible for comparison.
        assert set(json.loads(c["stored_labels"])) == {"fish:snapper"}
        # Ordering: the compound rows rank above a clean confirmation.
        assert hits["images"][0]["review_priority"] >= 60


def test_priority_score_is_monotonic_in_severity():
    """More severe statuses must never score lower than less severe ones."""
    from gpo_ocr_audit.ocr_audit import score_priority
    from gpo_ocr_audit.schemas import EntityEvidence

    def mk(status: str) -> EntityEvidence:
        return EntityEvidence(
            image_id="i", image_sha256="", relative_image_path="", session_id="s",
            timestamp_ms=0, stored_entity_id=None, stored_source="collector",
            stored_ocr_text=None, ocr_engine="e", ocr_engine_version="1",
            raw_ocr="", normalized_ocr="", ocr_confidence=0.5,
            match_kind="NONE", ontology_status="COMPOUND", audit_status=status,
        )

    clean = score_priority(mk("CONFIRMED_MATCH"), 1, 1)
    unreadable = score_priority(mk("OCR_UNREADABLE"), 1, 1)
    conflict = score_priority(mk("OCR_CONFLICT"), 1, 1)
    unknown = score_priority(mk("UNKNOWN_ENTITY"), 1, 1)
    compound = score_priority(mk("COMPOUND_AMBIGUOUS"), 1, 1)

    # A clean confirmation still earns recurrence and confidence points, so it is
    # not literally zero - it must simply rank below every kind of trouble.
    assert clean >= 0
    assert clean < unreadable < conflict <= unknown <= compound, (
        f"severity ordering broke: clean={clean} unreadable={unreadable} "
        f"conflict={conflict} unknown={unknown} compound={compound}"
    )


def test_recurrence_raises_priority():
    """A concept seen in many sessions and days outranks a one-off sighting."""
    from gpo_ocr_audit.ocr_audit import score_priority
    from gpo_ocr_audit.schemas import EntityEvidence

    def mk() -> EntityEvidence:
        return EntityEvidence(
            image_id="i", image_sha256="", relative_image_path="", session_id="s",
            timestamp_ms=0, stored_entity_id=None, stored_source="collector",
            stored_ocr_text=None, ocr_engine="e", ocr_engine_version="1",
            raw_ocr="", normalized_ocr="", ocr_confidence=0.5,
            match_kind="NONE", ontology_status="UNKNOWN_ENTITY",
            audit_status="UNKNOWN_ENTITY",
        )

    once = score_priority(mk(), 1, 1)
    many = score_priority(mk(), 10, 6)
    assert many > once, (once, many)