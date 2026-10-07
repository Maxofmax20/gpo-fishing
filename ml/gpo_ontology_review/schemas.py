"""Record types for the in-game ontology review.

The central idea: a HUMAN decides what the GAME is naming, and the pipeline's
job is to make that decision cheap, ordered, and backed by screenshots.

Three things are deliberately kept apart:

``Concept``
    An **exact observed game name** - one specific string the game rendered
    (``crimson snapper``). Different names are different records even when they
    share a substring, because ``tigerfin``, ``exotic tigerfin`` and
    ``golden tigerfin`` may well be three different entities. The pipeline never
    merges them and never decides they are one.

``Family``
    The set of concepts sharing a base word. Provided for navigation, explicitly
    labelled as *not* an assertion that they are the same entity. Whether a
    family is one entity, several, or a base fish plus cosmetic modifiers is
    exactly the question the human is being asked.

``Decision``
    A human verdict. ``decision_maker`` is constrained to ``"human"``; no code
    path in this package can write a decision attributed to the collector, to
    OCR, to RAM++, or to the model. Ontology ambiguity has exactly one resolver
    and it is a person.

Nothing here mutates the knowledge base, the vocabulary, a label, a split, or a
threshold. A decision becomes a *proposal*; applying it is a separate,
explicitly-approved step.
"""

from __future__ import annotations

import enum
from dataclasses import asdict, dataclass, field
from typing import Any


class OntologyStatus(str, enum.Enum):
    """What the CURRENT KB can say about an observed name.

    Note the wording throughout: "can say", not "is". Every value below describes
    the state of the knowledge base relative to the text, not the truth about
    the game.
    """

    #: the name is exactly one canonical entity
    EXACT_ENTITY = "EXACT_ENTITY"
    #: the name contains two or more canonical entities; no single answer exists
    AMBIGUOUS = "AMBIGUOUS"
    #: the name is not representable in the current KB at all
    UNKNOWN = "UNKNOWN"
    #: the name is partly explainable (one real entity plus unexplained words)
    PARTIAL = "PARTIAL"
    #: banner / UI text, not an entity name
    UI_NOISE = "UI_NOISE"


class ReviewPriority(str, enum.Enum):
    """Review ORDER only. Never affects a label.

    Thresholds are the ones specified for this campaign and are computed from the
    data, never assigned by hand.
    """

    P0 = "P0"   # >=100 occurrences, >=10 sessions, >=3 days
    P1 = "P1"   # >=25 occurrences, >=5 sessions
    P2 = "P2"   # >=5 occurrences
    P3 = "P3"   # singleton


class DecisionType(str, enum.Enum):
    """The single primary verdict a human may record for a concept."""

    #: the game names something the KB already has
    EXISTING_ENTITY = "EXISTING_ENTITY"
    #: the game names something the KB lacks; becomes a PROPOSAL, never a mutation
    NEW_ENTITY = "NEW_ENTITY"
    #: the game names a cosmetic variant of an existing entity
    VARIANT_OF_EXISTING = "VARIANT_OF_EXISTING"
    #: evidence is insufficient
    UNKNOWN = "UNKNOWN"


#: The ONLY permitted decision maker.
#:
#: Enforced on write, not just documented. A decision attributed to the
#: collector, to OCR, to RAM++ or to the model is rejected, because an automated
#: "decision" is not a decision - it is the ambiguity being laundered into
#: apparent ground truth, which is the failure this campaign exists to prevent.
ALLOWED_DECISION_MAKERS = frozenset({"human"})


