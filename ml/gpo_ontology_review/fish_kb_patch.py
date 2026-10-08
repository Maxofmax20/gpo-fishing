"""Fish KB patch: audit -> generate -> validate. Applies NOTHING.

WHY THIS EXISTS
===============

The human label-confirmation phase produced 1,865 reviews whose final entity
assignments name 16 real fish. 15 of those fish do not exist in the knowledge
base, which is the only thing keeping their rows out of training eligibility.

This module is the ANALYSIS half of the fix. It:

* ``audit``    - measures the current state from the real stores and writes a
                 before-state report. Nothing is assumed: rows, sessions and
                 days are recomputed from the review log joined with the
                 dataset's capture timestamps, KB membership from the effective
                 KB (bundled export + persisted overlay), thresholds parsed
                 from ``ml_capability.rs`` so they cannot drift from the code.
* ``generate`` - emits ``fish_kb_patch.json``: every mutation explicit, every
                 entity traceable to (a) human-confirmed review rows and
                 (b) the committed wiki bestiary snapshot. Old atomic fish
                 entities are CLASSIFIED (valid / legacy stem), never deleted.
* ``validate`` - re-checks a patch file: identity, provenance, aliases,
                 historical mapping and dataset compatibility. Exit non-zero
                 on any error.

The APPLY half lives in Rust (``examples/apply_kb_patch.rs``), because the KB's
canonical mutation path is the persisted overlay merged by
``config::Store::effective_knowledge`` through the validated
``KnowledgeBase::merge_import`` pipeline. This module never writes the overlay,
never touches the review log, and never edits eligibility flags.

ENTITY-ID POLICY
================

A review row's ``human_entity_id`` is the human-confirmed final entity and is
immutable evidence. Eligibility checks KB membership BY ID, so the KB entities
created here carry exactly the IDs the reviews reference
(``fish:crimsonsnapper``, ...). Each of those IDs equals ``fish:`` + the wiki
snapshot's ``matchKey`` for the same fish, so they are also deterministic from
the documented name - one canonical identity per fish, no parallel spellings.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]
WIKI_SNAPSHOT = REPO_ROOT / "reports" / "ontology_review" / "wiki_fish.json"
KB_EXPORT = REPO_ROOT / "ml" / "gpo_ocr_audit" / "kb_export.json"
FISH_VOCAB = REPO_ROOT / "FISH_VOCAB.txt"
ML_CAPABILITY = REPO_ROOT / "src-tauri" / "src" / "core" / "ml_capability.rs"
AUDIT_OUT = REPO_ROOT / "reports" / "ontology_review" / "fish_kb_audit_before.json"
PATCH_OUT = REPO_ROOT / "reports" / "ontology_review" / "fish_kb_patch.json"

PATCH_VERSION = 1
PATCH_SOURCE = "fish-kb-patch/v1"


def _appdata() -> Path:
    return Path(os.environ.get("APPDATA", "")) / "gpo-autofish"


def _reviews_path(appdata: Path) -> Path:
    return appdata / "reviews.jsonl"


def _overlay_path(appdata: Path) -> Path:
    return appdata / "knowledge_custom.json"


def _labels_path(appdata: Path) -> Path:
    return appdata / "datasets" / "gpo-vision" / "v1" / "labels.jsonl"


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _load_jsonl(path: Path) -> list[dict[str, Any]]:
    if not path.is_file():
        return []
    return [json.loads(l) for l in path.read_text(encoding="utf-8").splitlines() if l.strip()]


def load_thresholds() -> dict[str, int]:
    """Parse the training gate from the Rust source so it cannot drift."""
    src = ML_CAPABILITY.read_text(encoding="utf-8")
    out: dict[str, int] = {}
    for const in ("ENTITY_MIN_EXAMPLES", "ENTITY_MIN_SESSIONS", "FRUIT_MIN_ENTITIES"):
        m = re.search(rf"pub const {const}: usize = (\d+);", src)
        if not m:
            raise SystemExit(f"cannot parse {const} from {ML_CAPABILITY}")
        out[const.lower()] = int(m.group(1))
    return out


def load_wiki() -> dict[str, dict[str, Any]]:
    snap = json.loads(WIKI_SNAPSHOT.read_text(encoding="utf-8"))
    return {e["matchKey"]: e for e in snap["entries"]}


def load_effective_kb_ids(kb_path: Path = KB_EXPORT) -> set[str]:
    """Entity IDs in the EFFECTIVE KB.

    The committed export is regenerated with `kb_export --effective`, so it is
    the bundled base + the persisted overlay - the same KB the app runs with.
    """
    payload = json.loads(kb_path.read_text(encoding="utf-8"))
    return {e["id"] for e in payload["entities"]}


def load_overlay_ids(appdata: Path) -> set[str]:
    p = _overlay_path(appdata)
    if not p.is_file():
        return set()
    return {e["id"] for e in json.loads(p.read_text(encoding="utf-8"))}


def load_bundled_kb_ids(appdata: Path, kb_path: Path = KB_EXPORT) -> set[str]:
    """The compiled-in base = effective minus the persisted overlay.

    The legacy-stem classification applies to the compiled base only; overlay
    additions are curated, documented entities, never legacy stems.
    """
    return load_effective_kb_ids(kb_path) - load_overlay_ids(appdata)


def review_entity_stats(
    reviews: list[dict[str, Any]], labels_by_image: dict[str, dict[str, Any]]
) -> dict[str, dict[str, Any]]:
    """Rows / sessions / capture-days per human-confirmed entity, from evidence."""
    stats: dict[str, dict[str, Any]] = {}
    for r in reviews:
        eid = r.get("human_entity_id")
        if not eid:
            continue
        s = stats.setdefault(eid, {"rows": 0, "sessions": set(), "days": set(),
                                   "statuses": set(), "missing_labels_row": 0})
        s["rows"] += 1
        s["sessions"].add(r.get("session_id", ""))
        s["statuses"].add(r.get("review_status", ""))
        lab = labels_by_image.get(r.get("image_id", ""))
        if lab is None:
            s["missing_labels_row"] += 1
        else:
            ms = lab.get("timestamp_ms")
            if isinstance(ms, (int, float)):
                day = datetime.fromtimestamp(ms / 1000, tz=timezone.utc).date().isoformat()
                s["days"].add(day)
    return stats


def audit(appdata: Path, kb_path: Path = KB_EXPORT) -> dict[str, Any]:
    """The before-state report. Pure measurement; writes nothing but the report."""
    thresholds = load_thresholds()
    wiki = load_wiki()
    bundled_ids = load_bundled_kb_ids(appdata, kb_path)
    overlay_ids = load_overlay_ids(appdata)
    effective_ids = bundled_ids | overlay_ids

    reviews = _load_jsonl(_reviews_path(appdata))
    labels = _load_jsonl(_labels_path(appdata))
    labels_by_image = {l["image_id"]: l for l in labels if l.get("image_id")}

    stats = review_entity_stats(reviews, labels_by_image)
    by_status: dict[str, int] = {}
    for r in reviews:
        by_status[r.get("review_status", "?")] = by_status.get(r.get("review_status", "?"), 0) + 1

    entities: list[dict[str, Any]] = []
    for eid in sorted(stats):
        s = stats[eid]
        key = eid.split(":", 1)[1] if ":" in eid else eid
        w = wiki.get(key)
        eligible_rows = sum(
            1 for r in reviews
            if r.get("human_entity_id") == eid and r.get("training_eligible")
        )
        rows_ok = s["rows"] >= thresholds["entity_min_examples"]
        sessions_ok = len(s["sessions"]) >= thresholds["entity_min_sessions"]
        kb_present = eid in effective_ids
        conflict_only = s["statuses"] == {"CONFLICT"}
        reasons: list[str] = []
        if conflict_only:
            reasons.append("conflict: competing human verdicts, excluded until resolved")
        if not kb_present:
            reasons.append("KB membership missing")
        if not rows_ok:
            reasons.append(
                f"below ENTITY_MIN_EXAMPLES ({s['rows']}/{thresholds['entity_min_examples']})"
            )
        if not sessions_ok:
            reasons.append(
                f"below ENTITY_MIN_SESSIONS ({len(s['sessions'])}/{thresholds['entity_min_sessions']})"
            )
        entities.append({
            "entity_id": eid,
            "display_name": w["name"] if w else None,
            "wiki_documented": w is not None,
            "rows": s["rows"],
            "sessions": len(s["sessions"]),
            "days": len(s["days"]),
            "statuses": sorted(s["statuses"]),
            "kb_present": kb_present,
            "eligible_rows": eligible_rows,
            "threshold": {
                "rows_ok": rows_ok,
                "sessions_ok": sessions_ok,
                "clears": rows_ok and sessions_ok and not conflict_only,
            },
            "blocking_reason": "; ".join(reasons) if reasons else "none",
        })

    eligible_total = sum(1 for r in reviews if r.get("training_eligible"))
    vocab_lines = [l.strip() for l in FISH_VOCAB.read_text(encoding="utf-8").splitlines()
                   if l.strip() and not l.startswith("#")]
    kb_fish = sorted(i for i in bundled_ids if i.startswith("fish:"))

    return {
        "kind": "fish-kb-audit",
        "phase": "before",
        "generated_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "thresholds": thresholds,
        "thresholds_source": str(ML_CAPABILITY.relative_to(REPO_ROOT)),
        "kb": {
            "bundled_fish_entities": len(kb_fish),
            "overlay_present": bool(overlay_ids),
            "overlay_entities": len(overlay_ids),
            "fish_vocab_sha256": _sha256(FISH_VOCAB),
            "fish_vocab": vocab_lines,
        },
        "reviews": {
            "path": str(_reviews_path(appdata)),
            "sha256": _sha256(_reviews_path(appdata)),
            "total": len(reviews),
            "by_status": by_status,
            "eligible": eligible_total,
            "ineligible": len(reviews) - eligible_total,
        },
        "dataset": {
            "labels_path": str(_labels_path(appdata)),
            "labels_rows": len(labels),
            "review_rows_without_labels_row": sum(
                s["missing_labels_row"] for s in stats.values()
            ),
        },
        "entities": entities,
        "legacy_kb_fish": classify_legacy(kb_fish, wiki),
    }


def classify_legacy(kb_fish_ids: list[str], wiki: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    """Every bundled fish entity, classified against the wiki bestiary.

    Nothing here deletes or mutates: historical labels and the OCR matcher
    reference these entities. The classification decides which ones may act as
    canonical training classes (only documented fish) and records what a
    historical label most plausibly meant.
    """
    canonical_by_stem: dict[str, list[str]] = {}
    for key, entry in wiki.items():
        name = entry["name"].lower()
        for fish_id in kb_fish_ids:
            stem = fish_id.split(":", 1)[1]
            # A stem matches a documented name literally ("snapper" in "Crimson
            # Snapper") or with a trailing "fish" dropped ("pufferfish" reaches
            # the documented "Polka Puffer" names).
            variants = {stem, stem[:-4] if stem.endswith("fish") else stem}
            if any(v and v in name.replace("-", " ").replace("'", "") for v in variants):
                canonical_by_stem.setdefault(stem, []).append(f"fish:{key}")

    out: list[dict[str, Any]] = []
    for fish_id in kb_fish_ids:
        stem = fish_id.split(":", 1)[1]
        targets = sorted(canonical_by_stem.get(stem, []))
        if stem in wiki:
            role = "VALID_CANONICAL"
            reason = "documented in the wiki bestiary"
        elif targets:
            role = "LEGACY_STEM"
            reason = (
                "not a documented fish name; the documented fish containing this "
                f"stem are {', '.join(targets)}. Retained for the OCR matcher and "
                "historical labels; must not qualify new training rows"
            )
        else:
            role = "LEGACY_STEM"
            reason = (
                "not a documented fish name and no documented fish contains this "
                "stem. Retained for the OCR matcher and historical labels; must "
                "not qualify new training rows"
            )
        out.append({
            "entity_id": fish_id,
            "role": role,
            "migration_targets": targets,
            "ambiguous": len(targets) > 1,
            "reason": reason,
        })
    return out


def generate(appdata: Path, kb_path: Path = KB_EXPORT) -> dict[str, Any]:
    """Emit the reviewable patch artifact. Mutates NOTHING."""
    thresholds = load_thresholds()
    wiki = load_wiki()
    bundled_ids = load_bundled_kb_ids(appdata, kb_path)
    overlay_ids = load_overlay_ids(appdata)
    effective_ids = bundled_ids | overlay_ids

    reviews = _load_jsonl(_reviews_path(appdata))
    labels = _load_jsonl(_labels_path(appdata))
    labels_by_image = {l["image_id"]: l for l in labels if l.get("image_id")}
    stats = review_entity_stats(reviews, labels_by_image)

    snapshot = json.loads(WIKI_SNAPSHOT.read_text(encoding="utf-8"))
    retrieved = datetime.fromtimestamp(
        snapshot.get("retrievedAtUnix", 0), tz=timezone.utc
    ).date().isoformat()

    entities_to_add: list[dict[str, Any]] = []
    entities_reused: list[dict[str, Any]] = []
    unresolved: list[dict[str, Any]] = []

    for eid in sorted(stats):
        s = stats[eid]
        key = eid.split(":", 1)[1] if ":" in eid else eid
        w = wiki.get(key)
        evidence = {
            "human_reviewed_rows": s["rows"],
            "sessions": len(s["sessions"]),
            "days": len(s["days"]),
            "review_statuses": sorted(s["statuses"]),
        }
        if w is None:
            # Section 11: no wiki evidence -> never auto-create. Report and stop
            # for this concept. (fish:tiger reaches this branch.)
            unresolved.append({
                "entity_id": eid,
                **evidence,
                "reason": "no wiki bestiary entry; not one of the confirmed "
                          "documented fish. No entity is created for it.",
            })
            continue
        evidence.update({
            "wiki_name": w["name"],
            "wiki_url": w["url"],
            "wiki_rarity": w.get("rarity"),
            "wiki_seasonal": w.get("seasonal"),
            "wiki_snapshot_sha256": _sha256(WIKI_SNAPSHOT),
        })
        if eid in effective_ids:
            entities_reused.append({
                "entity_id": eid,
                "already_in": "overlay" if eid in overlay_ids else "bundled",
                **evidence,
            })
            continue
        entities_to_add.append({
            "entity": {
                "id": eid,
                "canonical_name": w["name"],
                "aliases": [],
                "category": "fish",
                "rarity": w.get("rarity"),
                "description": w.get("description"),
                "ocr_aliases": [],
                "visual_refs": [],
                "provenance": {
                    "source": (
                        f"{PATCH_SOURCE}: human-confirmed {s['rows']} reviewed rows "
                        f"over {len(s['sessions'])} sessions + wiki bestiary"
                    ),
                    "url": w["url"],
                    "updated": retrieved,
                },
            },
            "id_source": "human-confirmed review rows; equals 'fish:' + wiki matchKey",
            **evidence,
        })

    kb_fish = sorted(i for i in bundled_ids if i.startswith("fish:"))
    legacy = classify_legacy(kb_fish, wiki)
    # A migration entry may only point at entities that EXIST once the patch is
    # applied (already in the KB, or added by it). Anything else would be an
    # interpretation pointing into thin air, and the validator refuses those.
    after_ids = effective_ids | {a["entity"]["id"] for a in entities_to_add}
    migration_map = []
    for e in legacy:
        if e["role"] == "VALID_CANONICAL":
            continue
        targets = [t for t in e["migration_targets"] if t in after_ids]
        migration_map.append({
            "old_entity_id": e["entity_id"],
            "canonical_entity_id": targets[0] if len(targets) == 1 else None,
            "candidate_targets": targets,
            "reason": (
                "interpretation layer for historical labels only; raw labels are "
                "never rewritten"
                + ("; AMBIGUOUS - per-row human review already resolved these"
                   if e["ambiguous"] else "")
            ),
        })

    return {
        "kind": "fish-kb-patch",
        "version": PATCH_VERSION,
        "created_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "source": {
            "reviews": str(_reviews_path(appdata)),
            "reviews_sha256": _sha256(_reviews_path(appdata)),
            "wiki_snapshot": str(WIKI_SNAPSHOT.relative_to(REPO_ROOT)),
            "wiki_snapshot_sha256": _sha256(WIKI_SNAPSHOT),
            "kb_export": str(KB_EXPORT.relative_to(REPO_ROOT)),
            "kb_export_sha256": _sha256(KB_EXPORT),
            "thresholds": thresholds,
        },
        "entities_to_add": entities_to_add,
        "entities_to_update": [],
        "entities_reused": entities_reused,
        "aliases_to_add": [],
        "legacy_entities": legacy,
        "migration_map": migration_map,
        "unresolved_concepts": unresolved,
        "validation_results": [],
    }


def validate_patch(patch: dict[str, Any], appdata: Path,
                   kb_path: Path = KB_EXPORT) -> list[str]:
    """All checks from the patch-validation contract. Returns a list of errors."""
    errors: list[str] = []
    existing = load_effective_kb_ids(kb_path)
    wiki = load_wiki()

    # --- identity ------------------------------------------------------------
    seen_ids: set[str] = set()
    seen_names: set[str] = set()
    for item in patch["entities_to_add"]:
        ent = item["entity"]
        eid, name = ent["id"], ent["canonical_name"]
        if not eid.startswith("fish:") or ent["category"] != "fish":
            errors.append(f"{eid}: id/category family mismatch")
        if eid in seen_ids:
            errors.append(f"duplicate entity id in patch: {eid}")
        seen_ids.add(eid)
        norm = name.strip().lower()
        if norm in seen_names:
            errors.append(f"duplicate canonical name in patch: {name}")
        seen_names.add(norm)
        if eid in existing:
            errors.append(f"{eid}: already in KB; belongs in entities_reused")

    # --- provenance ----------------------------------------------------------
    for item in patch["entities_to_add"]:
        ent = item["entity"]
        prov = ent["provenance"]
        if not prov.get("source"):
            errors.append(f"{ent['id']}: no provenance source")
        url = prov.get("url") or ""
        if not url.startswith(("https://", "http://")):
            errors.append(f"{ent['id']}: provenance url missing or not http(s)")
        if not item.get("human_reviewed_rows"):
            errors.append(f"{ent['id']}: no human-reviewed rows as evidence")
        key = ent["id"].split(":", 1)[1]
        w = wiki.get(key)
        if w is None or w["name"] != ent["canonical_name"]:
            errors.append(f"{ent['id']}: canonical name does not match wiki evidence")

    # --- aliases -------------------------------------------------------------
    alias_owner: dict[str, str] = {}
    for item in patch["entities_to_add"]:
        ent = item["entity"]
        for alias in ent["aliases"] + ent["ocr_aliases"]:
            norm = alias.strip().lower()
            if not norm:
                errors.append(f"{ent['id']}: empty alias")
            if norm in alias_owner and alias_owner[norm] != ent["id"]:
                errors.append(
                    f"alias {alias!r} claimed by both {alias_owner[norm]} and {ent['id']}"
                )
            alias_owner[norm] = ent["id"]

    # --- historical mapping --------------------------------------------------
    add_ids = {i["entity"]["id"] for i in patch["entities_to_add"]}
    for m in patch["migration_map"]:
        if not m.get("old_entity_id") or not m.get("reason"):
            errors.append(f"migration entry missing old_entity_id/reason: {m}")
        tgt = m.get("canonical_entity_id")
        if tgt is not None and tgt not in add_ids and tgt not in existing:
            errors.append(f"migration target {tgt} exists nowhere after patch")
        for cand in m.get("candidate_targets", []):
            if cand not in add_ids and cand not in existing:
                errors.append(f"migration candidate {cand} exists nowhere after patch")

    # --- dataset compatibility ----------------------------------------------
    reviews = _load_jsonl(_reviews_path(appdata))
    review_ids = {r["human_entity_id"] for r in reviews if r.get("human_entity_id")}
    after = existing | add_ids
    for rid in sorted(review_ids):
        if rid not in after:
            errors.append(f"review-referenced entity {rid} would not exist after patch")
    # every review-referenced entity is either added, reused, or consciously
    # left unresolved with a recorded reason
    accounted = add_ids | {e["entity_id"] for e in patch["entities_reused"]} | {
        u["entity_id"] for u in patch["unresolved_concepts"]
    }
    for rid in sorted(review_ids):
        if rid not in accounted:
            errors.append(f"review-referenced entity {rid} is not accounted for in the patch")

    return errors


def _print_audit(a: dict[str, Any]) -> None:
    print(f"  thresholds: {a['thresholds']}  (from {a['thresholds_source']})")
    print(f"  KB: {a['kb']['bundled_fish_entities']} bundled fish, "
          f"overlay present: {a['kb']['overlay_present']}")
    print(f"  reviews: {a['reviews']['total']}  "
          f"eligible {a['reviews']['eligible']} / ineligible {a['reviews']['ineligible']}")
    print(f"  by status: {a['reviews']['by_status']}")
    print()
    print(f"  {'ENTITY':<28} {'rows':>5} {'sess':>4} {'days':>4} {'KB?':>4} {'elig':>5}  blocking")
    print("  " + "-" * 88)
    for e in a["entities"]:
        print(f"  {e['entity_id']:<28} {e['rows']:>5} {e['sessions']:>4} {e['days']:>4} "
              f"{'yes' if e['kb_present'] else 'NO':>4} {e['eligible_rows']:>5}  "
              f"{e['blocking_reason']}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("command", choices=["audit", "generate", "validate", "refresh-workstation"])
    ap.add_argument("--appdata", type=Path, default=_appdata())
    ap.add_argument("--patch", type=Path, default=PATCH_OUT)
    ap.add_argument("--phase", default="before", choices=["before", "after"])
    ap.add_argument("--out", type=Path, default=None)
    args = ap.parse_args()

    if args.command == "refresh-workstation":
        # Rebuild workstation/kb_entities.json from the CURRENT export, which the
        # caller has just regenerated with `kb_export --effective`. This is the
        # same transform the ontology pipeline uses (build_kb_entities), so the
        # review screens and the candidate builder see the KB the app runs with.
        from .workstation import build_kb_entities

        payload = json.loads(KB_EXPORT.read_text(encoding="utf-8"))
        data = build_kb_entities(payload)
        out = REPO_ROOT / "reports" / "ontology_review" / "workstation" / "kb_entities.json"
        out.write_text(json.dumps(data, indent=1, sort_keys=True) + "\n", encoding="utf-8")
        fish = sum(1 for e in data["entities"] if e["id"].startswith("fish:"))
        print(f"  kb_entities.json refreshed: {data['entityCount']} entities ({fish} fish)")
        return 0

    if args.command == "audit":
        report = audit(args.appdata)
        report["phase"] = args.phase
        out = args.out or AUDIT_OUT
        if not out.is_absolute():
            out = REPO_ROOT / out
        out.write_text(json.dumps(report, indent=1) + "\n", encoding="utf-8")
        _print_audit(report)
        print(f"\n  written: {out.relative_to(REPO_ROOT)}")
        return 0

    if args.command == "generate":
        patch = generate(args.appdata)
        errors = validate_patch(patch, args.appdata)
        patch["validation_results"] = (
            [{"check": "all", "ok": True}] if not errors
            else [{"check": "patch", "ok": False, "error": e} for e in errors]
        )
        args.patch.write_text(json.dumps(patch, indent=1) + "\n", encoding="utf-8")
        print(f"  entities to add : {len(patch['entities_to_add'])}")
        print(f"  entities reused : {len(patch['entities_reused'])}")
        print(f"  legacy classified: {len(patch['legacy_entities'])}")
        print(f"  migration entries: {len(patch['migration_map'])}")
        print(f"  unresolved       : {len(patch['unresolved_concepts'])}")
        print(f"  validation       : {'CLEAN' if not errors else f'{len(errors)} ERRORS'}")
        for e in errors:
            print(f"    ERROR: {e}")
        print(f"  written: {args.patch.relative_to(REPO_ROOT)}")
        return 1 if errors else 0

    # validate
    patch = json.loads(args.patch.read_text(encoding="utf-8"))
    errors = validate_patch(patch, args.appdata)
    if errors and all("already in KB; belongs in entities_reused" in e for e in errors):
        print("  patch already consumed by the effective KB: every entity is present.")
        print("  A re-apply is a no-op (idempotent), not an error.")
        return 0
    if errors:
        for e in errors:
            print(f"  ERROR: {e}")
        return 1
    print("  patch validates clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
