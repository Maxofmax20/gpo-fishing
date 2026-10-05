//! Deterministic model-readiness engine (v5.6, §16-22).
//!
//! Lifecycle, strictly ordered, each with explicit checks:
//!   TRAINING_READY -> CANDIDATE_READY -> SHADOW_READY -> PRODUCTION_READY
//! plus NOT_READY. A later stage never implies without its own evidence.
//!
//! Thresholds live in `TrainingSettings.readiness_*` (documented defaults,
//! user-visible in the Training Center). PRODUCTION_READY is structurally
//! unreachable: no production-authorization mechanism exists in this
//! codebase, so the gate reports BLOCKED with that reason rather than a
//! fake pass. Production control stays OFF regardless of model quality.

use serde::{Deserialize, Serialize};

use super::registry::{CandidateRecord, Lifecycle, SoakStats};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReadinessStatus {
    TrainingReady,
    CandidateReady,
    ShadowReady,
    ProductionReady,
    NotReady,
}

impl ReadinessStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ReadinessStatus::TrainingReady => "TRAINING_READY",
            ReadinessStatus::CandidateReady => "CANDIDATE_READY",
            ReadinessStatus::ShadowReady => "SHADOW_READY",
            ReadinessStatus::ProductionReady => "PRODUCTION_READY",
            ReadinessStatus::NotReady => "NOT_READY",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadinessCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelReadiness {
    pub family: String,
    pub status: ReadinessStatus,
    pub checks: Vec<ReadinessCheck>,
    pub blockers: Vec<String>,
    pub evidence: serde_json::Value,
}

/// Threshold bundle. Defaults are documented in config.rs alongside the
/// settings; every gate below names the threshold it applied.
#[derive(Debug, Clone)]
pub struct ReadinessThresholds {
    pub min_macro_f1: f32,
    pub min_worst_class_f1: f32,
    pub min_shadow_events: usize,
    pub min_review_coverage: f32,
}

impl Default for ReadinessThresholds {
    fn default() -> Self {
        Self {
            // Promotion already requires beating the current model; readiness
            // additionally requires absolute quality so a weak incumbent
            // cannot bless a weak successor.
            min_macro_f1: 0.70,
            min_worst_class_f1: 0.50,
            // Soak: enough live shadow events to measure agreement ±10%.
            min_shadow_events: 100,
            // Reviewed share of the family's scoped rows.
            min_review_coverage: 0.50,
        }
    }
}

