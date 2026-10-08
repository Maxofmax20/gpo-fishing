"""Python half of the cross-language projection drift guard.

`src-tauri/tests/resolution_consistency.rs` verifies that the Rust projection
(`core::training::project_training_labels`) produces the same class distribution
as the manifest committed in `src-tauri/tests/fixtures/`. This test verifies the
manifest is still what `reviewed_dataset.build` produces from the fixture's own
inputs, so the Rust guard cannot pass against a stale expectation.

Synthetic inputs only; the fixture is regenerated nowhere at test time.
"""

from __future__ import annotations

import base64
import json
import tempfile
from pathlib import Path

from ml.gpo_train import reviewed_dataset

REPO_ROOT = Path(__file__).resolve().parents[3]
FIXTURE = REPO_ROOT / "src-tauri" / "tests" / "fixtures" / "projected_class_distribution.json"
PNG_1X1 = base64.b64decode(
    b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
)


def _materialise(fx: dict, tmp: Path) -> tuple[Path, Path, Path]:
    appdata = tmp / "appdata"
    images = appdata / "datasets" / "gpo-vision" / "v1" / "images"
    images.mkdir(parents=True)
    for label in fx["labels"]:
        (images / f"{label['image_id']}.png").write_bytes(PNG_1X1)
    (appdata / "datasets" / "gpo-vision" / "v1" / "labels.jsonl").write_text(
        "".join(json.dumps(l, sort_keys=True) + "\n" for l in fx["labels"]), encoding="utf-8")
    (appdata / "reviews.jsonl").write_text(
        "".join(json.dumps(r, sort_keys=True) + "\n" for r in fx["reviews"]), encoding="utf-8")
    kb = tmp / "kb_export.json"
    kb.write_text(json.dumps(fx["kb"], sort_keys=True), encoding="utf-8")
    wiki = tmp / "wiki_fish.json"
    wiki.write_text(json.dumps(fx["wiki"], sort_keys=True), encoding="utf-8")
    return appdata, kb, wiki


def test_the_committed_fixture_manifest_is_still_what_the_builder_produces() -> None:
    fx = json.loads(FIXTURE.read_text(encoding="utf-8"))
    with tempfile.TemporaryDirectory() as td:
        appdata, kb, wiki = _materialise(fx, Path(td))
        manifest = reviewed_dataset.build(appdata, kb, wiki)
    manifest.pop("created_from", None)  # absolute temp paths, excluded from the fixture
    assert manifest == fx["manifest"], (
        "the committed projection fixture no longer matches "
        "ml/gpo_train/reviewed_dataset.build; regenerate it with "
        "`python -m ml.gpo_train.make_projected_fixture` and review the diff"
    )


def test_the_fixture_exercises_every_rule_the_projection_implements() -> None:
    fx = json.loads(FIXTURE.read_text(encoding="utf-8"))
    reviews = fx["reviews"]
    assert any(r["training_eligible"] for r in reviews), "needs eligible reviews"
    assert any(not r["training_eligible"] for r in reviews), "needs exclusions"
    labels_by_id = {l["image_id"]: l for l in fx["labels"]}
    assert any(r.get("human_entity_id") and r["human_entity_id"] != labels_by_id[r["image_id"]]["entity_id"]
               for r in reviews), "needs relabelling"
    reviewed = {r["image_id"] for r in reviews}
    assert any(l["image_id"] not in reviewed for l in fx["labels"]), "needs unreviewed rows"
    assert fx["manifest"]["blocked_classes"], "needs a class-gate-blocked class"
    # Every session must be pure in one split, or the guard would be meaningless.
    splits = {}
    for row in fx["manifest"]["rows"]:
        prev = splits.setdefault(row["session_id"], row["split"])
        assert prev == row["split"], f"session {row['session_id']} spans splits"