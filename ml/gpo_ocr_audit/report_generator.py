"""Human-readable reports, contact sheets, and a searchable index.

Everything here is DERIVED. It reads the evidence records and renders them; it
never writes to the dataset, the KB, the registry, or any label.

The reports exist so one person can open a single document and answer, for any
name the game has ever written:

* what did it actually write, and how often?
* which exact images prove it?
* what does the dataset currently call those images?
* what does the KB know about that name - entity, alias, OCR error, compound, or
  nothing at all?
* across how many sessions and days?

Contact sheets are rendered into ``reports/ocr_audit/contact_sheets/`` from the
original PNGs. The originals are opened read-only and never written.
"""

from __future__ import annotations

import json
import re
import sqlite3
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Iterable

from .gpo_correlator import session_day
from .schemas import AuditStatus, EntityEvidence, ImageCategory

SLUG_RE = re.compile(r"[^a-z0-9]+")


def slug(text: str, limit: int = 48) -> str:
    s = SLUG_RE.sub("-", text.lower()).strip("-")
    return (s or "empty")[:limit]


def _pct(n: int, d: int) -> str:
    return f"{100.0 * n / d:.1f}%" if d else "n/a"


# --------------------------------------------------------------------------
# Aggregation
# --------------------------------------------------------------------------

def summarise(evs: list[EntityEvidence]) -> dict[str, Any]:
    """Every count the reports need, computed once.

    Denominators are always the full image count, and ambiguous groups are
    reported on their own lines rather than folded into a neighbour, so the
    numbers add up.
    """
    n = len(evs)
    by_status = Counter(e.audit_status for e in evs)
    by_category = Counter(e.image_category for e in evs)
    by_kind = Counter(e.match_kind for e in evs)
    by_onto = Counter(e.ontology_status for e in evs)
    by_split = Counter(e.split for e in evs)

    ocr_ok = sum(1 for e in evs if e.raw_ocr.strip())
    ocr_err = sum(1 for e in evs if e.ocr_error)

    confs = [e.ocr_confidence for e in evs if e.ocr_confidence is not None]
    confs_sorted = sorted(confs)

    def q(p: float) -> float | None:
        if not confs_sorted:
            return None
        i = min(len(confs_sorted) - 1, int(p * len(confs_sorted)))
        return round(confs_sorted[i], 4)

    rules = Counter()
    for e in evs:
        for r in e.correction_rules_applied:
            rules[r] += 1

    # Fresh OCR vs the collector's stored OCR: a direct measure of how much the
    # collector's own text can be trusted.
    stored_nonempty = sum(1 for e in evs if (e.stored_ocr_text or "").strip())
    both = [e for e in evs
            if (e.stored_ocr_text or "").strip() and e.normalized_ocr.strip()]
    agree = sum(1 for e in both if e.normalized_ocr == " ".join(
        e.stored_ocr_text.lower().split()))

    return {
        "total": n,
        "ocr_success": ocr_ok,
        "ocr_failure": n - ocr_ok,
        "ocr_engine_errors": ocr_err,
        "rejected_detections": sum(e.rejected_detections for e in evs),
        "by_status": dict(sorted(by_status.items())),
        "by_category": dict(sorted(by_category.items())),
        "by_match_kind": dict(sorted(by_kind.items())),
        "by_ontology_status": dict(sorted(by_onto.items())),
        "by_split": dict(sorted(by_split.items())),
        "confidence": {
            "n": len(confs),
            "min": round(confs_sorted[0], 4) if confs_sorted else None,
            "p10": q(0.10), "median": q(0.50), "p90": q(0.90),
            "mean": round(sum(confs) / len(confs), 4) if confs else None,
            "max": round(confs_sorted[-1], 4) if confs_sorted else None,
        },
        "correction_rules": dict(rules.most_common()),
        "stored_vs_fresh": {
            "stored_nonempty": stored_nonempty,
            "comparable": len(both),
            "identical_after_normalisation": agree,
            "agreement_rate": _pct(agree, len(both)) if both else "n/a",
        },
        "human_reviews": sum(1 for e in evs if e.human_reviewed),
        "requires_human_review": sum(1 for e in evs if e.requires_human_review),
    }


