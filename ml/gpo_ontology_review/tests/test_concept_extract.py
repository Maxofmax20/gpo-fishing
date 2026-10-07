"""Tests for concept grouping, variant grouping and decomposition.

The behaviour under test is what keeps the human's workload honest: 411 raw OCR
strings reduce to a short list of game names WITHOUT merging two different names,
and WITHOUT inventing any name the game did not produce.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from gpo_ontology_review.concept_extract import (  # noqa: E402
    MAX_NAME_TOKENS,
    decompose,
    extract_concept_name,
    family_for,
    group_key,
    looks_like_ui,
    priority_for,
    strip_banner_residue,
)

UI = ["new item", "check your backpack", "you caught", "line broke", "got away"]


def _name(raw: str) -> str | None:
    return extract_concept_name(raw, UI)[0]


# --------------------------------------------------------------------------
# Grouping: merge RENDERINGS, never different names
# --------------------------------------------------------------------------

def test_spacing_variants_of_one_name_group_together():
    """The OCR sometimes drops a space. That is a rendering difference, not a
    different name."""
    assert group_key("crimson snapper") == group_key("crimsonsnapper")
    assert group_key("candy corn squid") == group_key("candycornsquid")
    assert group_key("skeletal shark") == group_key("skeletalshark")
    assert group_key("golden ribbon angelfish") == group_key("goldenribbonangelfish")


def test_different_names_never_merge():
    """THE property. `tigerfin`, `exotic tigerfin` and `golden tigerfin` may be
    three different entities, so they must stay three separate concepts."""
    keys = {group_key(n) for n in
            ("tigerfin", "exotic tigerfin", "golden tigerfin", "golden tigerfinz")}
    assert len(keys) == 4, keys
    # And modifiers never leak into one another's group.
    assert group_key("crimson snapper") != group_key("crimson puffer")
    assert group_key("golden polka puffer") != group_key("golden ribbon angelfish")


def test_group_key_preserves_letter_order():
    """Removing spaces must not reorder words: `angelfish golden` is a different
    sequence of letters from `golden angelfish`."""
    assert group_key("golden angelfish") != group_key("angelfish golden")
    assert group_key("crimson snapper") != group_key("snapper crimson")
    # And a spaced name DOES equal its concatenation, which is the whole point of
    # the key: `crimson snapper` and `crimsonsnapper` are one concept.
    assert group_key("crimson snapper") == group_key("crimsonsnapper")


# --------------------------------------------------------------------------
# Banner residue
# --------------------------------------------------------------------------

def test_banner_residue_is_stripped_and_reported():
    for raw, expected in [
        ("ne item crimson snapper", "crimson snapper"),
        ("nevitemskeletalshark", "skeletalshark"),
        ("new item candy corn squid", "candy corn squid"),
        ("newitemskeletalshark", "skeletalshark"),
    ]:
        assert _name(raw) == expected, (raw, _name(raw))


def test_removal_is_reported_not_silent():
    _n, notes = extract_concept_name("ne item crimson snapper", UI)
    assert notes["banner_removed"], notes


def test_stripped_residue_is_only_template_wording():
    """The removed words must be template words, never part of a name. If a rule
    ever ate a real name's first word, the name would silently change."""
    residue_words = {"item", "items", "new", "nev", "ne", "itern"}
    name, notes = extract_concept_name("ne item golden tigerfin", UI)
    for frag in notes["banner_removed"]:
        for w in frag.split():
            assert w in residue_words, (
                f"banner residue rule removed {w!r}, which is not template wording"
            )


# --------------------------------------------------------------------------
# Bracket-glyph artefact
# --------------------------------------------------------------------------

def test_trailing_z_is_repaired_on_multi_token_names():
    """The bracket artefact appears mostly on compound names, where removing it
    cannot damage a real word."""
    assert _name("zebra ribbon angelfishz") == "zebra ribbon angelfish"
    assert _name("crimson polka pufferz") == "crimson polka puffer"
    assert _name("dark skeletal sharkz") == "dark skeletal shark"


def test_a_real_trailing_z_word_is_never_damaged():
    """Regression test. A blanket trailing-`z` rule turned `quiz` into `qui` and
    `fizz` into `fiz` - real corruption of real names."""
    for word in ("quiz", "fizz", "puzzle", "fuzz", "jazz", "fez"):
        assert _name(word) == word, f"{word!r} was damaged to {_name(word)!r}"


def test_a_single_token_artefact_stays_a_separate_concept():
    """`tigerfinz` is one token, so it is NOT repaired - and that is deliberate.
    The alternative damages `quiz`."""
    assert _name("tigerfinz") == "tigerfinz"


# --------------------------------------------------------------------------
# The rejected leading-junk heuristic
# --------------------------------------------------------------------------

