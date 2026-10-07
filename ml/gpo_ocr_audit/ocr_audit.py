"""Audit orchestrator: image -> fresh OCR -> ontology -> one verdict per row.

This module owns the decision that turns evidence into a single primary
``audit_status``. Everything else (reading, matching, reporting) is elsewhere.

THE STATUS RULE
===============

Exactly one status per image, chosen by precedence. The order encodes a simple
principle: a stronger claim about the text beats a weaker one, and "I could not
read it" always beats "it means something".

1. ``OCR_UNREADABLE``     no text recovered (empty, or the engine failed)
2. ``NON_FISH``           text is a known banner template with no entity name
3. ``COMPOUND_AMBIGUOUS`` two or more whole-word canonical candidates
4. ``UNKNOWN_ENTITY``     a real name the KB cannot account for, in whole or
                          in part (a known species plus unexplained words, or a
                          KB name occurring only as a fragment of a longer word)
5. ``OCR_CONFLICT``       exactly one candidate, and it is NOT the stored label
6. ``CONFIRMED_MATCH``    exactly one candidate, equal to the stored label,
                          reached exactly or after normalisation
7. ``ALIAS_MATCH``        as above, reached through a declared alias
8. ``OCR_VARIANT_MATCH``  as above, reached through a registered OCR repair or
                          lossy confusable folding
9. ``NO_DECISION``        a single candidate but nothing to compare against

Note the ordering of 3/4 above 5/6. When a name is compound, the stored label is
irrelevant: there is no single answer to compare it to, so the row is ambiguous
regardless of what was stored. Collapsing that would recreate the original bug.
"""

from __future__ import annotations

import json
import os
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Iterable

from . import ocr_engine
from .gpo_correlator import DatasetIndex, ModelRunner, find_latest_snapshot
from .ontology_matcher import Ontology, load_ontology
from .schemas import (
    AuditStatus,
    EntityEvidence,
    ImageCategory,
    MatchKind,
)
from .text_normalizer import normalize_pipeline

PIPELINE_VERSION = "ocr-audit/1.0.0"

#: Rules-version stamp. Bump when OCR_CORRECTION_RULES changes, so a cached
#: analysis can never be attributed to a rule set that did not produce it.
NORMALIZATION_RULES_VERSION = "ocr-rules/1.0.0"


@dataclass
class RunConfig:
    dataset_root: Path
    repo_root: Path
    out_dir: Path
    workers: int | None = None
    limit: int = 0
    use_model: bool = True
    quiet: bool = False


def _log(cfg: RunConfig, msg: str) -> None:
    if not cfg.quiet:
        print(msg, file=sys.stderr, flush=True)


def categorise(row, match, ont: Ontology) -> str:
    """Decide what KIND of frame this is.

    Region and game state come from the collector's metadata; the entity's KB
    category comes from the matched text. Where they disagree the frame is still
    classified by what it depicts, and the disagreement is visible in the
    evidence record rather than smoothed over.
    """
    if match.status.value == "UNREADABLE":
        return ImageCategory.OCR_FAILURE.value

    region = (row.region_name or "") if row else ""
    state = (row.game_state or "") if row else ""

    if region in ("drop_banner", "confirm_banner"):
        return ImageCategory.BANNER.value
    if state == "waiting_for_bite":
        return ImageCategory.WAITING.value
    if state == "bite":
        return ImageCategory.BITE.value

    # A drop frame: the entity's own KB category decides fish vs fruit vs bait.
    cats = set(match.categories.values())
    for ent in match.matched:
        cats.add(ont.category_of(ent))
    for ent in match.candidates:
        cats.add(ont.category_of(ent))
    if "fish" in cats:
        return ImageCategory.FISH_DROP.value
    if "fruit" in cats:
        return ImageCategory.FRUIT_DROP.value
    if "bait" in cats:
        return ImageCategory.BAIT_DROP.value

    # No entity resolved. Use the stored label's category purely as a fallback
    # description of the crop, never as evidence of correctness.
    if row and row.entity_id:
        cat = ont.category_of(row.entity_id)
        if cat == "fish":
            return ImageCategory.FISH_DROP.value
        if cat == "fruit":
            return ImageCategory.FRUIT_DROP.value
        if cat == "bait":
            return ImageCategory.BAIT_DROP.value
    if region == "bar":
        return ImageCategory.UI.value
    if match.status.value == "UI_TEMPLATE" or match.ui_templates:
        return ImageCategory.UI.value
    return ImageCategory.UNKNOWN.value


