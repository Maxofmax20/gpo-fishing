"""Fresh duplicate / leakage / quality audit for the rebuilt 15-class dataset.

WHY FRESH
=========

The old audit numbers (253 aHash groups crossing splits) describe the OLD
unreviewed dataset with its OLD labels and OLD splits. This audit measures the
rebuilt reviewed dataset (``reviewed_dataset.py``) as it actually is.

WHAT IS MEASURED
================

Quality (per included row):
    zero-byte file, undecodable image, non-RGB-convertible, degenerate
    dimensions, blank / extremely dark / extremely bright frames.

Duplicates:
    byte-identical (sha256 of the file)
    pixel-identical (sha256 of the decoded RGB array)
    aHash twins (8x8 average hash, exact 64-bit equality)

Leakage:
    any of the above grouped ACROSS splits. The split is session-pure by
    construction, so a byte/pixel twin crossing splits means the same capture
    landed in two splits through TWO sessions - real leakage.

FAIL CLOSED
    * any included image is undecodable or zero-byte
    * any byte-identical or pixel-identical pair crosses splits
    aHash cross-split twins are REPORTED (near-identical captures of genuinely
    different fish of the same class are the classifier's whole problem), but
    they are counted and listed so the number is never a surprise.

Writes <manifest dir>/dataset_audit.json. Reads nothing else mutable; never
writes to any source file.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any

import numpy as np
from PIL import Image

REPO_ROOT = Path(__file__).resolve().parents[2]

AHASH_SIZE = 8
#: mean-pixel bounds for "blank"/"extreme" frames (0..255).
DARK_MEAN = 5.0
BRIGHT_MEAN = 250.0


def _ahash(img: Image.Image) -> int:
    g = np.asarray(img.convert("L").resize((AHASH_SIZE, AHASH_SIZE), Image.BILINEAR), dtype=np.float64)
    mean = g.mean()
    bits = (g > mean).flatten()
    out = 0
    for b in bits:
        out = (out << 1) | int(b)
    return out


def audit(manifest: dict[str, Any], appdata: Path) -> dict[str, Any]:
    images_dir = appdata / "datasets" / "gpo-vision" / "v1" / "images"
    rows = manifest["rows"]

    quality_failures: list[dict[str, Any]] = []
    byte_groups: dict[str, list[int]] = defaultdict(list)
    pixel_groups: dict[str, list[int]] = defaultdict(list)
    ahash_groups: dict[int, list[int]] = defaultdict(list)
    dims: dict[str, int] = defaultdict(int)

    for idx, row in enumerate(rows):
        image_id = row["image_id"]
        path = images_dir / f"{image_id}.png"
        if not path.is_file():
            quality_failures.append({"image_id": image_id, "reason": "missing-file"})
            continue
        size = path.stat().st_size
        if size == 0:
            quality_failures.append({"image_id": image_id, "reason": "zero-byte"})
            continue
        raw = path.read_bytes()
        byte_groups[hashlib.sha256(raw).hexdigest()].append(idx)
        try:
            img = Image.open(path).convert("RGB")
            arr = np.asarray(img)
        except Exception as e:  # noqa: BLE001 - any decode failure is a failure
            quality_failures.append({"image_id": image_id, "reason": f"undecodable: {type(e).__name__}"})
            continue
        w, h = img.size
        dims[f"{w}x{h}"] += 1
        if w < 8 or h < 8:
            quality_failures.append({"image_id": image_id, "reason": f"degenerate-dimensions:{w}x{h}"})
            continue
        mean = float(arr.mean())
        if mean <= DARK_MEAN:
            quality_failures.append({"image_id": image_id, "reason": f"dark-frame:mean={mean:.1f}"})
        elif mean >= BRIGHT_MEAN:
            quality_failures.append({"image_id": image_id, "reason": f"bright-frame:mean={mean:.1f}"})
        pixel_groups[hashlib.sha256(arr.tobytes()).hexdigest()].append(idx)
        ahash_groups[_ahash(img)].append(idx)

    def groups_crossing(groups: dict, what: str) -> list[dict[str, Any]]:
        out = []
        for key, idxs in sorted(groups.items(), key=lambda kv: str(kv[0])):
            if len(idxs) < 2:
                continue
            splits = sorted({rows[i]["split"] for i in idxs})
            classes = sorted({rows[i]["entity_id"] for i in idxs})
            sessions = sorted({rows[i]["session_id"] for i in idxs})
            out.append({
                "kind": what,
                "size": len(idxs),
                "splits": splits,
                "classes": classes,
                "sessions": len(sessions),
                "cross_split": len(splits) > 1,
                "cross_class": len(classes) > 1,
                "image_ids": sorted(rows[i]["image_id"] for i in idxs),
            })
        return out

    byte_g = groups_crossing(byte_groups, "byte")
    pixel_g = groups_crossing(pixel_groups, "pixel")
    ahash_g = groups_crossing(ahash_groups, "ahash")

    cross_split_byte = [g for g in byte_g if g["cross_split"]]
    cross_split_pixel = [g for g in pixel_g if g["cross_split"]]
    cross_split_ahash = [g for g in ahash_g if g["cross_split"]]
    cross_class_ahash = [g for g in ahash_g if g["cross_class"]]

    failures: list[str] = []
    if quality_failures:
        failures.append(f"{len(quality_failures)} quality failure(s)")
    if cross_split_byte:
        failures.append(f"{len(cross_split_byte)} byte-identical group(s) cross splits")
    if cross_split_pixel:
        failures.append(f"{len(cross_split_pixel)} pixel-identical group(s) cross splits")

    return {
        "kind": "reviewed-fish-dataset-audit",
        "dataset_sha256": manifest["dataset_sha256"],
        "rows_audited": len(rows),
        "dimensions": dict(sorted(dims.items())),
        "quality_failures": quality_failures,
        "duplicates": {
            "byte_groups": len(byte_g),
            "byte_groups_cross_split": len(cross_split_byte),
            "pixel_groups": len(pixel_g),
            "pixel_groups_cross_split": len(cross_split_pixel),
            "ahash_twin_groups": len(ahash_g),
            "ahash_twin_groups_cross_split": len(cross_split_ahash),
            "ahash_twin_groups_cross_class": len(cross_class_ahash),
        },
        "groups": {
            "byte_cross_split": cross_split_byte,
            "pixel_cross_split": cross_split_pixel,
            "ahash_cross_split": cross_split_ahash,
            "ahash_cross_class": cross_class_ahash,
        },
        "verdict": "FAIL" if failures else "PASS",
        "failures": failures,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--appdata", type=Path,
                    default=Path(os.environ.get("APPDATA", "")) / "gpo-autofish")
    ap.add_argument("--manifest", type=Path,
                    default=REPO_ROOT / "ml" / "output" / "reviewed-fish-v1" / "dataset_manifest.json")
    ap.add_argument("--out", type=Path, default=None)
    args = ap.parse_args()

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    report = audit(manifest, args.appdata)
    out = args.out or args.manifest.with_name("dataset_audit.json")
    out.write_text(json.dumps(report, indent=1, sort_keys=True) + "\n", encoding="utf-8")

    d = report["duplicates"]
    print(f"  rows audited: {report['rows_audited']}")
    print(f"  dimensions  : {report['dimensions']}")
    print(f"  quality failures        : {len(report['quality_failures'])}")
    print(f"  byte dup groups         : {d['byte_groups']} (cross-split {d['byte_groups_cross_split']})")
    print(f"  pixel dup groups        : {d['pixel_groups']} (cross-split {d['pixel_groups_cross_split']})")
    print(f"  aHash twin groups       : {d['ahash_twin_groups']} "
          f"(cross-split {d['ahash_twin_groups_cross_split']}, cross-class {d['ahash_twin_groups_cross_class']})")
    print(f"  verdict: {report['verdict']}")
    for f in report["failures"]:
        print(f"    FAIL: {f}")
    print(f"  written: {out}")
    return 1 if report["verdict"] == "FAIL" else 0


if __name__ == "__main__":
    sys.exit(main())
