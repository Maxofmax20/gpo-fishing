//! Review -> dataset -> snapshot -> candidate lifecycle (v5.6, hardened
//! v5.6.1).
//!
//! Windows-only like the other OS-coupled tests. Proves, on disposable data:
//!   row collected -> human review (correct + conflict + resolve + skip)
//!   -> dataset label updated through annotate() -> undo -> audit rebuild ->
//!   snapshot freezes the surviving rows AND the review fingerprint.
//!
//! The v5.6.1 additions are the point of this file: a human exclusion must
//! PHYSICALLY remove the row from the training snapshot, and the effective
//! state must be recoverable from the append-only audit alone.

#![cfg(windows)]

use std::path::PathBuf;

use gpo_autofish_lib::core::ml_dataset::MlDatasetStore;
use gpo_autofish_lib::core::review::{
    self, EligibilityInput, ReviewInput, ReviewStatus, ReviewStore,
};
use gpo_autofish_lib::core::training;

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gpo-review-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("datasets").join("gpo-vision").join("v1").join("images")).unwrap();
    d
}

fn seed_rows(dir: &PathBuf) {
    let v1 = dir.join("datasets").join("gpo-vision").join("v1");
    let row = |img: &str, sess: &str, ts: u64, ent: Option<&str>| {
        let ent_json = ent.map(|e| format!("\"{e}\"")).unwrap_or_else(|| "null".to_string());
        format!(
            "{{\"image_id\":\"{img}\",\"dataset_version\":1,\"session_id\":\"{sess}\",\"task\":\"entity_recognition\",\"ocr_text\":\"Golden Fish\",\"region_name\":\"drop\",\"ui_label\":null,\"bbox\":null,\"game_state\":\"catch_result\",\"entity_id\":{ent_json},\"annotator\":\"collector\",\"timestamp_ms\":{ts},\"source\":\"gameplay\",\"confidence\":null,\"hard_example\":false,\"hard_reason\":null,\"corrections\":[]}}"
        )
    };
    let body = vec![
        row("rv-aa", "sess-r1", 1000, Some("fish:golden")),
        row("rv-bb", "sess-r1", 2000, None),
        row("rv-cc", "sess-r2", 3000, Some("fish:shark")),
    ]
    .join("\n")
        + "\n";
    std::fs::write(v1.join("labels.jsonl"), body).unwrap();
    std::fs::write(v1.join("manifest.json"), r#"{"version":1,"split_overrides":{}}"#).unwrap();
    // Real (tiny) decodable PNGs so the bad-image gate passes honestly.
    let mut png = Vec::new();
    {
        use image::ImageEncoder;
        let raw = vec![128u8; 8 * 8 * 4];
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&raw, 8, 8, image::ExtendedColorType::Rgba8)
            .unwrap();
    }
    for id in ["rv-aa", "rv-bb", "rv-cc"] {
        std::fs::write(v1.join("images").join(format!("{id}.png")), &png).unwrap();
    }
}

fn elig(has_entity: bool) -> EligibilityInput {
    EligibilityInput {
        png_decodable: true,
        canonical_valid: true,
        is_duplicate: false,
        has_provenance: true,
        has_entity,
    }
}

