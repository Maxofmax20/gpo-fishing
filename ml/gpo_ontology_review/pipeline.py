"""Build the in-game ontology review dataset from the OCR audit's evidence.

READ-ONLY with respect to everything that matters.

This pipeline reads ``reports/ocr_audit/evidence.jsonl`` and writes files under
``reports/ontology_review/``. It does not touch the knowledge base, the
vocabulary, the dataset, the model registry, any split, or any readiness gate,
and there is no code path in this package that could.

Its whole purpose is to turn a few hundred mysterious OCR strings into a short,
ordered, evidence-backed list of decisions for a human - and to make sure that
nothing is decided without one.
"""

from __future__ import annotations

import json
import time
from pathlib import Path
from typing import Any

from .concept_extract import (
    build_base_report,
    build_concepts,
    build_families,
    build_modifier_report,
)
from .decisions import DecisionStore, decision_counts, training_ready_counts
from .evidence_select import Candidate, select_representative
from .patch import export_patch, validate
from .workstation import (
    build_index,
    build_kb_entities,
    build_structural,
    index_digest,
    render_thumbs,
)
from .reporting import (
    build_graph,
    write_ascii_graph,
    write_base_report,
    write_contact_sheets,
    write_family_report,
    write_family_report as _wfr,
    write_modifier_report as _wmr,
    write_ontology_review,
)

PIPELINE_VERSION = "ontology-review/1.0.0"

#: Frames that can carry an entity name. An idle fishing-bar frame reading `x1` is
#: UI; turning it into an ontology question would bury the real ones.
ENTITY_FRAMES = frozenset({"FISH_DROP", "FRUIT_DROP", "BAIT_DROP", "BANNER", "UNKNOWN"})


def _session_day(session_id: str | None) -> str:
    if not session_id:
        return ""
    parts = session_id.split("_")
    return parts[1] if len(parts) > 1 else ""


def load_ui_phrases(kb_path: Path) -> list[str]:
    """Banner templates from the KB, used to keep system text out of the concepts."""
    payload = json.loads(kb_path.read_text(encoding="utf-8"))
    return [e["canonical_name"] for e in payload["entities"] if e["category"] == "ui_term"]


