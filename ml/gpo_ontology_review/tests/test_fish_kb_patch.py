"""Tests for ``ml.gpo_ontology_review.fish_kb_patch``.

The module only ANALYSES (audit/generate/validate); these tests pin the rules
the KB patch depends on, using synthetic fixtures in temp dirs - never the
real review log, overlay, dataset, or live KB export. Plain functions, per
this suite's runner.
"""

from __future__ import annotations

import json
import tempfile
from pathlib import Path

from ml.gpo_ontology_review import fish_kb_patch as fkp


def _write_jsonl(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")


def _appdata(tmp: Path, reviews: list[dict], labels: list[dict]) -> Path:
    appdata = tmp / "appdata"
    _write_jsonl(appdata / "reviews.jsonl", reviews)
    _write_jsonl(appdata / "datasets" / "gpo-vision" / "v1" / "labels.jsonl", labels)
    return appdata


def _review(image: str, entity: str | None, session: str = "s1",
            status: str = "REVIEWED_CORRECTED", eligible: bool = False) -> dict:
    return {
        "image_id": image,
        "session_id": session,
        "human_entity_id": entity,
        "review_status": status,
        "training_eligible": eligible,
    }


def _label(image: str, ts_ms: int) -> dict:
    return {"image_id": image, "timestamp_ms": ts_ms}


#: The pre-patch bundled fish (core::fruit::KNOWN_FISH). The committed export is
#: now the EFFECTIVE KB (bundled + overlay), so tests write this fixture to pin
#: the world a patch is generated against, independent of the live KB's state.
PRE_PATCH_FISH = [
    "snapper", "crimson", "tuna", "bluefin", "shark", "tiger", "colossal", "megalodon",
    "golden", "boot", "trash", "seaweed", "salmon", "bass", "catfish", "carp", "trout",
    "swordfish", "marlin", "squid", "octopus", "crab", "lobster", "shrimp", "ray", "manta",
    "barracuda", "angelfish", "clownfish", "kraken", "beast", "eel", "piranha", "pufferfish",
    "halibut", "cod", "sailfish", "turtle", "jellyfish", "sunfish", "flying",
]


def _kb_fixture(tmp: Path) -> Path:
    """A synthetic kb_export.json holding exactly the pre-patch bundled fish."""
    entities = [
        {
            "id": f"fish:{name}",
            "canonical_name": name.capitalize(),
            "aliases": [],
            "ocr_aliases": [],
            "category": "fish",
            "rarity": None,
        }
        for name in PRE_PATCH_FISH
    ]
    path = tmp / "kb_export_fixture.json"
    path.write_text(json.dumps({
        "schema": 1, "kb_version": 1, "fish_vocab": [], "fish_vocab_sha256": "x",
        "entity_count": len(entities), "entities": entities,
    }), encoding="utf-8")
    return path


def test_thresholds_are_parsed_from_the_rust_source() -> None:
    t = fkp.load_thresholds()
    # Values come from ml_capability.rs; if the code changes, this test forces
    # a conscious decision here too rather than drifting silently.
    assert t["entity_min_examples"] == 20
    assert t["entity_min_sessions"] == 3


def test_only_human_confirmed_wiki_documented_entities_are_added() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        reviews = [_review(f"img{i}", "fish:anglerfish") for i in range(25)]
        reviews.append(_review("ghost1", "fish:nosuchfish"))
        reviews.append(_review("un1", None, status="REVIEWED_UNKNOWN"))
        labels = [_label(f"img{i}", 1_700_000_000_000 + i * 86_400_000) for i in range(25)]
        appdata = _appdata(tmp, reviews, labels)

        patch = fkp.generate(appdata, _kb_fixture(tmp))
        add_ids = {a["entity"]["id"] for a in patch["entities_to_add"]}

        # Anglerfish: confirmed + wiki-documented -> added.
        assert "fish:anglerfish" in add_ids
        # The ghost entity has reviews but NO wiki entry -> unresolved, not added.
        assert "fish:nosuchfish" not in add_ids
        unresolved = {u["entity_id"] for u in patch["unresolved_concepts"]}
        assert "fish:nosuchfish" in unresolved
        # A row with no entity at all contributes nothing.
        assert len(patch["entities_to_add"]) == 1

        ent = next(a for a in patch["entities_to_add"] if a["entity"]["id"] == "fish:anglerfish")
        assert ent["entity"]["canonical_name"] == "Anglerfish"
        assert ent["entity"]["category"] == "fish"
        assert ent["human_reviewed_rows"] == 25
        assert ent["wiki_url"].startswith("https://")
        # No aliases anywhere: the OCR-audit contract requires fish to have none.
        assert ent["entity"]["aliases"] == []
        assert ent["entity"]["ocr_aliases"] == []


def test_existing_kb_entities_are_reused_not_duplicated() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        reviews = [_review("a", "fish:swordfish"), _review("b", "fish:anglerfish")]
        appdata = _appdata(tmp, reviews, [_label("a", 1), _label("b", 2)])
        patch = fkp.generate(appdata, _kb_fixture(tmp))
        add_ids = {a["entity"]["id"] for a in patch["entities_to_add"]}
        reused = {r["entity_id"] for r in patch["entities_reused"]}
        assert "fish:swordfish" not in add_ids
        assert "fish:swordfish" in reused
        assert "fish:anglerfish" in add_ids


def test_generation_is_deterministic() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        reviews = [_review(f"i{i}", "fish:anglerfish", session=f"s{i % 4}") for i in range(30)]
        labels = [_label(f"i{i}", 1_700_000_000_000) for i in range(30)]
        appdata = _appdata(tmp, reviews, labels)
        kb = _kb_fixture(tmp)
        a = fkp.generate(appdata, kb)
        b = fkp.generate(appdata, kb)
        a.pop("created_at")
        b.pop("created_at")
        assert a == b


def _valid_patch(appdata: Path, kb: Path) -> dict:
    reviews = [_review(f"i{i}", "fish:anglerfish") for i in range(25)]
    labels = [_label(f"i{i}", 1_700_000_000_000) for i in range(25)]
    _write_jsonl(appdata / "reviews.jsonl", reviews)
    _write_jsonl(appdata / "datasets" / "gpo-vision" / "v1" / "labels.jsonl", labels)
    return fkp.generate(appdata, kb)


def test_a_generated_patch_validates_clean() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        appdata = tmp / "appdata"
        appdata.mkdir(parents=True)
        kb = _kb_fixture(tmp)
        patch = _valid_patch(appdata, kb)
        assert fkp.validate_patch(patch, appdata, kb) == []


def test_duplicate_ids_are_rejected() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        appdata = tmp / "appdata"
        appdata.mkdir(parents=True)
        kb = _kb_fixture(tmp)
        patch = _valid_patch(appdata, kb)
        patch["entities_to_add"].append(patch["entities_to_add"][0])
        errors = fkp.validate_patch(patch, appdata, kb)
        assert any("duplicate" in e for e in errors)


def test_missing_provenance_is_rejected() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        appdata = tmp / "appdata"
        appdata.mkdir(parents=True)
        kb = _kb_fixture(tmp)
        patch = _valid_patch(appdata, kb)
        patch["entities_to_add"][0]["entity"]["provenance"]["url"] = ""
        errors = fkp.validate_patch(patch, appdata, kb)
        assert any("url" in e for e in errors)


def test_a_review_entity_unaccounted_for_is_rejected() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        appdata = tmp / "appdata"
        appdata.mkdir(parents=True)
        kb = _kb_fixture(tmp)
        patch = _valid_patch(appdata, kb)
        patch["entities_to_add"] = []
        patch["unresolved_concepts"] = []
        errors = fkp.validate_patch(patch, appdata, kb)
        assert any("not accounted for" for e in errors)


def test_migration_targets_must_exist_after_the_patch() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        appdata = tmp / "appdata"
        appdata.mkdir(parents=True)
        kb = _kb_fixture(tmp)
        patch = _valid_patch(appdata, kb)
        patch["migration_map"] = [{
            "old_entity_id": "fish:snapper",
            "canonical_entity_id": "fish:nowhere",
            "candidate_targets": [],
            "reason": "test",
        }]
        errors = fkp.validate_patch(patch, appdata, kb)
        assert any("nowhere" in e for e in errors)


def test_all_bundled_fish_are_classified_and_none_deleted() -> None:
    wiki = fkp.load_wiki()
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        (tmp / "appdata").mkdir()
        bundled = sorted(
            i for i in fkp.load_bundled_kb_ids(tmp / "appdata", _kb_fixture(tmp))
            if i.startswith("fish:")
        )
        legacy = fkp.classify_legacy(bundled, wiki)
    assert len(legacy) == len(PRE_PATCH_FISH)
    valid = [e for e in legacy if e["role"] == "VALID_CANONICAL"]
    assert [e["entity_id"] for e in valid] == ["fish:swordfish"]
    by_id = {e["entity_id"]: e for e in legacy}
    assert by_id["fish:crimson"]["ambiguous"]
    assert by_id["fish:snapper"]["migration_targets"] == ["fish:crimsonsnapper"]
    assert by_id["fish:pufferfish"]["migration_targets"] == [
        "fish:crimsonpolkapuffer",
        "fish:goldenpolkapuffer",
    ]
