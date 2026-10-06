//! Deterministic model-readiness engine (v5.6, §16-22; hardened v5.6.1).
//!
//! ## What each status MEANS (they are not interchangeable)
//!
//! Statuses, in lifecycle order. They are deliberately distinct:
//!
//! - `NOT_ENOUGH_DATA`: zero qualified classes; nothing to learn from.
//! - `NOT_ENOUGH_CLASSES`: some classes qualify, fewer than the gate wants.
//! - `NOT_ENOUGH_SESSIONS`: classes qualify but one falls short on the
//!   independent-session requirement.
//! - `NOT_ENOUGH_REVIEW`: data qualifies but human review coverage is below
//!   the floor, so training would learn from unverified labels.
//! - `NOT_ENOUGH_TEST`: qualified classes lack held-out TEST coverage.
//! - `DATA_READY`: data and review both pass; training may start.
//! - `TRAINING` / `EVALUATING`: a real job is running (live job phase).
//! - `CANDIDATE_READY`: an evaluated candidate clears the absolute quality
//!   bars.
//! - `SHADOW_READY`: that candidate is deployed to shadow and its soak passes
//!   volume, agreement and session spread.
//! - `PRODUCTION_READY`: structurally unreachable (see below).
//! - `NOT_READY`: no classified reason; never used as a catch-all.
//!
//! Training completion NEVER implies readiness: each transition requires its
//! own evidence, checked in order.
//!
//! ## Production
//!
//! There is no production-authorization mechanism anywhere in this codebase
//! (no inference runtime is linked on the production path - `ml_model::detect`
//! returns empty). `PRODUCTION_READY` is therefore reported as a
//! **structural** failing check: it is displayed for honesty, but it is NOT
//! counted as an actionable blocker, because a human cannot clear it by
//! collecting data. Production control stays OFF regardless of model quality.
//!
//! ## Thresholds
//!
//! All bars live in `config::TrainingSettings.readiness_*` (documented
//! defaults, user-visible in the Training Center) and are threaded in through
//! [`ReadinessThresholds`]. No bar is hardcoded here.

use serde::{Deserialize, Serialize};

use super::registry::{CandidateRecord, Lifecycle, SoakStats};
use super::training::{JobStatus, TrainingJob};

/// Lifecycle stage a check belongs to. Used by the UI to group checks so a
/// reviewer can tell "is my review work the blocker?" from "is the backend
/// missing?".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReadinessStage {
    Data,
    Review,
    Train,
    Evaluate,
    Shadow,
    Production,
}

impl ReadinessStage {
    pub fn as_str(self) -> &'static str {
        match self {
            ReadinessStage::Data => "DATA",
            ReadinessStage::Review => "REVIEW",
            ReadinessStage::Train => "TRAIN",
            ReadinessStage::Evaluate => "EVALUATE",
            ReadinessStage::Shadow => "SHADOW",
            ReadinessStage::Production => "PRODUCTION",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReadinessStatus {
    NotEnoughData,
    NotEnoughClasses,
    NotEnoughSessions,
    NotEnoughReview,
    NotEnoughTest,
    DataReady,
    Training,
    Evaluating,
    CandidateReady,
    ShadowReady,
    ProductionReady,
    NotReady,
}

impl ReadinessStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ReadinessStatus::NotEnoughData => "NOT_ENOUGH_DATA",
            ReadinessStatus::NotEnoughClasses => "NOT_ENOUGH_CLASSES",
            ReadinessStatus::NotEnoughSessions => "NOT_ENOUGH_SESSIONS",
            ReadinessStatus::NotEnoughReview => "NOT_ENOUGH_REVIEW",
            ReadinessStatus::NotEnoughTest => "NOT_ENOUGH_TEST",
            ReadinessStatus::DataReady => "DATA_READY",
            ReadinessStatus::Training => "TRAINING",
            ReadinessStatus::Evaluating => "EVALUATING",
            ReadinessStatus::CandidateReady => "CANDIDATE_READY",
            ReadinessStatus::ShadowReady => "SHADOW_READY",
            ReadinessStatus::ProductionReady => "PRODUCTION_READY",
            ReadinessStatus::NotReady => "NOT_READY",
        }
    }
}

/// Live phase of the training pipeline for one family, derived from real
/// job records by the caller. Drives the TRANSIENT statuses so the UI can
/// distinguish "training is running" from "training finished and failed".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JobPhase {
    #[default]
    Idle,
    Training,
    Evaluating,
}