def per_class_quality(evs: list[EntityEvidence], fish_vocab: list[str]) -> dict[str, dict]:
    """Per-fish-class breakdown of the audit.

    ``effective_clean`` is the number that actually matters: rows whose text
    confirms the stored label. It is what a reviewer could keep, and it is
    strictly less than the row count wherever contamination exists.
    """
    out: dict[str, dict] = {}
    for cls in fish_vocab:
        rows = [e for e in evs if e.stored_entity_id == cls]
        c = {
            "total": len(rows),
            "confirmed": 0, "ocr_variant": 0, "alias": 0,
            "conflict": 0, "compound": 0, "unknown": 0, "unreadable": 0,
            "no_decision": 0, "non_fish": 0,
            "human_reviewed": 0, "in_snapshot": 0,
        }
        by_split = Counter()
        sessions: set[str] = set()
        for e in rows:
            st = e.audit_status
            if st == AuditStatus.CONFIRMED_MATCH.value:
                c["confirmed"] += 1
            elif st == AuditStatus.OCR_VARIANT_MATCH.value:
                c["ocr_variant"] += 1
            elif st == AuditStatus.ALIAS_MATCH.value:
                c["alias"] += 1
            elif st == AuditStatus.OCR_CONFLICT.value:
                c["conflict"] += 1
            elif st == AuditStatus.COMPOUND_AMBIGUOUS.value:
                c["compound"] += 1
            elif st == AuditStatus.UNKNOWN_ENTITY.value:
                c["unknown"] += 1
            elif st == AuditStatus.OCR_UNREADABLE.value:
                c["unreadable"] += 1
            elif st == AuditStatus.NO_DECISION.value:
                c["no_decision"] += 1
            elif st == AuditStatus.NON_FISH.value:
                c["non_fish"] += 1
            if e.human_reviewed:
                c["human_reviewed"] += 1
            if e.in_latest_snapshot:
                c["in_snapshot"] += 1
            by_split[e.split] += 1
            sessions.add(e.session_id)
        c["effective_clean"] = c["confirmed"] + c["ocr_variant"] + c["alias"]
        c["needs_human"] = c["total"] - c["effective_clean"]
        c["clean_share"] = _pct(c["effective_clean"], c["total"]) if c["total"] else "n/a"
        c["sessions"] = len(sessions)
        c["days"] = len({session_day(s) for s in sessions if session_day(s)})
        c["by_split"] = dict(sorted(by_split.items()))
        out[cls] = c
    return out


def build_concepts(evs: list[EntityEvidence]) -> list[dict[str, Any]]:
    """Group evidence rows by the concept their text names.

    One group per distinct name the game wrote. Each carries its recurrence
    across sessions and days, because a name seen in six independent runs on four
    days is materially different evidence from a one-frame OCR slip.
    """
    groups: dict[str, dict[str, Any]] = {}
    for e in evs:
        key = (e.residual_text or e.normalized_ocr or "").strip().lower()
        if not key:
            continue
        g = groups.setdefault(key, {
            "concept": key,
            "occurrences": 0,
            "sessions": set(),
            "days": set(),
            "stored_labels": Counter(),
            "candidates": Counter(),
            "fragments": Counter(),
            "statuses": Counter(),
            "raw_variants": Counter(),
            "example_images": [],
            "image_ids": [],
            "confidence": [],
            "corrections": Counter(),
            "first_seen": e.timestamp_ms,
            "last_seen": e.timestamp_ms,
            "priority": 0,
        })
        g["occurrences"] += 1
        g["sessions"].add(e.session_id)
        d = session_day(e.session_id)
        if d:
            g["days"].add(d)
        if e.stored_entity_id:
            g["stored_labels"][e.stored_entity_id] += 1
        for c in e.candidate_entities:
            g["candidates"][c] += 1
        for t in e.compound_tokens:
            g["fragments"][f"{t.token} -> {t.entity_id}"] += 1
        g["statuses"][e.audit_status] += 1
        g["raw_variants"][e.raw_ocr] += 1
        for r in e.correction_rules_applied:
            g["corrections"][r] += 1
        if e.ocr_confidence is not None:
            g["confidence"].append(e.ocr_confidence)
        g["image_ids"].append(e.image_id)
        if len(g["example_images"]) < 12:
            g["example_images"].append(e.image_id)
        g["first_seen"] = min(g["first_seen"], e.timestamp_ms)
        g["last_seen"] = max(g["last_seen"], e.timestamp_ms)
        g["priority"] = max(g["priority"], e.review_priority)

    out = []
    for g in groups.values():
        conf = g.pop("confidence")
        out.append({
            **g,
            "session_count": len(g["sessions"]),
            "day_count": len(g["days"]),
            "days": sorted(g["days"]),
            "sessions": sorted(g["sessions"]),
            "stored_labels": dict(g["stored_labels"].most_common()),
            "candidates": dict(g["candidates"].most_common()),
            "fragments": dict(g["fragments"].most_common()),
            "statuses": dict(g["statuses"].most_common()),
            "corrections": dict(g["corrections"].most_common()),
            "raw_variants": dict(g["raw_variants"].most_common(8)),
            "mean_confidence": round(sum(conf) / len(conf), 4) if conf else None,
            "image_count": len(g["image_ids"]),
        })
    # Most frequent first, then alphabetical, so the ordering is deterministic.
    out.sort(key=lambda g: (-g["occurrences"], g["concept"]))
    return out


