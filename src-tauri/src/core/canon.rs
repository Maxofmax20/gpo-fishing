//! Canonical entity view + resolver built FROM [`KnowledgeBase`].
//!
//! - Never duplicates the KB lists: every view entry is built by iterating
//!   `kb.entities()` and cloning its stable `id` unchanged.
//! - `fishing_drop = true` for `Fish`/`Fruit` (they come from the project's
//!   curated fishing lists: `DEFAULT_FRUITS` / `KNOWN_FISH`). `false` for
//!   `Bait`/`UiTerm` (and any other category).
//!   NOTE: per-state fishing mechanics (rates, pity, locations) are
//!   **unverified** here — this flag only marks list membership, not gameplay.
//! - `seasonal` is always `false`: [`GpoEntity`] carries no seasonal flag, so
//!   we do NOT invent one. If the KB ever gains seasonality, wire it here.
//! - `active` is always `true` for bundled entries.
//! - `wiki_source` clones `entity.provenance.source`.
//! - `wiki_url` clones `entity.provenance.url` (which is `None` for all
//!   bundled entries) — no fake Wiki URLs are ever constructed.
//!
//! # Honesty report — requested fishing-drop names NOT in the KB
//!
//! The following requested names were verified against
//! `KnowledgeBase::bundled()` via `find_by_name` and do **NOT** exist there,
//! therefore they are **NOT added** anywhere by this module:
//!
//! - "Blue-Lip Grouper" — NOT in KB
//! - "Tigerfin" — NOT in KB (KB has separate `tiger` fish token, not `tigerfin`)
//! - "Skeletal Shark" — NOT in KB (KB has `shark`, not `skeletal shark`)
//! - "Sunken Armor" — NOT in KB
//! - "Candy Corn Squid" — NOT in KB (KB has `squid`, not `candy corn squid`)
//!
//! See test `requested_drop_names_are_not_in_kb` which enforces this.

use crate::core::fruit;
use crate::core::knowledge::{EntityCategory, KnowledgeBase};

/// Canonical, UI-safe view of one [`crate::core::knowledge::GpoEntity`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalEntity {
    /// Stable id, cloned unchanged from the KB (e.g. `"fish:golden"`).
    pub entity_id: String,
    pub category: EntityCategory,
    /// True for Fish/Fruit (curated fishing lists). False otherwise.
    /// Per-state mechanics are unverified — membership flag only.
    pub fishing_drop: bool,
    pub canonical_name: String,
    /// All known aliases from the KB (`aliases` + `ocr_aliases`), unchanged.
    pub aliases: Vec<String>,
    pub rarity: Option<String>,
    /// Always false: the KB carries no seasonal flag, so we do not invent one.
    pub seasonal: bool,
    /// Always true for bundled entries.
    pub active: bool,
    /// From `entity.provenance.source`.
    pub wiki_source: String,
    /// From `entity.provenance.url` (`None` for all bundled entries).
    pub wiki_url: Option<String>,
    /// `entity.visual_refs.len()` (0 for all bundled entries).
    pub visual_refs_count: usize,
}

/// True for Fish/Fruit categories (project's curated fishing lists).
/// False for Bait/UiTerm and any other category.
///
/// Per-state fishing mechanics are unverified — this only marks which
/// curated list the entity came from.
pub fn is_fishing_drop(category: EntityCategory) -> bool {
    matches!(category, EntityCategory::Fish | EntityCategory::Fruit)
}

/// Build the canonical view from the passed KB. Never hardcodes entity lists.
pub fn canonical_view(kb: &KnowledgeBase) -> Vec<CanonicalEntity> {
    kb.entities()
        .iter()
        .map(|e| CanonicalEntity {
            entity_id: e.id.clone(),
            category: e.category,
            fishing_drop: is_fishing_drop(e.category),
            canonical_name: e.canonical_name.clone(),
            aliases: e
                .aliases
                .iter()
                .chain(e.ocr_aliases.iter())
                .cloned()
                .collect(),
            rarity: e.rarity.clone(),
            seasonal: false,
            active: true,
            wiki_source: e.provenance.source.clone(),
            wiki_url: e.provenance.url.clone(),
            visual_refs_count: e.visual_refs.len(),
        })
        .collect()
}

/// Resolver outcome. Never invents ids; every id comes from the passed KB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Exact(String),
    Ambiguous(Vec<String>),
    Unknown,
}

