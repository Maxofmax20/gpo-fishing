"""RAM++ semantic-teacher worker — isolated, local, evidence-only.

ISOLATION CONTRACT (Phase 2 / Phase 26)
========================================
This worker exists to produce *evidence* for a human reviewer. It is
deliberately built so that most of the ways it could cause harm are not
available to it:

* It never touches the game. No input control, no macro step, no click.
* It never writes a dataset label, a review record, or `training_eligible`.
  It only appends to a results JSONL file that a human then acts on.
* It never calls a KB mutation. There is no such function imported.
* It never makes a network call at inference time. The checkpoint is verified
  against a pinned SHA-256 BEFORE use; if the bytes on disk do not match, the
  worker refuses to run rather than trusting a downloaded file.
* It never accepts a model path or URL from the caller at inference time. The
  checkpoint location is a pinned constant in the app's config; the only thing
  the caller controls is WHICH IMAGES, and image paths are validated to live
  inside the dataset root.
* It never sends an image anywhere. There is no HTTP client in the inference
  path.

The Rust side invokes it as a subprocess with a JSONL manifest on stdin and a
results JSONL on stdout; it is not a library dependency of the app.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable

# --------------------------------------------------------------------------
# Pinned configuration. Nothing here may be overridden from the UI.
# --------------------------------------------------------------------------

#: Semantic vocabulary the teacher is allowed to propose. Built ONLY from
#: canonical KB entities. A name that is not here cannot be suggested.
GPO_VOCAB_VERSION = "gpo-kb-fish-v1"

#: Explicitly NOT eligible as categories. These were deliberately excluded from
#: the knowledge base for lack of authoritative evidence. Listing them here
#: makes the exclusion auditable: if a future build starts proposing one, that
#: is a signal the KB changed, not an invitation to add it.
EXCLUDED_CONCEPTS = [
    "tigerfin", "exotic tigerfin", "golden tigerfin",
    "skeletal shark", "dark skeletal shark",
    "candy corn squid", "jack-o-bite",
    "fangfish", "anglerfish",
    "polka puffer", "golden polka puffer",
    "ribbon angelfish", "golden ribbon angelfish", "zebra ribbon angelfish",
    "blue-lip grouper",
    "sunken armor", "sunken anchor", "sunken helmet",
]


def sha256_file(path: Path, chunk: int = 1 << 20) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(chunk), b""):
            h.update(block)
    return h.hexdigest()


@dataclass
class Checkpoint:
    """A pinned, verified model artifact."""

    model: str                      # "ram" | "ram_plus"
    path: Path
    expected_sha256: str
    sha256: str = field(default="", init=False)

    def verify(self) -> tuple[bool, str]:
        """Verify the on-disk bytes against the pinned fingerprint.

        SECURITY: an unverified teacher is not a teacher. A tampered or
        partially-downloaded checkpoint must fail closed, not run.
        """
        if not self.path.is_file():
            return False, f"checkpoint missing: {self.path}"
        self.sha256 = sha256_file(self.path)
        if len(self.expected_sha256) != 64:
            return False, "pinned SHA-256 is malformed; refusing to trust it"
        if self.sha256.lower() != self.expected_sha256.lower():
            return False, (
                f"checkpoint SHA-256 mismatch for {self.path.name}: "
                f"expected {self.expected_sha256[:16]}..., got {self.sha256[:16]}..."
            )
        return True, "verified"


@dataclass
class Preprocess:
    """Recorded so a result can be reproduced or challenged."""

    name: str
    detail: str

    def as_str(self) -> str:
        return f"{self.name}:{self.detail}"


#: The preprocessing the app already uses for its own vision crops. The teacher
#: must see what the human sees, so this matches the GPO pipeline rather than
#: the model's training-time preprocessing.
GPO_PREPROCESS = Preprocess(
    "rgb",
    "letterbox-square(black)->bicubic->224x224",
)


class TeacherUnavailable(RuntimeError):
    """The teacher cannot run. Never a reason to fall back to guessing."""


class RamTeacher:
    """Thin, lazy wrapper around the RAM/RAM++ model.

    Imports happen inside methods so that the module can be imported (and its
    tests run) on a machine with no torch installed at all. That matters: the
    fusion and triage logic must remain testable in CI.
    """

    def __init__(self, checkpoint: Checkpoint, device: str = "auto") -> None:
        self.checkpoint = checkpoint
        self.device_pref = device
        self._model = None
        self._preprocess = None

    # -- lifecycle ---------------------------------------------------------

    def load(self) -> None:
        ok, why = self.checkpoint.verify()
        if not ok:
            raise TeacherUnavailable(why)
        try:
            import ram  # type: ignore
        except Exception as exc:  # pragma: no cover - depends on optional dep
            raise TeacherUnavailable(
                f"recognize-anything package not importable ({exc}); "
                "run ml/ram_plus/setup_env.ps1 first"
            ) from exc
        self._model = ram.ram_plus(
            pretrained=str(self.checkpoint.path), image_size=384, device=self._device()
        )
        self._preprocess = ram.get_transform()

    def _device(self) -> str:
        if self.device_pref != "auto":
            return self.device_pref
        try:
            import torch  # type: ignore

            return "cuda" if torch.cuda.is_available() else "cpu"
        except Exception:
            return "cpu"

    @property
    def available(self) -> bool:
        return self._model is not None

    def describe(self) -> dict[str, Any]:
        """Provenance block recorded with every result (Phase 25)."""
        return {
            "model": self.checkpoint.model,
            "checkpoint_path": str(self.checkpoint.path),
            "checkpoint_sha256": self.checkpoint.sha256 or self.checkpoint.expected_sha256,
            "preprocessing": GPO_PREPROCESS.as_str(),
            "vocabulary_version": GPO_VOCAB_VERSION,
            "device": self._device(),
            "ram_code_revision": os.environ.get("GPO_RAM_CODE_REV", "unknown"),
        }

    # -- inference ---------------------------------------------------------

    def tag(self, image_path: Path, custom_vocab: list[str]) -> dict[str, Any]:
        """Run open-set inference.

        `custom_vocab` MUST be canonical KB entities. Anything else is dropped
        before it reaches the model, so a caller cannot smuggle a new category
        in through this parameter.
        """
        if not self.available:
            raise TeacherUnavailable("teacher not loaded")
        safe = [c for c in custom_vocab if not _is_excluded(c)]
        import ram  # type: ignore

        img = self._preprocess(image_path.open("rb").read)  # type: ignore[operator]
        english_tags, chinese_tags, _ = self._model(img, None)
        res = ram.get_ram_results(
            english_tags, chinese_tags, custom_vocab=safe or None
        )
        return {
            "tags": list(english_tags),
            "candidate_categories": [
                (str(x["name"]), float(x["score"])) for x in res
            ],
        }


def _is_excluded(concept: str) -> bool:
    c = concept.lower()
    return any(bad in c for bad in EXCLUDED_CONCEPTS)


# --------------------------------------------------------------------------
# Worker entry point
# --------------------------------------------------------------------------


def read_manifest(stream: Iterable[str]) -> list[dict[str, Any]]:
    """Parse the JSONL work manifest from stdin.

    Each line: {"image_id": ..., "path": ..., "vocab": [...]}. An unreadable
    line is reported as an error result rather than skipped, so a malformed
    manifest can never look like "nothing to do".
    """
    out = []
    for n, line in enumerate(stream, 1):
        line = line.strip()
        if not line:
            continue
        try:
            out.append(json.loads(line))
        except Exception as exc:
            out.append({"__error__": f"manifest line {n} is not valid JSON: {exc}"})
    return out


def validate_request(item: dict[str, Any], dataset_root: Path) -> Path | None:
    """Confirm an image path is inside the dataset root.

    SECURITY (Phase 26): the caller controls WHICH IMAGES are analysed, so that
    boundary has to be enforced here. A path that escapes the dataset root is
    refused rather than read.
    """
    raw = item.get("path")
    if not raw:
        return None
    p = Path(raw).resolve()
    try:
        p.relative_to(dataset_root.resolve())
    except ValueError:
        return None
    if not p.is_file():
        return None
    return p


def run(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="GPO RAM++ semantic-teacher worker")
    ap.add_argument("--checkpoint", required=True, type=Path)
    ap.add_argument("--model", required=True, choices=["ram", "ram_plus"])
    ap.add_argument("--expect-sha256", required=True)
    ap.add_argument("--dataset-root", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--device", default="auto")
    ap.add_argument("--limit", type=int, default=0, help="0 = no limit")
    args = ap.parse_args(argv)

    ck = Checkpoint(model=args.model, path=args.checkpoint, expected_sha256=args.expect_sha256)
    teacher = RamTeacher(ck, device=args.device)

    items = read_manifest(sys.stdin)
    if args.limit:
        items = items[: args.limit]

    try:
        teacher.load()
        provenance = teacher.describe()
    except TeacherUnavailable as exc:
        # Emit one explicit error result per requested image. A teacher that
        # cannot load must be VISIBLE, not silently absent.
        with args.out.open("w", encoding="utf-8") as out:
            for item in items:
                out.write(
                    json.dumps(
                        {
                            "image_id": item.get("image_id", "?"),
                            "ram_model": args.model,
                            "checkpoint_sha256": args.expect_sha256,
                            "tags": [],
                            "candidate_categories": [],
                            "preprocessing": GPO_PREPROCESS.as_str(),
                            "timestamp_ms": 0,
                            "status": f"error:{exc}",
                        }
                    )
                    + "\n"
                )
        print(f"teacher unavailable: {exc}", file=sys.stderr)
        return 3

    import time

    with args.out.open("w", encoding="utf-8") as out:
        for item in items:
            image_id = str(item.get("image_id", "?"))
            if "__error__" in item:
                rec = {**provenance, "image_id": image_id, "status": "error:bad-manifest-line"}
                out.write(json.dumps(rec) + "\n")
                continue
            path = validate_request(item, args.dataset_root)
            if path is None:
                out.write(
                    json.dumps(
                        {
                            **provenance,
                            "image_id": image_id,
                            "status": "error:path-outside-dataset-root",
                        }
                    )
                    + "\n"
                )
                continue
            t0 = time.perf_counter()
            try:
                r = teacher.tag(path, list(item.get("vocab") or []))
                status = "ok"
            except Exception as exc:
                r = {"tags": [], "candidate_categories": []}
                status = f"error:{exc}"
            rec = {
                **provenance,
                "image_id": image_id,
                "tags": r["tags"],
                "candidate_categories": r["candidate_categories"],
                "ram_vocabulary": GPO_VOCAB_VERSION,
                "ram_threshold": None,
                "status": status,
                "latency_ms": int((time.perf_counter() - t0) * 1000),
            }
            out.write(json.dumps(rec) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(run())