# --------------------------------------------------------------------------
# Markdown reports
# --------------------------------------------------------------------------

def _table(headers: list[str], rows: list[list[Any]]) -> str:
    if not rows:
        return "_(none)_\n"
    out = ["| " + " | ".join(headers) + " |",
           "|" + "|".join("---" for _ in headers) + "|"]
    for r in rows:
        out.append("| " + " | ".join("" if c is None else str(c) for c in r) + " |")
    return "\n".join(out) + "\n"


def write_summary(path: Path, s: dict, classes: dict, index_stats: dict,
                  prov: dict, concepts: list[dict]) -> None:
    n = s["total"]
    st = s["by_status"]
    cat = s["by_category"]
    lines = [
        "# OCR Audit — Summary",
        "",
        "Generated by the OCR audit pipeline. Every number below is measured from the",
        "real dataset; nothing is estimated and no synthetic record was created.",
        "",
        "## Dataset",
        "",
        _table(["property", "value"], [
            ["dataset", f"`{index_stats['root_name']}` v{index_stats['dataset_version']}"],
            ["rows / images audited", n],
            ["sessions", index_stats["sessions"]],
            ["calendar days", f"{index_stats['calendar_days']} "
                              f"({index_stats['day_range'][0]} … {index_stats['day_range'][-1]})"],
            ["train / validation / test", f"{index_stats['splits'].get('train', 0)} / "
                                          f"{index_stats['splits'].get('validation', 0)} / "
                                          f"{index_stats['splits'].get('test', 0)}"],
            ["sessions spanning >1 split",
             len(index_stats["sessions_spanning_multiple_splits"])],
            ["existing human reviews", index_stats["human_reviews"]],
            ["  …of those training-eligible", index_stats["eligible_reviews"]],
            ["KB version", prov["kb"]["kb_version"]],
            ["KB entities", prov["kb"]["entities"]],
            ["canonical fish vocabulary", len(prov["kb"]["fish_vocab"])],
            ["OCR engine", f"{prov['ocr_engine']} {prov['ocr_engine_version']}"],
            ["CUDA used", "no (CPU only)"],
        ]),
        "",
        "## OCR outcome",
        "",
        _table(["metric", "count", "share"], [
            ["images processed", n, "100.0%"],
            ["OCR produced text", s["ocr_success"], _pct(s["ocr_success"], n)],
            ["OCR produced no text", s["ocr_failure"], _pct(s["ocr_failure"], n)],
            ["engine errors", s["ocr_engine_errors"], _pct(s["ocr_engine_errors"], n)],
            ["low-confidence boxes rejected", s["rejected_detections"], "—"],
        ]),
        "",
        "OCR confidence distribution (min of per-image box confidences):",
        "",
        _table(["min", "p10", "median", "p90", "max", "mean"],
               [[s["confidence"]["min"], s["confidence"]["p10"],
                 s["confidence"]["median"], s["confidence"]["p90"],
                 s["confidence"]["max"], s["confidence"]["mean"]]]),
        "",
        "## Audit status — one verdict per image",
        "",
        _table(["status", "count", "share"],
               [[k, v, _pct(v, n)] for k, v in sorted(st.items(), key=lambda kv: -kv[1])]),
        "",
        "## Frame categories",
        "",
        _table(["category", "count", "share"],
               [[k, v, _pct(v, n)] for k, v in sorted(cat.items(), key=lambda kv: -kv[1])]),
        "",
        "## How the text reached the ontology",
        "",
        "A separate axis from the verdict: this is *how* a match was obtained, so a",
        "lossy confusable fold is never presented as an exact match.",
        "",
        _table(["match kind", "count"],
               [[k, v] for k, v in sorted(s["by_match_kind"].items(), key=lambda kv: -kv[1])]),
        "",
        _table(["ontology status", "count"],
               [[k, v] for k, v in sorted(s["by_ontology_status"].items(),
                                         key=lambda kv: -kv[1])]),
        "",
        "## Fresh OCR vs the collector's stored OCR",
        "",
        "The dataset's own `ocr_text` was written by the app's recogniser. Comparing",
        "it with this audit's independent re-read measures how far it can be trusted.",
        "",
        _table(["metric", "value"], [
            ["rows with stored OCR text", s["stored_vs_fresh"]["stored_nonempty"]],
            ["rows comparable (both sides non-empty)", s["stored_vs_fresh"]["comparable"]],
            ["identical after normalisation", s["stored_vs_fresh"]["identical_after_normalisation"]],
            ["agreement", s["stored_vs_fresh"]["agreement_rate"]],
        ]),
        "",
        "## OCR correction rules applied",
        "",
        _table(["rule", "times applied"],
               [[k, v] for k, v in s["correction_rules"].items()]) if s["correction_rules"]
        else "_No correction rule fired._\n",
        "",
        "## Per-class clean share (canonical vocabulary, in vocabulary order)",
        "",
        _table(["class", "rows", "clean", "clean %", "conflict", "compound",
                "unknown", "unreadable", "human reviewed"],
               [[cls, c["total"], c["effective_clean"], c["clean_share"], c["conflict"],
                 c["compound"], c["unknown"], c["unreadable"], c["human_reviewed"]]
                for cls, c in classes.items()]),
        "",
        "## Top concepts by frequency",
        "",
        _table(["concept", "rows", "sessions", "days", "stored labels", "candidates",
                "top status"],
               [[f"`{g['concept']}`", g["occurrences"], g["session_count"], g["day_count"],
                 ", ".join(f"{k}×{v}" for k, v in list(g["stored_labels"].items())[:3]),
                 ", ".join(list(g["candidates"])[:3]),
                 max(g["statuses"].items(), key=lambda kv: kv[1])[0]]
                for g in concepts[:25]]),
        "",
    ]
    path.write_text("\n".join(lines), encoding="utf-8")


