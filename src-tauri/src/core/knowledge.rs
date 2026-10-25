//! Local structured GPO knowledge base.
//!
//! The bot's perception must not depend on raw OCR strings alone. This module
//! owns a curated, versioned table of Grand Piece Online entities (fruits,
//! fish, bait tiers, UI terms) seeded from the same data the matcher already
//! trusts (`core::fruit`), plus a validated import pipeline for external
//! sources (e.g. a MediaWiki API):
//!
//! ```text
//! GPO Knowledge Source
//!         ↓
//! Normalizer (ImportEntity)
//!         ↓
//! Validator (validate_import)
//!         ↓
//! Local Knowledge DB (merge_import, provenance-preserving)
//!         ↓
//! Perception Matcher (match_text)
//! ```
//!
//! Rules: every entity carries provenance; curated `bundled` entries are
//! never silently overwritten by imports; malformed/duplicate remote data is
//! reported, not merged. No network access happens in unit tests.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::core::fruit::{self, KNOWN_FISH};

pub const KNOWLEDGE_VERSION: u32 = 1;
pub const BUNDLED_SOURCE: &str = "bundled:core::fruit/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityCategory {
    Fish,
    Fruit,
    Bait,
    Item,
    UiTerm,
    Location,
    Npc,
    Weapon,
    Unknown,
}

impl EntityCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            EntityCategory::Fish => "fish",
            EntityCategory::Fruit => "fruit",
            EntityCategory::Bait => "bait",
            EntityCategory::Item => "item",
            EntityCategory::UiTerm => "ui_term",
            EntityCategory::Location => "location",
            EntityCategory::Npc => "npc",
            EntityCategory::Weapon => "weapon",
            EntityCategory::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "fish" => EntityCategory::Fish,
            "fruit" => EntityCategory::Fruit,
            "bait" => EntityCategory::Bait,
            "item" => EntityCategory::Item,
            "ui_term" | "uiterm" | "ui" => EntityCategory::UiTerm,
            "location" => EntityCategory::Location,
            "npc" => EntityCategory::Npc,
            "weapon" => EntityCategory::Weapon,
            _ => EntityCategory::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityProvenance {
    pub source: String,
    pub url: Option<String>,
    pub updated: Option<String>,
}