def attach_representative_evidence(concepts, evidence_path: Path) -> int:
    """Pick representative images per concept and attach them.

    Done in a second pass because selection needs all rows of a concept, and
    ``build_concepts`` streams the file once to avoid holding 15k records.
    """
    wanted = {c.concept_id: c for c in concepts}
    # Map concept name back to its bucket by group key.
    by_key: dict[str, list] = {c.group_key: [] for c in concepts}
    concepts_by_key = {c.group_key: c for c in concepts}

    with evidence_path.open(encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            if row.get("image_category") not in ENTITY_FRAMES:
                continue
            # Recompute the concept name the same way build_concepts did.
            from .concept_extract import extract_concept_name, group_key

            name, _notes = extract_concept_name(row.get("residual_text") or "", UI_PHRASES)
            if not name:
                continue
            key = group_key(name)
            c = concepts_by_key.get(key)
            if c is None:
                continue
            by_key[key].append(Candidate(
                image_id=row["image_id"],
                session_id=row.get("session_id") or "",
                day=_session_day(row.get("session_id")),
                raw_text=row.get("raw_ocr") or "",
                concept_name=name,
                stored_entity_id=row.get("stored_entity_id"),
                ocr_confidence=row.get("ocr_confidence"),
                timestamp_ms=row.get("timestamp_ms") or 0,
            ))

    n = 0
    for key, cands in by_key.items():
        c = concepts_by_key[key]
        c.representative_images = select_representative(cands)
        n += 1
    return n


#: Populated by ``run`` so ``attach_representative_evidence`` can reuse them.
UI_PHRASES: list[str] = []


def run(evidence_path: Path, kb_path: Path, out_dir: Path,
        dataset_root: Path | None, quiet: bool = False,
        contact_sheets: bool = True) -> dict[str, Any]:
    t0 = time.time()
    out_dir.mkdir(parents=True, exist_ok=True)

    global UI_PHRASES
    UI_PHRASES = load_ui_phrases(kb_path)
    kb_payload = json.loads(kb_path.read_text(encoding="utf-8"))

    if not quiet:
        print("[1/5] grouping observed names into concepts", flush=True)
    concepts, stats = build_concepts(evidence_path, UI_PHRASES, ENTITY_FRAMES)
    # system_text.json records what was excluded, so nothing is quietly dropped
    stats_note = {
        "note": "banner / system text excluded from concepts; see summary.md",
    }

    if not quiet:
        print(f"      {stats['concepts']} concepts from "
              f"{stats['raw_distinct_strings']} raw OCR strings", flush=True)
        print("[2/5] selecting representative evidence", flush=True)
    with_evidence = attach_representative_evidence(concepts, evidence_path)

    if not quiet:
        print("[3/5] building families, modifiers and base names", flush=True)
    families = build_families(concepts)
    modifiers = build_modifier_report(concepts)
    bases = build_base_report(concepts)

    store = DecisionStore(out_dir / "ontology_decisions.jsonl")
    decided = store.apply_to(concepts)
    if not quiet:
        print(f"      {decided} concept(s) already have a human decision", flush=True)

    if not quiet:
        print("[4/5] validating and writing reports", flush=True)
    report = validate(concepts, store, kb_payload)
    write_ontology_review(out_dir / "ontology_review.md", concepts, families,
                          kb_payload, stats)
    _wmr(out_dir / "modifiers.md", modifiers)
    _wfr(out_dir / "families.md", families)
    write_base_report(out_dir / "base_names.md", bases)
    write_ascii_graph(out_dir / "ontology_graph.txt", concepts)

    graph = build_graph(concepts, families)
    (out_dir / "ontology_graph.json").write_text(
        json.dumps(graph, indent=2, sort_keys=True), encoding="utf-8"
    )

    concepts_payload = {
        "pipeline_version": PIPELINE_VERSION,
        "note": (
            "One record per exact observed game name. current_kb_candidates are "
            "candidates, not answers. human_decision is null until a human rules."
        ),
        "stats": {**stats, **stats_note},
        "concepts": [c.to_dict() for c in concepts],
    }
    (out_dir / "concepts.json").write_text(
        json.dumps(concepts_payload, indent=2, sort_keys=True), encoding="utf-8"
    )

    # --- workstation artefacts (compact index + thumbnails) -------------
    ws_dir = out_dir / "workstation"
    ws_dir.mkdir(parents=True, exist_ok=True)
    index = build_index(concepts, kb_payload, modifiers, families, bases)
    kb_entities = build_kb_entities(kb_payload)
    structural = build_structural(modifiers, families, bases)
    (ws_dir / "index.json").write_text(
        json.dumps(index, indent=1, sort_keys=True), encoding="utf-8")
    (ws_dir / "kb_entities.json").write_text(
        json.dumps(kb_entities, indent=1, sort_keys=True), encoding="utf-8")
    (ws_dir / "structural.json").write_text(
        json.dumps(structural, indent=1, sort_keys=True), encoding="utf-8")

    thumb_stats = {"rendered": 0, "missing": 0, "error": "dataset root unavailable"}
    if dataset_root is not None and (dataset_root / "images").is_dir():
        thumb_stats = render_thumbs(concepts, dataset_root / "images", ws_dir / "thumbs")
    if not quiet:
        print(f"      workstation index: {index['counts']}  "
              f"thumbs: {thumb_stats.get('rendered')} rendered, "
              f"{thumb_stats.get('missing')} missing", flush=True)

    ws_summary = {
        "index_sha256": index_digest(index),
        "index_path": "reports/ontology_review/workstation/index.json",
        "index_bytes": (ws_dir / "index.json").stat().st_size,
        "kb_entities": kb_entities["entityCount"],
        "structural_modifiers": len(structural["modifiers"]),
        "structural_families": len(structural["families"]),
        "thumbnails": thumb_stats,
    }

    training = training_ready_counts(concepts, store.current())
    (out_dir / "training_consequences.json").write_text(
        json.dumps(training, indent=2, sort_keys=True), encoding="utf-8"
    )

    sheets: list[str] = []
    if contact_sheets and dataset_root is not None:
        sheets = write_contact_sheets(out_dir, concepts, dataset_root)

    summary = _summary(concepts, families, modifiers, bases, stats, store,
                       report, sheets, t0, ws_summary)
    (out_dir / "summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True), encoding="utf-8"
    )

    if not quiet:
        print("[5/5] done", flush=True)
    return {"concepts": concepts, "families": families, "modifiers": modifiers,
            "bases": bases, "stats": stats, "validation": report,
            "summary": summary, "sheets": sheets, "training": training,
            "decisions": store.current()}


def _summary(concepts, families, modifiers, bases, stats, store, report,
             sheets, t0, ws_summary) -> dict[str, Any]:
    decided = [c for c in concepts if c.human_decision]
    pending = [c for c in concepts if not c.human_decision]
    by_priority = {}
    for c in concepts:
        by_priority.setdefault(c.priority, []).append(c.concept_id)

    multi = [f for f in families.values() if not f["is_singleton"]]
    compound = [c for c in concepts if c.modifiers]
    unknown = [c for c in concepts
               if c.ontology_status == "UNKNOWN_ENTITY_VALUE"
               or c.ontology_status == "UNKNOWN"]
    ambiguous = [c for c in concepts if c.ontology_status == "AMBIGUOUS"]
    exact = [c for c in concepts if c.ontology_status == "EXACT_ENTITY"]

    counts = decision_counts(store.all().__iter__().__self__ if False else store.all())

    return {
        "pipeline_version": PIPELINE_VERSION,
        "generated_at_unix": int(t0),
        "elapsed_seconds": round(time.time() - t0, 1),
        "totals": {
            "raw_distinct_ocr_strings": stats["raw_distinct_strings"],
            "concepts": len(concepts),
            "concept_families": len(families),
            "multi_member_families": len(multi),
            "compound_concepts": len(compound),
            "ambiguous_concepts": len(ambiguous),
            "exact_entity_concepts": len(exact),
            "unknown_concepts": len(unknown),
            "rows_considered": stats["evidence_rows_considered"],
            "rows_skipped_system_text": stats["rows_skipped_as_ui_noise"],
            "rows_skipped_non_entity_frame": stats["rows_skipped_non_entity_frame"],
        },
        "review_queue": {
            band: sorted(by_priority.get(band, []),
                         key=lambda cid: next(c.occurrences for c in concepts
                                             if c.concept_id == cid),
                         reverse=True)
            for band in ("P0", "P1", "P2", "P3")
        },
        "decisions": {
            "recorded": len(decided),
            "pending": len(pending),
            "by_type": counts,
            "path": "reports/ontology_review/ontology_decisions.jsonl",
        },
        "validation": report.to_dict(),
        "contact_sheets": len(sheets),
        "workstation": ws_summary,
        "top_concepts": [
            {
                "concept_id": c.concept_id,
                "display_name": c.display_name,
                "occurrences": c.occurrences,
                "sessions": len(c.sessions),
                "days": len(c.calendar_days),
                "priority": c.priority,
                "ontology_status": c.ontology_status,
                "kb_candidates": c.current_kb_candidates,
                "decision": c.human_decision or "PENDING",
            }
            for c in sorted(concepts, key=lambda x: (-x.occurrences, x.display_name))[:20]
        ],
        "production_status": "MODEL_NOT_READY",
        "training_started": False,
    }