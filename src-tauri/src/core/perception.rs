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
                EntityCategory::Fish | EntityCategory::Fruit | EntityCategory::Item | EntityCategory::UiTerm
            ),
            ScreenKind::BaitMenu => matches!(category, EntityCategory::Bait | EntityCategory::UiTerm),
            ScreenKind::Shop => matches!(
                category,
                EntityCategory::Bait | EntityCategory::Item | EntityCategory::UiTerm
            ),
            ScreenKind::SpawnBanner => matches!(
                category,
                EntityCategory::Fruit | EntityCategory::Location | EntityCategory::UiTerm | EntityCategory::Npc
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
fn score_word(word: &str, entity: &GpoEntity, fuzzy_threshold: f64) -> Option<(f32, String, String)> {
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
            return Some((1.0, kind.to_string(), format!("'{word}' matches {kind} '{n}'")));
        }
    }
    // 2. OCR-cleaned exact (0→o, 1|l→i, @→a, 5→s).
    let cleaned = fruit::clean_ocr_word(word);
    if cleaned != lower {
        for (n, canonical) in &names {
            if n.eq_ignore_ascii_case(&cleaned) {
                let kind = if *canonical { "normalized_ocr" } else { "alias" };
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

/// Match OCR text against the knowledge base. Returns candidates sorted by
/// descending confidence (best first), each with explainable evidence.
pub fn match_entities(
    kb: &KnowledgeBase,
    text: &str,
    screen: ScreenKind,
    fuzzy_threshold: f64,
) -> Vec<EntityMatch> {
    let norm = fruit::normalize(text);
    let words: Vec<&str> = norm.split_whitespace().filter(|w| w.len() >= 3).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<EntityMatch> = Vec::new();
    for entity in kb.entities() {
        let mut best: Option<(f32, Evidence)> = None;
        for w in &words {
            if let Some((score, kind, detail)) = score_word(w, entity, fuzzy_threshold) {
                let ev = Evidence { kind, detail, weight: score };
                if best.as_ref().map_or(true, |(b, _)| score > *b) {
                    best = Some((score, ev));
                }
            }
        }
        let Some((mut score, primary)) = best else { continue };
        let mut evidence = vec![primary];
        // Knowledge existence: the entity is real (not an OCR phantom).
        evidence.push(Evidence {
            kind: "knowledge_match".to_string(),
            detail: format!("'{}' exists in knowledge base ({})", entity.canonical_name, entity.provenance.source),
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
        out.push(EntityMatch {
            entity_id: entity.id.clone(),
            canonical_name: entity.canonical_name.clone(),
            category: entity.category.as_str().to_string(),
            confidence: round2(score.clamp(0.0, 1.0)),
            evidence,
        });
    }
    out.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
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
                    detail: format!("vision '{}' ({:.2}) agrees with '{}'", hint.label, hint.confidence, c.canonical_name),
                    weight: w,
                });
                c.confidence = round2((c.confidence + w).min(1.0));
            } else {
                c.evidence.push(Evidence {
                    kind: "visual_mismatch".to_string(),
                    detail: format!("vision '{}' does not match '{}'", hint.label, c.canonical_name),
                    weight: -0.10,
                });
                c.confidence = round2((c.confidence - 0.10).max(0.0));
            }
        }
        candidates.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
    }

    let top = candidates.first().cloned();
    match top {
        Some(m) if m.confidence >= confirm_threshold => Observation {
            timestamp_ms: now_ms(),
            source: ObservationSource::Ocr,
            region: region.to_string(),
            screen,
            ocr: Some(TextReading { text: text.to_string(), region: region.to_string() }),
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
                ocr: Some(TextReading { text: text.to_string(), region: region.to_string() }),
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
                ocr.candidates.first().map(|c| c.confidence).unwrap_or(0.0).min(
                    ml.candidates.first().map(|c| c.confidence).unwrap_or(0.0),
                ),
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

    fn kb() -> &'static KnowledgeBase {
        KnowledgeBase::bundled()
    }

    #[test]
    fn exact_ocr_confirms_with_evidence() {
        let o = correlate_text(kb(), "You fished up a Suna fruit", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
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
        let o = correlate_text(kb(), "KRAK3N", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
        let m = o.entity.expect("kraken must confirm via fuzzy+knowledge+context");
        assert_eq!(m.canonical_name.to_ascii_lowercase(), "kraken");
        assert!(o.unknown_reason.is_none());
    }

    #[test]
    fn weak_evidence_yields_unknown_never_hallucination() {
        let o = correlate_text(kb(), "xqz wobble fnord", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
        assert!(o.entity.is_none());
        assert!(o.unknown_reason.as_deref().unwrap_or("").contains("No knowledge-base entity"));
        assert!(o.confidence < 0.80);
    }

    #[test]
    fn below_threshold_candidate_stays_unknown_with_reason() {
        // A short ambiguous fragment must not confirm even if fuzzy fire.
        let o = correlate_text(kb(), "sna", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
        assert!(o.entity.is_none(), "3-letter fragment must not confirm Suna");
    }

    #[test]
    fn alias_resolves_with_alias_evidence() {
        let o = correlate_text(kb(), "got Phoenix fruit", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
        let m = o.entity.expect("phoenix alias must confirm");
        assert_eq!(m.canonical_name, "Tori");
        assert!(m.evidence.iter().any(|e| e.kind == "alias"));
    }

    #[test]
    fn vision_agreement_raises_and_mismatch_lowers() {
        let agree = VisionHint { label: "Suna".into(), confidence: 0.9 };
        let o1 = correlate_text(kb(), "Suna fruit", "drop", ScreenKind::Fishing, 0.85, 0.80, Some(agree));
        let disagree = VisionHint { label: "Boot".into(), confidence: 0.9 };
        let o2 = correlate_text(kb(), "Suna fruit", "drop", ScreenKind::Fishing, 0.85, 0.80, Some(disagree));
        let c1 = o1.entity.map(|m| m.confidence).unwrap_or(0.0);
        assert!(o1.candidates.iter().any(|c| c.evidence.iter().any(|e| e.kind == "visual_agreement")));
        assert!(o2.candidates.iter().any(|c| c.evidence.iter().any(|e| e.kind == "visual_mismatch")));
        assert!(c1 >= o2.confidence);
    }

    #[test]
    fn context_mismatch_penalizes_implausible_category() {
        let bait = correlate_text(kb(), "Legendary Bait", "bait_menu", ScreenKind::BaitMenu, 0.85, 0.80, None);
        assert!(bait.entity.is_some());
        let off = correlate_text(kb(), "Legendary Bait", "bait_menu", ScreenKind::SpawnBanner, 0.85, 0.80, None);
        // Same text, wrong screen: confidence must drop (may still confirm
        // on strong text evidence, but strictly lower).
        assert!(off.confidence <= bait.confidence);
        assert!(off.candidates.iter().any(|c| c.evidence.iter().any(|e| e.kind == "context_mismatch")));
    }

    #[test]
    fn confidence_has_no_fake_precision() {
        let o = correlate_text(kb(), "Mera Mera fruit drop", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
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
            |f| Ok(if f.w > 100 { "Bari".to_string() } else { "   ".to_string() }),
            &frame,
        )
        .unwrap();
        assert_eq!((t2.as_str(), v2), ("Bari", "upscale2x"));
        let (t3, v3) =
            ocr_with_fallback(|_| Ok("   ".to_string()), &frame).unwrap();
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
            unknown_reason: if name.is_none() { Some("ml unknown".into()) } else { None },
        }
    }

    #[test]
    fn fusion_agreement_confirms_with_boost() {
        let ocr = correlate_text(kb(), "Suna fruit", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
        // Cap: agreement boost never exceeds 1.0.
        let fused = fuse_observations(&ocr, Some(&ml_obs(Some("Suna"), 0.91)));
        let m = fused.entity.expect("agreement confirms");
        assert_eq!(m.canonical_name, "Suna");
        assert!(m.evidence.iter().any(|e| e.kind == "ml_agreement"));
    }

    #[test]
    fn fusion_ml_abstain_keeps_ocr() {
        let ocr = correlate_text(kb(), "Suna fruit", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
        let fused = fuse_observations(&ocr, Some(&ml_obs(None, 0.0)));
        assert_eq!(fused.entity.unwrap().canonical_name, "Suna");
    }

    #[test]
    fn fusion_no_ml_degrades_to_ocr() {
        let ocr = correlate_text(kb(), "Suna fruit", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
        let fused = fuse_observations(&ocr, None);
        assert_eq!(fused.entity.unwrap().canonical_name, "Suna");
    }

    #[test]
    fn fusion_ocr_abstain_uses_ml() {
        let ocr = correlate_text(kb(), "xqz wobble", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
        assert!(ocr.entity.is_none());
        let fused = fuse_observations(&ocr, Some(&ml_obs(Some("Kraken"), 0.91)));
        assert_eq!(fused.entity.unwrap().canonical_name, "Kraken");
    }

    #[test]
    fn fusion_disagreement_is_uncertain_never_coin_flip() {
        let ocr = correlate_text(kb(), "Suna fruit", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
        let fused = fuse_observations(&ocr, Some(&ml_obs(Some("Mera"), 0.91)));
        assert!(fused.entity.is_none(), "disagreement must not pick a side");
        assert!(fused.unknown_reason.as_deref().unwrap_or("").contains("UNCERTAIN"));
    }

    #[test]
    fn fusion_both_unknown_stays_unknown() {
        let ocr = correlate_text(kb(), "xqz wobble", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
        let fused = fuse_observations(&ocr, Some(&ml_obs(None, 0.0)));
        assert!(fused.entity.is_none());
    }

    #[test]
    fn empty_text_is_unknown() {
        let o = correlate_text(kb(), "   ", "drop", ScreenKind::Fishing, 0.85, 0.80, None);
        assert!(o.entity.is_none());
        assert_eq!(o.confidence, 0.0);
    }
}
