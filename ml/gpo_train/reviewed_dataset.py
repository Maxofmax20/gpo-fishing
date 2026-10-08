"""Rebuild the fish training dataset from the AUTHORITATIVE current state.

WHY NOT THE OLD PATH
====================

``fish_train.py`` (fish_v1) trains on ``labels.jsonl.entity_id`` - the OLD
bare-stem labels. The human review phase replaced those judgements: the
authoritative label for a reviewed image is now the review record's
``human_entity_id`` (16 canonical, wiki-documented fish), and inclusion is the
review store's ``training_eligible`` flag, recomputed against the EFFECTIVE KB.

This module rebuilds the dataset from exactly that state:

    reviews.jsonl   (human verdicts + training_eligible)
        x labels.jsonl (image path, session, capture timestamp, game_state)
        x effective KB (canonical membership)
        x class gate (ENTITY_MIN_EXAMPLES / ENTITY_MIN_SESSIONS, parsed from
          ml_capability.rs so it cannot drift from the code)

INCLUSION (all required)
    review record exists
    AND review_status in {REVIEWED_CORRECT, REVIEWED_CORRECTED}
    AND training_eligible is true
    AND human_entity_id is an effective-KB entity
    AND the entity's class clears the training gate (>=20 eligible rows over
        >=3 sessions, measured over the eligible reviewed universe)
    AND a labels.jsonl row exists with game_state == "catch_result"
    AND the image file is present on disk

EXCLUSIONS are counted per reason - nothing is silently dropped. Anglerfish
rows stay valid and row-eligible but are excluded here because their CLASS
gate is blocked (12/20); when the gate clears, the same builder includes them
with no code change.

DETERMINISM
    Rows are sorted by image_id; the manifest digest covers the class map and
    every included row. Same inputs -> byte-identical manifest.

The module never writes reviews.jsonl, labels.jsonl, the KB, or FISH_VOCAB.txt.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]
KB_EXPORT = REPO_ROOT / "ml" / "gpo_ocr_audit" / "kb_export.json"
ML_CAPABILITY = REPO_ROOT / "src-tauri" / "src" / "core" / "ml_capability.rs"
OUTPUT_ROOT = REPO_ROOT / "ml" / "output"

#: Review verdicts that permit training (mirrors core::review::is_reviewed,
#: minus REVIEWED_UNKNOWN, which carries no entity).
TRAINABLE_STATUSES = {"REVIEWED_CORRECT", "REVIEWED_CORRECTED"}

DATASET_KIND = "reviewed-fish-dataset"
DATASET_VERSION = 1


def _sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def _sha256_file(p: Path) -> str:
    return _sha256_bytes(p.read_bytes())


def load_thresholds() -> dict[str, int]:
    """Parse the training gate from the Rust source so it cannot drift."""
    import re

    src = ML_CAPABILITY.read_text(encoding="utf-8")
    out: dict[str, int] = {}
    for const in ("ENTITY_MIN_EXAMPLES", "ENTITY_MIN_SESSIONS"):
        m = re.search(rf"pub const {const}: usize = (\d+);", src)
        if not m:
            raise SystemExit(f"cannot parse {const} from {ML_CAPABILITY}")
        out[const.lower()] = int(m.group(1))
    return out


def fnv1a_session_split(session_id: str) -> str:
    """Byte-identical port of MlDatasetStore::split_of (FNV-1a64 70/15/15)."""
    h = 0xCBF29CE484222325
    for b in session_id.encode("utf-8"):
        h ^= b
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    r = h % 100
    if r <= 69:
        return "train"
    if r <= 84:
        return "validation"
    return "test"


def _load_jsonl(path: Path) -> list[dict[str, Any]]:
    if not path.is_file():
        raise SystemExit(f"required input missing: {path}")
    rows = []
    with path.open(encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            if not line.strip():
                continue
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError as e:
                raise SystemExit(f"{path}:{n}: unparseable line ({e}); refusing to "
                                 f"build because exclusions could not be verified")
    return rows


def load_effective_kb_ids(kb_path: Path = KB_EXPORT) -> set[str]:
    payload = json.loads(kb_path.read_text(encoding="utf-8"))
    return {e["id"] for e in payload["entities"]}


WIKI_SNAPSHOT = REPO_ROOT / "reports" / "ontology_review" / "wiki_fish.json"


def load_wiki_fish_ids(wiki_path: Path = WIKI_SNAPSHOT) -> set[str]:
    """`fish:<matchKey>` for every fish the wiki bestiary documents.

    A training class must be a DOCUMENTED fish. This is what keeps a legacy
    bare stem (`fish:snapper`) or an undocumented id (`fish:tiger`) out of the
    class map even if enough reviewed rows ever reference it: the KB-patch
    contract created canonical entities only from wiki evidence, and the
    training vocabulary inherits that rule.
    """
    snap = json.loads(wiki_path.read_text(encoding="utf-8"))
    return {f"fish:{e['matchKey']}" for e in snap["entries"]}


def build(appdata: Path, kb_path: Path = KB_EXPORT,
          wiki_path: Path = WIKI_SNAPSHOT,
          thresholds: dict[str, int] | None = None) -> dict[str, Any]:
    """Build the deterministic dataset manifest. Pure: writes nothing."""
    th = thresholds or load_thresholds()
    reviews_path = appdata / "reviews.jsonl"
    labels_path = appdata / "datasets" / "gpo-vision" / "v1" / "labels.jsonl"
    images_dir = appdata / "datasets" / "gpo-vision" / "v1" / "images"

    reviews = _load_jsonl(reviews_path)
    labels = _load_jsonl(labels_path)
    labels_by_image = {l["image_id"]: l for l in labels if l.get("image_id")}
    kb_ids = load_effective_kb_ids(kb_path)
    wiki_ids = load_wiki_fish_ids(wiki_path)

    exclusions: dict[str, int] = {}

    def bump(reason: str) -> None:
        exclusions[reason] = exclusions.get(reason, 0) + 1

    # ---- first pass: the eligible reviewed universe, per class -------------
    per_class: dict[str, dict[str, Any]] = {}
    candidates: list[dict[str, Any]] = []
    for r in reviews:
        image_id = r.get("image_id", "")
        status = r.get("review_status", "")
        entity = r.get("human_entity_id")
        if status not in TRAINABLE_STATUSES:
            bump(f"verdict:{status.lower() or 'missing'}")
            continue
        if not r.get("training_eligible"):
            bump(f"not-eligible:{r.get('excluded_reason') or 'unspecified'}")
            continue
        if not entity:
            bump("no-human-entity")
            continue
        if entity not in kb_ids:
            bump("entity-not-in-kb")
            continue
        if entity not in wiki_ids:
            # Legacy bare stem (fish:snapper) or undocumented id (fish:tiger):
            # the KB may carry it for historical compatibility, but it is not a
            # documented fish and can never be a canonical training class.
            bump(f"entity-not-wiki-documented:{entity}")
            continue
        lab = labels_by_image.get(image_id)
        if lab is None:
            bump("missing-labels-row")
            continue
        if lab.get("game_state") != "catch_result":
            bump("not-catch-result")
            continue
        if not (images_dir / f"{image_id}.png").is_file():
            bump("missing-image")
            continue
        candidates.append({"review": r, "label": lab, "entity": entity})
        c = per_class.setdefault(entity, {"rows": 0, "sessions": set()})
        c["rows"] += 1
        c["sessions"].add(r.get("session_id", ""))

    # ---- class gate: which classes may train --------------------------------
    qualified: list[str] = []
    blocked: dict[str, dict[str, Any]] = {}
    for entity in sorted(per_class):
        c = per_class[entity]
        ok = (c["rows"] >= th["entity_min_examples"]
              and len(c["sessions"]) >= th["entity_min_sessions"])
        if ok:
            qualified.append(entity)
        else:
            blocked[entity] = {
                "rows": c["rows"],
                "sessions": len(c["sessions"]),
                "reason": (f"class gate blocked: {c['rows']}/{th['entity_min_examples']} rows, "
                           f"{len(c['sessions'])}/{th['entity_min_sessions']} sessions"),
            }
    class_map = sorted(qualified)  # alphabetical: stable logit order
    in_scope = set(class_map)

    # ---- second pass: emit rows for qualified classes only ------------------
    rows: list[dict[str, Any]] = []
    for cand in candidates:
        entity = cand["entity"]
        if entity not in in_scope:
            bump(f"class-gate-blocked:{entity}")
            continue
        r, lab = cand["review"], cand["label"]
        rows.append({
            "image_id": r["image_id"],
            "session_id": r.get("session_id", ""),
            "timestamp_ms": lab.get("timestamp_ms", 0),
            "entity_id": entity,
            "split": fnv1a_session_split(r.get("session_id", "")),
            "review_id": r.get("review_id", ""),
            "review_status": r.get("review_status", ""),
            "dataset_version": r.get("dataset_version", 0),
            "hard_example": bool(lab.get("hard_example", False)),
            "ocr_text_present": bool((lab.get("ocr_text") or "").strip()),
        })
    rows.sort(key=lambda x: x["image_id"])

    # ---- session purity guard (must be impossible by construction) ----------
    seen: dict[str, str] = {}
    for row in rows:
        prev = seen.setdefault(row["session_id"], row["split"])
        if prev != row["split"]:
            raise SystemExit(f"session {row['session_id']} spans splits; refusing")

    split_rows = {"train": 0, "validation": 0, "test": 0}
    split_sessions: dict[str, set[str]] = {"train": set(), "validation": set(), "test": set()}
    split_days: dict[str, set[str]] = {"train": set(), "validation": set(), "test": set()}
    from datetime import datetime, timezone
    for row in rows:
        s = row["split"]
        split_rows[s] += 1
        split_sessions[s].add(row["session_id"])
        if row["timestamp_ms"]:
            split_days[s].add(
                datetime.fromtimestamp(row["timestamp_ms"] / 1000, tz=timezone.utc).date().isoformat()
            )

    per_class_counts = {e: sum(1 for row in rows if row["entity_id"] == e) for e in class_map}
    per_class_sessions = {
        e: len({row["session_id"] for row in rows if row["entity_id"] == e}) for e in class_map
    }
    val_support = {e: sum(1 for row in rows if row["entity_id"] == e and row["split"] == "validation")
                   for e in class_map}
    test_support = {e: sum(1 for row in rows if row["entity_id"] == e and row["split"] == "test")
                    for e in class_map}

    vocab_sha = _sha256_bytes("\n".join(class_map).encode("utf-8"))
    digest_payload = json.dumps({"class_map": class_map, "rows": rows}, sort_keys=True)
    dataset_digest = _sha256_bytes(digest_payload.encode("utf-8"))

    return {
        "kind": DATASET_KIND,
        "dataset_version": DATASET_VERSION,
        "created_from": {
            "reviews": str(reviews_path),
            "reviews_sha256": _sha256_file(reviews_path),
            "labels": str(labels_path),
            "labels_sha256": _sha256_file(labels_path),
            "kb_export": str(kb_path),
            "kb_export_sha256": _sha256_file(kb_path),
        },
        "thresholds": th,
        "class_map": class_map,
        "class_map_width": len(class_map),
        "vocab_sha256": vocab_sha,
        "dataset_sha256": dataset_digest,
        "rows_included": len(rows),
        "rows": rows,
        "exclusions": dict(sorted(exclusions.items())),
        "blocked_classes": blocked,
        "per_class": {
            e: {"rows": per_class_counts[e], "sessions": per_class_sessions[e],
                "val_support": val_support[e], "test_support": test_support[e]}
            for e in class_map
        },
        "splits": {
            s: {"rows": split_rows[s],
                "sessions": len(split_sessions[s]),
                "days": len(split_days[s])}
            for s in ("train", "validation", "test")
        },
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--appdata", type=Path,
                    default=Path(os.environ.get("APPDATA", "")) / "gpo-autofish")
    ap.add_argument("--out", type=Path, default=None,
                    help="manifest output path (default: ml/output/reviewed-fish-v1/dataset_manifest.json)")
    args = ap.parse_args()

    manifest = build(args.appdata)
    out = args.out or (OUTPUT_ROOT / "reviewed-fish-v1" / "dataset_manifest.json")
    if not out.is_absolute():
        out = REPO_ROOT / out
    out.parent.mkdir(parents=True, exist_ok=True)
    # Deterministic bytes: sorted keys, fixed separators, trailing newline.
    text = json.dumps(manifest, indent=1, sort_keys=True) + "\n"
    out.write_text(text, encoding="utf-8")

    print(f"  class map ({manifest['class_map_width']}): {manifest['class_map']}")
    print(f"  included: {manifest['rows_included']} rows")
    print(f"  splits  : {json.dumps(manifest['splits'])}")
    print(f"  blocked : {json.dumps(manifest['blocked_classes'])}")
    print(f"  excluded: {json.dumps(manifest['exclusions'])}")
    print(f"  dataset sha256: {manifest['dataset_sha256']}")
    print(f"  vocab   sha256: {manifest['vocab_sha256']}")
    print(f"  written: {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
