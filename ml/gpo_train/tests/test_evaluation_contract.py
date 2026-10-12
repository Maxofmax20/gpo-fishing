import hashlib
import json
from types import SimpleNamespace

import pytest

from ml.gpo_train.evaluation_contract import holdout_fingerprint, new_run_directory, validate_reviewed_manifest
from ml.gpo_train.metrics import pick_rejection_threshold


def test_threshold_uses_calibrated_probabilities_once():
    result = pick_rejection_threshold([[0.99, 0.01], [0.65, 0.35]], [0, 1])
    assert result["threshold"] > 0.65
    assert result["kept"] == 0.5
    assert result["kept_accuracy"] == 1


def test_unavailable_operating_point_retains_nothing():
    result = pick_rejection_threshold([[0.9, 0.1]], [1])
    assert result == {"threshold": None, "kept": 0.0, "kept_accuracy": None, "available": False}


def test_threshold_checks_probability_contract():
    with pytest.raises(ValueError):
        pick_rejection_threshold([[2, -1]], [0])


def test_holdout_identity_tracks_images_and_labels(tmp_path):
    image = tmp_path / "a.png"
    image.write_bytes(b"first")
    row = SimpleNamespace(image_id="a", session_id="s", game_state="catch_result", entity_id="fish:a", image_path=image)
    snapshot = SimpleNamespace(rows=[row])
    original = holdout_fingerprint(snapshot, [0], ["fish:a"])
    image.write_bytes(b"second")
    assert holdout_fingerprint(snapshot, [0], ["fish:a"]) != original
    image.write_bytes(b"first")
    row.entity_id = "fish:b"
    assert holdout_fingerprint(snapshot, [0], ["fish:a"]) != original


def test_run_directory_is_immutable(tmp_path):
    new_run_directory(tmp_path, "run-1")
    with pytest.raises(FileExistsError):
        new_run_directory(tmp_path, "run-1")
    with pytest.raises(ValueError):
        new_run_directory(tmp_path, "../escape")


def test_manifest_detects_cross_split_session():
    rows = [{"image_id": f"{i:064x}", "session_id": "same", "split": split, "entity_id": "fish:a"}
            for i, split in enumerate(["train", "validation"] + ["test"] * 10)]
    vocab = ["fish:a"]
    manifest = {"class_map": vocab, "class_map_width": 1, "rows": rows,
                "vocab_sha256": hashlib.sha256(b"fish:a").hexdigest(),
                "dataset_sha256": hashlib.sha256(json.dumps({"class_map": vocab, "rows": rows}, sort_keys=True).encode()).hexdigest()}
    with pytest.raises(ValueError, match="session crosses"):
        validate_reviewed_manifest(manifest, vocab)
