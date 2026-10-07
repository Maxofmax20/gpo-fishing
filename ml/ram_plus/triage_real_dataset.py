"""Evidence-fusion triage over the REAL dataset.

Runs the same agreement logic the Rust reviewer uses, over every real fish and
fruit row, and reports what it would surface to a human. This is a MEASUREMENT
of the triage's value, not a claim about labels: it never writes a label.

Purpose: answer "would a semantic teacher actually change what a human reviews
first?" using the 8,609 real rows already collected.
"""

from __future__ import annotations

import collections
import json
import os
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent

# The canonical KB entities, mirrored from the Rust side for reporting only.
# Nothing here can create or alter an entity.
FISH = [
    "fish:angelfish", "fish:crimson", "fish:golden", "fish:pufferfish",
    "fish:shark", "fish:snapper", "fish:squid", "fish:swordfish",
]
FRUIT = []  # populated from the live KB below if available


def norm(s: str) -> str:
    keep = []
    for c in s:
        if c in "<>*.,\"'|\\":
            continue
        keep.append(c.lower() if c.isalnum() else " ")
    return " ".join("".join(keep).split())


def short_name(entity: str) -> str:
    return entity.split(":", 1)[1] if ":" in entity else entity


# Common OCR corruptions of canonical fish names, observed in the real dataset.
# `csvordfish` is `Swordfish`, `g0lden` is `golden`, `anselfish` is `angelfish`.
# These are OCR ARTEFACTS, not unknown visual concepts, and must not be reported
# as new entities - that is exactly the failure this split prevents.
OCR_VARIANT_FIXES = {
    "csvordfish": "swordfish", "cvordfish": "swordfish", "sw0rdfish": "swordfish",
    "g0lden": "golden", "go1den": "golden", "g01den": "golden",
    "anselfish": "angelfish", "anqdfish": "angelfish", "anselfish": "angelfish",
    "shask": "shark", "snaper": "snapper", "suid": "squid",
    "puffer": "pufferfish", "purfer": "pufferfish",
}


def repair_ocr_variants(n: str) -> str:
    out = n
    for bad, good in OCR_VARIANT_FIXES.items():
        out = out.replace(bad, good)
    return out


def resolve_ocr(text: str, kb: set[str]) -> str | None:
    n = norm(text)
    hits = [e for e in kb if short_name(e) and short_name(e) in norm(e) and norm(e) in n]
    # exactly one hit resolves; two is a human decision
    return hits[0] if len(hits) == 1 else None


