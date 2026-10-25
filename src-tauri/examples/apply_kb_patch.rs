//! Apply a reviewed fish KB patch through the repository's canonical mutation
//! path. All logic lives in [`core::kb_patch`]; this is the thin CLI over it.
//!
//! ```text
//! cargo run --example apply_kb_patch              # validate + dry run
//! cargo run --example apply_kb_patch -- --apply   # write the overlay
//! cargo run --example apply_kb_patch -- --patch <path> [--apply]
//! ```

use gpo_autofish_lib::app::data_dir;
use gpo_autofish_lib::config::Store as ConfigStore;
use gpo_autofish_lib::core::kb_patch;
use gpo_autofish_lib::core::ml_model::sha256_hex;
use std::path::PathBuf;

fn find_patch() -> Option<PathBuf> {
    let rel = PathBuf::from("reports")
        .join("ontology_review")
        .join("fish_kb_patch.json");
    let mut dir = std::env::current_dir().ok()?;
    for _ in 0..8 {
        let candidate = dir.join(&rel);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

fn fail(msg: &str) -> ! {
    eprintln!("  REFUSED: {msg}");
    std::process::exit(1)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let apply = args.iter().any(|a| a == "--apply");
    let patch_path = args
        .windows(2)
        .find(|w| w[0] == "--patch")
        .map(|w| PathBuf::from(&w[1]))
        .or_else(find_patch)
        .unwrap_or_else(|| fail("no patch file found; pass --patch <path>"));

    let raw = std::fs::read_to_string(&patch_path)
        .unwrap_or_else(|e| fail(&format!("cannot read {}: {e}", patch_path.display())));
    let patch_digest = sha256_hex(raw.as_bytes());
    let doc = match kb_patch::parse_patch(&raw) {
        Ok(d) => d,
        Err(e) => fail(&e),
    };

    let dir = data_dir();
    let store = ConfigStore::new(dir.clone());
    let kb = store.effective_knowledge();

    for u in &doc.unresolved_concepts {
        println!(
            "  unresolved concept (reported, NOT created): {}",
            u.entity_id
        );
    }

    // Re-apply is a legitimate no-op, not an error: the patch has been consumed.
    match kb_patch::consumed_state(&doc, &kb) {
        kb_patch::ConsumedState::Consumed => {
            println!();
            println!("  0 mutations: every patch entity is already in the effective KB,");
            println!("  identical. The patch was already applied (idempotent no-op).");
            println!("  KB digest: {}", kb_patch::kb_digest(&kb));
            return;
        }
        kb_patch::ConsumedState::Conflict(e) => fail(&e),
        kb_patch::ConsumedState::Fresh => {}
    }

    let validated = match kb_patch::validate_additions(&doc, &kb) {
        Ok(v) => v,
        Err(errors) => {
            for e in &errors {
                eprintln!("  ERROR: {e}");
            }
            fail(&format!(
                "{} validation error(s); the overlay was not touched",
                errors.len()
            ));
        }
    };

    println!();
    println!(
        "  patch        : {} (sha256 {}…)",
        patch_path.display(),
        &patch_digest[..16]
    );
    println!("  validated    : {} entit(ies) to add", validated.len());
    for r in &doc.entities_reused {
        println!("  reused (already in KB, untouched): {}", r.entity_id);
    }

    if !apply {
        println!();
        println!("  dry run. Re-run with --apply to write the overlay.");
        return;
    }

    match kb_patch::apply_patch(&store, &dir, &validated) {
        Err(e) => fail(&e),
        Ok(out) if out.added == 0 => {
            println!();
            println!(
                "  0 mutations: {} entit(ies) already present, identical (idempotent).",
                out.already_present
            );
            println!("  KB digest: {}", out.post_digest);
        }
        Ok(out) => {
            if let Some(b) = &out.backup {
                println!("  backup       : {}", b.display());
            }
            println!();
            println!(
                "  applied {} entit(ies) through the overlay store",
                out.added
            );
            println!("  KB digest pre : {}", out.pre_digest);
            println!("  KB digest post: {}", out.post_digest);
            println!();
            println!("  next: cargo run --example kb_export -- ml/gpo_ocr_audit/kb_export.json --effective");
            println!("        python -m ml.gpo_ontology_review.fish_kb_patch refresh-workstation");
            println!("        cargo run --example recompute_eligibility -- --apply");
        }
    }
}
