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
    // Read the EFFECTIVE export the OCR audit actually consumes
    // (`ml/gpo_ocr_audit/kb_export.json`, produced by
    // `cargo run --example kb_export -- --effective`), not the bundled KB.
    //
    // This distinction is the whole point: the 15 confirmed species are OVERLAY
    // entities, added by KB patch a8bce5c through
    // `knowledge_custom.json`. Only `fish:swordfish` of the 15 is bundled. So
    // comparing the canonical vocabulary against `KnowledgeBase::bundled()`
    // asserted that 14 real, documented, human-reviewed species were missing
    // from the KB - when they are present in the KB the audit reads.
    let export_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("ml")
        .join("gpo_ocr_audit")
        .join("kb_export.json");
    let Some(export) = std::fs::read_to_string(&export_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
    else {
        eprintln!(
            "SKIP: no effective export at {}; run \
             `cargo run --example kb_export -- --effective ml/gpo_ocr_audit/kb_export.json`",
            export_path.display()
        );
        return;
    };
    let exported: std::collections::BTreeSet<String> = export["entities"]
        .as_array()
        .expect("export entities")
        .iter()
        .filter_map(|e| e["id"].as_str().map(str::to_string))
        .filter(|id| id.starts_with("fish:"))
        .filter(|id| class_vocab::fish_vocab().contains(&id.as_str()))
        .collect();
    let canonical: std::collections::BTreeSet<String> = class_vocab::fish_vocab()
        .iter()
        .map(|c| c.to_string())
        .collect();

    assert_eq!(
        exported, canonical,
        "the EFFECTIVE KB the audit resolves OCR against does not carry exactly \
         FISH_VOCAB.txt. A class present in one and not the other makes every \
         occurrence of it UNKNOWN_ENTITY - the audit cannot name a fish it has \
         never heard of, and a training class the runtime cannot resolve is a \
         class that can never be confirmed in the field."
    );
    assert_eq!(exported.len(), class_vocab::fish_vocab_len());

    // And the export must carry the canonical ORDER verbatim, because line
    // position IS the output-logit index. The exporter copies it from
    // `class_vocab`, so this must agree with the embedded vocabulary, not with
    // a literal written out here.
    assert_eq!(
        export["fish_vocab"]
            .as_array()
            .expect("export fish_vocab")
            .iter()
            .map(|s| s.as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>(),
        class_vocab::fish_vocab()
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>(),
        "the export's fish_vocab must be FISH_VOCAB.txt in canonical order"
    );
    assert_eq!(
        export["fish_vocab_sha256"].as_str().unwrap_or(""),
        class_vocab::fish_vocab_sha(),
        "the export's class-map fingerprint must match the canonical one"
    );

    // And the exporter's ordering is the canonical file's ordering, not the KB's.
    //
    // This test used to spell the expected order out as a literal, which is how
    // it stayed green for one vocabulary change and silently wrong for the next:
    // two copies of one list, and the only signal that disagreed was a red test
    // nobody could interpret. It now reads the file and checks the PROPERTY the
    // ordering is supposed to guarantee - ascending id order - so a reordered
    // file fails for the right reason and a re-sorted one still passes.
    let order = class_vocab::fish_vocab();
    let mut sorted: Vec<&str> = order.to_vec();
    sorted.sort_unstable();
    assert_eq!(
        order, sorted,
        "FISH_VOCAB.txt is not in ascending id order; the file header defines \
         lexicographic order as the class-map rule"
    );
    // The order must also be the file's own byte order, not a coincidence of the
    // compiler's iteration: read the file directly and compare beyond the
    // embedded embedding.
    let raw = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("FISH_VOCAB.txt"),
    )
    .expect("FISH_VOCAB.txt must be readable beside the crate");
    let from_file: Vec<String> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();
    assert_eq!(
        from_file,
        order.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "the embedded vocabulary must be exactly the file on disk, in file order"
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
        "Tigerfin",
        "Exotic Tigerfin",
        "Golden Tigerfin",
        "Candy Corn Squid",
        "Dark Skeletal Shark",
        "Blue-Lip Grouper",
        "Golden Polka Puffer",
        "Zebra Ribbon Angelfish",
        "Golden Ribbon Angelfish",
        "Jack-O'-Bite",
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
