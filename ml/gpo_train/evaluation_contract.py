"""Shared input integrity and frozen holdout identity, independent of torch."""
import hashlib
import json
from pathlib import Path


def holdout_fingerprint(snapshot, indices, classes):
    rows = []
    for i in indices:
        r = snapshot.rows[i]
        rows.append([r.image_id, r.session_id, r.game_state, r.entity_id,
                     hashlib.sha256(Path(r.image_path).read_bytes()).hexdigest()])
    payload = {"schema": 1, "classes": list(classes), "rows": sorted(rows)}
    return hashlib.sha256(json.dumps(payload, sort_keys=True).encode()).hexdigest()


def validate_reviewed_manifest(manifest, canonical_classes):
    vocab = manifest["class_map"]
    rows = manifest["rows"]
    if vocab != list(canonical_classes) or len(vocab) != manifest["class_map_width"]:
        raise ValueError("class map must match the canonical vocabulary in order")
    vocab_sha = hashlib.sha256("\n".join(vocab).encode()).hexdigest()
    digest = hashlib.sha256(json.dumps({"class_map": vocab, "rows": rows}, sort_keys=True).encode()).hexdigest()
    if manifest.get("vocab_sha256") != vocab_sha or manifest.get("dataset_sha256") != digest:
        raise ValueError("manifest fingerprint mismatch")
    sessions, ids = {}, set()
    support = {(c, split): 0 for c in vocab for split in ("train", "validation", "test")}
    for r in rows:
        session, split, image = r["session_id"], r["split"], r["image_id"]
        if not session or split not in ("train", "validation", "test") or r["entity_id"] not in vocab:
            raise ValueError("invalid session, split or entity")
        if image in ids or len(image) not in (16, 64) or any(c not in "0123456789abcdef" for c in image):
            raise ValueError("duplicate or invalid image id")
        ids.add(image)
        if sessions.setdefault(session, split) != split:
            raise ValueError("session crosses splits")
        support[r["entity_id"], split] += 1
    # Diagnostic inputs cannot silently become admission-eligible training.
    for c in vocab:
        for split, minimum in (("train", 20), ("validation", 1), ("test", 1)):
            if support[c, split] < minimum:
                raise ValueError(f"{c}: {split} support below {minimum}")


def new_run_directory(output_root, run_id):
    if not run_id or Path(run_id).name != run_id or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_" for c in run_id):
        raise ValueError("run-id must be a single alphanumeric directory name")
    path = Path(output_root) / run_id
    path.mkdir(parents=True, exist_ok=False)
    (path / "checkpoints").mkdir()
    return str(path)