#[test]
fn e2e_review_correct_updates_dataset_and_snapshot_freezes_reviews() {
    let dir = fresh_dir("correct");
    seed_rows(&dir);
    let store = ReviewStore::new(dir.clone());
    let ds = MlDatasetStore::new(dir.clone());

    // 1. Human confirms the collector's entity.
    let rec = review::apply_human_review(
        &store,
        ReviewInput {
            image_id: "rv-aa",
            session_id: "sess-r1",
            human_entity_id: Some("fish:golden"),
            canonical_name: Some("Golden"),
            correction_reason: None,
            model_prediction: Some("fish:golden"),
            model_confidence: Some(0.9),
            dataset_version: 1,
            eligibility: elig(true),
        },
    )
    .unwrap();
    assert_eq!(rec.review_status, ReviewStatus::ReviewedCorrect);
    assert_eq!(rec.human_entity_id.as_deref(), Some("fish:golden"));
    assert_eq!(rec.model_prediction.as_deref(), Some("fish:golden"));

    // 2. A competing verdict becomes a Conflict, never a silent overwrite,
    //    and a conflict is NOT training-eligible.
    let rec2 = review::apply_human_review(
        &store,
        ReviewInput {
            image_id: "rv-aa",
            session_id: "sess-r1",
            human_entity_id: Some("fish:shark"),
            canonical_name: Some("Shark"),
            correction_reason: Some("second look"),
            model_prediction: None,
            model_confidence: None,
            dataset_version: 1,
            // The real command layer passes valid inputs; the store must
            // still refuse to un-demote a conflict.
            eligibility: elig(true),
        },
    )
    .unwrap();
    assert_eq!(rec2.review_status, ReviewStatus::Conflict);
    assert!(!rec2.training_eligible, "a conflict must never be training-eligible");
    assert_eq!(rec2.excluded_reason.as_deref(), Some("conflict"));

    // 3. Resolution settles it with an audit trail AND keeps it eligible
    //    (v5.6.0 excluded every UI-resolved conflict as "invalid canonical
    //    mapping" because the canonical name was never supplied).
    let rec3 = review::resolve_conflict(
        &store,
        "rv-aa",
        "fish:golden",
        Some("Golden"),
        "checked pixels",
        &elig(true),
    )
    .unwrap();
    assert_eq!(rec3.review_status, ReviewStatus::ReviewedCorrected);
    assert!(rec3.training_eligible, "a resolved conflict must re-enter training");
    let audit = store.audit_tail(50);
    let kinds: Vec<&str> = audit.iter().map(|e| e.event.as_str()).collect();
    assert!(kinds.contains(&"REVIEW_CREATED"), "{kinds:?}");
    assert!(kinds.contains(&"CONFLICT"), "{kinds:?}");
    assert!(kinds.contains(&"RESOLVED"), "{kinds:?}");

    // 4. Dataset linkage: resolving writes the HUMAN label via annotate().
    ds.annotate("rv-aa", None, None, None, Some("fish:golden".to_string()), "human", false, None)
        .unwrap();
    let back = ds.annotations().into_iter().find(|r| r.image_id == "rv-aa").unwrap();
    assert_eq!(back.entity_id.as_deref(), Some("fish:golden"));
    assert_eq!(back.annotator, "human");

    // 5. Snapshot freezes the surviving rows AND the review state.
    let rows = ds.annotations();
    let snap = training::snapshot_dataset(&dir, &dir.join("snap"), &rows, 1).unwrap();
    assert_ne!(snap.review_fingerprint, "none", "reviews exist so fingerprint must be set");
    assert!(dir.join("snap").join("reviews.jsonl").exists());
    assert_eq!(snap.entity_registry_version, 1);
    // Only rv-aa is reviewed+eligible; rv-bb/rv-cc are unreviewed and kept.
    assert_eq!(snap.rows, 3);
    assert_eq!(snap.rows_unreviewed, 2);
    assert_eq!(snap.rows_excluded_by_review, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn e2e_excluded_rows_are_physically_absent_from_the_training_snapshot() {
    let dir = fresh_dir("exclude");
    seed_rows(&dir);
    let store = ReviewStore::new(dir.clone());
    let ds = MlDatasetStore::new(dir.clone());

    // rv-aa: confirmed -> trains.
    review::apply_human_review(
        &store,
        ReviewInput {
            image_id: "rv-aa",
            session_id: "sess-r1",
            human_entity_id: Some("fish:golden"),
            canonical_name: Some("Golden"),
            correction_reason: None,
            model_prediction: Some("fish:golden"),
            model_confidence: Some(0.9),
            dataset_version: 1,
            eligibility: elig(true),
        },
    )
    .unwrap();
    // rv-cc: skipped -> must NOT train, even though labels.jsonl still has it.
    review::skip_review(&store, "rv-cc", "sess-r2", Some("too blurry".to_string()), 1).unwrap();
    // rv-bb: reviewed as unknown -> must NOT train.
    review::apply_human_review(
        &store,
        ReviewInput {
            image_id: "rv-bb",
            session_id: "sess-r1",
            human_entity_id: None,
            canonical_name: None,
            correction_reason: None,
            model_prediction: Some("fish:mystery"),
            model_confidence: Some(0.2),
            dataset_version: 1,
            eligibility: EligibilityInput { has_entity: false, ..elig(false) },
        },
    )
    .unwrap();

    let rows = ds.annotations();
    assert_eq!(rows.len(), 3, "the dataset itself is untouched");
    let snap = training::snapshot_dataset(&dir, &dir.join("snap"), &rows, 1).unwrap();

    assert_eq!(snap.rows, 1, "only the confirmed row may train");
    assert_eq!(snap.rows_excluded_by_review, 2);
    assert_eq!(snap.rows_unreviewed, 0);

    // Verify the trainer-facing file, not just the metadata.
    let frozen = std::fs::read_to_string(dir.join("snap").join("labels.jsonl")).unwrap();
    let ids: Vec<String> = frozen
        .lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v.get("image_id").unwrap().as_str().unwrap().to_string()
        })
        .collect();
    assert_eq!(ids, vec!["rv-aa".to_string()], "frozen labels must contain only eligible rows");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn e2e_all_excluded_fails_closed_rather_than_training_on_nothing() {
    let dir = fresh_dir("failclosed");
    seed_rows(&dir);
    let store = ReviewStore::new(dir.clone());
    for (id, sess) in [("rv-aa", "sess-r1"), ("rv-bb", "sess-r1"), ("rv-cc", "sess-r2")] {
        review::skip_review(&store, id, sess, None, 1).unwrap();
    }
    let rows = MlDatasetStore::new(dir.clone()).annotations();
    let err = training::snapshot_dataset(&dir, &dir.join("snap"), &rows, 1).unwrap_err();
    assert!(err.contains("0 rows"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn e2e_state_is_rebuildable_from_the_append_only_audit() {
    let dir = fresh_dir("rebuild");
    seed_rows(&dir);
    let store = ReviewStore::new(dir.clone());
    review::apply_human_review(
        &store,
        ReviewInput {
            image_id: "rv-aa",
            session_id: "sess-r1",
            human_entity_id: Some("fish:golden"),
            canonical_name: Some("Golden"),
            correction_reason: None,
            model_prediction: Some("fish:golden"),
            model_confidence: Some(0.9),
            dataset_version: 1,
            eligibility: elig(true),
        },
    )
    .unwrap();
    review::skip_review(&store, "rv-cc", "sess-r2", Some("blurry".to_string()), 1).unwrap();

    let before: Vec<(String, String)> = store
        .list()
        .into_iter()
        .map(|r| (r.image_id, r.review_status.as_str().to_string()))
        .collect();
    let fp_before = store.fingerprint();

    // Simulate total loss of the state file: only the audit survives.
    std::fs::remove_file(dir.join("reviews.jsonl")).unwrap();
    assert!(store.list().is_empty(), "state really is gone");
    assert_eq!(store.rebuild_from_audit().unwrap(), 2);

    let after: Vec<(String, String)> = store
        .list()
        .into_iter()
        .map(|r| (r.image_id, r.review_status.as_str().to_string()))
        .collect();
    assert_eq!(after, before, "rebuild must reproduce the exact effective state");
    assert_eq!(store.fingerprint(), fp_before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn e2e_undo_restores_unreviewed_without_erasing_history() {
    let dir = fresh_dir("undo");
    seed_rows(&dir);
    let store = ReviewStore::new(dir.clone());
    review::apply_human_review(
        &store,
        ReviewInput {
            image_id: "rv-aa",
            session_id: "sess-r1",
            human_entity_id: Some("fish:golden"),
            canonical_name: Some("Golden"),
            correction_reason: None,
            model_prediction: Some("fish:golden"),
            model_confidence: Some(0.9),
            dataset_version: 1,
            eligibility: elig(true),
        },
    )
    .unwrap();
    let events_before = store.audit().len();

    let undone = review::undo_review(&store, "rv-aa").unwrap();
    assert_eq!(undone.review_status, ReviewStatus::Unreviewed);
    assert!(!undone.training_eligible, "an undone review must not train");
    assert_eq!(undone.excluded_reason.as_deref(), Some("undone"));

    // History is preserved (append-only), not rewritten.
    let events_after = store.audit();
    assert!(events_after.len() > events_before, "undo must append, never rewrite");
    assert!(
        events_after.iter().any(|e| e.event == review::AUDIT_RESTORED),
        "undo must be auditable"
    );
    // The earlier REVIEW_CREATED is still in the log.
    assert!(events_after.iter().any(|e| e.event == "REVIEW_CREATED"));

    // And the snapshot now excludes it again.
    let rows = MlDatasetStore::new(dir.clone()).annotations();
    let snap = training::snapshot_dataset(&dir, &dir.join("snap"), &rows, 1).unwrap();
    assert_eq!(snap.rows_excluded_by_review, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn e2e_unknown_review_is_excluded_never_trained() {
    let dir = fresh_dir("unknown");
    seed_rows(&dir);
    let store = ReviewStore::new(dir.clone());

    let rec = review::apply_human_review(
        &store,
        ReviewInput {
            image_id: "rv-bb",
            session_id: "sess-r1",
            human_entity_id: None,
            canonical_name: None,
            correction_reason: None,
            model_prediction: None,
            model_confidence: None,
            dataset_version: 1,
            eligibility: EligibilityInput { has_entity: false, ..elig(false) },
        },
    )
    .unwrap();
    assert_eq!(rec.review_status, ReviewStatus::ReviewedUnknown);
    assert!(!rec.training_eligible);
    assert!(rec.excluded_reason.is_some());

    // Eligibility gate agrees independently.
    let (ok, reason) = review::evaluate_eligibility(true, None, false, false, true);
    assert!(!ok);
    assert!(reason.unwrap().contains("unknown"));

    // Coverage counts it as reviewed-unknown, never as eligible.
    let cov = review::coverage(&store.list());
    assert_eq!(cov.unknown, 1);
    assert_eq!(cov.eligible, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn e2e_corrupt_state_line_survives_the_next_write() {
    let dir = fresh_dir("corrupt");
    seed_rows(&dir);
    let store = ReviewStore::new(dir.clone());
    // Write a valid record, then corrupt the file with an unparseable line.
    review::apply_human_review(
        &store,
        ReviewInput {
            image_id: "rv-aa",
            session_id: "sess-r1",
            human_entity_id: Some("fish:golden"),
            canonical_name: Some("Golden"),
            correction_reason: None,
            model_prediction: Some("fish:golden"),
            model_confidence: Some(0.9),
            dataset_version: 1,
            eligibility: elig(true),
        },
    )
    .unwrap();
    let mut raw = std::fs::read_to_string(dir.join("reviews.jsonl")).unwrap();
    raw.push_str("{ this is not json from a future version\n");
    std::fs::write(dir.join("reviews.jsonl"), raw).unwrap();
    assert_eq!(store.corrupt_lines().len(), 1);
    assert_eq!(store.list().len(), 1, "the good record still parses");

    // The next write must NOT destroy the unreadable line.
    review::skip_review(&store, "rv-cc", "sess-r2", None, 1).unwrap();
    assert_eq!(
        store.corrupt_lines().len(),
        1,
        "an unreadable line must be preserved, not silently deleted"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn e2e_status_names_round_trip_through_parse() {
    // The frontend sends SCREAMING_SNAKE_CASE; the filter must accept it.
    for name in [
        "UNREVIEWED",
        "REVIEWED_CORRECT",
        "REVIEWED_CORRECTED",
        "REVIEWED_UNKNOWN",
        "REVIEWED_SKIPPED",
        "CONFLICT",
    ] {
        assert_eq!(ReviewStatus::parse(name).unwrap().as_str(), name);
    }
    assert!(ReviewStatus::parse("nonsense").is_none());
    assert!(ReviewStatus::parse("conflict").is_some(), "case-insensitive");
}
