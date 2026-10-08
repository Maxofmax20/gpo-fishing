"""One-off generator for src-tauri/tests/fixtures/projected_class_distribution.json.

NOT part of any suite: it writes the shared fixture that the Rust drift guard
(`src-tauri/tests/projected_labels_consistency.rs`) and the Python regeneration
test both read. Kept out of the suites so a stale expectation cannot be
"refreshed" by running the tool it is supposed to contradict - the Python test
re-derives the expectation from the embedded inputs instead.
"""

from __future__ import annotations

import base64
import json
import shutil
import tempfile
from pathlib import Path

import sys

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT))

from ml.gpo_train import reviewed_dataset  # noqa: E402

OUT = REPO_ROOT / "src-tauri" / "tests" / "fixtures" / "projected_class_distribution.json"
PNG_1X1 = base64.b64decode(
    b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
)

SPECIES = ["fish:anglerfish", "fish:candycornsquid", "fish:crimsonsnapper", "fish:zebraribbonangelfish"]
STEMS = ["fish:crimson", "fish:snapper", "fish:squid"]

KB = {
    "schema": 1,
    "kb_version": 1,
    "entities": [
        {"id": i, "canonical_name": i.split(":", 1)[1].title(), "aliases": [], "ocr_aliases": [],
         "category": "fish", "rarity": None, "description": None, "visual_refs": [],
         "provenance": {"source": "fixture", "url": None, "updated": None}}
        for i in sorted(SPECIES + STEMS)
    ],
}
WIKI = {"entries": [{"matchKey": i.split(":", 1)[1]} for i in sorted(SPECIES)]}


def _label(image_id: str, session: str, entity: str | None, ms: int) -> dict:
    return {
        "image_id": image_id,
        "dataset_version": 1,
        "session_id": session,
        "task": "entity_recognition",
        "ocr_text": f"<{entity or 'none'}>",
        "region_name": "drop",
        "ui_label": None,
        "bbox": None,
        "game_state": "catch_result",
        "entity_id": entity,
        "annotator": "collector",
        "timestamp_ms": ms,
        "source": "ui",
        "confidence": 0.9,
        "hard_example": image_id.endswith("7"),
        "hard_reason": None,
        "corrections": [],
        "event_id": None,
        "frame_index": None,
    }


def build_inputs() -> tuple[list[dict], list[dict]]:
    """Collector labels + human reviews. The shape that matters: humans move
    most stem rows to species, reject a few, and leave some unreviewed."""
    labels: list[dict] = []
    reviews: list[dict] = []
    ms = 1_700_000_000_000

    def add(image_id: str, session: str, entity: str, human: str | None, eligible: bool = True) -> None:
        nonlocal ms
        ms += 1000
        labels.append(_label(image_id, session, entity, ms))
        if human is None and eligible:
            return  # unreviewed: kept by the projection, absent from the manifest
        reviews.append({
            "review_id": f"rv-{image_id}",
            "image_id": image_id,
            "session_id": session,
            "review_status": "REVIEWED_CORRECTED" if eligible else "REVIEWED_SKIPPED",
            "training_eligible": eligible,
            "human_entity_id": human if eligible else None,
            "excluded_reason": None if eligible else "human skipped",
            "dataset_version": 1,
        })

    # 30 crimson rows: 16 -> crimson snapper, 8 -> candy corn squid, 6 unreviewed.
    for i in range(30):
        sess = f"sess-crimson-{i % 6}"
        human = "fish:crimsonsnapper" if i < 16 else ("fish:candycornsquid" if i < 24 else None)
        add(f"crimson-{i:03d}", sess, "fish:crimson", human)
    # 25 snapper rows, every one of them a human-verified crimson snapper.
    for i in range(25):
        add(f"snapper-{i:03d}", f"sess-snapper-{i % 5}", "fish:snapper", "fish:crimsonsnapper")
    # 22 squid rows: 20 -> candy corn squid, 2 rejected outright.
    for i in range(22):
        sess = f"sess-squid-{i % 4}"
        if i < 20:
            add(f"squid-{i:03d}", sess, "fish:squid", "fish:candycornsquid")
        else:
            add(f"squid-{i:03d}", sess, "fish:squid", None, eligible=False)
    # 25 zebra ribbon angelfish rows across 5 sessions, confirmed as collected.
    for i in range(25):
        add(f"zebra-{i:03d}", f"sess-zebra-{i % 5}", "fish:zebraribbonangelfish",
            "fish:zebraribbonangelfish")
    # 12 anglerfish rows over 3 sessions: below the class gate on purpose.
    for i in range(12):
        add(f"angler-{i:03d}", f"sess-angler-{i % 3}", "fish:anglerfish", "fish:anglerfish")
    return labels, reviews


def main() -> int:
    labels, reviews = build_inputs()
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        appdata = tmp / "appdata"
        images = appdata / "datasets" / "gpo-vision" / "v1" / "images"
        images.mkdir(parents=True)
        for l in labels:
            (images / f"{l['image_id']}.png").write_bytes(PNG_1X1)
        (appdata / "datasets" / "gpo-vision" / "v1" / "labels.jsonl").write_text(
            "".join(json.dumps(l, sort_keys=True) + "\n" for l in labels), encoding="utf-8")
        (appdata / "reviews.jsonl").write_text(
            "".join(json.dumps(r, sort_keys=True) + "\n" for r in reviews), encoding="utf-8")
        kb = tmp / "kb_export.json"
        kb.write_text(json.dumps(KB, sort_keys=True), encoding="utf-8")
        wiki = tmp / "wiki_fish.json"
        wiki.write_text(json.dumps(WIKI, sort_keys=True), encoding="utf-8")
        manifest = reviewed_dataset.build(appdata, kb, wiki)

    # `created_from` carries absolute paths of a temp dir, which would make the
    # fixture machine-specific; every other field is path-free and deterministic.
    manifest.pop("created_from", None)
    fixture = {
        "kind": "projected-class-distribution-fixture",
        "version": 1,
        "note": (
            "Shared cross-language fixture. `manifest` is what "
            "ml/gpo_train/reviewed_dataset.py builds from `labels` + `reviews` "
            "(KB + wiki passed in `kb`/`wiki`); the Rust guard re-derives the same "
            "distribution from core::training::project_training_labels."
        ),
        "thresholds": manifest["thresholds"],
        "labels": labels,
        "reviews": reviews,
        "kb": KB,
        "wiki": WIKI,
        "manifest": manifest,
    }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(fixture, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {OUT}")
    print(f"  rows_included={manifest['rows_included']} class_map={manifest['class_map']}")
    print(f"  blocked={list(manifest['blocked_classes'])}")
    shutil.rmtree(OUT.parent / "__pycache__", ignore_errors=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())