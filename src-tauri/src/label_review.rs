//! Fish label confirmation: the human step that turns proposals into reviews.
//!
//! # What this is
//!
//! `ml/gpo_train/candidates.py` proposes a fish label for every image whose text it
//! could read. Those are machine opinions. This module is where a human accepts or
//! rejects them, and it writes the result through
//! [`crate::core::review::apply_human_review`] - the same store every other review
//! path uses, so nothing here is a special case that training would not see.
//!
//! # Deliberately simple
//!
//! The question a human is answering is "are these all Crimson Snapper?", not
//! "is row 4,117 a Crimson Snapper?". So the screen shows one FISH at a time as a
//! grid of screenshots, and the whole batch is one judgement. A reviewer confirms
//! the obvious batches with one keystroke and flags the handful that look wrong.
//!
//! # What it does not do
//!
//! * It does not make a row training-eligible. Eligibility is computed by the
//!   review store from `canonical_valid` against the EFFECTIVE knowledge base
//!   (bundled + curator overlay). A confirmed row whose entity is not yet in the
//!   KB records a genuine human judgement and stays ineligible until a person
//!   applies the KB patch (`examples/apply_kb_patch.rs`). The screen says so
//!   rather than letting a reviewer assume the batch unlocked training.
//! * It does not invent, edit, or delete a knowledge-base entity.
//! * It never writes to `labels.jsonl`.
//!
//! # Image access
//!
//! Thumbnails are pre-rendered by the Python step. An image id is validated against
//! the set the batch file actually references, and must look like a hex id, so a
//! request never becomes a path.

use crate::commands_ontology_workstation as core;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::LazyLock;

/// The committed batch file, parsed once.
static BATCHES: LazyLock<Option<serde_json::Value>> = LazyLock::new(|| {
    core::find_report_dir()
        .and_then(|d| read_json(&d.join("workstation").join("label_batches.json")))
});

/// Every image id any batch references. Membership is what makes a thumbnail
/// request safe; shape alone would not be enough.
static KNOWN_IMAGE_IDS: LazyLock<Option<HashSet<String>>> = LazyLock::new(|| {
    let mut set = HashSet::new();
    if let Some(b) = BATCHES.as_ref() {
        if let Some(list) = b.get("batches").and_then(|b| b.as_array()) {
            for batch in list {
                if let Some(rows) = batch.get("rows").and_then(|r| r.as_array()) {
                    for r in rows {
                        if let Some(id) = r.get("imageId").and_then(|x| x.as_str()) {
                            set.insert(id.to_string());
                        }
                    }
                }
            }
        }
    }
    (!set.is_empty()).then_some(set)
});

/// Canonical KB entity ids, used only to report honestly which proposed entities
/// already exist.
///
/// Read from the EFFECTIVE knowledge base (compiled bundled base + the persisted
/// curator overlay), the same source `commands_review` and the bot itself use.
/// Reading the exported snapshot instead is how this screen and the real KB
/// drifted apart once already.
static KB_IDS: LazyLock<Option<HashSet<String>>> = LazyLock::new(|| {
    let kb = crate::config::Store::new(crate::app::data_dir()).effective_knowledge();
    let ids: HashSet<String> = kb.entities().iter().map(|e| e.id.clone()).collect();
    (!ids.is_empty()).then_some(ids)
});

