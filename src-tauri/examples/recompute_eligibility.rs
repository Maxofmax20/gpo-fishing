//! Recompute `training_eligible` for existing reviews, from the EFFECTIVE
//! knowledge base (compiled bundled base + persisted `knowledge_custom.json`
//! overlay, merged through the validated import pipeline).
//!
//! # Why this exists
//!
//! The first version of `label_review::confirm_batch` built its eligibility with
//! `EligibilityInput::library_default`, whose documented contract is "for pure
//! library callers that have no KB access" and which treats any non-empty name as
//! canonical. Used there it marked 1,790 rows `training_eligible: true` for fish
//! that do not exist in the knowledge base, while the screen told the reviewer the
//! opposite. That bug is fixed and pinned by a test; this tool repairs or refreshes
//! the derived flag whenever KB membership changes - most notably after the fish
//! KB patch (`examples/apply_kb_patch.rs`) adds the human-confirmed entities.
//!
//! # What it touches - and what it never touches
//!
//! It recomputes ONLY `training_eligible` / `excluded_reason`, through
//! [`recompute_row_eligibility`], which writes an explicit
//! `ELIGIBILITY_RECOMPUTED` audit event per changed row. The human judgements -
//! verdict, status, attribution, `reviewed_at` - are not modified: a mechanical
//! recompute is not a new human review and must not be timestamped as one.
//!
//! Rows without a confirmed human verdict (unreviewed, skipped, conflict) are
//! never touched: a proposal alone is never eligible, and a conflict stays out
//! until a human resolves it.
//!
//! Usage:
//!   cargo run --example recompute_eligibility            # report only
//!   cargo run --example recompute_eligibility -- --apply  # write through the store

use gpo_autofish_lib::config::Store as ConfigStore;
use gpo_autofish_lib::core::review::{recompute_row_eligibility, EligibilityInput, ReviewStore};
use std::collections::HashSet;

fn main() {
    let apply = std::env::args().any(|a| a == "--apply");
    let data_dir = gpo_autofish_lib::app::data_dir();

    // The SAME effective KB the app uses: bundled + persisted overlay. Reading a
    // stale export instead is how eligibility and reality drifted apart before.
    let kb = ConfigStore::new(data_dir.clone()).effective_knowledge();
    let kb_ids: HashSet<String> = kb.entities().iter().map(|e| e.id.clone()).collect();
    if kb_ids.is_empty() {
        eprintln!("  the effective knowledge base is EMPTY - refusing to recompute,");
        eprintln!("  because that would mark every row ineligible for the wrong reason.");
        std::process::exit(1);
    }

    let store = ReviewStore::new(data_dir.clone());
    let records = store.list();
    if records.is_empty() {
        eprintln!("  no reviews in {}", data_dir.display());
        std::process::exit(1);
    }
    println!("  {} review(s) loaded", records.len());
    println!("  effective knowledge base: {} entities", kb_ids.len());

    // Dry-run accounting first: what WOULD change, per entity.
    let mut would_promote = 0usize;
    let mut would_demote = 0usize;
    let mut unchanged = 0usize;
    let mut untouched = 0usize; // unreviewed / skipped / conflict
    for r in &records {
        let eid = r.human_entity_id.clone().unwrap_or_default();
        let input = EligibilityInput {
            png_decodable: true, // measured at review time; not re-decoded here
            canonical_valid: !eid.is_empty() && kb_ids.contains(&eid),
            is_duplicate: false,
            has_provenance: !r.session_id.is_empty(),
            has_entity: !eid.is_empty(),
        };
        if !gpo_autofish_lib::core::review::is_reviewed(r.review_status) {
            untouched += 1;
            continue;
        }
        let (eligible, _) = gpo_autofish_lib::core::review::evaluate_input(&input);
        if eligible == r.training_eligible {
            unchanged += 1;
        } else if eligible {
            would_promote += 1;
        } else {
            would_demote += 1;
        }
    }

    println!();
    println!("  would become eligible   : {would_promote}");
    println!("  would become ineligible : {would_demote}");
    println!("  already correct         : {unchanged}");
    println!("  untouched (no verdict)  : {untouched}");

    if !apply {
        println!();
        println!("  report only. Re-run with --apply to write through the review store.");
        return;
    }

    let mut promoted = 0usize;
    let mut demoted = 0usize;
    let mut failed = 0usize;
    for r in &records {
        let eid = r.human_entity_id.clone().unwrap_or_default();
        let input = EligibilityInput {
            png_decodable: true,
            canonical_valid: !eid.is_empty() && kb_ids.contains(&eid),
            is_duplicate: false,
            has_provenance: !r.session_id.is_empty(),
            has_entity: !eid.is_empty(),
        };
        match recompute_row_eligibility(&store, &r.image_id, input) {
            Ok(Some(change)) => {
                if change.now_eligible {
                    promoted += 1;
                } else {
                    demoted += 1;
                }
            }
            Ok(None) => {}
            Err(_) => failed += 1,
        }
    }
    println!();
    println!("  applied: {promoted} promoted, {demoted} demoted, {failed} failed");
    println!("  every change is an ELIGIBILITY_RECOMPUTED audit event; the human");
    println!("  judgements, attribution and timestamps are exactly as they were.");
}