def main() -> int:
    appdata = os.environ.get("APPDATA")
    root = Path(appdata) / "gpo-autofish" / "datasets" / "gpo-vision" / "v1"
    labels = root / "labels.jsonl"
    if not labels.is_file():
        print(f"dataset not found: {labels}")
        return 2

    rows = []
    with labels.open(encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                rows.append(json.loads(line))
            except Exception:
                pass

    # The ontology the resolver is allowed to choose from. Deliberately the
    # CANONICAL fish list only: the point is to find rows whose OCR names
    # something outside it.
    kb = set(FISH)

    stats = collections.Counter()
    unresolved_examples = collections.Counter()
    per_class = collections.defaultdict(collections.Counter)
    clean_per_class = collections.Counter()
    total_per_class = collections.Counter()

    for r in rows:
        ent = r.get("entity_id") or ""
        if not (ent.startswith("fish:") or ent.startswith("fruit:")):
            continue
        stats["entity_rows"] += 1
        total_per_class[ent] += 1

        ocr = (r.get("ocr_text") or "").strip()
        n = norm(ocr)
        n_rep = repair_ocr_variants(n)
        if n_rep != n:
            stats["ocr_variant_repaired"] += 1
        hits = sorted({short_name(e) for e in kb if short_name(e) and norm(short_name(e)) in n_rep})

        if not ocr:
            stats["no_ocr_text"] += 1
            continue

        if len(hits) == 1:
            stats["ocr_single_canonical"] += 1
            if ent == "fish:" + hits[0]:
                stats["ocr_agrees_with_label"] += 1
                clean_per_class[ent] += 1
            else:
                stats["ocr_CONFLICTS_with_label"] += 1
                per_class[ent]["ocr_conflict"] += 1
        elif len(hits) >= 2:
            stats["ocr_AMBIGUOUS_two_or_more_kb_names"] += 1
            key = " + ".join(hits)
            unresolved_examples[key] += 1
            per_class[ent]["ambiguous"] += 1
        else:
            stats["ocr_name_outside_kb"] += 1
            # keep the recognisable head of the string for reporting
            head = n.replace("nevitem", "").replace("new item", "").strip()
            head = head[:40] if head else n[:40]
            unresolved_examples[head or "(empty)"] += 1
            per_class[ent]["outside_kb"] += 1

    print("=" * 74)
    print("EVIDENCE FUSION TRIAGE — real dataset, no RAM++ required")
    print("=" * 74)
    print(f"rows with an entity label : {stats['entity_rows']}")
    print(f"  of those, no OCR text   : {stats['no_ocr_text']}")
    print(f"  OCR variant repaired    : {stats['ocr_variant_repaired']}  (OCR noise, NOT unknown entities)")
    print()
    print("OCR vs stored label, against the CANONICAL fish ontology only:")
    print(f"  exactly one KB name, agrees with label : {stats['ocr_agrees_with_label']:>5}")
    print(f"  exactly one KB name, CONFLICTS         : {stats['ocr_CONFLICTS_with_label']:>5}")
    print(f"  AMBIGUOUS (>=2 KB names in one string) : {stats['ocr_AMBIGUOUS_two_or_more_kb_names']:>5}")
    print(f"  names NO KB entity at all              : {stats['ocr_name_outside_kb']:>5}")
    contested = (
        stats["ocr_CONFLICTS_with_label"]
        + stats["ocr_AMBIGUOUS_two_or_more_kb_names"]
        + stats["ocr_name_outside_kb"]
    )
    decided = stats["ocr_agrees_with_label"] + contested
    print()
    print(f"  => {contested} of {decided} entity rows cannot be confirmed by OCR alone")
    print(f"     ({contested / max(decided,1):.1%} need a human, or better evidence)")

    print()
    print("UNRESOLVED VISUAL CONCEPTS (never auto-created, always human-reviewed):")
    for name, n in unresolved_examples.most_common(14):
        if name.startswith("ambiguous:"):
            continue
        print(f"  {n:>5} rows   {name!r}")

    print()
    print("AMBIGUOUS between existing canonical entities (the Crimson Snapper case):")
    for name, n in unresolved_examples.most_common(20):
        if name.startswith("ambiguous:"):
            print(f"  {n:>5} rows   {name[10:]}")

    print()
    print(f"{'stored label':<20}{'total':>7}{'clean':>7}{'ambiguous':>11}{'outside':>9}{'conflict':>10}")
    for c in sorted(total_per_class, key=lambda k: -total_per_class[k]):
        t = total_per_class[c]
        pc = per_class[c]
        print(
            f"{c:<20}{t:>7}{clean_per_class[c]:>7}{pc['ambiguous']:>11}"
            f"{pc['outside_kb']:>9}{pc['ocr_conflict']:>10}"
        )

    # A machine-readable artefact for the Rust/UI side.
    out = {
        "source": "real dataset triage (no teacher required)",
        "dataset_rows_with_entity": stats["entity_rows"],
        "rows_no_ocr_text": stats["no_ocr_text"],
        "ocr_agrees_with_label": stats["ocr_agrees_with_label"],
        "ocr_conflicts_with_label": stats["ocr_CONFLICTS_with_label"],
        "ocr_ambiguous_multi_kb": stats["ocr_AMBIGUOUS_two_or_more_kb_names"],
        "ocr_outside_kb": stats["ocr_name_outside_kb"],
        "unresolved_concepts": [
            {"concept": k, "rows": v} for k, v in unresolved_examples.most_common()
        ],
        "per_class": {
            c: {"total": total_per_class[c], "clean": clean_per_class[c], **dict(per_class[c])}
            for c in sorted(total_per_class)
        },
    }
    dest = REPO / "ml" / "ram_plus" / "triage_report.json"
    dest.write_text(json.dumps(out, indent=2), encoding="utf-8")
    print(f"\nwrote {dest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())