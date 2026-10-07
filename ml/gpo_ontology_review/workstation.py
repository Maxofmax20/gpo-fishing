"""Build the compact index and thumbnails the review workstation loads.

WHY A SEPARATE INDEX
====================

``concepts.json`` is ~300 KB and carries ``all_image_ids`` - up to 308 ids for one
concept. The workstation needs a concept's summary and its twelve representative
images; it never needs the full id list. Shipping the 300 KB file to a browser and
parsing it per keystroke is the kind of thing that makes a review tool feel slow, so
the build step emits a second, purpose-built artefact:

``workstation/index.json``
    Per-concept summary, no ``all_image_ids``, plus the review order.
``workstation/kb_entities.json``
    Canonical KB entities for the searchable "existing entity" picker, with
    aliases, category and rarity.
``workstation/structural.json``
    Modifiers and families - the evidence that many decisions are NOT independent.
``workstation/thumbs/<image_id>.jpg``
    Deterministic thumbnails, generated once at build time.

``concepts.json`` is left exactly as it was. The markdown reports and any existing
consumer keep working; this is an additional artefact, not a replacement.

THUMBNAILS
==========

Generated here rather than on demand because resizing in Rust on every request
would put image work in the request path, which is precisely what makes a UI feel
heavy. Fixed width, fixed quality, deterministic: identical bytes for identical
input, so the directory is reproducible and can be regenerated or deleted freely.

Every thumbnail is keyed by IMAGE id, not by concept, because the same image can
appear in several concepts and in a family comparison.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any

from .concept_extract import build_base_report, build_families, build_modifier_report
from .schemas import OntologyConcept

#: Thumbnail width in pixels. Chosen to stay legible at the workstation's grid
#: cell size on a 1366-wide window while remaining small on disk.
THUMB_WIDTH = 320

#: JPEG quality. 82 keeps glyph edges readable - these crops are mostly small text,
#: and aggressive compression turns `Swordfish` into a smudge, which is exactly the
#: information the reviewer is reading.
THUMB_QUALITY = 82


def thumb_filename(image_id: str) -> str:
    return f"{image_id}.jpg"


def build_index(
    concepts: list[OntologyConcept],
    kb_payload: dict,
    modifiers: list[dict[str, Any]],
    families: dict[str, dict[str, Any]],
    bases: list[dict[str, Any]],
) -> dict[str, Any]:
    """Compact per-concept index plus the deterministic review order.

    Review order is computed here, once, so the UI never re-derives it and two
    clients always see the same order:

        P0, P1, P2, P3
          then occurrences desc
          then sessions desc
          then days desc
          then display_name asc   (total, so the order cannot wobble)
    """
    order = sorted(
        concepts,
        key=lambda c: (
            {"P0": 0, "P1": 1, "P2": 2, "P3": 3}.get(c.priority, 9),
            -c.occurrences,
            -len(c.sessions),
            -len(c.calendar_days),
            c.display_name,
        ),
    )

    out_concepts = []
    for rank, c in enumerate(order):
        out_concepts.append({
            "rank": rank,
            "conceptId": c.concept_id,
            "displayName": c.display_name,
            "normalizedName": c.normalized_name,
            "groupKey": c.group_key,
            "occurrences": c.occurrences,
            "sessionCount": len(c.sessions),
            "dayCount": len(c.calendar_days),
            "priority": c.priority,
            "priorityReasons": c.priority_reasons,
            "ontologyStatus": c.ontology_status,
            "currentKbCandidates": c.current_kb_candidates,
            "baseName": c.base_name,
            "modifiers": c.modifiers,
            "familyId": c.family_id,
            "storedLabels": [[k, v] for k, v in c.stored_labels.items()],
            "storedAgreesWithCandidate": c.stored_agrees_with_candidate,
            "ocrVariants": c.ocr_variants[:16],
            "variantCounts": c.variant_counts,
            # `representative_images` holds dicts (select_representative returns
            # plain records), so this reads them by key.
            "evidence": [
                {
                    "imageId": e["image_id"],
                    "sessionId": e["session_id"],
                    "day": e["day"],
                    "rawOcr": e["raw_ocr"],
                    "storedEntityId": e["stored_entity_id"],
                    "ocrConfidence": e["ocr_confidence"],
                    "timestampMs": e["timestamp_ms"],
                    "selectedBecause": e["selected_because"],
                }
                for e in c.representative_images
            ],
            "humanDecision": c.human_decision,
            "decisionNote": c.decision_note,
        })

    return {
        "kind": "ontology_review_workstation_index",
        "version": 1,
        "ordering": "priority band, then occurrences, sessions, days, display name",
        "counts": {
            "total": len(concepts),
            "p0": sum(1 for c in concepts if c.priority == "P0"),
            "p1": sum(1 for c in concepts if c.priority == "P1"),
            "p2": sum(1 for c in concepts if c.priority == "P2"),
            "p3": sum(1 for c in concepts if c.priority == "P3"),
        },
        "concepts": out_concepts,
    }


def build_kb_entities(kb_payload: dict) -> dict[str, Any]:
    """Searchable canonical KB entities for the "existing entity" picker.

    Sorted by id so the client's own filter is stable. Includes aliases because
    the reviewer may recognise `Barrier` while searching for something else.
    """
    out = []
    for e in kb_payload["entities"]:
        out.append({
            "id": e["id"],
            "canonicalName": e["canonical_name"],
            "aliases": e.get("aliases") or [],
            "ocrAliases": e.get("ocr_aliases") or [],
            "category": e["category"],
            "rarity": e.get("rarity"),
            "inFishVocab": e["id"] in kb_payload.get("fish_vocab", []),
        })
    out.sort(key=lambda x: x["id"])
    return {
        "kind": "ontology_review_kb_entities",
        "kbVersion": kb_payload.get("kb_version"),
        "entityCount": len(out),
        "fishVocab": kb_payload.get("fish_vocab", []),
        "entities": out,
    }


def build_structural(
    modifiers: list[dict[str, Any]],
    families: dict[str, dict[str, Any]],
    bases: list[dict[str, Any]],
) -> dict[str, Any]:
    """Cross-concept signals, so the reviewer can see which decisions are NOT
    independent.

    The important example: `golden` precedes three different bases, and `crimson`
    precedes two. Deciding `golden tigerfin` and `golden polka puffer` separately,
    without noticing they share a modifier, is how an inconsistent ontology gets
    built. The panel makes the relationship visible; it never draws a conclusion.
    """
    return {
        "kind": "ontology_review_structural",
        "disclaimer": (
            "Evidence only. A modifier recurring across bases shows a PATTERN, not a "
            "role. Whether it is a colour, a rarity, or a species is a human decision."
        ),
        "modifiers": [
            {
                "modifier": m["modifier"],
                "occurrences": m["occurrences"],
                "conceptCount": m["concept_count"],
                "sessionCount": m["session_count"],
                "dayCount": m["day_count"],
                "bases": m["bases"],
                "concepts": [
                    {"conceptId": c["concept_id"], "displayName": c["display_name"],
                     "occurrences": c["occurrences"]}
                    for c in m["concepts"][:10]
                ],
            }
            for m in modifiers
        ],
        "families": [
            {
                "familyId": f["family_id"],
                "baseName": f["base_name"],
                "memberCount": f["member_count"],
                "isSingleton": f["is_singleton"],
                "occurrences": f["occurrences"],
                "sessionCount": f["session_count"],
                "dayCount": f["day_count"],
                # Renamed field-by-field rather than passed through: `build_families`
                # works in snake_case internally, but every OTHER key in this
                # artefact is camelCase for the TypeScript client. Passing the
                # members through verbatim once made the family panel read
                # `conceptId`, find nothing, and silently render an empty family
                # - a bug the real-data end-to-end test caught.
                "members": [
                    {
                        "conceptId": m["concept_id"],
                        "displayName": m["display_name"],
                        "modifiers": m.get("modifiers", []),
                        "occurrences": m.get("occurrences", 0),
                    }
                    for m in f["members"]
                ],
            }
            for f in sorted(families.values(), key=lambda x: -x["occurrences"])
        ],
        "bases": [
            {
                "baseName": b["base_name"],
                "occurrences": b["occurrences"],
                "conceptCount": b["concept_count"],
                "sessionCount": b["session_count"],
                "dayCount": b["day_count"],
                "occursBare": b["occurs_bare"],
                "bareConcept": b["bare_concept"],
                "kbCandidates": b["kb_candidates"],
                "modifierTotals": b["modifier_totals"],
                # Same reason as the family members above: renamed, not passed through.
                "concepts": [
                    {
                        "conceptId": c["concept_id"],
                        "displayName": c["display_name"],
                        "occurrences": c["occurrences"],
                        "modifiers": c.get("modifiers", []),
                    }
                    for c in b["concepts"][:10]
                ],
            }
            for b in bases
        ],
    }


def render_thumbs(
    concepts: list[OntologyConcept],
    images_dir: Path,
    out_dir: Path,
    width: int = THUMB_WIDTH,
    quality: int = THUMB_QUALITY,
) -> dict[str, Any]:
    """Write one deterministic thumbnail per referenced image.

    Returns stats including how many source images were missing, which must be
    reported rather than hidden: a concept whose evidence cannot be rendered is a
    problem the reviewer needs to see.
    """
    try:
        from PIL import Image
    except ImportError:
        return {"rendered": 0, "skipped": 0, "missing": 0, "error": "Pillow not installed"}

    out_dir.mkdir(parents=True, exist_ok=True)
    wanted: set[str] = set()
    for c in concepts:
        for e in c.representative_images:
            wanted.add(e["image_id"])

    rendered = missing = 0
    for image_id in sorted(wanted):
        src = images_dir / f"{image_id}.png"
        dest = out_dir / thumb_filename(image_id)
        if not src.is_file():
            missing += 1
            continue
        try:
            with Image.open(src) as im:
                im = im.convert("RGB")
                # Only downscale; never upscale a small crop.
                if im.width > width:
                    h = max(1, round(im.height * width / im.width))
                    im = im.resize((width, h), Image.LANCZOS)
                im.save(dest, "JPEG", quality=quality, optimize=True)
            rendered += 1
        except Exception:
            missing += 1

    return {
        "rendered": rendered,
        "missing": missing,
        "referenced": len(wanted),
        "width": width,
        "quality": quality,
    }


def index_digest(payload: dict[str, Any]) -> str:
    """Stable digest of the index, recorded in the build summary."""
    blob = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(blob).hexdigest()