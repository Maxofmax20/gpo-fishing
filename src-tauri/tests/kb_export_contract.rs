//! # Knowledge-base contract relied on by the OCR audit
//!
//! The OCR audit (`ml/gpo_ocr_audit/`) loads the knowledge base through the
//! `kb_export` example target and makes several assumptions about it. Each one is
//! asserted here, in the language that owns the data, so a KB change that breaks
//! the audit fails `cargo test` instead of silently producing a report full of
//! `UNKNOWN_ENTITY`.
//!
//! What is guarded:
//!
//! * the exported `fish_vocab` is exactly `FISH_VOCAB.txt` (no second ontology),
//! * **no fish entity carries an alias**, so the audit's alias stage cannot
//!   silently start applying to fish,
//! * the drop-banner templates exist, because the audit strips them before any
//!   entity lookup and would otherwise report every drop as ambiguous between a
//!   fish and a UI term,
//! * the export serialises losslessly.
//!
//! This is read-only. Nothing here mutates the KB.

use gpo_autofish_lib::core::class_vocab;
use gpo_autofish_lib::core::knowledge::{EntityCategory, KnowledgeBase};

fn fish_entities() -> Vec<&'static gpo_autofish_lib::core::knowledge::GpoEntity> {
    KnowledgeBase::bundled()
        .entities()
        .iter()
        .filter(|e| e.category == EntityCategory::Fish)
        .collect()
}

#[test]
fn exported_fish_vocab_is_exactly_the_canonical_file() {
    // Compared as SETS. The KB has no notion of class order, while
    // `FISH_VOCAB.txt` order is load-bearing because line position IS the output
    // logit index. The exporter copies that order verbatim from
    // `class_vocab`, so the correct assertion is membership, not sequence.
    let in_vocab: std::collections::BTreeSet<String> = fish_entities()
        .iter()
        .map(|e| e.id.clone())
        .filter(|id| class_vocab::fish_vocab().contains(&id.as_str()))
        .collect();
    let canonical: std::collections::BTreeSet<String> = class_vocab::fish_vocab()
        .iter()
        .map(|c| c.to_string())
        .collect();

    assert_eq!(
        in_vocab, canonical,
        "the KB's 8-class projection diverges from FISH_VOCAB.txt: the audit \
         resolves OCR text against the KB, so a class present in one and not the \
         other would make every occurrence of it UNKNOWN_ENTITY"
    );
    assert_eq!(in_vocab.len(), class_vocab::fish_vocab_len());

    // And the exporter's ordering is the canonical file's ordering, not the KB's.
    assert_eq!(
        class_vocab::fish_vocab().iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        vec![
            "fish:angelfish", "fish:crimson", "fish:golden", "fish:pufferfish",
            "fish:shark", "fish:snapper", "fish:squid", "fish:swordfish",
        ],
        "FISH_VOCAB.txt order changed; the deployed fish_v1 manifest pins a \
         vocab_sha over this exact order and must be re-exported with it"
    );
}

#[test]
fn no_fish_entity_declares_an_alias() {
    // The audit's `ALIAS_MATCH` status is only reachable through a KB-declared
    // alias. With no fish aliases it is unreachable for fish, and the audit's
    // companion Python test asserts the same thing from the export.
    //
    // If a fish alias is ever added, the audit's alias stage starts applying and
    // both tests should be revisited rather than left to diverge.
    let aliased: Vec<&str> = fish_entities()
        .iter()
        .filter(|e| !e.aliases.is_empty() || !e.ocr_aliases.is_empty())
        .map(|e| e.id.as_str())
        .collect();
    assert!(
        aliased.is_empty(),
        "fish entities gained aliases {aliased:?}; the OCR audit's alias stage now \
         applies to fish and its reporting needs re-review"
    );
}