def write_compound(path: Path, concepts: list[dict], classes: dict) -> None:
    rows = [g for g in concepts
            if g["statuses"].get(AuditStatus.COMPOUND_AMBIGUOUS.value, 0) > 0]
    rows.sort(key=lambda g: (-g["statuses"].get(AuditStatus.COMPOUND_AMBIGUOUS.value, 0),
                             g["concept"]))
    lines = [
        "# OCR Audit — Compound names",
        "",
        "A compound name contains two or more whole-word canonical entities, so **no",
        "single stored label can be correct for it** without a human decision.",
        "",
        "This is the case that produced the original contamination: the collector's",
        "resolver picked one component arbitrarily.",
        "",
        f"**{len(rows)} distinct concept(s) affected.**",
        "",
    ]
    for g in rows:
        lines += [
            f"## `{g['concept']}`",
            "",
            _table(["property", "value"], [
                ["rows", g["occurrences"]],
                ["compound rows", g["statuses"].get(AuditStatus.COMPOUND_AMBIGUOUS.value, 0)],
                ["sessions / days", f"{g['session_count']} / {g['day_count']}"],
                ["canonical candidates",
                 ", ".join(f"`{k}` ×{v}" for k, v in g["candidates"].items())],
                ["entity fragments found",
                 "; ".join(f"`{k}`" for k in list(g["fragments"])[:6]) or "—"],
                ["stored labels it was filed under",
                 ", ".join(f"`{k}` ×{v}" for k, v in g["stored_labels"].items())],
                ["mean OCR confidence", g["mean_confidence"]],
                ["example image ids", ", ".join(f"`{i}`" for i in g["example_images"][:6])],
                ["OCR variants seen",
                 "; ".join(f"`{k}` ×{v}" for k, v in list(g["raw_variants"].items())[:5])],
            ]),
            "",
        ]
    path.write_text("\n".join(lines), encoding="utf-8")


def write_unknown(path: Path, concepts: list[dict]) -> None:
    rows = [g for g in concepts
            if g["statuses"].get(AuditStatus.UNKNOWN_ENTITY.value, 0) > 0]
    rows.sort(key=lambda g: (-g["occurrences"], g["concept"]))
    lines = [
        "# OCR Audit — Unresolved visual concepts",
        "",
        "Names the game wrote that the knowledge base cannot account for.",
        "",
        "> These are **not** entities. They are CANDIDATE ONTOLOGY CONCEPTS awaiting a",
        "> human decision. Nothing in this pipeline can create one, and none was created.",
        "",
        f"**{len(rows)} distinct concept(s).**",
        "",
    ]
    for g in rows:
        lines += [
            f"## `{g['concept']}`",
            "",
            _table(["property", "value"], [
                ["occurrences", g["occurrences"]],
                ["sessions / days", f"{g['session_count']} / {g['day_count']}"],
                ["KB status", "NOT PRESENT as a canonical entity"],
                ["KB fragment hints (not resolutions)",
                 ", ".join(f"`{k}`" for k in list(g["fragments"])[:4]) or "none"],
                ["fuzzy suggestions only",
                 ", ".join(f"`{k}`" for k in list(g["candidates"])[:4]) or "none"],
                ["stored labels it was filed under",
                 ", ".join(f"`{k}` ×{v}" for k, v in g["stored_labels"].items())],
                ["correction rules applied",
                 ", ".join(f"`{k}` ×{v}" for k, v in g["corrections"].items()) or "none"],
                ["mean OCR confidence", g["mean_confidence"]],
                ["example image ids", ", ".join(f"`{i}`" for i in g["example_images"][:6])],
                ["human decision", "REQUIRED"],
            ]),
            "",
        ]
    path.write_text("\n".join(lines), encoding="utf-8")


