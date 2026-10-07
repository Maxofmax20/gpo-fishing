"""Human-readable ontology reports, the relationship graph, and contact sheets.

Everything written here is DERIVED. It reads concepts and decisions and renders
them; it writes nothing to the knowledge base, the vocabulary, the dataset, or
the model.

The reports are designed to be usable with NO UI at all, because the decision
being asked for - "what does the game actually name?" - needs a human, and a
human should not have to install and run the app to answer it.
"""

from __future__ import annotations

import json
from collections import Counter
from pathlib import Path
from typing import Any, Iterable

from .evidence_select import selection_summary
from .schemas import DecisionType, OntologyConcept, OntologyStatus

BANNER = (
    "Names below are what the GAME appears to be writing. The current knowledge base's\n"
    "candidates are shown as evidence, not as an answer: the ontology under review is\n"
    "exactly the thing in question, so it must not bias the answer.\n"
)


def _t(rows: list[list[Any]], headers: list[str]) -> str:
    if not rows:
        return "_(none)_\n"
    out = ["| " + " | ".join(headers) + " |",
           "|" + "|".join("---" for _ in headers) + "|"]
    for r in rows:
        out.append("| " + " | ".join("" if c is None else str(c) for c in r) + " |")
    return "\n".join(out) + "\n"


def _decision_cell(c: OntologyConcept) -> str:
    return c.human_decision or "**PENDING**"


# --------------------------------------------------------------------------
# Main report
# --------------------------------------------------------------------------

def write_ontology_review(
    path: Path,
    concepts: list[OntologyConcept],
    families: dict[str, dict[str, Any]],
    kb_payload: dict,
    stats: dict[str, Any],
) -> None:
    by_priority = Counter(c.priority for c in concepts)
    decided = sum(1 for c in concepts if c.human_decision)
    pending = len(concepts) - decided

    lines = [
        "# In-game ontology review",
        "",
        "Every concept the game appears to name, with the evidence behind it.",
        "",
        BANNER,
        "## How to read this",
        "",
        "- **Occurrences** - rows whose freshly-read text matched this name.",
        "- **Sessions / Days** - how many independent macro runs and calendar days",
        "  produced it. Recurrence is the strongest evidence that a name is real",
        "  game output rather than one OCR slip.",
        "- **Current KB candidates** - what today's knowledge base *could* claim.",
        "  Shown for reference only.",
        "- **Stored labels** - what the dataset currently calls these rows.",
        "  **Evidence, not truth**: those labels were produced under the ontology",
        "  now in question.",
        "- **Decision** - PENDING until a human rules. Nothing is pre-selected.",
        "",
        "## Totals",
        "",
        _t([
            ["Distinct observed game names", stats.get("concepts")],
            ["…from raw OCR strings", stats.get("raw_distinct_strings")],
            ["Rows considered on entity frames", stats.get("evidence_rows_considered")],
            ["Rows skipped as banner/system text", stats.get("rows_skipped_as_ui_noise")],
            ["Rows skipped (not an entity frame)", stats.get("rows_skipped_non_entity_frame")],
            ["Concept families", len(families)],
            ["Decisions recorded", decided],
            ["Decisions pending", pending],
        ], ["property", "value"]),
        "",
        "### Review queue by priority band",
        "",
        _t([[b, by_priority.get(b, 0),
             {"P0": ">=100 rows, >=10 sessions, >=3 days",
              "P1": ">=25 rows, >=5 sessions",
              "P2": ">=5 rows",
              "P3": "singleton"}[b]]
            for b in ("P0", "P1", "P2", "P3")],
           ["band", "concepts", "criterion"]),
        "",
    ]

    for band in ("P0", "P1", "P2", "P3"):
        rows = [c for c in concepts if c.priority == band]
        if not rows:
            continue
        lines += [f"## {band} — {len(rows)} concept(s)", ""]
        lines += [_concept_block(c) for c in rows]

    path.write_text("\n".join(lines), encoding="utf-8")


