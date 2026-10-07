"""GPO OCR → dataset intelligence audit (read-only).

Imports nothing from the app's control path. It has no access to game input,
macros, the controller, the model registry, or the knowledge base's mutators, and
it never writes to the dataset. Its only outputs are files under the reports
directory.

Entry point::

    python -m ml.gpo_ocr_audit
"""

__all__ = [
    "schemas",
    "text_normalizer",
    "ontology_matcher",
    "ocr_engine",
    "gpo_correlator",
    "ocr_audit",
    "report_generator",
]