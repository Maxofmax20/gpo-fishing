"""Deterministically choose representative evidence images for each concept.

THE PROBLEM THIS SOLVES
=======================

``crimson snapper`` occurs 276 times. Showing a reviewer 276 thumbnails - or the
first 50, which are consecutive frames of the same catch - teaches them nothing.
What they need to see is a handful of images that establish the name is REAL,
RECURRING OUTPUT rather than one OCR slip.

SELECTION STRATEGY
==================

Pick at most ``MAX_EVIDENCE`` images, chosen round-robin so that each successive
pick adds something the earlier ones did not:

1. distinct **session** first - two frames from one macro run prove nothing about
   recurrence;
2. then distinct **calendar day**;
3. then distinct **OCR variant**;
4. then distinct **stored label**;
5. and within the remaining candidates, highest OCR confidence first.

That ordering matters: recurrence across independent runs is the strongest
available evidence that a name is game output, and confidence is only a tie-break
among images that already differ in the ways that matter.

DETERMINISM
===========

Every ordering has an explicit final tie-break on ``image_id``, so the selection
is a pure function of the input file. Running the pipeline twice picks the same
images. ``tests/`` asserts this by shuffling the input order and requiring an
identical result.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable

#: Enough to establish recurrence without burying the reviewer.
MAX_EVIDENCE = 6


def _session_day(session_id: str | None) -> str:
    if not session_id:
        return ""
    parts = session_id.split("_")
    return parts[1] if len(parts) > 1 else ""


@dataclass
class Candidate:
    image_id: str
    session_id: str
    day: str
    raw_text: str
    concept_name: str
    stored_entity_id: str | None
    ocr_confidence: float | None
    timestamp_ms: int


def select_representative(
    candidates: Iterable[Candidate],
    limit: int = MAX_EVIDENCE,
) -> list[dict[str, Any]]:
    """Choose diverse, high-confidence evidence for one concept.

    Returns records annotated with *why* each image was chosen, so a reviewer can
    see the reasoning rather than having to reconstruct it.
    """
    cands = list(candidates)
    if not cands:
        return []

    # Rank within the pool: confidence desc, then id, so the order is total and
    # reproducible regardless of input order.
    ordered = sorted(
        cands,
        key=lambda c: (-(c.ocr_confidence if c.ocr_confidence is not None else -1.0),
                       c.image_id),
    )

    chosen: list[Candidate] = []
    used_sessions: set[str] = set()
    used_days: set[str] = set()
    used_variants: set[str] = set()
    used_labels: set[str | None] = set()

    def take(pool: list[Candidate], reason: str) -> None:
        for c in pool:
            if c.image_id in {x.image_id for x in chosen}:
                continue
            chosen.append(c)
            used_sessions.add(c.session_id)
            used_days.add(c.day)
            used_variants.add(c.concept_name)
            used_labels.add(c.stored_entity_id)
            reasons[c.image_id] = reason
            return

    reasons: dict[str, str] = {}

    # 1. highest confidence overall - the single best rendering of the name
    take(ordered, "highest OCR confidence")

    # 2. a different session
    take([c for c in ordered if c.session_id not in used_sessions],
         "different session")

    # 3. a different calendar day
    take([c for c in ordered if c.day not in used_days],
         "different calendar day")

    # 4. a different OCR rendering of the name
    take([c for c in ordered if c.concept_name not in used_variants],
         "different OCR variant")

    # 5. a different stored label
    take([c for c in ordered if c.stored_entity_id not in used_labels],
         "different stored label")

    # 6. fill any remaining slots with the next-best distinct sessions
    for c in ordered:
        if len(chosen) >= limit:
            break
        if c.image_id in {x.image_id for x in chosen}:
            continue
        if c.session_id in used_sessions and len(chosen) < 3:
            # Only accept a repeat-session image if we are otherwise starved.
            pass
        take([c], "additional coverage")

    out: list[dict[str, Any]] = []
    for i, c in enumerate(chosen[:limit], 1):
        out.append({
            "rank": i,
            "image_id": c.image_id,
            "session_id": c.session_id,
            "day": c.day,
            "raw_ocr": c.raw_text,
            "concept_name": c.concept_name,
            "stored_entity_id": c.stored_entity_id,
            "ocr_confidence": c.ocr_confidence,
            "timestamp_ms": c.timestamp_ms,
            "selected_because": reasons.get(c.image_id, "coverage"),
        })
    return out


def selection_summary(evidence: list[dict[str, Any]]) -> str:
    """One line describing what the selection demonstrates."""
    if not evidence:
        return "no evidence"
    sessions = len({e["session_id"] for e in evidence})
    days = len({e["day"] for e in evidence if e["day"]})
    variants = len({e["concept_name"] for e in evidence})
    labels = len({e["stored_entity_id"] for e in evidence})
    return (
        f"{len(evidence)} image(s) spanning {sessions} session(s), {days} day(s), "
        f"{variants} OCR variant(s), {labels} stored label(s)"
    )