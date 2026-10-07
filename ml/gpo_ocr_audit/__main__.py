"""GPO OCR → dataset intelligence audit.

A reproducible pipeline that reads every dataset image, extracts its text,
resolves that text against the GPO knowledge base, joins the result to the
existing stored labels, and emits an auditable evidence map plus human-readable
reports.

Run it as a module from the repository root::

    python -m ml.gpo_ocr_audit

Requires no GPU. See ``README.md`` for the full contract.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from .ocr_audit import PIPELINE_VERSION, RunConfig, run
from .gpo_correlator import default_dataset_root
from .report_generator import (
    build_concepts,
    per_class_quality,
    summarise,
    write_agreement_matrix,
    write_class_quality,
    write_compound,
    write_conflicts,
    write_contact_sheets,
    write_ocr_quality,
    write_search_index,
    write_sessions,
    write_summary,
    write_unknown,
)

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUT = REPO_ROOT / "reports" / "ocr_audit"


def _parse_args(argv: list[str] | None) -> argparse.Namespace:
    ap = argparse.ArgumentParser(
        prog="python -m ml.gpo_ocr_audit",
        description="OCR -> GPO dataset intelligence audit (read-only).",
    )
    ap.add_argument("--dataset-root", type=Path, default=None,
                    help="dataset dir holding labels.jsonl + images/ "
                         "(default: %%APPDATA%%/gpo-autofish/datasets/gpo-vision/v1)")
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT,
                    help=f"output directory (default: {DEFAULT_OUT})")
    ap.add_argument("--workers", type=int, default=None,
                    help="OCR worker processes (default: cores-2, max 8)")
    ap.add_argument("--limit", type=int, default=0,
                    help="audit only the first N images (debugging; a limited run "
                         "is NOT a valid audit and says so in the report)")
    ap.add_argument("--no-model", action="store_true",
                    help="skip the shipped-model cross-check column")
    ap.add_argument("--no-contact-sheets", action="store_true",
                    help="skip contact-sheet rendering")
    ap.add_argument("--query", type=str, default=None,
                    help="after auditing, search the index for this text and print hits")
    ap.add_argument("--quiet", action="store_true")
    return ap.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    ds = args.dataset_root or default_dataset_root()
    if not (ds / "labels.jsonl").is_file():
        print(f"ERROR: no labels.jsonl under {ds}", file=sys.stderr)
        print("Pass --dataset-root explicitly if the data lives elsewhere.",
              file=sys.stderr)
        return 2

    cfg = RunConfig(
        dataset_root=ds,
        repo_root=REPO_ROOT,
        out_dir=args.out,
        workers=args.workers,
        limit=args.limit,
        use_model=not args.no_model,
        quiet=args.quiet,
    )
    result = run(cfg)

    if not args.quiet:
        print("[+] aggregating and writing reports", file=sys.stderr, flush=True)

    evs = result["evidences"]
    index = result["index"]
    ont = result["ontology"]
    prov = result["provenance"]
    out = cfg.out_dir

    s = summarise(evs)
    classes = per_class_quality(evs, ont.fish_vocab)
    concepts = build_concepts(evs)

    write_summary(out / "summary.md", s, classes, index.stats(), prov, concepts)
    write_class_quality(out / "class_quality.md", classes, s)
    write_compound(out / "compound_names.md", concepts, classes)
    write_unknown(out / "unknown_entities.md", concepts)
    write_conflicts(out / "conflicts.md", evs, concepts)
    write_sessions(out / "sessions.md", evs, index.stats())
    write_ocr_quality(out / "ocr_quality.md", s, evs)
    write_agreement_matrix(out / "agreement_matrix.md", evs)
    write_search_index(out / "search_index.sqlite", evs, concepts)

    sheets: list[str] = []
    if not args.no_contact_sheets:
        sheets = write_contact_sheets(out, evs, cfg.dataset_root, concepts, classes)

    _write_readme(out, prov, s, sheets, bool(args.limit))

    print(json.dumps({
        "pipeline": PIPELINE_VERSION,
        "images_audited": s["total"],
        "ocr_success": s["ocr_success"],
        "by_status": s["by_status"],
        "by_category": s["by_category"],
        "contact_sheets": len(sheets),
        "out_dir": str(out),
        "production_status": "MODEL_NOT_READY",
    }, indent=2))

    if args.query:
        from .report_generator import query_index
        hits = query_index(out / "search_index.sqlite", args.query)
        print(json.dumps(hits, indent=2)[:8000])

    return 0


def _write_readme(out: Path, prov: dict, s: dict, sheets: list[str], limited: bool) -> None:
    """Index of the artefact set, plus an explicit statement of what did NOT happen."""
    kb = prov["kb"]
    lines = [
        "# OCR Audit — artefact index",
        "",
        "Produced by `python -m ml.gpo_ocr_audit`. Everything here is **derived** from",
        "the dataset and is safe to delete and regenerate at any time.",
        "",
        "## Files",
        "",
        "| file | what it answers |",
        "|---|---|",
        "| `evidence.jsonl` | One line per image: the complete, replayable analysis. |",
        "| `provenance.json` | Dataset/KB/OCR fingerprints and configuration. |",
        "| `summary.md` | Headline counts, confidences, per-class clean share. |",
        "| `class_quality.md` | Per-fish-class breakdown of what supports each label. |",
        "| `compound_names.md` | Names containing 2+ canonical entities. |",
        "| `unknown_entities.md` | **Candidate ontology concepts** needing a decision. |",
        "| `conflicts.md` | Rows where the text disagrees with the stored label. |",
        "| `sessions.md` | Recurrence per session and per calendar day. |",
        "| `ocr_quality.md` | Confidence distribution and correction-rule firing. |",
        "| `agreement_matrix.md` | Stored / model / OCR three-way comparison. |",
        "| `search_index.sqlite` | Queryable: concept text → images, sessions, labels. |",
        "| `contact_sheets/` | Grids of the actual source images with captions. |",
        "",
        "## Querying the index",
        "",
        "```text",
        "python -m ml.gpo_ocr_audit --query \"crimson snapper\"",
        "```",
        "",
        "or directly:",
        "",
        "```sql",
        "sqlite3 search_index.sqlite",
        "SELECT concept, occurrences, sessions FROM concepts ORDER BY occurrences DESC;",
        "SELECT image_id, stored_entity_id, raw_ocr FROM evidence",
        "  WHERE residual_text LIKE '%crimson snapper%';",
        "```",
        "",
        "## Provenance of this run",
        "",
        f"- pipeline: `{prov['pipeline_version']}`",
        f"- normalisation rules: `{prov['normalization_rules_version']}`",
        f"- OCR engine: `{prov['ocr_engine']} {prov['ocr_engine_version']}`",
        f"- OCR fingerprint: `{prov['ocr_engine_fingerprint']}`",
        f"- KB: version {kb['kb_version']}, {kb['entities']} entities "
        f"({kb['fish_vocab_sha256'][:16]}…)",
        f"- `FISH_VOCAB.txt` sha256: `{prov['fish_vocab_sha256'][:16]}…`",
        f"- `labels.jsonl` sha256: `{prov['labels_jsonl_sha256'][:16]}…`",
        f"- `evidence.jsonl` sha256: `{prov['evidence_sha256'][:16]}…`",
        f"- CUDA used: {'yes' if prov['cuda_used'] else 'no'}",
        "",
        "## What this pipeline did NOT do",
        "",
        "- **No new ontology entity was created.** Names it cannot resolve are reported",
        "  as *candidate* concepts in `unknown_entities.md`; the KB is untouched.",
        "- **No training label was changed.** `labels.jsonl` is opened read-only and no",
        "  `training_eligible` flag was written anywhere.",
        "- **No TEST data was used for training or tuning.** TEST rows appear in the",
        "  reports for reporting only; the split assignment is imported from the",
        "  project's existing rule and never re-derived.",
        "- **No production safety gate was weakened.** No threshold was lowered, no",
        "  review requirement altered, no model promoted or retrained.",
        "- **No game input was controlled.** The pipeline imports no input, macro or",
        "  controller module.",
        "- **No synthetic data.** Every row traces to a real image by SHA-256.",
        "",
        "## Current production status",
        "",
        "```text",
        "MODEL_NOT_READY",
        "```",
        "",
        "This audit produces evidence about data quality. It cannot and does not change",
        "the model's readiness status.",
        "",
    ]
    if limited:
        lines = [
            "> **WARNING: this was a `--limit`ed run and is NOT a valid audit.**",
            "> Only a subset of images was processed. The counts below do not describe",
            "> the whole dataset.",
            "",
        ] + lines
    lines.insert(2, f"Images audited: **{s['total']}**. Contact sheets: "
                    f"**{len(sheets)}**.")
    (out / "README.md").write_text("\n".join(lines), encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())