fn read_json(p: &std::path::Path) -> Option<serde_json::Value> {
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn label_thumb_dir() -> Option<PathBuf> {
    Some(
        core::find_report_dir()?
            .join("workstation")
            .join("label_thumbs"),
    )
}

// ===========================================================================
// Read
// ===========================================================================

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchSummary {
    pub entity_id: String,
    pub name: String,
    pub row_count: usize,
    pub session_count: usize,
    pub avg_confidence: f64,
    pub min_confidence: f64,
    /// False for 15 of 16: the entity is real but the KB does not have it yet.
    pub in_knowledge_base: bool,
    pub stored_label_conflicts: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchIndex {
    pub available: bool,
    pub detail: String,
    pub batch_count: usize,
    pub total_rows: usize,
    pub rows_missing_from_kb: Vec<String>,
    pub batches: Vec<BatchSummary>,
}

/// The batch list WITHOUT per-row detail.
///
/// Rows are fetched one batch at a time. Shipping all 2,021 up front would put
/// ~400 KB on the wire for a screen that shows one fish at a time.
pub fn batch_index() -> BatchIndex {
    let Some(b) = BATCHES.as_ref() else {
        return BatchIndex {
            available: false,
            detail: "No label batches found. Build them with: \
                     python -m ml.gpo_train candidates"
                .into(),
            batch_count: 0,
            total_rows: 0,
            rows_missing_from_kb: vec![],
            batches: vec![],
        };
    };
    let batches = b
        .get("batches")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::with_capacity(batches.len());
    // Recompute KB membership here rather than trusting the built file: the KB may
    // have been patched since the last `candidates` run, and a stale "missing" flag
    // would be worse than none.
    let kb = KB_IDS.as_ref();
    for batch in &batches {
        let entity_id = batch
            .get("entityId")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let in_kb = kb.map(|s| s.contains(&entity_id)).unwrap_or(false);
        out.push(BatchSummary {
            entity_id,
            name: batch
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            row_count: batch.get("rowCount").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
            session_count: batch
                .get("sessionCount")
                .and_then(|x| x.as_u64())
                .unwrap_or(0) as usize,
            avg_confidence: batch
                .get("avgConfidence")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0),
            min_confidence: batch
                .get("minConfidence")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0),
            in_knowledge_base: in_kb,
            stored_label_conflicts: batch
                .get("storedLabelConflicts")
                .and_then(|x| x.as_u64())
                .unwrap_or(0) as usize,
        });
    }
    let total_rows = out.iter().map(|b| b.row_count).sum();
    let missing: Vec<String> = out
        .iter()
        .filter(|b| !b.in_knowledge_base)
        .map(|b| b.entity_id.clone())
        .collect();
    let detail = format!(
        "{} fish batches covering {total_rows} rows. {} of them name an entity the \
         knowledge base does not have yet: confirming those records your judgement \
         but leaves the rows ineligible until you apply the proposed KB patch.",
        out.len(),
        missing.len()
    );
    BatchIndex {
        available: true,
        detail,
        batch_count: out.len(),
        total_rows,
        rows_missing_from_kb: missing,
        batches: out,
    }
}

/// One batch in full, including rows.
pub fn batch_rows(entity_id: &str) -> Option<serde_json::Value> {
    let b = BATCHES.as_ref()?;
    b.get("batches")?
        .as_array()?
        .iter()
        .find(|x| x.get("entityId").and_then(|v| v.as_str()) == Some(entity_id))
        .cloned()
}

// ===========================================================================
// Image
// ===========================================================================

/// A batch thumbnail as bytes, or None.
pub fn thumbnail_bytes(image_id: &str) -> Option<Vec<u8>> {
    if !core::is_plausible_image_id(image_id) {
        return None;
    }
    let known = KNOWN_IMAGE_IDS.as_ref()?;
    if !known.contains(image_id) {
        return None;
    }
    std::fs::read(label_thumb_dir()?.join(format!("{image_id}.jpg"))).ok()
}