@dataclass
class HumanDecision:
    """One human verdict on one concept.

    A NEW_ENTITY decision records a PROPOSAL. It does not create an entity, and
    nothing in this package writes to ``knowledge.rs`` or ``FISH_VOCAB.txt``.
    """

    concept_id: str
    decision: str
    #: required when decision is EXISTING_ENTITY or VARIANT_OF_EXISTING
    existing_entity_id: str | None = None
    #: required when decision is NEW_ENTITY
    proposed_entity_name: str | None = None
    #: optional extra modifier/appearance qualifier for a variant
    variant_descriptor: str | None = None
    #: always "human"; validated on write
    decision_maker: str = "human"
    timestamp: str = ""
    #: the images the human actually looked at, so the decision is auditable
    evidence_image_ids: list[str] = field(default_factory=list)
    notes: str = ""

    def validate(self) -> list[str]:
        """Return a list of problems; empty means valid.

        Checks the four decision shapes and rejects any attempt to attribute a
        decision to a machine.
        """
        problems: list[str] = []
        if self.decision_maker not in ALLOWED_DECISION_MAKERS:
            problems.append(
                f"decision_maker must be one of {sorted(ALLOWED_DECISION_MAKERS)}, "
                f"got {self.decision_maker!r}; only a human may resolve ontology "
                f"ambiguity"
            )
        try:
            kind = DecisionType(self.decision)
        except ValueError:
            problems.append(
                f"decision must be one of {[d.value for d in DecisionType]}, "
                f"got {self.decision!r}"
            )
            return problems

        if kind in (DecisionType.EXISTING_ENTITY, DecisionType.VARIANT_OF_EXISTING):
            if not self.existing_entity_id:
                problems.append(
                    f"{kind.value} requires existing_entity_id to name the entity "
                    f"the game is referring to"
                )
        if kind is DecisionType.NEW_ENTITY and not self.proposed_entity_name:
            problems.append("NEW_ENTITY requires proposed_entity_name")
        if kind is DecisionType.VARIANT_OF_EXISTING and not self.variant_descriptor:
            problems.append(
                "VARIANT_OF_EXISTING requires variant_descriptor, because calling "
                "something a variant is a claim about the base entity"
            )
        if not self.concept_id:
            problems.append("concept_id is required")
        return problems

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


@dataclass
class OntologyConcept:
    """One exact observed game name, with everything known about it."""

    concept_id: str
    display_name: str
    normalized_name: str
    #: Space- and punctuation-insensitive key. This is what groups OCR renderings
    #: of the SAME name together. It is a grouping key only - it never implies the
    #: two strings name different entities either.
    group_key: str

    occurrences: int = 0
    sessions: list[str] = field(default_factory=list)
    calendar_days: list[str] = field(default_factory=list)
    first_seen_ms: int = 0
    last_seen_ms: int = 0

    #: every distinct OCR string that mapped onto this concept
    ocr_variants: list[str] = field(default_factory=list)
    #: how many rows each variant contributed
    variant_counts: dict[str, int] = field(default_factory=dict)

    #: canonical entities the CURRENT KB could claim, if any. Candidates, never
    #: answers - the human is asked what the game names, not which fragment fits.
    current_kb_candidates: list[str] = field(default_factory=list)
    ontology_status: str = OntologyStatus.UNKNOWN.value

    #: linguistic decomposition. EVIDENCE ONLY - never an ontology claim.
    base_name: str | None = None
    modifiers: list[str] = field(default_factory=list)

    #: the family this concept belongs to (shared base word)
    family_id: str | None = None

    #: what the dataset currently calls these rows. EVIDENCE, NOT TRUTH: these
    #: labels were produced under the ontology now in question.
    stored_labels: dict[str, int] = field(default_factory=dict)
    total_labeled_rows: int = 0
    #: rows where a single KB candidate equals the stored label
    stored_agrees_with_candidate: int = 0

    #: deterministically chosen, deliberately diverse evidence
    representative_images: list[dict[str, Any]] = field(default_factory=list)
    all_image_ids: list[str] = field(default_factory=list)

    priority: str = ReviewPriority.P3.value
    priority_reasons: list[str] = field(default_factory=list)

    human_decision: str | None = None
    decision_note: str | None = None

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


def concept_id_for(normalized_name: str) -> str:
    """Stable, readable id: ``crimson snapper`` -> ``concept:crimson-snapper``."""
    slug = "-".join(normalized_name.split())
    return f"concept:{slug}"