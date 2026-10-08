"""Resolution report: the label-vocabulary defect, measured before and after.

READ-ONLY over ``%APPDATA%\\gpo-autofish``. Replays, over every human-reviewed
fish row:

* the CURRENT runtime scoring (per-word ``score_word`` from
  ``core/perception.rs``, faithfully reimplemented), and
* the NEW phrase-first ladder (UI-template strip -> longest-span phrase ->
  per-word fallback), the fix this report accompanies,

and buckets every row by where the runtime lands relative to the human verdict
(``reviews.jsonl.human_entity_id``):

* ``correct_species``   - runtime id == human id AND human id is in the
                          15-class v2 vocabulary
* ``legacy_stem``       - runtime id is a bundled bare stem outside the v2
                          vocabulary (fish:snapper, fish:crimson, ...)
* ``different_species`` - runtime id is in the v2 vocabulary but != human id
* ``unknown``           - no confirmed resolution
* ``ambiguous``         - (new ladder only) the name matched >1 identity and
                          was correctly left unresolved

It also reports the perfect-model agreement CEILING: the share of rows whose
runtime id could agree with a perfect 15-class model's top class, against the
80% shadow-agreement floor (`readiness_min_shadow_agreement`).

Determinism: same inputs -> byte-identical report. Input SHA-256s included.

Usage: python -m gpo_train.resolution_report [--out reports/resolution/resolution_report.json]
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
FUZZY_THRESHOLD = 0.85
CONFIRM_THRESHOLD = 0.80
#: score_word's internal floor: max(fuzzy_threshold, 0.88).
FUZZY_FLOOR = 0.88

V2_CLASSES = {
    "fish:bluelipgrouper", "fish:candycornsquid", "fish:crimsonpolkapuffer",
    "fish:crimsonsnapper", "fish:darkskeletalshark", "fish:exotictigerfin",
    "fish:fangfish", "fish:goldenpolkapuffer", "fish:goldenribbonangelfish",
    "fish:goldentigerfin", "fish:jackobite", "fish:skeletalshark",
    "fish:swordfish", "fish:tigerfin", "fish:zebraribbonangelfish",
}


# ---------------------------------------------------------------------------
# fruit.rs ports (normalize / clean_ocr_word) - byte-faithful
# ---------------------------------------------------------------------------

def normalize(text: str) -> str:
    out: list[str] = []
    last_space = True
    for ch in text:
        c = ch.lower() if (ch.isalnum() or ch == "/") else " "
        if c == " ":
            if not last_space:
                out.append(" ")
            last_space = True
        else:
            out.append(c)
            last_space = False
    return "".join(out).strip()


def clean_ocr_word(w: str) -> str:
    return "".join(
        {"0": "o", "1": "i", "|": "i", "l": "i", "@": "a", "5": "s"}.get(c, c.lower())
        for c in w
    )


# ---------------------------------------------------------------------------
# strsim::jaro_winkler port (standard algorithm, prefix bonus 0.1, max 4)
# ---------------------------------------------------------------------------

def _jaro(a: str, b: str) -> float:
    if a == b:
        return 1.0
    la, lb = len(a), len(b)
    if not la or not lb:
        return 0.0
    window = max(la, lb) // 2 - 1
    if window < 0:
        window = 0
    a_match = [False] * la
    b_match = [False] * lb
    matches = 0
    for i in range(la):
        lo = max(0, i - window)
        hi = min(i + window + 1, lb)
        for j in range(lo, hi):
            if not b_match[j] and a[i] == b[j]:
                a_match[i] = True
                b_match[j] = True
                matches += 1
                break
    if matches == 0:
        return 0.0
    trans = 0
    j = 0
    for i in range(la):
        if a_match[i]:
            while not b_match[j]:
                j += 1
            if a[i] != b[j]:
                trans += 1
            j += 1
    trans //= 2
    return (matches / la + matches / lb + (matches - trans) / matches) / 3.0


def jaro_winkler(a: str, b: str) -> float:
    j = _jaro(a, b)
    prefix = 0
    for x, y in zip(a, b):
        if x == y and prefix < 4:
            prefix += 1
        else:
            break
    return j + prefix * 0.1 * (1.0 - j)


# ---------------------------------------------------------------------------
# KB model (from kb_export.json --effective)
# ---------------------------------------------------------------------------

class Kb:
    def __init__(self, payload: dict, overlay_ids: set[str]) -> None:
        self.entities = payload["entities"]  # exporter sorts by id
        self.ui_templates: list[tuple[str, str]] = []
        for e in self.entities:
            if e["category"] == "ui_term":
                for name in [e["canonical_name"], *(e.get("aliases") or []), *(e.get("ocr_aliases") or [])]:
                    n = normalize(name)
                    if n:
                        self.ui_templates.append((n, e["id"]))
        self.ui_templates.sort(key=lambda t: (-len(t[0]), t[0], t[1]))
        # phrase name index: normalized name -> sorted unique ids (non-ui)
        self.name_index: dict[str, list[str]] = {}
        for e in self.entities:
            if e["category"] == "ui_term":
                continue
            for name in [e["canonical_name"], *(e.get("aliases") or []), *(e.get("ocr_aliases") or [])]:
                n = normalize(name)
                if n:
                    ids = self.name_index.setdefault(n, [])
                    if e["id"] not in ids:
                        ids.append(e["id"])
        for ids in self.name_index.values():
            ids.sort()
        # alias-linkage for identity collapse (Tori <-> Phoenix)
        self.names_by_id = {
            e["id"]: {
                normalize(n)
                for n in [e["canonical_name"], *(e.get("aliases") or []), *(e.get("ocr_aliases") or [])]
            }
            for e in self.entities
        }
        self.canonical_by_id = {e["id"]: e["canonical_name"] for e in self.entities}
        # legacy stems = bundled fish ids that are NOT confirmed species
        fish_ids = {e["id"] for e in self.entities if e["category"] == "fish"}
        self.legacy_stems = (fish_ids - overlay_ids) - {"fish:swordfish"}


def _score_word(word: str, names: list[str], fuzzy_threshold: float) -> float | None:
    """score_word best-of exact/cleaned/fuzzy. Returns the raw score or None."""
    if len(word) < 3:
        return None
    lower = word.lower()
    for n in names:
        if n.lower() == word or n.lower() == lower:
            return 1.0
    cleaned = clean_ocr_word(word)
    if cleaned != lower:
        for n in names:
            if n.lower() == cleaned:
                return 0.95
    best = 0.0
    for n in names:
        if len(n) <= 4 or len(lower) <= 4:
            continue
        s = max(jaro_winkler(lower, n.lower()), jaro_winkler(cleaned, n.lower()))
        best = max(best, s)
    if best >= max(fuzzy_threshold, FUZZY_FLOOR):
        return best
    return None


def _word_stage(kb: Kb, stage_text: str, fuzzy_threshold: float) -> list[tuple[float, str, int]]:
    """The CURRENT per-word stage. Returns (raw_score, entity_id, kb_position)
    for every candidate, best first (raw desc, position asc)."""
    words = [w for w in stage_text.split() if len(w) >= 3]
    if not words:
        return []
    out: list[tuple[float, str, int]] = []
    for pos, e in enumerate(kb.entities):
        names = [e["canonical_name"], *(e.get("aliases") or []), *(e.get("ocr_aliases") or [])]
        best = None
        for w in words:
            s = _score_word(w, names, fuzzy_threshold)
            if s is not None and (best is None or s > best):
                best = s
        if best is None:
            continue
        # knowledge existence + Fishing context (all these categories plausible)
        score = best + 0.05 + 0.03
        out.append((score, e["id"], pos))
    out.sort(key=lambda t: (-t[0], t[2]))
    return out[:5]


def _strip_templates(kb: Kb, norm_text: str) -> str:
    residual = norm_text
    for tmpl, _id in kb.ui_templates:
        idx = residual.find(tmpl)
        if idx < 0:
            continue
        end = idx + len(tmpl)
        left_ok = idx == 0 or residual[idx - 1] == " "
        right_ok = end == len(residual) or residual[end] == " "
        if not (left_ok and right_ok):
            continue
        residual = residual[:idx] + " " + residual[end:]
    return " ".join(residual.split())


def _longest_spans(kb: Kb, residual: str) -> list[tuple[int, int, list[str]]]:
    tokens = residual.split()
    taken = [False] * len(tokens)
    spans: list[tuple[int, int, list[str]]] = []
    for name in sorted(kb.name_index, key=lambda n: (-len(n), n)):
        words = name.split()
        w = len(words)
        if not w or w > len(tokens):
            continue
        for start in range(len(tokens) - w + 1):
            if any(taken[start:start + w]):
                continue
            if tokens[start:start + w] == words:
                for i in range(start, start + w):
                    taken[i] = True
                spans.append((start, w, kb.name_index[name]))
    return spans


def _alias_components(kb: Kb, ids: list[str]) -> list[list[str]]:
    comps: list[list[str]] = []
    for i in ids:
        canon = normalize(kb.canonical_by_id.get(i, ""))
        placed = False
        for comp in comps:
            if any(canon in kb.names_by_id.get(o, set())
                   or normalize(kb.canonical_by_id.get(o, "")) in kb.names_by_id.get(i, set())
                   for o in comp):
                comp.append(i)
                placed = True
                break
        if not placed:
            comps.append([i])
    return comps


def resolve_current(kb: Kb, text: str) -> tuple[str | None, str]:
    """The pre-fix runtime: per-word stage on the full normalized text.
    Returns (confirmed entity id or None, detail)."""
    norm = normalize(text)
    cands = _word_stage(kb, norm, FUZZY_THRESHOLD)
    if cands and min(cands[0][0], 1.0) >= CONFIRM_THRESHOLD:
        return cands[0][1], "word-stage"
    return None, "no-candidate" if not cands else "below-threshold"


def resolve_new(kb: Kb, text: str) -> tuple[str | None, str]:
    """The phrase-first ladder. Returns (entity id or None, detail)."""
    norm = normalize(text)
    if not norm:
        return None, "empty"
    residual = _strip_templates(kb, norm)
    if residual:
        spans = _longest_spans(kb, residual)
        if spans:
            ids = sorted({i for _, _, owners in spans for i in owners})
            comps = _alias_components(kb, ids)
            if len(comps) > 1:
                return None, "ambiguous:" + ",".join(sorted(c[0] for c in comps))
            if any(w >= 2 for _, w, _ in spans):
                return comps[0][0], "phrase"
    stage_text = norm if not residual else residual
    cands = _word_stage(kb, stage_text, FUZZY_THRESHOLD)
    if cands and min(cands[0][0], 1.0) >= CONFIRM_THRESHOLD:
        return cands[0][1], "word-stage"
    return None, "no-candidate" if not cands else "below-threshold"


def bucket(kb: Kb, runtime_id: str | None, human_id: str | None, ambiguous: bool) -> str:
    """Bucket rules, identical to `examples/resolution_replay.rs` so the two
    replays are directly comparable."""
    if ambiguous:
        return "ambiguous"
    if runtime_id is None:
        return "unknown"
    if human_id in V2_CLASSES and runtime_id == human_id:
        return "correct_species"
    if runtime_id in kb.legacy_stems:
        return "legacy_stem"
    return "different_species"


def _sha256(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def _load_jsonl(p: Path) -> list[dict[str, Any]]:
    return [json.loads(l) for l in p.read_text(encoding="utf-8").splitlines() if l.strip()]


def run(appdata: Path, kb_path: Path = KB_EXPORT) -> dict[str, Any]:
    labels_path = appdata / "datasets" / "gpo-vision" / "v1" / "labels.jsonl"
    reviews_path = appdata / "reviews.jsonl"
    overlay_path = appdata / "knowledge_custom.json"

    labels = _load_jsonl(labels_path)
    reviews = _load_jsonl(reviews_path)
    overlay_ids = {e["id"] for e in json.loads(overlay_path.read_text(encoding="utf-8"))}
    kb = Kb(json.loads(kb_path.read_text(encoding="utf-8")), overlay_ids)

    ocr_by_image = {l["image_id"]: (l.get("ocr_text") or "") for l in labels if l.get("image_id")}
    collector_by_image = {l["image_id"]: l.get("entity_id") for l in labels if l.get("image_id")}

    buckets = {"before": {}, "after": {}}
    details: dict[str, dict[str, int]] = {"before": {}, "after": {}}
    ceiling_numer = {"before": 0, "after": 0}
    ceiling_denom = 0
    human_differs_collector = 0
    reviewed = 0

    for r in sorted(reviews, key=lambda x: x.get("image_id", "")):
        human = r.get("human_entity_id")
        if not human:
            continue
        reviewed += 1
        text = ocr_by_image.get(r["image_id"], "")
        if collector_by_image.get(r["image_id"]) != human:
            human_differs_collector += 1
        in_vocab = human in V2_CLASSES
        if in_vocab:
            ceiling_denom += 1

        rid_b, _ = resolve_current(kb, text)
        b = bucket(kb, rid_b, human, False)
        buckets["before"][b] = buckets["before"].get(b, 0) + 1
        if in_vocab and rid_b == human:
            ceiling_numer["before"] += 1

        rid_a, det = resolve_new(kb, text)
        a = bucket(kb, rid_a, human, det.startswith("ambiguous:"))
        buckets["after"][a] = buckets["after"].get(a, 0) + 1
        details["after"][det.split(":")[0]] = details["after"].get(det.split(":")[0], 0) + 1
        if in_vocab and rid_a == human:
            ceiling_numer["after"] += 1

    def shares(counts: dict[str, int]) -> dict[str, dict[str, Any]]:
        return {
            k: {"count": counts.get(k, 0), "share": counts.get(k, 0) / reviewed if reviewed else 0.0}
            for k in ("correct_species", "legacy_stem", "different_species", "unknown", "ambiguous")
        }

    return {
        "kind": "resolution-report",
        "version": 1,
        "inputs": {
            "labels": str(labels_path), "labels_sha256": _sha256(labels_path),
            "reviews": str(reviews_path), "reviews_sha256": _sha256(reviews_path),
            "kb_export": str(kb_path), "kb_export_sha256": _sha256(kb_path),
            "overlay_entities": len(overlay_ids),
        },
        "reviewed_rows_with_human_entity": reviewed,
        "v2_class_count": len(V2_CLASSES),
        "buckets_before": shares(buckets["before"]),
        "buckets_after": shares(buckets["after"]),
        "resolution_path_after": dict(sorted(details["after"].items())),
        "perfect_model_agreement_ceiling": {
            "definition": (
                "share of rows whose human entity is in the 15-class v2 vocabulary "
                "where the runtime-resolved id equals the human id - i.e. the only "
                "rows a perfect 15-class model could agree with the runtime on"
            ),
            "before": {"count": ceiling_numer["before"], "of": ceiling_denom,
                       "rate": ceiling_numer["before"] / ceiling_denom if ceiling_denom else 0.0},
            "after": {"count": ceiling_numer["after"], "of": ceiling_denom,
                      "rate": ceiling_numer["after"] / ceiling_denom if ceiling_denom else 0.0},
            "shadow_agreement_floor": 0.80,
            "floor_reachable_before": (ceiling_numer["before"] / ceiling_denom if ceiling_denom else 0.0) >= 0.80,
            "floor_reachable_after": (ceiling_numer["after"] / ceiling_denom if ceiling_denom else 0.0) >= 0.80,
        },
        "human_differs_from_collector": {
            "count": human_differs_collector,
            "of": reviewed,
            "meaning": "rows the in-app snapshot now relabels with the human verdict",
        },
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--appdata", type=Path,
                    default=Path(os.environ.get("APPDATA", "")) / "gpo-autofish")
    ap.add_argument("--out", type=Path,
                    default=REPO_ROOT / "reports" / "resolution" / "resolution_report.json")
    ap.add_argument("--vocab-decision", action="store_true",
                    help="also emit the fish-vocabulary decision report "
                         "(reports/resolution/vocab_decision.json); the arithmetic "
                         "lives in gpo_train.vocab_decision")
    ap.add_argument("--vocab-out", type=Path,
                    default=REPO_ROOT / "reports" / "resolution" / "vocab_decision.json")
    args = ap.parse_args()
    if args.vocab_decision:
        # Delegated, not duplicated: one implementation of the projection, one
        # place where either option could be "chosen" by accident.
        from . import vocab_decision
        return vocab_decision.main_for(args.appdata, args.vocab_out)
    report = run(args.appdata)
    out = args.out if args.out.is_absolute() else REPO_ROOT / args.out
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=1, sort_keys=True) + "\n", encoding="utf-8")

    def line(name: str, d: dict[str, Any]) -> str:
        return f"    {name:<18} {d['count']:>5}  {d['share'] * 100:5.1f}%"

    print(f"  reviewed rows: {report['reviewed_rows_with_human_entity']}")
    print("  BEFORE (per-word runtime):")
    for k, v in report["buckets_before"].items():
        print(line(k, v))
    print("  AFTER (phrase-first ladder):")
    for k, v in report["buckets_after"].items():
        print(line(k, v))
    c = report["perfect_model_agreement_ceiling"]
    print(f"  ceiling: {c['before']['count']}/{c['before']['of']} = {c['before']['rate'] * 100:.1f}% "
          f"-> {c['after']['count']}/{c['after']['of']} = {c['after']['rate'] * 100:.1f}% "
          f"(floor 80%; reachable after: {c['floor_reachable_after']})")
    print(f"  human != collector: {report['human_differs_from_collector']['count']} "
          f"of {report['human_differs_from_collector']['of']}")
    print(f"  written: {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
