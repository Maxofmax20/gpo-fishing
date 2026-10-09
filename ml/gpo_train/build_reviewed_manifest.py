"""Materialise the training manifest that `fish_train_v2.py` reads.

The manifest is an OUTPUT under `ml/output/`, NOT app state. It is built from the
live store read-only and the committed stratified-split artifact, applying the
override map explicitly instead of reading it back from
`%APPDATA%\\gpo-autofish\\datasets\\gpo-vision\\v1\\manifest.json`.

That distinction is deliberate: the operator applies the override map to the
live manifest with `python -m ml.gpo_train.apply_split_overrides`, and until
they do, the two are intentionally out of step. Building the training manifest
from the COMMITTED map keeps the artifact reproducible (its SHA-256 stamps
`reports/resolution/split_overrides.json`) while leaving app state alone.

Deterministic: same inputs, byte-identical manifest.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT))

from ml.gpo_train.reviewed_dataset import build as build_manifest  # noqa: E402
from ml.gpo_train.vocab_decision import FISH_VOCAB, parse_vocab  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--overrides", type=Path,
                    default=REPO_ROOT / "reports" / "resolution" / "split_overrides.json")
    ap.add_argument("--out", type=Path,
                    default=REPO_ROOT / "ml" / "output" / "reviewed-fish-v1" / "dataset_manifest.json")
    ap.add_argument("--appdata", type=Path,
                    default=Path(os.environ.get("APPDATA", "")) / "gpo-autofish")
    args = ap.parse_args()

    vocab = parse_vocab(FISH_VOCAB.read_text(encoding="utf-8"))
    overrides = json.loads(args.overrides.read_text(encoding="utf-8"))["split_overrides"]

    manifest = build_manifest(
        args.appdata,
        kb_path=REPO_ROOT / "ml" / "gpo_ocr_audit" / "kb_export.json",
        wiki_path=REPO_ROOT / "reports" / "ontology_review" / "wiki_fish.json",
        overrides=overrides,
    )
    manifest["provenance"] = {
        "built_by": "ml/gpo_train/build_reviewed_manifest.py",
        "split_overrides_sha256": hashlib.sha256(
            args.overrides.read_bytes()).hexdigest(),
        "split_overrides_applied": len(overrides),
        "vocab_source": "FISH_VOCAB.txt (root)",
        "vocab_sha256": manifest["vocab_sha256"],
        "note": (
            "manifest built with the COMMITTED stratified override map, which the "
            "operator applies to the live dataset manifest separately"
        ),
    }

    def canonical(b):
        return json.dumps(b, sort_keys=True)

    assert manifest["class_map"] == vocab, (
        "the training manifest's class map drifted from FISH_VOCAB.txt"
    )

    out = args.out if args.out.is_absolute() else REPO_ROOT / args.out
    out.parent.mkdir(parents=True, exist_ok=True)
    blob = json.dumps(manifest, indent=1, sort_keys=True) + "\n"
    out.write_text(blob, encoding="utf-8")
    # round-trip proof: what landed is what we meant
    assert canonical(json.loads(out.read_text(encoding="utf-8"))) == canonical(manifest)

    pc = {e: d["test_support"] for e, d in manifest["per_class"].items()}
    print(f"  class_map_width: {manifest['class_map_width']} rows_included: {manifest['rows_included']}")
    print(f"  splits: {json.dumps(manifest['splits'])}")
    print(f"  dataset_sha256: {manifest['dataset_sha256']}")
    print(f"  per-class TEST support:")
    for e in sorted(pc):
        flag = "OK " if pc[e] >= 10 else "LOW"
        print(f"    [{flag}] {e:<30} {pc[e]:>3}")
    print(f"  below 10: {sorted(e for e, v in pc.items() if v < 10) or 'none'}")
    print(f"  written: {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