def _concept_block(c: OntologyConcept) -> str:
    lines = [
        f"### {c.display_name.title()}",
        "",
        _t([
            ["Occurrences", c.occurrences],
            ["Sessions", len(c.sessions)],
            ["Calendar days", len(c.calendar_days)],
            ["Priority", f"{c.priority} ({'; '.join(c.priority_reasons)})"],
            ["Concept id", f"`{c.concept_id}`"],
        ], ["property", "value"]),
        "",
    ]

    if c.base_name:
        lines += [
            "**Decomposition (linguistic evidence only, not an ontology claim):**",
            "",
            _t([[
                c.base_name,
                ", ".join(c.modifiers) if c.modifiers else "(none - single word)",
                c.family_id,
            ]], ["base name", "modifiers", "family"]),
            "",
        ]

    if c.current_kb_candidates:
        status_note = {
            OntologyStatus.AMBIGUOUS.value:
                "contains more than one canonical entity, so today's KB has no single answer",
            OntologyStatus.EXACT_ENTITY.value:
                "is exactly one canonical entity",
            OntologyStatus.PARTIAL.value: "is only partly explainable",
        }.get(c.ontology_status, "")
        lines += [
            "**Current KB candidates:**",
            "",
            *[f"- `{e}`" for e in c.current_kb_candidates],
            "",
            f"_{c.ontology_status}{': ' + status_note if status_note else ''}_",
            "",
        ]
    else:
        lines += [
            "**Current KB candidates:** none — the knowledge base has no entity for this name.",
            "",
        ]

    if c.stored_labels:
        rows = [[f"`{k}`", v] for k, v in c.stored_labels.items()]
        lines += ["**Stored labels (evidence, not truth):**", "", _t(rows, ["label", "rows"]), ""]
        if c.stored_agrees_with_candidate:
            lines += [
                f"_{c.stored_agrees_with_candidate} of {c.occurrences} rows have a stored "
                f"label equal to one KB candidate. That means the label agrees with one "
                f"COMPONENT of the name — it does NOT mean the name is confirmed._",
                "",
            ]

    if c.ocr_variants:
        lines += [
            "**OCR variants seen:**",
            "",
            *[f"- `{v}`" + (f" ×{c.variant_counts[v]}" if v in c.variant_counts else "")
              for v in c.ocr_variants[:12]],
            "",
        ]

    if c.representative_images:
        lines += [
            "**Representative evidence:**",
            "",
            _t([[
                e["rank"], f"`{e['image_id']}`", e["day"] or "-",
                e["session_id"].replace("session_", "") if e["session_id"] else "-",
                f"`{(e['raw_ocr'] or '')[:40]}`",
                f"`{e['stored_entity_id']}`" if e["stored_entity_id"] else "-",
                f"{e['ocr_confidence']:.3f}" if e["ocr_confidence"] is not None else "-",
                e["selected_because"],
            ] for e in c.representative_images],
               ["#", "image id", "day", "session", "OCR text", "stored label",
                "conf", "chosen because"]),
            "",
            f"_{selection_summary(c.representative_images)}_",
            "",
        ]

    lines += [f"**Decision:** {_decision_cell(c)}", "", "---", ""]
    return "\n".join(lines)


# --------------------------------------------------------------------------
# Modifier and base-name reports
# --------------------------------------------------------------------------

def write_modifier_report(path: Path, modifiers: list[dict[str, Any]]) -> None:
    lines = [
        "# Recurring modifiers",
        "",
        "Words that recur in front of a base name across the observed game names.",
        "",
        "> **EVIDENCE ONLY.** A high count for `golden` says the word recurs. It does",
        "> NOT say `golden` is a colour rather than a species. That is the question",
        "> under review: the current KB treats `golden` and `crimson` as species, and",
        "> they may instead be modifiers on species.",
        "",
        _t([[
            m["modifier"], m["occurrences"], m["concept_count"], m["session_count"],
            m["day_count"], ", ".join(m["bases"][:4]),
            ", ".join(f"{x['display_name']} ({x['occurrences']})"
                      for x in m["concepts"][:4]),
        ] for m in modifiers],
           ["modifier", "rows", "concepts", "sessions", "days", "bases it precedes",
            "examples"]),
        "",
    ]
    path.write_text("\n".join(lines), encoding="utf-8")