def test_no_rule_eats_the_first_letter_of_a_real_name():
    """Regression test for a bug that corrupted the whole concept list.

    An earlier "strip a stray leading character" rule matched EVERY name and turned
    `zebra ribbon angelfish` into `ebra ribbon angelfish`, `crimson snapper` into
    `rimson snapper` and `swordfish` into `wordfish`. That single rule destroyed the
    concept list a human has to rule on.
    """
    for name in ("zebra ribbon angelfish", "crimson snapper", "swordfish",
                 "golden tigerfin", "dark skeletal shark", "candy corn squid",
                 "exotic tigerfin", "blue lip grouper", "tigerfin", "fangfish"):
        assert _name(name) == name, f"{name!r} was mangled to {_name(name)!r}"


def test_a_leading_s_artefact_stays_a_separate_concept():
    """The stray prefixes are real, but separating them is the honest outcome:
    they are visibly odd and a human sees them immediately."""
    assert _name("sgolden tigerfin") == "sgolden tigerfin"
    assert _name("scrimson polka puffer") == "scrimson polka puffer"


# --------------------------------------------------------------------------
# System text must not become a concept
# --------------------------------------------------------------------------

def test_long_announcement_text_is_not_a_concept():
    """The Seeing Eye bait banner, hub spawn notices and despawn timers are real
    game text but are not entity names."""
    for banner in [
        "eeing eye you drop checkyourbackpa pity 3 40 legendary",
        "a kilo has at trading hub",
        "ropped bomb vill despavn in 10 minutes",
        "you can only store one of each fruit dropped mero vill despavn in 10 minutes",
        "all seeing eye a devilfruit drop legendary pity 23 40 youcanonlystoreoneof "
        "eachfruit droppedbombvilldespavnin10 minutes",
    ]:
        assert _name(banner) is None, f"{banner!r} became a concept"


def test_real_names_survive_the_length_filter():
    for name in ("swordfish", "tigerfin", "crimson snapper", "crimson polka puffer",
                 "dark skeletal shark", "golden ribbon angelfish",
                 "zebra ribbon angelfish", "blue lip grouper", "jack o bite"):
        assert len(name.split()) <= MAX_NAME_TOKENS
        assert _name(name) == name, (name, _name(name))


# --------------------------------------------------------------------------
# Decomposition: linguistic evidence only
# --------------------------------------------------------------------------

def test_decompose_splits_modifier_from_base():
    assert decompose("crimson snapper") == ("snapper", ["crimson"])
    assert decompose("golden ribbon angelfish") == ("angelfish", ["golden", "ribbon"])
    assert decompose("crimson polka puffer") == ("puffer", ["crimson", "polka"])
    assert decompose("dark skeletal shark") == ("shark", ["dark", "skeletal"])
    assert decompose("golden tigerfin") == ("tigerfin", ["golden"])
    assert decompose("swordfish") == ("swordfish", [])


def test_decompose_drops_trailing_function_words():
    assert decompose("king of the shark") == ("shark", ["king", "of", "the"])


def test_family_is_keyed_on_the_base_word():
    assert family_for("tigerfin", "exotic tigerfin") == family_for("tigerfin", "tigerfin")
    assert family_for("tigerfin", "golden tigerfin") == "family:tigerfin"
    # Different bases never share a family.
    assert family_for("snapper", "crimson snapper") != family_for("squid", "candy corn squid")


def test_family_exists_even_without_a_base():
    assert family_for(None, "something").startswith("family:")


# --------------------------------------------------------------------------
# Priority bands
# --------------------------------------------------------------------------

def test_priority_bands_match_the_specified_thresholds():
    assert priority_for(120, 12, 4)[0] == "P0"
    assert priority_for(100, 10, 3)[0] == "P0"
    assert priority_for(99, 10, 3)[0] == "P1"      # misses the occurrence floor
    assert priority_for(120, 9, 3)[0] == "P1"      # misses the session floor
    assert priority_for(30, 6, 1)[0] == "P1"
    assert priority_for(5, 1, 1)[0] == "P2"
    assert priority_for(4, 1, 1)[0] == "P3"
    assert priority_for(1, 1, 1)[0] == "P3"


def test_priority_never_decreases_with_evidence():
    """More evidence must never mean lower priority."""
    weak = int(priority_for(30, 6, 2)[0][1])
    strong = int(priority_for(300, 40, 5)[0][1])
    assert strong <= weak, (strong, weak)


def test_ui_detection_needs_two_phrases_or_a_long_string():
    assert looks_like_ui("new item check your backpack line broke", UI)
    assert not looks_like_ui("crimson snapper", UI)


def test_priority_scorer_cannot_see_a_label():
    """Ordering only. `priority_for` is not given a label, so it cannot use one."""
    import inspect

    params = list(inspect.signature(priority_for).parameters)
    assert params == ["occurrences", "sessions", "days"], params