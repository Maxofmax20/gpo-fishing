"""Validate human decisions and export a PROPOSED patch - never apply it.

``validate``
    Checks the decisions against each other and against the real knowledge base:
    duplicate ids, duplicate names, collisions with existing aliases, invalid
    vocabulary entries, and concepts still awaiting a decision.

``export``
    Writes ``proposed_ontology_patch.md`` and ``proposed_ontology.json``. These are
    DOCUMENTS FOR A PERSON TO APPROVE. Nothing in this module touches
    ``knowledge.rs``, ``FISH_VOCAB.txt``, the dataset, or the registry, and
    ``tests/`` asserts the absence of any such write path.

The patch is deliberately a separate artifact from the decisions so that approving
it is a distinct, visible act.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable

from .decisions import DecisionStore, proposals
from .schemas import DecisionType, HumanDecision, OntologyConcept

#: Entity id shape the rest of the system expects.
ID_RE = re.compile(r"^[a-z_]+:[a-z0-9][a-z0-9_-]*$")


@dataclass
class ValidationReport:
    errors: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    pending: list[str] = field(default_factory=list)
    counts: dict[str, int] = field(default_factory=dict)

    @property
    def ok(self) -> bool:
        return not self.errors

    def to_dict(self) -> dict[str, Any]:
        return {
            "ok": self.ok,
            "errors": self.errors,
            "warnings": self.warnings,
            "pending_concepts": len(self.pending),
            "counts": self.counts,
        }


def slugify(name: str) -> str:
    """``Crimson Snapper`` -> ``crimson-snapper``, for an id suffix.

    Used only to SUGGEST an id in the patch. It never decides one.
    """
    slug = re.sub(r"[^a-z0-9]+", "-", name.strip().lower()).strip("-")
    return slug or "unnamed"


def suggested_id(name: str, category: str = "fish") -> str:
    return f"{category}:{slugify(name)}"


def validate(
    concepts: list[OntologyConcept],
    decisions: DecisionStore,
    kb_payload: dict,
) -> ValidationReport:
    """Check decisions for internal consistency and KB collisions."""
    rep = ValidationReport()
    kb_ids: set[str] = {e["id"] for e in kb_payload["entities"]}
    kb_names: dict[str, str] = {}
    kb_aliases: dict[str, str] = {}
    for e in kb_payload["entities"]:
        kb_names[e["canonical_name"].strip().lower()] = e["id"]
        for a in (e.get("aliases") or []) + (e.get("ocr_aliases") or []):
            kb_aliases.setdefault(a.strip().lower(), e["id"])

    known_concepts = {c.concept_id for c in concepts}
    current = decisions.current()
    all_decisions = decisions.all()

    # --- duplicate ids -------------------------------------------------
    seen_ids: dict[str, int] = {}
    for d in all_decisions:
        seen_ids[d.concept_id] = seen_ids.get(d.concept_id, 0) + 1
    redecided = {k: v for k, v in seen_ids.items() if v > 1}
    if redecided:
        rep.warnings.append(
            f"{len(redecided)} concept(s) were decided more than once; the LAST "
            f"decision is authoritative and the earlier ones remain in the log: "
            f"{sorted(redecided)[:8]}"
        )

    # --- unknown concept ids -------------------------------------------
    for cid in sorted(set(current) - known_concepts):
        rep.errors.append(
            f"decision references unknown concept {cid!r}; it does not exist in "
            f"the current concept dataset"
        )

    # --- per-decision checks -------------------------------------------
    proposed_names: dict[str, str] = {}   # lower name -> concept_id
    proposed_ids: dict[str, str] = {}     # id -> concept_id
    for cid, d in sorted(current.items()):
        problems = d.validate()
        for p in problems:
            rep.errors.append(f"{cid}: {p}")
        if problems:
            continue

        if d.decision in (DecisionType.EXISTING_ENTITY.value,
                          DecisionType.VARIANT_OF_EXISTING.value):
            if d.existing_entity_id not in kb_ids:
                rep.errors.append(
                    f"{cid}: existing_entity_id {d.existing_entity_id!r} is not a "
                    f"KB entity"
                )

        if d.decision == DecisionType.NEW_ENTITY.value:
            name = (d.proposed_entity_name or "").strip()
            low = name.lower()
            if not name:
                rep.errors.append(f"{cid}: proposed_entity_name is empty")
                continue
            if low in proposed_names:
                rep.errors.append(
                    f"{cid}: proposes the same name as {proposed_names[low]!r} "
                    f"({name!r}); two concepts cannot both become one entity"
                )
            proposed_names[low] = cid

            if low in kb_names:
                rep.errors.append(
                    f"{cid}: proposes {name!r} but the KB ALREADY has that canonical "
                    f"name as {kb_names[low]!r}; use EXISTING_ENTITY instead"
                )
            if low in kb_aliases:
                rep.errors.append(
                    f"{cid}: proposes {name!r} which collides with an existing "
                    f"ALIAS of {kb_aliases[low]!r}"
                )

            sid = suggested_id(name)
            if sid in kb_ids:
                rep.errors.append(
                    f"{cid}: suggested id {sid!r} already exists in the KB"
                )
            if not ID_RE.match(sid):
                rep.errors.append(f"{cid}: suggested id {sid!r} is malformed")
            if sid in proposed_ids:
                rep.errors.append(
                    f"{cid}: suggested id {sid!r} collides with {proposed_ids[sid]!r}"
                )
            proposed_ids[sid] = cid

        if d.decision == DecisionType.UNKNOWN.value and not (d.notes or "").strip():
            rep.warnings.append(
                f"{cid}: decided UNKNOWN with no note; a short reason helps whoever "
                f"returns to this concept"
            )

    # --- pending -------------------------------------------------------
    rep.pending = sorted(known_concepts - set(current))
    rep.counts = {
        "concepts_total": len(concepts),
        "concepts_decided": len(set(current) & known_concepts),
        "concepts_pending": len(rep.pending),
        "proposed_new_entities": len(proposed_names),
        "kb_entities": len(kb_ids),
    }
    return rep


def export_patch(
    concepts: list[OntologyConcept],
    decisions: DecisionStore,
    kb_payload: dict,
    report: ValidationReport,
    out_dir: Path,
) -> dict[str, Any]:
    """Write a PROPOSED patch for human approval.

    Produces two documents and touches nothing else. The word "proposed" is in the
    filenames because these files must never be mistaken for an applied change.
    """
    current = decisions.current()
    by_id = {c.concept_id: c for c in concepts}
    props = proposals(current.values())

    rows = []
    for p in props:
        c = by_id.get(p["concept_id"])
        rows.append({
            "concept_id": p["concept_id"],
            "observed_game_name": c.display_name if c else None,
            "proposed_entity_name": p["proposed_entity_name"],
            "suggested_id": suggested_id(p["proposed_entity_name"] or ""),
            "occurrences": c.occurrences if c else 0,
            "sessions": len(c.sessions) if c else 0,
            "days": len(c.calendar_days) if c else 0,
            "evidence_image_ids": p["evidence_image_ids"][:6],
            "notes": p["notes"],
        })
    rows.sort(key=lambda r: (-r["occurrences"], r["concept_id"]))

    payload = {
        "generated_by": "ml.gpo_ontology_review export",
        "APPLIED": False,
        "approval_required": (
            "This file is a PROPOSAL. Applying it means editing knowledge.rs and "
            "FISH_VOCAB.txt by hand and re-running the class-map contract tests. "
            "Nothing in this toolchain does that automatically."
        ),
        "kb_version": kb_payload.get("kb_version"),
        "kb_entity_count": len(kb_payload["entities"]),
        "current_fish_vocab": kb_payload.get("fish_vocab"),
        "validation": report.to_dict(),
        "proposed_new_entities": rows,
        "decided_as_existing": sorted(
            d.existing_entity_id for d in current.values()
            if d.decision == DecisionType.EXISTING_ENTITY.value and d.existing_entity_id
        ),
        "decided_as_variant": sorted(
            f"{d.concept_id} -> {d.existing_entity_id} ({d.variant_descriptor})"
            for d in current.values()
            if d.decision == DecisionType.VARIANT_OF_EXISTING.value
        ),
        "decided_unknown": sorted(
            d.concept_id for d in current.values()
            if d.decision == DecisionType.UNKNOWN.value
        ),
    }

    json_path = out_dir / "proposed_ontology.json"
    json_path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")

    md = _render_patch_md(payload)
    md_path = out_dir / "proposed_ontology_patch.md"
    md_path.write_text(md, encoding="utf-8")

    return {
        "json": str(json_path),
        "markdown": str(md_path),
        "proposed_count": len(rows),
        "applied": False,
    }


def _render_patch_md(p: dict[str, Any]) -> str:
    lines = [
        "# PROPOSED ontology patch",
        "",
        "> **THIS IS A PROPOSAL, NOT AN APPLIED CHANGE.**",
        "> Nothing here has been written to `knowledge.rs` or `FISH_VOCAB.txt`.",
        "> A person must review this file and apply it deliberately.",
        "",
        "## Validation",
        "",
        f"- validation passed: **{p['validation']['ok']}**",
        f"- concepts: {p['validation']['counts']['concepts_total']}",
        f"- decided: {p['validation']['counts']['concepts_decided']}",
        f"- pending: {p['validation']['counts']['concepts_pending']}",
        f"- proposed new entities: {p['validation']['counts']['proposed_new_entities']}",
        "",
    ]
    if p["validation"]["errors"]:
        lines += ["### Errors", ""] + [f"- {e}" for e in p["validation"]["errors"]] + [""]
    if p["validation"]["warnings"]:
        lines += ["### Warnings", ""] + [f"- {w}" for w in p["validation"]["warnings"]] + [""]

    lines += [
        "## Proposed new entities",
        "",
        "Each row is a human decision that the game names something the knowledge",
        "base does not contain. **None of these exist yet.**",
        "",
        "| observed game name | proposed canonical name | suggested id | rows | sessions | days |",
        "|---|---|---|---:|---:|---:|",
    ]
    if p["proposed_new_entities"]:
        for r in p["proposed_new_entities"]:
            lines.append(
                f"| `{r['observed_game_name']}` | {r['proposed_entity_name']} | "
                f"`{r['suggested_id']}` | {r['occurrences']} | {r['sessions']} | "
                f"{r['days']} |"
            )
    else:
        lines.append("| _(no NEW_ENTITY decisions yet)_ | | | | | |")

    lines += [
        "",
        "## Decided as an EXISTING entity",
        "",
    ]
    if p["decided_as_existing"]:
        lines += [f"- `{e}`" for e in p["decided_as_existing"]]
    else:
        lines.append("_none yet_")

    lines += ["", "## Decided as a VARIANT of an existing entity", ""]
    if p["decided_as_variant"]:
        lines += [f"- {v}" for v in p["decided_as_variant"]]
    else:
        lines.append("_none yet_")

    lines += ["", "## Decided UNKNOWN (evidence insufficient)", ""]
    if p["decided_unknown"]:
        lines += [f"- `{c}`" for c in p["decided_unknown"]]
    else:
        lines.append("_none yet_")

    lines += [
        "",
        "## How to apply (manually, after review)",
        "",
        "1. Add each proposed entity to the knowledge base with its aliases.",
        "2. Append its id to `FISH_VOCAB.txt` **in the intended output-logit order** -",
        "   order is load-bearing, and the deployed `fish_v1` artifact pins a",
        "   `vocab_sha` over it, so changing it invalidates the shipped model.",
        "3. Re-run `cargo test` (`class_map_contract` and `kb_export_contract`) - the",
        "   contract tests will refuse an inconsistent class map.",
        "4. Re-export the KB: `cargo run --example kb_export`.",
        "5. Re-run the OCR audit so the new entities are recognised.",
        "",
        "Only after all of the above should a training snapshot be rebuilt.",
        "",
    ]
    return "\n".join(lines)