def write_base_report(path: Path, bases: list[dict[str, Any]]) -> None:
    lines = [
        "# Base-name analysis",
        "",
        "Terminal words of the observed game names, and what the current knowledge",
        "base maps onto them.",
        "",
        "> This table exists because the existing KB appears to have been built from",
        "> **fragments of compound names** rather than from names the game renders.",
        "> A base that only ever appears with a modifier, and has its own KB entity,",
        "> is a strong hint that the entity describes a fragment rather than a thing",
        "> the game calls on its own.",
        "",
        _t([[
            b["base_name"], b["occurrences"], b["concept_count"], b["session_count"],
            b["day_count"],
            "yes" if b["occurs_bare"] else "**never bare**",
            ", ".join(f"`{x}`" for x in b["kb_candidates"]) or "none",
            ", ".join(f"{c['display_name']} ({c['occurrences']})"
                      for c in b["concepts"][:4]),
        ] for b in bases],
           ["base", "rows", "concepts", "sessions", "days", "appears bare?",
            "current KB entities", "members"]),
        "",
        "## Modifiers seen per base",
        "",
        _t([[b["base_name"], ", ".join(f"{k}×{v}" for k, v in b["modifier_totals"].items())]
            for b in bases],
           ["base", "modifiers observed"]),
        "",
    ]
    path.write_text("\n".join(lines), encoding="utf-8")


def write_family_report(path: Path, families: dict[str, dict[str, Any]]) -> None:
    multi = [f for f in families.values() if not f["is_singleton"]]
    single = [f for f in families.values() if f["is_singleton"]]
    lines = [
        "# Concept families",
        "",
        "Concepts grouped by shared base word.",
        "",
        "> **A family is NOT an assertion that its members are the same entity.**",
        "> Whether a family is one entity, several entities, a base fish plus cosmetic",
        "> modifiers, or unrelated names that happen to share a word is exactly the",
        "> question the review asks. The family view exists to make that question",
        "> visible, not to answer it.",
        "",
        f"## Families with more than one member ({len(multi)})",
        "",
    ]
    for f in sorted(multi, key=lambda x: -x["occurrences"]):
        lines += [
            f"### {f['base_name']}",
            "",
            _t([[
                f"`{m['display_name']}`",
                ", ".join(m["modifiers"]) if m["modifiers"] else "(bare)",
                m["occurrences"], f"`{m['concept_id']}`",
            ] for m in f["members"]],
               ["observed name", "modifiers", "rows", "concept id"]),
            "",
            f"_{f['occurrences']} rows · {f['session_count']} sessions · "
            f"{f['day_count']} days. Question: {f['decision_required']}_",
            "",
        ]
    lines += [
        f"## Singleton families ({len(single)})",
        "",
        "A name with no other member sharing its base word. No base/modifier",
        "structure has been observed for these, and none is implied.",
        "",
        _t([[f["base_name"], f"`{f['members'][0]['display_name']}`",
             f["members"][0]["occurrences"]]
            for f in sorted(single, key=lambda x: -x["occurrences"])],
           ["base", "observed name", "rows"]),
        "",
    ]
    path.write_text("\n".join(lines), encoding="utf-8")


# --------------------------------------------------------------------------
# Relationship graph
# --------------------------------------------------------------------------