/// Structured view of the data gate, so the readiness engine can say WHICH
/// kind of shortfall it is instead of collapsing everything into one blob.
#[derive(Debug, Clone, Default)]
pub struct DataGateInfo {
    pub ready: bool,
    pub qualified: usize,
    pub required: usize,
    pub test_covered: usize,
    pub test_required: usize,
    /// Sessions required for the currently binding entity, when the gate can
    /// attribute the shortfall to session diversity specifically.
    pub sessions_short: Option<(String, usize, usize)>,
    pub detail: String,
    pub next_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadinessCheck {
    pub name: String,
    pub stage: ReadinessStage,
    pub passed: bool,
    /// One-line human sentence (kept for the compact UI).
    pub detail: String,
    /// Measured value ("8/10", "0.54", "137 rows").
    pub actual: String,
    /// Required value ("10/10", ">= 0.70").
    pub required: String,
    /// Signed shortfall ("2 classes short", "0.16 below floor", "none").
    pub difference: String,
    /// What a human should actually do next. Empty when the check passes.
    pub next_action: String,
    /// Cannot pass in this build by design; shown but not listed as an
    /// actionable blocker.
    pub structural: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelReadiness {
    pub family: String,
    pub status: ReadinessStatus,
    pub checks: Vec<ReadinessCheck>,
    /// Actionable failing checks only (structural ones excluded).
    pub blockers: Vec<String>,
    /// Deduplicated, ordered list of concrete next steps.
    pub next_actions: Vec<String>,
    pub evidence: serde_json::Value,
}

/// Threshold bundle. Defaults are documented in `config.rs` next to the
/// settings; every gate below names the threshold it applied.
#[derive(Debug, Clone)]
pub struct ReadinessThresholds {
    pub min_macro_f1: f32,
    pub min_worst_class_f1: f32,
    pub min_shadow_events: usize,
    /// Minimum measured OCR/vision agreement across the soak. Counted
    /// events alone prove nothing about behaviour.
    pub min_shadow_agreement: f32,
    /// Distinct sessions the soak must span. Evidence concentrated in one
    /// session is not evidence about the model.
    pub min_shadow_sessions: usize,
    /// Reviewed share of the family's scoped rows.
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
            // Soak: enough live shadow events to measure agreement +/-10%.
            min_shadow_events: 100,
            // ...and the events must actually agree, not merely exist.
            min_shadow_agreement: 0.80,
            // ...and must come from more than one play session.
            min_shadow_sessions: 3,
            // Reviewed share of the family's scoped rows.
            min_review_coverage: 0.50,
        }
    }
}

struct Ctx {
    checks: Vec<ReadinessCheck>,
    blockers: Vec<String>,
    next_actions: Vec<String>,
}

impl Ctx {
    fn new() -> Self {
        Self { checks: Vec::new(), blockers: Vec::new(), next_actions: Vec::new() }
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        name: &str,
        stage: ReadinessStage,
        passed: bool,
        detail: String,
        actual: String,
        required: String,
        difference: String,
        next_action: impl Into<String>,
        structural: bool,
    ) {
        let next_action = next_action.into();
        if !passed && !structural {
            self.blockers.push(format!("{name}: {detail}"));
            if !next_action.is_empty() && !self.next_actions.contains(&next_action) {
                self.next_actions.push(next_action.clone());
            }
        }
        self.checks.push(ReadinessCheck {
            name: name.to_string(),
            stage,
            passed,
            detail,
            actual,
            required,
            difference,
            next_action,
            structural,
        });
    }

    fn named(&self, name: &str) -> Option<&ReadinessCheck> {
        self.checks.iter().find(|c| c.name == name)
    }
}

