"""Tests for ontology matching and the compound analyzer.

The behaviour under test is the one that matters most in this project: a name
that maps to zero or several entities is never resolved. Every "must not resolve"
test here corresponds to a mistake the project has already made once.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from gpo_ocr_audit.ontology_matcher import (  # noqa: E402
    Ontology,
    load_ontology,
)
from gpo_ocr_audit.schemas import AuditStatus, MatchKind, OntologyStatus  # noqa: E402
from gpo_ocr_audit.text_normalizer import normalize_pipeline  # noqa: E402


def _match(ont: Ontology, raw: str):
    norm, _steps, _applied, _folded = normalize_pipeline(raw)
    return ont.match(raw, norm)


def _ont() -> Ontology:
    return load_ontology()


# --------------------------------------------------------------------------
# The exact cases the audit exists to get right
# --------------------------------------------------------------------------

def test_crimson_snapper_yields_both_entities_and_is_ambiguous():
    """`Crimson Snapper` names two canonical entities and must choose neither."""
    for raw in ["New Item <Crimson Snapper>", "Nev Item<crimsonSnapper>",
                "You caught a Crimson Snapper!"]:
        m = _match(_ont(), raw)
        assert m.kind == MatchKind.AMBIGUOUS, (raw, m.kind)
        assert m.status == OntologyStatus.COMPOUND, (raw, m.status)
        assert set(m.candidates) == {"fish:crimson", "fish:snapper"}, (raw, m.candidates)
        assert not m.matched, "a compound name must never resolve to one entity"


def test_concatenated_compound_is_still_detected():
    """The game often renders it with no space at all."""
    m = _match(_ont(), "Nev Item<crimsonSnapper>")
    assert set(m.candidates) == {"fish:crimson", "fish:snapper"}, m.candidates
    # Both are flagged as fragments, not whole words - which is why they cannot
    # be treated as a clean single-entity match.
    assert all(not t.whole_word for t in m.compound_tokens), m.compound_tokens


def test_candy_corn_squid_does_not_resolve_to_squid():
    """`squid` is present but `candy corn` is not in the KB, so the name is
    only partly explained. Resolving it to fish:squid is the error under test."""
    m = _match(_ont(), "New Item <Candy Corn Squid>")
    assert m.status == OntologyStatus.UNKNOWN_ENTITY, (m.status, m.reason)
    assert not m.matched, f"'candy corn' is unknown; must not match {m.matched}"
    assert "fish:squid" in {t.entity_id for t in m.compound_tokens}


def test_tigerfin_does_not_resolve_to_tiger():
    """`tiger` occurs inside `tigerfin`. That is a fragment, not a match."""
    for raw in ["New Item <Tigerfin>", "New Item <Exotic Tigerfin>",
                "Nev Item<Exotic Tigerfinz>"]:
        m = _match(_ont(), raw)
        assert m.matched == [], (raw, m.matched)
        assert m.kind in (MatchKind.PARTIAL, MatchKind.NONE), (raw, m.kind)
        assert m.status == OntologyStatus.UNKNOWN_ENTITY, (raw, m.status)


def test_dark_skeletal_shark_is_ambiguous_not_shark():
    """`skeletal` is unknown, but `dark` IS a KB fruit and `shark` is a KB fish."""
    m = _match(_ont(), "New Item <Dark Skeletal Shark>")
    assert m.status == OntologyStatus.COMPOUND, (m.status, m.reason)
    assert set(m.candidates) == {"fish:shark", "fruit:dark"}, m.candidates
    assert not m.matched


def test_golden_ribbon_angelfish_is_ambiguous():
    m = _match(_ont(), "New Item <Golden Ribbon Angelfish>")
    assert m.kind == MatchKind.AMBIGUOUS
    assert set(m.candidates) == {"fish:angelfish", "fish:golden"}, m.candidates


def test_golden_polka_puffer_is_partly_explained():
    """`golden` is known; `polka puffer` is not."""
    m = _match(_ont(), "New Item <Golden Polka Puffer>")
    assert m.status == OntologyStatus.UNKNOWN_ENTITY
    assert not m.matched


def test_blue_lip_grouper_is_unknown():
    m = _match(_ont(), "New Item <Blue-Lip Grouper>")
    assert m.status == OntologyStatus.UNKNOWN_ENTITY
    assert not m.matched and not m.candidates


def test_banner_template_is_never_an_entity_candidate():
    """`ui:new-item` is furniture. If it leaked in, every drop would read as
    ambiguous between a fish and a UI term."""
    for raw in ["New Item <Swordfish>", "Nev Item<crimsonSnapper>"]:
        m = _match(_ont(), raw)
        assert all(not c.startswith("ui:") for c in m.candidates), m.candidates
        assert "ui:new-item" in m.ui_templates, m.ui_templates


def test_a_clean_single_name_matches_exactly():
    for raw, want in [
        ("New Item <Swordfish>", "fish:swordfish"),
        ("New Item <Pufferfish>", "fish:pufferfish"),
        ("New Item <Snapper>", "fish:snapper"),
    ]:
        m = _match(_ont(), raw)
        assert m.matched == [want], (raw, m.matched)
        assert m.kind == MatchKind.EXACT, (raw, m.kind)
        assert m.status == OntologyStatus.EXACT_ENTITY


def test_confusable_match_is_never_reported_as_exact():
    """`5hark` really is shark, but the pixels never said so."""
    m = _match(_ont(), "5hark")
    assert m.matched == ["fish:shark"]
    assert m.kind == MatchKind.CONFUSABLE, m.kind
    assert m.kind != MatchKind.EXACT
    assert m.status == OntologyStatus.CONFUSABLE_ENTITY


def test_a_pure_ui_banner_is_non_entity():
    m = _match(_ont(), "Line broke!")
    assert m.status == OntologyStatus.UI_TEMPLATE
    assert not m.matched and not m.candidates
    assert "ui:line-broke" in m.ui_templates


def test_empty_text_is_unreadable():
    m = _match(_ont(), "")
    assert m.status == OntologyStatus.UNREADABLE


def test_longest_span_wins_for_substring_collisions():
    """`golden` must beat `gold` (fruit:goru); `common bait` must beat `common`."""
    m = _match(_ont(), "New Item <Golden>")
    assert m.matched == ["fish:golden"], m.matched
    m2 = _match(_ont(), "New Item <Common Bait>")
    assert m2.matched == ["bait:common"], m2.matched


def test_category_is_reported_for_each_candidate():
    """A FRUIT candidate inside a fish drop is evidence the label is compound."""
    m = _match(_ont(), "New Item <Dark Skeletal Shark>")
    assert m.categories.get("fruit:dark") == "fruit"
    assert m.categories.get("fish:shark") == "fish"


def test_fuzzy_candidates_are_suggestions_only():
    """A fuzzy hit must never appear in `matched`."""
    m = _match(_ont(), "New Item <Swoffish>")
    if m.kind == MatchKind.FUZZY:
        assert not m.matched
        assert m.fuzzy
        assert m.status == OntologyStatus.FUZZY_CANDIDATE


def test_ontology_matches_the_canonical_vocabulary_file():
    """The export must agree with FISH_VOCAB.txt, not restate it differently."""
    root = Path(__file__).resolve().parents[3]
    vocab = []
    for line in (root / "FISH_VOCAB.txt").read_text(encoding="utf-8").splitlines():
        t = line.strip()
        if t and not t.startswith("#"):
            vocab.append(t)
    assert _ont().fish_vocab == vocab, (
        "kb_export.json's fish_vocab diverges from FISH_VOCAB.txt; the exporter "
        "and the canonical file must not disagree"
    )


def test_kb_contains_no_fish_aliases_so_alias_stage_is_honest():
    """All 41 fish entities have no aliases, so an ALIAS_MATCH can never occur
    for a fish. Asserting this keeps a future alias addition honest."""
    ont = _ont()
    fish_with_alias = [
        e.id for e in ont.entities
        if e.category == "fish" and (e.aliases or e.ocr_aliases)
    ]
    assert fish_with_alias == [], (
        f"fish entities gained aliases: {fish_with_alias}; the audit's alias "
        f"stage now applies to fish and needs re-review"
    )


def test_status_classification_precedence():
    """Ambiguity must outrank conflict: a compound has no comparable label."""
    from gpo_ocr_audit.ocr_audit import classify, is_entity_frame

    class Row:
        region_name = "drop"
        game_state = "catch_result"
        entity_id = "fish:snapper"

    ont = _ont()
    m = _match(ont, "New Item <Crimson Snapper>")
    assert classify(Row(), m, "fish:snapper", None) == AuditStatus.COMPOUND_AMBIGUOUS
    assert is_entity_frame(Row())

    m2 = _match(ont, "New Item <Swordfish>")
    assert classify(Row(), m2, "fish:swordfish", None) == AuditStatus.CONFIRMED_MATCH

    class BadRow(Row):
        entity_id = "fish:shark"
    assert classify(BadRow(), m2, "fish:shark", None) == AuditStatus.OCR_CONFLICT


def test_non_drop_frames_never_confirm_an_entity():
    """A fishing-bar crop showing an entity name cannot confirm a drop label."""
    from gpo_ocr_audit.ocr_audit import classify, is_entity_frame

    class BarRow:
        region_name = "bar"
        game_state = "waiting_for_bite"
        entity_id = None

    ont = _ont()
    assert not is_entity_frame(BarRow())
    m = _match(ont, "Shark")
    assert classify(BarRow(), m, None, None) == AuditStatus.NO_DECISION