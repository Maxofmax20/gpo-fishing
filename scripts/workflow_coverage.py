"""Workflow coverage: extract complete WAITING->BITE->RESULT chains per session.

Reports sequence-level evidence for the v5.2.0 gate review (Q20-21):
how many real sessions contain a complete observable fishing workflow,
per split (trainer's exact FNV-1a session splits), plus what post-RESULT
evidence exists (entity linkage, OCR, action/CONFIRMATION fields — the
latter are absent by schema design and reported as gaps, not filled in).

Writes docs/workflow_coverage_v1.json. Read-only over the dataset.
"""

from __future__ import annotations

import json
import os
import sys
from collections import defaultdict

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(REPO, "ml"))
from gpo_train.dataset import fnv1a_session_split  # noqa: E402

APPDATA = os.environ.get("APPDATA", "")
LABELS = os.path.join(APPDATA, "gpo-autofish", "datasets", "gpo-vision", "v1", "labels.jsonl")
OUT = os.path.join(REPO, "docs", "workflow_coverage_v1.json")


def main() -> int:
    rows = []
    with open(LABELS, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    by_sess: dict[str, list[dict]] = defaultdict(list)
    for r in rows:
        by_sess[r.get("session_id", "")].append(r)

    complete: dict[str, list[str]] = {"train": [], "validation": [], "test": []}
    partial_bite_less = 0
    result_with_entity = 0
    result_with_ocr = 0
    result_total = 0
    sessions_with_action_fields = 0
    action_keys = {"action", "confirmation", "policy", "would_be_action"}
    for sess, v in by_sess.items():
        v.sort(key=lambda r: int(r.get("timestamp_ms", 0) or 0))
        if any(action_keys & set(r.keys()) for r in v):
            sessions_with_action_fields += 1
        stage = 0
        saw_result = False
        for r in v:
            s = r.get("game_state") or ""
            if stage == 0 and s == "waiting_for_bite":
                stage = 1
            elif stage == 1 and s == "bite":
                stage = 2
            elif stage == 2 and s == "catch_result":
                stage = 3
                break
            if s == "catch_result":
                saw_result = True
        if stage == 3:
            complete[fnv1a_session_split(sess)].append(sess)
        elif saw_result and stage == 0:
            partial_bite_less += 1
        for r in v:
            if (r.get("game_state") or "") == "catch_result":
                result_total += 1
                if r.get("entity_id"):
                    result_with_entity += 1
                if (r.get("ocr_text") or "").strip():
                    result_with_ocr += 1

    report = {
        "coverage_version": 1,
        "rows": len(rows),
        "sessions": len(by_sess),
        "complete_workflow_sessions": {k: {"n": len(v), "sessions": sorted(v)} for k, v in complete.items()},
        "partial_result_without_bite_chain": partial_bite_less,
        "result_rows": {"total": result_total, "with_entity": result_with_entity,
                       "with_ocr": result_with_ocr},
        "sessions_with_action_or_confirmation_fields": sessions_with_action_fields,
        "gaps": [
            "no ACTION/CONFIRMATION fields exist in the v1 schema (action_ui + confirmation gates stay red)",
            "RESULT without a WAITING->BITE prefix cannot be scored as a complete workflow",
        ],
    }
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2)
    print(f"rows={len(rows)} sessions={len(by_sess)} "
          f"complete train/val/test={len(complete['train'])}/{len(complete['validation'])}/{len(complete['test'])} "
          f"result entity {result_with_entity}/{result_total} ocr {result_with_ocr}/{result_total} "
          f"action_fields {sessions_with_action_fields}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
