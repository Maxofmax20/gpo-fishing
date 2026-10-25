//! Multi-source perception: RAW SCREEN DATA → INTERPRETED OBSERVATION →
//! CONFIRMED GAME STATE.
//!
//! OCR stays one evidence source among several. [`correlate_text`] fuses an
//! OCR reading with the GPO knowledge base (and an optional vision hint)
//! into a structured [`Observation`] that names its evidence and refuses to
//! hallucinate: weak evidence yields `Unknown` with a reason, never a guess.
//!
//! Confidence is explainable and coarse (2 decimals): it is a calibrated
//! combination of named evidence weights, not a measured probability.

use serde::{Deserialize, Serialize};
use strsim::jaro_winkler;

use crate::core::fruit;
use crate::core::knowledge::{EntityCategory, GpoEntity, KnowledgeBase};

/// Where one piece of evidence came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSource {
    Ocr,
    Vision,
    Region,
    Knowledge,
}

/// Screen context the observation was made in. Constrains which entity
/// categories are plausible (context compatibility evidence).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenKind {
    Fishing,
    BaitMenu,
    Shop,
    SpawnBanner,
    Unknown,
}

impl ScreenKind {
    fn plausible(self, category: EntityCategory) -> bool {
        match self {
            ScreenKind::Fishing => matches!(
                category,
                EntityCategory::Fish
                    | EntityCategory::Fruit
                    | EntityCategory::Item
                    | EntityCategory::UiTerm
            ),
            ScreenKind::BaitMenu => {
                matches!(category, EntityCategory::Bait | EntityCategory::UiTerm)
            }
            ScreenKind::Shop => matches!(
                category,
                EntityCategory::Bait | EntityCategory::Item | EntityCategory::UiTerm
            ),
            ScreenKind::SpawnBanner => matches!(
                category,
                EntityCategory::Fruit
                    | EntityCategory::Location
                    | EntityCategory::UiTerm
                    | EntityCategory::Npc
            ),
            ScreenKind::Unknown => true,
        }
    }
}

/// One named, weighted piece of evidence behind a candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub kind: String,
    pub detail: String,
    pub weight: f32,
}

/// A knowledge-base entity proposed for an observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityMatch {
    pub entity_id: String,
    pub canonical_name: String,
    pub category: String,
    pub confidence: f32,
    pub evidence: Vec<Evidence>,
}

/// Raw OCR reading feeding correlation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextReading {
    pub text: String,
    pub region: String,
}

/// Optional vision hint (e.g. from a [`crate::core::vision_provider`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionHint {
    pub label: String,
    pub confidence: f32,
}

/// Interpreted observation: always carries either a confirmed entity or an
/// explicit unknown-with-reason. Never a bare guess.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub timestamp_ms: u64,
    pub source: ObservationSource,
    pub region: String,
    pub screen: ScreenKind,
    pub ocr: Option<TextReading>,
    pub vision_hint: Option<VisionHint>,
    pub candidates: Vec<EntityMatch>,
    /// Confidence of the top candidate (0 when unknown). Rounded to 2dp.
    pub confidence: f32,
    pub entity: Option<EntityMatch>,
    pub unknown_reason: Option<String>,
}

