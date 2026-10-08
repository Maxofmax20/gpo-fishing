//! Cross-language drift guard: the Rust resolver and the Python-facing KB
//! export must agree on what a documented fish name resolves to.
//!
//! `kb_export.json` is what the Python ontology matcher reads; the effective
//! Rust KB is what `correlate_text` reads. This test fails if the two ever
//! disagree about the id a canonical fish name resolves to - which is exactly
//! how the 8-class stem contamination existed for months: two implementations
//! of "the same" ontology, quietly different.

#![cfg(windows)]

use gpo_autofish_lib::core::knowledge::{ImportEntity, KnowledgeBase};
use gpo_autofish_lib::core::perception::{match_entities, ScreenKind};

fn find_export() -> Option<std::path::PathBuf> {
    let rel = std::path::Path::new("ml").join("gpo_ocr_audit").join("kb_export.json");
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

/// The effective KB as the app computes it: bundled + every exported entity
/// the bundled base lacks (the persisted overlay entities), through the same
/// validated merge path. Returns the KB and the ids that were added (the 15
/// confirmed species), so the test knows exactly which ids are species.
fn effective_kb(export: &serde_json::Value) -> (KnowledgeBase, std::collections::HashSet<String>) {
    let mut kb = KnowledgeBase::bundled().clone();
    let have: std::collections::HashSet<String> =
        kb.entities().iter().map(|e| e.id.clone()).collect();
    let imports: Vec<ImportEntity> = export["entities"]
        .as_array()
        .expect("export entities")
        .iter()
        .filter(|e| !have.contains(e["id"].as_str().unwrap_or("")))
        .map(|e| ImportEntity {
            id: e["id"].as_str().unwrap().to_string(),
            canonical_name: e["canonical_name"].as_str().unwrap().to_string(),
            aliases: e["aliases"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            category: e["category"].as_str().unwrap().to_string(),
            rarity: e["rarity"].as_str().map(str::to_string),
            description: None,
            ocr_aliases: e["ocr_aliases"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            visual_refs: vec![],
            provenance: gpo_autofish_lib::core::knowledge::EntityProvenance {
                source: "kb_export.json".to_string(),
                url: Some("https://grand-piece-online.fandom.com/wiki/Fishing/Bestiary".to_string()),
                updated: None,
            },
        })
        .collect();
    let added: std::collections::HashSet<String> =
        imports.iter().map(|i| i.id.clone()).collect();
    let report = kb.merge_import(&imports, false);
    assert_eq!(report.added, 15, "the export must carry the 15 overlay species: {report:?}");
    (kb, added)
}

#[test]
fn every_documented_fish_name_resolves_to_its_export_id() {
    let Some(path) = find_export() else {
        eprintln!("SKIP: kb_export.json not found");
        return;
    };
    let export: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let (kb, mut species) = effective_kb(&export);
    // The 16 documented fish = the 15 overlay species + bundled fish:swordfish.
    species.insert("fish:swordfish".to_string());

    let mut checked = 0usize;
    for e in export["entities"].as_array().unwrap() {
        let id = e["id"].as_str().unwrap();
        let name = e["canonical_name"].as_str().unwrap();
        if !species.contains(id) {
            continue;
        }
        let candidates = match_entities(&kb, name, ScreenKind::Fishing, 0.85);
        if name.contains(' ') {
            // A multi-word species resolves as a PHRASE: exactly one identity,
            // and no contained stem may compete.
            assert_eq!(
                candidates.len(),
                1,
                "{name} must resolve to exactly one identity, got {:?}",
                candidates.iter().map(|c| &c.entity_id).collect::<Vec<_>>()
            );
        } else {
            // A single-word species resolves through the per-word stage, which
            // legitimately lists up to 5 candidates; the top one confirms.
            assert!(!candidates.is_empty(), "{name} produced no candidates");
        }
        assert_eq!(candidates[0].entity_id, id, "{name} resolved to the wrong entity");
        checked += 1;
    }
    assert_eq!(checked, 16, "all 16 documented fish must be checked");

    // And the guard the whole fix exists for: a species name must never offer
    // a legacy stem as a competing candidate.
    let candidates = match_entities(&kb, "Crimson Snapper", ScreenKind::Fishing, 0.85);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].entity_id, "fish:crimsonsnapper");
    assert!(!candidates.iter().any(|c| c.entity_id == "fish:crimson"));
    assert!(!candidates.iter().any(|c| c.entity_id == "fish:snapper"));
}