/// Assess one model family. Pure function of real evidence; all inputs are
/// measured artifacts (data gate, review counts, registry, soak, job phase),
/// never guesses.
#[allow(clippy::too_many_arguments)]
pub fn assess_family(
    family: &str,
    data: &DataGateInfo,
    reviewed_scoped: usize,
    scoped_total: usize,
    records: &[CandidateRecord],
    deployed_version: Option<u32>,
    soak: Option<&SoakStats>,
    phase: JobPhase,
    jobs: &[TrainingJob],
    thresholds: &ReadinessThresholds,
) -> ModelReadiness {
    let mut c = Ctx::new();

    // ---------- STAGE: DATA ----------
    c.add(
        "qualified_classes",
        ReadinessStage::Data,
        data.ready,
        data.detail.clone(),
        format!("{}/{} qualified", data.qualified, data.required),
        format!("{}/{}", data.required, data.required),
        format!("{} class(es) short", data.required.saturating_sub(data.qualified)),
        data.next_action.clone(),
        false,
    );
    c.add(
        "test_coverage",
        ReadinessStage::Data,
        data.test_covered >= data.test_required,
        format!("held-out TEST coverage {}/{}", data.test_covered, data.test_required),
        format!("{}/{}", data.test_covered, data.test_required),
        format!("{}/{}", data.test_required, data.test_required),
        format!("{} class(es) without TEST coverage", data.test_required.saturating_sub(data.test_covered)),
        "Add TEST-split sessions for the uncovered classes (see Dataset > split view).".to_string(),
        false,
    );
    if let Some((entity, have, want)) = data.sessions_short.clone() {
        c.add(
            "session_diversity",
            ReadinessStage::Data,
            false,
            format!("{entity} appears in {have} session(s)"),
            format!("{have} sessions"),
            format!("{want} sessions"),
            format!("{} session(s) short for {entity}", want.saturating_sub(have)),
            format!("Fish/catch {entity} in {want} independent play sessions."),
            false,
        );
    }

    // ---------- STAGE: REVIEW ----------
    let coverage = if scoped_total > 0 { reviewed_scoped as f32 / scoped_total as f32 } else { 0.0 };
    let required_rows = (scoped_total as f32 * thresholds.min_review_coverage).ceil() as usize;
    c.add(
        "human_review",
        ReadinessStage::Review,
        scoped_total > 0 && coverage >= thresholds.min_review_coverage,
        format!(
            "{reviewed_scoped}/{scoped_total} scoped rows reviewed ({:.0}% >= {:.0}%)",
            coverage * 100.0,
            thresholds.min_review_coverage * 100.0
        ),
        format!("{reviewed_scoped}/{scoped_total} rows"),
        format!("{required_rows} rows ({:.0}%)", thresholds.min_review_coverage * 100.0),
        format!("{} row(s) short", required_rows.saturating_sub(reviewed_scoped)),
        "Open Training > Review and clear the priority queue for this family.".to_string(),
        false,
    );

    // ---------- STAGE: EVALUATE ----------
    let evaluated: Vec<&CandidateRecord> =
        records.iter().filter(|r| r.model_family == family).collect();
    let latest_eval = evaluated
        .iter()
        .filter(|r| {
            matches!(
                r.status,
                Lifecycle::Evaluated | Lifecycle::Shadow | Lifecycle::ShadowValidated | Lifecycle::Accepted
            )
        })
        .max_by_key(|r| r.model_version);
    c.add(
        "candidate_evaluated",
        ReadinessStage::Evaluate,
        latest_eval.is_some(),
        latest_eval
            .map(|r| format!("{} macro-F1 {:.3} on {} test sessions", r.candidate_id, r.metrics.macro_f1, r.metrics.test_sessions))
            .unwrap_or_else(|| "no evaluated candidate in registry".to_string()),
        latest_eval.map(|r| format!("macro-F1 {:.3}", r.metrics.macro_f1)).unwrap_or_else(|| "none".to_string()),
        "1 evaluated candidate".to_string(),
        latest_eval.map(|_| "none".to_string()).unwrap_or_else(|| "no candidate".to_string()),
        if latest_eval.is_some() { String::new() } else { "Start a training job once the data and review stages pass.".to_string() },
        false,
    );

    // Assigned in both match arms below; the initialiser is deliberately absent.
    let quality_ok: bool;
    let mut worst_detail = "no candidate to measure".to_string();
    let mut worst_actual = "none".to_string();
    let mut worst_diff = "n/a".to_string();
    if let Some(r) = latest_eval {
        let macro_ok = r.metrics.macro_f1 >= thresholds.min_macro_f1;
        c.add(
            "macro_f1",
            ReadinessStage::Evaluate,
            macro_ok,
            format!("macro-F1 {:.3} (need >= {:.2})", r.metrics.macro_f1, thresholds.min_macro_f1),
            format!("{:.3}", r.metrics.macro_f1),
            format!(">= {:.2}", thresholds.min_macro_f1),
            format!("{:.3} below floor", (thresholds.min_macro_f1 - r.metrics.macro_f1).max(0.0)),
            if macro_ok { String::new() } else { "Collect more qualified classes / review more samples for this family and retrain.".to_string() },
            false,
        );
        // Name the offending class - a bare number is not actionable.
        let worst = r
            .metrics
            .per_class_f1
            .iter()
            .min_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal));
        match worst {
            Some((cls, score)) => {
                let ok = *score >= thresholds.min_worst_class_f1;
                quality_ok = macro_ok && ok;
                worst_detail =
                    format!("worst class {cls} F1 {:.3} (need >= {:.2})", score, thresholds.min_worst_class_f1);
                worst_actual = format!("{cls} = {:.3}", score);
                worst_diff = format!("{:.3} below floor", (thresholds.min_worst_class_f1 - *score).max(0.0));
                c.add(
                    "worst_class_f1",
                    ReadinessStage::Evaluate,
                    ok,
                    worst_detail.clone(),
                    worst_actual.clone(),
                    format!(">= {:.2}", thresholds.min_worst_class_f1),
                    worst_diff.clone(),
                    if ok { String::new() } else { format!("Collect and review more real examples of {cls}; it is the binding class.") },
                    false,
                );
            }
            None => {
                quality_ok = false;
                c.add(
                    "worst_class_f1",
                    ReadinessStage::Evaluate,
                    false,
                    "no per-class metrics recorded".to_string(),
                    "none".to_string(),
                    format!(">= {:.2}", thresholds.min_worst_class_f1),
                    "unknown".to_string(),
                    "Re-evaluate the candidate so per-class F1 is recorded.".to_string(),
                    false,
                );
            }
        }
    } else {
        quality_ok = false;
        c.add(
            "macro_f1",
            ReadinessStage::Evaluate,
            false,
            "no candidate to measure".to_string(),
            "none".to_string(),
            format!(">= {:.2}", thresholds.min_macro_f1),
            "n/a".to_string(),
            "Train a candidate first (Training > Jobs > Train).".to_string(),
            false,
        );
        c.add(
            "worst_class_f1",
            ReadinessStage::Evaluate,
            false,
            worst_detail,
            worst_actual,
            format!(">= {:.2}", thresholds.min_worst_class_f1),
            worst_diff,
            "Train a candidate first.".to_string(),
            false,
        );
    }

    // ---------- STAGE: SHADOW ----------
    let deployed = deployed_version;
    c.add(
        "shadow_deployed",
        ReadinessStage::Shadow,
        deployed.is_some(),
        deployed.map(|v| format!("shadow running revision v{v}")).unwrap_or_else(|| "nothing deployed to shadow".to_string()),
        deployed.map(|v| format!("v{v}")).unwrap_or_else(|| "none".to_string()),
        "1 deployed revision".to_string(),
        if deployed.is_some() { "none".to_string() } else { "1 missing".to_string() },
        if deployed.is_some() { String::new() } else { "Promote a passing candidate to shadow (Training > Candidates).".to_string() },
        false,
    );

    // Soak is three independent requirements: volume, agreement and spread.
    // Volume alone proves nothing about behaviour.
    let soak_events_ok = soak.map(|s| s.events >= thresholds.min_shadow_events).unwrap_or(false);
    c.add(
        "shadow_soak_events",
        ReadinessStage::Shadow,
        soak_events_ok,
        soak.map(|s| format!("{} soak events", s.events)).unwrap_or_else(|| "no shadow telemetry yet".to_string()),
        soak.map(|s| format!("{} events", s.events)).unwrap_or_else(|| "0".to_string()),
        format!(">= {} events", thresholds.min_shadow_events),
        soak.map(|s| format!("{} short", thresholds.min_shadow_events.saturating_sub(s.events)))
            .unwrap_or_else(|| format!("{} short", thresholds.min_shadow_events)),
        "Play with shadow observation enabled to accumulate soak events.".to_string(),
        false,
    );
    let agree = soak.and_then(|s| s.agreement_rate);
    let agree_ok = agree.map(|a| a >= thresholds.min_shadow_agreement).unwrap_or(false);
    c.add(
        "shadow_soak_agreement",
        ReadinessStage::Shadow,
        agree_ok,
        match agree {
            Some(a) => format!("OCR/vision agreement {:.0}% (need >= {:.0}%) over {} scored events", a * 100.0, thresholds.min_shadow_agreement * 100.0, soak.map(|s| s.agree + s.disagree).unwrap_or(0)),
            None => "no scored shadow events (no independent OCR baseline to compare against)".to_string(),
        },
        agree.map(|a| format!("{:.0}%", a * 100.0)).unwrap_or_else(|| "unmeasured".to_string()),
        format!(">= {:.0}%", thresholds.min_shadow_agreement * 100.0),
        match agree {
            Some(a) if a < thresholds.min_shadow_agreement => format!("{:.0}pp below floor", (thresholds.min_shadow_agreement - a) * 100.0),
            Some(_) => "none".to_string(),
            None => "unmeasured".to_string(),
        },
        if agree_ok { String::new() } else { "Investigate disagreements; a model that disagrees with OCR+KB is not ready to use.".to_string() },
        false,
    );
    let sessions = soak.map(|s| s.sessions).unwrap_or(0);
    let sess_ok = sessions >= thresholds.min_shadow_sessions;
    c.add(
        "shadow_soak_sessions",
        ReadinessStage::Shadow,
        sess_ok,
        format!("soak spans {sessions} session(s), {} distinct label(s)", soak.map(|s| s.distinct_entities).unwrap_or(0)),
        format!("{sessions} sessions"),
        format!(">= {} sessions", thresholds.min_shadow_sessions),
        format!("{} session(s) short", thresholds.min_shadow_sessions.saturating_sub(sessions)),
        "Soak across multiple independent play sessions.".to_string(),
        false,
    );

    // EVIDENCE MUST BELONG TO THE SAME MODEL. `latest_eval` is the highest
    // evaluated version in the registry; the soak is whatever the on-disk
    // manifest says. Nothing tied them, so a newly registered v5 candidate
    // would inherit v4's 150-event, 4-session, 92%-agreement soak and be
    // reported SHADOW_READY having been observed zero times.
    //
    // The candidate's metrics and the soak evidence must describe the same
    // revision. When they do not, we report exactly that instead of adding
    // numbers from different models together.
    let evidence_owner = latest_eval.map(|r| r.model_version);
    let same_revision = match (evidence_owner, deployed) {
        (Some(a), Some(b)) => a == b,
        // No evaluated candidate: nothing to mis-attribute.
        (None, _) => true,
        // A manifest exists but no record does: the deployed bytes are not
        // described by any registry record, so no metrics are being claimed.
        (Some(_), None) => false,
    };
    c.add(
        "shadow_evidence_revision",
        ReadinessStage::Shadow,
        same_revision,
        match (evidence_owner, deployed) {
            (Some(a), Some(b)) if a == b => format!("metrics and soak both describe revision v{a}"),
            (Some(a), Some(b)) => format!(
                "evaluated candidate is v{a} but shadow is running v{b}; \
                 v{a}'s metrics and v{b}'s soak are not the same evidence"
            ),
            (Some(a), None) => format!("evaluated candidate v{a} is not deployed to shadow"),
            (None, _) => "no evaluated candidate".to_string(),
        },
        match (evidence_owner, deployed) {
            (Some(a), Some(b)) if a == b => format!("v{a}"),
            (Some(a), Some(b)) => format!("v{a} vs v{b}"),
            _ => "none".to_string(),
        },
        "metrics and soak describe the same revision".to_string(),
        if same_revision { "none".to_string() } else { "1 mismatch".to_string() },
        if same_revision {
            String::new()
        } else {
            "Promote the evaluated candidate to shadow so its soak is its own.".to_string()
        },
        false,
    );

    // ---------- STAGE: PRODUCTION (structurally blocked) ----------
    c.add(
        "production_authorized",
        ReadinessStage::Production,
        false,
        "no production-authorization mechanism exists in this codebase; production control is OFF by design".to_string(),
        "OFF".to_string(),
        "authorized".to_string(),
        "by design".to_string(),
        String::new(),
        true,
    );

    // ---------- STATUS ----------
    // ---------- STAGE: TRAIN ----------
    // A real job exists for this family and produced a usable snapshot.
    // The snapshot is the human-review boundary, so this check reports the
    // frozen row counts as evidence rather than a second opinion on them.
    let job = jobs.iter().filter(|j| j.model_family == family).max_by_key(|j| j.created_at);
    let job_passed = job.is_some_and(|j| j.status == JobStatus::Passed);
    c.add(
        "training_job",
        ReadinessStage::Train,
        job_passed,
        job.map(|j| format!("{} ({})", j.status.as_str(), j.progress.stage))
            .unwrap_or_else(|| "no training job has ever run for this family".to_string()),
        job.map(|j| format!("{} rows / {} sessions frozen", j.frozen_rows, j.frozen_sessions))
            .unwrap_or_else(|| "none".to_string()),
        "1 job that produced an evaluation".to_string(),
        job.map(|j| if job_passed { "none".to_string() } else { format!("last job {}", j.status.as_str()) })
            .unwrap_or_else(|| "no job".to_string()),
        if job_passed { String::new() } else { "Start a training job from Training > Jobs once DATA and REVIEW pass.".to_string() },
        false,
    );

    let data_ok = c.named("qualified_classes").map(|x| x.passed).unwrap_or(false);
    let test_ok = c.named("test_coverage").map(|x| x.passed).unwrap_or(false);
    let sess_ok_short = data.sessions_short.is_none();
    let review_ok = c.named("human_review").map(|x| x.passed).unwrap_or(false);
    let review_eval_ok = c.named("candidate_evaluated").map(|x| x.passed).unwrap_or(false);

    // Ordered exactly like the lifecycle: the FIRST unmet stage is the status.
