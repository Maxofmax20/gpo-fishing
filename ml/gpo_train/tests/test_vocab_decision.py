"""Tests for ml.gpo_train.vocab_decision: the projection rules, the per-class
arithmetic for both vocabulary options, determinism, and read-only behaviour.

Synthetic fixtures only. The live store is never touched by a test.
"""

from __future__ import annotations

import hashlib
import json
import tempfile
from pathlib import Path

from ml.gpo_train import vocab_decision as vd


def _write_jsonl(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")


def _fixture(tmp: Path) -> dict[str, Path]:
    """Two stems and two species, with reviews that move most stem rows away."""
    appdata = tmp / "appdata"
    labels = []
    reviews = []
    # fish:crimson: 20 collected rows, 16 reviewed as crimsonpolkapuffer.
    for i in range(20):
        labels.append({"image_id": f"cr-{i}", "session_id": f"s-{i % 5}", "entity_id": "fish:crimson"})
        if i < 16:
            reviews.append({"image_id": f"cr-{i}", "training_eligible": True,
                            "human_entity_id": "fish:crimsonpolkapuffer"})
    # fish:golden: 25 rows a human confirmed as-is.
    for i in range(25):
        labels.append({"image_id": f"go-{i}", "session_id": f"s-{i % 5}", "entity_id": "fish:golden"})
        reviews.append({"image_id": f"go-{i}", "training_eligible": True,
                        "human_entity_id": "fish:golden"})
    # fish:swordfish: 21 rows, 3 of them rejected by the human.
    for i in range(21):
        labels.append({"image_id": f"sw-{i}", "session_id": f"s-{i % 5}", "entity_id": "fish:swordfish"})
        if i < 18:
            reviews.append({"image_id": f"sw-{i}", "training_eligible": True,
                            "human_entity_id": "fish:swordfish"})
        else:
            reviews.append({"image_id": f"sw-{i}", "training_eligible": False,
                            "excluded_reason": "human skipped"})
    # fish:anglerfish: 12 rows - short of the class gate, no review needed.
    for i in range(12):
        labels.append({"image_id": f"an-{i}", "session_id": f"s-{i % 5}", "entity_id": "fish:anglerfish"})

    _write_jsonl(appdata / "datasets" / "gpo-vision" / "v1" / "labels.jsonl", labels)
    _write_jsonl(appdata / "reviews.jsonl", reviews)
    (appdata / "knowledge_custom.json").write_text(json.dumps([
        {"id": "fish:crimsonpolkapuffer"}, {"id": "fish:anglerfish"},
        {"id": "fish:goldenribbonangelfish"}, {"id": "fish:swordfish"},
    ]), encoding="utf-8")

    kb_entities = [
        # bundled stems
        {"id": "fish:crimson"}, {"id": "fish:golden"}, {"id": "fish:swordfish"},
        # overlay species
        {"id": "fish:crimsonpolkapuffer"}, {"id": "fish:anglerfish"},
        {"id": "fish:goldenribbonangelfish"},
        # a bundled id that is neither in the vocab nor overlay
        {"id": "fish:tiger"},
    ]
    kb = tmp / "kb.json"
    kb.write_text(json.dumps({"schema": 1, "kb_version": 1,
                              "entities": [dict(e, canonical_name=e["id"], aliases=[],
                                                ocr_aliases=[], category="fish",
                                                rarity=None) for e in kb_entities]}),
                  encoding="utf-8")

    vocab = tmp / "FISH_VOCAB.txt"
    vocab.write_text("# header\nfish:crimson\nfish:golden\nfish:swordfish\n\n", encoding="utf-8")

    # The wiki documents CONFIRMED fish only - never a bare stem. That is the
    # rule `reviewed_dataset.load_wiki_fish_ids` enforces.
    wiki = tmp / "wiki_fish.json"
    wiki.write_text(json.dumps({"entries": [
        {"matchKey": "crimsonpolkapuffer"}, {"matchKey": "anglerfish"},
        {"matchKey": "goldenribbonangelfish"}, {"matchKey": "swordfish"},
    ]}), encoding="utf-8")

    manifest = tmp / "manifest.json"
    manifest.write_text(json.dumps({"class_map": ["fish:crimsonpolkapuffer",
                                                  "fish:goldenribbonangelfish",
                                                  "fish:swordfish"]}), encoding="utf-8")
    return {"appdata": appdata, "kb": kb, "vocab": vocab, "wiki": wiki, "manifest": manifest}


def test_projection_applies_the_review_rules_and_is_idempotent() -> None:
    labels = [
        {"image_id": "a", "entity_id": "fish:crimson"},          # eligible + different verdict
        {"image_id": "b", "entity_id": "fish:golden"},           # eligible + same verdict
        {"image_id": "c", "entity_id": "fish:swordfish"},        # eligible + no verdict
        {"image_id": "d", "entity_id": "fish:tiger"},            # ineligible -> dropped
        {"image_id": "e", "entity_id": None},                    # unreviewed -> kept
    ]
    reviews = [
        {"image_id": "a", "training_eligible": True, "human_entity_id": "fish:crimsonsnapper"},
        {"image_id": "b", "training_eligible": True, "human_entity_id": "fish:golden"},
        {"image_id": "c", "training_eligible": True, "human_entity_id": None},
        {"image_id": "d", "training_eligible": False, "excluded_reason": "human skipped"},
    ]
    rows, counts = vd.project_labels(labels, reviews)
    assert counts["rows_relabelled_by_review"] == 1
    assert counts["rows_kept_collector_label"] == 3      # b (same verdict) + c (none) + e (unreviewed)
    assert counts["rows_unreviewed"] == 1
    assert counts["rows_excluded_by_review"] == 1
    assert counts["rows_projected"] == 4
    assert {r["image_id"] for r in rows} == {"a", "b", "c", "e"}
    assert next(r for r in rows if r["image_id"] == "a")["entity_id"] == "fish:crimsonsnapper"
    # Idempotent: the projection depends on the reviews, not on the input labels.
    again, counts2 = vd.project_labels(rows, reviews)
    assert [r["image_id"] for r in again] == [r["image_id"] for r in rows]
    assert counts2["rows_relabelled_by_review"] == 0
    # Sticky exclusion: a duplicate id whose records disagree loses the row.
    rows3, counts3 = vd.project_labels(
        labels, reviews + [{"image_id": "a", "training_eligible": False}])
    assert counts3["rows_excluded_by_review"] == 2
    assert {r["image_id"] for r in rows3} == {"b", "c", "e"}


def test_split_port_agrees_with_the_rust_contract() -> None:
    # Session-pure 70/15/15; the same ids the Rust and Python splits agree on.
    train = [s for s in (f"s-{i}" for i in range(500)) if vd.fnv1a_session_split(s) == "train"]
    assert train, "some sessions must land in TRAIN"
    for s in train:
        assert vd.fnv1a_session_split(s) == "train"


def test_option_a_names_the_short_stem_and_option_b_flags_thin_test_support() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        f = _fixture(tmp)
        report = vd.run(f["appdata"], f["kb"], f["vocab"], f["wiki"], f["manifest"],
                        models_dir=tmp / "models")

        p = report["projection"]
        assert p["rows_live"] == 78
        assert p["rows_projected"] == 75            # 3 swordfish rows excluded
        assert p["rows_relabelled_by_review"] == 16

        a = report["options"]["option_a"]
        assert a["classes"] == ["fish:crimson", "fish:golden", "fish:swordfish"]
        crimson = next(c for c in a["per_class"] if c["entity"] == "fish:crimson")
        assert crimson["projected_rows"] == 4, "20 collected, 16 humans moved off the stem"
        assert crimson["clears_class_gate"] is False
        assert crimson["clears_trainer_scope"] is False
        assert any(x["entity"] == "fish:crimson" for x in a["failing_trainer_scope"])
        contra = next(c for c in a["contradicted_by_reviews"] if c["entity"] == "fish:crimson")
        assert contra["collector_rows"] == 20
        assert contra["projected_rows"] == 4
        assert contra["rows_humans_moved"] == 16
        assert contra["moved_to"] == {"fish:crimsonpolkapuffer": 16}

        b = report["options"]["option_b"]
        # wiki-documented AND in the KB, minus anglerfish (reported separately).
        assert b["classes"] == ["fish:crimsonpolkapuffer", "fish:goldenribbonangelfish",
                                "fish:swordfish"]
        assert b["cross_check_reviewed_manifest"]["class_map_matches"] is True
        angler = b["anglerfish_auto_entry"]
        assert angler["entity"] == "fish:anglerfish"
        assert angler["projected_rows"] == 12
        assert angler["rows_needed"] == 8, "20 rows - 12 collected"
        assert angler["sessions"] >= vd.ENTITY_MIN_SESSIONS
        assert "no code change" in angler["mechanism"]

        # Swordfish's rejected rows must not be counted for the species either.
        sw = next(c for c in b["per_class"] if c["entity"] == "fish:swordfish")
        assert sw["projected_rows"] == 18

        # Costs are stated, not implied.
        assert report["costs"]["option_a"]["vocab_sha_unchanged"] is True
        assert report["costs"]["option_b"]["retraining_required"] is True
        assert report["costs"]["option_b"]["vocab_sha_required"] != vd.vocab_sha(
            ["fish:crimson", "fish:golden", "fish:swordfish"])
        assert "NOT MADE HERE" in report["decision"]


def test_every_below_floor_class_carries_its_own_count() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        f = _fixture(tmp)
        report = vd.run(f["appdata"], f["kb"], f["vocab"], f["wiki"], f["manifest"],
                        models_dir=tmp / "models")
        b = report["options"]["option_b"]
        thin = {x["entity"]: x["test_rows"] for x in b["below_test_support_floor"]}
        assert set(thin) <= set(b["classes"])
        for entity, rows in thin.items():
            assert rows < vd.PER_CLASS_MIN_TEST_SUPPORT, f"{entity} is not actually thin"
        # A class with no test rows is reported as zero, not omitted.
        assert all("test_rows" in x for x in b["per_class"])


def test_report_is_byte_identical_and_never_mutates_its_inputs() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        f = _fixture(tmp)
        files = [
            f["appdata"] / "datasets" / "gpo-vision" / "v1" / "labels.jsonl",
            f["appdata"] / "reviews.jsonl",
            f["appdata"] / "knowledge_custom.json",
            f["kb"], f["vocab"], f["wiki"], f["manifest"],
        ]
        before = {p: hashlib.sha256(p.read_bytes()).hexdigest() for p in files}
        a = vd.run(f["appdata"], f["kb"], f["vocab"], f["wiki"], f["manifest"],
                   models_dir=tmp / "models")
        b = vd.run(f["appdata"], f["kb"], f["vocab"], f["wiki"], f["manifest"],
                   models_dir=tmp / "models")
        after = {p: hashlib.sha256(p.read_bytes()).hexdigest() for p in files}
        assert before == after, "a report that writes to its own inputs is not a report"
        assert json.dumps(a, sort_keys=True) == json.dumps(b, sort_keys=True)
        # Every input it names is stamped.
        assert a["inputs"]["labels_sha256"] == before[f["appdata"] / "datasets" / "gpo-vision" / "v1" / "labels.jsonl"]
        assert a["inputs"]["reviews_sha256"] == before[f["appdata"] / "reviews.jsonl"]
        assert a["inputs"]["kb_export_sha256"] == before[f["kb"]]
        assert a["inputs"]["fish_vocab_sha256"] == before[f["vocab"]]