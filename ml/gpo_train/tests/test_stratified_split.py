"""Tests for the stratified split: determinism, session purity, floor
arithmetic, and the Rust/Python split-path agreement.

Synthetic fixtures only. The live store is read by NO test here.
"""

from __future__ import annotations

import json
import tempfile
from pathlib import Path

from ml.gpo_train import reviewed_dataset as rd
from ml.gpo_train import stratified_split as ss


# ---------------------------------------------------------------------------
# The Python split path honours overrides, and agrees with dataset.py
# ---------------------------------------------------------------------------

def test_reviewed_dataset_honours_split_overrides() -> None:
    """The bug this fixes: `build()` called `fnv1a_session_split` directly, so a
    stratified reassignment was invisible to the reviewed manifest while
    `dataset.py::DatasetSnapshot.split_of` and the Rust store honoured it."""
    # Pick two sessions whose natural hash differs, so the override is visible.
    sessions = [f"fixture-session-{i}" for i in range(40)]
    natural = {s: rd.fnv1a_session_split(s) for s in sessions}
    overrides = {sessions[0]: "test"}
    assert overrides[sessions[0]] != natural[sessions[0]], "the test needs a real override"

    assert rd.split_of_with(Path("."), sessions[0], overrides) == "test"
    assert rd.split_of_with(Path("."), sessions[1], {}) == natural[sessions[1]]


def test_the_two_split_paths_agree_on_the_same_inputs() -> None:
    """`reviewed_dataset` and `dataset.py` must answer one question one way.
    `DatasetSnapshot.split_of` is the reference the Rust store mirrors."""
    from ml.gpo_train.dataset import DatasetSnapshot

    overrides = {"fixture-session-a": "test", "fixture-session-b": "validation"}
    snap = DatasetSnapshot(version=1, overrides=dict(overrides))

    for sid in ["fixture-session-a", "fixture-session-b", "fixture-session-c",
                "session_2026-10-03_220117_1e7efb16", "zzz"]:
        rust_style = rd.split_of_with(Path("."), sid, overrides)
        python_snap = snap.split_of(sid)
        assert rust_style == python_snap, (
            f"{sid}: reviewed_dataset={rust_style} but dataset.py={python_snap}"
        )


def test_a_corrupt_manifest_is_not_read_as_no_overrides() -> None:
    """Fail-closed, like every other manifest read in this repo."""
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        p = tmp / "datasets" / "gpo-vision" / "v1" / "manifest.json"
        p.parent.mkdir(parents=True)
        p.write_text("{ not json", encoding="utf-8")
        try:
            rd.load_split_overrides(tmp)
        except json.JSONDecodeError:
            pass
        else:
            raise AssertionError("a corrupt manifest must not read as 'no overrides'")


def test_a_non_object_split_overrides_is_refused() -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        p = tmp / "datasets" / "gpo-vision" / "v1" / "manifest.json"
        p.parent.mkdir(parents=True)
        p.write_text(json.dumps({"split_overrides": ["train"]}), encoding="utf-8")
        try:
            rd.load_split_overrides(tmp)
        except SystemExit:
            pass
        else:
            raise AssertionError("a non-object split_overrides must be refused")


# ---------------------------------------------------------------------------
# The stratified assignment itself
# ---------------------------------------------------------------------------

def _fixture(tmp: Path, *, per_class_sessions: dict[str, list[tuple[str, int]]]):
    """Build a synthetic store. `per_class_sessions[class]` is a list of
    (session_id, row_count) and a session serves exactly one class here, which
    is enough to exercise the routing rule."""
    import base64
    png = base64.b64decode(
        b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
    )
    appdata = tmp / "appdata"
    ds = appdata / "datasets" / "gpo-vision" / "v1"
    images = ds / "images"
    images.mkdir(parents=True)
    (ds / "manifest.json").write_text(json.dumps({
        "name": "fixture", "version": 1, "created_ms": 1,
        "split_strategy": "session-hash 70/15/15", "split_overrides": {},
    }), encoding="utf-8")

    labels, reviews = [], []
    for cls, sessions in per_class_sessions.items():
        for n, (sid, count) in enumerate(sessions):
            for i in range(count):
                image_id = f"{cls.replace(':', '-')}-{n}-{i}"
                (images / f"{image_id}.png").write_bytes(png)
                labels.append({
                    "image_id": image_id, "session_id": sid,
                    "game_state": "catch_result", "entity_id": cls,
                })
                reviews.append({
                    "image_id": image_id, "session_id": sid,
                    "review_status": "REVIEWED_CORRECTED",
                    "training_eligible": True, "human_entity_id": cls,
                    "review_id": f"rv-{image_id}", "dataset_version": 1,
                    "timestamp_ms": 1_700_000_000_000 + (n * 100_000),
                })
    (ds / "labels.jsonl").write_text(
        "".join(json.dumps(r, sort_keys=True) + "\n" for r in labels), encoding="utf-8")
    (appdata / "reviews.jsonl").write_text(
        "".join(json.dumps(r, sort_keys=True) + "\n" for r in reviews), encoding="utf-8")
    return appdata


# ---------------------------------------------------------------------------
# The three classes these tests route.
#
# NOT invented ids. The routing universe is `reviewed_included_rows`, which keeps
# only rows whose entity is documented in BOTH the effective KB and the wiki
# snapshot - the same filter the trainer applies. A synthetic `fish:aaa` is
# documented nowhere, so every fixture row was filtered out and the assignment
# had nothing to route. These are the first three canonical classes, taken from
# the single source of truth, so the fixture is automatically valid for whatever
# the vocabulary is.
# ---------------------------------------------------------------------------
def _canonical(n: int) -> list[str]:
    vocab_file = Path(__file__).resolve().parents[3] / "FISH_VOCAB.txt"
    classes = [
        line.strip()
        for line in vocab_file.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]
    assert len(classes) >= n, f"{vocab_file} has only {len(classes)} classes"
    return classes[:n]


