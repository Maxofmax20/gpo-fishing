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
//!
//! # Shadowing diagnostics (honest duplicates — NOT a behaviour change)
//!
//! `KnowledgeBase` indexes `canonical_name` + `aliases` + `ocr_aliases` into a
//! single map with **first-wins** `or_insert`. `fruit::DEFAULT_FRUITS` lists
//! BOTH the canonical Japanese name and its English alias (e.g. `Tori` and
//! `Phoenix`), and in 26 of those 27 pairs the canonical is pushed first, so
//! the duplicate fruit row is unreachable through `find_by_name`. (The one
//! exception is Smoke/Moku, where `DEFAULT_FRUITS` lists `Smoke` before
//! `Moku`, so nothing is shadowed — see
//! `smoke_moku_pair_is_the_one_duplicate_pair_that_does_not_shadow`.)
//!
//! We deliberately do **not** change that resolution: `resolve("Dark")`
//! returning `fruit:yami` is CORRECT (Yami is the real fruit, `fruit:dark` is
//! the redundant duplicate row). Changing it would be a behavioural
//! regression. Instead the duplication is made **visible and honest** via
//! [`shadowed_entities`] and [`CanonicalEntity::shadowed_by`].
//!
//! # Resolver hardening
//!
//! - Exact path: unchanged (`kb.find_by_name` on the plain normalised key).
//! - Bounded unicode/OCR exact retry: [`ocr_variant_fold`], fail-closed.
//! - Anchored substring fallback: input must be a **prefix of a whitespace
//!   token** of the candidate and at least [`MIN_SUBSTRING_INPUT_LEN`] chars,
//!   otherwise `"on"` would "match" `crimson`, `marlin`, `cod`, `pufferfish`.

use crate::core::fruit;
use crate::core::knowledge::{EntityCategory, KnowledgeBase};
use serde::{Deserialize, Serialize};

/// Minimum length (in chars) of the **normalised input** before the anchored
/// substring fallback is allowed to run.
///
/// Rationale: unanchored `contains` on very short inputs is meaningless in
/// this KB — `"on"` is a substring of `crimson`, `marlin`, `cod`,
/// `pufferfish`, ... which produced cross-category `Ambiguous` explosions.
/// 4 is the shortest input that can still carry a real word prefix
/// (`snapp`, `snap`, `shar`).
pub const MIN_SUBSTRING_INPUT_LEN: usize = 4;

/// Canonical, UI-safe view of one [`crate::core::knowledge::GpoEntity`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// `Some(<winning entity id>)` when this entity is **unreachable** through
    /// `kb.find_by_name` because another entity claimed its normalised
    /// canonical name first (KB index is first-wins). `None` otherwise.
    ///
    /// Purely diagnostic: it changes no resolution. See [`shadowed_entities`].
    #[serde(default)]
    pub shadowed_by: Option<String>,
}

/// One KB entity that can never be found by name because a different entity
/// owns its normalised canonical name (see module docs on shadowing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowedEntity {
    /// The unreachable (duplicate) entity's own id.
    pub entity_id: String,
    /// Its canonical name, unchanged from the KB.
    pub canonical_name: String,
    /// Id of the entity that wins the KB name index for that key.
    pub shadowed_by: String,
    /// Canonical name of the winning entity.
    pub shadowed_by_name: String,
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
    let shadow_list = shadowed_entities(kb);
    let shadowed: std::collections::HashMap<&str, &str> =
        shadow_list.iter().map(|s| (s.entity_id.as_str(), s.shadowed_by.as_str())).collect();
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
            shadowed_by: shadowed.get(e.id.as_str()).map(|s| (*s).to_string()),
        })
        .collect()
}