/// Assess one model family. Pure function of real evidence; all inputs are
/// measured artifacts (gates, reviews, registry, soak), never guesses.
#[allow(clippy::too_many_arguments)]
pub fn assess_family(
    family: &str,
    data_gate_ready: bool,
    data_gate_detail: &str,
    reviewed_scoped: usize,
    scoped_total: usize,
    records: &[CandidateRecord],
    deployed_version: Option<u32>,
    deployed_metrics: Option<(f32, f32)>,
    soak: Option<&SoakStats>,
    thresholds: &ReadinessThresholds,
) -> ModelReadiness {
    let mut checks = Vec::new();
    let mut blockers = Vec::new();
    let mut check = |name: &str, passed: bool, detail: String| {
        if !passed {
            blockers.push(format!("{name}: {detail}"));
        }
        checks.push(ReadinessCheck { name: name.to_string(), passed, detail });
    };

    // 1. Training data gate (capability evidence).
    check("training_data", data_gate_ready, data_gate_detail.to_string());

    // 2. Human review coverage over the family's scoped rows.
    let coverage = if scoped_total > 0 { reviewed_scoped as f32 / scoped_total as f32 } else { 0.0 };
    check(
        "human_review",
        coverage >= thresholds.min_review_coverage && scoped_total > 0,
        format!(
            "{reviewed_scoped}/{scoped_total} scoped rows reviewed ({:.0}% >= {:.0}%)",
            coverage * 100.0,
            thresholds.min_review_coverage * 100.0
        ),
    );

    // 3. Candidate evaluated (registry).
    let evaluated: Vec<&CandidateRecord> =
        records.iter().filter(|r| r.model_family == family).collect();
    let latest_eval = evaluated
        .iter()
        .filter(|r| matches!(r.status, Lifecycle::Evaluated | Lifecycle::Shadow | Lifecycle::ShadowValidated | Lifecycle::Accepted))
        .max_by_key(|r| r.model_version);
    check(
        "candidate_evaluated",
        latest_eval.is_some(),
        latest_eval
            .map(|r| format!("{} macro-F1 {:.3} on {} test sessions", r.candidate_id, r.metrics.macro_f1, r.metrics.test_sessions))
            .unwrap_or_else(|| "no evaluated candidate in registry".to_string()),
    );

    // 4. Absolute quality bars on the latest evaluated candidate.
    let mut quality_ok = false;
    if let Some(r) = latest_eval {
        let macro_ok = r.metrics.macro_f1 >= thresholds.min_macro_f1;
        let worst = r.metrics.per_class_f1.values().cloned().fold(f32::INFINITY, f32::min);
        let worst_ok = worst >= thresholds.min_worst_class_f1;
        quality_ok = macro_ok && worst_ok;
        check("macro_f1", macro_ok, format!("{:.3} >= {:.2}", r.metrics.macro_f1, thresholds.min_macro_f1));
        check(
            "worst_class_f1",
            worst_ok,
            if worst.is_finite() {
                format!("{worst:.3} >= {:.2}", thresholds.min_worst_class_f1)
            } else {
                "no per-class metrics recorded".to_string()
            },
        );
    } else {
        check("macro_f1", false, "no candidate to measure".to_string());
        check("worst_class_f1", false, "no candidate to measure".to_string());
    }

    // 5. Shadow deployment + soak.
    let shadow_deployed = deployed_version.is_some();
    check(
        "shadow_deployed",
        shadow_deployed,
        deployed_version.map(|v| format!("shadow running v{v}")).unwrap_or_else(|| "nothing deployed to shadow".to_string()),
    );
    let soak_ok = soak.map(|s| s.events >= thresholds.min_shadow_events).unwrap_or(false);
    check(
        "shadow_soak",
        soak_ok,
        soak.map(|s| {
            format!(
                "{} events (need {}), agreement {}",
                s.events,
                thresholds.min_shadow_events,
                s.agreement_rate.map(|a| format!("{:.0}%", a * 100.0)).unwrap_or_else(|| "unmeasured".to_string())
            )
        })
        .unwrap_or_else(|| "no shadow telemetry yet".to_string()),
    );

    // 6. Production: structurally blocked (no authorization mechanism).
    check(
        "production_authorized",
        false,
        "no production-authorization mechanism exists in this codebase; production control is OFF by design".to_string(),
    );

    let stage = |ok: bool| ok;
    let training = stage(checks[0].passed && checks[1].passed);
    let candidate =
        training && latest_eval.is_some() && quality_ok;
    let shadow = candidate && shadow_deployed && soak_ok;
    let status = if shadow {
        // Even a fully soaked shadow model is NOT production: see check 6.
        ReadinessStatus::ShadowReady
    } else if candidate {
        ReadinessStatus::CandidateReady
    } else if training {
        ReadinessStatus::TrainingReady
    } else {
        ReadinessStatus::NotReady
    };

    ModelReadiness {
        family: family.to_string(),
        status,
        checks,
        blockers,
        evidence: serde_json::json!({
            "deployed_version": deployed_version,
            "deployed_metrics": deployed_metrics,
            "reviewed_scoped": reviewed_scoped,
            "scoped_total": scoped_total,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn rec(family: &str, version: u32, f1: f32, worst: f32) -> CandidateRecord {
        let mut per = HashMap::new();
        per.insert("a".to_string(), f1);
        per.insert("b".to_string(), worst);
        CandidateRecord {
            candidate_id: format!("{family}-v{version}-t"),
            model_family: family.to_string(),
            model_version: version,
            parent_name: format!("{family}_v1"),
            parent_sha: None,
            dataset_fingerprint: "fp".into(),
            dataset_version: 1,
            training_config_hash: "c".into(),
            code_version: "t".into(),
            job_id: "j".into(),
            created_at: 1,
            artifact_sha: "a".into(),
            manifest_sha: None,
            eval_sha: "e".into(),
            metrics: super::super::registry::ModelMetrics {
                accuracy: 0.8,
                macro_f1: f1,
                ece: None,
                test_sessions: 6,
                test_n: 100,
                per_class_f1: per,
                kept_accuracy: None,
            },
            classes: vec!["a".into(), "b".into()],
            temperature: 1.0,
            mean: [0.0; 3],
            std: [1.0; 3],
            status: Lifecycle::Evaluated,
            decision_reason: None,
        }
    }

    fn soak(events: usize) -> SoakStats {
        SoakStats {
            model: "fish_v1".into(),
            events,
            agree: events * 8 / 10,
            disagree: events * 2 / 10,
            no_ocr_baseline: 0,
            mean_confidence: Some(0.8),
            agreement_rate: Some(0.8),
        }
    }

    #[test]
    fn empty_evidence_is_not_ready_with_named_blockers() {
        let r = assess_family("fish", false, "0/10 qualified", 0, 100, &[], None, None, None, &ReadinessThresholds::default());
        assert_eq!(r.status, ReadinessStatus::NotReady);
        assert!(r.blockers.iter().any(|b| b.contains("training_data")));
        assert!(r.blockers.iter().any(|b| b.contains("production_authorized")));
    }

    #[test]
    fn good_data_without_candidate_is_training_ready_only() {
        let r = assess_family("fish", true, "8/10 qualified", 60, 100, &[], None, None, None, &ReadinessThresholds::default());
        assert_eq!(r.status, ReadinessStatus::TrainingReady);
    }

    #[test]
    fn strong_candidate_without_soak_is_candidate_ready() {
        let records = vec![rec("fish", 2, 0.80, 0.65)];
        let r = assess_family("fish", true, "ok", 60, 100, &records, None, None, None, &ReadinessThresholds::default());
        assert_eq!(r.status, ReadinessStatus::CandidateReady);
    }

    #[test]
    fn weak_macro_f1_blocks_candidate_stage() {
        let records = vec![rec("fish", 2, 0.60, 0.55)];
        let r = assess_family("fish", true, "ok", 60, 100, &records, None, None, None, &ReadinessThresholds::default());
        assert_eq!(r.status, ReadinessStatus::TrainingReady, "macro 0.60 < 0.70 must not advance");
        assert!(r.blockers.iter().any(|b| b.contains("macro_f1")));
    }

    #[test]
    fn weak_worst_class_blocks_candidate_stage() {
        let records = vec![rec("fish", 2, 0.80, 0.40)];
        let r = assess_family("fish", true, "ok", 60, 100, &records, None, None, None, &ReadinessThresholds::default());
        assert_eq!(r.status, ReadinessStatus::TrainingReady);
    }

    #[test]
    fn soaked_shadow_is_shadow_ready_never_production() {
        let records = vec![rec("fish", 2, 0.80, 0.65)];
        let s = soak(150);
        let r = assess_family("fish", true, "ok", 60, 100, &records, Some(2), Some((0.8, 0.8)), Some(&s), &ReadinessThresholds::default());
        assert_eq!(r.status, ReadinessStatus::ShadowReady);
        assert!(r.blockers.iter().any(|b| b.contains("production_authorized")));
    }

    #[test]
    fn thin_soak_keeps_candidate_stage() {
        let records = vec![rec("fish", 2, 0.80, 0.65)];
        let s = soak(20);
        let r = assess_family("fish", true, "ok", 60, 100, &records, Some(2), None, Some(&s), &ReadinessThresholds::default());
        assert_eq!(r.status, ReadinessStatus::CandidateReady);
    }
}