// ===========================================================================
// The write: a human review, through the real store
// ===========================================================================

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmInput {
    pub entity_id: String,
    pub image_ids: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmResult {
    pub ok: bool,
    pub confirmed: usize,
    pub skipped: usize,
    /// Rows recorded but ineligible because the entity is not in the KB yet.
    pub recorded_pending_entity: usize,
    pub entity_id: String,
    pub in_knowledge_base: bool,
    pub message: String,
}

/// Record a human review for each image in a batch.
///
/// Goes through `apply_human_review`, so the store's own conflict rule and
/// eligibility computation apply unchanged. A batch is a set of independent
/// judgements: one failure must not abandon the rest, so each row is attempted and
/// the outcome counted.
pub fn confirm_batch(input: ConfirmInput, data_dir: &std::path::Path) -> ConfirmResult {
    use crate::core::review::{
        apply_human_review, skip_review, EligibilityInput, ReviewInput, ReviewStore,
    };

    let entity_id = input.entity_id.trim().to_string();
    if entity_id.is_empty() {
        return ConfirmResult {
            ok: false,
            confirmed: 0,
            skipped: 0,
            recorded_pending_entity: 0,
            entity_id,
            in_knowledge_base: false,
            message: "entityId is required".into(),
        };
    }

    let Some(batch) = batch_rows(&entity_id) else {
        let message = format!("unknown batch {entity_id:?}");
        return ConfirmResult {
            ok: false,
            confirmed: 0,
            skipped: 0,
            recorded_pending_entity: 0,
            entity_id,
            in_knowledge_base: false,
            message,
        };
    };

    let session_of: std::collections::HashMap<&str, &str> = batch
        .get("rows")
        .and_then(|r| r.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    let id = r.get("imageId")?.as_str()?;
                    let s = r.get("sessionId")?.as_str().unwrap_or("");
                    Some((id, s))
                })
                .collect()
        })
        .unwrap_or_default();

    let in_kb = KB_IDS
        .as_ref()
        .map(|s| s.contains(&entity_id))
        .unwrap_or(false);

    let store = ReviewStore::new(data_dir.to_path_buf());
    let mut confirmed = 0usize;
    let mut skipped = 0usize;

    for image_id in &input.image_ids {
        if !core::is_plausible_image_id(image_id) {
            continue;
        }
        if !KNOWN_IMAGE_IDS
            .as_ref()
            .map(|s| s.contains(image_id))
            .unwrap_or(false)
        {
            continue;
        }
        let session_id = session_of.get(image_id.as_str()).copied().unwrap_or("");
        if session_id.is_empty() {
            skipped += 1;
            continue;
        }
        // `canonical_valid` must mean what its own doc says: the entity LITERALLY
        // EXISTS in the knowledge base.
        //
        // The first version of this called `EligibilityInput::library_default`,
        // whose contract is "for pure library callers that have no KB access" and
        // which treats any non-empty name as canonical. That marked 1,790 rows
        // `training_eligible: true` for entities the knowledge base does not have -
        // while the screen told the reviewer the opposite. The KB is loaded right
        // here, so the real membership check is available and is what is used.
        let eligibility = EligibilityInput {
            png_decodable: true,
            canonical_valid: in_kb,
            is_duplicate: false,
            has_provenance: !session_id.is_empty(),
            has_entity: true,
        };
        let result = apply_human_review(
            &store,
            ReviewInput {
                image_id,
                session_id,
                human_entity_id: Some(entity_id.as_str()),
                canonical_name: Some(entity_id.as_str()),
                correction_reason: Some("confirmed from the fish label review batch"),
                model_prediction: None,
                model_confidence: None,
                dataset_version: 1,
                eligibility,
            },
        );
        match result {
            Ok(_) => confirmed += 1,
            Err(_) => {
                // A row the store refuses (for example one already carrying a
                // conflicting verdict) is skipped rather than abandoned, and the
                // reviewer is told the batch was not fully applied.
                let _ = skip_review(
                    &store,
                    image_id,
                    session_id,
                    Some("label batch could not be applied".to_string()),
                    1,
                );
                skipped += 1;
            }
        }
    }

    let pending = if in_kb { 0 } else { confirmed };
    let message = if in_kb {
        format!(
            "{confirmed} review(s) recorded for {entity_id}. The entity exists in the \
             knowledge base, so these rows can become training-eligible."
        )
    } else {
        format!(
            "{confirmed} review(s) recorded for {entity_id}. The knowledge base has no \
             such entity yet, so the rows stay ineligible until you apply the \
             proposed KB patch by hand - the judgement is kept either way."
        )
    };

    ConfirmResult {
        ok: true,
        confirmed,
        skipped,
        recorded_pending_entity: pending,
        entity_id,
        in_knowledge_base: in_kb,
        message,
    }
}