/// Event-driven OCR with one bounded fallback: read the frame raw first;
/// if it yields no text and the frame is small, retry once at 2x upscale
/// (small in-game popups routinely OCR better enlarged). Returns the text
/// plus which variant produced it. Max 2 OCR passes, never in the hot loop —
/// callers are diagnostics/verification paths only.
pub fn ocr_with_fallback(
    read: impl Fn(&crate::core::types::Frame) -> Result<String, String>,
    frame: &crate::core::types::Frame,
) -> Result<(String, &'static str), String> {
    let raw = read(frame)?;
    if !raw.trim().is_empty() {
        return Ok((raw, "raw"));
    }
    if frame.w < 600 && frame.h < 600 {
        let big = frame.upscale(2);
        let up = read(&big)?;
        if !up.trim().is_empty() {
            return Ok((up, "upscale2x"));
        }
    }
    Ok((raw, "raw"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn round2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

/// Score one OCR word against one entity name. Returns (score, evidence
/// kind, detail) for the best of exact/normalized/cleaned/fuzzy.
fn score_word(
    word: &str,
    entity: &GpoEntity,
    fuzzy_threshold: f64,
) -> Option<(f32, String, String)> {
    if word.len() < 3 {
        return None;
    }
    let lower = word.to_ascii_lowercase();
    let names: Vec<(&str, bool)> = std::iter::once((entity.canonical_name.as_str(), true))
        .chain(entity.aliases.iter().map(|a| (a.as_str(), false)))
        .chain(entity.ocr_aliases.iter().map(|a| (a.as_str(), false)))
        .collect();

    // 1. Exact (case-insensitive) on canonical or alias.
    for (n, canonical) in &names {
        if n.eq_ignore_ascii_case(word) {
            let kind = if *canonical { "exact_ocr" } else { "alias" };
            return Some((
                1.0,
                kind.to_string(),
                format!("'{word}' matches {kind} '{n}'"),
            ));
        }
    }
    // 2. OCR-cleaned exact (0→o, 1|l→i, @→a, 5→s).
    let cleaned = fruit::clean_ocr_word(word);
    if cleaned != lower {
        for (n, canonical) in &names {
            if n.eq_ignore_ascii_case(&cleaned) {
                let kind = if *canonical {
                    "normalized_ocr"
                } else {
                    "alias"
                };
                return Some((
                    0.95,
                    kind.to_string(),
                    format!("'{word}' cleaned to '{cleaned}' matches {kind} '{n}'"),
                ));
            }
        }
    }
    // 3. Fuzzy, with the short-name guard: names of ≤4 chars must be exact
    // (a 3-letter OCR fragment must not fuzzy-match "Suna").
    let mut best: Option<(f64, String)> = None;
    for (n, _) in &names {
        if n.len() <= 4 || lower.len() <= 4 {
            continue;
        }
        let s = jaro_winkler(&lower, &n.to_ascii_lowercase())
            .max(jaro_winkler(&cleaned, &n.to_ascii_lowercase()));
        if best.as_ref().map_or(true, |(b, _)| s > *b) {
            best = Some((s, n.to_string()));
        }
    }
    match best {
        Some((s, n)) if s >= fuzzy_threshold.max(0.88) => Some((
            s as f32,
            "fuzzy_ocr".to_string(),
            format!("'{word}' fuzzy-matches '{n}' (similarity {:.2})", s),
        )),
        _ => None,
    }
}

// ===========================================================================
// Phrase-first, longest-span resolution
//
// The per-word stage above scores each WORD independently, so `Crimson
// Snapper` scored `fish:crimson` 1.000 and `fish:snapper` 1.000 while the real
// entity (`fish:crimsonsnapper`) reached 0.973 as a fuzzy match and ranked
// third. Measured over the 1,865 human-reviewed rows, that put the runtime on
// a legacy bare stem 68.7% of the time and made the shadow agreement gate
// structurally unreachable (a perfect model scored ~8.7% against an 80%
// floor).
//
// The stage below ports the algorithm `ml/gpo_ocr_audit/ontology_matcher.py`
// already specifies: strip UI templates, then claim the LONGEST contiguous
// token span present in the text, so a species name consumes its tokens and a
// stem inside it cannot compete. It runs BEFORE the per-word stage; the
// per-word stage survives unchanged as the fallback for single-word entities
// and garbled OCR.
// ===========================================================================

/// Evidence kind for a whole-name (multi-word) resolution.
pub const EVIDENCE_EXACT_PHRASE: &str = "exact_phrase";
/// Evidence kind marking a candidate that is ONE OF SEVERAL competitors for
/// the same name. `correlate_text` refuses to confirm any of them.
pub const EVIDENCE_AMBIGUOUS_PHRASE: &str = "ambiguous_phrase";
/// Evidence kind recording that a UI banner template was stripped before
/// lookup. Removal is evidence, never a silent edit.
pub const EVIDENCE_UI_TEMPLATE: &str = "ui_template";

/// UI-term templates as (normalized template text, owning entity id),
/// ordered longest-template-first with a total deterministic tie-break.
fn ui_templates(kb: &KnowledgeBase) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for e in kb.entities() {
        if e.category != EntityCategory::UiTerm {
            continue;
        }
        for name in std::iter::once(&e.canonical_name)
            .chain(e.aliases.iter())
            .chain(e.ocr_aliases.iter())
        {
            let n = fruit::normalize(name);
            if !n.is_empty() {
                out.push((n, e.id.clone()));
            }
        }
    }
    out.sort_by(|a, b| {
        b.0.len()
            .cmp(&a.0.len())
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.1.cmp(&b.1))
    });
    out.dedup();
    out
}

/// Strip banner templates from NORMALIZED text, longest template first,
/// mirroring `Ontology::strip_ui_templates`. One replacement per template;
/// token boundaries are required so `spawn` is never carved out of `spawned`.
/// Returns (residual, [(template, entity id)]) — the removal is reported, not
/// dropped silently.
fn strip_ui_templates(kb: &KnowledgeBase, norm_text: &str) -> (String, Vec<(String, String)>) {
    let mut residual = norm_text.to_string();
    let mut found: Vec<(String, String)> = Vec::new();
    for (tmpl, id) in ui_templates(kb) {
        let Some(idx) = residual.find(&tmpl) else {
            continue;
        };
        let end = idx + tmpl.len();
        let left_ok = idx == 0 || residual.as_bytes()[idx - 1] == b' ';
        let right_ok = end == residual.len() || residual.as_bytes()[end] == b' ';
        if !(left_ok && right_ok) {
            continue;
        }
        residual.replace_range(idx..end, " ");
        found.push((tmpl, id));
    }
    (
        residual.split_whitespace().collect::<Vec<_>>().join(" "),
        found,
    )
}

/// Every claimable name in the KB: normalized name -> owning entity ids
/// (sorted, deduped; >1 id = same-name tie, which is ambiguity, never a win).
/// UI terms are excluded: they are banner furniture, stripped by
/// [`strip_ui_templates`], never entities a name resolves to.
/// Ordered longest-name-first (chars), then name, for a greedy scan.
fn phrase_name_index(kb: &KnowledgeBase) -> Vec<(String, Vec<String>)> {
    let mut map: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for e in kb.entities() {
        if e.category == EntityCategory::UiTerm {
            continue;
        }
        for name in std::iter::once(&e.canonical_name)
            .chain(e.aliases.iter())
            .chain(e.ocr_aliases.iter())
        {
            let n = fruit::normalize(name);
            if n.is_empty() {
                continue;
            }
            let ids = map.entry(n).or_default();
            if !ids.contains(&e.id) {
                ids.push(e.id.clone());
            }
        }
    }
    let mut out: Vec<(String, Vec<String>)> = map.into_iter().collect();
    out.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    out
}

/// One claimed token span.
#[derive(Debug, Clone)]
struct SpanMatch {
    start: usize,
    words: usize,
    entity_ids: Vec<String>,
}

/// Greedy longest-span scan over the residual's tokens, mirroring
/// `Ontology.scan_tokens` at token granularity: names are tried
/// longest-first and a claimed span consumes its tokens, so `crimson snapper`
/// claims both words for `fish:crimsonsnapper` and `fish:crimson` /
/// `fish:snapper` cannot compete for them.
fn longest_span_matches(kb: &KnowledgeBase, residual: &str) -> Vec<SpanMatch> {
    let tokens: Vec<&str> = residual.split_whitespace().collect();
    let mut taken = vec![false; tokens.len()];
    let mut spans: Vec<SpanMatch> = Vec::new();
    for (name, ids) in phrase_name_index(kb) {
        let words: Vec<&str> = name.split_whitespace().collect();
        let w = words.len();
        if w == 0 || w > tokens.len() {
            continue;
        }
        for start in 0..=(tokens.len() - w) {
            if (start..start + w).any(|i| taken[i]) {
                continue;
            }
            if tokens[start..start + w] == words[..] {
                for t in taken.iter_mut().take(start + w).skip(start) {
                    *t = true;
                }
                spans.push(SpanMatch {
                    start,
                    words: w,
                    entity_ids: ids.clone(),
                });
            }
        }
    }
    spans.sort_by_key(|s| s.start);
    spans
}

/// What the phrase stage concluded.
enum PhraseOutcome {
    /// No span at all, or only single-word spans: defer to the per-word stage.
    NoMatch,
    /// Exactly one entity, named by a multi-word span.
    Resolved { entity_id: String },
    /// Two or more DISTINCT entity identities claimed (a same-span tie, or a
    /// residual region independently matching another canonical entity).
    /// Fail closed.
    Ambiguous { competitor_ids: Vec<String> },
}

/// Two ids are the SAME in-game identity when one's canonical name appears
/// among the other's names (canonical + aliases + ocr_aliases). The KB models
/// `Tori` and `Phoenix` as two entities linked exactly this way (a canonical
/// entity plus its reverse-alias entry); treating them as competitors would be
/// the phantom-ambiguity bug, not a fix.
fn alias_linked(a: &GpoEntity, b: &GpoEntity) -> bool {
    let a_names: std::collections::HashSet<String> = std::iter::once(&a.canonical_name)
        .chain(a.aliases.iter())
        .chain(a.ocr_aliases.iter())
        .map(|n| fruit::normalize(n))
        .collect();
    let b_names: std::collections::HashSet<String> = std::iter::once(&b.canonical_name)
        .chain(b.aliases.iter())
        .chain(b.ocr_aliases.iter())
        .map(|n| fruit::normalize(n))
        .collect();
    a_names.contains(&fruit::normalize(&b.canonical_name))
        || b_names.contains(&fruit::normalize(&a.canonical_name))
}

/// Group entity ids into alias-linked identities, each listed with its
/// KB-first entity first (the same winner the per-word stage's stable ordering
/// would pick). >1 group = genuine ambiguity between unrelated identities.
fn alias_components(kb: &KnowledgeBase, ids: &[String]) -> Vec<Vec<String>> {
    let mut components: Vec<Vec<String>> = Vec::new();
    for id in ids {
        let Some(entity) = kb.entities().iter().find(|e| &e.id == id) else {
            continue;
        };
        let mut placed = false;
        for comp in components.iter_mut() {
            let linked = comp.iter().any(|other| {
                kb.entities()
                    .iter()
                    .find(|e| &e.id == other)
                    .is_some_and(|o| alias_linked(entity, o))
            });
            if linked {
                comp.push(id.clone());
                placed = true;
                break;
            }
        }
        if !placed {
            components.push(vec![id.clone()]);
        }
    }
    // KB-first entity first within each component, for a deterministic winner.
    for comp in components.iter_mut() {
        comp.sort_by_key(|id| {
            kb.entities()
                .iter()
                .position(|e| &e.id == id)
                .unwrap_or(usize::MAX)
        });
    }
    components.sort_by_key(|comp| {
        kb.entities()
            .iter()
            .position(|e| e.id == comp[0])
            .unwrap_or(usize::MAX)
    });
    components
}

fn phrase_outcome(kb: &KnowledgeBase, spans: &[SpanMatch]) -> PhraseOutcome {
    if spans.is_empty() {
        return PhraseOutcome::NoMatch;
    }
    let mut ids: Vec<String> = spans
        .iter()
        .flat_map(|s| s.entity_ids.iter().cloned())
        .collect();
    ids.sort();
    ids.dedup();
    let components = alias_components(kb, &ids);
    if components.len() > 1 {
        return PhraseOutcome::Ambiguous {
            competitor_ids: components.into_iter().map(|c| c[0].clone()).collect(),
        };
    }
    // Exactly one identity: it resolves as a PHRASE only when a multi-word
    // span named it. A single-word hit is left to the per-word stage, which
    // owns the exact/alias/fuzzy evidence kinds for single-word names.
    if spans.iter().any(|s| s.words >= 2) {
        PhraseOutcome::Resolved {
            entity_id: components[0][0].clone(),
        }
    } else {
        PhraseOutcome::NoMatch
    }
}

/// Build one candidate with the shared evidence tail (knowledge existence,
/// context compatibility, stripped-template record). `primary` is the
/// stage-specific first evidence entry.
fn entity_match_for(
    entity: &GpoEntity,
    screen: ScreenKind,
    base: f32,
    primary: Evidence,
    templates: &[(String, String)],
) -> EntityMatch {
    let mut score = base;
    let mut evidence = vec![primary];
    for (tmpl, id) in templates {
        evidence.push(Evidence {
            kind: EVIDENCE_UI_TEMPLATE.to_string(),
            detail: format!("banner template '{tmpl}' ({id}) stripped before lookup"),
            weight: 0.0,
        });
    }
    evidence.push(Evidence {
        kind: "knowledge_match".to_string(),
        detail: format!(
            "'{}' exists in knowledge base ({})",
            entity.canonical_name, entity.provenance.source
        ),
        weight: 0.05,
    });
    score += 0.05;
    if screen.plausible(entity.category) {
        evidence.push(Evidence {
            kind: "context_compatible".to_string(),
            detail: format!("{:?} is plausible on {:?}", entity.category, screen),
            weight: 0.03,
        });
        score += 0.03;
    } else {
        evidence.push(Evidence {
            kind: "context_mismatch".to_string(),
            detail: format!("{:?} is unusual on {:?}", entity.category, screen),
            weight: -0.25,
        });
        score -= 0.25;
    }
    EntityMatch {
        entity_id: entity.id.clone(),
        canonical_name: entity.canonical_name.clone(),
        category: entity.category.as_str().to_string(),
        confidence: round2(score.clamp(0.0, 1.0)),
        evidence,
    }
}

/// Match OCR text against the knowledge base. Returns candidates sorted by
/// descending confidence (best first), each with explainable evidence.
pub fn match_entities(
    kb: &KnowledgeBase,
    text: &str,
    screen: ScreenKind,
    fuzzy_threshold: f64,
) -> Vec<EntityMatch> {
    let norm = fruit::normalize(text);
    if norm.is_empty() {
        return Vec::new();
    }
    // Stage 0: banner templates out first (recorded as evidence downstream).
    let (residual, templates) = strip_ui_templates(kb, &norm);
    // Stage 1: whole-name, longest-span resolution. When the whole text IS a
    // template (empty residual), the phrase stage has nothing to claim and the
    // per-word stage sees the ORIGINAL text, preserving the long-standing
    // behaviour that a bare banner name resolves to its ui_term entity.
    if !residual.is_empty() {
        let spans = longest_span_matches(kb, &residual);
        match phrase_outcome(kb, &spans) {
            PhraseOutcome::Resolved { entity_id } => {
                let Some(entity) = kb.entities().iter().find(|e| e.id == entity_id) else {
                    debug_assert!(false, "phrase stage emitted id outside the KB: {entity_id}");
                    return Vec::new();
                };
                return vec![entity_match_for(
                    entity,
                    screen,
                    1.0,
                    Evidence {
                        kind: EVIDENCE_EXACT_PHRASE.to_string(),
                        detail: format!(
                            "the whole name '{residual}' is one canonical entity, matched as a phrase"
                        ),
                        weight: 1.0,
                    },
                    &templates,
                )];
            }
            PhraseOutcome::Ambiguous { competitor_ids } => {
                let mut out: Vec<EntityMatch> = competitor_ids
                    .iter()
                    .filter_map(|id| kb.entities().iter().find(|e| &e.id == id))
                    .map(|entity| {
                        entity_match_for(
                            entity,
                            screen,
                            1.0,
                            Evidence {
                                kind: EVIDENCE_AMBIGUOUS_PHRASE.to_string(),
                                detail: format!(
                                    "'{residual}' matches {} distinct entities ({}); no single choice is made",
                                    competitor_ids.len(),
                                    competitor_ids.join(", ")
                                ),
                                weight: 1.0,
                            },
                            &templates,
                        )
                    })
                    .collect();
                out.sort_by(|a, b| {
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| a.entity_id.cmp(&b.entity_id))
                });
                out.truncate(5);
                return out;
            }
            PhraseOutcome::NoMatch => {}
        }
    }
    // Stage 2: the EXISTING per-word stage, unchanged, as the fallback for
    // single-word entities and garbled OCR.
    let stage_text = if residual.is_empty() {
        norm.as_str()
    } else {
        residual.as_str()
    };
    let words: Vec<&str> = stage_text
        .split_whitespace()
        .filter(|w| w.len() >= 3)
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(f32, EntityMatch)> = Vec::new();
    for entity in kb.entities() {
        let mut best: Option<(f32, Evidence)> = None;
        for w in &words {
            if let Some((score, kind, detail)) = score_word(w, entity, fuzzy_threshold) {
                let ev = Evidence {
                    kind,
                    detail,
                    weight: score,
                };
                if best.as_ref().map_or(true, |(b, _)| score > *b) {
                    best = Some((score, ev));
                }
            }
        }
        let Some((mut score, primary)) = best else {
            continue;
        };
        let mut evidence = vec![primary];
        // Stripped banner templates are recorded, never silently dropped.
        for (tmpl, id) in &templates {
            evidence.push(Evidence {
                kind: EVIDENCE_UI_TEMPLATE.to_string(),
                detail: format!("banner template '{tmpl}' ({id}) stripped before lookup"),
                weight: 0.0,
            });
        }
        // Knowledge existence: the entity is real (not an OCR phantom).
        evidence.push(Evidence {
            kind: "knowledge_match".to_string(),
            detail: format!(
                "'{}' exists in knowledge base ({})",
                entity.canonical_name, entity.provenance.source
            ),
            weight: 0.05,
        });
        score += 0.05;
        // Context compatibility.
        if screen.plausible(entity.category) {
            evidence.push(Evidence {
                kind: "context_compatible".to_string(),
                detail: format!("{:?} is plausible on {:?}", entity.category, screen),
                weight: 0.03,
            });
            score += 0.03;
        } else {
            evidence.push(Evidence {
                kind: "context_mismatch".to_string(),
                detail: format!("{:?} is unusual on {:?}", entity.category, screen),
                weight: -0.25,
            });
            score -= 0.25;
        }
        out.push((
            score,
            EntityMatch {
                entity_id: entity.id.clone(),
                canonical_name: entity.canonical_name.clone(),
                category: entity.category.as_str().to_string(),
                confidence: round2(score.clamp(0.0, 1.0)),
                evidence,
            },
        ));
    }
    // Sort on the UNCLAMPED score. Confidence is displayed at 2dp by design,
    // but a fuzzy 0.93 and an exact 1.00 both clamp to 1.0, and sorting on the
    // clamped value let insertion order hand the win to the fuzzy match
    // (measured: `Anglerfish` resolved to fish:angelfish, not fish:anglerfish).
    // Equal raw scores keep the KB's own entity order (the stable ordering the
    // clamped sort produced), so alias-linked identities resolve as before.
    out.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                let pos = |id: &str| {
                    kb.entities()
                        .iter()
                        .position(|e| e.id == id)
                        .unwrap_or(usize::MAX)
                };
                pos(&a.1.entity_id).cmp(&pos(&b.1.entity_id))
            })
    });
    let mut out: Vec<EntityMatch> = out.into_iter().map(|(_, m)| m).collect();
    out.truncate(5);
    out
}

