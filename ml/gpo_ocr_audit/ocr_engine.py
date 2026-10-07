"""Independent OCR over every dataset image, with mandatory image hashing.

DESIGN NOTES
============

**Why a fresh OCR engine at all?** The dataset already carries an ``ocr_text``
field written by the collector. The audit needs to compare against it, but it must
not *rely* on it: those strings were produced by the app's own recogniser, and
re-reading them tells us nothing about what is actually rendered on the pixels.
This module therefore re-reads every image itself and records the two readings
side by side. Their disagreement is evidence.

**CPU-only, no CUDA.** RapidOCR runs ONNX on CPU. No GPU is required or used, so
the audit is reproducible on any machine.

**Image hash is mandatory.** Every record carries the SHA-256 of the exact bytes
analysed. That is what makes a text finding traceable to an image rather than to
a filename that could change.

**Caching is keyed on content and configuration.** The cache key is
``image_sha256 + engine fingerprint``. Changing the engine, its version, or any
preprocessing option changes the fingerprint and therefore invalidates every
cached entry, so a stale result can never be silently reused. Cache *hits* are
byte-identical to a fresh run, so caching cannot alter the audit's output.
"""

from __future__ import annotations

import hashlib
import json
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

ENGINE_NAME = "rapidocr-onnxruntime"

#: Minimum per-box confidence for a text detection to be treated as real.
#:
#: Tuned against the actual engine output on these panels, not guessed. The bar
#: frames produce confident nonsense ("X1"), so the floor filters junk while
#: keeping every genuine drop-banner reading, which the engine scores 0.80+.
#: Anything below is recorded as a rejected detection, not silently dropped, so
#: the OCR-quality report can show what was suppressed.
MIN_BOX_CONFIDENCE = 0.30

#: Boxes scoring below this are rejected as noise.
STRICT_CONFIDENCE = 0.50


def sha256_file(path: Path, chunk: int = 1 << 20) -> str:
    """SHA-256 of a file's bytes.

    Streaming so a 14k-image dataset never has to fit in memory.
    """
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for block in iter(lambda: fh.read(chunk), b""):
            h.update(block)
    return h.hexdigest()


def engine_version() -> str:
    """Installed engine version, read from the package.

    Falls back to ``unknown`` rather than raising: an unpinned engine must be
    visible in the provenance as unpinned, not silently treated as pinned.
    """
    try:
        from importlib.metadata import version
        return version(ENGINE_NAME)
    except Exception:
        return "unknown"


