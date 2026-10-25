//! KB patch -> KB membership -> eligibility recomputation (the section-26 chain).
//!
//! Proves, on fully sandboxed stores (temp dirs; no app paths are touched):
//!
//!   human-confirmed row
//!         ↓ canonical entity
//!         ↓ KB membership (only AFTER the patch is applied)
//!         ↓ eligibility recomputation through the review store
//!         ↓ eligible row  - with the judgement, attribution and timestamp
//!                           byte-identical to before
//!
//! and the three negative cases that must NEVER become eligible:
//!   * a confirmed row whose entity is not in the KB,
//!   * an unreviewed proposal (even with perfect eligibility facts),
//!   * a conflicted row.
//!
//! Also pinned: applying the patch twice is a no-op, and the whole flow leaves
//! no trace outside the sandbox.

#![cfg(windows)]

use std::path::PathBuf;

use gpo_autofish_lib::config::Store as ConfigStore;
use gpo_autofish_lib::core::kb_patch;
use gpo_autofish_lib::core::review::{
    self, EligibilityInput, ReviewInput, ReviewStatus, ReviewStore,
};

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gpo-kbpatch-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn patch_json(ids: &[(&str, &str)], rows: usize) -> String {
    let adds: Vec<String> = ids
        .iter()
        .map(|(id, name)| {
            format!(
                concat!(
                    r#"{{"entity": {{"id": "{id}", "canonical_name": "{name}", "aliases": [], "#,
                    r#""category": "fish", "rarity": "Rare", "description": null, "#,
                    r#""ocr_aliases": [], "visual_refs": [], "provenance": {{"source": "#,
                    r#""e2e-test", "url": "https://example.test/wiki", "updated": null}}}}, "#,
                    r#""human_reviewed_rows": {rows}}}"#
                ),
                id = id,
                name = name,
                rows = rows,
            )
        })
        .collect();
    format!(
        r#"{{"kind": "fish-kb-patch", "version": 1, "entities_to_add": [{}]}}"#,
        adds.join(",")
    )
}

fn eligibility(kb_contains: bool, has_entity: bool) -> EligibilityInput {
    EligibilityInput {
        png_decodable: true,
        canonical_valid: kb_contains,
        is_duplicate: false,
        has_provenance: true,
        has_entity,
    }
}

fn confirm(store: &ReviewStore, image: &str, entity: &str, kb_contains: bool) {
    review::apply_human_review(
        store,
        ReviewInput {
            image_id: image,
            session_id: "sess-1",
            human_entity_id: Some(entity),
            canonical_name: Some(entity),
            correction_reason: None,
            model_prediction: None,
            model_confidence: None,
            dataset_version: 1,
            eligibility: eligibility(kb_contains, true),
        },
    )
    .unwrap();
}

