//! Baseline evaluation: measure the CURRENT pipeline before any model.
//!
//! Stages (text observations; vision stages apply where frames exist):
//!
//! ```text
//! ocr_exact      — raw OCR verdict only (fruit::detect_* rules)
//! ocr_knowledge  — OCR + normalization + GPO knowledge correlation
//! combined       — knowledge verdict, vision-hint fusion when present
//! ```
//!
//! Metrics per stage: accuracy, precision, recall, F1, unknown rate,
//! mean latency. With fewer than [`MIN_LABELED`] labeled samples the report
//! honestly states INSUFFICIENT LABELED DATA instead of fabricating numbers.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

use crate::core::fruit;
use crate::core::knowledge::KnowledgeBase;
use crate::core::ml_dataset::{MlAnnotation, MlDatasetStore};
use crate::core::perception::{correlate_text, ScreenKind};

/// Minimum labeled samples before metrics are reported.
pub const MIN_LABELED: usize = 10;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StageMetrics {
    pub n: usize,
    pub accuracy: f32,
    pub precision: f32,
    pub recall: f32,
    pub f1: f32,
    pub unknown_rate: f32,
    pub mean_latency_ms: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineReport {
    pub dataset: String,
    pub labeled_samples: usize,
    pub insufficient_data: bool,
    pub stages: HashMap<String, StageMetrics>,
    /// Gold label → predicted label → count (predicted `_unknown` allowed).
    pub confusion: HashMap<String, HashMap<String, usize>>,
    pub notes: Vec<String>,
}

fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

/// Gold entity id for an annotation: explicit `entity_id`, else the UI term
/// itself is not an entity (returns None → sample skipped for entity scoring).
fn gold_entity(a: &MlAnnotation) -> Option<String> {
    a.entity_id.clone()
}

/// OCR-only verdict mapped to an entity-ish prediction:
/// Some(canonical) on fruit/fish detection, None on unknown.
fn ocr_only_predict(kb: &KnowledgeBase, text: &str, lex: &fruit::Lexicon) -> Option<String> {
    if let Some(d) = fruit::detect_drop(lex, text) {
        if let Some(name) = d.name {
            if let Some(e) = kb.find_by_name(&name) {
                return Some(e.id.clone());
            }
            return Some(format!("raw:{name}"));
        }
        return Some("raw:fruit-unknown".to_string());
    }
    if fruit::is_known_fish_or_item(text) {
        // Best-effort: first known fish word as the prediction.
        let norm = fruit::normalize(text);
        for w in norm.split_whitespace() {
            if let Some(e) = kb.find_by_name(w) {
                return Some(e.id.clone());
            }
        }
        return Some("raw:fish-unknown".to_string());
    }
    None
}

/// Evaluate labeled entity-recognition samples through each stage.
/// Only samples with an `entity_id` gold label and OCR text score.
pub fn evaluate_entity_samples(
    kb: &KnowledgeBase,
    lex: &fruit::Lexicon,
    samples: &[MlAnnotation],
) -> BaselineReport {
    let scored: Vec<&MlAnnotation> = samples
        .iter()
        .filter(|a| {
            a.task == crate::core::ml_dataset::MlTask::EntityRecognition
                && a.entity_id.is_some()
                && !a.ocr_text.trim().is_empty()
        })
        .collect();

    let mut report = BaselineReport {
        dataset: "gpo-vision".to_string(),
        labeled_samples: scored.len(),
        insufficient_data: scored.len() < MIN_LABELED,
        stages: HashMap::new(),
        confusion: HashMap::new(),
        notes: Vec::new(),
    };

    if report.insufficient_data {
        report.notes.push(format!(
            "INSUFFICIENT LABELED DATA: {} labeled entity samples, need {} for metrics.",
            scored.len(),
            MIN_LABELED
        ));
        return report;
    }

    // Stage predictions: Vec<(gold, predicted Option)>.
    let mut ocr_only: Vec<(String, Option<String>)> = Vec::new();
    let mut ocr_knowledge: Vec<(String, Option<String>)> = Vec::new();
    let mut combined: Vec<(String, Option<String>)> = Vec::new();
    let mut lat_ocr = 0u128;
    let mut lat_kb = 0u128;

    for a in &scored {
        let gold = gold_entity(a).unwrap();
        let text = &a.ocr_text;

        let t0 = Instant::now();
        let p1 = ocr_only_predict(kb, text, lex);
        lat_ocr += t0.elapsed().as_micros();

        let t1 = Instant::now();
        let obs = correlate_text(kb, text, &a.region_name, ScreenKind::Fishing, lex.fuzzy_threshold, 0.80, None);
        lat_kb += t1.elapsed().as_micros();
        let p2 = obs.entity.map(|m| {
            kb.find_by_name(&m.canonical_name)
                .map(|e| e.id.clone())
                .unwrap_or(m.entity_id)
        });

        // Combined: knowledge verdict (vision fusion lands here once ML
        // observations exist; today identical to ocr_knowledge by construction).
        let p3 = p2.clone();

        ocr_only.push((gold.clone(), p1));
        ocr_knowledge.push((gold.clone(), p2));
        combined.push((gold, p3));
    }

    let n = scored.len();
    report.stages.insert("ocr_exact".into(), metrics_of(&ocr_only, lat_ocr, n));
    report.stages.insert("ocr_knowledge".into(), metrics_of(&ocr_knowledge, lat_kb, n));
    report.stages.insert("combined".into(), metrics_of(&combined, lat_kb, n));

    for (gold, pred) in &combined {
        *report
            .confusion
            .entry(gold.clone())
            .or_default()
            .entry(pred.clone().unwrap_or_else(|| "_unknown".to_string()))
            .or_insert(0) += 1;
    }
    report.notes.push(
        "Vision stages not scored: labeled bar-frame samples required (none collected yet).".to_string(),
    );
    report
}

fn metrics_of(pairs: &[(String, Option<String>)], lat_us: u128, n: usize) -> StageMetrics {
    if n == 0 {
        return StageMetrics::default();
    }
    // Micro-averaged over gold classes: correct / (correct + wrong + unknown).
    let mut correct = 0usize;
    let mut wrong = 0usize;
    let mut unknown = 0usize;
    for (gold, pred) in pairs {
        match pred {
            Some(p) if p == gold => correct += 1,
            Some(_) => wrong += 1,
            None => unknown += 1,
        }
    }
    let total = (correct + wrong + unknown).max(1) as f32;
    let accuracy = correct as f32 / total;
    // Precision/recall with Unknown treated as abstention: precision over
    // attempted, recall over all.
    let attempted = (correct + wrong).max(1) as f32;
    let precision = correct as f32 / attempted;
    let recall = correct as f32 / total;
    let f1 = if precision + recall > 0.0 {
        2.0 * precision * recall / (precision + recall)
    } else {
        0.0
    };
    StageMetrics {
        n,
        accuracy: round3(accuracy),
        precision: round3(precision),
        recall: round3(recall),
        f1: round3(f1),
        unknown_rate: round3(unknown as f32 / total),
        mean_latency_ms: round3(lat_us as f32 / 1000.0 / n as f32),
    }
}

/// Convenience: evaluate the on-disk training dataset. Only entity-
/// recognition samples with gold `entity_id`s score today (state
/// classification has no predictor yet — those rows are ignored, and the
/// report says so via the vision-stages note when nothing else scores).
pub fn evaluate_store(
    kb: &KnowledgeBase,
    lex: &fruit::Lexicon,
    store: &MlDatasetStore,
) -> BaselineReport {
    evaluate_entity_samples(kb, lex, &store.annotations())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ml_dataset::MlTask;

    fn kb() -> &'static KnowledgeBase {
        KnowledgeBase::bundled()
    }

    fn lex() -> fruit::Lexicon {
        fruit::Lexicon::default()
    }

    fn ann(text: &str, entity: &str) -> MlAnnotation {
        MlAnnotation {
            image_id: format!("img-{}", text.len()),
            dataset_version: 1,
            session_id: "sess-t".into(),
            task: MlTask::EntityRecognition,
            ocr_text: text.into(),
            region_name: "drop".into(),
            ui_label: None,
            bbox: None,
            game_state: None,
            entity_id: Some(entity.into()),
            annotator: "t".into(),
            timestamp_ms: 1,
            source: "t".into(),
            confidence: None,
            hard_example: false,
            hard_reason: None,
            corrections: vec![],
            event_id: None,
            frame_index: None,
        }
    }

    #[test]
    fn insufficient_data_is_reported_not_fabricated() {
        let samples = vec![ann("Suna fruit", "fruit:suna")];
        let rep = evaluate_entity_samples(kb(), &lex(), &samples);
        assert!(rep.insufficient_data);
        assert!(rep.stages.is_empty());
        assert!(rep.notes.iter().any(|n| n.contains("INSUFFICIENT")));
    }

    #[test]
    fn perfect_knowledge_scores_high_and_unknowns_count() {
        let mut samples = Vec::new();
        for _ in 0..6 {
            samples.push(ann("Suna fruit drop", "fruit:suna"));
        }
        for _ in 0..4 {
            samples.push(ann("Mera fruit drop", "fruit:mera"));
        }
        // Two garbage samples labeled as an entity the pipeline cannot see:
        // both stages must abstain (unknown), not hallucinate.
        for _ in 0..2 {
            samples.push(ann("xqz wobble fnord", "fruit:suna"));
        }
        let rep = evaluate_entity_samples(kb(), &lex(), &samples);
        assert!(!rep.insufficient_data);
        assert_eq!(rep.labeled_samples, 12);
        let kb_stage = &rep.stages["ocr_knowledge"];
        assert!(kb_stage.accuracy >= 0.8, "acc={}", kb_stage.accuracy);
        assert!(kb_stage.unknown_rate > 0.0, "garbage must abstain");
        assert!(kb_stage.f1 > kb_stage.accuracy * 0.5);
        assert!(rep.stages["ocr_exact"].n == 12);
        assert!(rep.confusion.contains_key("fruit:suna"));
    }

    #[test]
    fn entity_ids_beat_display_names() {
        // Gold labels are stable ids; predictions resolve through them.
        let samples = vec![ann("Phoenix fruit", "fruit:tori"); 10];
        let rep = evaluate_entity_samples(kb(), &lex(), &samples);
        assert!(!rep.insufficient_data);
        assert_eq!(rep.stages["ocr_knowledge"].accuracy, 1.0);
    }

    #[test]
    fn empty_evaluation_is_insufficient() {
        let rep = evaluate_entity_samples(kb(), &lex(), &[]);
        assert!(rep.insufficient_data);
    }
}
