"""Reproducible loader for the real gpo-vision/v1 production dataset.

Reads (never writes) %APPDATA%/gpo-autofish/datasets/gpo-vision/v1/:
labels.jsonl + manifest.json + images/. Replicates the project's split
rule EXACTLY (FNV-1a session hash 70/15/15 + manifest split_overrides) and
the training-eligibility semantics from core/ml_dataset.rs:

- RESULT rows train the STATE head always (catch_result is certain by
  collection event), and the ENTITY head only when entity-linked.
- WAITING/BITE rows train the STATE head when stable (no different-state
  row from the same session within +-TRANSITION_WINDOW_MS).
- Transition-adjacent WAITING/BITE rows are EXCLUDED from train/val and
  reported as a separate transition-eval set (never silent IID labels).
- Rows with missing images are excluded everywhere.

No test information may enter training: splits are session-pure and the
TEST split is only read by the final evaluation.
"""

from __future__ import annotations

import json
import os
from dataclasses import dataclass, field

STATE_LABELS = ["waiting_for_bite", "bite", "catch_result"]
TRANSITION_WINDOW_MS = 2000
INPUT_SIZE = 96


def fnv1a_session_split(session_id: str) -> str:
    """Byte-identical port of MlDatasetStore::split_of (FNV-1a64)."""
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


@dataclass
class Row:
    image_id: str
    session_id: str
    timestamp_ms: int
    game_state: str | None
    entity_id: str | None
    hard_example: bool
    image_path: str


@dataclass
class DatasetSnapshot:
    version: int
    rows: list = field(default_factory=list)
    overrides: dict = field(default_factory=dict)

    def split_of(self, session_id: str) -> str:
        if session_id in self.overrides:
            return self.overrides[session_id]
        return fnv1a_session_split(session_id)


def dataset_root() -> str:
    appdata = os.environ.get("APPDATA")
    if not appdata:
        raise RuntimeError("APPDATA is not set; production dataset location unknown")
    return os.path.join(appdata, "gpo-autofish", "datasets", "gpo-vision", "v1")


def load_snapshot(root: str | None = None) -> DatasetSnapshot:
    root = root or dataset_root()
    with open(os.path.join(root, "manifest.json"), encoding="utf-8") as f:
        manifest = json.load(f)
    version = int(manifest.get("version", 0))
    overrides = dict(manifest.get("split_overrides", {}) or {})
    images_dir = os.path.join(root, "images")
    rows: list[Row] = []
    with open(os.path.join(root, "labels.jsonl"), encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            a = json.loads(line)
            image_id = a.get("image_id", "")
            rows.append(
                Row(
                    image_id=image_id,
                    session_id=a.get("session_id", ""),
                    timestamp_ms=int(a.get("timestamp_ms", 0) or 0),
                    game_state=a.get("game_state"),
                    entity_id=a.get("entity_id"),
                    hard_example=bool(a.get("hard_example", False)),
                    image_path=os.path.join(images_dir, f"{image_id}.png"),
                )
            )
    return DatasetSnapshot(version=version, rows=rows, overrides=overrides)


def compute_transition_ids(rows: list[Row]) -> set[int]:
    """Indices of WAITING/BITE rows with a different-state same-session
    neighbor within +-TRANSITION_WINDOW_MS (mirrors state_eligibility)."""
    by_sess: dict[str, list[int]] = {}
    for i, r in enumerate(rows):
        if r.game_state in ("waiting_for_bite", "bite"):
            by_sess.setdefault(r.session_id, []).append(i)
    out: set[int] = set()
    for idxs in by_sess.values():
        idxs.sort(key=lambda i: rows[i].timestamp_ms)
        for pos, i in enumerate(idxs):
            t = rows[i].timestamp_ms
            for j in (idxs[pos - 1] if pos > 0 else None, idxs[pos + 1] if pos + 1 < len(idxs) else None):
                if j is None:
                    continue
                other = rows[j].game_state
                if other is not None and other != rows[i].game_state:
                    if abs(rows[j].timestamp_ms - t) <= TRANSITION_WINDOW_MS:
                        out.add(i)
                        break
    return out


@dataclass
class SplitSet:
    train: list = field(default_factory=list)
    validation: list = field(default_factory=list)
    test: list = field(default_factory=list)
    # Transition rows stay attributed to their own split: the test portion
    # is reported only at the end, never used during development/selection.
    transition_train: list = field(default_factory=list)
    transition_validation: list = field(default_factory=list)
    transition_test: list = field(default_factory=list)
    excluded: list = field(default_factory=list)


def build_splits(snap: DatasetSnapshot) -> SplitSet:
    """Session-pure splits + eligibility buckets. Raises on any session
    appearing in more than one split (must never happen by construction)."""
    import os as _os

    transition = compute_transition_ids(snap.rows)
    out = SplitSet()
    seen_sessions: dict[str, str] = {}
    for i, r in enumerate(snap.rows):
        if not r.session_id or not _os.path.isfile(r.image_path):
            out.excluded.append(i)
            continue
        split = snap.split_of(r.session_id)
        prev = seen_sessions.setdefault(r.session_id, split)
        if prev != split:
            raise RuntimeError(f"session {r.session_id} spans splits")
        bucket = split if split in ("train", "validation", "test") else "train"
        if r.game_state == "catch_result":
            getattr(out, bucket).append(i)
        elif r.game_state in ("waiting_for_bite", "bite"):
            if i in transition:
                getattr(out, f"transition_{bucket}").append(i)
            else:
                getattr(out, bucket).append(i)
        else:
            out.excluded.append(i)
    # Session-overlap guard across the three sets.
    def sess_of(idxs: list[int]) -> set[str]:
        return {snap.rows[i].session_id for i in idxs}

    s_tr, s_va, s_te = sess_of(out.train), sess_of(out.validation), sess_of(out.test)
    assert not (s_tr & s_va) and not (s_tr & s_te) and not (s_va & s_te), "session overlap across splits"
    return out