def is_entity_frame(row) -> bool:
    """Does this crop ever display an entity name?

    Only the drop region (and the two banners) does. A fishing-bar crop during
    WAITING or BITE shows a countdown and a bait count, so any text found there
    is UI, not an entity name.

    This distinction matters because without it every bar frame reading ``x1``
    becomes an UNKNOWN_ENTITY, which would bury the handful of rows that really
    are unresolved concepts under thousands of UI frames.
    """
    if row is None:
        return True
    if row.region_name in ("drop", "drop_banner", "confirm_banner"):
        return True
    return (row.game_state or "") == "catch_result"


def classify(row, match, stored_entity: str | None, is_confirmable_kind=None) -> AuditStatus:
    """Pick the single primary status. See the module docstring for precedence.

    ``is_confirmable_kind`` is accepted for call-site symmetry with older
    revisions and is intentionally unused: confirmability is a property of the
    resulting status, not an input to it.
    """
    del is_confirmable_kind
    status = match.status.value

    if status == "UNREADABLE":
        return AuditStatus.OCR_UNREADABLE

    if status == "COMPOUND":
        # Two or more candidates: no single answer exists, so the stored label
        # is not even comparable. Ambiguity outranks conflict.
        return AuditStatus.COMPOUND_AMBIGUOUS

    if status == "UI_TEMPLATE":
        return AuditStatus.NON_FISH

    unknown = (
        match.kind in (MatchKind.NONE, MatchKind.FUZZY, MatchKind.PARTIAL)
        or status == "UNKNOWN_ENTITY"
    )
    if unknown:
        if match.kind == MatchKind.NONE and not match.residual:
            return AuditStatus.NON_FISH
        if not is_entity_frame(row):
            # Text on a frame that never shows an entity is UI text. Record it,
            # but do not raise it to an ontology question.
            return AuditStatus.NON_FISH
        return AuditStatus.UNKNOWN_ENTITY

    # From here there is exactly one candidate entity.
    matched = match.matched[0] if match.matched else None

    if not is_entity_frame(row):
        # An entity name was read off a bar/idle frame. That is either a false
        # OCR read or a UI label; it can never confirm a stored drop label.
        return AuditStatus.NO_DECISION

    if stored_entity and matched and matched != stored_entity:
        return AuditStatus.OCR_CONFLICT

    if not stored_entity:
        # A clean single match, but there is nothing stored to confirm it
        # against. Reporting this as a confirmation would overstate what we know.
        return AuditStatus.NO_DECISION

    kind = MatchKind(match.kind)
    if kind in (MatchKind.EXACT, MatchKind.NORMALIZED):
        return AuditStatus.CONFIRMED_MATCH
    if kind == MatchKind.ALIAS:
        return AuditStatus.ALIAS_MATCH
    if kind in (MatchKind.CONFUSABLE, MatchKind.OCR_VARIANT):
        return AuditStatus.OCR_VARIANT_MATCH
    return AuditStatus.NO_DECISION


def score_priority(ev: EntityEvidence, session_count: int, day_count: int) -> int:
    """Rank a row for human review. Ordering ONLY.

    Returns a non-negative integer; higher means "look sooner". It can never
    change a label, and the function has no access to one.

    The intent is to spend a reviewer's attention where the cheap label is most
    likely wrong, and to spend less of it on rows where everything already
    agrees.
    """
    status = AuditStatus(ev.audit_status)
    score = 0

    if status == AuditStatus.COMPOUND_AMBIGUOUS:
        score += 50
    elif status == AuditStatus.UNKNOWN_ENTITY:
        score += 45
    elif status == AuditStatus.OCR_CONFLICT:
        score += 40
    elif status == AuditStatus.OCR_UNREADABLE:
        score += 10
    elif status == AuditStatus.NO_DECISION:
        score += 5
    elif status == AuditStatus.CONFIRMED_MATCH:
        score += 0

    # Confidence: a confident reading that disagrees is worth more attention.
    if ev.ocr_confidence is not None:
        score += int(round((ev.ocr_confidence - 0.5) * 20))

    # Recurrence across independent sessions and days is real evidence that a
    # concept is genuine rather than a one-off OCR slip.
    score += min(session_count, 10) * 3
    score += min(day_count, 6) * 4

    # A category mismatch (a FRUIT candidate inside a fish drop) usually means
    # the name is compound and the stored label is one arbitrary component.
    if ev.candidate_categories_mismatched:
        score += 8

    return max(score, 0)


