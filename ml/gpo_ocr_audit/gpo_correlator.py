"""Join every OCR finding back to the existing GPO dataset metadata.

READ-ONLY, and that is the entire contract of this module.

It loads ``labels.jsonl``, the dataset manifest, the review records, the latest
training snapshot and (optionally) the shipped ONNX model, and exposes them as
lookups for the audit. It has no method that writes a label, sets
``training_eligible``, edits the knowledge base, touches the model registry, or
reaches game input - it imports none of those modules at all, which is a stronger
guarantee than a promise in a docstring.

The split assignment is NOT reimplemented here. It imports
``gpo_train.dataset.fnv1a_session_split``, the existing byte-identical port of
``MlDatasetStore::split_of``. Re-deriving the rule would risk a second, subtly
different definition of TEST, and TEST membership is sacred.
"""

from __future__ import annotations

import json
import os
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

try:  # namespace-package import; `ml` has no __init__.py by design
    from gpo_train.dataset import fnv1a_session_split
except ImportError:  # pragma: no cover - only when ml/ is not on the path
    def fnv1a_session_split(session_id: str) -> str:
        """Fallback copy of FNV-1a64 70/15/15. Identical to the Rust rule.

        Duplicated only so the module is importable standalone; the repository
        path is preferred and the test suite asserts the two agree.
        """
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


#: Session ids look like ``session_YYYY-MM-DD_HHMMSS_hash``; the date segment is
#: the calendar day of the run.
SESSION_DATE_INDEX = 1


def session_day(session_id: str | None) -> str:
    """Calendar day of a session, or ``''`` when the id is not a session id."""
    if not session_id:
        return ""
    parts = session_id.split("_")
    if len(parts) > SESSION_DATE_INDEX:
        return parts[SESSION_DATE_INDEX]
    return ""


@dataclass
class DatasetRow:
    """One row of ``labels.jsonl``, plus the split it belongs to."""

    image_id: str
    session_id: str
    timestamp_ms: int
    task: str | None
    region_name: str | None
    game_state: str | None
    ui_label: str | None
    entity_id: str | None
    annotator: str | None
    ocr_text: str | None
    confidence: float | None
    hard_example: bool
    bbox: Any
    event_id: str | None
    frame_index: int | None
    corrections: list
    split: str

    @property
    def day(self) -> str:
        return session_day(self.session_id)


@dataclass
class ReviewRecord:
    image_id: str
    review_status: str
    human_entity_id: str | None
    reviewer_version: str | None
    training_eligible: bool
    excluded_reason: str | None
    correction_reason: str | None
    ocr_text: str | None
    reviewed_at: int | None


