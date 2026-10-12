//! Live name recognition only. A closed-set fish classifier cannot establish
//! catch success, fruit identity, pity, or a storage/drop decision.
use crate::core::{fruit, knowledge::{EntityCategory, KnowledgeBase}};
use crate::core::fruit::Lexicon;

/// Read-only recognition catalog from the already reviewed, committed KB patch.
/// This also makes bundled model names available on a fresh installation without
/// rewriting the user's knowledge overlay or any human-review records.
pub fn with_model_names(mut kb: KnowledgeBase) -> KnowledgeBase {
    static NAMES: std::sync::LazyLock<Vec<crate::core::knowledge::ImportEntity>> = std::sync::LazyLock::new(|| {
        crate::core::kb_patch::parse_patch(include_str!("../../../reports/ontology_review/fish_kb_patch.json"))
            .map(|doc| doc.entities_to_add.into_iter()
                .filter(|entry| entry.human_reviewed_rows > 0 && entry.entity.category == "fish")
                .map(|entry| entry.entity).collect()).unwrap_or_default()
    });
    let missing: Vec<_> = NAMES.iter().filter(|entry| !kb.entities().iter().any(|e| e.id == entry.id)).cloned().collect();
    kb.merge_import(&missing, false);
    kb
}

/// Require independent fish evidence before applying a closed-set prediction.
/// Keep names outside the trained vocabulary (including Anglerfish) from OCR.
pub fn eligible(lex: &Lexicon, raw: &str, classes: &[String], kb: &KnowledgeBase) -> bool {
    if fruit::detect_catch(lex, raw) != fruit::CatchVerdict::Caught { return false; }
    let (kind, name) = fruit::parse_catch_item(lex, raw);
    if kind != "fish" { return false; }
    if let Some(entity) = kb.find_by_name(&name) {
        return entity.category == EntityCategory::Fish && classes.contains(&entity.id);
    }
    // Partial OCR still needs an explicit fish noun in the caught-item scope.
    // A generic New Item banner or a fruit-spawn announcement is insufficient.
    let scoped = fruit::normalize(&fruit::catch_name_text(lex, raw));
    scoped.split_whitespace().any(|w| matches!(w,
        "fish" | "shark" | "squid" | "snapper" | "grouper" | "puffer" | "angelfish"))
}

pub fn prediction_name(kb: &KnowledgeBase, id: &str, confidence: f32) -> Option<String> {
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) { return None; }
    kb.entities().iter().find(|e| e.id == id && e.category == EntityCategory::Fish)
        .map(|e| e.canonical_name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn classes() -> Vec<String> { vec!["fish:darkskeletalshark".into(), "fish:swordfish".into()] }
    #[test]
    fn confirmed_supported_and_partial_fish_use_vision() {
        let lex = Lexicon::default(); let owned = with_model_names(KnowledgeBase::bundled().clone()); let kb = &owned;
        assert!(eligible(&lex, "New Item <Dark Skeletal Shark>", &classes(), kb));
        assert!(eligible(&lex, "You caught a Fish!", &classes(), kb));
        assert!(eligible(&lex, "Horo has spawned at Trading Hub\nNew Item <Dark Skeletal Shark>", &classes(), kb));
    }
    #[test]
    fn fruit_unknown_items_failures_and_unsupported_species_keep_ocr() {
        let lex = Lexicon::default(); let owned = with_model_names(KnowledgeBase::bundled().clone()); let kb = &owned;
        for raw in ["New Item <Pika>", "You fished up a Devil Fruit!", "New Item <Unknown>",
            "Horo has spawned at Trading Hub", "You failed to catch a fish", "New Item <Anglerfish>", ""] {
            assert!(!eligible(&lex, raw, &classes(), kb), "{raw}");
        }
    }
    #[test]
    fn every_deployed_class_resolves_to_a_supported_live_name() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!("../../models/fish_v1.json")).unwrap();
        let classes: Vec<String> = manifest["classes"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        let owned = with_model_names(KnowledgeBase::bundled().clone()); let kb = &owned; let lex = Lexicon::default();
        assert_eq!(classes.len(), 15);
        for id in &classes {
            let name = prediction_name(kb, id, 0.8).expect("model class must resolve to its KB fish name");
            assert!(eligible(&lex, &format!("New Item <{name}>"), &classes, kb), "{id}: {name}");
        }
    }
    #[test]
    fn output_requires_known_fish_and_finite_probability() {
        let owned = with_model_names(KnowledgeBase::bundled().clone()); let kb = &owned;
        assert_eq!(prediction_name(kb, "fish:swordfish", 0.8).as_deref(), Some("Swordfish"));
        for (id, confidence) in [("fruit:pika", 0.9), ("fish:unknown", 0.9), ("fish:swordfish", f32::NAN), ("fish:swordfish", 1.1)] {
            assert!(prediction_name(kb, id, confidence).is_none());
        }
    }
}
