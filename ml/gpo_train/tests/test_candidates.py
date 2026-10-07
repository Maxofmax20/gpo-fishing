"""Tests for candidate labelling.

The property that matters most is the negative one: a row the labeler cannot read
must never be turned into a confident claim. A wrong label is far more expensive
than a missing one, because a missing row costs a little data and a wrong one costs
a model that is confidently incorrect.
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from gpo_train.candidates import (  # noqa: E402
    CONFIDENT_AT,
    MIN_CONFIDENCE,
    Candidate,
    build_candidates,
    load_bestiary,
    norm_key,
    score_row,
    similarity,
    tier_for,
    write_candidates,
)

BESTIARY = [
    {"name": "Zebra Ribbon Angelfish", "matchKey": "zebraribbonangelfish", "rarity": "Rare"},
    {"name": "Crimson Snapper", "matchKey": "crimsonsnapper", "rarity": "Rare"},
    {"name": "Anglerfish", "matchKey": "anglerfish", "rarity": "Legendary"},
    {"name": "Exotic Tigerfin", "matchKey": "exotictigerfin", "rarity": "Rare"},
]


def row(image_id, text, session="s1", stored=None):
    return {
        "image_id": image_id,
        "session_id": session,
        "ocr_text": text,
        "entity_id": stored,
    }


# ===========================================================================
# Normalisation
# ===========================================================================

def test_norm_key_drops_accents_punctuation_and_space():
    assert norm_key("Skeletal Shark") == "skeletalshark"
    assert norm_key("Jack-O'-Bite") == "jackobite"
    assert norm_key("Blue-Lip Grouper") == "bluelipgrouper"
    assert norm_key("Café Fish") == "cafefish"


# ===========================================================================
# Similarity
# ===========================================================================

def test_similarity_handles_a_clean_read():
    assert similarity("zebraribbonangelfish", "zebraribbonangelfish") == 1.0


def test_similarity_survives_a_garbled_banner_prefix():
    # The dominant failure in the live data: a mangled "New Item" glued to the
    # front of an otherwise perfect fish name.
    for text in (
        "uevjltem«Crimson snapper*",
        "Nev; Item«Exotic Tiserfin»",
        "uev.ltem•zebra Ribbon AnseLfish>",
        ".=uevzltem •zebra Ribbon AnseLfish>",
    ):
        best = max(similarity(norm_key(text), e["matchKey"]) for e in BESTIARY)
        assert best >= 0.80, f"{text!r} scored only {best:.2f}"


def test_similarity_scores_a_slightly_wrong_fish_below_a_right_one():
    right = similarity(norm_key("Nev;ltem <FansfisB"), "anglerfish")
    wrong = similarity(norm_key("Nev;ltem <FansfisB"), "crimsonsnapper")
    assert right > wrong


# ===========================================================================
# Scoring a row
# ===========================================================================

def test_a_clean_read_picks_the_right_fish():
    entry, score = score_row("New Item <Crimson Snapper>", BESTIARY)
    assert entry["name"] == "Crimson Snapper"
    assert score >= CONFIDENT_AT


def test_an_empty_row_scores_nothing():
    entry, score = score_row("", BESTIARY)
    assert entry is None
    assert score == 0.0


def test_system_text_is_not_forced_into_a_fish():
    # The observed failure this guards: banner text like "spooksville astuo ..."
    # has enough letters to look vaguely fishy to a weak matcher.
    entry, score = score_row("town of beginnings safezone protected", BESTIARY)
    assert score < MIN_CONFIDENCE, f"system text scored {score:.2f}"


def test_scoring_is_deterministic():
    a = score_row("uevjltem«Crimson snapper*", BESTIARY)
    b = score_row("uevjltem«Crimson snapper*", BESTIARY)
    assert a == b


# ===========================================================================
# Tiers: the negative cases matter most
# ===========================================================================

def test_no_text_is_its_own_tier_never_a_rejection():
    # These are different claims. "I could not read it" is not "it is not a fish",
    # and collapsing them would quietly shrink the dataset by three quarters.
    assert tier_for(0.0, has_text=False) == "no_text"
    assert tier_for(1.0, has_text=False) == "no_text"


def test_the_tier_boundaries():
    assert tier_for(CONFIDENT_AT, True) == "confident"
    assert tier_for(MIN_CONFIDENCE, True) == "guessable"
    assert tier_for(MIN_CONFIDENCE - 0.01, True) == "below"
    assert tier_for(0.0, True) == "below"


# ===========================================================================
# Building the proposal set
# ===========================================================================

def test_build_candidates_proposes_and_tiers_correctly():
    rows = [
        row("a", "New Item <Crimson Snapper>", "s1", "fish:snapper"),
        row("b", "auevjltem «Crimson Snapper»", "s2"),
        row("c", "town of beginnings safezone protected", "s1"),
        row("d", "", "s1"),
        row("e", "random letters xyzzy", "s1"),
    ]
    cands, summary = build_candidates(rows, BESTIARY)

    assert summary["tiers"]["confident"] == 2
    assert summary["tiers"]["no_text"] == 1
    assert summary["tiers"]["below"] == 2
    assert {c.image_id for c in cands} == {"a", "b"}


def test_a_heavy_garble_is_not_promoted_to_confident():
    # `FansfisB` loses too many characters of "Anglerfish" to clear the bar. It is
    # recorded as unproposable rather than guessed at, which is the point of a
    # confidence tier: the labeler is allowed to say "I don't know".
    entry, score = score_row("Nev;ltem <FansfisB", BESTIARY)
    assert score < CONFIDENT_AT
    cands, summary = build_candidates([row("b", "Nev;ltem <FansfisB", "s2")], BESTIARY)
    assert cands == [], "a below-threshold row must not produce a proposal"
    assert summary["tiers"]["below"] == 1


def test_a_stored_label_that_disagrees_is_flagged():
    # The dataset's stored labels use classes the wiki says do not exist, so most
    # confident rows will disagree. That has to be visible per row, or a reviewer
    # inherits the old assumption without seeing it.
    rows = [row("a", "New Item <Crimson Snapper>", "s1", "fish:angelfish")]
    cands, _ = build_candidates(rows, BESTIARY)
    assert len(cands) == 1
    assert cands[0].stored_conflicts is True
    assert cands[0].stored_entity_id == "fish:angelfish"
    assert cands[0].proposed_name == "Crimson Snapper"


def test_an_agreeing_stored_label_is_not_flagged_as_a_conflict():
    rows = [row("a", "New Item <Crimson Snapper>", "s1", "fish:crimsonsnapper")]
    cands, _ = build_candidates(rows, BESTIARY)
    assert cands[0].stored_conflicts is False


def test_agreement_with_the_stored_label_is_not_used_to_choose():
    # If the stored label broke ties, a wrong stored label would win forever. The
    # proposal must come from the text alone.
    rows = [row("a", "New Item <Crimson Snapper>", "s1", "fish:angelfish")]
    cands, _ = build_candidates(rows, BESTIARY)
    assert cands[0].proposed_name == "Crimson Snapper"


def test_sessions_are_counted_per_class_for_the_training_threshold():
    rows = [
        row("a", "New Item <Crimson Snapper>", "s1"),
        row("b", "New Item <Crimson Snapper>", "s2"),
        row("c", "New Item <Crimson Snapper>", "s2"),
    ]
    _, summary = build_candidates(rows, BESTIARY)
    pc = summary["perClass"]["Crimson Snapper"]
    assert pc["confidentRows"] == 3
    assert pc["confidentSessions"] == 2, "three rows in two sessions is two sessions"


def test_no_candidate_is_ever_marked_training_eligible():
    # The whole point: these are proposals. No eligibility field may appear.
    rows = [row("a", "New Item <Crimson Snapper>", "s1", "fish:snapper")]
    cands, _ = build_candidates(rows, BESTIARY)
    blob = str([c.to_json() for c in cands]).lower()
    for forbidden in ("training_eligible", "eligible", "review_state", "reviewed"):
        assert forbidden not in blob, f"a proposal must not carry {forbidden!r}"


def test_an_empty_bestiary_is_refused(tmp_free=None):
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        p = Path(td) / "wiki_fish.json"
        p.write_text('{"entries": []}', encoding="utf-8")
        try:
            load_bestiary(p)
        except ValueError:
            return
        raise AssertionError("an empty bestiary must be refused, not silently accepted")


# ===========================================================================
# Output
# ===========================================================================

def test_written_metadata_carries_the_warning_and_provenance(tmp_free=None):
    import json
    import tempfile

    cands = [
        Candidate(
            image_id="a", session_id="s1",
            proposed_entity_id="fish:crimsonsnapper",
            proposed_name="Crimson Snapper",
            confidence=0.93, tier="confident",
            ocr_text="New Item <Crimson Snapper>", stored_entity_id=None,
        )
    ]
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "candidates.jsonl"
        meta = write_candidates(out, cands, {"tiers": {}, "perClass": {}},
                               source="test", bestiary_sha="deadbeef")
        assert out.is_file()
        written = json.loads(out.read_text(encoding="utf-8").strip())
        assert written["imageId"] == "a"
        assert written["confidence"] == 0.93
        assert "MACHINE PROPOSALS" in meta["warning"]
        assert meta["bestiarySha256"] == "deadbeef"
        assert len(meta["contentSha256"]) == 64
        assert "Not labels" in meta["warning"]


def test_the_hash_is_stable_for_identical_input(tmp_free=None):
    import tempfile

    cands = [
        Candidate("a", "s1", "fish:x", "X", 0.9, "confident", "t", None),
    ]
    hashes = []
    with tempfile.TemporaryDirectory() as td:
        for name in ("a.jsonl", "b.jsonl"):
            m = write_candidates(Path(td) / name, cands, {"tiers": {}, "perClass": {}},
                                 source="t", bestiary_sha="s")
            hashes.append(m["contentSha256"])
    assert hashes[0] == hashes[1]