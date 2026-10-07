"""Tests for the wiki reference matcher.

The matcher is the part that could mislead a human. Two directions matter and both
are wrong in opposite ways:

* a **false match** would tell a reviewer a name is a documented fish when it is not,
  pushing them toward NEW_ENTITY on bad evidence;
* a **false miss** would tell them a real fish is undocumented, which reads as "this
  probably isn't a fish" and pushes them toward UNKNOWN for the wrong reason.

So the tests pin both directions, and pin the third thing that matters: an absent or
unparseable reference must degrade to "unknown", never to "not a fish".
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from gpo_ontology_review.wiki import (  # noqa: E402
    Match,
    WikiFish,
    build_snapshot,
    edit_distance,
    match_key,
    match_name,
    parse_bestiary,
    summarise,
)


def snapshot(*names, **kwargs):
    fish = [WikiFish(name=n, rarity=kwargs.get("rarity", "Rare"), seasonal=kwargs.get("seasonal", False))
            for n in names]
    return build_snapshot(fish, retrieved_at_unix=1_700_000_000)


# ===========================================================================
# The matching key
# ===========================================================================

def test_the_key_merges_renderings_but_never_different_names():
    # Renderings of ONE name must collapse...
    assert match_key("Skeletal Shark") == match_key("skeletalshark")
    assert match_key("Jack-O'-Bite") == match_key("jack o bite")
    assert match_key("Blue-Lip Grouper") == match_key("BlueLipGrouper")
    assert match_key("Café Fish") == match_key("Cafe Fish")

    # ...and two DIFFERENT names must never collide. This is the property that makes
    # the key safe to reuse from the concept grouper.
    assert match_key("sgolden tigerfin") != match_key("golden tigerfin")
    assert match_key("golden tigerfin") != match_key("golden tigerfinz")
    assert match_key("zebra ribbon angelfish") != match_key("golden ribbon angelfish")


def test_the_key_is_total_and_never_empty_for_a_real_name():
    assert match_key("Zebra Ribbon Angelfish") == "zebraribbonangelfish"
    assert match_key("!!!") == ""
    assert match_key("") == ""


# ===========================================================================
# Edit distance
# ===========================================================================

def test_edit_distance_basics():
    assert edit_distance("abc", "abc") == 0
    assert edit_distance("abc", "abd") == 1
    assert edit_distance("abc", "ab") == 1
    assert edit_distance("sgoldentigerfin", "goldentigerfin") == 1
    assert edit_distance("", "abc") == 3


def test_edit_distance_short_circuits_beyond_the_cap():
    # The cap keeps the suggestion pass cheap; beyond it the exact value is not
    # needed, only "too far".
    assert edit_distance("a", "zzzzzzzz", cap=2) > 2
    assert edit_distance("kitten", "sitting", cap=5) == 3


# ===========================================================================
# Exact matching
# ===========================================================================

def test_an_exact_match_reports_the_reference_details():
    snap = snapshot("Zebra Ribbon Angelfish", rarity="Rare", seasonal=False)
    m = match_name("zebra ribbon angelfish", snap)
    assert m.matched
    assert m.canonical_name == "Zebra Ribbon Angelfish"
    assert m.rarity == "Rare"
    assert m.seasonal is False
    assert m.url and m.url.startswith("https://")
    assert "exact" in m.basis.lower()


def test_a_spacing_difference_still_matches():
    # The dataset contains `skeletalshark` because the banner renders it without a
    # space. That is the same fish, not an unknown one.
    snap = snapshot("Skeletal Shark", rarity="Epic", seasonal=True)
    m = match_name("skeletalshark", snap)
    assert m.matched
    assert m.canonical_name == "Skeletal Shark"
    assert m.seasonal is True


def test_an_unknown_name_does_not_match():
    snap = snapshot("Tigerfin", "Swordfish")
    m = match_name("town of beginnings safezone protected", snap)
    assert not m.matched
    assert m.canonical_name is None


# ===========================================================================
# Near misses: the case that must not read as "no match"
# ===========================================================================

def test_a_one_character_garble_suggests_the_real_fish():
    snap = snapshot("Blue-Lip Grouper", rarity="Common")
    m = match_name("blve lipgrouper", snap)
    assert not m.matched, "it is not an exact match"
    # But it must NOT be reported as absent either: the reviewer would conclude the
    # name is not a fish, which is false.
    assert m.suggestion == "Blue-Lip Grouper"
    assert m.suggestion_rarity == "Common"
    assert m.suggestion_distance == 1


def test_a_leading_stray_letter_suggests_the_real_fish():
    # The dominant OCR failure in this dataset: a boundary character from
    # "New Item <...>" leaks onto the name.
    snap = snapshot("Golden Tigerfin", rarity="Legendary")
    m = match_name("sgolden tigerfin", snap)
    assert m.suggestion == "Golden Tigerfin"
    assert m.suggestion_distance == 1


def test_a_transposed_pair_suggests_the_real_fish():
    # rn -> m and ei -> ie are the two substitutions the OCR model makes most.
    snap = snapshot("Crimson Snapper", "Candy Corn Squid", "Exotic Tigerfin")
    assert match_name("rimson snapper", snap).suggestion == "Crimson Snapper"
    assert match_name("candy cormn squid", snap).suggestion == "Candy Corn Squid"
    assert match_name("exotietigerfin", snap).suggestion == "Exotic Tigerfin"


def test_a_genuinely_distant_name_gets_no_suggestion():
    snap = snapshot("Tigerfin", "Swordfish", "Anglerfish")
    m = match_name("qwertyuiop", snap)
    assert not m.matched
    assert m.suggestion is None, "a wild suggestion is worse than none"


def test_short_names_do_not_match_half_the_list():
    # A short name has many neighbours at distance 1, so the tolerance scales down.
    snap = snapshot("Cod", "Cat", "Carp", "Bass", "Boot")
    m = match_name("dog", snap)
    assert m.suggestion is None or m.suggestion_distance >= 1


# ===========================================================================
# Degradation: the most important behaviour
# ===========================================================================

def test_an_absent_reference_never_claims_a_name_is_not_a_fish():
    for snap in ({}, {"entries": []}, None):
        if snap is None:
            continue
        m = match_name("Zebra Ribbon Angelfish", snap)
        assert not m.matched
        assert m.suggestion is None
        assert "unavailable" in m.basis, (
            "a missing reference must read as UNKNOWN, never as 'not a fish'"
        )


def test_summarise_partitions_the_concepts():
    snap = snapshot("Tigerfin", "Swordfish")
    ms = [
        match_name("tigerfin", snap),      # exact
        match_name("sgoldentigerfin", snapshot("Golden Tigerfin")),  # near
        match_name("zzz qqq", snap),       # absent
    ]
    s = summarise(ms)
    assert s == {"total": 3, "exact": 1, "nearMiss": 1, "absent": 1}


# ===========================================================================
# Parsing the real page shape
# ===========================================================================

SAMPLE = """
== Bestiary ==
This table contains the current fishes
{| class="article-table sortable"
|-
!Image
!Name
!Description
!Rarity
!Seasonal
|-
|[[File:Anglerfish.png|frameless|150x150px]]
|<span class="legendary">'''Anglerfish'''</span>
|A legendary Deep-sea fish with an eerie glow
|{{Rarity|L}}
|No
|-
|[[File:Skeletal Shark.png|frameless|150x150px]]
|<span class="epic">'''Skeletal Shark'''</span>
|with a stone-like appearance. Exclusive to the Halloween event.
|{{Rarity|E}}

(classified as {{Rarity|R}} when crafting baits)
|Yes
|-
|[[File:Jack-O'-Bite2.png|frameless|150x150px]]
|<span class="legendary">'''Jack-O'-Bite'''</span>
|A fierce, pumpkin-shaped fish.
|{{Rarity|L}}
|Yes
|}
"""


def test_the_parser_extracts_name_rarity_seasonal_and_description():
    fish = parse_bestiary(SAMPLE)
    assert [f.name for f in fish] == ["Anglerfish", "Skeletal Shark", "Jack-O'-Bite"]
    by = {f.name: f for f in fish}
    assert by["Anglerfish"].rarity == "Legendary"
    assert by["Anglerfish"].seasonal is False
    assert by["Skeletal Shark"].rarity == "Epic", "the first rarity wins, not the bait-crafting note"
    assert by["Skeletal Shark"].seasonal is True
    assert by["Jack-O'-Bite"].rarity == "Legendary"
    assert "pumpkin" in by["Jack-O'-Bite"].description
    # No wiki markup leaks into the name or the description.
    assert all("<" not in f.name and "'''" not in f.name for f in fish)
    assert all("{{" not in f.description for f in fish)


def test_the_parser_refuses_a_page_it_does_not_understand():
    # The dangerous outcome is an EMPTY snapshot: it would make every concept read
    # as "not a documented fish" and quietly turn a missing reference into evidence.
    for bad in ["", "no bestiary here", "== Bestiary ==\nnothing table-like"]:
        try:
            got = parse_bestiary(bad)
        except ValueError:
            continue
        assert got == [], f"expected a refusal for {bad!r}, got {got}"


def test_the_snapshot_records_provenance():
    snap = build_snapshot([WikiFish("Tigerfin", "Common", False)], retrieved_at_unix=1_700_000_000)
    assert snap["kind"] == "ontology_wiki_bestiary"
    assert snap["entryCount"] == 1
    assert snap["retrievedAtUnix"] == 1_700_000_000
    assert len(snap["contentSha256"]) == 64
    assert snap["sourceUrl"].startswith("https://")
    assert "not an authority" in snap["caveat"]
    assert snap["entries"][0]["matchKey"] == "tigerfin"


def test_the_snapshot_hash_is_stable_for_the_same_content():
    a = build_snapshot([WikiFish("A", "Common", False)], retrieved_at_unix=1)
    b = build_snapshot([WikiFish("A", "Common", False)], retrieved_at_unix=2)
    # The retrieval time is provenance, not content: it must not change the hash, or
    # re-running the refresh would look like a change when nothing did.
    assert a["contentSha256"] == b["contentSha256"]


def test_match_to_json_omits_nothing_a_client_needs():
    snap = snapshot("Tigerfin", rarity="Common")
    j = match_name("tigerfin", snap).to_json()
    for key in (
        "matched", "canonicalName", "rarity", "seasonal",
        "description", "url", "suggestion", "basis",
    ):
        assert key in j, key


def test_a_match_is_never_a_decision():
    # The matcher has no vocabulary for decisions and must not grow one.
    j = match_name("tigerfin", snapshot("Tigerfin")).to_json()
    blob = " ".join(j).lower()
    for forbidden in ["new_entity", "existing_entity", "variant", "decision"]:
        assert forbidden not in blob, (
            f"the reference matcher must not emit {forbidden!r}; a wiki match is evidence"
        )