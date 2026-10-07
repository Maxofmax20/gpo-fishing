"""In-game ontology discovery and human review.

READ-ONLY with respect to everything that matters. This package contains no code
path that writes to the knowledge base, ``FISH_VOCAB.txt``, the dataset, the
model registry, a split, or a readiness gate.

A human decision is recorded as a PROPOSAL. Applying it to production ontology is
a separate, explicitly-approved step that this toolchain deliberately does not
perform.
"""

__all__ = [
    "schemas",
    "concept_extract",
    "evidence_select",
    "decisions",
    "patch",
    "reporting",
    "pipeline",
]