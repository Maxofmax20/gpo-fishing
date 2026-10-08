//! READ-ONLY evidence for the fish training gate: what the gate measures on the
//! live store, and which check it fails.
//!
//! Run:
//! ```text
//! cargo run --example gate_evidence
//! cargo run --example gate_evidence -- <store-dir>
//! ```
//!
//! It reads `<store>/datasets/gpo-vision/v1/labels.jsonl` and
//! `<store>/reviews.jsonl`, projects the human verdicts onto the collected rows
//! (the same `project_training_labels` the snapshot and every gate use) and
//! prints the per-class TRAIN/TEST support the trainer will actually see, next
//! to what the LIVE file claims.
//!
//! The sufficiency verdict comes from
//! `commands_training::fish_train_support_failure` - the function the gate
//! itself calls - so this output cannot drift from what the app decides.
//!
//! Writes nothing. Trains nothing. Promotes nothing.

use gpo_autofish_lib::commands_training::fish_train_support_failure;
use gpo_autofish_lib::core::class_vocab;
use gpo_autofish_lib::core::ml_capability::{self, ENTITY_MIN_EXAMPLES};
use gpo_autofish_lib::core::ml_dataset::MlDatasetStore;
use gpo_autofish_lib::core::registry;
use gpo_autofish_lib::core::review::ReviewStore;
use gpo_autofish_lib::core::training;

fn store_dir() -> std::path::PathBuf {
    std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var("APPDATA").unwrap_or_default())
                .join("gpo-autofish")
        })
}

fn main() -> Result<(), String> {
    let dir = store_dir();
    println!("store: {}", dir.display());

    let live = MlDatasetStore::new(dir.clone())
        .annotations_checked()
        .map_err(|e| format!("labels.jsonl unreadable: {e}"))?;
    let projected = training::project_training_labels(&live, &ReviewStore::new(dir.clone()));
    if let Some(err) = &projected.read_error {
        return Err(format!("cannot measure the gate: {err}"));
    }
    println!(
        "\nprojection: {} live rows -> {} training rows ({} relabelled by review, \
{} kept the collector label, {} unreviewed, {} excluded by review)",
        projected.rows.len() + projected.rows_excluded_by_review,
        projected.rows.len(),
        projected.rows_relabelled_by_review,
        projected.rows_kept_collector_label,
        projected.rows_unreviewed,
        projected.rows_excluded_by_review,
    );

    let required = class_vocab::fish_vocab();
    let live_support = training::train_support_per_class(&live, "fish:");
    let support = training::train_support_per_class(&projected.rows, "fish:");

    println!(
        "\n{:<22} {:>6} {:>10} {:>7} {:>5} {:>6}",
        "class", "live", "projected", "TRAIN", "VAL", "TEST"
    );
    for c in required.iter().copied() {
        let (mut tr, mut va, mut te) = (0usize, 0usize, 0usize);
        for r in &projected.rows {
            if r.entity_id.as_deref() != Some(c) {
                continue;
            }
            match MlDatasetStore::split_of(&r.session_id).as_str() {
                "train" => tr += 1,
                "validation" => va += 1,
                _ => te += 1,
            }
        }
        println!(
            "{:<22} {:>6} {:>10} {:>7} {:>5} {:>6}",
            c,
            live_support.get(c).copied().unwrap_or(0),
            support.get(c).copied().unwrap_or(0),
            tr,
            va,
            te
        );
    }

    println!(
        "\nentity qualification over the PROJECTED rows \
(ENTITY_MIN_EXAMPLES={ENTITY_MIN_EXAMPLES}):"
    );
    for q in ml_capability::qualify_entities(&projected.rows, "fish:") {
        println!(
            "  {:<22} rows={:<5} sessions={:<3} test_covered={}",
            q.entity, q.examples, q.sessions, q.test_covered
        );
    }

    println!("\nreview coverage floor, over the projected set:");
    let reviewed = projected
        .rows
        .iter()
        .filter(|r| projected.reviewed_eligible_ids.contains(r.image_id.as_str()))
        .count();
    let scoped = projected.rows.iter().filter(|r| {
        r.entity_id.as_deref().is_some_and(|e| e.starts_with("fish:"))
    }).count();
    println!("  {reviewed}/{scoped} fish rows human-reviewed and training-eligible");

    match fish_train_support_failure(required, &support, ENTITY_MIN_EXAMPLES) {
        None => println!("\nfish class sufficiency: PASS"),
        Some(why) => println!("\nfish class sufficiency: FAIL\n  {why}"),
    }

    let registry_path = dir.join("training").join("registry").join("registry.json");
    let registry_sha = std::fs::read(&registry_path)
        .map(|b| gpo_autofish_lib::core::ml_model::sha256_hex(&b))
        .unwrap_or_else(|_| "absent".to_string());
    println!("\nregistry sha256: {registry_sha}");
    for c in registry::load_registry(&dir) {
        println!("  candidate {} -> {:?}", c.candidate_id, c.status);
    }
    Ok(())
}