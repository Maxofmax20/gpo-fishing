"""Apply the stratified split overrides to the dataset manifest.

READ-MOSTLY, ONE WRITE. `stratified_split.py` produces the override map as a
committed artifact (`reports/resolution/split_overrides.json`) and never touches
`%APPDATA%`; this is the one-line command the operator runs to apply it.

    python -m ml.gpo_train.apply_split_overrides --dry-run   # show, write nothing
    python -m ml.gpo_train.apply_split_overrides             # write the manifest

THE WRITE IS SAFE BY CONSTRUCTION
  * it writes `manifest.json` atomically (temp file + rename), so a crash cannot
    leave a torn manifest;
  * it preserves every other manifest field, including `name`, `version` and
    `created_ms`;
  * it merges rather than replaces, so an unrelated override someone added by
    hand is not silently dropped - it is reported instead;
  * it refuses to write if the resulting manifest is not valid JSON, and it
    re-reads and re-parses the written bytes before reporting success.

Nothing else in the store is touched. `labels.jsonl`, `reviews.jsonl` and the
image files are read, never written.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--overrides", type=Path,
                    default=REPO_ROOT / "reports" / "resolution" / "split_overrides.json")
    ap.add_argument("--appdata", type=Path,
                    default=Path(os.environ.get("APPDATA", "")) / "gpo-autofish")
    ap.add_argument("--dry-run", action="store_true",
                    help="print the diff and write nothing")
    args = ap.parse_args()

    payload = json.loads(args.overrides.read_text(encoding="utf-8"))
    new_overrides = payload.get("split_overrides") or {}
    if not new_overrides:
        raise SystemExit(f"{args.overrides} carries no split_overrides; nothing to apply")

    manifest_path = args.appdata / "datasets" / "gpo-vision" / "v1" / "manifest.json"
    if not manifest_path.is_file():
        raise SystemExit(
            f"no dataset manifest at {manifest_path}. Initialise the dataset store "
            "in the app first (Dataset > init); a store with no manifest cannot "
            "record a split decision."
        )
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    current = dict(manifest.get("split_overrides") or {})

    # An operator's own overrides are never silently replaced.
    added = {k: v for k, v in new_overrides.items() if k not in current}
    changed = {k: v for k, v in new_overrides.items() if k in current and current[k] != v}
    untouched = {k: v for k, v in current.items() if k not in new_overrides}
    if untouched:
        print(f"  keeping {len(untouched)} override(s) already present and not in the map:")
        for k in sorted(untouched):
            print(f"    {k} -> {untouched[k]} (unchanged)")

    merged = dict(current)
    merged.update(new_overrides)

    print(f"  manifest: {manifest_path}")
    print(f"  split_overrides: {len(current)} -> {len(merged)} "
          f"(+{len(added)} new, {len(changed)} changed)")
    for k in sorted(added):
        print(f"    + {k} -> {added[k]}")
    for k in sorted(changed):
        print(f"    ~ {k}: {current[k]} -> {changed[k]}")

    if args.dry_run:
        print("  --dry-run: nothing written")
        return 0

    out = dict(manifest)
    out["split_overrides"] = merged
    if "split_strategy" in out:
        out["split_strategy"] = (
            str(out["split_strategy"]).rstrip(".")
            + "; stratified group assignment via split_overrides "
              "(reports/resolution/split_overrides.json)"
        )
    blob = json.dumps(out, indent=1, sort_keys=True) + "\n"

    # Atomic: write a sibling temp file in the SAME directory (same filesystem,
    # so the rename cannot fail across devices), then rename over the target.
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(
        dir=str(manifest_path.parent), prefix=".manifest-", suffix=".tmp"
    )
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(blob)
            fh.flush()
            os.fsync(fh.fileno())
        os.replace(tmp_name, manifest_path)
    except BaseException:
        Path(tmp_name).unlink(missing_ok=True)
        raise

    # Verify what actually landed, rather than assuming the write worked.
    back = json.loads(manifest_path.read_text(encoding="utf-8"))
    assert back.get("split_overrides") == merged, "the written manifest does not round-trip"
    assert set(back.keys()) >= {"name", "version", "split_strategy"}, \
        "a manifest field was lost by the write"
    print(f"  wrote {manifest_path} ({len(merged)} override(s)), verified by re-reading")
    return 0


if __name__ == "__main__":
    sys.exit(main())
