"""Tests for ontology matching and the compound analyzer.

The behaviour under test is the one that matters most in this project: a name
that maps to zero or several entities is never resolved. Every "must not resolve"
test here corresponds to a mistake the project has already made once.

Post KB-patch (fish_kb_patch.json, applied through the overlay): the sixteen
documented, human-confirmed fish resolve to their whole-name entities. What has
NOT changed: garbled or concatenated renderings still fail closed, and no name
ever resolves to a bare stem (`fish:squid`, `fish:shark`, `fish:tiger`, ...).
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

def test_crimson_snapper_resolves_to_the_whole_name_entity():
    """Post KB-patch, `Crimson Snapper` is one documented, human-confirmed entity.

    The safety property the old ambiguous-world test guarded survives unchanged:
    the name must NEVER resolve to either stem (`fish:crimson` / `fish:snapper`).
    What changed is only that it now has a legitimate whole-name home."""
    for raw in ["New Item <Crimson Snapper>", "Nev Item <Crimson Snapper>"]:
        m = _match(_ont(), raw)
        assert m.matched == ["fish:crimsonsnapper"], (raw, m.matched)
        assert m.kind == MatchKind.EXACT, (raw, m.kind)
        assert m.status == OntologyStatus.EXACT_ENTITY, (raw, m.status)
        assert "fish:crimson" not in m.matched and "fish:snapper" not in m.matched


def test_concatenated_compound_is_still_detected():
    """The game often renders it with no space at all. The whole-name entity's
    canonical form carries the space, so the garbled rendering still cannot
    reach it - it decomposes into the two stems and stays unresolved."""
    m = _match(_ont(), "Nev Item<crimsonSnapper>")
    assert set(m.candidates) == {"fish:crimson", "fish:snapper"}, m.candidates
    assert not m.matched, "a garbled compound must never resolve"
    # Both are flagged as fragments, not whole words - which is why they cannot
    # be treated as a clean single-entity match.
    assert all(not t.whole_word for t in m.compound_tokens), m.compound_tokens


def test_candy_corn_squid_does_not_resolve_to_squid():
    """Resolving the name to the bare stem `fish:squid` remains the error under
    test. Post-patch it has a whole-name entity and resolves there instead."""
    m = _match(_ont(), "New Item <Candy Corn Squid>")
    assert m.matched == ["fish:candycornsquid"], m.matched
    assert "fish:squid" not in m.matched
    assert m.kind == MatchKind.EXACT


def test_tigerfin_does_not_resolve_to_tiger():
    """`tiger` occurs inside `tigerfin`. That is a fragment, not a match.
    Post-patch the whole names have their own entities; the fragment still
    loses, and a GARBLED rendering still fails closed."""
    m = _match(_ont(), "New Item <Tigerfin>")
    assert m.matched == ["fish:tigerfin"], m.matched
    m = _match(_ont(), "New Item <Exotic Tigerfin>")
    assert m.matched == ["fish:exotictigerfin"], m.matched
    m = _match(_ont(), "Nev Item<Exotic Tigerfinz>")
    assert m.matched == [], m.matched
    assert m.status == OntologyStatus.UNKNOWN_ENTITY, m.status
    assert "fish:tiger" not in (m.matched or [])


def test_dark_skeletal_shark_resolves_wholesale_not_to_shark():
    """Post-patch the documented shark has its own entity; `fish:shark` must
    still never win."""
    m = _match(_ont(), "New Item <Dark Skeletal Shark>")
    assert m.matched == ["fish:darkskeletalshark"], m.matched
    assert m.kind == MatchKind.EXACT
    assert "fish:shark" not in m.matched


def test_golden_ribbon_angelfish_resolves_wholesale():
    m = _match(_ont(), "New Item <Golden Ribbon Angelfish>")
    assert m.matched == ["fish:goldenribbonangelfish"], m.matched
    assert m.kind == MatchKind.EXACT
    assert "fish:angelfish" not in m.matched and "fish:golden" not in m.matched


def test_golden_polka_puffer_resolves_wholesale():
    m = _match(_ont(), "New Item <Golden Polka Puffer>")
    assert m.matched == ["fish:goldenpolkapuffer"], m.matched
    assert m.kind == MatchKind.EXACT


def test_blue_lip_grouper_resolves_wholesale():
    m = _match(_ont(), "New Item <Blue-Lip Grouper>")
    assert m.matched == ["fish:bluelipgrouper"], m.matched
    assert m.kind == MatchKind.EXACT


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
    """A FRUIT candidate inside a fish drop is evidence the label is compound.
    (`Dark Pufferfish` is not a documented fish, so it still decomposes.)"""
    m = _match(_ont(), "New Item <Dark Pufferfish>")
    assert m.categories.get("fruit:dark") == "fruit"
    assert m.categories.get("fish:pufferfish") == "fish"


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
    """No fish entity has aliases, so an ALIAS_MATCH can never occur for a
    fish. Asserting this keeps a future alias addition honest. (The KB-patch
    additions deliberately carry none, exactly so this stays true.)"""
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
    # The garbled concatenated form still cannot resolve, so a stored stem label
    # against it is still compound-ambiguous - ambiguity outranks conflict.
    m = _match(ont, "Nev Item<crimsonSnapper>")
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