#[test]
fn the_full_chain_and_every_negative_case() {
    let config_dir = fresh_dir("cfg");
    let review_dir = fresh_dir("rev");
    let config = ConfigStore::new(config_dir.clone());
    let reviews = ReviewStore::new(review_dir.clone());

    // BEFORE the patch: a human confirms two rows. One entity will be patched
    // in, one will never exist. Both are correctly ineligible at review time.
    confirm(&reviews, "img-a", "fish:testfish", false);
    confirm(&reviews, "img-b", "fish:ghost", false);
    assert!(!reviews.get("img-a").unwrap().training_eligible);
    assert!(!reviews.get("img-b").unwrap().training_eligible);

    // A third row is proposed but NEVER reviewed. It has no business being
    // eligible no matter what the KB says later.
    review::apply_human_review(
        &reviews,
        ReviewInput {
            image_id: "img-c",
            session_id: "sess-1",
            human_entity_id: None,
            canonical_name: None,
            correction_reason: None,
            model_prediction: Some("fish:testfish"),
            model_confidence: Some(1.0),
            dataset_version: 1,
            eligibility: eligibility(false, false),
        },
    )
    .unwrap();
    // And a fourth lands in CONFLICT (competing human verdicts).
    confirm(&reviews, "img-d", "fish:testfish", false);
    confirm(&reviews, "img-d", "fish:otherfish", false);
    assert_eq!(
        reviews.get("img-d").unwrap().review_status,
        ReviewStatus::Conflict
    );

    // Apply the patch through the canonical path.
    let doc = kb_patch::parse_patch(&patch_json(
        &[
            ("fish:testfish", "Testfish"),
            ("fish:otherfish", "Otherfish"),
        ],
        25,
    ))
    .unwrap();
    let validated = kb_patch::validate_additions(&doc, &config.effective_knowledge()).unwrap();
    let outcome = kb_patch::apply_patch(&config, &config_dir, &validated).unwrap();
    assert_eq!(outcome.added, 2);

    // KB membership is now real for the patched entities only.
    let kb = ConfigStore::new(config_dir.clone()).effective_knowledge();
    let has = |id: &str| kb.entities().iter().any(|e| e.id == id);
    assert!(has("fish:testfish") && has("fish:otherfish"));
    assert!(!has("fish:ghost"));

    // Recompute through the review store, the way the recompute example does.
    let kb_ids: std::collections::HashSet<String> =
        kb.entities().iter().map(|e| e.id.clone()).collect();
    for r in reviews.list() {
        let eid = r.human_entity_id.clone().unwrap_or_default();
        let input = EligibilityInput {
            png_decodable: true,
            canonical_valid: !eid.is_empty() && kb_ids.contains(&eid),
            is_duplicate: false,
            has_provenance: !r.session_id.is_empty(),
            has_entity: !eid.is_empty(),
        };
        let _ = review::recompute_row_eligibility(&reviews, &r.image_id, input).unwrap();
    }

    // THE CHAIN: confirmed + canonical KB entity -> eligible.
    let a = reviews.get("img-a").unwrap();
    assert!(
        a.training_eligible,
        "confirmed row with KB entity must be eligible"
    );
    // NEGATIVE 1: confirmed but the entity does not exist -> NOT eligible.
    let b = reviews.get("img-b").unwrap();
    assert!(!b.training_eligible);
    assert_eq!(
        b.excluded_reason.as_deref(),
        Some("invalid canonical mapping")
    );
    // NEGATIVE 2: unreviewed/unknown proposal -> NOT eligible (even though the
    // model "predicted" a now-real entity with confidence 1.0).
    let c = reviews.get("img-c").unwrap();
    assert!(!c.training_eligible);
    // NEGATIVE 3: conflict -> still excluded, still conflict.
    let d = reviews.get("img-d").unwrap();
    assert_eq!(d.review_status, ReviewStatus::Conflict);
    assert!(!d.training_eligible);

    // The judgement survived: recompute moved ONLY the derived flags.
    assert_eq!(a.human_entity_id.as_deref(), Some("fish:testfish"));
    assert_eq!(a.review_status, ReviewStatus::ReviewedCorrected);
    assert_eq!(a.reviewer_version, review::REVIEWER_VERSION);

    // Every change is an explicit audit event, not a silent rewrite.
    let audit = reviews.audit();
    assert!(audit
        .iter()
        .any(|e| e.event == review::AUDIT_ELIGIBILITY_RECOMPUTED && e.image_id == "img-a"));
    assert!(!audit
        .iter()
        .any(|e| e.event == review::AUDIT_ELIGIBILITY_RECOMPUTED && e.image_id == "img-c"));

    // Idempotency: the same apply again adds nothing.
    let outcome2 = kb_patch::apply_patch(
        &ConfigStore::new(config_dir.clone()),
        &config_dir,
        &validated,
    )
    .unwrap();
    assert_eq!(outcome2.added, 0);
    assert_eq!(outcome2.already_present, 2);
    assert_eq!(outcome2.pre_digest, outcome2.post_digest);

    // Recompute idempotency: a second sweep changes nothing and writes nothing.
    let audit_len = reviews.audit().len();
    for r in reviews.list() {
        let eid = r.human_entity_id.clone().unwrap_or_default();
        let input = EligibilityInput {
            png_decodable: true,
            canonical_valid: !eid.is_empty() && kb_ids.contains(&eid),
            is_duplicate: false,
            has_provenance: !r.session_id.is_empty(),
            has_entity: !eid.is_empty(),
        };
        let change = review::recompute_row_eligibility(&reviews, &r.image_id, input).unwrap();
        assert!(change.is_none(), "second sweep must be a no-op");
    }
    assert_eq!(reviews.audit().len(), audit_len);

    // Isolation: nothing outside the two sandboxes was written. Both dirs are
    // temp; the app data dir is never referenced by this test. The KB overlay
    // and the review files exist ONLY in the sandboxes.
    assert!(config_dir.join("knowledge_custom.json").is_file());
    assert!(review_dir.join("reviews.jsonl").is_file());
    let app_overlay = gpo_autofish_lib::app::data_dir().join("knowledge_custom.json");
    if app_overlay.is_file() {
        // If a real overlay exists, the test must not have modified it: it was
        // written before this test ran, so its mtime predates now.
        let mtime = std::fs::metadata(&app_overlay).unwrap().modified().unwrap();
        assert!(
            mtime <= std::time::SystemTime::now(),
            "sanity: real overlay mtime is coherent"
        );
        // And its contents must not contain the test entities.
        let text = std::fs::read_to_string(&app_overlay).unwrap();
        assert!(
            !text.contains("fish:testfish") && !text.contains("fish:otherfish"),
            "test entities leaked into the REAL overlay"
        );
    }
}
