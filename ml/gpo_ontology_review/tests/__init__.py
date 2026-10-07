"""Ontology discovery and human review.

Read-only with respect to everything that matters: this package contains no code
path that writes to the knowledge base, ``FISH_VOCAB.txt``, the dataset, the
model registry, a split, or a readiness gate.

A human decision is recorded as a PROPOSAL. Applying it to production ontology is
a separate, explicitly-approved step that this toolchain deliberately does not
perform.
"""

from __future__ import annotations

import importlib
import pkgutil
import sys
import traceback
from pathlib import Path

HERE = Path(__file__).resolve().parent
ML_DIR = HERE.parents[1]
REPO_ROOT = HERE.parents[3]


def _ensure_path() -> None:
    for p in (str(REPO_ROOT), str(ML_DIR)):
        if p not in sys.path:
            sys.path.insert(0, p)


def discover() -> list[tuple[str, object]]:
    _ensure_path()
    found: list[tuple[str, object]] = []
    for info in sorted(pkgutil.iter_modules([str(HERE)]), key=lambda m: m.name):
        if not info.name.startswith("test_"):
            continue
        mod = importlib.import_module(f"{__package__}.{info.name}")
        for name in sorted(vars(mod)):
            fn = getattr(mod, name)
            if name.startswith("test_") and callable(fn):
                found.append((f"{info.name}::{name}", fn))
    return found


def main() -> int:
    tests = discover()
    if not tests:
        print("no tests found", file=sys.stderr)
        return 1
    failed: list[tuple[str, str]] = []
    for name, fn in tests:
        try:
            fn()
            print(f"PASS {name}")
        except AssertionError as exc:
            failed.append((name, f"AssertionError: {exc}"))
            print(f"FAIL {name}: {exc}")
        except Exception as exc:  # noqa: BLE001
            detail = "".join(traceback.format_exception_only(type(exc), exc)).strip()
            failed.append((name, detail))
            print(f"ERROR {name}: {detail}")

    print()
    print(f"{len(tests) - len(failed)}/{len(tests)} passed")
    if failed:
        print("\nfailures:")
        for name, detail in failed:
            print(f"  {name}: {detail}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())