def build_graph(
    concepts: list[OntologyConcept],
    families: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    """Machine-readable relationship graph. Investigation only."""
    nodes, edges = [], []
    for c in concepts:
        nodes.append({
            "id": c.concept_id,
            "kind": "concept",
            "label": c.display_name,
            "priority": c.priority,
            "ontology_status": c.ontology_status,
            "base_name": c.base_name,
            "modifiers": c.modifiers,
            "occurrences": c.occurrences,
            "session_count": len(c.sessions),
            "day_count": len(c.calendar_days),
            "human_decision": c.human_decision,
        })
        if c.family_id:
            edges.append({"from": c.concept_id, "to": c.family_id, "rel": "member_of"})
        for e in c.current_kb_candidates:
            edges.append({"from": c.concept_id, "to": e, "rel": "kb_candidate"})
        for lbl, n in c.stored_labels.items():
            edges.append({"from": c.concept_id, "to": lbl, "rel": "stored_label",
                          "rows": n})
        for v in c.ocr_variants:
            edges.append({"from": c.concept_id, "to": v, "rel": "ocr_variant"})

    for fid, f in families.items():
        nodes.append({
            "id": fid,
            "kind": "family",
            "label": f["base_name"],
            "member_count": f["member_count"],
            "is_singleton": f["is_singleton"],
            "occurrences": f["occurrences"],
        })

    return {
        "kind": "ontology_relationship_graph",
        "purpose": "investigation only; not an authority and not applied anywhere",
        "nodes": sorted(nodes, key=lambda n: n["id"]),
        "edges": sorted(edges, key=lambda e: (e["from"], e["to"], e["rel"])),
    }


def write_ascii_graph(path: Path, concepts: list[OntologyConcept]) -> None:
    """A readable tree per concept, matching the shape a reviewer reasons about."""
    lines = [
        "# Ontology relationship graph (ASCII)",
        "",
        "Each concept shown as the tree a reviewer reasons about: which KB entities",
        "it could be, which stored labels it produced, and how much evidence backs it.",
        "",
        "Investigation only - this graph asserts nothing.",
        "",
    ]
    for c in concepts:
        lines.append(f"{c.display_name}")
        lines.append(f" |")
        for e in c.current_kb_candidates:
            lines.append(f" +-- kb candidate: {e}")
        for v in c.ocr_variants[:4]:
            n = c.variant_counts.get(v, 0)
            lines.append(f" +-- ocr variant:   \"{v}\" x{n}")
        for lbl, n in c.stored_labels.items():
            lines.append(f" +-- stored label:  {lbl} x{n}")
        lines.append(f" +-- images:        {c.occurrences}")
        lines.append(f" +-- sessions:      {len(c.sessions)}")
        lines.append(f" +-- days:          {len(c.calendar_days)}")
        if c.human_decision:
            lines.append(f" +-- DECISION:      {c.human_decision}")
        else:
            lines.append(f" +-- DECISION:      PENDING")
        lines.append("")
    path.write_text("\n".join(lines), encoding="utf-8")


# --------------------------------------------------------------------------
# Contact sheets
# --------------------------------------------------------------------------

def write_contact_sheets(
    out_dir: Path,
    concepts: list[OntologyConcept],
    dataset_root: Path,
    limit_concepts: int = 40,
    per_concept: int = 6,
) -> list[str]:
    """Render the representative images so a human can eyeball each concept.

    Only the concepts a human has NOT yet decided get sheets, so re-running after
    some decisions are made does not regenerate work already done.
    """
    try:
        from PIL import Image, ImageDraw
    except ImportError:
        return []

    sheets_dir = out_dir / "contact_sheets"
    sheets_dir.mkdir(parents=True, exist_ok=True)
    images_dir = dataset_root / "images"
    written: list[str] = []

    pending = [c for c in concepts if not c.human_decision][:limit_concepts]
    cols = 3
    cell_w, cell_h = 320, 120
    cap_h = 62

    from .concept_extract import group_key

    for c in pending:
        ev = c.representative_images[:per_concept]
        if not ev:
            continue
        rows_n = (len(ev) + cols - 1) // cols
        sheet = Image.new("RGB", (cols * cell_w, rows_n * (cell_h + cap_h)), (16, 16, 20))
        draw = ImageDraw.Draw(sheet)
        for i, e in enumerate(ev):
            cx = (i % cols) * cell_w
            cy = (i // cols) * (cell_h + cap_h)
            path = images_dir / f"{e['image_id']}.png"
            try:
                with Image.open(path) as im:
                    im = im.convert("RGB")
                    im.thumbnail((cell_w - 4, cell_h - 4))
                    sheet.paste(im, (cx + 2, cy + 2))
            except Exception:
                draw.text((cx + 6, cy + 6), "<unreadable>", fill=(210, 90, 90))
            y = cy + cell_h + 2
            for line, colour in (
                (e["image_id"], (150, 200, 255)),
                (f"ocr: {(e['raw_ocr'] or '')[:38]}", (180, 180, 180)),
                (f"stored: {e['stored_entity_id'] or '(none)'}", (220, 220, 220)),
                (f"{e['day']} {e['selected_because'][:22]}", (255, 195, 120)),
            ):
                draw.text((cx + 4, y), line[:46], fill=colour)
                y += 13
        dest = sheets_dir / f"{group_key(c.display_name)[:44]}.png"
        sheet.save(dest)
        written.append(str(dest.relative_to(out_dir.parent)))
    return written