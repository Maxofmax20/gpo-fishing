"""Test package for the OCR audit pipeline.

Deliberately dependency-free: the suite runs on a bare CPython with no pytest
installed, so a contributor can verify the pipeline's safety properties without
installing anything beyond ``requirements.txt``.

Run it with::

    python -m ml.gpo_ocr_audit.tests
"""