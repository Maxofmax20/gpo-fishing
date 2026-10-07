"""Candidate fish labels from OCR text, scored against the game's wiki bestiary.

WHAT THIS IS
============

A proposal file. For every image whose text can be read at all, this scores the
observed OCR against the 16 fish the game actually has, and records which one it
most likely is plus how confident that is.

It writes a NEW file. It never touches `labels.jsonl`, never sets
`training_eligible`, and never records a review. A candidate is a machine opinion,
and a machine opinion is not a label.

WHY IT IS ONLY A PROPOSAL
=========================

Measured on the live dataset, not assumed:

* **11,436 of 14,805 rows (77%) carry no OCR text at all.** There is nothing to
  score. No amount of cleverness here can label them; they need re-collection or a
  different signal.
* Where text exists it is noisy in specific, repeatable ways:
  `AnseLfish` for Angelfish, `Tiserfin` for Tigerfin, `CSndyuCom` for CandyCorn,
  and a garbled banner prefix (`uevjltem`, `Nev; Item`) glued to the front.
* Fresh OCR and the collector's OCR agree only **21.9%** of the time.

So a confidence tier is not decoration; it is the difference between a usable
proposal and a wrong label stated confidently. Three tiers:

`confident` (>= 0.80)
    Offered for bulk confirmation. A human still confirms every one, but can do it
    by looking at a grid rather than reading each row.
`guessable` (0.65 - 0.80)
    Shown, never pre-selected. Usually the right fish with a garbled letter, but
    also the band where a banner word starts to look like a species.
`below` (< 0.65)
    Not proposed. Most are system text such as "spooksville astuo ...".

The negative case matters as much as the positive: a row with no text is recorded
as `no_text`, never as "not a fish". Those are different claims, and conflating them
would quietly shrink the dataset.

WHAT A CONFIRMED CANDIDATE BUYS
===============================

Nothing by itself. A confirmed candidate becomes a `reviews.jsonl` record through the
existing human review path, and only then can a row become training-eligible. This
module's job is to make that review fast, not to skip it.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import time
import unicodedata
from dataclasses import dataclass, field
from difflib import SequenceMatcher
from pathlib import Path
from typing import Any, Iterable

#: Confidence at or above which a candidate is offered for confirmation.
CONFIDENT_AT = 0.80

#: Below this, nothing is proposed. System text and banner fragments land here.
MIN_CONFIDENCE = 0.65

TIERS = ("confident", "guessable", "below", "no_text")


def norm_key(s: str) -> str:
    """Reduce text to its comparison key: no accents, punctuation or spaces."""
    s = unicodedata.normalize("NFKD", s)
    s = "".join(c for c in s if not unicodedata.combining(c))
    return re.sub(r"[^a-z0-9]+", "", s.lower())


def _windows(text: str, size: int) -> Iterable[str]:
    """Every substring of `text` with length `size`."""
    if len(text) <= size:
        yield text
        return
    for i in range(len(text) - size + 1):
        yield text[i : i + size]


def similarity(observed_key: str, target_key: str) -> float:
    """Best ratio between the observed text and a target name.

    Two passes, because the observed text has two shapes:

    * the WHOLE string, which handles a clean read; and
    * the best fixed-length WINDOW, which handles a name buried inside a garbled
      banner (`uevjltem«Crimson snapper*`).

    The window pass is what makes the banner-prefix problem tractable without
    hand-maintaining a list of every way the model mangles the word "Item".
    """
    if not observed_key or not target_key:
        return 0.0
    best = SequenceMatcher(None, observed_key, target_key).ratio()
    size = len(target_key)
    for w in _windows(observed_key, size):
        r = SequenceMatcher(None, w, target_key).ratio()
        if r > best:
            best = r
    return best


@dataclass
class Candidate:
    image_id: str
    session_id: str
    proposed_entity_id: str
    proposed_name: str
    confidence: float
    tier: str
    ocr_text: str
    stored_entity_id: str | None = None
    #: Whether the collector already stored a DIFFERENT entity. Recorded so the
    #: reviewer sees the disagreement instead of inheriting it.
    stored_conflicts: bool = field(default=False)

    def to_json(self) -> dict[str, Any]:
        return {
            "imageId": self.image_id,
            "sessionId": self.session_id,
            "proposedEntityId": self.proposed_entity_id,
            "proposedName": self.proposed_name,
            "confidence": round(self.confidence, 4),
            "tier": self.tier,
            "ocrText": self.ocr_text,
            "storedEntityId": self.stored_entity_id,
            "storedConflicts": self.stored_conflicts,
        }


def load_bestiary(path: Path) -> list[dict[str, Any]]:
    """The committed wiki snapshot. Its entries become the class vocabulary."""
    data = json.loads(path.read_text(encoding="utf-8"))
    entries = data.get("entries") or []
    if not entries:
        raise ValueError(f"{path} has no entries; refusing to label against nothing")
    return entries


def score_row(ocr_text: str, entries: list[dict[str, Any]]) -> tuple[dict[str, Any] | None, float]:
    """Best-scoring fish for one row, or (None, 0.0).

    The winner is the argmax over all 16. Ties break on the entry order in the
    snapshot, which is sorted by name, so the result is deterministic.
    """
    key = norm_key(ocr_text)
    if not key:
        return None, 0.0
    best_entry: dict[str, Any] | None = None
    best_score = 0.0
    for e in entries:
        r = similarity(key, e["matchKey"])
        if r > best_score:
            best_entry, best_score = e, r
    return best_entry, best_score


def tier_for(confidence: float, has_text: bool) -> str:
    if not has_text:
        # Explicitly NOT "below": no text is absence of evidence, not evidence that
        # the row is not a fish.
        return "no_text"
    if confidence >= CONFIDENT_AT:
        return "confident"
    if confidence >= MIN_CONFIDENCE:
        return "guessable"
    return "below"


def build_candidates(
    rows: Iterable[dict[str, Any]],
    entries: list[dict[str, Any]],
) -> tuple[list[Candidate], dict[str, Any]]:
    """Score every row. Returns (confident-tier candidates, per-class summary)."""
    out: list[Candidate] = []
    per_class: dict[str, dict[str, Any]] = {
        e["name"]: {"rows": 0, "sessions": set(), "confident": 0, "guessable": 0}
        for e in entries
    }
    tier_counts = dict.fromkeys(TIERS, 0)

    for r in rows:
        text = (r.get("ocr_text") or "").strip()
        entry, score = score_row(text, entries)
        tier = tier_for(score, bool(text))
        tier_counts[tier] += 1
        if entry is None or tier not in ("confident", "guessable"):
            continue

        stored = r.get("entity_id")
        c = Candidate(
            image_id=r["image_id"],
            session_id=r.get("session_id") or "",
            proposed_entity_id=f"fish:{norm_key(entry['name'])}",
            proposed_name=entry["name"],
            confidence=score,
            tier=tier,
            ocr_text=text,
            stored_entity_id=stored,
            stored_conflicts=bool(stored) and stored != f"fish:{norm_key(entry['name'])}",
        )
        out.append(c)

        p = per_class[entry["name"]]
        p["rows"] += 1
        p["sessions"].add(c.session_id)
        p[tier] += 1

    summary = {
        "tiers": tier_counts,
        "perClass": {
            name: {
                # Confident rows only: this is what a confirmation pass would cover.
                "confidentRows": v["confident"],
                "confidentSessions": len(v["sessions"]) if v["confident"] else 0,
                "allProposedRows": v["rows"],
            }
            for name, v in per_class.items()
        },
    }
    return out, summary


def write_candidates(path: Path, candidates: list[Candidate], summary: dict[str, Any],
                     *, source: str, bestiary_sha: str) -> dict[str, Any]:
    """Write the proposal file. The only thing this module writes."""
    path.parent.mkdir(parents=True, exist_ok=True)
    blob = "\n".join(
        json.dumps(c.to_json(), sort_keys=True) for c in candidates
    )
    path.write_text(blob + ("\n" if blob else ""), encoding="utf-8")
    meta = {
        "kind": "training_candidate_labels",
        "version": 1,
        "source": source,
        "bestiarySha256": bestiary_sha,
        "candidateCount": len(candidates),
        "tiers": summary["tiers"],
        "perClass": summary["perClass"],
        "thresholds": {"confidentAt": CONFIDENT_AT, "minConfidence": MIN_CONFIDENCE},
        "contentSha256": hashlib.sha256(blob.encode("utf-8")).hexdigest(),
        "warning": (
            "MACHINE PROPOSALS. Not labels. No row here is training-eligible until a "
            "human confirms it through the review store. Stored labels in the "
            "dataset disagree with these in many cases and the disagreement is "
            "recorded per row for the reviewer to see."
        ),
    }
    path.with_suffix(".meta.json").write_text(
        json.dumps(meta, indent=1, sort_keys=True), encoding="utf-8"
    )
    return meta

# ===========================================================================
# CLI
# ===========================================================================

def _default_dataset() -> Path:
    appdata = os.environ.get("APPDATA")
    if not appdata:
        raise SystemExit("APPDATA not set; pass --labels explicitly")
    return Path(appdata) / "gpo-autofish" / "datasets" / "gpo-vision" / "v1"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        prog="python -m ml.gpo_train.candidates",
        description=(
            "Propose fish labels from OCR text, scored against the game's wiki "
            "bestiary. Writes proposals only: no row becomes training-eligible."
        ),
    )
    ap.add_argument("--labels", type=Path, default=None,
                    help="labels.jsonl (default: the live dataset)")
    ap.add_argument("--wiki", type=Path,
                    default=Path("reports/ontology_review/wiki_fish.json"))
    ap.add_argument("--out", type=Path,
                    default=Path("reports/training_candidates/candidates.jsonl"))
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args(argv)

    labels = args.labels or (_default_dataset() / "labels.jsonl")
    if not labels.is_file():
        print(f"  no dataset at {labels}", file=sys.stderr)
        return 1

    t0 = time.perf_counter()
    with labels.open(encoding="utf-8") as fh:
        rows = [json.loads(line) for line in fh if line.strip()]
    entries = load_bestiary(args.wiki)
    cands, summary = build_candidates(rows, entries)
    snap = json.loads(args.wiki.read_text(encoding="utf-8"))
    meta = write_candidates(args.out, cands, summary,
                            source=str(labels), bestiary_sha=snap.get("contentSha256", ""))

    if not args.quiet:
        dt = time.perf_counter() - t0
        print(f"  scored {len(rows)} rows in {dt:.1f}s")
        print(f"  tiers: {summary['tiers']}")
        print(f"  wrote {meta['candidateCount']} proposals -> {args.out}")
        print()
        print(f"  {'CLASS':<26} {'ROWS':>5} {'SESS':>5}  needs >=20 rows and >=3 sessions")
        print("  " + "-" * 60)
        clearing = 0
        for name, v in sorted(summary["perClass"].items(),
                              key=lambda kv: -kv[1]["confidentRows"]):
            ok = v["confidentRows"] >= 20 and v["confidentSessions"] >= 3
            clearing += ok
            why = []
            if v["confidentRows"] < 20:
                why.append(f"rows<20 ({v['confidentRows']})")
            if v["confidentSessions"] < 3:
                why.append(f"sess<3 ({v['confidentSessions']})")
            print(f"  {name:<26} {v['confidentRows']:>5} {v['confidentSessions']:>5}  "
                  f"{'CLEARS' if ok else 'BLOCKED: ' + ', '.join(why)}")
        print("  " + "-" * 60)
        print(f"  {clearing} of {len(entries)} classes clear the training thresholds")
        print(f"  {summary['tiers']['confident']} rows need human confirmation")
        print()
        print("  These are PROPOSALS. No row is training-eligible until a human")
        print("  confirms it through the review store.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
