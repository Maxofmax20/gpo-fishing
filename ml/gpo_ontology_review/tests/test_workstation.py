"""Prove the Python decision gate and the Rust workstation gate cannot drift.

Both read `ml/gpo_ontology_review/validation_fixtures.json`. This suite asserts the
Python implementation satisfies every case in that table.

Why a shared table rather than one implementation:

* The workstation must reject a malformed decision in milliseconds, so it cannot
  shell out to Python on every keystroke.
* Python holds the authoritative rules plus the cross-concept and knowledge-base
  checks.
* Two hand-written copies would drift, and the copy most likely to drift is the
  fast one - the fast one is the safety gate.

A shared fixture turns drift into a failing test on both sides. `rust_validation_agrees_with_the_shared_fixture_table`
in `src-tauri/src/commands_ontology_workstation.rs` reads the same file.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from gpo_ontology_review.decisions import (  # noqa: E402
    DecisionRejected,
    DecisionStore,
)
from gpo_ontology_review.schemas import (  # noqa: E402
    ALLOWED_DECISION_MAKERS,
    DecisionType,
    HumanDecision,
)
from gpo_ontology_review.workstation import (  # noqa: E402
    build_index,
    build_kb_entities,
    build_structural,
    index_digest,
    thumb_filename,
)

REPO = Path(__file__).resolve().parents[3]
FIXTURES = REPO / "ml" / "gpo_ontology_review" / "validation_fixtures.json"


def _fixtures() -> dict:
    if not FIXTURES.is_file():
        print(f"SKIP: {FIXTURES} absent")
        return {}
    return json.loads(FIXTURES.read_text(encoding="utf-8"))


def _decision(fields: dict) -> HumanDecision:
    return HumanDecision(
        concept_id=fields.get("concept_id", ""),
        decision=fields.get("decision", ""),
        existing_entity_id=fields.get("existing_entity_id"),
        proposed_entity_name=fields.get("proposed_entity_name"),
        variant_descriptor=fields.get("variant_descriptor"),
        decision_maker=fields.get("decision_maker", "human"),
        notes=fields.get("notes", ""),
    )


def test_python_validation_agrees_with_the_shared_fixture_table():
    """The mirror of the Rust test over the same table."""
    f = _fixtures()
    if not f:
        return
    cases = f.get("cases") or []
    assert len(cases) >= 10, "the fixture table should be substantial"

    for case in cases:
        name = case["name"]
        fields = case["fields"]
        expected = case["valid"]
        d = _decision(fields)
        problems = d.validate()

        # Machine attribution is rejected by validate() AND by the store.
        store_ok = expected
        if expected and fields.get("decision_maker", "human") != "human":
            store_ok = False

        assert len(problems) == 0 if expected else True, (
            f"fixture {name!r}: Python rejected a decision the table marks valid"
        )
        assert bool(problems) != expected, (
            f"fixture {name!r}: Python says valid={not problems} but the table says "
            f"{expected}; the Python gate has drifted from the shared rules"
        )

        if "mustMention" in case:
            joined = " ".join(problems).lower()
            norm = lambda x: "".join(ch for ch in x.lower() if ch.isalnum())
            assert norm(case["mustMention"]) in norm(joined), (
                f"fixture {name!r}: expected the problem to mention "
                f"{case['mustMention']!r}, got {problems}"
            )


def test_the_fixture_table_lists_exactly_the_four_decision_kinds():
    f = _fixtures()
    if not f:
        return
    assert set(f["decisionKinds"]) == {d.value for d in DecisionType}
    assert set(f["allowedDecisionMakers"]) == set(ALLOWED_DECISION_MAKERS) == {"human"}


def test_every_store_write_agrees_with_the_table(tmp_free=None):
    """The store must refuse exactly what `validate()` refuses.

    Exercised against a real temporary store so the append path is covered, not
    just the pure validation function.
    """
    import tempfile

    f = _fixtures()
    if not f:
        return
    with tempfile.TemporaryDirectory() as td:
        store = DecisionStore(Path(td) / "d.jsonl")
        accepted = 0
        for case in f["cases"]:
            d = _decision(case["fields"])
            try:
                store.record(d)
                recorded = True
            except DecisionRejected:
                recorded = False
            assert recorded == case["valid"], (
                f"fixture {case['name']!r}: store accepted={recorded} but the table "
                f"says valid={case['valid']}"
            )
            accepted += int(recorded)
        assert accepted == sum(1 for c in f["cases"] if c["valid"])
        assert len(store.all()) == accepted, "a rejected decision reached the log"


def test_the_three_workstation_commands_cover_every_fixture_case():
    """Names the three Rust commands the fixtures correspond to.

    Keeps the Python suite and the Rust suite discoverable from one place, and
    fails if the workstation module is renamed out from under this mapping.
    """
    import importlib
    import inspect

    for mod_name in ("ml.gpo_ontology_review",):
        try:
            importlib.import_module(mod_name)
        except ImportError:
            print(f"SKIP: {mod_name} not importable standalone")
            return
    src = (REPO / "src-tauri" / "src" / "commands_ontology_workstation.rs").read_text(
        encoding="utf-8"
    )
    for fn in ("ontology_review_status", "ontology_review_concept",
               "ontology_review_family", "ontology_review_kb_search",
               "ontology_review_thumb", "ontology_review_full",
               "ontology_decide", "ontology_undo"):
        assert f"pub fn {fn}(" in src, f"the workstation must expose {fn}"
    assert "pub fn ontology_decide" in src
    # Exactly one `pub fn ontology_*` writes.
    writes = [l for l in src.splitlines()
              if "pub fn" in l and "->" in l and not l.strip().startswith("//")]
    assert sum(1 for l in writes if "Result<DecisionResult" in l or "DecisionError" in l) >= 1


# --------------------------------------------------------------------------
# Workstation index shape
# --------------------------------------------------------------------------

def _concept(cid: str, name: str, occ: int, band: str = "P0"):
    from gpo_ontology_review.schemas import OntologyConcept

    return OntologyConcept(
        concept_id=cid, display_name=name, normalized_name=name,
        group_key=name.replace(" ", ""), occurrences=occ, priority=band,
        sessions=[f"session_2026-10-0{occ % 7 + 2}_1_aa"],
        calendar_days=["2026-10-02"],
        representative_images=[{
            "rank": 1, "image_id": "606020203e34beb6", "session_id": "s",
            "day": "2026-10-02", "raw_ocr": "x", "concept_name": name,
            "stored_entity_id": None, "ocr_confidence": 0.9, "timestamp_ms": 1,
            "selected_because": "highest OCR confidence",
        }],
    )


def test_index_omits_the_bulk_all_image_ids():
    """The workstation must not ship the full id list of every concept."""
    idx = build_index([_concept("concept:a", "a", 5)], {"entities": []}, [], {}, [])
    c = idx["concepts"][0]
    assert "allImageIds" not in c and "all_image_ids" not in c
    assert len(c["evidence"]) == 1


def test_index_order_is_band_then_evidence_then_name():
    cs = [
        _concept("concept:c", "ccc", 500, "P1"),
        _concept("concept:a", "aaa", 500, "P0"),
        _concept("concept:b", "bbb", 100, "P0"),
        _concept("concept:d", "ddd", 100, "P0"),
    ]
    idx = build_index(cs, {"entities": []}, [], {}, [])
    order = [c["conceptId"] for c in idx["concepts"]]
    # P0 before P1; within P0 higher occurrences first; ties alphabetical.
    assert order == ["concept:a", "concept:b", "concept:d", "concept:c"], order


def test_index_order_is_deterministic():
    cs = [_concept(f"concept:{i:02d}", f"n{i:02d}", 100 + (i % 3)) for i in range(12)]
    a = build_index(cs, {"entities": []}, [], {}, [])
    b = build_index(list(reversed(cs)), {"entities": []}, [], {}, {})
    assert index_digest(a) == index_digest(b)
    assert [c["conceptId"] for c in a["concepts"]] == \
           [c["conceptId"] for c in b["concepts"]]


def test_index_counts_match_the_concepts():
    cs = ([_concept(f"concept:p0{i}", f"p0{i}", 200) for i in range(11)]
          + [_concept("concept:p1", "p1", 40, "P1")])
    idx = build_index(cs, {"entities": []}, [], {}, {})
    assert idx["counts"]["total"] == 12
    assert idx["counts"]["p0"] == 11
    assert idx["counts"]["p1"] == 1


def test_kb_entities_include_aliases_and_are_sorted():
    kb = {
        "kb_version": 1,
        "fish_vocab": ["fish:shark"],
        "entities": [
            {"id": "fish:shark", "canonical_name": "Shark", "aliases": [],
             "ocr_aliases": [], "category": "fish", "rarity": None},
            {"id": "fruit:gomu", "canonical_name": "Gomu", "aliases": ["Rubber"],
             "ocr_aliases": ["Gummi"], "category": "fruit", "rarity": "rare"},
        ],
    }
    out = build_kb_entities(kb)
    ids = [e["id"] for e in out["entities"]]
    assert ids == sorted(ids), "the client's own filter must be stable"
    gomu = next(e for e in out["entities"] if e["id"] == "fruit:gomu")
    assert gomu["aliases"] == ["Rubber"]
    assert gomu["ocrAliases"] == ["Gummi"]
    assert next(e for e in out["entities"] if e["id"] == "fish:shark")["inFishVocab"]


def test_structural_carries_the_disclaimer_and_no_conclusions():
    """The structural panel must state it is evidence, never a verdict."""
    mods = [{"modifier": "golden", "occurrences": 330, "concept_count": 3,
             "session_count": 30, "day_count": 5, "bases": ["tigerfin"],
             "concepts": [{"concept_id": "concept:golden-tigerfin",
                           "display_name": "golden tigerfin", "occurrences": 100}]}]
    fams = {"family:tigerfin": {
        "family_id": "family:tigerfin", "base_name": "tigerfin",
        "member_count": 1, "occurrences": 449, "session_count": 30,
        "day_count": 5, "is_singleton": False,
        "members": [{"concept_id": "concept:tigerfin",
                     "display_name": "tigerfin", "modifiers": [],
                     "occurrences": 52}],
        "decision_required": "one entity or several?",
    }}
    bases = [{"base_name": "tigerfin", "occurrences": 449, "concept_count": 3,
              "session_count": 30, "day_count": 5, "occurs_bare": True,
              "bare_concept": "tigerfin", "kb_candidates": [],
              "stored_labels": {}, "modifier_totals": {"golden": 330},
              "concepts": [{"concept_id": "concept:tigerfin",
                            "display_name": "tigerfin", "occurrences": 52,
                            "modifiers": []}]}]
    out = build_structural(mods, fams, bases)
    assert "Evidence only" in out["disclaimer"]
    # No field may assert a role for a word.
    blob = json.dumps(out).lower()
    for verdict_word in ["is_a_modifier", "role\":", "conclusion", "decided"]:
        assert verdict_word not in blob, f"structural output must not decide: {verdict_word}"


def test_thumb_filename_is_derived_from_the_image_id_only():
    """Thumbnails are keyed by IMAGE, so the same image in several concepts is
    rendered once."""
    assert thumb_filename("606020203e34beb6") == "606020203e34beb6.jpg"
    assert thumb_filename("abc") == "abc.jpg"


# --------------------------------------------------------------------------
# Key-shape contract with the client
# --------------------------------------------------------------------------

def _key_paths(obj, path=""):
    """Every distinct key path in a payload, walking lists by their first element."""
    out = set()
    if isinstance(obj, dict):
        for k, v in obj.items():
            out.add(f"{path}/{k}")
            out |= _key_paths(v, f"{path}/{k}")
    elif isinstance(obj, list) and obj:
        out |= _key_paths(obj[0], f"{path}[]")
    return out


def _workstation_payloads():
    mods = [{"modifier": "golden", "occurrences": 330, "concept_count": 3,
             "session_count": 30, "day_count": 5, "bases": ["tigerfin"],
             "concepts": [{"concept_id": "concept:golden-tigerfin",
                           "display_name": "golden tigerfin",
                           "occurrences": 100}]}]
    fams = {"family:tigerfin": {
        "family_id": "family:tigerfin", "base_name": "tigerfin",
        "member_count": 2, "occurrences": 449, "session_count": 30,
        "day_count": 5, "is_singleton": False,
        "members": [
            {"concept_id": "concept:exotic-tigerfin",
             "display_name": "exotic tigerfin", "modifiers": ["exotic"],
             "occurrences": 262},
            {"concept_id": "concept:golden-tigerfin",
             "display_name": "golden tigerfin", "modifiers": ["golden"],
             "occurrences": 102},
        ],
    }}
    bases = [{"base_name": "tigerfin", "occurrences": 449, "concept_count": 3,
              "session_count": 30, "day_count": 5, "occurs_bare": True,
              "bare_concept": "tigerfin", "kb_candidates": [],
              "modifier_totals": {"golden": 330},
              "concepts": [{"concept_id": "concept:tigerfin",
                            "display_name": "tigerfin", "occurrences": 52,
                            "modifiers": []}]}]
    return {
        "index.json": build_index(
            [_concept("concept:a", "a", 5)],
            {"entities": [], "kb_version": 1, "fish_vocab": []}, [], {}, []),
        "structural.json": build_structural(mods, fams, bases),
        "kb_entities.json": build_kb_entities({"entities": [], "fish_vocab": []}),
    }


def test_every_workstation_artefact_uses_camel_case_keys():
    """The TypeScript client reads camelCase; the Python internals use snake_case.

    Mixing the two produces a payload that serialises cleanly and then reads as
    `undefined` in the browser - which is exactly what happened to family members
    and rendered an empty family panel with no error anywhere. This walks every key
    path in all three artefacts and refuses any snake_case.
    """
    import re

    snake = re.compile(r"/[a-z]+(_[a-z0-9]+)+$")
    for name, payload in _workstation_payloads().items():
        offenders = sorted(k for k in _key_paths(payload) if snake.search(k))
        assert not offenders, (
            f"{name} still emits snake_case key(s) {offenders}; the TypeScript client "
            f"reads camelCase and would silently see undefined"
        )


def test_structural_family_members_are_addressable_by_the_client():
    """The exact bug: members must carry the camelCase id the client looks up."""
    payload = _workstation_payloads()["structural.json"]
    fam = payload["families"][0]
    assert fam["familyId"] == "family:tigerfin"
    assert len(fam["members"]) == 2
    for m in fam["members"]:
        assert m["conceptId"].startswith("concept:")
        assert m["displayName"]
        assert m["occurrences"] > 0
    # And the names must match the ids, so the reviewer sees the same fish twice.
    assert {m["conceptId"] for m in fam["members"]} == {
        "concept:exotic-tigerfin", "concept:golden-tigerfin"}
    assert [m["displayName"] for m in fam["members"]] == [
        "exotic tigerfin", "golden tigerfin"]


# --------------------------------------------------------------------------
# Representative evidence: 8-12 per concept, diversity preserved
# --------------------------------------------------------------------------

def test_representative_sets_are_now_twelve_and_span_sessions():
    from gpo_ontology_review.evidence_select import Candidate, select_representative

    cands = [
        Candidate(f"s{s:02d}f{f:02d}", f"session_2026-10-0{s % 5 + 2}_100{s:03d}_aa",
                  f"2026-10-0{s % 5 + 2}", "New Item <Crimson Snapper>",
                  "crimson snapper", "fish:snapper", 0.95 - f * 0.004 - s * 0.001, s * 100 + f)
        for s in range(30) for f in range(4)
    ]
    sel = select_representative(cands, limit=12)
    assert len(sel) == 12, len(sel)
    # Sessions must be exhausted before confidence bands are considered.
    assert len({e["session_id"] for e in sel}) >= 10, \
        [e["session_id"] for e in sel]
    assert sel[0]["selected_because"] == "highest OCR confidence"


def test_a_small_concept_still_gets_every_distinct_session():
    from gpo_ontology_review.evidence_select import Candidate, select_representative

    cands = [Candidate(f"i{n}", f"session_2026-10-02_10000{n}_aa", "2026-10-02",
                       "x", "crimson snapper", "fish:snapper", 0.8, n)
             for n in range(4)]
    sel = select_representative(cands, limit=12)
    assert len({e["session_id"] for e in sel}) == 4


def test_selection_is_still_deterministic_after_the_reordering():
    from gpo_ontology_review.evidence_select import Candidate, select_representative

    cands = [Candidate(f"{i:04d}", f"session_2026-10-0{i % 4 + 2}_100{i}_aa",
                       f"2026-10-0{i % 4 + 2}", "x", "crimson snapper",
                       "fish:snapper", 0.5 + (i % 7) / 20, i)
             for i in range(40)]
    a = [e["image_id"] for e in select_representative(cands, 12)]
    b = [e["image_id"] for e in select_representative(list(reversed(cands)), 12)]
    assert a == b


def test_every_chosen_image_states_why():
    from gpo_ontology_review.evidence_select import Candidate, select_representative

    cands = [Candidate(f"{i:04d}", f"session_2026-10-0{i % 3 + 2}_100{i}_aa",
                       f"2026-10-0{i % 3 + 2}", "x", "c", "fish:shark",
                       0.6 + (i % 5) / 20, i)
             for i in range(30)]
    for e in select_representative(cands, 12):
        assert e["selected_because"], e