impl EntityProvenance {
    pub fn bundled() -> Self {
        Self { source: BUNDLED_SOURCE.to_string(), url: None, updated: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpoEntity {
    pub id: String,
    pub canonical_name: String,
    pub aliases: Vec<String>,
    pub category: EntityCategory,
    pub rarity: Option<String>,
    pub description: Option<String>,
    pub ocr_aliases: Vec<String>,
    pub visual_refs: Vec<String>,
    pub related: Vec<String>,
    pub provenance: EntityProvenance,
}

/// One normalized record from an external source, awaiting validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportEntity {
    pub id: String,
    pub canonical_name: String,
    pub aliases: Vec<String>,
    pub category: String,
    pub rarity: Option<String>,
    pub description: Option<String>,
    pub ocr_aliases: Vec<String>,
    pub visual_refs: Vec<String>,
    pub provenance: EntityProvenance,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MergeReport {
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct KnowledgeBase {
    pub version: u32,
    entities: Vec<GpoEntity>,
    /// Normalized name/alias → entity index (first wins on collision).
    index: HashMap<String, usize>,
}

fn norm_key(s: &str) -> String {
    fruit::normalize(s)
}

impl KnowledgeBase {
    pub fn bundled() -> &'static Self {
        static BUNDLED: LazyLock<KnowledgeBase> = LazyLock::new(KnowledgeBase::build_bundled);
        &BUNDLED
    }

    pub fn entities(&self) -> &[GpoEntity] {
        &self.entities
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn find_by_name(&self, name: &str) -> Option<&GpoEntity> {
        self.index.get(&norm_key(name)).map(|&i| &self.entities[i])
    }

    pub fn by_category(&self, category: EntityCategory) -> Vec<&GpoEntity> {
        self.entities.iter().filter(|e| e.category == category).collect()
    }

    fn push(&mut self, e: GpoEntity) {
        let idx = self.entities.len();
        for key in std::iter::once(&e.canonical_name)
            .chain(e.aliases.iter())
            .chain(e.ocr_aliases.iter())
        {
            let k = norm_key(key);
            if !k.is_empty() {
                self.index.entry(k).or_insert(idx);
            }
        }
        self.entities.push(e);
    }

    /// Validate one imported record. Returns the entity on success or a
    /// human-readable reason on failure (reported, never merged).
    pub fn validate_import(item: &ImportEntity) -> Result<GpoEntity, String> {
        let id = item.id.trim();
        if id.is_empty() {
            return Err("import entity has empty id".to_string());
        }
        if id.len() > 128 {
            return Err(format!("import entity id too long: {id}"));
        }
        let name = item.canonical_name.trim();
        if name.is_empty() {
            return Err(format!("import entity '{id}' has empty canonical_name"));
        }
        let category = EntityCategory::parse(&item.category);
        if category == EntityCategory::Unknown {
            return Err(format!("import entity '{id}' has unknown category '{}'", item.category));
        }
        if item.provenance.source.trim().is_empty() {
            return Err(format!("import entity '{id}' has no provenance source"));
        }
        if let Some(url) = &item.provenance.url {
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(format!("import entity '{id}' has invalid provenance url"));
            }
        }
        Ok(GpoEntity {
            id: id.to_string(),
            canonical_name: name.to_string(),
            aliases: dedup_names(&item.aliases),
            category,
            rarity: item.rarity.clone().map(|r| r.trim().to_string()).filter(|r| !r.is_empty()),
            description: item.description.clone().map(|d| d.trim().to_string()).filter(|d| !d.is_empty()),
            ocr_aliases: dedup_names(&item.ocr_aliases),
            visual_refs: item.visual_refs.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            related: Vec::new(),
            provenance: EntityProvenance {
                source: item.provenance.source.trim().to_string(),
                url: item.provenance.url.clone(),
                updated: item.provenance.updated.clone(),
            },
        })
    }

    /// Merge validated imports. Curated `bundled` entries are never
    /// overwritten unless `overwrite_bundled` is true; same-id imports with
    /// a different canonical name are skipped (renames need a curator).
    pub fn merge_import(&mut self, items: &[ImportEntity], overwrite_bundled: bool) -> MergeReport {
        let mut report = MergeReport::default();
        let mut seen_ids = std::collections::HashSet::new();
        for item in items {
            if !seen_ids.insert(item.id.trim().to_string()) {
                report.skipped += 1;
                report.errors.push(format!("duplicate import id '{}' in same batch", item.id.trim()));
                continue;
            }
            match Self::validate_import(item) {
                Err(e) => {
                    report.skipped += 1;
                    report.errors.push(e);
                }
                Ok(entity) => {
                    if let Some(pos) = self.entities.iter().position(|e| e.id == entity.id) {
                        let existing = &self.entities[pos];
                        let bundled = existing.provenance.source == BUNDLED_SOURCE;
                        if bundled && !overwrite_bundled {
                            report.skipped += 1;
                            continue;
                        }
                        if existing.canonical_name != entity.canonical_name {
                            report.skipped += 1;
                            report.errors.push(format!(
                                "rename of '{}' from '{}' to '{}' requires curator review",
                                entity.id, existing.canonical_name, entity.canonical_name
                            ));
                            continue;
                        }
                        // Re-index (aliases may have grown).
                        self.entities[pos] = entity;
                        self.reindex();
                        report.updated += 1;
                    } else {
                        self.push(entity);
                        report.added += 1;
                    }
                }
            }
        }
        report
    }

    fn reindex(&mut self) {
        self.index.clear();
        for (idx, e) in self.entities.iter().enumerate() {
            for key in std::iter::once(&e.canonical_name)
                .chain(e.aliases.iter())
                .chain(e.ocr_aliases.iter())
            {
                let k = norm_key(key);
                if !k.is_empty() {
                    self.index.entry(k).or_insert(idx);
                }
            }
        }
    }

    fn build_bundled() -> Self {
        let mut kb = KnowledgeBase { version: KNOWLEDGE_VERSION, entities: Vec::new(), index: HashMap::new() };

        // Fruits: canonical names from the trusted fruit list, rarity from
        // the battle-tested classifier, aliases only for unambiguous pairs.
        for name in fruit::DEFAULT_FRUITS {
            let rarity = fruit::fruit_rarity(name).as_str().to_string();
            let aliases = FRUIT_ALIASES
                .iter()
                .find(|(a, _)| a.eq_ignore_ascii_case(name))
                .map(|(_, rest)| rest.iter().map(|s| s.to_string()).collect())
                .unwrap_or_default();
            // Also index reverse: if this name appears as an alias target of
            // another canonical entry, both directions resolve.
            kb.push(GpoEntity {
                id: format!("fruit:{}", name.to_ascii_lowercase()),
                canonical_name: name.to_string(),
                aliases,
                category: EntityCategory::Fruit,
                rarity: Some(rarity),
                description: None,
                ocr_aliases: Vec::new(),
                visual_refs: Vec::new(),
                related: Vec::new(),
                provenance: EntityProvenance::bundled(),
            });
        }
        // Reverse alias entries so alias-first lookups still resolve.
        for (canonical, rest) in FRUIT_ALIASES {
            if kb.find_by_name(canonical).is_some() {
                continue;
            }
            let first = rest.first().unwrap_or(canonical);
            kb.push(GpoEntity {
                id: format!("fruit:{}", canonical.to_ascii_lowercase()),
                canonical_name: first.to_string(),
                aliases: vec![canonical.to_string()],
                category: EntityCategory::Fruit,
                rarity: Some(fruit::fruit_rarity(first).as_str().to_string()),
                description: None,
                ocr_aliases: Vec::new(),
                visual_refs: Vec::new(),
                related: Vec::new(),
                provenance: EntityProvenance::bundled(),
            });
        }

        // Fish from the trusted fish list.
        for name in KNOWN_FISH {
            kb.push(GpoEntity {
                id: format!("fish:{}", name.to_ascii_lowercase().replace(' ', "-")),
                canonical_name: capitalize(name),
                aliases: Vec::new(),
                category: EntityCategory::Fish,
                rarity: None,
                description: None,
                ocr_aliases: Vec::new(),
                visual_refs: Vec::new(),
                related: Vec::new(),
                provenance: EntityProvenance::bundled(),
            });
        }

        // Bait tiers (generic menu labels).
        for (id, name) in [("common", "Common Bait"), ("rare", "Rare Bait"), ("legendary", "Legendary Bait")] {
            kb.push(GpoEntity {
                id: format!("bait:{id}"),
                canonical_name: name.to_string(),
                aliases: vec![id.to_string()],
                category: EntityCategory::Bait,
                rarity: Some(capitalize(id)),
                description: None,
                ocr_aliases: Vec::new(),
                visual_refs: Vec::new(),
                related: Vec::new(),
                provenance: EntityProvenance::bundled(),
            });
        }

        // UI terminology the bot already keys on (drop/catch/fail/spawn).
        for phrase in fruit::DEFAULT_DROP_PHRASES
            .iter()
            .chain(fruit::DEFAULT_CATCH_PHRASES.iter())
            .chain(fruit::DEFAULT_FAIL_PHRASES.iter())
            .chain(["spawned", "spawn"].iter())
        {
            kb.push(GpoEntity {
                id: format!("ui:{}", phrase.to_ascii_lowercase().replace(' ', "-")),
                canonical_name: phrase.to_string(),
                aliases: Vec::new(),
                category: EntityCategory::UiTerm,
                rarity: None,
                description: None,
                ocr_aliases: Vec::new(),
                visual_refs: Vec::new(),
                related: Vec::new(),
                provenance: EntityProvenance::bundled(),
            });
        }

        kb
    }
}

/// Unambiguous same-fruit pairs harvested from the rarity classifier arms
/// (both names classify identically there). Format: (canonical, [aliases]).
/// Only pairs with a shared in-game identity are listed — same-rarity alone
/// is NOT enough to alias two fruits together.
const FRUIT_ALIASES: &[(&str, &[&str])] = &[
    ("Tori", &["Phoenix"]),
    ("Venom", &["Doku"]),
    ("Buddha", &["Daibutsu"]),
    ("Pteranodon", &["Ptera"]),
    ("Soru", &["Soul"]),
    ("T-Rex", &["Trex"]),
    ("Pika", &["Glint"]),
    ("Magu", &["Magma"]),
    ("Hie", &["Ice"]),
    ("Goro", &["Rumble"]),
    ("Mera", &["Flame", "Fire"]),
    ("Suna", &["Sand"]),
    ("Yami", &["Dark", "Darkness"]),
    ("Yuki", &["Snow"]),
    ("Moku", &["Smoke"]),
    ("Gura", &["Tremor", "Quake"]),
    ("Zushi", &["Gravity"]),
    ("Paw", &["Nikyu"]),
    ("Ito", &["String"]),
    ("Kage", &["Shadow"]),
    ("Goru", &["Gold"]),
    ("Bisu", &["Biscuit"]),
    ("Gas", &["Gasu"]),
    ("Yomi", &["Revive"]),
    ("Kira", &["Diamond"]),
    ("Gomu", &["Rubber"]),
    ("Bomb", &["Bomu"]),
    ("Bari", &["Barrier"]),
    ("Mero", &["Love"]),
    ("Horo", &["Hollow"]),
    ("Kilo", &["Weight"]),
    ("Suke", &["Clear"]),
    ("Heal", &["Chiyu"]),
];

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

fn dedup_names(names: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for n in names {
        let t = n.trim();
        if t.is_empty() {
            continue;
        }
        if seen.insert(t.to_ascii_lowercase()) {
            out.push(t.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_covers_fruits_fish_and_ui_terms() {
        let kb = KnowledgeBase::bundled();
        assert!(kb.len() > fruit::DEFAULT_FRUITS.len() + KNOWN_FISH.len());
        assert!(!kb.by_category(EntityCategory::Fruit).is_empty());
        assert!(!kb.by_category(EntityCategory::Fish).is_empty());
        assert!(!kb.by_category(EntityCategory::UiTerm).is_empty());
        assert!(!kb.by_category(EntityCategory::Bait).is_empty());
    }

    #[test]
    fn alias_lookup_resolves_both_directions() {
        let kb = KnowledgeBase::bundled();
        let a = kb.find_by_name("phoenix").expect("phoenix");
        let b = kb.find_by_name("Tori").expect("tori");
        assert_eq!(a.id, b.id);
        assert_eq!(a.rarity.as_deref(), Some("Mythical"));
        assert_eq!(kb.find_by_name("sand").unwrap().id, kb.find_by_name("Suna").unwrap().id);
    }

    #[test]
    fn validation_rejects_malformed_imports() {
        let bad = ImportEntity {
            id: "".into(),
            canonical_name: "X".into(),
            aliases: vec![],
            category: "fruit".into(),
            rarity: None,
            description: None,
            ocr_aliases: vec![],            visual_refs: vec![],            provenance: EntityProvenance { source: "wiki:test".into(), url: None, updated: None },
        };
        assert!(KnowledgeBase::validate_import(&bad).is_err());
        let bad_cat = ImportEntity { id: "x".into(), category: "starship".into(), ..bad.clone() };
        assert!(KnowledgeBase::validate_import(&bad_cat).is_err());
        let bad_url = ImportEntity {
            id: "x".into(),
            canonical_name: "X".into(),
            category: "fish".into(),
            provenance: EntityProvenance { source: "wiki:test".into(), url: Some("ftp://x".into()), updated: None },
            ..bad.clone()
        };
        assert!(KnowledgeBase::validate_import(&bad_url).is_err());
    }

    #[test]
    fn merge_never_silently_overwrites_bundled() {
        let mut kb = KnowledgeBase::bundled().clone();
        let before = kb.find_by_name("Suna").unwrap().rarity.clone();
        let evil = ImportEntity {
            id: "fruit:suna".into(),
            canonical_name: "Suna".into(),
            aliases: vec![],
            category: "fruit".into(),
            rarity: Some("Common".into()),
            description: None,
            ocr_aliases: vec![],            visual_refs: vec![],            provenance: EntityProvenance { source: "wiki:test".into(), url: None, updated: None },
        };
        let rep = kb.merge_import(std::slice::from_ref(&evil), false);
        assert_eq!(rep.updated, 0);
        assert_eq!(kb.find_by_name("Suna").unwrap().rarity, before);
        // Explicit curator override works and keeps new provenance.
        let rep2 = kb.merge_import(std::slice::from_ref(&evil), true);
        assert_eq!(rep2.updated, 1);
        assert_eq!(kb.find_by_name("Suna").unwrap().provenance.source, "wiki:test");
    }

    #[test]
    fn merge_rejects_renames_and_reports_duplicates() {
        let mut kb = KnowledgeBase::bundled().clone();
        let rename = ImportEntity {
            id: "fruit:suna".into(),
            canonical_name: "Totally Different".into(),
            aliases: vec![],
            category: "fruit".into(),
            rarity: None,
            description: None,
            ocr_aliases: vec![],            visual_refs: vec![],            provenance: EntityProvenance { source: "wiki:test".into(), url: None, updated: None },
        };
        let rep = kb.merge_import(&[rename.clone(), rename], true);
        assert_eq!(rep.added, 0);
        assert!(rep.errors.iter().any(|e| e.contains("rename")));
        assert!(rep.errors.iter().any(|e| e.contains("duplicate")));
        assert_eq!(kb.find_by_name("Suna").unwrap().canonical_name, "Suna");
    }

    #[test]
    fn merge_adds_genuinely_new_entities_with_provenance() {
        let mut kb = KnowledgeBase::bundled().clone();
        let n0 = kb.len();
        let item = ImportEntity {
            id: "location:rose-kingdom".into(),
            canonical_name: "Rose Kingdom".into(),
            aliases: vec!["Rose".into()],
            category: "location".into(),
            rarity: None,
            description: Some("Second sea starter island".into()),
            ocr_aliases: vec![],            visual_refs: vec![],            provenance: EntityProvenance {
                source: "wiki:test".into(),
                url: Some("https://example.test/wiki/Rose_Kingdom".into()),
                updated: Some("2026-01-01".into()),
            },
        };
        let rep = kb.merge_import(std::slice::from_ref(&item), false);
        assert_eq!(rep.added, 1);
        assert_eq!(kb.len(), n0 + 1);
        assert_eq!(kb.find_by_name("rose").unwrap().id, "location:rose-kingdom");
    }
}
