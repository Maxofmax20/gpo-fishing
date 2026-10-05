//! RESULT-level entity recognition (§5-9, §22-23 of the v5.3.0 plan).
//!
//! The OCR+KB matcher (`perception::correlate_text`) already produces scored
//! candidates with evidence. This module turns one `Observation` into a
//! RESULT-typed decision with an explicit taxonomy:
//!
//! ```text
//! FISH / DEVIL_FRUIT / BAIT / OTHER / UNKNOWN
//! ```
//!
//! Rules (non-negotiable):
//! - Confidence is the matcher's measured score for the ACCEPTED claim.
//!   UNKNOWN carries confidence 0.0: there is no claim to support. The
//!   rejected best score is kept as evidence, never as confidence.
//! - A UI-phrase match ("devil fruit" banner text) proves an event class,
//!   not an identity: it yields a discounted DevilFruit/Other claim, never
//!   a concrete entity_id.
//! - Nothing here reads pixels or calls a model; when vision evidence
//!   exists it arrives as `VisionHint` through `correlate_text` and is
//!   recorded with its bounded weight. No vision at runtime => no vision
//!   evidence (never synthesized).

use serde::{Deserialize, Serialize};

use super::perception::{Evidence, Observation};

/// Entity type taxonomy for RESULT decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Fish,
    DevilFruit,
    Bait,
    Other,
    Unknown,
}

impl EntityType {
    pub fn as_str(self) -> &'static str {
        match self {
            EntityType::Fish => "fish",
            EntityType::DevilFruit => "fruit",
            EntityType::Bait => "bait",
            EntityType::Other => "other",
            EntityType::Unknown => "unknown",
        }
    }
}

/// Where a decision's support came from. Never manufactured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntitySource {
    Ocr,
    Kb,
    Vision,
    Temporal,
    Hybrid,
}

impl EntitySource {
    pub fn as_str(self) -> &'static str {
        match self {
            EntitySource::Ocr => "ocr",
            EntitySource::Kb => "kb",
            EntitySource::Vision => "vision",
            EntitySource::Temporal => "temporal",
            EntitySource::Hybrid => "hybrid",
        }
    }
}

/// One RESULT entity decision. `confirmed` is ALWAYS false here: confirmation
/// is a workflow event (action + game response), never a property of a
/// single observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultEntity {
    pub event_id: Option<String>,
    pub state: String,
    pub entity_type: EntityType,
    pub entity_id: Option<String>,
    pub entity_name: Option<String>,
    pub confidence: f32,
    pub source: EntitySource,
    pub evidence: Vec<Evidence>,
    pub confirmed: bool,
    pub unknown_reason: Option<String>,
}

fn unknown(event_id: Option<String>, evidence: Vec<Evidence>, reason: String) -> ResultEntity {
    ResultEntity {
        event_id,
        state: "RESULT".to_string(),
        entity_type: EntityType::Unknown,
        entity_id: None,
        entity_name: None,
        confidence: 0.0,
        source: EntitySource::Ocr,
        evidence,
        confirmed: false,
        unknown_reason: Some(reason),
    }
}

/// Decide from one OCR+KB observation. Pure function.
pub fn decide_from_observation(event_id: Option<String>, obs: &Observation) -> ResultEntity {
    let Some(m) = obs.entity.as_ref() else {
        let reason = obs.unknown_reason.clone().unwrap_or_else(|| "no entity matched".to_string());
        return unknown(event_id, obs.candidates.iter().flat_map(|c| c.evidence.clone()).take(3).collect(), reason);
    };
    let (entity_type, discount, note) = match m.category.as_str() {
        "fish" => (EntityType::Fish, 1.0, None),
        "fruit" => (EntityType::DevilFruit, 1.0, None),
        "bait" => (EntityType::Bait, 1.0, None),
        // A UI-phrase match ("devil fruit", "you got") proves an event
        // class, not an identity: discounted claim, no entity_id.
        "ui_term" => {
            let is_fruit_event = ["devil-fruit", "devil-fruit-drop", "got-a-devil-fruit"]
                .iter()
                .any(|k| m.entity_id.contains(k));
            if is_fruit_event {
                (EntityType::DevilFruit, 0.5, Some("ui-phrase proves a fruit event, not which fruit".to_string()))
            } else {
                (EntityType::Other, 0.5, Some("ui-phrase proves an event, not an identity".to_string()))
            }
        }
        _ => (EntityType::Other, 0.5, Some("unmapped KB category: no identity claim".to_string())),
    };
    let mut evidence = m.evidence.clone();
    if let Some(n) = &note {
        evidence.push(Evidence { kind: "type_mapping".to_string(), detail: n.clone(), weight: discount - 1.0 });
    }
    let has_identity = discount >= 1.0;
    ResultEntity {
        event_id,
        state: "RESULT".to_string(),
        entity_type,
        entity_id: if has_identity { Some(m.entity_id.clone()) } else { None },
        entity_name: if has_identity { Some(m.canonical_name.clone()) } else { None },
        confidence: round2(m.confidence * discount),
        source: if obs.vision_hint.is_some() { EntitySource::Hybrid } else { EntitySource::Ocr },
        evidence,
        confirmed: false,
        unknown_reason: None,
    }
}

fn round2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

// ---- temporal aggregation (documented k-of-n, tested) ----

