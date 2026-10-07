"""Append-only store for human ontology decisions.

THE CENTRAL SAFETY PROPERTY
===========================

A ``NEW_ENTITY`` decision records a **PROPOSAL**. It does not create an entity.

There is deliberately no code path anywhere in this package that writes to
``knowledge.rs``, ``FISH_VOCAB.txt``, the dataset, or the model registry. The
workflow is:

    OBSERVED GAME NAME
            ↓
    HUMAN PROPOSAL                    ← this module, and nothing else
            ↓
    ONTOLOGY REVIEW REPORT
            ↓
    EXPLICIT APPROVAL (a person)
            ↓
    KB / VOCAB UPDATE

The gap between step 2 and step 4 is the whole point. An ontology that silently
acquires entities from OCR is the exact failure that produced the current
contamination: names appeared because a resolver produced them, not because
anyone decided the game contains them.

WHO MAY DECIDE
==============

``decision_maker`` must be ``"human"``. The collector, OCR, RAM++ and the model
are all rejected on write. An automated "decision" is not a decision - it is the
ambiguity being laundered into apparent ground truth.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable

from .schemas import (
    ALLOWED_DECISION_MAKERS,
    DecisionType,
    HumanDecision,
)

#: Machine identities that are structurally forbidden from being a decision maker.
FORBIDDEN_MAKERS = ("collector", "ocr", "ram", "ram++", "ram_plus", "model",
                    "onnx", "auto", "pipeline", "semantic_teacher")


class DecisionRejected(ValueError):
    """Raised when a decision may not be recorded. Never swallowed."""


@dataclass
class DecisionStore:
    """Append-only JSONL of human decisions.

    Append-only on purpose: overwriting a decision would erase the fact that it
    was ever made differently, which is exactly the audit trail this campaign
    needs. The LAST decision for a concept is the current one; earlier ones remain
    on disk.
    """

    path: Path

    def __post_init__(self) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)

    # -- reading ---------------------------------------------------------

    def all(self) -> list[HumanDecision]:
        if not self.path.is_file():
            return []
        out: list[HumanDecision] = []
        with self.path.open(encoding="utf-8") as fh:
            for n, line in enumerate(fh, 1):
                line = line.strip()
                if not line:
                    continue
                try:
                    data = json.loads(line)
                except json.JSONDecodeError as exc:
                    raise DecisionRejected(
                        f"{self.path.name}:{n} is not valid JSON: {exc}; the "
                        f"decision log is append-only and must not be corrupted"
                    ) from exc
                out.append(HumanDecision(**data))
        return out

    def current(self) -> dict[str, HumanDecision]:
        """Latest decision per concept, in file order."""
        out: dict[str, HumanDecision] = {}
        for d in self.all():
            out[d.concept_id] = d
        return out

    def history_for(self, concept_id: str) -> list[HumanDecision]:
        return [d for d in self.all() if d.concept_id == concept_id]

    # -- writing ---------------------------------------------------------

    def record(self, decision: HumanDecision) -> HumanDecision:
        """Validate and append. Raises rather than writing anything invalid.

        Returns the stored decision so a caller can echo the exact record.
        """
        problems = decision.validate()
        if problems:
            raise DecisionRejected(
                "decision rejected:\n  - " + "\n  - ".join(problems)
            )
        maker = decision.decision_maker.strip().lower()
        if maker in FORBIDDEN_MAKERS:
            raise DecisionRejected(
                f"decision_maker {decision.decision_maker!r} is a machine identity; "
                f"only a human may resolve ontology ambiguity"
            )
        if maker not in ALLOWED_DECISION_MAKERS:
            raise DecisionRejected(
                f"decision_maker must be one of {sorted(ALLOWED_DECISION_MAKERS)}"
            )

        with self.path.open("a", encoding="utf-8", newline="\n") as fh:
            fh.write(json.dumps(decision.to_dict(), sort_keys=True, ensure_ascii=False) + "\n")
        return decision

    def apply_to(self, concepts: Iterable[Any]) -> int:
        """Stamp the current decision onto each concept record.

        Returns how many concepts were decided. Nothing here can create a concept
        or change a count; it only attaches a verdict to a record that already
        exists.
        """
        current = self.current()
        n = 0
        for c in concepts:
            d = current.get(c.concept_id)
            if d is not None:
                c.human_decision = d.decision
                c.decision_note = d.notes
                n += 1
        return n


# --------------------------------------------------------------------------
# Derived views over the decisions
# --------------------------------------------------------------------------

def proposals(decisions: Iterable[HumanDecision]) -> list[dict[str, Any]]:
    """Human PROPOSALS for entities the KB lacks.

    This is a proposal list, not an entity list. Applying it is a separate,
    explicitly-approved step performed by ``export``, which writes a PATCH FILE -
    it still does not modify production.
    """
    out = []
    for d in decisions:
        if d.decision != DecisionType.NEW_ENTITY.value:
            continue
        out.append({
            "concept_id": d.concept_id,
            "proposed_entity_name": d.proposed_entity_name,
            "notes": d.notes,
            "evidence_image_ids": d.evidence_image_ids,
            "decided_at": d.timestamp,
        })
    out.sort(key=lambda p: (-len(p["evidence_image_ids"]), p["concept_id"]))
    return out


def decision_counts(decisions: Iterable[HumanDecision]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for d in decisions:
        counts[d.decision] = counts.get(d.decision, 0) + 1
    return dict(sorted(counts.items()))


def training_ready_counts(concepts: Iterable[Any],
                          decisions: dict[str, HumanDecision]) -> dict[str, Any]:
    """How many HUMAN-CONFIRMED examples exist per REAL in-game entity.

    This is the number that matters for training, and it deliberately does NOT use
    stored collector labels. Those labels were produced under the ontology now in
    question; counting them would be counting the contamination.

    Only concepts a human resolved to a single canonical entity contribute.
    """
    per_entity: dict[str, dict[str, Any]] = {}
    for c in concepts:
        d = decisions.get(c.concept_id)
        if d is None:
            continue
        if d.decision == DecisionType.EXISTING_ENTITY.value and d.existing_entity_id:
            target = d.existing_entity_id
            kind = "existing"
        elif d.decision == DecisionType.NEW_ENTITY.value and d.proposed_entity_name:
            target = f"proposed:{d.proposed_entity_name}"
            kind = "proposed"
        elif d.decision == DecisionType.VARIANT_OF_EXISTING.value and d.existing_entity_id:
            target = d.existing_entity_id
            kind = "variant_of"
        else:
            continue
        e = per_entity.setdefault(target, {
            "entity_id": target,
            "kind": kind,
            "human_confirmed_rows": 0,
            "concepts": [],
            "sessions": set(),
            "days": set(),
        })
        e["human_confirmed_rows"] += c.occurrences
        e["concepts"].append({
            "concept_id": c.concept_id,
            "display_name": c.display_name,
            "rows": c.occurrences,
        })
        e["sessions"].update(c.sessions)
        e["days"].update(c.calendar_days)

    for e in per_entity.values():
        e["session_count"] = len(e["sessions"])
        e["day_count"] = len(e["days"])
        e["concepts"].sort(key=lambda x: -x["rows"])
        e.pop("sessions", None)
        e.pop("days", None)

    return {
        "entities": sorted(per_entity.values(),
                           key=lambda x: (-x["human_confirmed_rows"], x["entity_id"])),
        "note": (
            "Counts HUMAN-CONFIRMED rows only. Stored collector labels are "
            "excluded by design: they were produced under the ontology now in "
            "question, so counting them would count the contamination."
        ),
    }