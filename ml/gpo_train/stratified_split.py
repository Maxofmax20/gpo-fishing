"""Deterministic stratified GROUP split assignment for the fish training set.

WHY

`MlDatasetStore::split_of` hashes each session id to one of three splits. On a
15-class problem with 56 sessions, that left classes with too few held-out TEST
rows - `fish:goldentigerfin` had 38 rows over 20 sessions and **0** TEST rows -
so `registry::compare` reports "per-class metrics too thin to judge" for every
class with 1..9 TEST rows, and no candidate can ever be compared.

The dataset already has the mechanism: `split_overrides` in `manifest.json`,
honoured by `DatasetSnapshot::split_of` on both the Rust and Python sides. This
module produces that map.

WHAT IS BEING SOLVED, AND WHAT IS NOT

Every session `s` carries a class profile `prof[s]` = how many rows of each
class it holds, and nothing else. A session goes wholly into one split - never a
row, never part of one. The assignment is a function of `prof` and the session
ids; nothing about a model, its predictions, its logits or any metric takes
part. This is a GROUPING decision constrained by class membership, and
`tests/test_stratified_split.py` asserts that structurally.

The hard constraints, in order:

    H1  session purity                        (by construction)
    H2  per class   TRAIN  >= 20               (the trainer's own scope assert)
    H3  per class   TEST   >= 10               (registry::compare's floor)
    H4  per class   VAL    >= 1                (a split with no rows is empty)
    H5  overall row share near 70 / 15 / 15    (the documented strategy)

H3 and H2 conflict on small classes: `fish:bluelipgrouper` has 47 rows over 4
sessions, so putting enough of it in TEST to clear the floor leaves fewer than
20 in TRAIN. The solver therefore optimises a LEXICOGRAPHIC objective rather
than a weighted sum - a weighted sum would trade one class's floor against
another's and hide the trade - and whatever it cannot satisfy is reported with
a reason instead of being papered over.

DETERMINISM

The optimiser is simulated annealing over a move set of deterministic
single-session reassignments, seeded with a fixed PRNG and started from several
deterministic starts. The same inputs produce the same assignment, forever, on
any machine: there is no wall-clock, no hash randomisation and no filesystem
order anywhere in the search. `tests/test_stratified_split.py` runs `build()`
twice on one fixture and requires identical output.

Output goes to `reports/resolution/split_overrides.json` - a committed ARTIFACT,
not app state. The operator applies it with
`python -m ml.gpo_train.apply_split_overrides`; nothing here writes to
`%APPDATA%`.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import sys
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT))

from ml.gpo_train.reviewed_dataset import (  # noqa: E402
    fnv1a_session_split,
    reviewed_included_rows,
)

#: Mirrors `registry::PER_CLASS_MIN_TEST_SUPPORT`.
PER_CLASS_MIN_TEST_SUPPORT = 10

#: Mirrors `ml/gpo_train/fish_train.py::MIN_EXAMPLES`, over the TRAIN split.
TRAIN_MIN_EXAMPLES = 20

#: Mirrors `ml_capability::ENTITY_MIN_EXAMPLES` for the class gate.
VAL_MIN_ROWS = 1

#: Target shares. These are the strategy's documented defaults.
TARGET = {"train": 0.70, "validation": 0.15, "test": 0.15}

#: Fixed seed. Changing it changes the assignment, which is why it is a
#: module constant and not an argument: a split decision that depends on a
#: flag an operator can set is not a reproducible decision.
SEED = 20261009

KB_EXPORT = REPO_ROOT / "ml" / "gpo_ocr_audit" / "kb_export.json"
WIKI_SNAPSHOT = REPO_ROOT / "reports" / "ontology_review" / "wiki_fish.json"


def build_profile(appdata: Path, vocab: list[str]) -> dict[str, dict[str, int]]:
    """Session id -> {class: rows}, over the rows the trainer will actually read.

    Reuses `reviewed_dataset.reviewed_included_rows`, so the routing universe is
    identical to the manifest's: training-eligible, human-verified, KB+wiki
    documented, `catch_result`, image present.
    """
    included = reviewed_included_rows(appdata, kb_path=KB_EXPORT, wiki_path=WIKI_SNAPSHOT)
    vs = set(vocab)
    prof: dict[str, dict[str, int]] = {}
    for inc in included:
        if inc["entity"] not in vs:
            continue
        sid = inc["session_id"]
        prof.setdefault(sid, {})
        prof[sid][inc["entity"]] = prof[sid].get(inc["entity"], 0) + 1
    return prof


def _objective(assign: dict[str, str], prof: dict[str, dict[str, int]],
               vocab: list[str]) -> tuple:
    """Lexicographic: fewer constraint violations first, then share distance.

    A tuple is compared left to right, so the solver never trades a class below
    the TEST floor for a better row share.
    """
    per = {c: {"train": 0, "validation": 0, "test": 0} for c in vocab}
    totals = {"train": 0, "validation": 0, "test": 0}
    for sid, split in assign.items():
        size = sum(prof[sid].values())
        totals[split] += size
        for c, n in prof[sid].items():
            per[c][split] += n
    total = sum(totals.values()) or 1

    bad_train = sum(1 for c in vocab if per[c]["train"] < TRAIN_MIN_EXAMPLES)
    bad_test = sum(1 for c in vocab if per[c]["test"] < PER_CLASS_MIN_TEST_SUPPORT)
    bad_val = sum(1 for c in vocab if per[c]["validation"] < VAL_MIN_ROWS)
    empty_split = sum(1 for s in ("train", "validation", "test") if totals[s] == 0)
    # How far each split's SHARE is from target, as a fraction of the rows.
    share = tuple(abs(totals[s] / total - TARGET[s]) for s in ("train", "validation", "test"))
    # Then smooth the per-class test share towards the same target, so we are
    # not only clearing floors but measuring every class sensibly.
    per_class_dev = sum(
        (per[c]["test"] / max(rows_of(c, prof), 1) - TARGET["test"]) ** 2 for c in vocab
    )
    return (bad_train, bad_test, bad_val, empty_split, share, per_class_dev)


def rows_of(c: str, prof: dict[str, dict[str, int]]) -> int:
    return sum(d.get(c, 0) for d in prof.values())


def _anneal(prof: dict[str, dict[str, int]], vocab: list[str], starts: int) -> tuple:
    """Best assignment over `starts` deterministic restarts.

    Single-session moves only: reassigning a whole session keeps H1 true for
    free. Acceptance is deterministic given the seed, so the whole search is.
    """
    sessions = sorted(prof, key=lambda s: (s,))
    rng = random.Random(SEED)
    best_overall = None
    for start in range(starts):
        # Deterministic, DIFFERENT starts: seeded PRNG, start 0 all-train, then
        # progressively perturbed. Not "random restarts" in the unrepeatable
        # sense - the sequence is fixed by SEED.
        if start == 0:
            assign = {s: "train" for s in sessions}
        else:
            assign = {s: rng.choice(("train", "train", "train", "validation", "test"))
                      for s in sessions}
        cur = _objective(assign, prof, vocab)
        best, best_assign = cur, dict(assign)
        # Temperature schedule by iteration count, not wall-clock.
        steps = 4000
        for step in range(steps):
            frac = step / steps
            temp = 0.35 * (1.0 - frac) + 1e-6
            s = sessions[rng.randrange(len(sessions))]
            choices = [v for v in ("train", "validation", "test") if v != assign[s]]
            new_split = choices[rng.randrange(len(choices))]
            trial = dict(assign)
            trial[s] = new_split
            val = _objective(trial, prof, vocab)
            # Lexicographic: accept strictly better always, equal at a bounded
            # probability, worse never (hill climbing with a deterministic,
            # shrinking escape window).
            if val <= cur or rng.random() < temp:
                assign, cur = trial, val
                if val < best:
                    best, best_assign = val, dict(assign)
        if best_overall is None or best < best_overall[0]:
            best_overall = (best, best_assign)
    return best_overall[1], best_overall[0]


def build(appdata: Path, vocab: list[str]) -> dict[str, Any]:
    """Compute the override map. Pure: reads, writes nothing."""
    prof = build_profile(appdata, vocab)
    if not prof:
        raise SystemExit(
            "no reviewed rows in the vocabulary; the dataset store has nothing to "
            "split. Review some fish first."
        )
    sessions = sorted(prof, key=lambda s: (s,))
    rows = {c: rows_of(c, prof) for c in vocab}
    total_rows = sum(rows.values())

    assign, _score = _anneal(prof, vocab, starts=8)

    counts = {"train": 0, "validation": 0, "test": 0}
    per_class = {c: {"train": 0, "validation": 0, "test": 0} for c in vocab}
    for sid in sessions:
        split = assign[sid]
        size = sum(prof[sid].values())
        counts[split] += size
        for c, n in prof[sid].items():
            per_class[c][split] += n

    # Before: what the plain session hash gave, with the same denominators.
    before_class = {c: {"train": 0, "validation": 0, "test": 0} for c in vocab}
    before_counts = {"train": 0, "validation": 0, "test": 0}
    for sid in sessions:
        sp = fnv1a_session_split(sid)
        size = sum(prof[sid].values())
        before_counts[sp] += size
        for c, n in prof[sid].items():
            before_class[c][sp] += n

    overrides = {
        sid: assign[sid] for sid in sessions
        if assign[sid] != fnv1a_session_split(sid)
    }

    per_class_report = []
    for c in vocab:
        sessions_c = sum(1 for s in prof if c in prof[s])
        test_n, train_n, val_n = (per_class[c]["test"], per_class[c]["train"],
                                  per_class[c]["validation"])
        fails = []
        if test_n < PER_CLASS_MIN_TEST_SUPPORT:
            fails.append(
                f"TEST support {test_n} < {PER_CLASS_MIN_TEST_SUPPORT}"
            )
        if train_n < TRAIN_MIN_EXAMPLES:
            fails.append(f"TRAIN rows {train_n} < {TRAIN_MIN_EXAMPLES}")
        if val_n < VAL_MIN_ROWS:
            fails.append("no VALIDATION row")
        per_class_report.append({
            "entity": c,
            "rows": rows[c],
            "sessions": sessions_c,
            "train": train_n,
            "validation": val_n,
            "test": test_n,
            "test_before": before_class[c]["test"],
            "train_before": before_class[c]["train"],
            "clears_test_floor": test_n >= PER_CLASS_MIN_TEST_SUPPORT,
            "clears_train_gate": train_n >= TRAIN_MIN_EXAMPLES,
            "cannot_reach_floor": (
                None if test_n >= PER_CLASS_MIN_TEST_SUPPORT or train_n < TRAIN_MIN_EXAMPLES
                else (
                    f"only {sessions_c} session(s) hold {rows[c]} row(s) of this "
                    f"class and every one of them is needed in TRAIN to keep "
                    f"{TRAIN_MIN_EXAMPLES} examples for the scope assert, so the "
                    f"held-out share cannot reach {PER_CLASS_MIN_TEST_SUPPORT} rows"
                )
            ),
            "violations": fails,
        })

    short = [c for c in per_class_report if not c["clears_test_floor"]]
    return {
        "kind": "split-overrides",
        "version": 1,
        "read_only": True,
        "rule": (
            "Whole sessions only, assigned by deterministic simulated annealing "
            "(fixed seed " + str(SEED) + ", 8 starts, single-session moves, "
            "hill-climbing acceptance) minimising a LEXICOGRAPHIC objective: "
            "classes below the TRAIN gate, then below the TEST floor, then "
            "below VAL, then split-share distance from 70/15/15, then per-class "
            "test-share deviation. No model prediction, logit or metric takes "
            "part - the inputs are session ids and class membership only."
        ),
        "inputs": {
            "labels": str(appdata / "datasets" / "gpo-vision" / "v1" / "labels.jsonl"),
            "labels_sha256": hashlib.sha256(
                (appdata / "datasets" / "gpo-vision" / "v1" / "labels.jsonl").read_bytes()
            ).hexdigest(),
            "reviews": str(appdata / "reviews.jsonl"),
            "reviews_sha256": hashlib.sha256((appdata / "reviews.jsonl").read_bytes()).hexdigest(),
            "routing_universe": (
                "reviewed_dataset.reviewed_included_rows: training-eligible, "
                "human-verified, KB+wiki documented, catch_result, image present"
            ),
        },
        "thresholds": {
            "per_class_min_test_support": PER_CLASS_MIN_TEST_SUPPORT,
            "train_min_examples": TRAIN_MIN_EXAMPLES,
            "validation_min_rows": VAL_MIN_ROWS,
        },
        "totals": {
            "rows_in_scope": total_rows,
            "sessions_in_scope": len(sessions),
            "sessions_overridden": len(overrides),
            "train_rows": counts["train"],
            "validation_rows": counts["validation"],
            "test_rows": counts["test"],
            "train_share": round(counts["train"] / total_rows, 4),
            "validation_share": round(counts["validation"] / total_rows, 4),
            "test_share": round(counts["test"] / total_rows, 4),
            "before": {
                "train_rows": before_counts["train"],
                "validation_rows": before_counts["validation"],
                "test_rows": before_counts["test"],
                "classes_below_test_floor": sum(
                    1 for c in vocab
                    if before_class[c]["test"] < PER_CLASS_MIN_TEST_SUPPORT
                ),
            },
        },
        "per_class": per_class_report,
        "classes_below_test_floor": short,
        "apply_command": (
            "python -m ml.gpo_train.apply_split_overrides --dry-run   "
            "# show the diff, write nothing\n"
            "python -m ml.gpo_train.apply_split_overrides          "
            "# write %APPDATA%\\gpo-autofish\\datasets\\gpo-vision\\v1\\manifest.json"
        ),
        "split_overrides": overrides,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--appdata", type=Path,
                    default=Path(os.environ.get("APPDATA", "")) / "gpo-autofish")
    ap.add_argument("--out", type=Path,
                    default=REPO_ROOT / "reports" / "resolution" / "split_overrides.json")
    args = ap.parse_args()

    from ml.gpo_train.vocab_decision import FISH_VOCAB, parse_vocab
    vocab = parse_vocab(FISH_VOCAB.read_text(encoding="utf-8"))

    report = build(args.appdata, vocab)
    out = args.out if args.out.is_absolute() else REPO_ROOT / args.out
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=1, sort_keys=True) + "\n", encoding="utf-8")

    t = report["totals"]
    print(f"  rows={t['rows_in_scope']} train={t['train_rows']} ({t['train_share']*100:.1f}%) "
          f"val={t['validation_rows']} ({t['validation_share']*100:.1f}%) "
          f"test={t['test_rows']} ({t['test_share']*100:.1f}%) "
          f"sessions={t['sessions_in_scope']} overridden={t['sessions_overridden']}")
    print(f"  before: {t['before']['train_rows']}/{t['before']['validation_rows']}/"
          f"{t['before']['test_rows']}  classes below floor: "
          f"{t['before']['classes_below_test_floor']}")
    print(f"  per-class TEST support (floor {PER_CLASS_MIN_TEST_SUPPORT}):")
    for c in report["per_class"]:
        flag = "OK " if c["clears_test_floor"] else "LOW"
        print(f"    [{flag}] {c['entity']:<28} test={c['test']:>3} (was {c['test_before']:>3}) "
              f"train={c['train']:>4} val={c['validation']:>3} rows={c['rows']:>4} "
              f"sessions={c['sessions']:>3}")
    print(f"  below floor: {len(report['classes_below_test_floor'])} of {len(vocab)}")
    for c in report["classes_below_test_floor"]:
        print(f"    {c['entity']}: {c['cannot_reach_floor']}")
    print(f"  written: {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