/// Resolve user/OCR input against the passed KB.
///
/// - Normalize: lowercase, trim, collapse spaces (via `fruit::normalize`,
///   which also maps non-alphanumerics to spaces — a superset that stays
///   consistent with `KnowledgeBase::find_by_name`).
/// - `Exact(entity_id)` on canonical/alias/ocr-alias match (first-wins, same
///   as the KB index).
/// - `Ambiguous(ids)` when the normalized input is a strict substring of 2+
///   distinct entities' names/aliases AND there was no exact match.
///   Single-substring hits return `Unknown` (we never guess).
/// - `Unknown` otherwise.
///
/// `"skeletal shark"` returns `Exact` ONLY if that exact entity exists in the
/// passed KB; in the current bundled KB it does not, so it yields `Unknown`.
pub fn resolve(kb: &KnowledgeBase, input: &str) -> Resolution {
    let norm = fruit::normalize(input);
    if norm.is_empty() {
        return Resolution::Unknown;
    }
    if let Some(e) = kb.find_by_name(&norm) {
        return Resolution::Exact(e.id.clone());
    }
    let mut hits: Vec<String> = Vec::new();
    for e in kb.entities() {
        let mut matched = false;
        for cand in std::iter::once(&e.canonical_name)
            .chain(e.aliases.iter())
            .chain(e.ocr_aliases.iter())
        {
            let cnorm = fruit::normalize(cand);
            if cnorm.is_empty() {
                continue;
            }
            // Strict substring: longer than the input and contains it.
            if cnorm.len() > norm.len() && cnorm.contains(&norm) {
                matched = true;
                break;
            }
        }
        if matched {
            hits.push(e.id.clone());
        }
    }
    hits.sort();
    hits.dedup();
    if hits.len() >= 2 {
        Resolution::Ambiguous(hits)
    } else {
        // 0 hits, or a single substring hit with no exact match: never guess.
        Resolution::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::knowledge::KnowledgeBase;

    /// Requested names that must NOT be invented. All verified absent.
    const REQUESTED_ABSENT: &[&str] = &[
        "Blue-Lip Grouper",
        "Tigerfin",
        "Skeletal Shark",
        "Sunken Armor",
        "Candy Corn Squid",
    ];

    #[test]
    fn exact_match_on_canonical() {
        let kb = KnowledgeBase::bundled();
        // Fish canonical "Golden" -> stable id "fish:golden".
        assert_eq!(
            resolve(kb, "Golden"),
            Resolution::Exact("fish:golden".to_string())
        );
        assert_eq!(
            resolve(kb, "  golden  "),
            Resolution::Exact("fish:golden".to_string())
        );
        // Fruit canonical.
        let suna = kb.find_by_name("Suna").expect("Suna in KB");
        assert_eq!(
            resolve(kb, "Suna"),
            Resolution::Exact(suna.id.clone())
        );
    }

    #[test]
    fn alias_match_phoenix_to_tori_line() {
        let kb = KnowledgeBase::bundled();
        // FRUIT_ALIASES has ("Tori", &["Phoenix"]); both directions resolve
        // to the same bundled entity "fruit:tori".
        let tori = kb.find_by_name("Tori").expect("Tori in KB");
        assert_eq!(tori.id, "fruit:tori");
        let via_phoenix = kb.find_by_name("phoenix").expect("phoenix alias in KB");
        assert_eq!(via_phoenix.id, tori.id);
        assert_eq!(
            resolve(kb, "Phoenix"),
            Resolution::Exact("fruit:tori".to_string())
        );
        assert_eq!(
            resolve(kb, "phoenix"),
            Resolution::Exact("fruit:tori".to_string())
        );
    }

    #[test]
    fn ambiguous_only_when_two_plus_overlaps_exist() {
        let kb = KnowledgeBase::bundled();
        // "fish" has no exact entity but is a strict substring of many fish
        // entities (catfish, swordfish, angelfish, clownfish, pufferfish,
        // sailfish, jellyfish, sunfish, ...) plus UI phrases like
        // "you fished" / "fished up". Must be Ambiguous with 2+ real KB ids.
        assert!(kb.find_by_name("fish").is_none());
        match resolve(kb, "fish") {
            Resolution::Ambiguous(ids) => {
                assert!(
                    ids.len() >= 2,
                    "expected 2+ overlaps for 'fish', got {ids:?}"
                );
                let valid: std::collections::HashSet<&str> =
                    kb.entities().iter().map(|e| e.id.as_str()).collect();
                for id in &ids {
                    assert!(valid.contains(id.as_str()), "id {id} not in KB");
                }
            }
            other => panic!("expected Ambiguous for 'fish', got {other:?}"),
        }
        // Exact takes precedence: "shark" exists as fish:shark, so it must
        // be Exact even though it could overlap others.
        assert_eq!(
            resolve(kb, "shark"),
            Resolution::Exact("fish:shark".to_string())
        );
    }

    #[test]
    fn unknown_input() {
        let kb = KnowledgeBase::bundled();
        assert_eq!(resolve(kb, "zzzqqqxzy123"), Resolution::Unknown);
        assert_eq!(resolve(kb, ""), Resolution::Unknown);
        assert_eq!(resolve(kb, "   "), Resolution::Unknown);
        // "snappe" overlaps fish:snapper AND ui:line-snapped: Ambiguous
        // (fail-closed) rather than a guess at either.
        assert!(kb.find_by_name("snappe").is_none());
        match resolve(kb, "snappe") {
            Resolution::Ambiguous(ids) => {
                assert!(ids.contains(&"fish:snapper".to_string()));
                assert!(ids.contains(&"ui:line-snapped".to_string()));
            }
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn skeletal_shark_is_not_exact_without_kb_entity() {
        let kb = KnowledgeBase::bundled();
        // Honesty: only Exact if the KB really has that entity.
        assert!(kb.find_by_name("skeletal shark").is_none());
        match resolve(kb, "skeletal shark") {
            Resolution::Exact(id) => panic!("must not invent Exact({id}) for skeletal shark"),
            Resolution::Ambiguous(_) | Resolution::Unknown => {}
        }
    }

    #[test]
    fn entity_id_stability() {
        let kb = KnowledgeBase::bundled();
        let golden = kb.find_by_name("golden").expect("golden in KB");
        assert_eq!(golden.id, "fish:golden");
        assert_eq!(golden.canonical_name, "Golden");
        let tori = kb.find_by_name("Tori").expect("Tori in KB");
        assert_eq!(tori.id, "fruit:tori");
        // Canonical view preserves ids unchanged.
        let view = canonical_view(kb);
        let vg = view.iter().find(|c| c.entity_id == "fish:golden").expect("view has fish:golden");
        assert_eq!(vg.canonical_name, "Golden");
    }

    #[test]
    fn resolver_never_returns_id_outside_kb() {
        let kb = KnowledgeBase::bundled();
        let valid: std::collections::HashSet<&str> =
            kb.entities().iter().map(|e| e.id.as_str()).collect();
        let words = [
            "Suna", "sand", "Phoenix", "tori", "Golden", "golden", "shark",
            "fish", "devil", "tuna", "crab", "squid", "T-Rex", "trex",
            "Common Bait", "common", "you caught", "spawned",
            "zzzqqqxzy123", "", "   ",
            "Blue-Lip Grouper", "Tigerfin", "Skeletal Shark", "Sunken Armor",
            "Candy Corn Squid", "skeletal shark", "snappe",
        ];
        for w in words {
            match resolve(kb, w) {
                Resolution::Exact(id) => {
                    assert!(valid.contains(id.as_str()), "Exact({id}) for {w:?} not in KB");
                }
                Resolution::Ambiguous(ids) => {
                    assert!(ids.len() >= 2, "Ambiguous for {w:?} must hold 2+ ids");
                    for id in &ids {
                        assert!(valid.contains(id.as_str()), "Ambiguous id {id} for {w:?} not in KB");
                    }
                }
                Resolution::Unknown => {}
            }
        }
    }

    #[test]
    fn requested_drop_names_are_not_in_kb() {
        let kb = KnowledgeBase::bundled();
        // Report: all five requested names are absent, therefore NOT added.
        // - Blue-Lip Grouper: NOT in KB
        // - Tigerfin: NOT in KB
        // - Skeletal Shark: NOT in KB
        // - Sunken Armor: NOT in KB
        // - Candy Corn Squid: NOT in KB
        for name in REQUESTED_ABSENT {
            assert!(
                kb.find_by_name(name).is_none(),
                "{name:?} unexpectedly found in KB"
            );
            match resolve(kb, name) {
                Resolution::Exact(id) => {
                    panic!("{name:?} must not resolve Exact({id}); it is not in the KB")
                }
                Resolution::Ambiguous(ids) => {
                    // Ambiguous would still only list real KB ids, never the
                    // requested name itself as a new entity.
                    let valid: std::collections::HashSet<&str> =
                        kb.entities().iter().map(|e| e.id.as_str()).collect();
                    for id in &ids {
                        assert!(valid.contains(id.as_str()));
                    }
                }
                Resolution::Unknown => {}
            }
        }
        // The canonical view contains zero invented entities for these names.
        let view = canonical_view(kb);
        for name in REQUESTED_ABSENT {
            let norm = fruit::normalize(name);
            assert!(
                !view.iter().any(|c| fruit::normalize(&c.canonical_name) == norm),
                "{name:?} must not appear as a canonical entity"
            );
        }
    }

    #[test]
    fn canonical_view_flags_and_provenance() {
        let kb = KnowledgeBase::bundled();
        let view = canonical_view(kb);
        assert_eq!(view.len(), kb.len());
        // fishing_drop: true for Fish/Fruit, false for Bait/UiTerm.
        for c in &view {
            match c.category {
                EntityCategory::Fish | EntityCategory::Fruit => {
                    assert!(c.fishing_drop, "{} should be fishing_drop", c.entity_id)
                }
                EntityCategory::Bait | EntityCategory::UiTerm => {
                    assert!(!c.fishing_drop, "{} should not be fishing_drop", c.entity_id)
                }
                _ => assert!(!c.fishing_drop),
            }
            // seasonal is always false (KB carries no such flag — not invented).
            assert!(!c.seasonal);
            assert!(c.active);
            // Bundled provenance: url is None, so wiki_url is None (no fakes).
            assert_eq!(c.wiki_url, None);
            assert!(!c.wiki_source.is_empty());
            assert_eq!(c.visual_refs_count, 0);
        }
        // Spot-check: Tori aliases include Phoenix from FRUIT_ALIASES.
        let tori = view.iter().find(|c| c.entity_id == "fruit:tori").expect("fruit:tori");
        assert!(tori.aliases.iter().any(|a| a.eq_ignore_ascii_case("Phoenix")));
    }
}
