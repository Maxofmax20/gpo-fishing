"""Tests for the reviewed fish dataset rebuild (section-20 matrix).

Synthetic fixtures only: temp reviews.jsonl + labels.jsonl + KB export + wiki
snapshot + 1x1 PNG images. The real stores are never touched.
"""

from __future__ import annotations

import json
import tempfile
from pathlib import Path

from PIL import Image

from ml.gpo_train import reviewed_dataset as rd


def _write_jsonl(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")


def _png(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    Image.new("RGB", (16, 16), (10, 20, 30)).save(path)


def _fixture(tmp: Path, reviews: list[dict], labels: list[dict],
             kb_fish: list[str], wiki_keys: list[str]) -> tuple[Path, Path, Path]:
    appdata = tmp / "appdata"
    _write_jsonl(appdata / "reviews.jsonl", reviews)
    _write_jsonl(appdata / "datasets" / "gpo-vision" / "v1" / "labels.jsonl", labels)
    for lab in labels:
        _png(appdata / "datasets" / "gpo-vision" / "v1" / "images" / f"{lab['image_id']}.png")
    kb = tmp / "kb.json"
    kb.write_text(json.dumps({"entities": [{"id": i} for i in kb_fish]}), encoding="utf-8")
    wiki = tmp / "wiki.json"
    # NOTE the bracket placement: the comprehension must be INSIDE the list,
    # or this is a dict comprehension that keeps only the last key.
    wiki.write_text(json.dumps({"entries": [{"matchKey": k, "name": k} for k in wiki_keys]}),
                    encoding="utf-8")
    return appdata, kb, wiki


def _review(image: str, entity: str | None, session: str = "s1",
            status: str = "REVIEWED_CORRECTED", eligible: bool = True) -> dict:
    return {
        "image_id": image, "session_id": session, "human_entity_id": entity,
        "review_status": status, "training_eligible": eligible, "review_id": f"rev-{image}",
        "dataset_version": 1,
    }


def _label(image: str, session: str = "s1") -> dict:
    return {"image_id": image, "session_id": session, "game_state": "catch_result",
            "timestamp_ms": 1_700_000_000_000}


TH = {"entity_min_examples": 3, "entity_min_sessions": 2}


def test_a_confirmed_canonical_row_is_included_with_full_provenance() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        reviews = [_review(f"i{s}{n}", "fish:alpha", session=f"s{s}") for s in (1, 2) for n in range(3)]
        labels = [_label(f"i{s}{n}", f"s{s}") for s in (1, 2) for n in range(3)]
        appdata, kb, wiki = _fixture(tmp, reviews, labels, ["fish:alpha"], ["alpha"])
        m = rd.build(appdata, kb, wiki, TH)
        assert m["rows_included"] == 6
        assert m["class_map"] == ["fish:alpha"]
        row = m["rows"][0]
        for field in ("image_id", "session_id", "timestamp_ms", "entity_id",
                      "review_id", "review_status", "split", "dataset_version"):
            assert field in row, f"missing provenance field {field}"
        assert m["exclusions"] == {}


def test_anglerfish_style_blocked_class_is_excluded_and_reported() -> None:
    """12 rows over 9 sessions with a 20/3 gate: blocked, never trained."""
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        reviews = [_review(f"a{i}", "fish:anglerfish", session=f"s{i % 9}") for i in range(12)]
        labels = [_label(f"a{i}", f"s{i % 9}") for i in range(12)]
        appdata, kb, wiki = _fixture(tmp, reviews, labels, ["fish:anglerfish"], ["anglerfish"])
        th = {"entity_min_examples": 20, "entity_min_sessions": 3}
        m = rd.build(appdata, kb, wiki, th)
        assert m["rows_included"] == 0
        assert m["class_map"] == []
        blocked = m["blocked_classes"]["fish:anglerfish"]
        assert blocked["rows"] == 12 and blocked["sessions"] == 9
        assert "12/20" in blocked["reason"]
        assert m["exclusions"]["class-gate-blocked:fish:anglerfish"] == 12


def test_conflict_unknown_skipped_unreviewed_badimage_are_excluded() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        reviews = [
            _review("ok1", "fish:alpha", "s1"),
            _review("ok2", "fish:alpha", "s1"),
            _review("ok3", "fish:alpha", "s2"),
            _review("c1", "fish:tiger", "s1", status="CONFLICT", eligible=False),
            _review("u1", None, "s1", status="REVIEWED_UNKNOWN", eligible=False),
            _review("s1", None, "s1", status="REVIEWED_SKIPPED", eligible=False),
            _review("b1", "fish:alpha", "s1", eligible=False),  # e.g. bad image
        ]
        labels = [_label(i) for i in ("ok1", "ok2", "ok3", "c1", "u1", "s1", "b1")]
        appdata, kb, wiki = _fixture(tmp, reviews, labels, ["fish:alpha", "fish:tiger"], ["alpha"])
        m = rd.build(appdata, kb, wiki, TH)
        included = {r["image_id"] for r in m["rows"]}
        assert included == {"ok1", "ok2", "ok3"}
        assert m["exclusions"]["verdict:conflict"] == 1
        assert m["exclusions"]["verdict:reviewed_unknown"] == 1
        assert m["exclusions"]["verdict:reviewed_skipped"] == 1
        assert any(k.startswith("not-eligible:") for k in m["exclusions"])


def test_an_unreviewed_row_can_never_be_included() -> None:
    """A proposal without a human verdict is not data, whatever its score."""
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        reviews = [_review(f"r{i}", "fish:alpha", "s1") for i in range(3)]
        labels = [_label(f"r{i}", "s1") for i in range(3)] + [_label("proposal-only", "s1")]
        appdata, kb, wiki = _fixture(tmp, reviews, labels, ["fish:alpha"], ["alpha"])
        m = rd.build(appdata, kb, wiki, TH)
        assert "proposal-only" not in {r["image_id"] for r in m["rows"]}


def test_legacy_stems_and_undocumented_ids_are_never_classes() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        reviews = ([_review(f"s{i}", "fish:snapper", session=f"s{i % 2}") for i in range(6)]
                   + [_review(f"t{i}", "fish:tiger", session=f"s{i % 2}") for i in range(6)])
        labels = [_label(f"s{i}", f"s{i % 2}") for i in range(6)] + [_label(f"t{i}", f"s{i % 2}") for i in range(6)]
        # Both ids exist in the KB (historical compatibility) - but neither is
        # a documented fish, so neither may become a training class.
        appdata, kb, wiki = _fixture(tmp, reviews, labels, ["fish:snapper", "fish:tiger"], ["crimsonsnapper"])
        m = rd.build(appdata, kb, wiki, TH)
        assert m["class_map"] == []
        assert m["rows_included"] == 0
        assert m["exclusions"]["entity-not-wiki-documented:fish:snapper"] == 6
        assert m["exclusions"]["entity-not-wiki-documented:fish:tiger"] == 6


def test_class_map_is_deterministic_and_width_checked() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        # 3 rows over 2 sessions per class, so both clear the toy gate.
        reviews = ([_review(f"a{i}", "fish:beta", f"s{i % 2}") for i in range(3)]
                   + [_review(f"b{i}", "fish:alpha", f"s{i % 2}") for i in range(3)])
        labels = ([_label(f"a{i}", f"s{i % 2}") for i in range(3)]
                  + [_label(f"b{i}", f"s{i % 2}") for i in range(3)])
        appdata, kb, wiki = _fixture(tmp, reviews, labels, ["fish:alpha", "fish:beta"], ["alpha", "beta"])
        m1 = rd.build(appdata, kb, wiki, TH)
        m2 = rd.build(appdata, kb, wiki, TH)
        assert m1["class_map"] == ["fish:alpha", "fish:beta"]  # sorted, stable
        assert m1["class_map_width"] == 2
        assert m1["dataset_sha256"] == m2["dataset_sha256"]
        assert m1["vocab_sha256"] == m2["vocab_sha256"]
        assert json.dumps(m1, sort_keys=True) == json.dumps(m2, sort_keys=True)


def test_session_purity_holds_by_construction() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        sessions = [f"sess-{i:03d}" for i in range(30)]
        reviews, labels = [], []
        for i, s in enumerate(sessions):
            reviews.append(_review(f"img{i}", "fish:alpha", s))
            labels.append(_label(f"img{i}", s))
        appdata, kb, wiki = _fixture(tmp, reviews, labels, ["fish:alpha"], ["alpha"])
        m = rd.build(appdata, kb, wiki, TH)
        split_of = {}
        for row in m["rows"]:
            prev = split_of.setdefault(row["session_id"], row["split"])
            assert prev == row["split"], f"session {row['session_id']} spans splits"
        # And every split assignment matches the canonical FNV function.
        for row in m["rows"]:
            assert row["split"] == rd.fnv1a_session_split(row["session_id"])


def test_missing_image_and_non_catch_result_are_excluded_with_reasons() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        # 3 valid rows over 2 sessions (gate-clearing) + the 2 defective rows.
        reviews = [_review("r0", "fish:alpha", "s1"), _review("v1", "fish:alpha", "s1"),
                   _review("v2", "fish:alpha", "s2"),
                   _review("r1", "fish:alpha", "s1"), _review("r2", "fish:alpha", "s2")]
        labels = [_label(i, s) for i, s in
                  (("r0", "s1"), ("v1", "s1"), ("v2", "s2"), ("r1", "s1"), ("r2", "s2"))]
        labels[4]["game_state"] = "waiting_for_bite"
        appdata, kb, wiki = _fixture(tmp, reviews, labels, ["fish:alpha"], ["alpha"])
        # Remove one image after the fixture created it.
        (appdata / "datasets" / "gpo-vision" / "v1" / "images" / "r1.png").unlink()
        m = rd.build(appdata, kb, wiki, TH)
        assert {r["image_id"] for r in m["rows"]} == {"r0", "v1", "v2"}
        assert m["exclusions"]["missing-image"] == 1
        assert m["exclusions"]["not-catch-result"] == 1


def test_source_digests_are_recorded() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        reviews = [_review(f"r{i}", "fish:alpha", "s1") for i in range(3)]
        labels = [_label(f"r{i}", "s1") for i in range(3)]
        appdata, kb, wiki = _fixture(tmp, reviews, labels, ["fish:alpha"], ["alpha"])
        m = rd.build(appdata, kb, wiki, TH)
        cf = m["created_from"]
        assert len(cf["reviews_sha256"]) == 64
        assert len(cf["labels_sha256"]) == 64
        assert len(cf["kb_export_sha256"]) == 64