/// Verdict of a temporal evidence window over entity readings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemporalVerdict {
    /// At least `k` readings name the same concrete entity within the window
    /// with no conflicting concrete reading.
    Stable(String),
    /// Conflicting concrete readings inside the window.
    Unstable,
    /// Not enough readings yet.
    Insufficient,
}

/// k-of-n voter over timestamped entity readings (`None` = UNKNOWN frame).
/// A conflicting concrete entity vetoes stability: flip-flopping is
/// evidence AGAINST every candidate, reported as Unstable (never averaged).
#[derive(Debug, Clone)]
pub struct TemporalBuffer {
    k: usize,
    window_ms: u64,
    readings: Vec<(u64, Option<String>)>,
}

impl TemporalBuffer {
    pub fn new(k: usize, window_ms: u64) -> Self {
        Self { k: k.max(1), window_ms, readings: Vec::new() }
    }

    pub fn push(&mut self, entity_id: Option<String>, now_ms: u64) -> TemporalVerdict {
        self.readings.push((now_ms, entity_id));
        let cutoff = now_ms.saturating_sub(self.window_ms);
        self.readings.retain(|(t, _)| *t >= cutoff);
        while self.readings.len() > self.k * 3 {
            self.readings.remove(0);
        }
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for (_, e) in &self.readings {
            if let Some(id) = e {
                *counts.entry(id.as_str()).or_default() += 1;
            }
        }
        if counts.len() > 1 {
            return TemporalVerdict::Unstable;
        }
        match counts.iter().next() {
            Some((id, n)) if *n >= self.k => TemporalVerdict::Stable((*id).to_string()),
            _ => TemporalVerdict::Insufficient,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::knowledge::KnowledgeBase;
    use super::super::perception::{ScreenKind, correlate_text};

    fn kb() -> &'static KnowledgeBase {
        KnowledgeBase::bundled()
    }

    fn obs_for(text: &str) -> Observation {
        correlate_text(kb(), text, "drop", ScreenKind::Fishing, 0.88, 0.80, None)
    }

    #[test]
    fn known_fish_yields_typed_identity() {
        let d = decide_from_observation(None, &obs_for("You caught Golden Fish"));
        assert_eq!(d.entity_type, EntityType::Fish);
        assert_eq!(d.entity_id.as_deref(), Some("fish:golden"));
        assert!(d.confidence >= 0.80, "measured matcher score, got {}", d.confidence);
        assert_eq!(d.source, EntitySource::Ocr);
        assert!(!d.confirmed, "single observations are never confirmed");
        assert!(!d.evidence.is_empty());
    }

    #[test]
    fn known_fruit_yields_typed_identity() {
        let d = decide_from_observation(None, &obs_for("fished up a Suna devil fruit"));
        assert_eq!(d.entity_type, EntityType::DevilFruit);
        assert_eq!(d.entity_id.as_deref(), Some("fruit:suna"));
    }

    #[test]
    fn gibberish_yields_unknown_with_zero_confidence() {
        let d = decide_from_observation(None, &obs_for("zxqwv blorpt 123"));
        assert_eq!(d.entity_type, EntityType::Unknown);
        assert_eq!(d.entity_id, None);
        assert_eq!(d.confidence, 0.0, "UNKNOWN carries no claim, hence no confidence");
        assert!(d.unknown_reason.is_some());
    }

    #[test]
    fn ui_phrase_proves_event_not_identity() {
        // "devil fruit" banner phrase with no name: fruit EVENT, no identity.
        let d = decide_from_observation(None, &obs_for("devil fruit"));
        assert_eq!(d.entity_type, EntityType::DevilFruit);
        assert_eq!(d.entity_id, None, "phrase match must not invent an identity");
        assert!(d.confidence < 0.80, "discounted below confirm threshold");
    }

    #[test]
    fn ambiguous_ocr_stays_unknown() {
        // Fragment that could be several things: below-threshold => UNKNOWN.
        let d = decide_from_observation(None, &obs_for("ish"));
        assert_eq!(d.entity_type, EntityType::Unknown);
    }

    #[test]
    fn temporal_accumulates_weak_readings() {
        let mut buf = TemporalBuffer::new(3, 10_000);
        assert_eq!(buf.push(None, 0), TemporalVerdict::Insufficient);
        assert_eq!(buf.push(None, 100), TemporalVerdict::Insufficient);
        assert_eq!(buf.push(Some("fish:golden".into()), 200), TemporalVerdict::Insufficient);
        assert_eq!(buf.push(Some("fish:golden".into()), 300), TemporalVerdict::Insufficient);
        assert_eq!(
            buf.push(Some("fish:golden".into()), 400),
            TemporalVerdict::Stable("fish:golden".to_string())
        );
    }

    #[test]
    fn temporal_flip_flop_is_unstable_not_averaged() {
        let mut buf = TemporalBuffer::new(2, 10_000);
        buf.push(Some("fish:shark".into()), 0);
        buf.push(Some("fish:shark".into()), 100);
        assert_eq!(buf.push(Some("fish:golden".into()), 200), TemporalVerdict::Unstable);
    }

    #[test]
    fn temporal_window_expires_stale_votes() {
        let mut buf = TemporalBuffer::new(2, 150);
        buf.push(Some("fish:shark".into()), 0);
        buf.push(Some("fish:shark".into()), 100);
        // Both votes expire by t=1000 with a fresh UNKNOWN reading.
        assert_eq!(buf.push(None, 1000), TemporalVerdict::Insufficient);
    }
}