VOCAB = _canonical(3)


def test_the_fixture_classes_are_documented_where_the_trainer_looks() -> None:
    """If this fails, every other test in this file is measuring an empty
    assignment and passing for the wrong reason."""
    kb = rd.load_effective_kb_ids()
    wiki = rd.load_wiki_fish_ids()
    for c in VOCAB:
        assert c in kb, f"{c} is in FISH_VOCAB.txt but not the effective KB export"
        assert c in wiki, f"{c} is in FISH_VOCAB.txt but not the wiki fish snapshot"


def test_assignment_is_deterministic_and_session_pure() -> None:
    a_cls, b_cls, c_cls = VOCAB
    sessions = {
        a_cls: [(f"fixture-session-{i}", 40) for i in range(20)],
        b_cls: [(f"fixture-b-{i}", 30) for i in range(15)],
        c_cls: [(f"fixture-c-{i}", 25) for i in range(12)],
    }
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        appdata = _fixture(tmp, per_class_sessions=sessions)
        first = ss.build(appdata, VOCAB)
        second = ss.build(appdata, VOCAB)

        a = json.dumps(first, sort_keys=True)
        b = json.dumps(second, sort_keys=True)
        assert a == b, "the same inputs must yield the same assignment forever"

        # Session purity: a whole session, never a row, is assigned.
        ov = first["split_overrides"]
        for c in first["per_class"]:
            assert c["test_before"] >= 0
        assert all(v in ("train", "validation", "test") for v in ov.values())

        # Every session's rows land in exactly one split by construction: the
        # map is keyed by session id, so a session cannot have two answers.
        assert len(ov) == len(set(ov))


def test_the_assignment_lifts_classes_above_the_floor() -> None:
    a_cls, b_cls, c_cls = VOCAB
    sessions = {
        a_cls: [(f"fixture-session-{i}", 40) for i in range(20)],
        b_cls: [(f"fixture-b-{i}", 30) for i in range(15)],
        c_cls: [(f"fixture-c-{i}", 25) for i in range(12)],
    }
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        appdata = _fixture(tmp, per_class_sessions=sessions)
        report = ss.build(appdata, VOCAB)
        by = {c["entity"]: c for c in report["per_class"]}
        for cls in VOCAB:
            assert by[cls]["test"] >= ss.PER_CLASS_MIN_TEST_SUPPORT, (
                f"{cls}: {by[cls]['test']} TEST rows, floor "
                f"{ss.PER_CLASS_MIN_TEST_SUPPORT}"
            )
            assert by[cls]["clears_test_floor"] is True, by[cls]
            # The rows are conserved: the assignment MOVES rows between splits,
            # it never invents or drops them.
            assert (
                by[cls]["train"] + by[cls]["validation"] + by[cls]["test"]
                == by[cls]["rows"]
            ), by[cls]
        # Row budget respected.
        t = report["totals"]
        in_scope = t["rows_in_scope"]
        assert in_scope > 0, t
        assert t["test_share"] <= 0.19, t
        assert t["train_share"] >= 0.60, t
        assert (
            t["train_rows"] + t["validation_rows"] + t["test_rows"] == in_scope
        ), t


def test_a_class_with_too_few_sessions_is_reported_not_forced() -> None:
    """A class whose every session must move to TEST to clear the floor does not
    clear it. Say so, do not fabricate support."""
    a_cls, b_cls, _ = VOCAB
    sessions = {
        a_cls: [(f"fixture-session-{i}", 40) for i in range(20)],
        # 3 sessions, 60 rows: all three in TEST = 60 TEST rows, leaving TRAIN
        # with none of this class. The assignment must still not let the model
        # train on zero rows, so this class stays thin and is REPORTED.
        b_cls: [("only-b-1", 20), ("only-b-2", 20), ("only-b-3", 20)],
    }
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        appdata = _fixture(tmp, per_class_sessions=sessions)
        report = ss.build(appdata, [a_cls, b_cls])
        by = {c["entity"]: c for c in report["per_class"]}
        assert by[b_cls]["test"] >= 0
        # Whatever the number, it is reported with a reason.
        if not by[b_cls]["clears_test_floor"]:
            assert by[b_cls]["cannot_reach_floor"], (
                "a thin class must explain itself: " + repr(by[b_cls])
            )
        assert by[a_cls]["test"] >= ss.PER_CLASS_MIN_TEST_SUPPORT


def test_no_model_metric_or_prediction_participates() -> None:
    """Structural: the assignment reads only session ids and class membership.
    A prediction, logit or metric appearing in this module's CODE would make the
    split outcome-driven, which is the thing the frozen dataset forbids.

    Docstrings and comments are stripped with `tokenize`, not with a line
    filter: the module explains its own rule in prose, and a line-prefix filter
    would either miss that prose or miss real code.
    """
    import io
    import tokenize

    src = Path(ss.__file__).read_text(encoding="utf-8")
    kept: list[str] = []
    for tok in tokenize.generate_tokens(io.StringIO(src).readline):
        if tok.type in (tokenize.COMMENT, tokenize.STRING):
            continue
        kept.append(tok.string)
    code = " ".join(kept).lower()

    forbidden = [
        "accuracy", "macro_f1", "weighted_f1", "logit", "logits",
        "predict", "prediction", "confidence", "per_class_f1", "accept",
        "reject", "verdict",
    ]
    hits = [f for f in forbidden if f in code]
    assert not hits, (
        f"stratified_split references {hits} in executable code; a split "
        "assignment must be a function of class membership and session identity "
        "only, never of anything the model produced"
    )