def write_conflicts(path: Path, evs: list[EntityEvidence], concepts: list[dict]) -> None:
    """Conflicts sorted by severity: the evidence against a stored label, most
    damaging first."""
    conflicts = [e for e in evs
                 if e.audit_status in (AuditStatus.OCR_CONFLICT.value,
                                       AuditStatus.COMPOUND_AMBIGUOUS.value,
                                       AuditStatus.UNKNOWN_ENTITY.value)]
    # Group identical OCR concepts together: the pair (stored label -> what the
    # text says) with the candidates it offered, so one row per distinct concept
    # rather than one per image.
    pairs: dict[tuple[str, str, str], dict[str, Any]] = {}
    for e in conflicts:
        key = (e.stored_entity_id or "(none)",
               e.residual_text or e.normalized_ocr,
               e.audit_status)
        g = pairs.setdefault(key, {"n": 0, "cands": Counter(), "sessions": set()})
        g["n"] += 1
        for c in e.candidate_entities:
            g["cands"][c] += 1
        g["sessions"].add(e.session_id)
    ordered = sorted(pairs.items(), key=lambda kv: -kv[1]["n"])
    lines = [
        "# OCR Audit — Conflicts and unresolved rows",
        "",
        "Rows where the freshly-read text does not agree with the stored label, or",
        "does not resolve to any single entity. Grouped by concept and ordered by how",
        "many rows share each interpretation.",
        "",
        f"**{len(conflicts)} of {len(evs)} rows ({_pct(len(conflicts), len(evs))}) are in "
        f"conflict or unresolved.**",
        "",
        "No stored label was modified. Every row below still carries its original",
        "`entity_id` in `evidence.jsonl`.",
        "",
        _table(["rows", "stored label", "OCR interpretation", "audit status",
                "candidates", "sessions"],
               [[g["n"], f"`{k[0]}`", f"`{k[1]}`", k[2],
                 ", ".join(f"`{c}`" for c, _ in g["cands"].most_common(4)) or "—",
                 len(g["sessions"])]
                for k, g in ordered[:60]]),
        "",
        "## Highest-priority individual rows",
        "",
        "Ranked by disagreement, recurrence across independent sessions, and OCR",
        "confidence. **Ordering only — this score never changes a label.**",
        "",
        _table(["priority", "image_id", "stored", "OCR text", "status", "candidates",
                "session", "reasons"],
               [[e.review_priority, f"`{e.image_id}`", f"`{e.stored_entity_id}`",
                 f"`{(e.residual_text or e.normalized_ocr)[:48]}`", e.audit_status,
                 ", ".join(f"`{c}`" for c in e.candidate_entities) or "—",
                 e.session_id, "; ".join(e.priority_reasons)[:110]]
                for e in sorted(evs, key=lambda x: (-x.review_priority, x.image_id))[:60]]),
        "",
    ]
    path.write_text("\n".join(lines), encoding="utf-8")


def write_class_quality(path: Path, classes: dict, s: dict) -> None:
    lines = [
        "# OCR Audit — Per-class quality",
        "",
        "For every class in `FISH_VOCAB.txt`, in vocabulary order.",
        "",
        "`effective clean` = rows whose freshly-read text actually supports the stored",
        "label (confirmed, alias, or OCR-variant). It is the only figure that counts as",
        "evidence for that class.",
        "",
    ]
    for cls, c in classes.items():
        lines += [
            f"## `{cls}`",
            "",
            _table(["metric", "value"], [
                ["total stored rows", c["total"]],
                ["OCR-confirmed", c["confirmed"]],
                ["OCR-variant matches", c["ocr_variant"]],
                ["alias matches", c["alias"]],
                ["**effective clean**", f"**{c['effective_clean']}** ({c['clean_share']})"],
                ["OCR conflicts", c["conflict"]],
                ["compound (unresolvable)", c["compound"]],
                ["unknown concept", c["unknown"]],
                ["unreadable", c["unreadable"]],
                ["no decision", c["no_decision"]],
                ["non-fish text", c["non_fish"]],
                ["rows needing a human", c["needs_human"]],
                ["already human-reviewed", c["human_reviewed"]],
                ["already in latest training snapshot", c["in_snapshot"]],
                ["sessions / days", f"{c['sessions']} / {c['days']}"],
                ["split distribution", ", ".join(f"{k}={v}" for k, v in c["by_split"].items())],
            ]),
            "",
        ]
    path.write_text("\n".join(lines), encoding="utf-8")


