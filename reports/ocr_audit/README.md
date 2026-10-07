# OCR Audit — artefact index

Images audited: **14805**. Contact sheets: **131**.
Produced by `python -m ml.gpo_ocr_audit`. Everything here is **derived** from
the dataset and is safe to delete and regenerate at any time.

## Files

| file | what it answers |
|---|---|
| `evidence.jsonl` | One line per image: the complete, replayable analysis. |
| `provenance.json` | Dataset/KB/OCR fingerprints and configuration. |
| `summary.md` | Headline counts, confidences, per-class clean share. |
| `class_quality.md` | Per-fish-class breakdown of what supports each label. |
| `compound_names.md` | Names containing 2+ canonical entities. |
| `unknown_entities.md` | **Candidate ontology concepts** needing a decision. |
| `conflicts.md` | Rows where the text disagrees with the stored label. |
| `sessions.md` | Recurrence per session and per calendar day. |
| `ocr_quality.md` | Confidence distribution and correction-rule firing. |
| `agreement_matrix.md` | Stored / model / OCR three-way comparison. |
| `search_index.sqlite` | Queryable: concept text → images, sessions, labels. |
| `contact_sheets/` | Grids of the actual source images with captions. |

## Querying the index

```text
python -m ml.gpo_ocr_audit --query "crimson snapper"
```

or directly:

```sql
sqlite3 search_index.sqlite
SELECT concept, occurrences, sessions FROM concepts ORDER BY occurrences DESC;
SELECT image_id, stored_entity_id, raw_ocr FROM evidence
  WHERE residual_text LIKE '%crimson snapper%';
```

## Provenance of this run

- pipeline: `ocr-audit/1.0.0`
- normalisation rules: `ocr-rules/1.0.0`
- OCR engine: `rapidocr-onnxruntime 1.2.3`
- OCR fingerprint: `12de598070fe22db0be7ca012f85a5ef26930d9bebddcc786bfaa1511f002e06`
- KB: version 1, 141 entities (256508a6e89ed593…)
- `FISH_VOCAB.txt` sha256: `256508a6e89ed593…`
- `labels.jsonl` sha256: `a6fac80bd41202fa…`
- `evidence.jsonl` sha256: `24df6a4b40d5d6fd…`
- CUDA used: no

## What this pipeline did NOT do

- **No new ontology entity was created.** Names it cannot resolve are reported
  as *candidate* concepts in `unknown_entities.md`; the KB is untouched.
- **No training label was changed.** `labels.jsonl` is opened read-only and no
  `training_eligible` flag was written anywhere.
- **No TEST data was used for training or tuning.** TEST rows appear in the
  reports for reporting only; the split assignment is imported from the
  project's existing rule and never re-derived.
- **No production safety gate was weakened.** No threshold was lowered, no
  review requirement altered, no model promoted or retrained.
- **No game input was controlled.** The pipeline imports no input, macro or
  controller module.
- **No synthetic data.** Every row traces to a real image by SHA-256.

## Current production status

```text
MODEL_NOT_READY
```

This audit produces evidence about data quality. It cannot and does not change
the model's readiness status.
