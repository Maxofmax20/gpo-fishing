"""Record types and the audit taxonomy for the GPO OCR audit pipeline.

Two INDEPENDENT axes describe every image, and keeping them separate is the
whole point of the exercise:

``image_category``
    What the frame *is* (a fish drop, a fruit drop, idle UI, a banner ...).
    Derived from the collector's region metadata plus what the text says.

``audit_status``
    What the TEXT ANALYSIS concluded about the stored label. Exactly one of
    these per image.

They are deliberately not merged. A fish drop whose text is unreadable is
``FISH_DROP`` + ``OCR_UNREADABLE``; a fish drop whose text names three unknown
things is ``FISH_DROP`` + ``UNKNOWN_ENTITY``. Collapsing them would hide exactly
the rows a human needs to see.

Nothing in this module writes anything. The types describe records; producing
them is the pipeline's job, and they carry no authority over labels, the KB, or
the model registry.
"""

from __future__ import annotations

import enum
from dataclasses import dataclass, field, asdict
from typing import Any


class ImageCategory(str, enum.Enum):
    """What kind of frame this is. Derived, never assumed from the label."""

    FISH_DROP = "FISH_DROP"
    FRUIT_DROP = "FRUIT_DROP"
    BAIT_DROP = "BAIT_DROP"
    UI = "UI"
    WAITING = "WAITING"
    BITE = "BITE"
    BANNER = "BANNER"
    UNKNOWN = "UNKNOWN"
    OCR_FAILURE = "OCR_FAILURE"


class MatchKind(str, enum.Enum):
    """HOW a text reached an entity.

    Kept separate from :class:`AuditStatus` so a fuzzy match can never be
    presented as an exact one. The distinction is load-bearing: an OCR-folded
    match (`0/o`, `1/l`, `5/s`) is lossy and must not be reported as if the text
    literally said the entity's name.
    """

    #: raw text is byte-identical to a canonical name
    EXACT = "EXACT"
    #: differs only by case / punctuation / whitespace / unicode folding
    NORMALIZED = "NORMALIZED"
    #: reached through a KB-declared alias
    ALIAS = "ALIAS"
    #: reached through an explicitly registered OCR correction rule
    OCR_VARIANT = "OCR_VARIANT"
    #: reached through lossy confusable folding (`ocr_variant_fold`)
    CONFUSABLE = "CONFUSABLE"
    #: edit-distance candidate; a suggestion, never a resolution
    FUZZY = "FUZZY"
    #: the entity name occurs INSIDE a longer word, so it does not resolve
    #: anything - e.g. `tiger` inside `tigerfin`. A suggestion at best.
    PARTIAL = "PARTIAL"
    #: two or more distinct entities matched
    AMBIGUOUS = "AMBIGUOUS"
    #: nothing matched
    NONE = "NONE"


class OntologyStatus(str, enum.Enum):
    """What the knowledge base knows about the text."""

    EXACT_ENTITY = "EXACT_ENTITY"
    ALIAS_ENTITY = "ALIAS_ENTITY"
    OCR_VARIANT_ENTITY = "OCR_VARIANT_ENTITY"
    CONFUSABLE_ENTITY = "CONFUSABLE_ENTITY"
    FUZZY_CANDIDATE = "FUZZY_CANDIDATE"
    COMPOUND = "COMPOUND"
    UNKNOWN_ENTITY = "UNKNOWN_ENTITY"
    UI_TEMPLATE = "UI_TEMPLATE"
    NO_ENTITY_TEXT = "NO_ENTITY_TEXT"
    UNREADABLE = "UNREADABLE"


