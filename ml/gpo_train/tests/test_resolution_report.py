"""Tests for ml.gpo_train.resolution_report: bucket arithmetic, determinism,
and the read-only guarantee. Synthetic fixtures only."""

from __future__ import annotations

import hashlib
import json
import tempfile
from pathlib import Path

from ml.gpo_train import resolution_report as rr


def _write_jsonl(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")


def _kb_fixture(tmp: Path) -> Path:
    entities = [
        {"id": "fish:crimsonsnapper", "canonical_name": "Crimson Snapper",
         "aliases": [], "ocr_aliases": [], "category": "fish", "rarity": None},
        {"id": "fish:snapper", "canonical_name": "Snapper",
         "aliases": [], "ocr_aliases": [], "category": "fish", "rarity": None},
        {"id": "fish:crimson", "canonical_name": "Crimson",
         "aliases": [], "ocr_aliases": [], "category": "fish", "rarity": None},
        {"id": "fish:swordfish", "canonical_name": "Swordfish",
         "aliases": [], "ocr_aliases": [], "category": "fish", "rarity": None},
        {"id": "ui:new-item", "canonical_name": "New Item",
         "aliases": [], "ocr_aliases": [], "category": "ui_term", "rarity": None},
    ]
    p = tmp / "kb.json"
    p.write_text(json.dumps({"schema": 1, "kb_version": 1, "entities": entities}), encoding="utf-8")
    return p


def _fixture(tmp: Path, reviews: list[dict], labels: list[dict]) -> Path:
    appdata = tmp / "appdata"
    _write_jsonl(appdata / "reviews.jsonl", reviews)
    _write_jsonl(appdata / "datasets" / "gpo-vision" / "v1" / "labels.jsonl", labels)
    (appdata).mkdir(parents=True, exist_ok=True)
    (appdata / "knowledge_custom.json").write_text(
        json.dumps([{"id": "fish:crimsonsnapper", "canonical_name": "Crimson Snapper",
                     "aliases": [], "category": "fish", "rarity": None,
                     "description": None, "ocr_aliases": [], "visual_refs": [],
                     "provenance": {"source": "t", "url": None, "updated": None}}]),
        encoding="utf-8")
    return appdata


def _review(image: str, human: str) -> dict:
    return {"image_id": image, "human_entity_id": human, "training_eligible": True}


def _label(image: str, ocr: str, entity: str | None = None) -> dict:
    return {"image_id": image, "ocr_text": ocr, "entity_id": entity}


def test_bucket_arithmetic_on_synthetic_rows() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        kb = _kb_fixture(tmp)
        reviews = [
            _review("a", "fish:crimsonsnapper"),  # phrase resolves -> correct_species
            _review("b", "fish:crimsonsnapper"),  # single word -> stem -> legacy_stem
            _review("c", "fish:swordfish"),       # correct species
            _review("d", "fish:crimsonsnapper"),  # gibberish -> unknown
        ]
        labels = [
            _label("a", "New Item <Crimson Snapper>"),
            _label("b", "Snapper"),
            _label("c", "Swordfish"),
            _label("d", "zzq wobble"),
        ]
        appdata = _fixture(tmp, reviews, labels)
        report = rr.run(appdata, kb)
        after = report["buckets_after"]
        assert after["correct_species"]["count"] == 2
        assert after["legacy_stem"]["count"] == 1
        assert after["unknown"]["count"] == 1
        # Denominators everywhere: shares are over the reviewed count.
        assert after["correct_species"]["share"] == 0.5
        assert report["reviewed_rows_with_human_entity"] == 4
        # human != collector counted for the rows whose collector label differed.
        assert report["human_differs_from_collector"]["count"] == 4


def test_report_is_byte_identical_across_runs() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        kb = _kb_fixture(tmp)
        reviews = [_review("a", "fish:crimsonsnapper"), _review("b", "fish:swordfish")]
        labels = [_label("a", "Crimson Snapper"), _label("b", "Swordfish")]
        appdata = _fixture(tmp, reviews, labels)
        a = rr.run(appdata, kb)
        b = rr.run(appdata, kb)
        assert json.dumps(a, sort_keys=True) == json.dumps(b, sort_keys=True)


def test_the_report_never_mutates_its_inputs() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        kb = _kb_fixture(tmp)
        reviews = [_review("a", "fish:crimsonsnapper")]
        labels = [_label("a", "Crimson Snapper", "fish:snapper")]
        appdata = _fixture(tmp, reviews, labels)
        files = [
            appdata / "reviews.jsonl",
            appdata / "datasets" / "gpo-vision" / "v1" / "labels.jsonl",
            appdata / "knowledge_custom.json",
            kb,
        ]
        before = {f: hashlib.sha256(f.read_bytes()).hexdigest() for f in files}
        rr.run(appdata, kb)
        after = {f: hashlib.sha256(f.read_bytes()).hexdigest() for f in files}
        assert before == after


def test_the_new_ladder_fails_closed_on_a_two_stem_compound() -> None:
    """Without the species entity, `Crimson Snapper` names two stems and must
    be AMBIGUOUS - never a silent pick of either one."""
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        kb_path = tmp / "kb-stems-only.json"
        kb_path.write_text(json.dumps({"schema": 1, "kb_version": 1, "entities": [
            {"id": "fish:snapper", "canonical_name": "Snapper", "aliases": [],
             "ocr_aliases": [], "category": "fish", "rarity": None},
            {"id": "fish:crimson", "canonical_name": "Crimson", "aliases": [],
             "ocr_aliases": [], "category": "fish", "rarity": None},
        ]}), encoding="utf-8")
        # No overlay species at all.
        appdata = tmp / "appdata"
        _write_jsonl(appdata / "reviews.jsonl", [_review("a", "fish:crimsonsnapper")])
        _write_jsonl(appdata / "datasets" / "gpo-vision" / "v1" / "labels.jsonl",
                     [_label("a", "Crimson Snapper")])
        (appdata / "knowledge_custom.json").write_text("[]", encoding="utf-8")
        report = rr.run(appdata, kb_path)
        assert report["buckets_after"]["ambiguous"]["count"] == 1
        assert report["buckets_after"]["legacy_stem"]["count"] == 0
