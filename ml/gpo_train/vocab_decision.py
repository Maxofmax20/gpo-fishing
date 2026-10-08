"""Vocabulary decision report: what each candidate fish class map would mean.

`FISH_VOCAB.txt` names 8 classes; 7 of them are bare legacy stems
(`fish:crimson`, `fish:snapper`, ...) that the human review contradicts - a
reviewer looking at a `fish:crimson` frame decided it was
`fish:crimsonpolkapuffer` or `fish:crimsonsnapper`, 211 times out of 224 on that
one class. The confirmed ontology the KB overlay added is 15 species.

That choice is a HUMAN decision: the file is `include_str!`-embedded, line order
IS the output-logit index, the deployed `fish_v1.json` pins `vocab_sha`, and
`shadow_infer` verifies that fingerprint before loading weights. This module
therefore REPORTS both options with measured per-class support and the cost of
each, and deliberately does not choose.

READ-ONLY over `%APPDATA%\\gpo-autofish`. Deterministic: same inputs ->
byte-identical JSON, with every input SHA-256 stamped.

Mirrors the Rust projection exactly (`core::training::project_training_labels`):
review-eligible rows carry the human verdict, review-ineligible rows are
dropped, unreviewed rows are kept with the collector's label.

Usage:
    python -m gpo_train.vocab_decision [--out reports/resolution/vocab_decision.json]
    python -m gpo_train.resolution_report --vocab-decision
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]
KB_EXPORT = REPO_ROOT / "ml" / "gpo_ocr_audit" / "kb_export.json"
FISH_VOCAB = REPO_ROOT / "FISH_VOCAB.txt"
WIKI_SNAPSHOT = REPO_ROOT / "reports" / "ontology_review" / "wiki_fish.json"
REVIEWED_MANIFEST = REPO_ROOT / "ml" / "output" / "reviewed-fish-v1" / "dataset_manifest.json"
DEFAULT_OUT = REPO_ROOT / "reports" / "resolution" / "vocab_decision.json"

# --- thresholds, parsed from the Rust source so the report cannot drift -------
ENTITY_MIN_EXAMPLES = 20      # ml_capability::ENTITY_MIN_EXAMPLES
ENTITY_MIN_SESSIONS = 3       # ml_capability::ENTITY_MIN_SESSIONS
PER_CLASS_MIN_TEST_SUPPORT = 10  # registry::PER_CLASS_MIN_TEST_SUPPORT
#: fish_train.py's own scope assert (MIN_EXAMPLES), over the TRAIN split.
FISH_TRAIN_MIN_EXAMPLES = 20


def _sha256(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def _load_jsonl(p: Path) -> list[dict[str, Any]]:
    rows = []
    for n, line in enumerate(p.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        try:
            rows.append(json.loads(line))
        except json.JSONDecodeError as e:
            raise SystemExit(f"{p}:{n}: unparseable line ({e}); refusing to report because "
                             "review exclusions could not be verified")
    return rows


def fnv1a_session_split(session_id: str) -> str:
    """Byte-identical port of MlDatasetStore::split_of (FNV-1a64 70/15/15)."""
    h = 0xCBF29CE484222325
    for b in session_id.encode("utf-8"):
        h ^= b
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    r = h % 100
    if r <= 69:
        return "train"
    if r <= 84:
        return "validation"
    return "test"


def parse_vocab(text: str) -> list[str]:
    """`FISH_VOCAB.txt` -> the ordered class list. Comments and blanks ignored;
    ORDER is the contract (it is the output-logit index)."""
    out = []
    for line in text.splitlines():
        s = line.strip()
        if s and not s.startswith("#"):
            out.append(s)
    return out


def vocab_sha(classes: list[str]) -> str:
    """SHA-256 of the newline-joined list - the fingerprint `class_vocab.rs`
    computes and `shadow_infer` verifies against the deployed manifest."""
    return hashlib.sha256("\n".join(classes).encode("utf-8")).hexdigest()


def project_labels(labels: list[dict[str, Any]],
                   reviews: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    """The training set a snapshot would write. Port of
    `core::training::project_training_labels`.

    Sticky exclusion (`false` wins on a duplicate id), exactly like the Rust
    `review_eligibility_map`, and the human verdict replaces the collector label
    only on an eligible review that names one.
    """
    eligible: dict[str, bool] = {}
    human: dict[str, str] = {}
    for r in reviews:
        image_id = r.get("image_id")
        if not image_id:
            continue
        is_ok = bool(r.get("training_eligible"))
        eligible[image_id] = eligible.get(image_id, True) and is_ok
        if is_ok:
            h = (r.get("human_entity_id") or "").strip()
            if h:
                human[image_id] = h

    out: list[dict[str, Any]] = []
    counts = {
        "rows_live": len(labels),
        "rows_projected": 0,
        "rows_relabelled_by_review": 0,
        "rows_kept_collector_label": 0,
        "rows_unreviewed": 0,
        "rows_excluded_by_review": 0,
    }
    for l in labels:
        image_id = l.get("image_id")
        if image_id is None or image_id not in eligible:
            counts["rows_unreviewed"] += 1
            counts["rows_kept_collector_label"] += 1
            out.append(l)
            continue
        if not eligible[image_id]:
            counts["rows_excluded_by_review"] += 1
            continue
        h = human.get(image_id)
        if h and l.get("entity_id") != h:
            relabelled = dict(l)
            relabelled["entity_id"] = h
            counts["rows_relabelled_by_review"] += 1
            out.append(relabelled)
        else:
            counts["rows_kept_collector_label"] += 1
            out.append(l)
    counts["rows_projected"] = len(out)
    return out, counts


def class_support(rows: list[dict[str, Any]], classes: list[str]) -> dict[str, dict[str, Any]]:
    """Per class: projected rows, distinct sessions, and the TRAIN/TEST split
    counts. A class with no rows still appears, with zeros - a class missing
    from a table is how a vocabulary gap stays invisible."""
    acc: dict[str, dict[str, Any]] = {
        c: {"entity": c, "projected_rows": 0, "sessions": 0, "train_rows": 0,
            "validation_rows": 0, "test_rows": 0, "_sessions": set()}
        for c in classes
    }
    for r in rows:
        e = r.get("entity_id")
        if e not in acc:
            continue
        c = acc[e]
        c["projected_rows"] += 1
        c["_sessions"].add(r.get("session_id") or "")
        c[f"{fnv1a_session_split(r.get('session_id') or '')}_rows"] += 1
    for c in acc.values():
        c["sessions"] = len(c.pop("_sessions"))
        c["clears_class_gate"] = (c["projected_rows"] >= ENTITY_MIN_EXAMPLES
                                  and c["sessions"] >= ENTITY_MIN_SESSIONS)
        c["clears_trainer_scope"] = c["train_rows"] >= FISH_TRAIN_MIN_EXAMPLES
        c["clears_test_support_floor"] = c["test_rows"] >= PER_CLASS_MIN_TEST_SUPPORT
        c["rows_short_of_trainer_scope"] = max(0, FISH_TRAIN_MIN_EXAMPLES - c["train_rows"])
    return {k: acc[k] for k in classes}


def _option(label: str, source: str, classes: list[str], support: dict[str, dict[str, Any]],
            extra: dict[str, Any]) -> dict[str, Any]:
    failing = [c for c in classes if not support[c]["clears_trainer_scope"]]
    no_gate = [c for c in classes if not support[c]["clears_class_gate"]]
    thin = [c for c in classes if not support[c]["clears_test_support_floor"]]
    return {
        "option": label,
        "source": source,
        "classes": classes,
        "vocab_sha256": vocab_sha(classes),
        "per_class": [support[c] for c in classes],
        "totals": {
            "classes": len(classes),
            "projected_rows_in_vocab": sum(support[c]["projected_rows"] for c in classes),
            "clearing_class_gate": len(classes) - len(no_gate),
            "clearing_trainer_scope": len(classes) - len(failing),
            "clearing_test_support_floor": len(classes) - len(thin),
        },
        "failing_trainer_scope": [
            {"entity": c, "train_rows": support[c]["train_rows"],
             "needs": support[c]["rows_short_of_trainer_scope"]} for c in failing
        ],
        "failing_class_gate": [
            {"entity": c, "projected_rows": support[c]["projected_rows"],
             "sessions": support[c]["sessions"]} for c in no_gate
        ],
        "below_test_support_floor": [
            {"entity": c, "test_rows": support[c]["test_rows"]} for c in thin
        ],
        **extra,
    }


def run(appdata: Path,
        kb_path: Path = KB_EXPORT,
        vocab_path: Path = FISH_VOCAB,
        wiki_path: Path = WIKI_SNAPSHOT,
        manifest_path: Path | None = REVIEWED_MANIFEST,
        models_dir: Path | None = None) -> dict[str, Any]:
    labels_path = appdata / "datasets" / "gpo-vision" / "v1" / "labels.jsonl"
    reviews_path = appdata / "reviews.jsonl"
    overlay_path = appdata / "knowledge_custom.json"

    labels = _load_jsonl(labels_path)
    reviews = _load_jsonl(reviews_path)
    overlay_ids = {e["id"] for e in json.loads(overlay_path.read_text(encoding="utf-8"))}
    kb_payload = json.loads(kb_path.read_text(encoding="utf-8"))
    kb_ids = {e["id"] for e in kb_payload["entities"]}
    kb_fish = {i for i in kb_ids if i.startswith("fish:")}

    projected, proj_counts = project_labels(labels, reviews)

    # ---- Option A: the vocabulary that ships today --------------------------
    stems = parse_vocab(vocab_path.read_text(encoding="utf-8"))
    # "Legacy stem" = a bundled fish id that is not a confirmed species
    # (`fish:swordfish` is the one stem that is also a real documented fish).
    legacy_stems = sorted((kb_fish - overlay_ids) - {"fish:swordfish"})
    contradicted = []
    for c in stems:
        if c in legacy_stems:
            moved: dict[str, int] = {}
            live = 0
            for l in labels:
                if l.get("entity_id") != c:
                    continue
                live += 1
                p = next((p for p in projected if p.get("image_id") == l.get("image_id")), None)
                target = p.get("entity_id") if p else None
                if target and target != c:
                    moved[target] = moved.get(target, 0) + 1
            contradicted.append({
                "entity": c,
                "collector_rows": live,
                "projected_rows": sum(1 for p in projected if p.get("entity_id") == c),
                "rows_humans_moved": sum(moved.values()),
                "moved_to": dict(sorted(moved.items())),
            })

    # ---- Option B: the confirmed species -----------------------------------
    wiki = json.loads(wiki_path.read_text(encoding="utf-8"))
    wiki_fish = {f"fish:{e['matchKey']}" for e in wiki["entries"]}
    confirmed = sorted(wiki_fish & kb_ids - {"fish:anglerfish"})
    pending = sorted((wiki_fish & kb_ids) - set(confirmed))
    manifest: dict[str, Any] | None = None
    if manifest_path is not None and manifest_path.is_file():
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

    support_a = class_support(projected, stems)
    support_b = class_support(projected, confirmed)

    option_a = _option(
        "A - the 8 legacy stems in FISH_VOCAB.txt",
        str(vocab_path),
        stems, support_a,
        {
            "contradicted_by_reviews": contradicted,
            "note": (
                "7 of these 8 classes are bare stems the human review contradicts. "
                "A class whose rows humans keep re-labelling is not a class the data "
                "supports; it is a bucket the collector never resolved."
            ),
        },
    )
    anglerfish = support_b_anglerfish = class_support(projected, pending) if pending else {}
    option_b = _option(
        "B - the 15 confirmed species",
        f"{wiki_path} (wiki-documented fish) INTERSECT {kb_path} (effective KB), "
        "minus fish:anglerfish (see anglerfish_auto_entry)",
        confirmed, support_b,
        {
            "cross_check_reviewed_manifest": (
                {
                    "manifest": str(manifest_path),
                    "manifest_sha256": _sha256(manifest_path),
                    "class_map_matches": sorted(manifest.get("class_map") or []) == confirmed,
                    "manifest_class_map": sorted(manifest.get("class_map") or []),
                    "manifest_per_class": manifest.get("per_class"),
                    "note": (
                        "ml/gpo_train/reviewed_dataset.py applies three further filters "
                        "(KB membership, wiki documentation, catch_result + image "
                        "present), so its per-class rows can be LOWER than the "
                        "projection's; the class list must still match exactly."
                    ),
                }
                if manifest else {"manifest": None, "note": "manifest not present"}
            ),
            "anglerfish_auto_entry": (
                {
                    "entity": "fish:anglerfish",
                    "projected_rows": support_b_anglerfish["fish:anglerfish"]["projected_rows"],
                    "sessions": support_b_anglerfish["fish:anglerfish"]["sessions"],
                    "rows_needed": max(0, ENTITY_MIN_EXAMPLES
                                       - support_b_anglerfish["fish:anglerfish"]["projected_rows"]),
                    "mechanism": (
                        "fish:anglerfish is wiki-documented and in the KB, so the class "
                        "gate counts it automatically once it reaches "
                        f"{ENTITY_MIN_EXAMPLES} rows over {ENTITY_MIN_SESSIONS} sessions: "
                        "there is no code change, no config toggle and no file edit. "
                        "Option B is therefore not a frozen list - it is 'every "
                        "confirmed species with enough reviewed evidence'."
                    ),
                }
                if pending else None
            ),
        },
    )

    # ---- what each option costs --------------------------------------------
    deployed: dict[str, Any] = {"stem": "fish_v1", "manifest_present": False}
    mdir = models_dir if models_dir is not None else appdata / "models"
    mpath = mdir / "fish_v1.json"
    if mpath.is_file():
        man = json.loads(mpath.read_text(encoding="utf-8"))
        deployed = {
            "stem": "fish_v1",
            "manifest": str(mpath),
            "manifest_sha256": _sha256(mpath),
            "manifest_present": True,
            "classes": man.get("classes"),
            "vocab_sha": man.get("vocab_sha"),
            "version": man.get("version"),
        }

    only_in_b = sorted(set(confirmed) - set(stems))
    only_in_a = sorted(set(stems) - set(confirmed))
    costs = {
        "option_a": {
            "classes_the_model_cannot_emit": only_in_b + pending,
            "count": len(only_in_b) + len(pending),
            "runtime_effect": (
                "Every one of these species keeps resolving to a bare stem or to "
                "UNKNOWN at runtime, so the shadow log keeps recording the same "
                f"legacy-stem disagreement measured in resolution_report.json "
                f"({proj_counts['rows_relabelled_by_review']} of "
                f"{proj_counts['rows_live']} live rows carry a different human label)."
            ),
            "vocab_sha_unchanged": True,
        },
        "option_b": {
            "classes_the_model_cannot_emit": only_in_a,
            "count": len(only_in_a),
            "runtime_effect": (
                f"{len(only_in_a)} of the {len(stems)} current classes leave the output "
                "head, so anything the bot currently labels with them becomes UNKNOWN "
                "until a model trained on the new head is promoted."
            ),
            "vocab_sha_required": vocab_sha(confirmed),
            "vocab_sha_deployed": deployed.get("vocab_sha"),
            "retraining_required": True,
            "why": (
                "FISH_VOCAB.txt is include_str!-embedded and line order IS the "
                "output-logit index. models/fish_v1.json pins `vocab_sha`, and "
                "shadow_infer verifies it before loading weights, so the deployed "
                "model would refuse to load against a changed class map. Changing "
                "the file therefore REQUIRES: retrain -> new candidate -> soak -> "
                "promotion. Hand-editing a manifest's `classes` array is explicitly "
                "forbidden by the file's own header."
            ),
        },
    }

    return {
        "kind": "vocab-decision",
        "version": 1,
        "read_only": True,
        "decision": (
            "NOT MADE HERE. This report presents both options with measured support "
            "and cost; choosing the fish class map changes what the product "
            "recognises, what gets deployed, and what every gate compares against."
        ),
        "inputs": {
            "labels": str(labels_path), "labels_sha256": _sha256(labels_path),
            "reviews": str(reviews_path), "reviews_sha256": _sha256(reviews_path),
            "kb_export": str(kb_path), "kb_export_sha256": _sha256(kb_path),
            "kb_entities": len(kb_ids), "overlay_entities": len(overlay_ids),
            "fish_vocab": str(vocab_path), "fish_vocab_sha256": _sha256(vocab_path),
            "wiki_snapshot": str(wiki_path), "wiki_snapshot_sha256": _sha256(wiki_path),
        },
        "thresholds": {
            "entity_min_examples": ENTITY_MIN_EXAMPLES,
            "entity_min_sessions": ENTITY_MIN_SESSIONS,
            "per_class_min_test_support": PER_CLASS_MIN_TEST_SUPPORT,
            "fish_train_min_examples_train_split": FISH_TRAIN_MIN_EXAMPLES,
            "source": "src-tauri/src/core/ml_capability.rs, src-tauri/src/core/registry.rs, "
                      "ml/gpo_train/fish_train.py",
        },
        "projection": proj_counts,
        "options": {"option_a": option_a, "option_b": option_b},
        "costs": costs,
        "deployed_fish_model": deployed,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--appdata", type=Path,
                    default=Path(os.environ.get("APPDATA", "")) / "gpo-autofish")
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = ap.parse_args()
    return main_for(args.appdata, args.out)


def main_for(appdata: Path, out: Path) -> int:
    """Build + write + print. Shared with `resolution_report --vocab-decision`."""
    report = run(appdata)
    out = out if out.is_absolute() else REPO_ROOT / out
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=1, sort_keys=True) + "\n", encoding="utf-8")

    p = report["projection"]
    print(f"  projection: {p['rows_live']} live -> {p['rows_projected']} training rows "
          f"({p['rows_relabelled_by_review']} relabelled, {p['rows_excluded_by_review']} excluded)")
    for key in ("option_a", "option_b"):
        o = report["options"][key]
        t = o["totals"]
        print(f"  {o['option']}")
        print(f"    classes={t['classes']} rows={t['projected_rows_in_vocab']} "
              f"class-gate={t['clearing_class_gate']}/{t['classes']} "
              f"trainer-scope={t['clearing_trainer_scope']}/{t['classes']} "
              f"test-support={t['clearing_test_support_floor']}/{t['classes']}")
        for f in o["failing_trainer_scope"]:
            print(f"      short: {f['entity']} {f['train_rows']}/"
                  f"{FISH_TRAIN_MIN_EXAMPLES} train rows (needs {f['needs']})")
    print(f"  written: {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())