/// Fuse OCR text (+ optional vision hint) into one observation.
/// `confirm_threshold` (e.g. 0.80): top candidate at/above confirms, else
/// `Unknown` with a reason. Vision agreement adds weight; disagreement is
/// recorded but never silently overrides text evidence.
pub fn correlate_text(
    kb: &KnowledgeBase,
    text: &str,
    region: &str,
    screen: ScreenKind,
    fuzzy_threshold: f64,
    confirm_threshold: f32,
    vision_hint: Option<VisionHint>,
) -> Observation {
    let mut candidates = match_entities(kb, text, screen, fuzzy_threshold);

    // Phrase-level ambiguity is fail-closed and checked FIRST: neither the
    // confirm threshold nor vision fusion may pick one of the competitors.
    // Falling through to a confidence comparison here is precisely what
    // produced the original stem contamination.
    let competitors: Vec<String> = candidates
        .iter()
        .filter(|c| {
            c.evidence
                .iter()
                .any(|e| e.kind == EVIDENCE_AMBIGUOUS_PHRASE)
        })
        .map(|c| c.entity_id.clone())
        .collect();
    if !competitors.is_empty() {
        return Observation {
            timestamp_ms: now_ms(),
            source: ObservationSource::Ocr,
            region: region.to_string(),
            screen,
            ocr: Some(TextReading {
                text: text.to_string(),
                region: region.to_string(),
            }),
            vision_hint,
            confidence: candidates.first().map(|c| c.confidence).unwrap_or(0.0),
            entity: None,
            candidates,
            unknown_reason: Some(format!(
                "AMBIGUOUS: the name matches {} distinct entities ({}); no single choice is made.",
                competitors.len(),
                competitors.join(", ")
            )),
        };
    }

    // Vision fusion: a hint naming the same entity (normalized contains)
    // adds bounded weight and is recorded as evidence either way.
    if let Some(hint) = vision_hint.clone() {
        let hn = fruit::normalize(&hint.label);
        for c in candidates.iter_mut() {
            let cn = fruit::normalize(&c.canonical_name);
            if !hn.is_empty() && (hn.contains(&cn) || cn.contains(&hn)) {
                let w = (hint.confidence * 0.15).min(0.12);
                c.evidence.push(Evidence {
                    kind: "visual_agreement".to_string(),
                    detail: format!(
                        "vision '{}' ({:.2}) agrees with '{}'",
                        hint.label, hint.confidence, c.canonical_name
                    ),
                    weight: w,
                });
                c.confidence = round2((c.confidence + w).min(1.0));
            } else {
                c.evidence.push(Evidence {
                    kind: "visual_mismatch".to_string(),
                    detail: format!(
                        "vision '{}' does not match '{}'",
                        hint.label, c.canonical_name
                    ),
                    weight: -0.10,
                });
                c.confidence = round2((c.confidence - 0.10).max(0.0));
            }
        }
        candidates.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    let top = candidates.first().cloned();
    match top {
        Some(m) if m.confidence >= confirm_threshold => Observation {
            timestamp_ms: now_ms(),
            source: ObservationSource::Ocr,
            region: region.to_string(),
            screen,
            ocr: Some(TextReading {
                text: text.to_string(),
                region: region.to_string(),
            }),
            vision_hint,
            confidence: m.confidence,
            entity: Some(m.clone()),
            candidates,
            unknown_reason: None,
        },
        _ => {
            let reason = if candidates.is_empty() {
                format!("No knowledge-base entity matches OCR '{text}' (region {region}).")
            } else {
                let best = &candidates[0];
                format!(
                    "Best candidate '{}' confidence {:.2} below threshold {:.2}: weak/ambiguous evidence.",
                    best.canonical_name, best.confidence, confirm_threshold
                )
            };
            Observation {
                timestamp_ms: now_ms(),
                source: ObservationSource::Ocr,
                region: region.to_string(),
                screen,
                ocr: Some(TextReading {
                    text: text.to_string(),
                    region: region.to_string(),
                }),
                vision_hint,
                confidence: candidates.first().map(|c| c.confidence).unwrap_or(0.0),
                entity: None,
                candidates,
                unknown_reason: Some(reason),
            }
        }
    }
}

/// Fuse an OCR-based observation with an ML vision observation of the same
/// region into one verdict, implementing the agreement table:
///
/// ```text
/// OCR = X, ML = X (agree)            → confirm X (confidence boosted)
/// OCR = X, ML = unknown              → use OCR + KB (ML abstains)
/// OCR = unknown, ML = X + KB valid   → use ML (OCR abstained)
/// OCR = X, ML = Y (disagree)         → UNCERTAIN (never blindly pick one)
/// both unknown                       → UNKNOWN
/// ```
///
/// `ml` is `None` when no model observation exists (model unavailable or
/// region unsupported): fusion degrades honestly to the OCR branch.
pub fn fuse_observations(ocr: &Observation, ml: Option<&Observation>) -> Observation {
    let mut fused = ocr.clone();
    fused.source = ObservationSource::Vision;
    let Some(ml) = ml else {
        return fused;
    };
    let ocr_name = ocr.entity.as_ref().map(|m| m.canonical_name.clone());
    let ml_name = ml.entity.as_ref().map(|m| m.canonical_name.clone());
    match (ocr_name, ml_name) {
        (Some(o), Some(m)) if o == m => {
            // Agreement: boost, keep both evidence chains.
            if let Some(ent) = fused.entity.as_mut() {
                ent.confidence = round2((ent.confidence + 0.05).min(1.0));
                ent.evidence.push(Evidence {
                    kind: "ml_agreement".to_string(),
                    detail: format!("ML independently reports '{m}'"),
                    weight: 0.05,
                });
            }
            fused
        }
        (Some(_), None) => {
            // ML abstains: OCR + KB stands, abstention recorded.
            if let Some(ent) = fused.entity.as_mut() {
                ent.evidence.push(Evidence {
                    kind: "ml_abstain".to_string(),
                    detail: "ML produced no observation; OCR+knowledge verdict kept".to_string(),
                    weight: 0.0,
                });
            }
            fused
        }
        (None, Some(_)) => {
            // OCR abstained, ML reports: adopt ML verdict (it carries its own
            // evidence), marked as ML-sourced.
            let mut adopted = ml.clone();
            adopted.source = ObservationSource::Vision;
            adopted.region.clone_from(&fused.region);
            adopted
        }
        (Some(_), Some(_)) => {
            // Disagreement with both sides claiming: UNCERTAIN, never a coin
            // flip. Keep OCR candidates for inspection.
            fused.entity = None;
            fused.confidence = round2(
                ocr.candidates
                    .first()
                    .map(|c| c.confidence)
                    .unwrap_or(0.0)
                    .min(ml.candidates.first().map(|c| c.confidence).unwrap_or(0.0)),
            );
            fused.unknown_reason = Some(format!(
                "UNCERTAIN: OCR reports '{}' but ML reports '{}' — disagreeing evidence, manual review needed.",
                ocr.entity.as_ref().map(|m| m.canonical_name.as_str()).unwrap_or("?"),
                ml.entity.as_ref().map(|m| m.canonical_name.as_str()).unwrap_or("?"),
            ));
            fused
        }
        (None, None) => fused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::knowledge::{EntityProvenance, ImportEntity};

    fn kb() -> &'static KnowledgeBase {
        KnowledgeBase::bundled()
    }

    /// The bundled KB PLUS the 15 human-confirmed species, mirroring the
    /// persisted overlay (`knowledge_custom.json`) the app merges at runtime.
    /// Building it here keeps the tests hermetic: no AppData, no filesystem.
    fn kb_with_species() -> KnowledgeBase {
        let mut kb = KnowledgeBase::bundled().clone();
        let species = [
            ("anglerfish", "Anglerfish"),
            ("bluelipgrouper", "Blue-Lip Grouper"),
            ("candycornsquid", "Candy Corn Squid"),
            ("crimsonpolkapuffer", "Crimson Polka Puffer"),
            ("crimsonsnapper", "Crimson Snapper"),
            ("darkskeletalshark", "Dark Skeletal Shark"),
            ("exotictigerfin", "Exotic Tigerfin"),
            ("fangfish", "Fangfish"),
            ("goldenpolkapuffer", "Golden Polka Puffer"),
            ("goldenribbonangelfish", "Golden Ribbon Angelfish"),
            ("goldentigerfin", "Golden Tigerfin"),
            ("jackobite", "Jack-O'-Bite"),
            ("skeletalshark", "Skeletal Shark"),
            ("tigerfin", "Tigerfin"),
            ("zebraribbonangelfish", "Zebra Ribbon Angelfish"),
        ];
        let imports: Vec<ImportEntity> = species
            .iter()
            .map(|(id, name)| ImportEntity {
                id: format!("fish:{id}"),
                canonical_name: name.to_string(),
                aliases: vec![],
                category: "fish".to_string(),
                rarity: None,
                description: None,
                ocr_aliases: vec![],
                visual_refs: vec![],
                provenance: EntityProvenance {
                    source: "test:confirmed-species".to_string(),
                    url: Some(
                        "https://grand-piece-online.fandom.com/wiki/Fishing/Bestiary".to_string(),
                    ),
                    updated: None,
                },
            })
            .collect();
        let report = kb.merge_import(&imports, false);
        assert_eq!(report.added, 15, "test KB must carry all 15 species");
        kb
    }

    #[test]
    fn exact_ocr_confirms_with_evidence() {
        let o = correlate_text(
            kb(),
            "You fished up a Suna fruit",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        let m = o.entity.expect("Suna must confirm");
        assert_eq!(m.canonical_name, "Suna");
        assert!(m.confidence >= 0.80);
        assert!(m.evidence.iter().any(|e| e.kind == "exact_ocr"));
        assert!(m.evidence.iter().any(|e| e.kind == "knowledge_match"));
        assert!(o.unknown_reason.is_none());
    }

    #[test]
    fn ocr_error_fusion_confirms_kraken_example() {
        // "KRAK3N": cleaned (3→? no; KRAK3N vs kraken fuzzy) + knowledge +
        // context must confirm the real entity, never invent one.
        let o = correlate_text(
            kb(),
            "KRAK3N",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        let m = o
            .entity
            .expect("kraken must confirm via fuzzy+knowledge+context");
        assert_eq!(m.canonical_name.to_ascii_lowercase(), "kraken");
        assert!(o.unknown_reason.is_none());
    }

    #[test]
    fn weak_evidence_yields_unknown_never_hallucination() {
        let o = correlate_text(
            kb(),
            "xqz wobble fnord",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        assert!(o.entity.is_none());
        assert!(o
            .unknown_reason
            .as_deref()
            .unwrap_or("")
            .contains("No knowledge-base entity"));
        assert!(o.confidence < 0.80);
    }

    #[test]
    fn below_threshold_candidate_stays_unknown_with_reason() {
        // A short ambiguous fragment must not confirm even if fuzzy fire.
        let o = correlate_text(kb(), "sna", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
        assert!(
            o.entity.is_none(),
            "3-letter fragment must not confirm Suna"
        );
    }

    #[test]
    fn alias_resolves_with_alias_evidence() {
        let o = correlate_text(
            kb(),
            "got Phoenix fruit",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        let m = o.entity.expect("phoenix alias must confirm");
        assert_eq!(m.canonical_name, "Tori");
        assert!(m.evidence.iter().any(|e| e.kind == "alias"));
    }

    #[test]
    fn vision_agreement_raises_and_mismatch_lowers() {
        let agree = VisionHint {
            label: "Suna".into(),
            confidence: 0.9,
        };
        let o1 = correlate_text(
            kb(),
            "Suna fruit",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            Some(agree),
        );
        let disagree = VisionHint {
            label: "Boot".into(),
            confidence: 0.9,
        };
        let o2 = correlate_text(
            kb(),
            "Suna fruit",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            Some(disagree),
        );
        let c1 = o1.entity.map(|m| m.confidence).unwrap_or(0.0);
        assert!(o1
            .candidates
            .iter()
            .any(|c| c.evidence.iter().any(|e| e.kind == "visual_agreement")));
        assert!(o2
            .candidates
            .iter()
            .any(|c| c.evidence.iter().any(|e| e.kind == "visual_mismatch")));
        assert!(c1 >= o2.confidence);
    }

    #[test]
    fn context_mismatch_penalizes_implausible_category() {
        let bait = correlate_text(
            kb(),
            "Legendary Bait",
            "bait_menu",
            ScreenKind::BaitMenu,
            0.85,
            0.80,
            None,
        );
        assert!(bait.entity.is_some());
        let off = correlate_text(
            kb(),
            "Legendary Bait",
            "bait_menu",
            ScreenKind::SpawnBanner,
            0.85,
            0.80,
            None,
        );
        // Same text, wrong screen: confidence must drop (may still confirm
        // on strong text evidence, but strictly lower).
        assert!(off.confidence <= bait.confidence);
        assert!(off
            .candidates
            .iter()
            .any(|c| c.evidence.iter().any(|e| e.kind == "context_mismatch")));
    }

    #[test]
    fn confidence_has_no_fake_precision() {
        let o = correlate_text(
            kb(),
            "Mera Mera fruit drop",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        let c = o.confidence;
        assert_eq!(c, (c * 100.0).round() / 100.0);
        assert!(c <= 1.0);
    }

    #[test]
    fn ocr_fallback_prefers_raw_and_retries_upscale() {
        use crate::core::types::Frame;
        let frame = Frame::new(100, 40, vec![0u8; 100 * 40 * 4]);
        let (t, v) = ocr_with_fallback(|_| Ok("Suna".to_string()), &frame).unwrap();
        assert_eq!((t.as_str(), v), ("Suna", "raw"));
        let (t2, v2) = ocr_with_fallback(
            |f| {
                Ok(if f.w > 100 {
                    "Bari".to_string()
                } else {
                    "   ".to_string()
                })
            },
            &frame,
        )
        .unwrap();
        assert_eq!((t2.as_str(), v2), ("Bari", "upscale2x"));
        let (t3, v3) = ocr_with_fallback(|_| Ok("   ".to_string()), &frame).unwrap();
        assert_eq!((t3.trim(), v3), ("", "raw"));
    }

    fn ml_obs(name: Option<&str>, conf: f32) -> Observation {
        Observation {
            timestamp_ms: 1,
            source: crate::core::perception::ObservationSource::Vision,
            region: "drop".into(),
            screen: ScreenKind::Fishing,
            ocr: None,
            vision_hint: None,
            candidates: vec![],
            confidence: conf,
            entity: name.map(|n| EntityMatch {
                entity_id: format!("fruit:{}", n.to_ascii_lowercase()),
                canonical_name: n.into(),
                category: "fruit".into(),
                confidence: conf,
                evidence: vec![],
            }),
            unknown_reason: if name.is_none() {
                Some("ml unknown".into())
            } else {
                None
            },
        }
    }

    #[test]
    fn fusion_agreement_confirms_with_boost() {
        let ocr = correlate_text(
            kb(),
            "Suna fruit",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        // Cap: agreement boost never exceeds 1.0.
        let fused = fuse_observations(&ocr, Some(&ml_obs(Some("Suna"), 0.91)));
        let m = fused.entity.expect("agreement confirms");
        assert_eq!(m.canonical_name, "Suna");
        assert!(m.evidence.iter().any(|e| e.kind == "ml_agreement"));
    }

    #[test]
    fn fusion_ml_abstain_keeps_ocr() {
        let ocr = correlate_text(
            kb(),
            "Suna fruit",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        let fused = fuse_observations(&ocr, Some(&ml_obs(None, 0.0)));
        assert_eq!(fused.entity.unwrap().canonical_name, "Suna");
    }

    #[test]
    fn fusion_no_ml_degrades_to_ocr() {
        let ocr = correlate_text(
            kb(),
            "Suna fruit",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        let fused = fuse_observations(&ocr, None);
        assert_eq!(fused.entity.unwrap().canonical_name, "Suna");
    }

    #[test]
    fn fusion_ocr_abstain_uses_ml() {
        let ocr = correlate_text(
            kb(),
            "xqz wobble",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        assert!(ocr.entity.is_none());
        let fused = fuse_observations(&ocr, Some(&ml_obs(Some("Kraken"), 0.91)));
        assert_eq!(fused.entity.unwrap().canonical_name, "Kraken");
    }

    #[test]
    fn fusion_disagreement_is_uncertain_never_coin_flip() {
        let ocr = correlate_text(
            kb(),
            "Suna fruit",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        let fused = fuse_observations(&ocr, Some(&ml_obs(Some("Mera"), 0.91)));
        assert!(fused.entity.is_none(), "disagreement must not pick a side");
        assert!(fused
            .unknown_reason
            .as_deref()
            .unwrap_or("")
            .contains("UNCERTAIN"));
    }

    #[test]
    fn fusion_both_unknown_stays_unknown() {
        let ocr = correlate_text(
            kb(),
            "xqz wobble",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        let fused = fuse_observations(&ocr, Some(&ml_obs(None, 0.0)));
        assert!(fused.entity.is_none());
    }

    #[test]
    fn empty_text_is_unknown() {
        let o = correlate_text(kb(), "   ", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
        assert!(o.entity.is_none());
        assert_eq!(o.confidence, 0.0);
    }

    // ------------------------------------------------------------------
    // Phrase-first, longest-span resolution
    // ------------------------------------------------------------------

    #[test]
    fn confirmed_species_names_resolve_to_the_species() {
        let kb = kb_with_species();
        for (text, want) in [
            ("New Item <Crimson Snapper>", "fish:crimsonsnapper"),
            ("Crimson Snapper", "fish:crimsonsnapper"),
            ("Candy Corn Squid", "fish:candycornsquid"),
            ("Skeletal Shark", "fish:skeletalshark"),
            ("Exotic Tigerfin", "fish:exotictigerfin"),
            ("Zebra Ribbon Angelfish", "fish:zebraribbonangelfish"),
            ("Blue-Lip Grouper", "fish:bluelipgrouper"),
            ("Golden Tigerfin", "fish:goldentigerfin"),
            ("Jack-O'-Bite", "fish:jackobite"),
            ("Fangfish", "fish:fangfish"),
            ("Swordfish", "fish:swordfish"),
        ] {
            let o = correlate_text(&kb, text, "drop", ScreenKind::Fishing, 0.85, 0.80, None);
            let m = o.entity.unwrap_or_else(|| panic!("{text} must resolve"));
            assert_eq!(m.entity_id, want, "{text}");
            assert!(o.unknown_reason.is_none(), "{text}");
        }
    }

    #[test]
    fn a_compound_species_name_offers_no_stem_candidates() {
        let kb = kb_with_species();
        let candidates = match_entities(&kb, "Crimson Snapper", ScreenKind::Fishing, 0.85);
        assert_eq!(candidates.len(), 1, "one identity, no competing stems");
        assert_eq!(candidates[0].entity_id, "fish:crimsonsnapper");
        assert!(!candidates.iter().any(|c| c.entity_id == "fish:crimson"));
        assert!(!candidates.iter().any(|c| c.entity_id == "fish:snapper"));
        assert!(candidates[0]
            .evidence
            .iter()
            .any(|e| e.kind == EVIDENCE_EXACT_PHRASE));
    }

    #[test]
    fn the_bundled_kb_fails_closed_on_a_compound_species_name() {
        // Without the species (the pre-patch KB), the honest answer is
        // AMBIGUOUS between the two stems, never a silent pick of either one.
        let o = correlate_text(
            kb(),
            "Crimson Snapper",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        assert!(o.entity.is_none(), "must not collapse onto a stem");
        let reason = o.unknown_reason.unwrap_or_default();
        assert!(
            reason.contains("fish:crimson") && reason.contains("fish:snapper"),
            "{reason}"
        );
    }

    #[test]
    fn genuine_ambiguity_is_unknown_naming_the_competitors() {
        let kb = kb_with_species();
        let o = correlate_text(
            &kb,
            "Swordfish Suna",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        assert!(o.entity.is_none());
        let reason = o.unknown_reason.unwrap_or_default();
        assert!(
            reason.contains("fish:swordfish") && reason.contains("fruit:suna"),
            "{reason}"
        );
        // Both competitors are inspectable as candidates, none confirmed.
        assert!(o.candidates.iter().any(|c| c.entity_id == "fish:swordfish"));
        assert!(o.candidates.iter().any(|c| c.entity_id == "fruit:suna"));
        assert!(o.candidates.iter().all(|c| c
            .evidence
            .iter()
            .any(|e| e.kind == EVIDENCE_AMBIGUOUS_PHRASE)));
    }

    #[test]
    fn a_same_span_tie_between_unrelated_identities_is_unknown() {
        // Two entities whose OCR alias is the SAME name, neither claiming the
        // other's canonical name: a genuine tie with no honest winner.
        let mut kb = kb_with_species();
        let tie = |id: &str, canonical: &str| ImportEntity {
            id: id.to_string(),
            canonical_name: canonical.to_string(),
            aliases: vec![],
            category: "item".to_string(),
            rarity: None,
            description: None,
            ocr_aliases: vec!["Shared Trophy".to_string()],
            visual_refs: vec![],
            provenance: EntityProvenance {
                source: "test".into(),
                url: None,
                updated: None,
            },
        };
        kb.merge_import(
            &[tie("item:tie-a", "Tie A"), tie("item:tie-b", "Tie B")],
            false,
        );
        let o = correlate_text(
            &kb,
            "Shared Trophy",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        assert!(o.entity.is_none());
        let reason = o.unknown_reason.unwrap_or_default();
        assert!(
            reason.contains("item:tie-a") && reason.contains("item:tie-b"),
            "{reason}"
        );
    }

    #[test]
    fn ui_templates_are_stripped_before_lookup_and_recorded() {
        let kb = kb_with_species();
        let o = correlate_text(
            &kb,
            "New Item <Shark>",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        let m = o.entity.expect("shark must resolve");
        assert_eq!(m.entity_id, "fish:shark");
        assert!(!o.candidates.iter().any(|c| c.entity_id == "ui:new-item"));
        assert!(m
            .evidence
            .iter()
            .any(|e| e.kind == EVIDENCE_UI_TEMPLATE && e.detail.contains("ui:new-item")));
    }

    #[test]
    fn gibberish_with_a_banner_stays_unknown_with_a_reason() {
        let kb = kb_with_species();
        let o = correlate_text(
            &kb,
            "New Item <zzq wobble>",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        assert!(o.entity.is_none());
        assert!(o.unknown_reason.is_some());
    }

    #[test]
    fn every_returned_id_exists_in_the_kb() {
        let kb = kb_with_species();
        for text in [
            "Crimson Snapper",
            "Swordfish Suna",
            "New Item <Shark>",
            "Mera Mera",
            "Suna fruit",
            "got Phoenix fruit",
            "KRAK3N",
            "xqz wobble",
        ] {
            for c in match_entities(&kb, text, ScreenKind::Fishing, 0.85) {
                assert!(
                    kb.entities().iter().any(|e| e.id == c.entity_id),
                    "{} returned {} which is not in the KB",
                    text,
                    c.entity_id
                );
            }
        }
    }

    #[test]
    fn resolution_is_deterministic() {
        let kb = kb_with_species();
        for text in ["Crimson Snapper", "Swordfish Suna", "Mera Mera fruit drop"] {
            let a = match_entities(&kb, text, ScreenKind::Fishing, 0.85);
            let b = match_entities(&kb, text, ScreenKind::Fishing, 0.85);
            let ids_a: Vec<&str> = a.iter().map(|c| c.entity_id.as_str()).collect();
            let ids_b: Vec<&str> = b.iter().map(|c| c.entity_id.as_str()).collect();
            assert_eq!(ids_a, ids_b, "{text} must resolve identically every call");
        }
    }

    #[test]
    fn alias_linked_identities_are_not_phantom_ambiguity() {
        // The KB models Tori and Phoenix as two entities for one in-game
        // identity. That is a linked pair, never a competitor set.
        let o = correlate_text(
            kb(),
            "Tori Phoenix",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        assert!(
            o.entity.is_some(),
            "linked identities must not be ambiguous"
        );
    }

    #[test]
    fn single_word_names_keep_their_existing_evidence_kinds() {
        // The phrase stage must not swallow single-word resolutions: Suna
        // still carries exact_ocr (not exact_phrase), so downstream evidence
        // reading is unchanged.
        let o = correlate_text(
            kb(),
            "Suna fruit",
            "drop",
            ScreenKind::Fishing,
            0.85,
            0.80,
            None,
        );
        let m = o.entity.expect("Suna must confirm");
        assert!(m.evidence.iter().any(|e| e.kind == "exact_ocr"));
        assert!(!m.evidence.iter().any(|e| e.kind == EVIDENCE_EXACT_PHRASE));
    }

    #[test]
    fn a_pure_banner_template_still_resolves_to_its_ui_entity() {
        // A text that IS a template names the template's ui entity (the
        // residual is empty, so the per-word stage sees the original text).
        // `spawn` is EXACT for ui:spawn and fuzzy for ui:spawned; the unclamped
        // sort now ranks the exact match first, which is the semantically
        // correct answer (the old clamp-then-insertion order was luck).
        let o = correlate_text(kb(), "spawn", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
        assert_eq!(o.entity.map(|m| m.entity_id).as_deref(), Some("ui:spawn"));
    }
}