class DatasetIndex:
    """Read-only view over the live dataset and its review history."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.images_dir = root / "images"
        self.manifest: dict = {}
        self.rows: dict[str, DatasetRow] = {}
        self.reviews: dict[str, ReviewRecord] = {}
        self.snapshot_ids: set[str] = set()
        self.split_overrides: dict[str, str] = {}
        self.warnings: list[str] = []
        self._load()

    # -- loading ---------------------------------------------------------

    def _load(self) -> None:
        man = self.root / "manifest.json"
        if man.is_file():
            try:
                self.manifest = json.loads(man.read_text(encoding="utf-8"))
                self.split_overrides = dict(self.manifest.get("split_overrides") or {})
            except json.JSONDecodeError as exc:
                self.warnings.append(f"manifest.json unreadable: {exc}")

        labels = self.root / "labels.jsonl"
        bad = 0
        with labels.open(encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    r = json.loads(line)
                except json.JSONDecodeError:
                    bad += 1
                    continue
                sid = r.get("session_id") or ""
                self.rows[r["image_id"]] = DatasetRow(
                    image_id=r["image_id"],
                    session_id=sid,
                    timestamp_ms=int(r.get("timestamp_ms") or 0),
                    task=r.get("task"),
                    region_name=r.get("region_name"),
                    game_state=r.get("game_state"),
                    ui_label=r.get("ui_label"),
                    entity_id=r.get("entity_id"),
                    annotator=r.get("annotator"),
                    ocr_text=r.get("ocr_text"),
                    confidence=r.get("confidence"),
                    hard_example=bool(r.get("hard_example")),
                    bbox=r.get("bbox"),
                    event_id=r.get("event_id"),
                    frame_index=r.get("frame_index"),
                    corrections=r.get("corrections") or [],
                    split=self.split_of(sid),
                )
        if bad:
            self.warnings.append(f"{bad} unparseable line(s) in labels.jsonl")

    def split_of(self, session_id: str) -> str:
        """Manifest override first, then the session hash. Never re-derived."""
        if session_id in self.split_overrides:
            return self.split_overrides[session_id]
        return fnv1a_session_split(session_id)

    def attach_reviews(self, reviews_path: Path) -> None:
        if not reviews_path.is_file():
            self.warnings.append(f"no reviews file at {reviews_path.name}")
            return
        with reviews_path.open(encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    r = json.loads(line)
                except json.JSONDecodeError:
                    continue
                self.reviews[r["image_id"]] = ReviewRecord(
                    image_id=r["image_id"],
                    review_status=r.get("review_status") or "",
                    human_entity_id=r.get("human_entity_id"),
                    reviewer_version=r.get("reviewer_version"),
                    training_eligible=bool(r.get("training_eligible")),
                    excluded_reason=r.get("excluded_reason"),
                    correction_reason=r.get("correction_reason"),
                    ocr_text=r.get("ocr_text"),
                    reviewed_at=r.get("reviewed_at"),
                )

    def attach_snapshot(self, snapshot_path: Path) -> None:
        """Record which image ids the newest training snapshot already learned.

        Used only for reporting. A row being in the snapshot is a fact about the
        past, not a licence: it is never used to skip analysis or to treat a row
        as correct.
        """
        if not snapshot_path.is_file():
            return
        try:
            snap = json.loads(snapshot_path.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            return
        for key in ("images", "image_ids", "rows"):
            vals = snap.get(key)
            if isinstance(vals, list):
                for v in vals:
                    self.snapshot_ids.add(v if isinstance(v, str) else v.get("image_id"))
                return

    # -- lookups ---------------------------------------------------------

    def image_path(self, image_id: str) -> Path:
        return self.images_dir / f"{image_id}.png"

    def get(self, image_id: str) -> DatasetRow | None:
        return self.rows.get(image_id)

    def stats(self) -> dict[str, Any]:
        splits: dict[str, int] = {}
        sessions: dict[str, set[str]] = {}
        days: set[str] = set()
        for r in self.rows.values():
            splits[r.split] = splits.get(r.split, 0) + 1
            sessions.setdefault(r.session_id, set()).add(r.split)
            if r.day:
                days.add(r.day)
        leaking = sorted(s for s, v in sessions.items() if len(v) > 1)
        return {
            "root_name": self.root.name,
            "dataset_version": self.manifest.get("version"),
            "rows": len(self.rows),
            "sessions": len(sessions),
            "calendar_days": len(days),
            "day_range": [min(days), max(days)] if days else [],
            "splits": dict(sorted(splits.items())),
            "sessions_spanning_multiple_splits": leaking,
            "human_reviews": len(self.reviews),
            "eligible_reviews": sum(
                1 for r in self.reviews.values() if r.training_eligible
            ),
            "in_latest_snapshot": len(self.snapshot_ids),
            "warnings": list(self.warnings),
        }


# --------------------------------------------------------------------------
# Model cross-check
# --------------------------------------------------------------------------

@dataclass
class ModelRunner:
    """Read-only inference with the SHIPPED fish model.

    A fourth evidence source for the cross-check, nothing more. The
    preprocessing is taken verbatim from the model's own manifest, so the audit
    cannot accidentally evaluate it under different conditions than it was
    trained for - a mismatch there would look like model error when it is really
    pipeline error.

    The model is never retrained, replaced, or promoted from here.
    """

    manifest: dict
    classes: list[str]
    _session: Any = None
    _mean: Any = None
    _std: Any = None
    load_error: str = ""
    #: Prediction failures are COUNTED, not silently swallowed. A run in which
    #: every prediction failed must not be indistinguishable from a run in which
    #: the model had nothing to say - that is precisely the silent-degradation
    #: failure this project has been guarding against everywhere else.
    prediction_failures: int = 0
    last_prediction_error: str = ""

    @classmethod
    def load(cls, models_dir: Path, name: str = "fish_v1") -> "ModelRunner | None":
        man_path = models_dir / f"{name}.json"
        onnx_path = models_dir / f"{name}.onnx"
        if not man_path.is_file():
            return None
        try:
            manifest = json.loads(man_path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as exc:
            raise RuntimeError(f"{name}.json unreadable: {exc}") from exc
        if not onnx_path.is_file():
            return None
        self = cls(manifest=manifest, classes=list(manifest.get("classes") or []))
        try:
            import numpy as np
            import onnxruntime as ort

            self._np = np
            self._session = ort.InferenceSession(
                str(onnx_path), providers=["CPUExecutionProvider"]
            )
            pre = manifest.get("preprocess") or {}
            size = int(pre.get("size") or manifest.get("input_width") or 96)
            self._size = size
            # (1, 3, 1, 1) to match the NCHW batch. Shaping these (1,1,1,3) is the
            # NHWC convention and raises a broadcast error the moment the layout is
            # corrected - which the failure counter reported rather than hiding.
            self._mean = np.asarray(pre.get("mean") or [0.0, 0.0, 0.0],
                                    dtype=np.float32).reshape(1, 3, 1, 1)
            self._std = np.asarray(pre.get("std") or [1.0, 1.0, 1.0],
                                   dtype=np.float32).reshape(1, 3, 1, 1)
        except Exception as exc:  # noqa: BLE001
            self.load_error = f"{type(exc).__name__}: {exc}"
        return self

    @property
    def available(self) -> bool:
        return self._session is not None

    def _preprocess(self, path: Path):
        """square-black pad -> bilinear resize -> normalise -> NCHW.

        The layout is NCHW because that is what the exported graph declares
        (`pixels: [batch, 3, 96, 96]`). Feeding NHWC raised an InvalidArgument
        that an earlier bare `except` swallowed, so the audit reported "0 rows
        carry a model prediction" instead of saying why.
        """
        from PIL import Image

        np = self._np
        size = self._size
        with Image.open(path) as im:
            im = im.convert("RGB")
            w, h = im.size
            side = max(w, h)
            canvas = Image.new("RGB", (side, side), (0, 0, 0))
            canvas.paste(im, ((side - w) // 2, (side - h) // 2))
            canvas = canvas.resize((size, size), Image.BILINEAR)
            arr = np.asarray(canvas, dtype=np.float32) / 255.0
        arr = arr[None, ...]                       # (1, H, W, 3)
        arr = np.transpose(arr, (0, 3, 1, 2))      # (1, 3, H, W)
        return (arr - self._mean) / self._std

    def predict(self, path: Path) -> tuple[str | None, float | None]:
        """Return ``(top_class, confidence)``; ``(None, None)`` on any failure.

        A failure is recorded in :attr:`prediction_failures` and
        :attr:`last_prediction_error` so it shows up in the provenance rather
        than vanishing.
        """
        if not self.available:
            return None, None
        try:
            batch = self._preprocess(path)
            out = self._session.run(None, {self._session.get_inputs()[0].name: batch})[0]
            logits = out[0].astype("float64")
            temp = float(self.manifest.get("temperature") or 1.0)
            if temp > 0:
                logits = logits / temp
            logits -= logits.max()
            probs = self._np.exp(logits)
            probs /= probs.sum()
            idx = int(probs.argmax())
            if idx >= len(self.classes):
                self.prediction_failures += 1
                self.last_prediction_error = (
                    f"argmax index {idx} outside the {len(self.classes)} declared classes"
                )
                return None, None
            return self.classes[idx], float(probs[idx])
        except Exception as exc:  # noqa: BLE001
            # Recorded, not guessed and not hidden.
            self.prediction_failures += 1
            self.last_prediction_error = f"{type(exc).__name__}: {exc}"
            return None, None

    def provenance(self) -> dict[str, Any]:
        return {
            "name": self.manifest.get("name"),
            "version": self.manifest.get("version"),
            "sha256": self.manifest.get("sha256"),
            "classes": self.classes,
            "vocab_sha": self.manifest.get("vocab_sha"),
            "temperature": self.manifest.get("temperature"),
            "preprocess": self.manifest.get("preprocess"),
            "available": self.available,
            "load_error": self.load_error,
            # Never let a fully-failing model column pass as "no opinion".
            "prediction_failures": self.prediction_failures,
            "last_prediction_error": self.last_prediction_error,
            "manifest_self_reported": {
                "test_accuracy": self.manifest.get("test_accuracy"),
                "macro_f1": self.manifest.get("macro_f1"),
                "ece": self.manifest.get("ece"),
                "test_n": self.manifest.get("test_n"),
            },
        }


def find_latest_snapshot(repo_root: Path) -> Path | None:
    """Newest ``dataset_snapshot.json`` under ``ml/output``, by mtime then name.

    Only used for reporting which rows a past snapshot already contained.
    """
    out = repo_root / "ml" / "output"
    if not out.is_dir():
        return None
    cands = sorted(out.glob("*/dataset_snapshot.json"))
    if not cands:
        return None
    return max(cands, key=lambda p: (p.stat().st_mtime, p.name))


def default_dataset_root(appdata: str | None = None) -> Path:
    """The live dataset root, discovered from the app data directory."""
    appdata = appdata or os.environ.get("APPDATA", "")
    if not appdata:
        raise RuntimeError("APPDATA not set; pass --dataset-root explicitly")
    return Path(appdata) / "gpo-autofish" / "datasets" / "gpo-vision" / "v1"