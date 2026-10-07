"""In-game ontology discovery and human review.

Turns the OCR audit's 411 raw OCR strings into a short, ordered, evidence-backed
list of DECISIONS for a human to make about what the game is actually naming.

READ-ONLY, by construction. This package contains no code path that writes to the
knowledge base, ``FISH_VOCAB.txt``, the dataset, the model registry, a split, or
a readiness gate. A human decision is recorded as a PROPOSAL; applying it is a
separate, explicitly-approved step.

Commands::

    python -m ml.gpo_ontology_review build      # concept dataset + all reports
    python -m ml.gpo_ontology_review decide     # record ONE human decision
    python -m ml.gpo_ontology_review validate   # check decisions for consistency
    python -m ml.gpo_ontology_review export     # write a PROPOSED patch (applies nothing)
    python -m ml.gpo_ontology_review status     # what is decided, what is pending
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from .decisions import DecisionRejected, DecisionStore
from .patch import export_patch, validate
from .pipeline import run

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_EVIDENCE = REPO_ROOT / "reports" / "ocr_audit" / "evidence.jsonl"
DEFAULT_KB = REPO_ROOT / "ml" / "gpo_ocr_audit" / "kb_export.json"
DEFAULT_OUT = REPO_ROOT / "reports" / "ontology_review"


def _dataset_root() -> Path | None:
    import os
    appdata = os.environ.get("APPDATA", "")
    if not appdata:
        return None
    p = Path(appdata) / "gpo-autofish" / "datasets" / "gpo-vision" / "v1"
    return p if (p / "labels.jsonl").is_file() else None


def cmd_wiki_refresh(args) -> int:
    """Fetch the wiki bestiary and rewrite the committed snapshot.

    The ONLY command in this tool that touches the network, and it is never invoked
    by `build`: a page load or a rebuild must not depend on the wiki being reachable.
    """
    from . import wiki as wiki_mod

    path = args.out / "wiki_fish.json"
    try:
        snap = wiki_mod.refresh_snapshot(path)
    except Exception as e:  # noqa: BLE001 - report, never half-write
        print(f"  wiki refresh FAILED: {e}", file=sys.stderr)
        print(f"  {path} left untouched; the previous snapshot is still valid.",
              file=sys.stderr)
        return 1
    print(f"  fetched {snap['entryCount']} fish from {snap['sourceHost']}")
    print(f"  sha256 {snap['contentSha256']}")
    print(f"  written to {path}")
    return 0


def cmd_build(args) -> int:
    if not args.evidence.is_file():
        print(f"ERROR: no audit evidence at {args.evidence}", file=sys.stderr)
        print("Run the OCR audit first:  python -m ml.gpo_ocr_audit", file=sys.stderr)
        return 2
    result = run(args.evidence, args.kb, args.out,
                 dataset_root=_dataset_root(),
                 quiet=args.quiet,
                 contact_sheets=not args.no_contact_sheets)
    s = result["summary"]
    print(json.dumps({
        "concepts": s["totals"]["concepts"],
        "from_raw_strings": s["totals"]["raw_distinct_ocr_strings"],
        "families": s["totals"]["concept_families"],
        "multi_member_families": s["totals"]["multi_member_families"],
        "compound_concepts": s["totals"]["compound_concepts"],
        "unknown_concepts": s["totals"]["unknown_concepts"],
        "decisions_recorded": s["decisions"]["recorded"],
        "decisions_pending": s["decisions"]["pending"],
        "validation_ok": result["validation"].ok,
        "contact_sheets": s["contact_sheets"],
        "out_dir": str(args.out),
        "training": "NOT STARTED",
        "production": "MODEL_NOT_READY",
    }, indent=2))
    return 0 if result["validation"].ok else 1


def cmd_decide(args) -> int:
    from .schemas import HumanDecision

    store = DecisionStore(args.out / "ontology_decisions.jsonl")
    images = [i.strip() for i in (args.evidence_images or "").split(",") if i.strip()]
    d = HumanDecision(
        concept_id=args.concept_id,
        decision=args.decision,
        existing_entity_id=args.existing_entity,
        proposed_entity_name=args.proposed_name,
        variant_descriptor=args.variant_descriptor,
        decision_maker="human",
        timestamp=args.timestamp or "",
        evidence_image_ids=images,
        notes=args.notes or "",
    )
    try:
        store.record(d)
    except DecisionRejected as exc:
        print(f"REJECTED\n{exc}", file=sys.stderr)
        return 2
    print(json.dumps(d.to_dict(), indent=2, sort_keys=True))
    print("\nRecorded. This is a PROPOSAL - it has not touched the knowledge base.")
    print("Run `python -m ml.gpo_ontology_review export` to regenerate the patch.")
    return 0


def cmd_validate(args) -> int:
    concepts_path = args.out / "concepts.json"
    if not concepts_path.is_file():
        print(f"ERROR: no concepts at {concepts_path}; run `build` first", file=sys.stderr)
        return 2
    payload = json.loads(concepts_path.read_text(encoding="utf-8"))

    class _Shim:
        def __init__(self, d):
            for k, v in d.items():
                setattr(self, k, v)

    concepts = [_Shim(d) for d in payload["concepts"]]
    store = DecisionStore(args.out / "ontology_decisions.jsonl")
    kb = json.loads(args.kb.read_text(encoding="utf-8"))
    rep = validate(concepts, store, kb)
    print(json.dumps(rep.to_dict(), indent=2, sort_keys=True))
    if rep.errors:
        print(f"\n{len(rep.errors)} error(s).", file=sys.stderr)
    return 0 if rep.ok else 1


def cmd_export(args) -> int:
    concepts_path = args.out / "concepts.json"
    if not concepts_path.is_file():
        print(f"ERROR: no concepts at {concepts_path}; run `build` first", file=sys.stderr)
        return 2
    payload = json.loads(concepts_path.read_text(encoding="utf-8"))

    class _Shim:
        def __init__(self, d):
            for k, v in d.items():
                setattr(self, k, v)

    concepts = [_Shim(d) for d in payload["concepts"]]
    store = DecisionStore(args.out / "ontology_decisions.jsonl")
    kb = json.loads(args.kb.read_text(encoding="utf-8"))
    rep = validate(concepts, store, kb)
    out = export_patch(concepts, store, kb, rep, args.out)
    print(json.dumps(out, indent=2))
    print("\nAPPLIED: false - the patch is a document for a human to review.")
    return 0 if rep.ok else 1


def cmd_status(args) -> int:
    summary_path = args.out / "summary.json"
    if not summary_path.is_file():
        print(f"ERROR: no summary at {summary_path}; run `build` first", file=sys.stderr)
        return 2
    s = json.loads(summary_path.read_text(encoding="utf-8"))
    print(json.dumps({
        "concepts": s["totals"]["concepts"],
        "decisions": s["decisions"],
        "review_queue_sizes": {k: len(v) for k, v in s["review_queue"].items()},
        "validation": s["validation"]["ok"],
        "training": "NOT STARTED",
        "production": "MODEL_NOT_READY",
    }, indent=2))
    return 0


def main(argv: list[str] | None = None) -> int:
    # Common options are declared on the top-level parser AND on each subparser,
    # so `--out` works on either side of the subcommand. argparse would otherwise
    # reject `build --out X`, which is the ordering people naturally type.
    def common(p: argparse.ArgumentParser) -> None:
        p.add_argument("--evidence", type=Path, default=argparse.SUPPRESS)
        p.add_argument("--kb", type=Path, default=argparse.SUPPRESS)
        p.add_argument("--out", type=Path, default=argparse.SUPPRESS)
        p.add_argument("--quiet", action="store_true", default=argparse.SUPPRESS)

    ap = argparse.ArgumentParser(
        prog="python -m ml.gpo_ontology_review",
        description="In-game ontology discovery and human review (read-only).",
    )
    ap.add_argument("--evidence", type=Path, default=DEFAULT_EVIDENCE)
    ap.add_argument("--kb", type=Path, default=DEFAULT_KB)
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    ap.add_argument("--quiet", action="store_true")
    sub = ap.add_subparsers(dest="cmd", required=True)

    b = sub.add_parser("build", help="build the concept dataset and all reports")
    common(b)
    b.add_argument("--no-contact-sheets", action="store_true")
    b.set_defaults(fn=cmd_build)

    w = sub.add_parser(
        "wiki-refresh",
        help="fetch the GPO wiki fish bestiary and update the committed snapshot",
    )
    common(w)
    w.set_defaults(fn=cmd_wiki_refresh)

    d = sub.add_parser("decide", help="record ONE human decision (a proposal)")
    common(d)
    d.add_argument("--concept-id", required=True)
    d.add_argument("--decision", required=True,
                   choices=["EXISTING_ENTITY", "NEW_ENTITY",
                            "VARIANT_OF_EXISTING", "UNKNOWN"])
    d.add_argument("--existing-entity", default=None)
    d.add_argument("--proposed-name", default=None)
    d.add_argument("--variant-descriptor", default=None)
    d.add_argument("--evidence-images", default=None,
                   help="comma-separated image ids the human actually looked at")
    d.add_argument("--notes", default=None)
    d.add_argument("--timestamp", default=None)
    d.set_defaults(fn=cmd_decide)

    v = sub.add_parser("validate", help="check decisions for consistency")
    common(v)
    v.set_defaults(fn=cmd_validate)

    e = sub.add_parser("export", help="write a PROPOSED patch (applies nothing)")
    common(e)
    e.set_defaults(fn=cmd_export)

    st = sub.add_parser("status", help="what is decided and what is pending")
    common(st)
    st.set_defaults(fn=cmd_status)

    args = ap.parse_args(argv)
    # SUPPRESS means "not supplied on the subparser", so the top-level default
    # survives unless the subparser actually provided a value.
    for name, default in (("evidence", DEFAULT_EVIDENCE), ("kb", DEFAULT_KB),
                          ("out", DEFAULT_OUT)):
        if not hasattr(args, name):
            setattr(args, name, default)
    return args.fn(args)


if __name__ == "__main__":
    raise SystemExit(main())