"""Minimal test runner, matching ml.gpo_ocr_audit and ml.gpo_ontology_review.

Kept dependency-free on purpose: this project runs its Python checks with plain
`python -m ...`, so adding a test framework would be a new install step on a
machine that already has everything it needs.
"""

from __future__ import annotations

import importlib
import inspect
import sys
import traceback


def main() -> int:
    import test_candidates as mod

    tests = [(n, f) for n, f in vars(mod).items()
             if n.startswith("test_") and callable(f)]
    passed = failed = 0
    for name, fn in tests:
        try:
            # Every fixture here is optional, so the signature is inspected rather
            # than forcing one shape on all of them.
            if len(inspect.signature(fn).parameters) == 0:
                fn()
            else:
                fn(None)
            print(f"PASS {name}")
            passed += 1
        except Exception:
            print(f"FAIL {name}")
            traceback.print_exc()
            failed += 1
    print(f"\n{passed}/{passed + failed} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.path.insert(0, str(__import__("pathlib").Path(__file__).parent))
    sys.exit(main())
