"""Tests for the OCR normalisation layer.

Each test pins a behaviour the pipeline's conclusions depend on. In particular
the "repair text, never ontology" property is asserted explicitly, because that
is the boundary that separates a typo fix from inventing an entity.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from gpo_ocr_audit.text_normalizer import (  # noqa: E402
    OCR_CORRECTION_RULES,
    apply_corrections,
    normalize,
    normalize_pipeline,
    ocr_variant_fold,
    unicode_fold,
)


def test_csvordfish_becomes_swordfish():
    """An observed real corruption from this dataset."""
    out, steps, applied = apply_corrections("csvordfish")
    assert out == "swordfish", out
    assert "sordfish" in applied
    assert any(s.stage == "correction_rule" for s in steps)


def test_candy_com_squid_becomes_candy_corn_squid():
    """'rn' read as 'm' at this text size."""
    out, _, applied = apply_corrections("candy com squid")
    assert out == "candy corn squid", out
    assert "rn_as_m" in applied


def test_tiserfin_becomes_tigerfin():
    """OCR-text repair only. 'tigerfin' is still NOT a KB entity."""
    out, _, applied = apply_corrections("tiserfin")
    assert out == "tigerfin", out
    assert "tiqerfin" in applied


def test_every_correction_is_recorded_not_silent():
    """Normalisation must never quietly change text."""
    raw = "Nev Item<Candy Com Squid>"
    norm, steps, applied, _folded = normalize_pipeline(raw)
    assert norm == "new item candy corn squid", norm
    assert applied, "expected at least one rule to fire"
    # The first step always carries the verbatim input, so the chain replays.
    assert steps[0].stage == "raw"
    assert steps[0].frm == raw
    # Every step states a reason.
    assert all(s.reason for s in steps)
    # And the recorded steps really do compose back to the final text.
    replay = steps[0].frm
    for s in steps[1:]:
        assert s.frm == replay, f"step {s.stage!r} does not continue from the previous text"
        replay = s.to
    assert replay == norm, (replay, norm)


def test_corrections_never_splice_a_known_entity_out_of_an_unknown_name():
    """THE safety property.

    A correction rule may repair a corrupted token, but it must never turn an
    unfamiliar name into a familiar one. The original contamination happened
    exactly there, so every excluded concept is asserted to survive intact.
    """
    # Names the KB does NOT hold must come out unchanged (modulo the
    # confusable/banner repairs, which are separate stages).
    for unknown in ["tigerfin", "exotic tigerfin", "candy corn squid",
                    "dark skeletal shark", "blue lip grouper", "jack o bite",
                    "golden polka puffer", "zebra ribbon angelfish",
                    "golden ribbon angelfish"]:
        out, _steps, _applied = apply_corrections(unknown)
        assert out == unknown, (
            f"{unknown!r} was rewritten to {out!r}; a correction rule must not "
            f"resolve an unfamiliar concept"
        )


def test_no_rule_can_produce_a_known_entity_from_those_names():
    """Stronger form: the token set may not gain a KB fish name."""
    known = {"swordfish", "shark", "snapper", "squid", "angelfish", "golden",
             "crimson", "pufferfish", "tiger"}
    for unknown in ["tigerfin", "candy corn squid", "skeletal shark",
                    "golden polka puffer", "zebra ribbon angelfish"]:
        out, _s, _a = apply_corrections(unknown)
        for tok in out.split():
            assert tok not in known or tok in unknown.split(), (
                f"{unknown!r} -> {out!r} invented the known entity {tok!r}"
            )


def test_every_rule_documents_its_reason():
    """A rule with no stated failure mode is not acceptable."""
    for rule in OCR_CORRECTION_RULES:
        assert rule.name and rule.reason and rule.pattern
        assert len(rule.reason) > 20, f"{rule.name}: reason too vague to be testable"
        # Rules must be token-level: no whitespace inside a pattern, because a
        # multi-token pattern could SPLICE an entity name across a word gap.
        assert not any(ch.isspace() for ch in rule.pattern.pattern), (
            f"{rule.name}: pattern {rule.pattern.pattern!r} spans whitespace, so it "
            f"could join two tokens into an entity name"
        )
        # And never produce plural/possessive/derived forms that would imply a
        # separate entity out of one word.
        assert rule.replacement == rule.replacement.strip().lower()


def test_unicode_fold_handles_fullwidth_nbsp_and_accents():
    folded, _ = unicode_fold("Ｓｗｏｒｄ\u00a0ﬁsh")
    # Fullwidth -> ASCII and NBSP -> space.
    assert folded.startswith("Sword"), repr(folded)
    assert "\u00a0" not in folded
    # A decomposed accent must not survive into the key.
    assert unicode_fold("swordfish\u0301")[0] == "swordfish"


def test_normalize_decomposes_ligatures():
    """NFKC in `normalize` handles the `fi` ligature, which `unicode_fold` does
    not. The split is deliberate: fold is minimal and near-invertible."""
    out, _ = normalize("ﬁsh")
    assert out == "fish", out


def test_normalize_is_idempotent():
    """The Rust side relies on this too: re-normalising a key is a no-op."""
    for s in ["Nev Item<crimsonSnapper>", "GOLDEN  RIBBON!!", "  x  "]:
        once, _ = normalize(s)
        twice, _ = normalize(once)
        assert once == twice, (s, once, twice)


def test_confusable_fold_is_lossy_and_labelled_as_such():
    """The fold collapses i/l and o/0, so even a CLEAN name changes.

    That is exactly why a folded hit may never be reported as an exact match, and
    it is asserted here so the reason cannot be quietly deleted.
    """
    out, steps = ocr_variant_fold("5w0rdf15h")
    assert out == "swordflsh", out          # 'i' -> 'l', digits folded
    assert out != "swordfish"
    assert any("not injective" in s.reason or "lossy" in s.reason for s in steps)

    clean, steps2 = ocr_variant_fold("swordfish")
    assert clean == "swordflsh", clean     # lossy even without any digit
    assert steps2, "a clean name still changes, and that must be recorded"


def test_confusable_fold_is_a_noop_for_names_without_those_letters():
    out, steps = ocr_variant_fold("snapper")
    assert out == "snapper"
    assert steps == []


def test_pipeline_returns_a_usable_folded_key():
    _norm, _steps, _applied, folded = normalize_pipeline("Nev Item<CrimsonSnapper>")
    assert folded == "new ltem crlmsonsnapper", folded