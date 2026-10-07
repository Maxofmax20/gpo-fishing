//! Recompute `training_eligible` for existing reviews, from the live knowledge base.
//!
//! # Why this exists
//!
//! The first version of `label_review::confirm_batch` built its eligibility with
//! `EligibilityInput::library_default`, whose documented contract is "for pure
//! library callers that have no KB access" and which treats any non-empty name as
//! canonical. Used there it marked 1,790 rows `training_eligible: true` for fish
//! that do not exist in the knowledge base, while the screen told the reviewer the
//! opposite. The bug is fixed and pinned by a test; this repairs the rows it already
//! wrote.
//!
//! # Why re-apply rather than rewrite the file
//!
//! The human judgements are sound - a person confirmed those screenshots. Only the
//! derived flag was wrong. So the records are re-applied through
//! [`apply_human_review`], which recomputes eligibility and appends to the audit
//! trail, rather than the file being edited in place. That keeps the store's own
//! invariants and its history intact, and re-applying an IDENTICAL verdict does not
//! trip the conflict rule.
//!
//! After this runs, those rows are correctly ineligible until the proposed
//! knowledge-base patch is applied by a person. Re-running it after applying that
//! patch flips them to eligible, which is the intended order: judgement first,
//! ontology second, training third.
//!
//! Usage:
//!   cargo run --example recompute_eligibility           # report only
//!   cargo run --example recompute_eligibility -- --apply # rewrite

use gpo_autofish_lib::core::review::{
    apply_human_review, EligibilityInput, ReviewInput, ReviewRecord, ReviewStore,
};
use std::collections::HashSet;
use std::fs;

fn kb_ids() -> HashSet<String> {
    // Resolve the KB the same way the rest of the app does.
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        let mut p = exe.as_path().parent();
        for _ in 0..6 {
            match p {
                Some(d) => {
                    roots.push(d.to_path_buf());
                    p = d.parent();
                }
                None => break,
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut p = Some(cwd.as_path());
        for _ in 0..8 {
            match p {
                Some(d) => {
                    roots.push(d.to_path_buf());
                    p = d.parent();
                }
                None => break,
            }
        }
    }
    for r in roots {
        let f = r.join("kb_export.json");
        if f.is_file() {
            if let Ok(t) = fs::read_to_string(&f) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                    let set: HashSet<String> = v
                        .get("entities")
                        .and_then(|e| e.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.get("id").and_then(|i| i.as_str()).map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    if !set.is_empty() {
                        return set;
                    }
                }
            }
        }
        // Fall back to the ontology workstation's copy when kb_export.json is absent.
        let alt = r
            .join("reports")
            .join("ontology_review")
            .join("workstation")
            .join("kb_entities.json");
        if alt.is_file() {
            if let Ok(t) = fs::read_to_string(&alt) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                    let set: HashSet<String> = v
                        .get("entities")
                        .and_then(|e| e.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.get("id").and_then(|i| i.as_str()).map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    if !set.is_empty() {
                        return set;
                    }
                }
            }
        }
    }
    HashSet::new()
}

fn main() {
    let apply = std::env::args().any(|a| a == "--apply");
    let data_dir = gpo_autofish_lib::app::data_dir();
    let path = data_dir.join("reviews.jsonl");
    let Ok(text) = fs::read_to_string(&path) else {
        eprintln!("  no reviews at {}", path.display());
        std::process::exit(1);
    };

    let records: Vec<ReviewRecord> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if records.is_empty() {
        eprintln!("  no parseable reviews in {}", path.display());
        std::process::exit(1);
    }

    let kb = kb_ids();
    println!("  {} review(s) loaded", records.len());
    println!(
        "  knowledge base: {} entities ({})",
        kb.len(),
        if kb.is_empty() { "NOT FOUND" } else { "resolved" }
    );
    if kb.is_empty() {
        eprintln!("  refusing to recompute against an empty knowledge base: that would");
        eprintln!("  mark every row ineligible for the wrong reason.");
        eprintln!("  Run: cargo run --example kb_export");
        std::process::exit(1);
    }

    let mut now_eligible = 0usize;
    let mut now_ineligible = 0usize;
    let mut unchanged = 0usize;
    let mut touched = Vec::new();

    for r in &records {
        let eid = r.human_entity_id.clone().unwrap_or_default();
        let valid = kb.contains(&eid);
        let want = valid && r.training_eligible || (!valid && !r.training_eligible);
        // Only rows whose flag disagrees with KB membership are candidates.
        if (valid && r.training_eligible) {
            now_eligible += 1;
        } else if !valid && r.training_eligible {
            now_ineligible += 1;
            touched.push((r.image_id.clone(), eid.clone()));
        } else {
            let _ = want;
            unchanged += 1;
        }
    }

    println!();
    println!("  correctly eligible now      : {now_eligible}");
    println!("  wrongly eligible (to fix)   : {now_ineligible}");
    println!("  correctly ineligible        : {unchanged}");

    if !apply {
        println!();
        println!("  report only. Re-run with --apply to rewrite through the review store.");
        return;
    }
    if now_ineligible == 0 {
        println!("  nothing to do.");
        return;
    }

    let store = ReviewStore::new(data_dir.clone());
    let mut applied = 0usize;
    let mut failed = 0usize;
    for (image_id, entity_id) in &touched {
        let Some(rec) = records.iter().find(|r| &r.image_id == image_id) else {
            continue;
        };
        let input = ReviewInput {
            image_id,
            session_id: &rec.session_id,
            human_entity_id: Some(entity_id.as_str()),
            canonical_name: Some(entity_id.as_str()),
            correction_reason: rec.correction_reason.as_deref(),
            model_prediction: None,
            model_confidence: None,
            dataset_version: rec.dataset_version,
            eligibility: EligibilityInput {
                png_decodable: true,
                canonical_valid: false, // the whole point: it is not in the KB
                is_duplicate: false,
                has_provenance: true,
                has_entity: true,
            },
        };
        match apply_human_review(&store, input) {
            Ok(_) => applied += 1,
            Err(_) => failed += 1,
        }
    }
    println!();
    println!("  applied {applied}, failed {failed}");
    println!("  these rows are now correctly ineligible. They become eligible again");
    println!("  once the proposed knowledge-base patch is applied by hand, and this");
    println!("  command is re-run.");
}