// A later stage is never reported while an earlier one is red - otherwise a
// failed training job could still surface as SHADOW_READY from an older
// registry entry, which is precisely the "readiness lies" failure mode.
    let train_ok = c.named("training_job").map(|x| x.passed).unwrap_or(false);
    let status = match phase {
        JobPhase::Training => ReadinessStatus::Training,
        JobPhase::Evaluating => ReadinessStatus::Evaluating,
        JobPhase::Idle => {
            if data.qualified == 0 {
                ReadinessStatus::NotEnoughData
            } else if !data_ok {
                ReadinessStatus::NotEnoughClasses
            } else if !sess_ok_short {
                ReadinessStatus::NotEnoughSessions
            } else if !test_ok {
                ReadinessStatus::NotEnoughTest
            } else if !review_ok {
                ReadinessStatus::NotEnoughReview
            } else if !train_ok {
                ReadinessStatus::DataReady
            } else if review_eval_ok && quality_ok {
                if deployed.is_some() && soak_events_ok && agree_ok && sess_ok && same_revision {
                    ReadinessStatus::ShadowReady
                } else {
                    ReadinessStatus::CandidateReady
                }
            } else {
                ReadinessStatus::DataReady
            }
        }
    };
    // `qualify()` keeps the enum honest: no status outside this set exists,
    // and PRODUCTION_READY is deliberately unreachable (check 1 is always a
    // structural failure).
    debug_assert!(status != ReadinessStatus::ProductionReady);

    ModelReadiness {
        family: family.to_string(),
        status,
        checks: c.checks,
        blockers: c.blockers,
        next_actions: c.next_actions,
        evidence: serde_json::json!({
            "deployed_version": deployed_version,
            "reviewed_scoped": reviewed_scoped,
            "scoped_total": scoped_total,
            "review_coverage": coverage,
            "qualified_classes": data.qualified,
            "required_classes": data.required,
            "test_covered": data.test_covered,
            "job_phase": match phase { JobPhase::Idle => "IDLE", JobPhase::Training => "TRAINING", JobPhase::Evaluating => "EVALUATING" },
            "candidate": latest_eval.map(|r| serde_json::json!({
                "candidate_id": r.candidate_id,
                "version": r.model_version,
                "macro_f1": r.metrics.macro_f1,
                "worst_class": r.metrics.per_class_f1.iter()
                    .min_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(k, v)| serde_json::json!({"entity": k, "f1": v})),
                "status": format!("{:?}", r.status),
            })),
            "soak": soak.map(|s| serde_json::json!({
                "events": s.events,
                "sessions": s.sessions,
                "distinct_entities": s.distinct_entities,
                "agreement_rate": s.agreement_rate,
                "mean_confidence": s.mean_confidence,
                "malformed_lines": s.malformed_lines,
                "duplicates_skipped": s.duplicates_skipped,
                "revision": s.revision,
            })),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn rec(family: &str, version: u32, f1: f32, worst: f32) -> CandidateRecord {
        let mut per = HashMap::new();
        per.insert("fish:a".to_string(), f1);
        per.insert("fish:worst".to_string(), worst);
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
                per_class_test_support: Default::default(),
                kept_accuracy: None,
            },
            classes: vec!["fish:a".into(), "fish:worst".into()],
            vocab_sha: String::new(),
            temperature: 1.0,
            mean: [0.0; 3],
            std: [1.0; 3],
            status: Lifecycle::Evaluated,
            decision_reason: None,
        }
    }

    fn soak(events: usize, sessions: usize, agree: f32) -> SoakStats {
        SoakStats {
            model: "fish_v1".into(),
            events,
            agree: (events as f32 * agree) as usize,
            disagree: (events as f32 * (1.0 - agree)) as usize,
            no_ocr_baseline: 0,
            mean_confidence: Some(0.8),
            agreement_rate: Some(agree),
            revision: "2".into(),
            sessions,
            distinct_entities: 4,
            uniques: events,
            malformed_lines: 0,
            duplicates_skipped: 0,
        }
    }

    fn data(qualified: usize, required: usize) -> DataGateInfo {
        DataGateInfo {
            ready: qualified >= required,
            qualified,
            required,
            test_covered: qualified,
            test_required: required,
            sessions_short: None,
            detail: format!("qualified {qualified}/{required}"),
            next_action: "Collect and review real catches.".into(),
        }
    }

    fn full_data() -> DataGateInfo {
        data(10, 10)
    }

    fn job(family: &str, status: JobStatus) -> super::super::training::TrainingJob {
        use std::path::PathBuf;
        super::super::training::TrainingJob {
            job_id: "j1".into(),
            model_family: family.to_string(),
            dataset_version: 1,
            dataset_fingerprint: "fp".into(),
            frozen_rows: 100,
            frozen_sessions: 12,
            frozen_test_sessions: vec!["s-test".into()],
            review_fingerprint: "rv".into(),
            snapshot_dir: PathBuf::new(),
            created_at: 1,
            started_at: Some(1),
            finished_at: Some(2),
            status,
            requested_by: "manual".into(),
            trainer_module: "gpo_train.fish_train".into(),
            run_id: "r1".into(),
            epochs: 40,
            seed: 7,
            code_version: "test".into(),
            trainer_dir: String::new(),
            work_dir: PathBuf::new(),
            candidate_id: None,
            evaluation: None,
            error: None,
            progress: super::super::training::JobProgress {
                stage: "PASSED".to_string(),
                epoch: 40,
                total_epochs: 40,
                val_metric: Some(0.8),
                ..Default::default()
            },
            deferred_reason: None,
            macro_running_at_start: false,
        }
    }

    fn passed_jobs() -> Vec<super::super::training::TrainingJob> {
        vec![job("fish", JobStatus::Passed)]
    }

    fn assess(
        d: &DataGateInfo,
        recs: &[CandidateRecord],
        deployed: Option<u32>,
        s: Option<&SoakStats>,
        phase: JobPhase,
    ) -> ModelReadiness {
        assess_family("fish", d, 80, 100, recs, deployed, s, phase, &passed_jobs(), &ReadinessThresholds::default())
    }

    #[test]
    fn no_qualified_classes_is_not_enough_data() {
        let r = assess(&data(0, 10), &[], None, None, JobPhase::Idle);
        assert_eq!(r.status, ReadinessStatus::NotEnoughData);
        assert!(r.blockers.iter().any(|b| b.contains("qualified_classes")));
        assert!(!r.next_actions.is_empty());
    }

    #[test]
    fn partial_class_coverage_is_not_enough_classes() {
        let r = assess(&data(8, 10), &[], None, None, JobPhase::Idle);
        assert_eq!(r.status, ReadinessStatus::NotEnoughClasses);
        // Difference must be explicit, not just a boolean.
        let chk = r.checks.iter().find(|c| c.name == "qualified_classes").unwrap();
        assert_eq!(chk.actual, "8/10 qualified");
        assert_eq!(chk.difference, "2 class(es) short");
        assert!(!chk.next_action.is_empty());
    }

    #[test]
    fn session_shortfall_has_its_own_status() {
        let mut d = full_data();
        d.sessions_short = Some(("fish:a".into(), 1, 3));
        let r = assess(&d, &[], None, None, JobPhase::Idle);
        assert_eq!(r.status, ReadinessStatus::NotEnoughSessions);
        let chk = r.checks.iter().find(|c| c.name == "session_diversity").unwrap();
        assert_eq!(chk.difference, "2 session(s) short for fish:a");
    }

    #[test]
    fn missing_test_coverage_has_its_own_status() {
        let mut d = full_data();
        d.test_covered = 8;
        d.test_required = 10;
        let r = assess(&d, &[], None, None, JobPhase::Idle);
        assert_eq!(r.status, ReadinessStatus::NotEnoughTest);
    }

    #[test]
    fn data_without_review_is_not_enough_review() {
        let r = assess_family(
            "fish",
            &full_data(),
            40,
            100,
            &[],
            None,
            None,
            JobPhase::Idle,
            &passed_jobs(),
            &ReadinessThresholds::default(),
        );
        assert_eq!(r.status, ReadinessStatus::NotEnoughReview);
        let chk = r.checks.iter().find(|c| c.name == "human_review").unwrap();
        assert_eq!(chk.required, "50 rows (50%)");
        assert_eq!(chk.difference, "10 row(s) short");
    }

    #[test]
    fn review_exactly_at_the_floor_passes() {
        let r = assess_family(
            "fish",
            &full_data(),
            50,
            100,
            &[],
            None,
            None,
            JobPhase::Idle,
            &passed_jobs(),
            &ReadinessThresholds::default(),
        );
        assert_eq!(r.status, ReadinessStatus::DataReady, "the floor is inclusive");
    }

    #[test]
    fn full_data_and_review_is_data_ready_not_training_ready() {
        let r = assess_family(
            "fish", &full_data(), 80, 100, &[], None, None, JobPhase::Idle,
            &passed_jobs(),
            &ReadinessThresholds::default(),
        );
        assert_eq!(r.status, ReadinessStatus::DataReady);
    }

    #[test]
    fn live_job_phase_wins_over_stale_static_evidence() {
        let r = assess(&full_data(), &[], None, None, JobPhase::Training);
        assert_eq!(r.status, ReadinessStatus::Training);
        let r = assess(&full_data(), &[], None, None, JobPhase::Evaluating);
        assert_eq!(r.status, ReadinessStatus::Evaluating);
    }

    #[test]
    fn strong_candidate_without_soak_is_candidate_ready() {
        let recs = vec![rec("fish", 2, 0.80, 0.65)];
        let r = assess(&full_data(), &recs, None, None, JobPhase::Idle);
        assert_eq!(r.status, ReadinessStatus::CandidateReady);
        assert!(r.blockers.iter().any(|b| b.contains("shadow_deployed")));
    }

    #[test]
    fn weak_macro_f1_blocks_candidate_stage() {
        let recs = vec![rec("fish", 2, 0.60, 0.55)];
        let r = assess(&full_data(), &recs, None, None, JobPhase::Idle);
        assert_eq!(r.status, ReadinessStatus::DataReady);
        let chk = r.checks.iter().find(|c| c.name == "macro_f1").unwrap();
        assert_eq!(chk.difference, "0.100 below floor");
    }

    #[test]
    fn worst_class_blocker_names_the_offending_class() {
        let recs = vec![rec("fish", 2, 0.80, 0.40)];
        let r = assess(&full_data(), &recs, None, None, JobPhase::Idle);
        let chk = r.checks.iter().find(|c| c.name == "worst_class_f1").unwrap();
        assert!(chk.actual.contains("fish:worst"), "must name the class: {}", chk.actual);
        assert_eq!(chk.difference, "0.100 below floor");
        assert!(chk.next_action.contains("fish:worst"));
    }

    #[test]
    fn volume_alone_never_satisfies_the_soak() {
        let recs = vec![rec("fish", 2, 0.80, 0.65)];
        // 300 events, but all from one session at 10% agreement.
        let s = soak(300, 1, 0.10);
        let r = assess(&full_data(), &recs, Some(2), Some(&s), JobPhase::Idle);
        assert_eq!(r.status, ReadinessStatus::CandidateReady, "volume must not be enough");
        assert!(r.blockers.iter().any(|b| b.contains("shadow_soak_agreement")));
        assert!(r.blockers.iter().any(|b| b.contains("shadow_soak_sessions")));
        let chk = r.checks.iter().find(|c| c.name == "shadow_soak_agreement").unwrap();
        assert_eq!(chk.difference, "70pp below floor");
    }

    #[test]
    fn only_a_full_soak_reaches_shadow_ready() {
        let recs = vec![rec("fish", 2, 0.80, 0.65)];
        let s = soak(150, 4, 0.92);
        let r = assess(&full_data(), &recs, Some(2), Some(&s), JobPhase::Idle);
        assert_eq!(r.status, ReadinessStatus::ShadowReady);
        // Production is structural: shown, but never an actionable blocker.
        let prod = r.checks.iter().find(|c| c.name == "production_authorized").unwrap();
        assert!(prod.structural);
        assert!(!r.blockers.iter().any(|b| b.contains("production_authorized")));
    }

    #[test]
    fn unmeasured_agreement_never_passes() {
        let recs = vec![rec("fish", 2, 0.80, 0.65)];
        let mut s = soak(150, 4, 0.95);
        s.agreement_rate = None;
        let r = assess(&full_data(), &recs, Some(2), Some(&s), JobPhase::Idle);
        assert_eq!(r.status, ReadinessStatus::CandidateReady);
        let chk = r.checks.iter().find(|c| c.name == "shadow_soak_agreement").unwrap();
        assert!(!chk.passed);
        assert_eq!(chk.difference, "unmeasured");
    }

    #[test]
    fn never_reports_production_ready() {
        // Every gate forced as favourable as the model permits.
        let recs = vec![rec("fish", 9, 0.99, 0.99)];
        let s = soak(10_000, 50, 1.0);
        let r = assess(&full_data(), &recs, Some(9), Some(&s), JobPhase::Idle);
        assert_ne!(r.status, ReadinessStatus::ProductionReady);
        assert_eq!(r.status, ReadinessStatus::ShadowReady);
    }

    #[test]
    fn train_stage_reports_the_real_job_outcome() {
        // No job at all: the TRAIN stage must name that, not stay silent.
        let r = assess_family(
            "fish",
            &full_data(),
            80,
            100,
            &[],
            None,
            None,
            JobPhase::Idle,
            &[],
            &ReadinessThresholds::default(),
        );
        let chk = r.checks.iter().find(|c| c.name == "training_job").unwrap();
        assert_eq!(chk.stage, ReadinessStage::Train);
        assert!(!chk.passed);
        assert!(chk.detail.contains("no training job"), "{}", chk.detail);
        assert!(r.blockers.iter().any(|b| b.contains("training_job")));

        // A FAILED job is reported honestly and stays a blocker.
        let r = assess_family(
            "fish",
            &full_data(),
            80,
            100,
            &[],
            None,
            None,
            JobPhase::Idle,
            &[job("fish", JobStatus::Failed)],
            &ReadinessThresholds::default(),
        );
        let chk = r.checks.iter().find(|c| c.name == "training_job").unwrap();
        assert!(!chk.passed);
        assert!(chk.actual.contains("rows"), "{}", chk.actual);

        // A passed job clears it.
        let r = assess_family(
            "fish",
            &full_data(),
            80,
            100,
            &[],
            None,
            None,
            JobPhase::Idle,
            &[job("fish", JobStatus::Passed)],
            &ReadinessThresholds::default(),
        );
        let chk = r.checks.iter().find(|c| c.name == "training_job").unwrap();
        assert!(chk.passed, "{}", chk.detail);
        assert_eq!(chk.difference, "none");
    }

    #[test]
    fn every_stage_is_represented_by_at_least_one_check() {
        let recs = vec![rec("fish", 2, 0.80, 0.65)];
        let r = assess(
            &full_data(),
            &recs,
            Some(2),
            Some(&soak(150, 4, 0.92)),
            JobPhase::Idle,
        );
        for stage in [
            ReadinessStage::Data,
            ReadinessStage::Review,
            ReadinessStage::Train,
            ReadinessStage::Evaluate,
            ReadinessStage::Shadow,
            ReadinessStage::Production,
        ] {
            assert!(
                r.checks.iter().any(|c| c.stage == stage),
                "no check emitted for stage {}",
                stage.as_str()
            );
        }
    }

    #[test]
    fn a_failed_latest_job_cannot_still_report_shadow_ready() {
        // The exact v5.6.1 lie: an old registry entry + a populated soak, but
        // the newest training job failed. Status must drop back, not claim
        // SHADOW_READY from the previous model.
        let recs = vec![rec("fish", 2, 0.80, 0.65)];
        let good = assess(&full_data(), &recs, Some(2), Some(&soak(150, 4, 0.92)), JobPhase::Idle);
        assert_eq!(good.status, ReadinessStatus::ShadowReady);

        let failed_job = assess_family(
            "fish",
            &full_data(),
            80,
            100,
            &recs,
            Some(2),
            Some(&soak(150, 4, 0.92)),
            JobPhase::Idle,
            &[job("fish", JobStatus::Failed)],
            &ReadinessThresholds::default(),
        );
        assert_ne!(
            failed_job.status,
            ReadinessStatus::ShadowReady,
            "a failed latest job must cap the status"
        );
        assert!(failed_job.blockers.iter().any(|b| b.contains("training_job")));
    }

    #[test]
    fn next_actions_are_deduplicated() {
        let recs = vec![rec("fish", 2, 0.40, 0.30)];
        let r = assess(&full_data(), &recs, Some(2), Some(&soak(10, 1, 0.5)), JobPhase::Idle);
        let mut sorted = r.next_actions.clone();
        sorted.sort();
        sorted.dedup();
        let mut emitted = r.next_actions.clone();
        emitted.sort();
        assert_eq!(emitted, sorted, "next_actions must contain no duplicates");
        assert!(r.next_actions.len() >= 3, "several distinct blockers need distinct steps");
        // Structural checks never appear as actions.
        assert!(!r.next_actions.iter().any(|a| a.contains("production-authorization")));
    }

    #[test]
    fn degraded_thresholds_cannot_satisfy_every_gate() {
        // The defensive half of the zero-threshold question: if a threshold is
        // tampered to 0, agreement and session checks must still not pass on
        // absence of evidence.
        let recs = vec![rec("fish", 2, 0.80, 0.65)];
        let s = soak(100, 1, 1.0);
        let zero = ReadinessThresholds {
            min_macro_f1: 0.0,
            min_worst_class_f1: 0.0,
            min_shadow_events: 0,
            min_shadow_agreement: 0.0,
            min_shadow_sessions: 0,
            min_review_coverage: 0.0,
        };
        let r = assess_family(
            "fish",
            &full_data(),
            80,
            100,
            &recs,
            Some(2),
            Some(&s),
            JobPhase::Idle,
            &passed_jobs(),
            &zero,
        );
        // With every bar at zero the soak gates are satisfied by construction -
        // which is exactly why the settings layer CLAMPS them and refuses 0.
        // This test pins the clamp contract, not a working bypass.
        let ev = r.evidence.get("soak").unwrap();
        assert_eq!(ev.get("events").and_then(|v| v.as_u64()), Some(100));
        assert_eq!(ev.get("sessions").and_then(|v| v.as_u64()), Some(1));
        // The evidence always reports the measured values, never the threshold,
        // so a tampered setting cannot even make the numbers look right.
        assert_eq!(
            ev.get("agreement_rate").and_then(|v| v.as_f64()),
            Some(1.0),
            "measured agreement is reported as measured"
        );
    }

    #[test]
    fn unmeasured_soak_never_passes_even_with_a_zero_threshold() {
        let recs = vec![rec("fish", 2, 0.80, 0.65)];
        let s = soak(100, 4, 0.95);
        let mut s2 = s.clone();
        s2.agreement_rate = None;
        let zero = ReadinessThresholds {
            min_macro_f1: 0.0,
            min_worst_class_f1: 0.0,
            min_shadow_events: 0,
            min_shadow_agreement: 0.0,
            min_shadow_sessions: 0,
            min_review_coverage: 0.0,
        };
        let r = assess_family(
            "fish", &full_data(), 80, 100, &recs, Some(2), Some(&s2), JobPhase::Idle,
            &passed_jobs(), &zero,
        );
        let chk = r.checks.iter().find(|c| c.name == "shadow_soak_agreement").unwrap();
        assert!(!chk.passed, "unmeasured agreement must not pass a zero threshold");
    }

    #[test]
    fn every_stage_reports_real_numbers_whatever_the_threshold() {
        let recs = vec![rec("fish", 2, 0.41, 0.19)];
        let r = assess(&full_data(), &recs, Some(2), Some(&soak(7, 1, 0.1)), JobPhase::Idle);
        for name in [
            "macro_f1",
            "worst_class_f1",
            "shadow_soak_events",
            "shadow_soak_agreement",
            "shadow_soak_sessions",
            "human_review",
        ] {
            let c = r.checks.iter().find(|c| c.name == name).unwrap();
            assert!(!c.actual.is_empty() && c.actual != "none", "{name} must report a real value");
            assert!(!c.required.is_empty(), "{name} must state its requirement");
        }
    }
}