def write_sessions(path: Path, evs: list[EntityEvidence], index_stats: dict) -> None:
    by_session: dict[str, list[EntityEvidence]] = defaultdict(list)
    for e in evs:
        by_session[e.session_id].append(e)
    lines = [
        "# OCR Audit — Session and day analysis",
        "",
        "Recurrence across independent runs is the strongest available signal that a",
        "name is a real recurring concept rather than a one-frame OCR artefact. It is",
        "still not proof of ontology membership: only a human can confirm that.",
        "",
        _table(["property", "value"], [
            ["sessions", len(by_session)],
            ["calendar days", index_stats["calendar_days"]],
            ["sessions spanning >1 split",
             len(index_stats["sessions_spanning_multiple_splits"])],
        ]),
        "",
    ]
    rows = []
    for sid, items in sorted(by_session.items()):
        st = Counter(i.audit_status for i in items)
        days = {session_day(i.session_id) for i in items}
        rows.append([
            f"`{sid}`", sorted(days)[0] if days else "", len(items),
            st.get(AuditStatus.CONFIRMED_MATCH.value, 0),
            st.get(AuditStatus.OCR_CONFLICT.value, 0),
            st.get(AuditStatus.COMPOUND_AMBIGUOUS.value, 0),
            st.get(AuditStatus.UNKNOWN_ENTITY.value, 0),
            st.get(AuditStatus.OCR_UNREADABLE.value, 0),
            sorted({i.split for i in items})[0],
        ])
    lines += [
        _table(["session", "day", "rows", "confirmed", "conflict", "compound",
                "unknown", "unreadable", "split"], rows),
        "",
        "## Per-day roll-up",
        "",
    ]
    by_day: dict[str, list[EntityEvidence]] = defaultdict(list)
    for e in evs:
        d = session_day(e.session_id)
        if d:
            by_day[d].append(e)
    day_rows = []
    for d, items in sorted(by_day.items()):
        st = Counter(i.audit_status for i in items)
        day_rows.append([
            d, len(items), len({i.session_id for i in items}),
            st.get(AuditStatus.CONFIRMED_MATCH.value, 0),
            st.get(AuditStatus.COMPOUND_AMBIGUOUS.value, 0),
            st.get(AuditStatus.UNKNOWN_ENTITY.value, 0),
        ])
    lines += [
        _table(["day", "rows", "sessions", "confirmed", "compound", "unknown"], day_rows),
        "",
    ]
    path.write_text("\n".join(lines), encoding="utf-8")


def write_agreement_matrix(path: Path, evs: list[EntityEvidence]) -> None:
    """OCR vs stored label vs model prediction.

    The model is a third opinion only. Its predictions are never ground truth and
    the matrix is analysis, not a label source.
    """
    modelled = [e for e in evs if e.model_prediction]
    rows = []
    for (stored, model, ocr), n in Counter(
        (e.stored_entity_id or "(none)",
         e.model_prediction or "(none)",
         e.matched_entities[0] if e.matched_entities else (e.residual_text or "(unresolved)"))
        for e in modelled
    ).most_common(45):
        rows.append([f"`{stored}`", f"`{model}`", f"`{ocr}`", n])
    lines = [
        "# OCR Audit — Agreement matrix (stored / model / OCR)",
        "",
        f"{len(modelled)} rows carry a model prediction.",
        "",
        "The `fish_v1` model is a THIRD opinion here. It was not retrained, replaced or",
        "promoted, and its predictions are never treated as ground truth. TEST rows are",
        "included for reporting only; nothing in this pipeline tunes on them.",
        "",
        _table(["stored label", "model prediction", "OCR interpretation", "rows"], rows),
        "",
    ]
    path.write_text("\n".join(lines), encoding="utf-8")


