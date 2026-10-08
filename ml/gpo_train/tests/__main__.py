"""Minimal test runner, matching ml.gpo_ocr_audit and ml.gpo_ontology_review.

Kept dependency-free on purpose: this project runs its Python checks with plain
`python -m ...`, so adding a test framework would be a new install step on a
machine that already has everything it needs.

Discovers every ``test_*.py`` module beside this file and runs every module
level ``test_*`` function, so a new suite is a new file and nothing else.
"""

from __future__ import annotations

import importlib
import inspect
import pkgutil
import sys
import traceback
from pathlib import Path

HERE = Path(__file__).resolve().parent


def main() -> int:
    sys.path.insert(0, str(HERE))
    sys.path.insert(0, str(HERE.parents[2]))  # repo root, for `ml.gpo_train...`
    tests: list[tuple[str, object]] = []
    for info in sorted(pkgutil.iter_modules([str(HERE)]), key=lambda m: m.name):
        if not info.name.startswith("test_"):
            continue
        mod = importlib.import_module(info.name)
        for name, fn in sorted(vars(mod).items()):
            if name.startswith("test_") and callable(fn):
                tests.append((f"{info.name}::{name}", fn))
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
    sys.exit(main())