/// Report every entity that is **unreachable** through
/// [`KnowledgeBase::find_by_name`] because a *different* entity claimed the
/// normalised form of its `canonical_name` first (the KB index is
/// first-wins `or_insert`).
///
/// Purely diagnostic — nothing is renamed, removed, or re-pointed. In the
/// bundled KB this is 26 duplicated fruit rows (`fruit::DEFAULT_FRUITS` lists
/// both `Tori` and `Phoenix`, and `Tori` is pushed first, so `fruit:phoenix`
/// is shadowed by `fruit:tori`, …).
///
/// NOTE: the winners are the *correct* rows (`resolve("Dark")` → `fruit:yami`
/// is right, `fruit:dark` is the redundant duplicate), so this list is a
/// curation to-do, never a bug to "fix" by changing resolution.
///
/// Sorted by `entity_id` for deterministic output.
pub fn shadowed_entities(kb: &KnowledgeBase) -> Vec<ShadowedEntity> {
    let mut out: Vec<ShadowedEntity> = Vec::new();
    for e in kb.entities() {
        let key = fruit::normalize(&e.canonical_name);
        if key.is_empty() {
            continue;
        }
        // `find_by_name` re-normalises; `fruit::normalize` is idempotent.
        if let Some(winner) = kb.find_by_name(&key) {
            if winner.id != e.id {
                out.push(ShadowedEntity {
                    entity_id: e.id.clone(),
                    canonical_name: e.canonical_name.clone(),
                    shadowed_by: winner.id.clone(),
                    shadowed_by_name: winner.canonical_name.clone(),
                });
            }
        }
    }
    out.sort_by(|a, b| a.entity_id.cmp(&b.entity_id));
    out
}

/// Resolver outcome. Never invents ids; every id comes from the passed KB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Exact(String),
    Ambiguous(Vec<String>),
    Unknown,
}

