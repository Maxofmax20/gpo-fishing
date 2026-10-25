//! Replay the human-reviewed fish rows through the REAL runtime resolver and
//! print the outcome buckets. This is the Rust-side verification of
//! `ml/gpo_train/resolution_report.py`: the Python report is only credible if
//! it matches what the actual code path does.
//!
//! Read-only: it writes nothing.
//!
//! Usage: cargo run --example resolution_replay

use gpo_autofish_lib::app::data_dir;
use gpo_autofish_lib::config::Store as ConfigStore;
use gpo_autofish_lib::core::perception::{correlate_text, ScreenKind};
use std::collections::{BTreeMap, HashSet};

const V2: [&str; 15] = [
    "fish:bluelipgrouper",
    "fish:candycornsquid",
    "fish:crimsonpolkapuffer",
    "fish:crimsonsnapper",
    "fish:darkskeletalshark",
    "fish:exotictigerfin",
    "fish:fangfish",
    "fish:goldenpolkapuffer",
    "fish:goldenribbonangelfish",
    "fish:goldentigerfin",
    "fish:jackobite",
    "fish:skeletalshark",
    "fish:swordfish",
    "fish:tigerfin",
    "fish:zebraribbonangelfish",
];

fn main() {
    let dir = data_dir();
    let labels_raw = std::fs::read_to_string(
        dir.join("datasets")
            .join("gpo-vision")
            .join("v1")
            .join("labels.jsonl"),
    )
    .unwrap();
    let reviews_raw = std::fs::read_to_string(dir.join("reviews.jsonl")).unwrap();
    let ocr: std::collections::HashMap<String, String> = labels_raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .map(|v| {
            (
                v["image_id"].as_str().unwrap_or("").to_string(),
                v["ocr_text"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();

    // Legacy stems = bundled fish ids minus the confirmed species.
    let bundled: HashSet<String> = gpo_autofish_lib::core::knowledge::KnowledgeBase::bundled()
        .entities()
        .iter()
        .filter(|e| e.id.starts_with("fish:"))
        .map(|e| e.id.clone())
        .collect();
    let vocab: HashSet<&str> = V2.iter().copied().collect();
    let stems: HashSet<&str> = bundled
        .iter()
        .map(|s| s.as_str())
        .filter(|id| !vocab.contains(*id))
        .collect();

    let kb = ConfigStore::new(dir).effective_knowledge();
    let mut buckets: BTreeMap<&str, usize> = BTreeMap::new();
    let mut reviewed = 0usize;
    let mut ceiling_numer = 0usize;
    let mut ceiling_denom = 0usize;

    for line in reviews_raw.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(r) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(human) = r["human_entity_id"].as_str() else {
            continue;
        };
        if human.is_empty() {
            continue;
        }
        reviewed += 1;
        let text = ocr
            .get(r["image_id"].as_str().unwrap_or(""))
            .cloned()
            .unwrap_or_default();
        let obs = correlate_text(&kb, &text, "drop", ScreenKind::Fishing, 0.85, 0.80, None);
        let runtime_id = obs.entity.as_ref().map(|m| m.entity_id.as_str());
        let ambiguous = obs
            .unknown_reason
            .as_deref()
            .map(|r| r.starts_with("AMBIGUOUS"))
            .unwrap_or(false);
        let in_vocab = vocab.contains(human);
        if in_vocab {
            ceiling_denom += 1;
        }
        let bucket = if ambiguous {
            "ambiguous"
        } else if runtime_id.is_none() {
            "unknown"
        } else if in_vocab && runtime_id == Some(human) {
            "correct_species"
        } else if runtime_id.is_some_and(|id| stems.contains(id)) {
            "legacy_stem"
        } else {
            "different_species"
        };
        *buckets.entry(bucket).or_default() += 1;
        if in_vocab && runtime_id == Some(human) {
            ceiling_numer += 1;
        }
    }

    println!("  reviewed rows: {reviewed}");
    for (k, v) in &buckets {
        println!(
            "    {k:<18} {v:>5}  {:5.1}%",
            *v as f64 / reviewed as f64 * 100.0
        );
    }
    println!(
        "  ceiling: {ceiling_numer}/{ceiling_denom} = {:.1}% (floor 80%)",
        ceiling_numer as f64 / ceiling_denom as f64 * 100.0
    );
}