class AuditStatus(str, enum.Enum):
    """The single primary verdict per image.

    Precedence, highest first (see ``classify`` in :mod:`ontology_matcher`)::

        OCR_UNREADABLE
        NON_FISH
        COMPOUND_AMBIGUOUS
        UNKNOWN_ENTITY
        OCR_CONFLICT
        CONFIRMED_MATCH
        ALIAS_MATCH
        OCR_VARIANT_MATCH
        NO_DECISION
    """

    #: OCR reached exactly the stored canonical entity
    CONFIRMED_MATCH = "CONFIRMED_MATCH"
    #: OCR reached the stored entity only after a registered OCR correction
    OCR_VARIANT_MATCH = "OCR_VARIANT_MATCH"
    #: OCR reached the stored entity through a declared alias
    ALIAS_MATCH = "ALIAS_MATCH"
    #: OCR names two or more canonical candidates; no single answer exists
    COMPOUND_AMBIGUOUS = "COMPOUND_AMBIGUOUS"
    #: OCR names something with no KB entity. Never auto-created.
    UNKNOWN_ENTITY = "UNKNOWN_ENTITY"
    #: OCR reaches a DIFFERENT canonical entity than the one stored
    OCR_CONFLICT = "OCR_CONFLICT"
    #: no usable text
    OCR_UNREADABLE = "OCR_UNREADABLE"
    #: text carries no entity at all (idle bar, banner, spawn notice ...)
    NON_FISH = "NON_FISH"
    #: one candidate but nothing to compare against / not enough to decide
    NO_DECISION = "NO_DECISION"

    @property
    def needs_human(self) -> bool:
        """Does a human have to look at this?

        True for everything except a clean confirmation and a clean
        non-entity frame. Deliberately generous: under-flagging here is how
        contaminated data reaches training.
        """
        return self not in (
            AuditStatus.CONFIRMED_MATCH,
            AuditStatus.NON_FISH,
        )

    @property
    def is_confirmable(self) -> bool:
        """Could this row ever serve as evidence for its stored label?

        Only an unambiguous confirmation. Compound, unknown, conflict and
        unreadable rows are all non-confirmable regardless of how confident the
        OCR was.
        """
        return self in (
            AuditStatus.CONFIRMED_MATCH,
            AuditStatus.ALIAS_MATCH,
            AuditStatus.OCR_VARIANT_MATCH,
        )


@dataclass(frozen=True)
class NormalizationStep:
    """ONE transformation, recorded.

    Normalisation is evidence-producing, so every change is logged rather than
    applied silently. A reader must be able to replay the pipeline from the raw
    text alone.
    """

    stage: str
    """`unicode_fold` | `normalize` | `ocr_variant_fold` | `correction_rule`"""
    frm: str
    to: str
    reason: str

    def to_dict(self) -> dict[str, str]:
        return asdict(self)


@dataclass
class CompoundFinding:
    """One place inside a name where a canonical entity's text occurs.

    ``crimson snapper`` yields two of these. Neither is chosen.

    ``whole_word`` is the safety-critical field. It is True only when the span
    lines up with word boundaries in the source text. A hit inside a longer word
    (``tiger`` inside ``tigerfin``) is recorded but flagged, because resolving
    it would be the exact mistake that produced the original contamination.
    """

    token: str
    entity_id: str
    canonical_name: str
    match_kind: str
    whole_word: bool = True
    start: int = 0


@dataclass
class EntityEvidence:
    """One image's complete, self-contained analysis."""

    image_id: str
    image_sha256: str
    relative_image_path: str
    session_id: str
    timestamp_ms: int

    # ---- what the dataset currently says -------------------------------
    stored_entity_id: str | None
    stored_source: str
    stored_ocr_text: str | None

    # ---- what a fresh OCR engine read off the pixels -------------------
    ocr_engine: str
    ocr_engine_version: str
    raw_ocr: str
    normalized_ocr: str
    ocr_confidence: float | None

    # ---- what the ontology says about that text ------------------------
    match_kind: str
    ontology_status: str

    normalization_steps: list[NormalizationStep] = field(default_factory=list)
    matched_entities: list[str] = field(default_factory=list)
    candidate_entities: list[str] = field(default_factory=list)
    compound: bool = False
    compound_tokens: list[CompoundFinding] = field(default_factory=list)
    ui_templates: list[str] = field(default_factory=list)
    residual_text: str = ""
    fuzzy_candidates: list[str] = field(default_factory=list)
    correction_rules_applied: list[str] = field(default_factory=list)

    # ---- verdicts --------------------------------------------------------
    image_category: str = ImageCategory.UNKNOWN.value
    audit_status: str = AuditStatus.NO_DECISION.value
    requires_human_review: bool = True
    review_priority: int = 0
    priority_reasons: list[str] = field(default_factory=list)

    # ---- context ---------------------------------------------------------
    game_state: str = ""
    region_name: str = ""
    ocr_error: str = ""
    rejected_detections: int = 0
    #: entity_id -> KB category for each candidate, so a report can flag
    #: "a FRUIT candidate inside a fish drop".
    candidate_categories: dict[str, str] = field(default_factory=dict)
    candidate_categories_mismatched: bool = False
    human_reviewed: bool = False
    human_review_status: str | None = None
    in_latest_snapshot: bool = False
    split: str | None = None
    model_prediction: str | None = None
    model_confidence: float | None = None

    # ---- traceability ----------------------------------------------------
    explanation: str = ""

    def to_dict(self) -> dict[str, Any]:
        """Serialise with no wall-clock field.

        Determinism (phase 28) requires two runs over the same immutable dataset
        to produce equivalent evidence, so nothing time-dependent may appear
        here. Generation time lives in ``provenance.json`` instead.
        """
        d = asdict(self)
        d["normalization_steps"] = [s.to_dict() for s in self.normalization_steps]
        d["compound_tokens"] = [asdict(t) for t in self.compound_tokens]
        return d