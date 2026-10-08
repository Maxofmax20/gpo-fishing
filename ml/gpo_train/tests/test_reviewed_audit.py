"""Tests for the rebuilt-dataset audit: fail-closed on leakage and bad images."""

from __future__ import annotations

import json
import tempfile
from pathlib import Path

import numpy as np
from PIL import Image

from ml.gpo_train import reviewed_audit as ra


def _manifest(rows: list[tuple[str, str, str]]) -> dict:
    """rows: (image_id, split, entity_id)."""
    return {
        "dataset_sha256": "x" * 64,
        "rows": [
            {"image_id": i, "split": s, "entity_id": e, "session_id": f"sess-{i}"}
            for i, s, e in rows
        ],
    }


def _appdata_with_images(tmp: Path, images: dict[str, np.ndarray | bytes | None]) -> Path:
    appdata = tmp / "appdata"
    images_dir = appdata / "datasets" / "gpo-vision" / "v1" / "images"
    images_dir.mkdir(parents=True)
    for image_id, content in images.items():
        p = images_dir / f"{image_id}.png"
        if content is None:
            continue  # missing file
        if isinstance(content, bytes):
            p.write_bytes(content)
        else:
            Image.fromarray(content).save(p)
    return appdata


def _img(seed: int, w: int = 32, h: int = 16) -> np.ndarray:
    rng = np.random.default_rng(seed)
    return rng.integers(0, 255, (h, w, 3), dtype=np.uint8)


def test_a_clean_dataset_passes() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        rows = [("a", "train", "fish:x"), ("b", "test", "fish:x"), ("c", "train", "fish:y")]
        appdata = _appdata_with_images(tmp, {"a": _img(1), "b": _img(2), "c": _img(3)})
        report = ra.audit(_manifest(rows), appdata)
        assert report["verdict"] == "PASS"
        assert report["quality_failures"] == []
        assert report["duplicates"]["byte_groups_cross_split"] == 0


def test_byte_identical_images_crossing_splits_fail_closed() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        same = _img(42)
        rows = [("a", "train", "fish:x"), ("b", "test", "fish:x"), ("c", "validation", "fish:y")]
        appdata = _appdata_with_images(tmp, {"a": same, "b": same, "c": _img(3)})
        report = ra.audit(_manifest(rows), appdata)
        assert report["verdict"] == "FAIL"
        assert any("byte-identical" in f for f in report["failures"])
        grp = report["groups"]["byte_cross_split"][0]
        assert set(grp["splits"]) == {"train", "test"}
        assert set(grp["image_ids"]) == {"a", "b"}


def test_an_undecodable_image_fails_closed() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        rows = [("a", "train", "fish:x"), ("b", "test", "fish:y")]
        appdata = _appdata_with_images(tmp, {"a": b"this is not a png", "b": _img(2)})
        report = ra.audit(_manifest(rows), appdata)
        assert report["verdict"] == "FAIL"
        assert any("undecodable" in q["reason"] for q in report["quality_failures"])


def test_a_dark_frame_is_a_quality_failure_not_a_silent_drop() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        dark = np.zeros((16, 32, 3), dtype=np.uint8)
        rows = [("a", "train", "fish:x"), ("b", "test", "fish:y")]
        appdata = _appdata_with_images(tmp, {"a": dark, "b": _img(2)})
        report = ra.audit(_manifest(rows), appdata)
        assert report["verdict"] == "FAIL"
        assert any("dark-frame" in q["reason"] for q in report["quality_failures"])


def test_same_split_twins_are_reported_but_do_not_fail() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        same = _img(7)
        rows = [("a", "train", "fish:x"), ("b", "train", "fish:x"), ("c", "test", "fish:y")]
        appdata = _appdata_with_images(tmp, {"a": same, "b": same, "c": _img(3)})
        report = ra.audit(_manifest(rows), appdata)
        assert report["verdict"] == "PASS"
        assert report["duplicates"]["byte_groups"] == 1
        assert report["duplicates"]["byte_groups_cross_split"] == 0