/// Dependency-free unicode folding (no new crates).
///
/// Deliberately minimal and *invertible-ish* (nothing is deleted except
/// combining marks):
/// - fullwidth ASCII `U+FF01..=U+FF5E` → ASCII `!..~`
/// - combining diacritical marks `U+0300..=U+036F` → dropped
///   (so a decomposed `i` + U+0301 folds back to plain `i`)
/// - non-breaking / ideographic spaces and curly quotes → ASCII `' '`
///   (same treatment `fruit::normalize` gives every punctuation char)
fn unicode_fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let cp = ch as u32;
        if (0x0300..=0x036F).contains(&cp) {
            continue; // combining diacritical mark
        }
        if (0xFF01..=0xFF5E).contains(&cp) {
            // Fullwidth forms always land on printable ASCII here.
            out.push(char::from_u32(cp - 0xFEE0).unwrap_or(ch));
            continue;
        }
        match ch {
            // NBSP family + ideographic space.
            '\u{00A0}' | '\u{2007}' | '\u{202F}' | '\u{3000}' => out.push(' '),
            // Curly quotes / primes: treated as plain separators, exactly like
            // every other punctuation char under `fruit::normalize`.
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{201C}' | '\u{201D}' | '\u{201E}'
            | '\u{2032}' | '\u{2033}' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

/// OCR-confusable folding. **LOSSY — read before use.**
///
/// `unicode_fold` first (fullwidth / diacritics / nbsp / curly quotes), then
/// `fruit::normalize` (lowercase, punctuation → space), then the *only*
/// three confusable classes we are willing to guess:
///
/// | class | folded to |
/// |-------|-----------|
/// | `0` `O` `o` | `o` |
/// | `1` `l` `I` `i` | `l` |
/// | `5` `S` `s` | `s` |
///
/// Because `fruit::normalize` already lowercases ASCII, `I`/`S`/`O` arrive
/// here as `i`/`s`/`o`; the uppercase forms are listed for clarity.
///
/// This mapping is **not injective**: `i` and `l` genuinely collide, and
/// `0`/`o`, `5`/`s` collide too. Nothing may treat a folded match as
/// authoritative on its own — see [`folded_hits`] / [`resolve`], which are
/// fail-closed (`Exact` only for exactly ONE distinct entity id, `Ambiguous`
/// for 2+).
pub fn ocr_variant_fold(s: &str) -> String {
    fruit::normalize(&unicode_fold(s))
        .chars()
        .map(|c| match c {
            '0' | 'O' | 'o' => 'o',
            '1' | 'l' | 'I' | 'i' => 'l',
            '5' | 'S' | 's' => 's',
            other => other,
        })
        .collect()
}

/// Ids whose folded canonical/alias/ocr-alias set contains `folded_query`,
/// sorted + deduped. `Unknown` (empty) when nothing folds onto the query.
fn folded_hits(kb: &KnowledgeBase, folded_query: &str) -> Vec<String> {
    if folded_query.is_empty() {
        return Vec::new();
    }
    let mut ids: Vec<String> = kb
        .entities()
        .iter()
        .filter(|e| {
            std::iter::once(&e.canonical_name)
                .chain(e.aliases.iter())
                .chain(e.ocr_aliases.iter())
                .any(|cand| ocr_variant_fold(cand) == folded_query)
        })
        .map(|e| e.id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// True when `norm` is a **prefix of one whitespace-delimited token** of
/// `cnorm` and that token is strictly longer than `norm`.
///
/// Strictness matters: a token equal to `norm` is an exact name, which belongs
/// to the Exact path (handled by `kb.find_by_name`), never to this fallback.
fn anchored_token_prefix(cnorm: &str, norm: &str) -> bool {
    cnorm.split(' ').any(|tok| tok.len() > norm.len() && tok.starts_with(norm))
}

/// Resolve user/OCR input against the passed KB.
///
/// Stages, in order:
///
/// 1. **Normalize** (lowercase, punctuation → space, collapse spaces) via
///    `fruit::normalize` — the exact same function the KB index uses, so the
///    two can never disagree about a key.
/// 2. **Exact** — `kb.find_by_name(norm)` → `Exact(id)`. Unchanged.
/// 3. **Bounded unicode/OCR exact retry** — only if stage 2 missed AND the
///    folded query differs from the plain key. `Exact` **only** when the folded
///    query hits exactly ONE distinct entity id; 2+ → `Ambiguous(ids)`;
///    0 → continue to stage 4. This is the safety property that stops a lossy
///    fold from collapsing two different KB entities into one confident answer.
/// 4. **Anchored substring fallback** — input must be at least
///    [`MIN_SUBSTRING_INPUT_LEN`] chars and be a prefix of a whitespace token
///    of a candidate name/alias. 2+ distinct hits → `Ambiguous(ids)`;
///    anything else (including a single hit) → `Unknown`; we never guess.
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
    // Stage 3: lossy OCR retry, fail-closed on any multi-entity collision.
    let folded = ocr_variant_fold(input);
    if !folded.is_empty() && folded != norm {
        let hits = folded_hits(kb, &folded);
        match hits.len() {
            0 => {}
            1 => return Resolution::Exact(hits[0].clone()),
            _ => return Resolution::Ambiguous(hits),
        }
    }
    // Stage 4: anchored fallback. Short fragments are refused outright.
    if norm.chars().count() < MIN_SUBSTRING_INPUT_LEN {
        return Resolution::Unknown;
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
            // Anchored: word-boundary prefix, strictly longer token.
            if anchored_token_prefix(&cnorm, &norm) {
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
        // 0 hits, or a single fallback hit with no exact match: never guess.
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

    // ---------------------------------------------------------------- tests

    /// Every name a bundled entity is findable by (same triple `resolve` scans).
    fn candidate_names(e: &crate::core::knowledge::GpoEntity) -> impl Iterator<Item = &str> {
        std::iter::once(e.canonical_name.as_str())
            .chain(e.aliases.iter().map(|s| s.as_str()))
            .chain(e.ocr_aliases.iter().map(|s| s.as_str()))
    }

    /// Four plausible OCR/mangled renderings of one name: fullwidth, digit
    /// confusables, nbsp/curly-quote punctuation, and a decomposed accent.
    fn ocr_mutations(s: &str) -> Vec<String> {
        let fullwidth = s
            .chars()
            .map(|c| {
                let cp = c as u32;
                if (0x21..=0x7E).contains(&cp) {
                    char::from_u32(cp + 0xFEE0).unwrap_or(c)
                } else {
                    c
                }
            })
            .collect();
        let digits = s
            .chars()
            .map(|c| match c {
                'o' | 'O' => '0',
                's' | 'S' => '5',
                'i' | 'I' | 'l' | 'L' => '1',
                other => other,
            })
            .collect();
        let spaces = s.replace(' ', "\u{00A0}").replace('-', "\u{2019}");
        let mut accented = String::from("\u{0301}");
        accented.push_str(s);
        vec![fullwidth, digits, spaces, accented]
    }

    #[test]
    fn resolve_outcome_table() {
        let kb = KnowledgeBase::bundled();

        // --- Exact: real KB entities, case/whitespace/confusable variants.
        assert_eq!(resolve(kb, "shark"), Resolution::Exact("fish:shark".to_string()));
        assert_eq!(resolve(kb, "snapper"), Resolution::Exact("fish:snapper".to_string()));
        assert_eq!(resolve(kb, "SWORDFISH"), Resolution::Exact("fish:swordfish".to_string()));
        assert_eq!(resolve(kb, "  Swordfish  "), Resolution::Exact("fish:swordfish".to_string()));
        // The one fishing-drop name from the "commonly cited" list that is
        // actually in the KB.
        assert!(kb.find_by_name("Swordfish").is_some());

        // --- Absent names: honest fail-closed, never invented.
        assert!(kb.find_by_name("skeletal shark").is_none());
        assert_eq!(resolve(kb, "Skeletal Shark"), Resolution::Unknown);
        assert_eq!(resolve(kb, "Dark Skeletal Shark"), Resolution::Unknown);
        assert_eq!(resolve(kb, "unknown fish"), Resolution::Unknown);
        // Random garbage / empty / whitespace-only.
        assert_eq!(resolve(kb, "zzq vvv 9182 !?"), Resolution::Unknown);
        assert_eq!(resolve(kb, ""), Resolution::Unknown);
        assert_eq!(resolve(kb, "   "), Resolution::Unknown);
        assert_eq!(resolve(kb, "\u{00A0}\u{3000}"), Resolution::Unknown);

        // --- Partial OCR confusables: the bounded folded retry helps.
        assert_eq!(resolve(kb, "5hark"), Resolution::Exact("fish:shark".to_string()));
        assert_eq!(resolve(kb, "5w0rdf15h"), Resolution::Exact("fish:swordfish".to_string()));
        assert_eq!(resolve(kb, "swordf1sh"), Resolution::Exact("fish:swordfish".to_string()));
        assert_eq!(
            resolve(kb, "\u{FF33}\u{FF57}\u{FF4F}\u{FF52}\u{FF44}fish"),
            Resolution::Exact("fish:swordfish".to_string()),
            "fullwidth 'Sword' + ascii 'fish' must fold to the exact fish"
        );

        // --- Anchored fallback: "snapp" is a prefix of the token "snapper"
        //     and of the token "snapped", so it is a genuine 2-way overlap
        //     and therefore fail-closed Ambiguous — never a guess, and in
        //     particular never `Exact("ui:line-snapped")`.
        match resolve(kb, "snapp") {
            Resolution::Ambiguous(ids) => {
                assert!(ids.contains(&"fish:snapper".to_string()), "{ids:?}");
                assert!(ids.contains(&"ui:line-snapped".to_string()), "{ids:?}");
            }
            other => panic!("'snapp' must be fail-closed Ambiguous, got {other:?}"),
        }

        // --- Punctuation INSIDE a word is NOT rejoined. `fruit::normalize`
        //     (shared with the KB index, not this module) maps every
        //     non-alphanumeric to a space, so "Sword-fish" normalises to the
        //     two tokens "sword fish", which is neither a KB key nor a
        //     single-token prefix of any KB name → Unknown (no wrong guess).
        assert_eq!(resolve(kb, "Sword-fish!"), Resolution::Unknown);
        assert_eq!(resolve(kb, "  Sword   fish  "), Resolution::Unknown);
        // Whitespace collapsing itself DOES work (verified here so the
        // limitation above is clearly about punctuation, not spacing).
        assert_eq!(resolve(kb, "  Common   Bait  "), Resolution::Exact("bait:common".to_string()));
        assert_eq!(resolve(kb, "T  Rex"), Resolution::Exact("fruit:t-rex".to_string()));
        // Known conservative limitation: the `ﬁ` ligature (U+FB01) is not
        // folded, so it does not reach `fish:swordfish`.
        assert_eq!(resolve(kb, "Sword\u{FB01}sh"), Resolution::Unknown);
    }

    #[test]
    fn short_fragments_do_not_explode_into_cross_category_ambiguity() {
        let kb = KnowledgeBase::bundled();
        // Unanchored `contains` used to make "on" hit crimson/marlin/cod/
        // pufferfish/... and "ar" hit myriad names: cross-category
        // Ambiguous explosions from 2-char fragments. Both are below
        // MIN_SUBSTRING_INPUT_LEN → Unknown.
        assert_eq!(MIN_SUBSTRING_INPUT_LEN, 4);
        for frag in ["on", "ar", "sh", "fi", "a", "1"] {
            assert_eq!(
                resolve(kb, frag),
                Resolution::Unknown,
                "{frag:?} must not fall back to substring matching"
            );
        }
        // 4-char word prefixes still work (boundary of the constant).
        assert_eq!(resolve(kb, "snap"), Resolution::Ambiguous(vec![
            "fish:snapper".to_string(),
            "ui:line-snapped".to_string(),
        ]));
        // Single fallback hit is still refused (never guess).
        assert_eq!(resolve(kb, "shar"), Resolution::Unknown);
    }

    #[test]
    fn shadowed_entities_reports_bundled_duplicate_fruits() {
        let kb = KnowledgeBase::bundled();
        let shadowed = shadowed_entities(kb);

        // EMPIRICAL COUNT for the bundled KB: 26.
        // The project's KB lists both the canonical Japanese fruit name and
        // its English alias in `DEFAULT_FRUITS` (e.g. "Tori" AND "Phoenix"),
        // and `fruit:dark`-style duplicates are therefore unreachable via
        // `find_by_name`. 26 of those 27 alias/canonical pairs collide in the
        // KB index; the Smoke/Moku pair does NOT (see the test below), which
        // is why the count is 26 and not 27.
        assert_eq!(shadowed.len(), 26, "ids: {:?}", shadowed.iter().map(|s| &s.entity_id).collect::<Vec<_>>());

        // Deterministic order.
        let mut sorted = shadowed.clone();
        sorted.sort_by(|a, b| a.entity_id.cmp(&b.entity_id));
        assert_eq!(shadowed, sorted, "shadowed_entities must be sorted by entity_id");

        // Every entry is a real KB pair, and the winner really wins the index.
        let valid: std::collections::HashSet<&str> =
            kb.entities().iter().map(|e| e.id.as_str()).collect();
        for s in &shadowed {
            assert!(valid.contains(s.entity_id.as_str()), "{}", s.entity_id);
            assert!(valid.contains(s.shadowed_by.as_str()), "{}", s.shadowed_by);
            assert_ne!(s.entity_id, s.shadowed_by, "a shadow report must name two different ids");
            let self_hit = kb.find_by_name(&s.canonical_name).expect("canonical name is indexed");
            assert_eq!(self_hit.id, s.shadowed_by, "{} claim is wrong", s.entity_id);
            assert_eq!(self_hit.canonical_name, s.shadowed_by_name);
        }

        // Representative pairs (plus every one of the 26, pinned).
        let expected: &[(&str, &str, &str)] = &[
            ("fruit:barrier", "Barrier", "fruit:bari"),
            ("fruit:biscuit", "Biscuit", "fruit:bisu"),
            ("fruit:bomu", "Bomu", "fruit:bomb"),
            ("fruit:chiyu", "Chiyu", "fruit:heal"),
            ("fruit:clear", "Clear", "fruit:suke"),
            ("fruit:daibutsu", "Daibutsu", "fruit:buddha"),
            ("fruit:dark", "Dark", "fruit:yami"),
            ("fruit:doku", "Doku", "fruit:venom"),
            ("fruit:flame", "Flame", "fruit:mera"),
            ("fruit:gasu", "Gasu", "fruit:gas"),
            ("fruit:glint", "Glint", "fruit:pika"),
            ("fruit:gravity", "Gravity", "fruit:zushi"),
            ("fruit:ice", "Ice", "fruit:hie"),
            ("fruit:love", "Love", "fruit:mero"),
            ("fruit:magma", "Magma", "fruit:magu"),
            ("fruit:nikyu", "Nikyu", "fruit:paw"),
            ("fruit:phoenix", "Phoenix", "fruit:tori"),
            ("fruit:revive", "Revive", "fruit:yomi"),
            ("fruit:rubber", "Rubber", "fruit:gomu"),
            ("fruit:rumble", "Rumble", "fruit:goro"),
            ("fruit:sand", "Sand", "fruit:suna"),
            ("fruit:shadow", "Shadow", "fruit:kage"),
            ("fruit:soul", "Soul", "fruit:soru"),
            ("fruit:string", "String", "fruit:ito"),
            ("fruit:tremor", "Tremor", "fruit:gura"),
            ("fruit:trex", "Trex", "fruit:t-rex"),
        ];
        assert_eq!(expected.len(), shadowed.len());
        for (id, name, winner) in expected {
            let s = shadowed.iter().find(|s| s.entity_id == *id)
                .unwrap_or_else(|| panic!("missing shadow report for {id}"));
            assert_eq!(s.canonical_name, *name, "{id} canonical name");
            assert_eq!(s.shadowed_by, *winner, "{id} winner");
        }
    }

    #[test]
    fn smoke_moku_pair_is_the_one_duplicate_pair_that_does_not_shadow() {
        let kb = KnowledgeBase::bundled();
        // `DEFAULT_FRUITS` lists "Smoke" BEFORE "Moku", and FRUIT_ALIASES says
        // ("Moku", ["Smoke"]) — i.e. for this pair the *alias* row is pushed
        // second, so `fruit:smoke` (canonical "Smoke") wins the index and
        // `fruit:moku` still owns its own key. Neither is unreachable, which
        // is why the bundled shadow count is 26 and not 27.
        let shadow_list = shadowed_entities(kb);
        let shadowed: std::collections::HashSet<&str> =
            shadow_list.iter().map(|s| s.entity_id.as_str()).collect();
        assert!(!shadowed.contains("fruit:smoke"), "Smoke wins its own key");
        assert!(!shadowed.contains("fruit:moku"), "Moku wins its own key");
        assert_eq!(kb.find_by_name("Smoke").expect("Smoke").id, "fruit:smoke");
        assert_eq!(kb.find_by_name("Moku").expect("Moku").id, "fruit:moku");
        // Resolution is unchanged and correct for the shadowed pairs too:
        // the winners are the real fruits, the duplicates are the noise.
        assert_eq!(resolve(kb, "Dark"), Resolution::Exact("fruit:yami".to_string()));
        assert_eq!(resolve(kb, "Phoenix"), Resolution::Exact("fruit:tori".to_string()));
        assert_eq!(resolve(kb, "Love"), Resolution::Exact("fruit:mero".to_string()));
    }

    #[test]
    fn canonical_view_marks_shadowed_entities() {
        let kb = KnowledgeBase::bundled();
        let view = canonical_view(kb);
        let shadowed = shadowed_entities(kb);
        assert_eq!(view.len(), kb.len());

        for c in &view {
            let expect = shadowed.iter().find(|s| s.entity_id == c.entity_id).map(|s| s.shadowed_by.clone());
            assert_eq!(c.shadowed_by, expect, "shadowed_by mismatch for {}", c.entity_id);
        }
        // Shadowed entries carry Some(...).
        let v = |id: &str| view.iter().find(|c| c.entity_id == id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(v("fruit:phoenix").shadowed_by.as_deref(), Some("fruit:tori"));
        assert_eq!(v("fruit:dark").shadowed_by.as_deref(), Some("fruit:yami"));
        assert_eq!(v("fruit:love").shadowed_by.as_deref(), Some("fruit:mero"));
        // Non-shadowed entries carry None.
        assert_eq!(v("fruit:tori").shadowed_by, None);
        assert_eq!(v("fruit:yami").shadowed_by, None);
        assert_eq!(v("fruit:mero").shadowed_by, None);
        assert_eq!(v("fish:swordfish").shadowed_by, None);
        assert_eq!(v("bait:common").shadowed_by, None);
        assert_eq!(v("ui:you-fished").shadowed_by, None);
        // Exactly `shadowed.len()` rows are flagged.
        assert_eq!(view.iter().filter(|c| c.shadowed_by.is_some()).count(), shadowed.len());
    }

    #[test]
    fn unicode_fold_and_ocr_variant_fold_are_conservative() {
        // unicode_fold: fullwidth ASCII → ASCII.
        assert_eq!(unicode_fold("\u{FF33}\u{FF57}\u{FF4F}\u{FF52}\u{FF44}"), "Sword");
        // unicode_fold: combining diacritics dropped.
        assert_eq!(unicode_fold("Sword\u{0301}fish"), "Swordfish");
        // unicode_fold: nbsp family + curly quotes → ASCII space.
        assert_eq!(unicode_fold("a\u{00A0}b\u{202F}c\u{2019}d\u{201C}e"), "a b c d e");
        // unicode_fold: everything else is untouched (no ligature folding).
        assert_eq!(unicode_fold("Sword\u{FB01}sh"), "Sword\u{FB01}sh");

        // ocr_variant_fold = normalize(unicode_fold(..)) + the 3 confusable classes.
        assert_eq!(ocr_variant_fold("Swordfish"), "swordflsh", "i folds into the l class (LOSSY)");
        assert_eq!(ocr_variant_fold("swordf1sh"), "swordflsh");
        assert_eq!(ocr_variant_fold("5w0rdf15h"), "swordflsh");
        assert_eq!(ocr_variant_fold("\u{FF33}wordfish"), "swordflsh");
        assert_eq!(ocr_variant_fold("Snapper"), "snapper", "no confusable in 'snapper'");
        assert_eq!(ocr_variant_fold("Common Bait"), "common balt");
        assert_eq!(ocr_variant_fold(""), "");
        assert_eq!(ocr_variant_fold("!!!"), "");
        // Only the documented three classes move: 2/6/8/B are untouched.
        assert_eq!(ocr_variant_fold("26 8B"), "26 8b");
    }

    #[test]
    fn ocr_folded_retry_is_ambiguous_when_folding_collides() {
        let kb = KnowledgeBase::bundled();
        // REAL collision in the bundled KB: "Phoenix" is fruit:tori's alias
        // AND fruit:phoenix's own canonical name, so they fold together.
        assert_eq!(folded_hits(kb, &ocr_variant_fold("Phoenix")), vec![
            "fruit:phoenix".to_string(),
            "fruit:tori".to_string(),
        ]);
        // "ph0enix" misses the plain index, so the folded retry sees BOTH.
        assert!(kb.find_by_name("ph0enix").is_none());
        assert_eq!(resolve(kb, "ph0enix"), Resolution::Ambiguous(vec![
            "fruit:phoenix".to_string(),
            "fruit:tori".to_string(),
        ]));
        // The plain path still wins for the honest spelling (unchanged Exact).
        assert_eq!(resolve(kb, "Phoenix"), Resolution::Exact("fruit:tori".to_string()));
    }

    #[test]
    fn folded_resolution_never_collapses_two_entities_into_one_exact() {
        let kb = KnowledgeBase::bundled();
        let valid: std::collections::HashSet<&str> =
            kb.entities().iter().map(|e| e.id.as_str()).collect();
        let mut checked = 0usize;

        for e in kb.entities() {
            // Sanity: every entity is its own folded hit.
            for folded_query in candidate_names(e).map(ocr_variant_fold) {
                let hits = folded_hits(kb, &folded_query);
                assert!(hits.contains(&e.id), "{} missing from its own folded hits", e.id);
                for id in &hits {
                    assert!(valid.contains(id.as_str()), "folded id {id} not in KB");
                }
                let mut sorted = hits.clone();
                sorted.sort();
                sorted.dedup();
                assert_eq!(hits, sorted, "folded_hits must be sorted + deduped");
            }
            // The property: mangled input may only yield Exact when the
            // folded query hits exactly ONE distinct entity id.
            for cand in candidate_names(e) {
                for m in ocr_mutations(cand) {
                    checked += 1;
                    let folded_query = ocr_variant_fold(&m);
                    let hits = folded_hits(kb, &folded_query);
                    if let Resolution::Exact(id) = resolve(kb, &m) {
                        assert!(valid.contains(id.as_str()));
                        // If the plain (unchanged) index did not answer,
                        // the Exact MUST come from the folded retry, which is
                        // allowed only for a unique folded hit.
                        let plain = kb.find_by_name(&m).map(|x| x.id.clone());
                        if plain.as_deref() != Some(id.as_str()) {
                            assert_eq!(
                                hits,
                                vec![id.clone()],
                                "folded Exact({id}) for {m:?} of {cand:?} collapsed {hits:?}"
                            );
                        }
                    }
                }
            }
        }
        assert!(checked > 300, "property sweep was too small: {checked}");
    }

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