def write_ocr_quality(path: Path, s: dict, evs: list[EntityEvidence]) -> None:
    """Character-level OCR error analysis, measured rather than guessed."""
    pairs: Counter = Counter()
    for e in evs:
        fresh = e.normalized_ocr
        stored = " ".join((e.stored_ocr_text or "").lower().split())
        if not stored or not fresh:
            continue
        # Align on equal-length prefixes to surface substitutions cheaply; only
        # reported as observations, never turned into correction rules.
        for a, b in zip(stored, fresh):
            if a != b:
                pairs[(a, b)] += 1
    lines = [
        "# OCR Audit — OCR quality",
        "",
        "## Confidence distribution",
        "",
        _table(["statistic", "value"],
               [[k, s["confidence"][k]] for k in
                ("n", "min", "p10", "median", "p90", "max", "mean")]),
        "",
        "## Correction rules that fired",
        "",
        "Each rule exists to fix a NAMED failure mode and is covered by a unit test.",
        "No rule was derived from frequency alone.",
        "",
        _table(["rule", "times applied"],
               [[f"`{k}`", v] for k, v in s["correction_rules"].items()])
        if s["correction_rules"] else "_None fired._\n",
        "",
        "## Most common single-character disagreements",
        "",
        "Observed between the collector's stored text and this audit's fresh read, on",
        "rows where both are non-empty. These are OBSERVATIONS for a human to judge.",
        "This pipeline adds no rule from this table automatically.",
        "",
        _table(["stored char", "fresh char", "count"],
               [[f"`{a}`", f"`{b}`", n] for (a, b), n in pairs.most_common(30)]),
        "",
        "## Rejected low-confidence detections",
        "",
        f"{s['rejected_detections']} detection box(es) fell below the confidence floor and",
        "were excluded from the reading. They are still recorded in the cache so the",
        "suppression is auditable rather than silent.",
        "",
    ]
    path.write_text("\n".join(lines), encoding="utf-8")


# --------------------------------------------------------------------------
# Search index (SQLite, FTS5 when available)
# --------------------------------------------------------------------------

def write_search_index(path: Path, evs: list[EntityEvidence],
                       concepts: list[dict]) -> None:
    """Queryable index: concept text -> images, sessions, labels, candidates.

    Built so a human can type `Crimson Snapper` and immediately get every image,
    OCR variant, session, stored label and ontology candidate behind it.
    """
    if path.exists():
        path.unlink()
    con = sqlite3.connect(path)
    cur = con.cursor()
    cur.executescript("""
        CREATE TABLE evidence (
            image_id TEXT PRIMARY KEY, image_sha256 TEXT, session_id TEXT,
            day TEXT, split TEXT, stored_entity_id TEXT, stored_source TEXT,
            stored_ocr TEXT, raw_ocr TEXT, normalized_ocr TEXT,
            ocr_confidence REAL, ocr_engine TEXT, match_kind TEXT,
            ontology_status TEXT, audit_status TEXT, image_category TEXT,
            residual_text TEXT, candidates TEXT, matched TEXT,
            compound INTEGER, model_prediction TEXT, model_confidence REAL,
            human_reviewed INTEGER, review_priority INTEGER, compound_flag TEXT
        );
        CREATE TABLE concepts (
            concept TEXT PRIMARY KEY, occurrences INTEGER, sessions INTEGER,
            days INTEGER, candidates TEXT, stored_labels TEXT, statuses TEXT,
            fragments TEXT, example_images TEXT, mean_confidence REAL,
            raw_variants TEXT, corrections TEXT
        );
        CREATE INDEX ix_evidence_residual ON evidence(residual_text);
        CREATE INDEX ix_evidence_ocr ON evidence(normalized_ocr);
        CREATE INDEX ix_evidence_status ON evidence(audit_status);
        CREATE INDEX ix_evidence_label ON evidence(stored_entity_id);
        CREATE INDEX ix_evidence_session ON evidence(session_id);
        CREATE INDEX ix_evidence_conf ON evidence(compound_flag);
    """)
    cur.executemany(
        "INSERT INTO evidence VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        [(
            e.image_id, e.image_sha256, e.session_id, session_day(e.session_id), e.split,
            e.stored_entity_id, e.stored_source, e.stored_ocr_text, e.raw_ocr,
            e.normalized_ocr, e.ocr_confidence, e.ocr_engine, e.match_kind,
            e.ontology_status, e.audit_status, e.image_category, e.residual_text,
            json.dumps(e.candidate_entities), json.dumps(e.matched_entities),
            1 if e.compound else 0, e.model_prediction, e.model_confidence,
            1 if e.human_reviewed else 0, e.review_priority, " ".join(e.raw_ocr.split()),
        ) for e in evs],
    )
    cur.executemany(
        "INSERT INTO concepts VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
        [(
            g["concept"], g["occurrences"], g["session_count"], g["day_count"],
            json.dumps(g["candidates"]), json.dumps(g["stored_labels"]),
            json.dumps(g["statuses"]), json.dumps(g["fragments"]),
            json.dumps(g["example_images"]), g["mean_confidence"],
            json.dumps(g["raw_variants"]), json.dumps(g["corrections"]),
        ) for g in concepts],
    )
    con.commit()
    con.close()