def engine_fingerprint(preproc: dict | None = None) -> str:
    """Identity of the OCR configuration, hashed.

    Changing the engine version or any preprocessing option changes this string,
    which invalidates the whole cache. That is the mechanism that prevents a
    result from one configuration being reused under another.
    """
    payload = {
        "engine": ENGINE_NAME,
        "version": engine_version(),
        "preprocessing": preproc or {},
        "min_confidence": MIN_BOX_CONFIDENCE,
        # Thread count is part of the identity. ONNX Runtime may reduce in a
        # different order with a different thread count, which can perturb a
        # confidence by a hair. Pinning it here means such a change invalidates
        # the cache instead of silently mixing two numeric regimes.
        "intra_op_threads": INTRA_OP_THREADS,
        # The engine's default detection resolution. Measured: dropping to 384 is
        # 2.5x faster but degrades the text this audit exists to report faithfully
        # ("New" -> "Ne", lost word spacing), so the default is kept and the
        # slowness is accepted rather than hidden.
        "det_limit_side_len": 736,
    }
    blob = json.dumps(payload, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(blob.encode("utf-8")).hexdigest()


@dataclass
class OcrReading:
    """One image's fresh OCR reading.

    ``text`` joins every surviving detection box. ``all_boxes`` keeps the
    rejected ones too, so OCR quality is auditable rather than asserted.
    """

    image_sha256: str
    text: str
    confidence: float | None
    boxes: list[dict]
    rejected: list[dict]
    elapsed_ms: int
    engine: str = ENGINE_NAME
    engine_version: str = ""
    error: str | None = None

    @property
    def ok(self) -> bool:
        return self.error is None and bool(self.text)


class OcrCache:
    """Append-only JSONL cache keyed by ``(image_sha256, fingerprint)``.

    An append-only log keeps the audit resumable after an interruption and makes
    the cache itself auditable: every entry records which configuration produced
    it. A corrupt trailing line (an interrupted run) is ignored rather than
    aborting the audit.
    """

    def __init__(self, path: Path) -> None:
        self.path = path
        self._mem: dict[str, dict] = {}
        self.hits = 0
        self.misses = 0
        if path.is_file():
            with path.open(encoding="utf-8") as fh:
                for line in fh:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        rec = json.loads(line)
                    except json.JSONDecodeError:
                        continue  # torn final line from an interrupted run
                    key = rec.get("key")
                    if key:
                        self._mem[key] = rec

    @staticmethod
    def key(image_sha256: str, fingerprint: str) -> str:
        return f"{image_sha256}:{fingerprint}"

    def get(self, image_sha256: str, fingerprint: str) -> OcrReading | None:
        rec = self._mem.get(self.key(image_sha256, fingerprint))
        if rec is None:
            self.misses += 1
            return None
        self.hits += 1
        return _reading_from_record(rec)

    def put(self, reading: OcrReading, fingerprint: str) -> None:
        rec = {
            "key": self.key(reading.image_sha256, fingerprint),
            "image_sha256": reading.image_sha256,
            "fingerprint": fingerprint,
            "engine": reading.engine,
            "engine_version": reading.engine_version,
            "text": reading.text,
            "confidence": reading.confidence,
            "boxes": reading.boxes,
            "rejected": reading.rejected,
            "elapsed_ms": reading.elapsed_ms,
            "error": reading.error,
        }
        self._mem[rec["key"]] = rec

    def flush(self) -> None:
        """Write every entry in sorted key order.

        Sorted so the cache file itself is deterministic, which keeps the audit's
        output reproducible even though the work was done in parallel.
        """
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with self.path.open("w", encoding="utf-8", newline="\n") as fh:
            for key in sorted(self._mem):
                fh.write(json.dumps(self._mem[key], sort_keys=True) + "\n")


def _reading_from_record(rec: dict) -> OcrReading:
    return OcrReading(
        image_sha256=rec["image_sha256"],
        text=rec.get("text", ""),
        confidence=rec.get("confidence"),
        boxes=rec.get("boxes") or [],
        rejected=rec.get("rejected") or [],
        elapsed_ms=int(rec.get("elapsed_ms") or 0),
        engine=rec.get("engine", ENGINE_NAME),
        engine_version=rec.get("engine_version", ""),
        error=rec.get("error"),
    )


def _coerce_conf(value) -> float | None:
    """RapidOCR 1.2.x returns confidence as a STRING on some builds.

    Returning it raw makes every comparison a string comparison, which silently
    ranks ``'0.9'`` below ``'0.85'``. Coerce once, here.
    """
    if value is None:
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


#: ONNX Runtime intra-op threads per worker process.
#:
#: ONNX Runtime defaults each session to the machine's core count. With a worker
#: pool that means `workers x cores` runnable threads on `cores` cores, and the
#: measured result was worse than no pool at all: 8 workers sustained 58 img/min
#: while a single process sustained 71 img/min. Pinning each session to one
#: thread and running sessions sequentially restores real parallelism, because
#: parallelism now comes from the pool instead of from thrashing.
#:
#: 1 thread is also what makes the audit's timing numbers reproducible.
INTRA_OP_THREADS = 1


def pin_onnx_threads(n: int = INTRA_OP_THREADS) -> bool:
    """Force every ONNX session to use ``n`` intra-op threads.

    Must be called BEFORE ``RapidOCR`` is constructed. RapidOCR binds
    ``SessionOptions`` into its own module namespace at import time, so patching
    ``onnxruntime.SessionOptions`` alone would not take effect - the reference is
    rebound on the RapidOCR utility module.

    Returns True if the patch was applied (or was already in place).
    """
    try:
        import onnxruntime as ort
        import rapidocr_onnxruntime.utils as ru
    except Exception:
        return False

    current = getattr(ru, "SessionOptions", None)
    if current is not None and getattr(current, "_gpo_thread_pinned", False):
        return True

    class PinnedSessionOptions(current):  # type: ignore[misc,valid-type]
        """SessionOptions with a bounded thread pool."""

        _gpo_thread_pinned = True

        def __init__(self, *args, **kwargs):
            super().__init__(*args, **kwargs)
            self.intra_op_num_threads = n
            self.inter_op_num_threads = 1
            # Sequential execution makes the thread count actually meaningful;
            # parallel execution spawns its own pool that ignores the cap.
            try:
                self.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
            except Exception:
                pass

    ru.SessionOptions = PinnedSessionOptions
    return True


class OcrEngine:
    """A single RapidOCR instance.

    One instance per process. RapidOCR holds ONNX sessions that are not cheap to
    build, so the worker pool constructs this once and reuses it.
    """

    def __init__(self) -> None:
        self._ocr = None

    def _ensure(self):
        if self._ocr is None:
            pin_onnx_threads()
            from rapidocr_onnxruntime import RapidOCR
            self._ocr = RapidOCR()
        return self._ocr

    def read(self, path: Path) -> OcrReading:
        """Read one image. Never raises; a failure becomes a recorded error.

        An unreadable image must be VISIBLE. Silently producing an empty reading
        would be indistinguishable from "the image contains no text", and that
        confusion is how bad data looks clean.
        """
        digest = sha256_file(path)
        if not self.available():
            return OcrReading(digest, "", None, [], [], 0,
                              engine_version=engine_version(),
                              error="engine-unavailable")
        try:
            import time
            # `_ensure()` is what actually constructs the ONNX sessions. Skipping
            # it here left `self._ocr` as None and every image recorded a
            # TypeError instead of being read.
            engine = self._ensure()
            t0 = time.perf_counter()
            out = engine(str(path))
            elapsed = int((time.perf_counter() - t0) * 1000)
        except Exception as exc:  # noqa: BLE001 - any engine failure must be recorded
            return OcrReading(digest, "", None, [], [], 0,
                              engine_version=engine_version(),
                              error=f"{type(exc).__name__}: {exc}")

        boxes: list[dict] = []
        rejected: list[dict] = []
        # RapidOCR returns `(detections, elapsed)`. Iterating that tuple directly
        # walks the per-stage timings as if they were detections, which yields
        # nonsense geometry and float/str type errors instead of text.
        result = out
        if isinstance(result, tuple):
            result = result[0]
        for item in result or []:
            try:
                quad, text, conf_raw = item[0], item[1], item[2]
            except (IndexError, TypeError, KeyError):
                continue
            if not isinstance(quad, (list, tuple)) or not quad:
                continue
            conf = _coerce_conf(conf_raw)
            try:
                xs = [float(p[0]) for p in quad]
                ys = [float(p[1]) for p in quad]
            except (TypeError, IndexError, ValueError):
                # A malformed geometry contributes nothing, but must not abort
                # the whole image.
                continue
            if conf is not None and conf < MIN_BOX_CONFIDENCE:
                rejected.append({"text": str(text), "confidence": conf})
                continue
            boxes.append({
                "text": str(text),
                "confidence": conf,
                "x0": min(xs), "y0": min(ys),
                "x1": max(xs), "y1": max(ys),
            })

        # Left-to-right, then top-to-bottom: the banner is a single line and
        # ordering must not depend on detection order.
        boxes.sort(key=lambda b: (round(b["y0"] / 12.0), b["x0"]))
        text = " ".join(b["text"] for b in boxes).strip()
        conf = min((b["confidence"] for b in boxes if b["confidence"] is not None),
                   default=None)
        return OcrReading(digest, text, conf, boxes, rejected, elapsed,
                          engine_version=engine_version())

    @staticmethod
    def available() -> bool:
        try:
            import rapidocr_onnxruntime  # noqa: F401
            return True
        except Exception:
            return False


# --------------------------------------------------------------------------
# Worker-pool plumbing
# --------------------------------------------------------------------------

_ENGINE: OcrEngine | None = None


def _init_worker() -> None:
    """Build one engine per worker process."""
    global _ENGINE
    _ENGINE = OcrEngine()


def _run_one(job: tuple[str, str]) -> dict:
    """Hash + read one image. Runs inside a pool worker."""
    image_id, abs_path = job
    assert _ENGINE is not None, "worker pool was not initialised"
    path = Path(abs_path)
    try:
        reading = _ENGINE.read(path)
    except Exception as exc:  # noqa: BLE001
        # Belt and braces: even an unexpected error becomes a recorded result
        # rather than killing the pool and losing the whole run.
        reading = OcrReading("", "", None, [], [], 0, error=f"{type(exc).__name__}: {exc}")
    return {
        "image_id": image_id,
        "image_sha256": reading.image_sha256,
        "text": reading.text,
        "confidence": reading.confidence,
        "boxes": reading.boxes,
        "rejected": reading.rejected,
        "elapsed_ms": reading.elapsed_ms,
        "error": reading.error,
        "engine_version": reading.engine_version,
    }


def default_workers() -> int:
    """Leave a couple of cores free; OCR is CPU-bound.

    Also capped so a 4-core laptop does not spawn 12 ONNX sessions and thrash.
    """
    try:
        cpus = os.cpu_count() or 2
    except Exception:
        cpus = 2
    return max(1, min(8, cpus - 2))


def read_all(
    jobs: Iterable[tuple[str, str]],
    cache: OcrCache,
    fingerprint: str,
    workers: int | None = None,
    progress=None,
) -> dict[str, dict]:
    """Read every image, using the cache where the content matches.

    Cache first: hashing still happens for every image (the hash is what proves
    which bytes were read) but an unchanged image skips the expensive inference.
    """
    from concurrent.futures import ProcessPoolExecutor

    jobs = list(jobs)
    out: dict[str, dict] = {}
    todo: list[tuple[str, str]] = []

    for image_id, abs_path in jobs:
        p = Path(abs_path)
        if not p.is_file():
            out[image_id] = {
                "image_id": image_id, "image_sha256": "", "text": "",
                "confidence": None, "boxes": [], "rejected": [], "elapsed_ms": 0,
                "error": "image-missing", "engine_version": engine_version(),
            }
            continue
        digest = sha256_file(p)
        hit = cache.get(digest, fingerprint)
        if hit is not None:
            rec = {
                "image_id": image_id, "image_sha256": digest, "text": hit.text,
                "confidence": hit.confidence, "boxes": hit.boxes,
                "rejected": hit.rejected, "elapsed_ms": hit.elapsed_ms,
                "error": hit.error, "engine_version": hit.engine_version,
            }
            out[image_id] = rec
        else:
            todo.append((image_id, abs_path))

    if not todo:
        if progress:
            progress(len(jobs), len(jobs), 0)
        return out

    n_workers = workers or default_workers()
    done = 0
    # Flush periodically. A 14k-image audit takes tens of minutes, and flushing
    # only at the end would throw away every result if the process were
    # interrupted - making the next run start from zero. The cache file is
    # rewritten in sorted key order, so a flush is idempotent and does not affect
    # the audit's output.
    flush_every = 500
    if n_workers <= 1:
        _init_worker()
        for job in todo:
            rec = _run_one(job)
            out[rec["image_id"]] = rec
            _remember(cache, rec, fingerprint)
            done += 1
            if progress and done % 50 == 0:
                progress(len(jobs), done, 0)
            if done % flush_every == 0:
                cache.flush()
    else:
        with ProcessPoolExecutor(
            max_workers=n_workers, initializer=_init_worker
        ) as pool:
            for rec in pool.map(_run_one, todo, chunksize=8):
                out[rec["image_id"]] = rec
                _remember(cache, rec, fingerprint)
                done += 1
                if progress and done % 50 == 0:
                    progress(len(jobs), done, 0)
                if done % flush_every == 0:
                    cache.flush()

    cache.flush()
    if progress:
        progress(len(jobs), done, 0)
    return out


def _remember(cache: OcrCache, rec: dict, fingerprint: str) -> None:
    if not rec.get("image_sha256"):
        return
    cache.put(
        OcrReading(
            image_sha256=rec["image_sha256"], text=rec["text"],
            confidence=rec["confidence"], boxes=rec["boxes"],
            rejected=rec["rejected"], elapsed_ms=rec["elapsed_ms"],
            engine_version=rec.get("engine_version", ""), error=rec.get("error"),
        ),
        fingerprint,
    )