/// Tauri command, for the desktop app.
#[tauri::command]
pub fn label_review_url() -> serde_json::Value {
    serde_json::json!({ "route": "/label-review" })
}

#[cfg(test)]
mod tests {
    fn own_code() -> String {
        let src = include_str!("label_review.rs");
        let body = src.split("#[cfg(test)]").next().unwrap_or(src);
        body.lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("//") && !t.starts_with("/*") && !t.starts_with('*')
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_batch_list_omits_rows_so_the_first_request_is_small() {
        let code = own_code();
        // Scoped to this one function: `batch_rows` and `confirm_batch` both read
        // `rows`, and slicing to the end of the file would make this test pass for
        // the wrong reason.
        let body = code
            .split("pub fn batch_index")
            .nth(1)
            // Split on CODE, not on the doc comment above `batch_rows`: `own_code`
            // strips comments, so a comment marker would never be found and the
            // slice would run to the end of the file.
            .and_then(|s| s.split("pub fn batch_rows").next())
            .unwrap_or("");
        assert!(body.contains("BatchSummary"), "the index builds summaries");
        assert!(
            !body.contains("\"rows\""),
            "the index must not inline per-row detail; rows are fetched per batch"
        );
    }

    #[test]
    fn image_access_requires_membership_not_just_shape() {
        let code = own_code();
        let f = code.split("pub fn thumbnail_bytes").nth(1).unwrap_or("");
        assert!(f.contains("is_plausible_image_id"));
        assert!(
            f.contains("KNOWN_IMAGE_IDS"),
            "a valid-looking id that no batch references must still be refused"
        );
    }

    #[test]
    fn confirmation_goes_through_the_real_review_store() {
        let code = own_code();
        assert!(
            code.contains("apply_human_review"),
            "must use the real store"
        );
        assert!(code.contains("ReviewInput"));
        assert!(code.contains("EligibilityInput"));
        // No bespoke eligibility maths: that would be a second, weaker gate.
        assert!(!code.contains("training_eligible"));
    }

    #[test]
    fn this_module_cannot_reach_the_kb_the_model_or_the_bot() {
        let code = own_code();
        for forbidden in [
            "FISH_VOCAB",
            "knowledge.rs",
            "class_vocab",
            "readiness",
            "registry",
            "ml_dataset",
            "bot::",
            "controller",
            "enigo",
            "SendInput",
            "labels.jsonl",
        ] {
            assert!(
                !code.contains(forbidden),
                "label review must not reference {forbidden:?}"
            );
        }
    }

    #[test]
    fn eligibility_requires_the_entity_to_exist_not_merely_to_be_named() {
        // The bug this pins: `EligibilityInput::library_default` treats any
        // non-empty name as canonical, because it is for callers with no KB access.
        // Using it here made 1,790 rows eligible for entities the knowledge base
        // does not contain, while the UI told the reviewer they were not.
        let code = own_code();
        let f = code.split("pub fn confirm_batch").nth(1).unwrap_or("");
        assert!(
            !f.contains("library_default"),
            "confirm_batch must not use the KB-less eligibility helper"
        );
        assert!(
            f.contains("canonical_valid: in_kb"),
            "canonical_valid must be real KB membership"
        );
    }

    #[test]
    fn a_missing_entity_is_reported_rather_than_papered_over() {
        // The whole screen would be misleading if it implied confirming a batch
        // unlocks training for an entity the knowledge base does not have.
        let code = own_code();
        let f = code.split("pub fn confirm_batch").nth(1).unwrap_or("");
        assert!(f.contains("in_kb"));
        assert!(f.contains("stay ineligible"));
    }

    #[test]
    fn unknown_fields_on_a_confirmation_are_rejected() {
        // Otherwise a client could send a field the server silently ignores, and
        // believe it had an effect.
        let code = own_code();
        assert!(
            code.contains("deny_unknown_fields"),
            "ConfirmInput must reject unknown fields"
        );
    }
}