def query_index(path: Path, term: str, limit: int = 40) -> dict[str, Any]:
    """Search helper used by the CLI and by a human at the prompt."""
    con = sqlite3.connect(path)
    con.row_factory = sqlite3.Row
    cur = con.cursor()
    like = f"%{term.lower()}%"
    concepts = cur.execute(
        "SELECT * FROM concepts WHERE concept LIKE ? "
        "ORDER BY occurrences DESC LIMIT ?", (like, limit)
    ).fetchall()
    rows = cur.execute(
        "SELECT * FROM evidence WHERE lower(residual_text) LIKE ? "
        "   OR lower(normalized_ocr) LIKE ? OR lower(compound_flag) LIKE ? "
        "   OR lower(raw_ocr) LIKE ? "
        "ORDER BY review_priority DESC, image_id LIMIT ?",
        (like, like, like, like, limit),
    ).fetchall()
    con.close()
    return {
        "term": term,
        "concepts": [dict(r) for r in concepts],
        "images": [dict(r) for r in rows],
    }


# --------------------------------------------------------------------------
# Contact sheets
# --------------------------------------------------------------------------

def write_contact_sheets(out_dir: Path, evs: list[EntityEvidence],
                         dataset_root: Path, concepts: list[dict],
                         classes: dict, limit: int = 24) -> list[str]:
    """Render grid sheets of the ACTUAL source images.

    Every thumbnail is captioned with image_id, stored label, OCR text and
    ontology status so a reviewer can judge the text analysis without opening
    each file. Originals are opened read-only; nothing is written back to the
    dataset.
    """
    try:
        from PIL import Image, ImageDraw
    except ImportError:
        return []

    sheet_dir = out_dir / "contact_sheets"
    sheet_dir.mkdir(parents=True, exist_ok=True)
    images_dir = dataset_root / "images"
    written: list[str] = []

    groups: list[tuple[str, list[EntityEvidence]]] = []

    # Required sheets: the four worst classes plus the unresolved groups.
    for cls, c in classes.items():
        if c["total"] == 0:
            continue
        rows = sorted((e for e in evs if e.stored_entity_id == cls),
                      key=lambda e: (-e.review_priority, e.image_id))[:limit]
        if rows:
            groups.append((f"class-{slug(cls)}", rows))
    for g in concepts:
        if g["statuses"].get(AuditStatus.COMPOUND_AMBIGUOUS.value, 0) == 0:
            continue
        if g["occurrences"] < 2:
            continue
        ids = {i for i in g["example_images"]}
        rows = sorted((e for e in evs if e.image_id in ids),
                      key=lambda e: e.image_id)[:limit]
        groups.append((f"compound-{slug(g['concept'])}", rows))
    for g in concepts:
        if g["statuses"].get(AuditStatus.UNKNOWN_ENTITY.value, 0) == 0:
            continue
        if g["occurrences"] < 2:
            continue
        ids = set(g["example_images"])
        rows = sorted((e for e in evs if e.image_id in ids), key=lambda e: e.image_id)[:limit]
        groups.append((f"unknown-{slug(g['concept'])}", rows))

    cols = 4
    cell_w, cell_h = 300, 150
    cap_h = 54
    for name, rows in groups:
        if not rows:
            continue
        rcount = (len(rows) + cols - 1) // cols
        sheet = Image.new("RGB", (cols * cell_w, rcount * (cell_h + cap_h)), (18, 18, 22))
        draw = ImageDraw.Draw(sheet)
        for i, e in enumerate(rows):
            cx = (i % cols) * cell_w
            cy = (i // cols) * (cell_h + cap_h)
            path = images_dir / f"{e.image_id}.png"
            try:
                with Image.open(path) as im:
                    im = im.convert("RGB")
                    im.thumbnail((cell_w - 4, cell_h - 4))
                    sheet.paste(im, (cx + 2, cy + 2))
            except Exception:
                draw.text((cx + 6, cy + 6), "<unreadable>", fill=(200, 80, 80))
            label = (e.stored_entity_id or "(none)")
            text = (e.residual_text or e.normalized_ocr or e.raw_ocr or "(no text)")[:34]
            y = cy + cell_h + 2
            for line, colour in (
                (f"{e.image_id}", (150, 200, 255)),
                (f"stored: {label}", (220, 220, 220)),
                (f"ocr: {text}", (170, 170, 170)),
                (f"{e.audit_status}", (255, 190, 120)),
            ):
                draw.text((cx + 4, y), line[:44], fill=colour)
                y += 12
        dest = sheet_dir / f"{name}.png"
        sheet.save(dest)
        written.append(str(dest.relative_to(out_dir.parent)))
    return written