def run(cfg: RunConfig) -> dict[str, Any]:
    """Execute the full audit and write every artefact.

    Returns a summary dict, which is also what the reports are built from.
    """
    t_start = time.time()
    out_dir = cfg.out_dir
    out_dir.mkdir(parents=True, exist_ok=True)

    _log(cfg, f"[1/6] loading dataset index from {cfg.dataset_root}")
    index = DatasetIndex(cfg.dataset_root)
    snap = find_latest_snapshot(cfg.repo_root)
    if snap is not None:
        index.attach_snapshot(snap)
    appdata = Path(cfg.dataset_root).parents[2]
    index.attach_reviews(appdata / "reviews.jsonl")

    _log(cfg, "[2/6] loading knowledge base export")
    ont: Ontology = load_ontology(str(Path(__file__).with_name("kb_export.json")))

    fingerprint = ocr_engine.engine_fingerprint(
        {"pipeline": PIPELINE_VERSION, "rules": NORMALIZATION_RULES_VERSION}
    )
    cache_path = out_dir / ".cache" / "ocr_cache.jsonl"
    cache = ocr_engine.OcrCache(cache_path)
    _log(cfg, f"      ocr engine={ocr_engine.ENGINE_NAME} v{ocr_engine.engine_version()} "
              f"fingerprint={fingerprint[:16]}...")

    rows = sorted(index.rows.values(), key=lambda r: r.image_id)
    if cfg.limit:
        rows = rows[: cfg.limit]
        _log(cfg, f"      LIMIT ACTIVE: only the first {cfg.limit} images will be audited")

    jobs = [(r.image_id, str(index.image_path(r.image_id))) for r in rows]

    _log(cfg, f"[3/6] reading {len(jobs)} images (cache: {cache.hits} hit / "
              f"{cache.misses} miss so far)")
    state = {"done": 0}

    def progress(total: int, done: int, _err: int) -> None:
        state["done"] = done
        if not cfg.quiet and done and done % 500 == 0:
            pct = 100.0 * done / max(total, 1)
            _log(cfg, f"      {done}/{total} ({pct:.0f}%)")

    readings = ocr_engine.read_all(jobs, cache, fingerprint,
                                   workers=cfg.workers, progress=progress)
    _log(cfg, f"      OCR done: {len(readings)} reading(s), cache "
              f"{cache.hits} hit / {cache.misses} miss")

    _log(cfg, "[4/6] loading shipped fish model for cross-check")
    runner = None
    if cfg.use_model:
        runner = ModelRunner.load(cfg.repo_root / "src-tauri" / "models", "fish_v1")
        if runner is None or not runner.available:
            _log(cfg, f"      model unavailable ({runner.load_error if runner else 'missing'})"
                      " - continuing without the model column")
        else:
            _log(cfg, f"      model {runner.manifest.get('name')} "
                      f"sha={str(runner.manifest.get('sha256'))[:16]}...")

    _log(cfg, "[5/6] analysing text against the ontology")
    evidences: list[EntityEvidence] = []
    for row in rows:
        rec = readings.get(row.image_id, {})
        raw = rec.get("text") or ""
        normalized, steps, applied, _folded = normalize_pipeline(raw)
        match = ont.match(raw, normalized)

        review = index.reviews.get(row.image_id)
        model_pred, model_conf = (None, None)
        if runner is not None and runner.available:
            model_pred, model_conf = runner.predict(index.image_path(row.image_id))

        category = categorise(row, match, ont)
        status = classify(row, match, row.entity_id, lambda: False)

        ev = EntityEvidence(
            image_id=row.image_id,
            image_sha256=rec.get("image_sha256") or "",
            relative_image_path=f"images/{row.image_id}.png",
            session_id=row.session_id,
            timestamp_ms=row.timestamp_ms,
            stored_entity_id=row.entity_id,
            stored_source=row.annotator or "unknown",
            stored_ocr_text=row.ocr_text,
            ocr_engine=ocr_engine.ENGINE_NAME,
            ocr_engine_version=rec.get("engine_version") or ocr_engine.engine_version(),
            raw_ocr=raw,
            normalized_ocr=normalized,
            ocr_confidence=rec.get("confidence"),
            normalization_steps=steps,
            match_kind=match.kind.value,
            ontology_status=match.status.value,
            matched_entities=match.matched,
            candidate_entities=match.candidates,
            compound=len(match.candidates) > 1,
            compound_tokens=match.compound_tokens,
            ui_templates=match.ui_templates,
            residual_text=match.residual,
            fuzzy_candidates=match.fuzzy,
            correction_rules_applied=applied,
            image_category=category,
            audit_status=status.value,
            requires_human_review=AuditStatus(status).needs_human
                                  and review is None,
            human_reviewed=review is not None,
            human_review_status=review.review_status if review else None,
            in_latest_snapshot=row.image_id in index.snapshot_ids,
            split=row.split,
            model_prediction=model_pred,
            model_confidence=model_conf,
            explanation=match.reason,
        )
        ev.game_state = row.game_state or ""
        ev.region_name = row.region_name or ""
        ev.ocr_error = rec.get("error") or ""
        ev.rejected_detections = len(rec.get("rejected") or [])
        ev.candidate_categories = match.categories
        # A candidate whose KB category differs from the stored label's is a
        # strong hint that the name is compound and the label is one component.
        stored_cat = ont.category_of(row.entity_id) if row.entity_id else ""
        ev.candidate_categories_mismatched = any(
            c != stored_cat and c not in ("unknown", "") for c in match.categories.values()
        )
        evidences.append(ev)

    _log(cfg, "[6/6] scoring priorities and writing evidence")
    concept_index = build_concept_index(evidences)
    for ev in evidences:
        sessions = concept_index.get(_concept_key(ev), set())
        days = {s.split("_")[1] for s in sessions if len(s.split("_")) > 1}
        ev.review_priority = score_priority(ev, len(sessions), len(days))
        ev.priority_reasons = _reasons(ev, len(sessions), len(days))

    evidence_path = out_dir / "evidence.jsonl"
    with evidence_path.open("w", encoding="utf-8", newline="\n") as fh:
        for ev in sorted(evidences, key=lambda e: e.image_id):
            fh.write(json.dumps(ev.to_dict(), sort_keys=True, ensure_ascii=False) + "\n")

    provenance = {
        "pipeline_version": PIPELINE_VERSION,
        "normalization_rules_version": NORMALIZATION_RULES_VERSION,
        "generated_at_unix": int(t_start),
        "dataset": index.stats(),
        "dataset_manifest_sha256": _sha256_of(cfg.dataset_root / "manifest.json"),
        "labels_jsonl_sha256": _sha256_of(cfg.dataset_root / "labels.jsonl"),
        "fish_vocab_sha256": ont.fish_vocab_sha256,
        "kb_export_sha256": _sha256_of(Path(__file__).with_name("kb_export.json")),
        "kb": ont.summary(),
        "ocr_engine": ocr_engine.ENGINE_NAME,
        "ocr_engine_version": ocr_engine.engine_version(),
        "ocr_engine_fingerprint": fingerprint,
        "model": runner.provenance() if runner else None,
        "training_snapshot_used": str(snap.name) if snap else None,
        "evidence_sha256": _sha256_of(evidence_path),
        "evidence_records": len(evidences),
        "elapsed_seconds": round(time.time() - t_start, 1),
        "workers": cfg.workers or ocr_engine.default_workers(),
        "cuda_used": False,
    }
    (out_dir / "provenance.json").write_text(
        json.dumps(provenance, indent=2, sort_keys=True), encoding="utf-8"
    )
    return {"evidences": evidences, "index": index, "ontology": ont,
            "provenance": provenance, "concepts": concept_index}


