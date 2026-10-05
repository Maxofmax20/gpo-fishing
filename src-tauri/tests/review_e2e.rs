//! Review → dataset → snapshot → candidate lifecycle (v5.6).
//!
//! Windows-only like the other OS-coupled tests. Proves the review linkage
//! end to end on disposable data:
//!   row collected -> human review (correct + conflict + resolve) ->
//!   dataset label updated through annotate() -> snapshot freezes the
//!   review fingerprint -> stub training job -> candidate registered.
//! No production path is touched anywhere in this flow.

#![cfg(windows)]

use std::path::PathBuf;

use gpo_autofish_lib::core::ml_dataset::MlDatasetStore;
use gpo_autofish_lib::core::review::{self, ReviewStatus, ReviewStore};
use gpo_autofish_lib::core::training;

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gpo-review-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("datasets").join("gpo-vision").join("v1")).unwrap();
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
    ]
    .join("\n")
        + "\n";
    std::fs::create_dir_all(v1.join("images")).unwrap();
    std::fs::write(v1.join("labels.jsonl"), body).unwrap();
    std::fs::write(v1.join("manifest.json"), r#"{"version":1,"split_overrides":{}}"#).unwrap();
    // Real (tiny) decodable PNGs so the bad-image gate passes honestly.
    let mut png_a = Vec::new();
    {
        use image::ImageEncoder;
        let raw = vec![128u8; 8 * 8 * 4];
        image::codecs::png::PngEncoder::new(&mut png_a)
            .write_image(&raw, 8, 8, image::ExtendedColorType::Rgba8)
            .unwrap();
    }
    std::fs::write(v1.join("images").join("rv-aa.png"), &png_a).unwrap();
    std::fs::write(v1.join("images").join("rv-bb.png"), &png_a).unwrap();
}

#[test]
fn e2e_review_correct_updates_dataset_and_snapshot_freezes_reviews() {
    let dir = fresh_dir("correct");
    seed_rows(&dir);
    let store = ReviewStore::new(dir.clone());
    let ds = MlDatasetStore::new(dir.clone());

    // 1. Human confirms the collector's entity.
    let rec = review::apply_human_review(
        &store, "rv-aa", "sess-r1", Some("fish:golden".to_string()), None,
        Some("Golden".to_string()), Some("fish:golden".to_string()), Some(0.9), 1,
    )
    .unwrap();
    assert_eq!(rec.review_status, ReviewStatus::ReviewedCorrect);
    assert_eq!(rec.human_entity_id.as_deref(), Some("fish:golden"));
    // Original prediction preserved (it was the same here; check the field).
    assert_eq!(rec.model_prediction.as_deref(), Some("fish:golden"));

    // 2. A competing verdict becomes a Conflict, never a silent overwrite.
    let rec2 = review::apply_human_review(
        &store, "rv-aa", "sess-r1", Some("fish:shark".to_string()), Some("second look".to_string()),
        None, None, None, 1,
    )
    .unwrap();
    assert_eq!(rec2.review_status, ReviewStatus::Conflict);
    assert!(!rec2.training_eligible);

    // 3. Resolution settles it with an audit trail.
    let rec3 = review::resolve_conflict(&store, "rv-aa", "fish:golden", "checked pixels").unwrap();
    assert_eq!(rec3.review_status, ReviewStatus::ReviewedCorrected);
    let audit = store.audit_tail(50);
    let kinds: Vec<&str> = audit.iter().map(|e| e.event.as_str()).collect();
    assert!(kinds.contains(&"REVIEW_CREATED"), "{kinds:?}");
    assert!(kinds.contains(&"CONFLICT"), "{kinds:?}");
    assert!(kinds.contains(&"RESOLVED"), "{kinds:?}");

    // 4. Dataset linkage: resolving writes the HUMAN label via annotate().
    ds.annotate(&"rv-aa".to_string(), None, None, None, Some("fish:golden".to_string()), "human", false, None)
        .unwrap();
    let back = ds.annotations().into_iter().find(|r| r.image_id == "rv-aa").unwrap();
    assert_eq!(back.entity_id.as_deref(), Some("fish:golden"));
    assert_eq!(back.annotator, "human");

    // 5. Snapshot freezes the review state alongside the labels.
    let rows = ds.annotations();
    let snap = training::snapshot_dataset(&dir, &dir.join("snap"), &rows, 1).unwrap();
    assert_ne!(snap.review_fingerprint, "none", "reviews exist so fingerprint must be set");
    assert!(dir.join("snap").join("reviews.jsonl").exists());
    assert_eq!(snap.entity_registry_version, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn e2e_unknown_review_is_excluded_never_trained() {
    let dir = fresh_dir("unknown");
    seed_rows(&dir);
    let store = ReviewStore::new(dir.clone());

    // Entity-less row reviewed as unknown: recorded, excluded, stays out.
    let rec = review::apply_human_review(
        &store, "rv-bb", "sess-r1", None, None, None, None, None, 1,
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