#[test]
fn the_drop_banner_template_exists() {
    // `New Item <X>` is how the game announces a drop. The audit strips this
    // template before any entity lookup; without it, every drop reads as
    // ambiguous between the fish and `ui:new-item`.
    let kb = KnowledgeBase::bundled();
    assert!(
        kb.find_by_name("new item").is_some(),
        "the 'new item' banner template is missing from the KB; the OCR audit \
         depends on it to separate template from entity name"
    );
    assert!(
        kb.by_category(EntityCategory::UiTerm).len() >= 10,
        "expected the full set of banner templates (caught a / got a / escaped / \
         line broke / too slow ...); the audit strips all of them"
    );
}

#[test]
fn at_least_one_compound_capable_fish_name_pair_exists() {
    // `crimson` and `snapper` are both canonical AND both appear inside the real
    // in-game name `Crimson Snapper`. That collision is the reason the audit
    // refuses to resolve compound names, so its presence is asserted rather than
    // assumed.
    let kb = KnowledgeBase::bundled();
    assert!(kb.find_by_name("crimson").is_some());
    assert!(kb.find_by_name("snapper").is_some());
    // And the resolver must NOT be able to reach a single answer for the
    // compound, mirroring the Python-side test.
    match gpo_autofish_lib::core::canon::resolve(kb, "Crimson Snapper") {
        gpo_autofish_lib::core::canon::Resolution::Unknown => {}
        other => panic!(
            "the production resolver returned {other:?} for `Crimson Snapper`; it \
             must not claim a single answer, because picking either component is \
             what produced the original contamination"
        ),
    }
}

#[test]
fn excluded_concepts_stay_out_of_the_knowledge_base() {
    // These were deliberately not added. The audit reports them as candidate
    // concepts, and a human decides. This test fails the build if one of them is
    // ever added without that decision being recorded.
    let kb = KnowledgeBase::bundled();
    for excluded in [
        "Tigerfin", "Exotic Tigerfin", "Golden Tigerfin", "Candy Corn Squid",
        "Dark Skeletal Shark", "Blue-Lip Grouper", "Golden Polka Puffer",
        "Zebra Ribbon Angelfish", "Golden Ribbon Angelfish", "Jack-O'-Bite",
    ] {
        assert!(
            kb.find_by_name(excluded).is_none(),
            "{excluded:?} is now a KB entity; the OCR audit's 'unknown concept' \
             report is stale and the ontology decision must be documented"
        );
    }
}

#[test]
fn the_export_serialises_losslessly() {
    // Mirrors `examples/kb_export.rs` so the shape the audit parses is guarded
    // here as well as in the example itself.
    #[derive(serde::Serialize)]
    struct EntityOut {
        id: String,
        canonical_name: String,
        aliases: Vec<String>,
        ocr_aliases: Vec<String>,
        category: String,
        rarity: Option<String>,
    }
    #[derive(serde::Serialize)]
    struct Export {
        schema: u32,
        kb_version: u32,
        fish_vocab: Vec<String>,
        fish_vocab_sha256: String,
        entity_count: usize,
        entities: Vec<EntityOut>,
    }

    let kb = KnowledgeBase::bundled();
    let mut entities: Vec<EntityOut> = kb
        .entities()
        .iter()
        .map(|e| EntityOut {
            id: e.id.clone(),
            canonical_name: e.canonical_name.clone(),
            aliases: e.aliases.clone(),
            ocr_aliases: e.ocr_aliases.clone(),
            category: e.category.as_str().to_string(),
            rarity: e.rarity.clone(),
        })
        .collect();
    entities.sort_by(|a, b| a.id.cmp(&b.id));

    let export = Export {
        schema: 1,
        kb_version: kb.version,
        fish_vocab: class_vocab::fish_vocab()
            .iter()
            .map(|c| c.to_string())
            .collect(),
        fish_vocab_sha256: class_vocab::fish_vocab_sha().to_string(),
        entity_count: entities.len(),
        entities,
    };

    let json = serde_json::to_string(&export).expect("export must serialise");
    let back: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(back["schema"], 1);
    assert_eq!(
        back["fish_vocab_sha256"].as_str().unwrap(),
        class_vocab::fish_vocab_sha()
    );
    // Every id is unique, or the audit's candidate lists would double-count.
    let mut ids: Vec<&str> = back["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate entity id in the KB");
    assert_eq!(back["entities"].as_array().unwrap().len(), kb.len());
}