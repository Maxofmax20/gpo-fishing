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
#:
#: Raised from 6 to 12 for the review workstation. A concept like `crimson snapper`
#: has 276 rows across 41 sessions and 5 days, and six images cannot show the
#: reviewer enough distinct renderings to judge whether it is one thing. Twelve
#: still fits one screen at the thumbnail size the workstation uses.
MAX_EVIDENCE = 12

#: Confidence bands, used as a final diversity axis.
#:
#: Picking twelve near-identical 0.9-confidence readings shows the reviewer one
#: thing. Spanning the confidence range shows them how reliably the text is read at
#: all, which is evidence about whether the NAME is what the pixels say.
CONFIDENCE_BANDS = 4


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

    Selection order, and why:

    ==========================  ==================================================
    1. highest confidence       one immediately shows the clearest rendering
    2. distinct SESSION         the strongest evidence available: the same name
       (repeated until           produced by INDEPENDENT macro runs is not an
        exhausted)                OCR coincidence
    3. distinct DAY             recurrence across calendar days
    4. distinct OCR VARIANT     whether the game renders the name consistently
    5. distinct STORED LABEL    how much the dataset disagrees with itself
    6. distinct CONFIDENCE      how readable the text is across the concept,
       BAND (repeated)           not only at its best
    7. fill                     anything not yet chosen, deterministically
    ==========================  ==================================================

    Steps 2-6 are repeated rather than taken once, because the coverage axes
    matter more than raw count: a concept with 41 sessions should show distinct
    sessions, not eleven views of one catch.

    Session and day coverage are tried in full before confidence bands, because
    recurrence proves the name is real game output while confidence spread only
    describes readability. Getting that order wrong would spend the reviewer's
    attention on the weaker signal.
    """
    cands = list(candidates)
    if not cands:
        return []

    # Total, reproducible rank within the pool. The explicit image_id tie-break is
    # what makes the selection a pure function of the input.
    ordered = sorted(
        cands,
        key=lambda c: (-(c.ocr_confidence if c.ocr_confidence is not None else -1.0),
                       c.image_id),
    )

    confs = [c.ocr_confidence for c in ordered if c.ocr_confidence is not None]

    def band(c: Candidate) -> int | None:
        if c.ocr_confidence is None or not confs:
            return None
        lo, hi = min(confs), max(confs)
        if hi <= lo:
            return 0
        idx = int((c.ocr_confidence - lo) / (hi - lo) * CONFIDENCE_BANDS)
        return max(0, min(CONFIDENCE_BANDS - 1, idx))

    chosen: list[Candidate] = []
    seen_ids: set[str] = set()
    used_sessions: set[str] = set()
    used_days: set[str] = set()
    used_variants: set[str] = set()
    used_labels: set[str | None] = set()
    used_bands: set[int] = set()
    reasons: dict[str, str] = {}

    def add(c: Candidate, reason: str) -> None:
        chosen.append(c)
        seen_ids.add(c.image_id)
        used_sessions.add(c.session_id)
        used_days.add(c.day)
        used_variants.add(c.concept_name)
        used_labels.add(c.stored_entity_id)
        b = band(c)
        if b is not None:
            used_bands.add(b)
        reasons[c.image_id] = reason

    # 1. the single clearest reading
    add(ordered[0], "highest OCR confidence")

    def fill_until_exhausted(reason: str, pool_fn) -> None:
        """Add picks that add NEW coverage on this axis, until none remain."""
        for c in ordered:
            if len(chosen) >= limit:
                return
            if c.image_id in seen_ids:
                continue
            if not pool_fn(c):
                continue
            add(c, reason)
            # One per pass over the pool; loop until the axis is exhausted.
            break
        # repeat while progress is possible
        while len(chosen) < limit:
            progressed = False
            for c in ordered:
                if c.image_id in seen_ids:
                    continue
                if pool_fn(c):
                    add(c, reason)
                    progressed = True
                    break
            if not progressed:
                return

    # 2. sessions - strongest evidence, so it is exhausted first
    fill_until_exhausted("different session", lambda c: c.session_id not in used_sessions)
    # 3. calendar days
    if len(chosen) < limit:
        fill_until_exhausted("different calendar day", lambda c: c.day not in used_days)
    # 4. OCR renderings
    if len(chosen) < limit:
        fill_until_exhausted("different OCR variant",
                             lambda c: c.concept_name not in used_variants)
    # 5. stored labels
    if len(chosen) < limit:
        fill_until_exhausted("different stored label",
                             lambda c: c.stored_entity_id not in used_labels)
    # 6. confidence bands
    if len(chosen) < limit:
        fill_until_exhausted("different OCR confidence band",
                             lambda c: band(c) not in used_bands)
    # 7. deterministic fill
    for c in ordered:
        if len(chosen) >= limit:
            break
        if c.image_id in seen_ids:
            continue
        add(c, "additional coverage")

    return [
        {
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
        }
        for i, c in enumerate(chosen[:limit], 1)
    ]


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