def _concept_key(ev: EntityEvidence) -> str:
    """Grouping key for a visual concept.

    Uses the residual name text, falling back to the normalised text, so every
    OCR spelling of one concept lands in one group.
    """
    key = ev.residual_text or ev.normalized_ocr or ev.raw_ocr
    return " ".join(key.lower().split())


def _reasons(ev: EntityEvidence, sessions: int, days: int) -> list[str]:
    out: list[str] = []
    st = AuditStatus(ev.audit_status)
    if st == AuditStatus.COMPOUND_AMBIGUOUS:
        out.append(f"compound name with {len(ev.candidate_entities)} candidates")
    if st == AuditStatus.UNKNOWN_ENTITY:
        out.append(f"name not accounted for by KB v{ev.ontology_status}")
    if st == AuditStatus.OCR_CONFLICT:
        out.append(f"OCR says {ev.matched_entities} but row stores {ev.stored_entity_id}")
    if ev.candidate_categories_mismatched:
        out.append("a candidate belongs to a different KB category than the stored label")
    if sessions > 1:
        out.append(f"seen in {sessions} independent session(s)")
    if days > 1:
        out.append(f"seen on {days} calendar day(s)")
    if ev.ocr_confidence is not None and ev.ocr_confidence >= 0.8:
        out.append(f"high OCR confidence ({ev.ocr_confidence:.2f})")
    return out


def build_concept_index(evidences: Iterable[EntityEvidence]) -> dict[str, set[str]]:
    """Concept text -> the set of sessions it appears in."""
    out: dict[str, set[str]] = {}
    for ev in evidences:
        if not ev.residual_text and ev.audit_status != AuditStatus.NON_FISH.value:
            continue
        out.setdefault(_concept_key(ev), set()).add(ev.session_id)
    return out


def _sha256_of(path: Path) -> str:
    try:
        return ocr_engine.sha256_file(path)
    except OSError:
        return ""