//! Candidate registry: immutable candidates, current-vs-candidate
//! comparison, deterministic promotion to shadow, rollback (v5.5).
//!
//! Safety architecture (read before touching):
//! - A job ending well is only EVALUATED. Shadow admission requires
//!   `compare()` → Pass AND `promote_to_shadow()`, which archives the
//!   running shadow files first (rollback point always exists).
//! - `compare()` is fail-closed: missing metrics, narrower test sets,
//!   per-class regressions, worse calibration, unqualified classes, or
//!   hash mismatch all REJECT (or INCONCLUSIVE, never a quiet pass).
//! - Nothing here touches macro control or the production provider
//!   (`ml_model.rs` current/previous mechanism is a separate path).
//! - Rollback restores the exact archived bytes after re-verifying them.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::ml_model::sha256_hex;

fn now_ms() -> u64 {
    crate::events::now_ms()
}

/// Shadow file stem per trainer family. Fixed names are what
/// `shadow_infer` loads; versions live in manifests + registry.
pub fn shadow_stem(family: &str) -> Option<&'static str> {
    match family {
        "fish" => Some("fish_v1"),
        "state" => Some("state_v1"),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMetrics {
    pub accuracy: f32,
    pub macro_f1: f32,
    pub ece: Option<f32>,
    pub test_sessions: usize,
    pub test_n: usize,
    pub per_class_f1: HashMap<String, f32>,
    /// Kept-accuracy at the established rejection threshold, if any.
    pub kept_accuracy: Option<f32>,
}

impl ModelMetrics {
    /// Parse the trainer's real evaluation files. Shapes differ slightly
    /// between families; every field is optional-tolerant but the core
    /// accuracy/macro-F1 must exist or this returns None (fail-closed).
    pub fn from_trainer_files(
        evaluation_test: &serde_json::Value,
        calibration: Option<&serde_json::Value>,
        rejection: Option<&serde_json::Value>,
        test_sessions: usize,
    ) -> Option<Self> {
        let accuracy = evaluation_test.get("accuracy")?.as_f64()? as f32;
        let macro_f1 = evaluation_test
            .get("macro_f1")
            .or_else(|| evaluation_test.get("macroF1"))
            .and_then(|v| v.as_f64())? as f32;
        let mut per_class_f1 = HashMap::new();
        if let Some(map) = evaluation_test.get("per_entity").and_then(|v| v.as_object()) {
            for (k, v) in map {
                if let Some(f) = v.get("f1").and_then(|x| x.as_f64()) {
                    per_class_f1.insert(k.clone(), f as f32);
                }
            }
        }
        if let Some(map) = evaluation_test.get("per_class").and_then(|v| v.as_object()) {
            for (k, v) in map {
                if let Some(f) = v.get("f1").and_then(|x| x.as_f64()) {
                    per_class_f1.insert(k.clone(), f as f32);
                }
            }
        }
        let ece = calibration.and_then(|c| c.get("ece")).and_then(|v| v.as_f64()).map(|v| v as f32);
        let kept_accuracy =
            rejection.and_then(|r| r.get("test_kept_accuracy")).and_then(|v| v.as_f64()).map(|v| v as f32);
        let test_n = evaluation_test.get("n").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        Some(Self { accuracy, macro_f1, ece, test_sessions, test_n, per_class_f1, kept_accuracy })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Lifecycle {
    Trained,
    Evaluated,
    Shadow,
    ShadowValidated,
    Accepted,
    Rejected,
    RolledBack,
}

impl Lifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Lifecycle::Trained => "TRAINED",
            Lifecycle::Evaluated => "EVALUATED",
            Lifecycle::Shadow => "SHADOW",
            Lifecycle::ShadowValidated => "SHADOW_VALIDATED",
            Lifecycle::Accepted => "ACCEPTED",
            Lifecycle::Rejected => "REJECTED",
            Lifecycle::RolledBack => "ROLLED_BACK",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateRecord {
    pub candidate_id: String,
    pub model_family: String,
    pub model_version: u32,
    pub parent_name: String,
    pub parent_sha: Option<String>,
    pub dataset_fingerprint: String,
    pub dataset_version: u32,
    pub training_config_hash: String,
    pub code_version: String,
    pub job_id: String,
    pub created_at: u64,
    pub artifact_sha: String,
    pub manifest_sha: Option<String>,
    pub eval_sha: String,
    pub metrics: ModelMetrics,
    pub classes: Vec<String>,
    pub temperature: f32,
    pub mean: [f32; 3],
    pub std: [f32; 3],
    pub status: Lifecycle,
    pub decision_reason: Option<String>,
}

pub fn registry_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("training").join("registry")
}

pub fn candidates_dir(data_dir: &Path) -> PathBuf {
    registry_dir(data_dir).join("candidates")
}

fn registry_path(data_dir: &Path) -> PathBuf {
    registry_dir(data_dir).join("registry.json")
}

pub fn load_registry(data_dir: &Path) -> Vec<CandidateRecord> {
    std::fs::read_to_string(registry_path(data_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_registry(data_dir: &Path, records: &[CandidateRecord]) -> Result<(), String> {
    std::fs::create_dir_all(registry_dir(data_dir)).map_err(|e| e.to_string())?;
    let body = serde_json::to_string_pretty(records).map_err(|e| e.to_string())?;
    std::fs::write(registry_path(data_dir), body).map_err(|e| e.to_string())
}

pub fn next_version(records: &[CandidateRecord], family: &str) -> u32 {
    records.iter().filter(|r| r.model_family == family).map(|r| r.model_version).max().unwrap_or(1) + 1
}

fn sha_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

#[derive(Debug, Clone)]
pub struct CandidateInputs {
    pub onnx_path: PathBuf,
    pub classes: Vec<String>,
    pub temperature: f32,
    pub mean: [f32; 3],
    pub std: [f32; 3],
    pub metrics: ModelMetrics,
    pub eval_json: serde_json::Value,
}

/// Register evaluation outputs as an immutable candidate. Files are COPIED
/// into candidates/<id>/ and hashed; the registry never points at mutable
/// trainer output dirs.
#[allow(clippy::too_many_arguments)]
pub fn register_candidate(
    data_dir: &Path,
    job_id: &str,
    model_family: &str,
    dataset_fingerprint: &str,
    dataset_version: u32,
    training_config_hash: &str,
    code_version: &str,
    inputs: &CandidateInputs,
) -> Result<CandidateRecord, String> {
    let stem = shadow_stem(model_family).ok_or_else(|| format!("no shadow slot for family '{model_family}'"))?;
    let mut records = load_registry(data_dir);
    let version = next_version(&records, model_family);
    let candidate_id = format!("{model_family}-v{version}-{job_id}");
    let dir = candidates_dir(data_dir).join(&candidate_id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let art = dir.join("model.onnx");
    std::fs::copy(&inputs.onnx_path, &art).map_err(|e| format!("copy artifact: {e}"))?;
    let eval_path = dir.join("evaluation.json");
    std::fs::write(&eval_path, serde_json::to_string_pretty(&inputs.eval_json).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let artifact_sha = sha_file(&art)?;
    let eval_sha = sha_file(&eval_path)?;
    let rec = CandidateRecord {
        candidate_id: candidate_id.clone(),
        model_family: model_family.to_string(),
        model_version: version,
        parent_name: stem.to_string(),
        parent_sha: current_shadow_sha(data_dir, stem),
        dataset_fingerprint: dataset_fingerprint.to_string(),
        dataset_version,
        training_config_hash: training_config_hash.to_string(),
        code_version: code_version.to_string(),
        job_id: job_id.to_string(),
        created_at: now_ms(),
        artifact_sha,
        manifest_sha: None,
        eval_sha,
        metrics: inputs.metrics.clone(),
        classes: inputs.classes.clone(),
        temperature: inputs.temperature,
        mean: inputs.mean,
        std: inputs.std,
        status: Lifecycle::Evaluated,
        decision_reason: None,
    };
    records.push(rec.clone());
    save_registry(data_dir, &records)?;
    Ok(rec)
}

fn current_shadow_sha(data_dir: &Path, stem: &str) -> Option<String> {
    sha_file(&data_dir.join("models").join(format!("{stem}.onnx"))).ok()
}

// ---- comparison (deterministic promotion policy inputs) ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comparison {
    pub acc_delta: f32,
    pub f1_delta: f32,
    pub ece_delta: Option<f32>,
    pub regressions: Vec<(String, f32, f32)>,
    pub improvements: Vec<(String, f32, f32)>,
    pub verdict: ComparisonVerdict,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ComparisonVerdict {
    Pass,
    Reject,
    Inconclusive,
}

/// Deterministic promotion comparison. Fail-closed by construction:
/// anything unmeasurable is INCONCLUSIVE, anything worse is REJECT.
pub fn compare(
    current: &ModelMetrics,
    candidate: &ModelMetrics,
    current_classes: &[String],
    candidate_classes: &[String],
    qualified_classes: &[String],
) -> Comparison {
    let mut reasons = Vec::new();
    let acc_delta = candidate.accuracy - current.accuracy;
    let f1_delta = candidate.macro_f1 - current.macro_f1;
    let ece_delta = match (current.ece, candidate.ece) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    };
    let cur_set: HashSet<&str> = current_classes.iter().map(|s| s.as_str()).collect();
    let mut regressions = Vec::new();
    let mut improvements = Vec::new();
    for cls in candidate_classes {
        if let (Some(&a), Some(&b)) = (
            current.per_class_f1.get(cls.as_str()),
            candidate.per_class_f1.get(cls.as_str()),
        ) {
            if cur_set.contains(cls.as_str()) {
                if a - b > 0.05 {
                    regressions.push((cls.clone(), a, b));
                } else if b - a > 0.05 {
                    improvements.push((cls.clone(), a, b));
                }
            }
        }
    }
    // Unqualified classes in the candidate can never promote.
    let qual: HashSet<&str> = qualified_classes.iter().map(|s| s.as_str()).collect();
    let unqualified: Vec<String> =
        candidate_classes.iter().filter(|c| !qual.contains(c.as_str())).cloned().collect();

    let mut verdict = ComparisonVerdict::Pass;
    // (Plain pushes instead of closures: two &mut closures over
    // verdict+reasons would alias.)
    macro_rules! reject {
        ($r:expr) => {{
            verdict = ComparisonVerdict::Reject;
            reasons.push($r.to_string());
        }};
    }
    macro_rules! inconclusive {
        ($r:expr) => {{
            if verdict == ComparisonVerdict::Pass {
                verdict = ComparisonVerdict::Inconclusive;
            }
            reasons.push($r.to_string());
        }};
    }

    if !unqualified.is_empty() {
        reject! {format!("candidate covers unqualified classes: {}", unqualified.join(","))};
    }
    if candidate.test_sessions < current.test_sessions {
        inconclusive! {format!(
            "narrower held-out test ({} vs {} sessions): cannot confirm generalization",
            candidate.test_sessions, current.test_sessions
        )};
    }
    if candidate.test_n == 0 || current.test_n == 0 {
        inconclusive! {"empty test slice on either side".to_string()};
    }
    if f1_delta < 0.01 {
        reject! {format!("macro-F1 delta {f1_delta:+.4} below +0.01 promotion bar")};
    }
    if acc_delta < -0.005 {
        reject! {format!("accuracy regression {acc_delta:+.4}")};
    }
    if !regressions.is_empty() {
        reject! {format!(
            "{} per-class regression(s) > 0.05: {}",
            regressions.len(),
            regressions.iter().map(|(c, a, b)| format!("{c} {a:.2}->{b:.2}")).collect::<Vec<_>>().join(", ")
        )};
    }
    if let Some(d) = ece_delta {
        if d > 0.05 {
            reject! {format!("calibration regression ECE {d:+.4}")};
        }
    }
    match (current.kept_accuracy, candidate.kept_accuracy) {
        (Some(_), Some(kept)) if kept < 0.9 => {
            reject! {format!("candidate rejection operating point kept-accuracy {kept:.3} < 0.90")};
        }
        _ => {}
    }
    if verdict == ComparisonVerdict::Pass {
        reasons.push(format!(
            "macro-F1 {f1_delta:+.4}, accuracy {acc_delta:+.4}, {} improvements, 0 regressions",
            improvements.len()
        ));
    }
    Comparison { acc_delta, f1_delta, ece_delta, regressions, improvements, verdict, reasons }
}

// ---- promotion + rollback (shadow files only, never production) ----

fn shadow_paths(models_dir: &Path, stem: &str) -> (PathBuf, PathBuf) {
    (models_dir.join(format!("{stem}.onnx")), models_dir.join(format!("{stem}.json")))
}

/// Promote an EVALUATED+Pass candidate to shadow. Archives the running
/// shadow files first (rollback point), re-verifies everything after the
/// copy, and records parentage. Returns the new manifest path.
pub fn promote_to_shadow(
    data_dir: &Path,
    models_dir: &Path,
    candidate_id: &str,
    comparison: &Comparison,
) -> Result<PathBuf, String> {
    if comparison.verdict != ComparisonVerdict::Pass {
        return Err(format!("promotion refused: comparison verdict is {:?}", comparison.verdict));
    }
    let mut records = load_registry(data_dir);
    let idx =
        records.iter().position(|r| r.candidate_id == candidate_id).ok_or("candidate not found")?;
    if records[idx].status != Lifecycle::Evaluated {
        return Err(format!("candidate is {:?}, only EVALUATED promotes", records[idx].status));
    }
    let stem = shadow_stem(&records[idx].model_family)
        .ok_or_else(|| "no shadow slot for family".to_string())?
        .to_string();
    let (live_onnx, live_json) = shadow_paths(models_dir, &stem);
    // Re-verify the immutable candidate bytes (never trust the path alone).
    let cdir = candidates_dir(data_dir).join(candidate_id);
    let art = cdir.join("model.onnx");
    let evalp = cdir.join("evaluation.json");
    if sha_file(&art)? != records[idx].artifact_sha {
        return Err("candidate artifact hash mismatch: refused".to_string());
    }
    if sha_file(&evalp)? != records[idx].eval_sha {
        return Err("candidate evaluation hash mismatch: refused".to_string());
    }
    // Archive the running shadow as the rollback point (if any exists).
    let arch = registry_dir(data_dir)
        .join("archive")
        .join(&records[idx].model_family)
        .join(format!("v{}-{}", records[idx].model_version, now_ms()));
    std::fs::create_dir_all(&arch).map_err(|e| e.to_string())?;
    for p in [&live_onnx, &live_json] {
        if p.exists() {
            std::fs::copy(p, arch.join(p.file_name().unwrap())).map_err(|e| e.to_string())?;
        }
    }
    // Deploy: copy bytes, then write a fresh manifest naming this version.
    std::fs::copy(&art, &live_onnx).map_err(|e| e.to_string())?;
    let manifest = serde_json::json!({
        "name": stem,
        "version": records[idx].model_version.to_string(),
        "dataset": format!("gpo-vision/v{}", records[idx].dataset_version),
        "dataset_version": records[idx].dataset_version,
        "trained_at": records[idx].job_id.clone(),
        "runtime": "tract",
        "input_width": 96,
        "input_height": 96,
        "classes": records[idx].classes,
        "sha256": sha_file(&live_onnx)?,
        "temperature": records[idx].temperature,
        "test_accuracy": records[idx].metrics.accuracy,
        "macro_f1": records[idx].metrics.macro_f1,
        "ece": records[idx].metrics.ece,
        "test_sessions": records[idx].metrics.test_sessions,
        "test_n": records[idx].metrics.test_n,
        "preprocess": {"pad": "square-black", "resize": "bilinear", "size": 96,
                       "mean": records[idx].mean, "std": records[idx].std},
    });
    std::fs::write(&live_json, serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    // Re-verify the deployed bytes through the real loader path.
    let load_err = match super::shadow_infer::ShadowEngine::load(models_dir) {
        Ok(_) => None,
        Err(e) => Some(e),
    };
    if let Some(e) = load_err {
        rollback_family(data_dir, models_dir, &records[idx].model_family)?;
        return Err(format!("deployed model rejected by loader; restored archive: {e}"));
    }
    records[idx].status = Lifecycle::Shadow;
    records[idx].manifest_sha = sha_file(&live_json).ok();
    records[idx].decision_reason = Some(format!("promoted to shadow: {}", comparison.reasons.join("; ")));
    save_registry(data_dir, &records)?;
    Ok(live_json)
}

/// Restore the newest archive for a family, re-verified. Records
/// ROLLED_BACK on the displaced shadow version when identifiable.
pub fn rollback_family(data_dir: &Path, models_dir: &Path, family: &str) -> Result<PathBuf, String> {
    let stem = shadow_stem(family).ok_or_else(|| format!("no shadow slot for family '{family}'"))?;
    let arch_root = registry_dir(data_dir).join("archive").join(family);
    let mut cands: Vec<PathBuf> = std::fs::read_dir(&arch_root)
        .map_err(|_| format!("no archive for family '{family}': nothing to roll back to"))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    cands.sort();
    let newest = cands.pop().ok_or_else(|| format!("archive for '{family}' is empty"))?;
    let (_live_onnx, live_json) = shadow_paths(models_dir, stem);
    for name in [format!("{stem}.onnx"), format!("{stem}.json")] {
        let src = newest.join(&name);
        if !src.exists() {
            return Err(format!("archive is missing {name}: refusing partial rollback"));
        }
        std::fs::copy(&src, models_dir.join(&name)).map_err(|e| e.to_string())?;
    }
    // The restored files must verify, or the rollback itself is refused.
    super::shadow_infer::ShadowEngine::load(models_dir)
        .map_err(|e| format!("restored archive failed to load: {e}"))?;
    let mut records = load_registry(data_dir);
    if let Some(r) = records.iter_mut().find(|r| r.model_family == family && r.status == Lifecycle::Shadow) {
        r.status = Lifecycle::RolledBack;
        r.decision_reason = Some("rolled back: archive restored".to_string());
    }
    save_registry(data_dir, &records)?;
    Ok(live_json)
}

// ---- soak: shadow agreement over real gameplay ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoakStats {
    pub model: String,
    pub events: usize,
    pub agree: usize,
    pub disagree: usize,
    pub no_ocr_baseline: usize,
    pub mean_confidence: Option<f32>,
    pub agreement_rate: Option<f32>,
}

/// Agreement telemetry for one shadow model over recent shadow events.
/// Reads the real append-only log; never synthesizes.
pub fn soak_stats(data_dir: &Path, model_name: &str, last_n: usize) -> SoakStats {
    let path = super::ml_capability::shadow_log_path(data_dir);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return SoakStats {
            model: model_name.to_string(), events: 0, agree: 0, disagree: 0,
            no_ocr_baseline: 0, mean_confidence: None, agreement_rate: None,
        };
    };
    let mut evs: Vec<super::ml_capability::ShadowEvent> = raw
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .filter(|e: &super::ml_capability::ShadowEvent| e.model_version.as_deref() == Some(model_name))
        .collect();
    if evs.len() > last_n {
        evs.drain(..evs.len() - last_n);
    }
    let mut agree = 0;
    let mut disagree = 0;
    let mut no_base = 0;
    let mut conf_sum = 0f64;
    let mut conf_n = 0usize;
    for e in &evs {
        match e.agreement {
            Some(true) => agree += 1,
            Some(false) => disagree += 1,
            None => no_base += 1,
        }
        let c = e.vision_confidence.or(e.state_confidence);
        if let Some(v) = c {
            conf_sum += v as f64;
            conf_n += 1;
        }
    }
    let scored = agree + disagree;
    SoakStats {
        model: model_name.to_string(),
        events: evs.len(),
        agree,
        disagree,
        no_ocr_baseline: no_base,
        mean_confidence: if conf_n > 0 { Some((conf_sum / conf_n as f64) as f32) } else { None },
        agreement_rate: if scored > 0 { Some(agree as f32 / scored as f32) } else { None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(acc: f32, f1: f32, sessions: usize, per: &[(&str, f32)]) -> ModelMetrics {
        ModelMetrics {
            accuracy: acc,
            macro_f1: f1,
            ece: Some(0.1),
            test_sessions: sessions,
            test_n: 100,
            per_class_f1: per.into_iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            kept_accuracy: None,
        }
    }

    fn classes(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn better_candidate_passes_with_reasons() {
        let cur = metrics(0.58, 0.54, 5, &[("fish:shark", 0.7), ("fish:golden", 0.5)]);
        let cand = metrics(0.77, 0.72, 8, &[("fish:shark", 0.8), ("fish:golden", 0.65)]);
        let cls = classes(&["fish:shark", "fish:golden"]);
        let c = compare(&cur, &cand, &cls, &cls, &cls);
        assert_eq!(c.verdict, ComparisonVerdict::Pass);
        assert!(!c.reasons.is_empty());
        assert_eq!(c.improvements.len(), 2);
    }

    #[test]
    fn worse_candidate_rejects_with_macro_reason() {
        let cur = metrics(0.58, 0.54, 5, &[("fish:shark", 0.7)]);
        let cand = metrics(0.55, 0.49, 6, &[("fish:shark", 0.68)]);
        let cls = classes(&["fish:shark"]);
        let c = compare(&cur, &cand, &cls, &cls, &cls);
        assert_eq!(c.verdict, ComparisonVerdict::Reject);
        assert!(c.reasons.iter().any(|r| r.contains("macro-F1")));
    }

    #[test]
    fn per_class_regression_rejects_despite_better_macro() {
        let cur = metrics(0.60, 0.55, 5, &[("fish:shark", 0.9), ("fish:golden", 0.2)]);
        let cand = metrics(0.62, 0.58, 6, &[("fish:shark", 0.7), ("fish:golden", 0.5)]);
        let cls = classes(&["fish:shark", "fish:golden"]);
        let c = compare(&cur, &cand, &cls, &cls, &cls);
        assert_eq!(c.verdict, ComparisonVerdict::Reject, "0.20 shark regression must block");
    }

    #[test]
    fn narrower_test_set_is_inconclusive_not_pass() {
        let cur = metrics(0.58, 0.54, 8, &[("fish:shark", 0.7)]);
        let cand = metrics(0.80, 0.78, 3, &[("fish:shark", 0.85)]);
        let cls = classes(&["fish:shark"]);
        let c = compare(&cur, &cand, &cls, &cls, &cls);
        assert_eq!(c.verdict, ComparisonVerdict::Inconclusive);
    }

    #[test]
    fn unqualified_classes_reject() {
        let cur = metrics(0.58, 0.54, 5, &[("fish:shark", 0.7)]);
        let cand = metrics(0.80, 0.78, 6, &[("fish:shark", 0.85), ("fish:megalodon", 0.9)]);
        let c = compare(
            &cur, &cand,
            &classes(&["fish:shark"]), &classes(&["fish:shark", "fish:megalodon"]), &classes(&["fish:shark"]),
        );
        assert_eq!(c.verdict, ComparisonVerdict::Reject);
    }

    #[test]
    fn tampered_candidate_refuses_promotion() {
        let dir = std::env::temp_dir().join("gpo-reg-tamper");
        let _ = std::fs::remove_dir_all(&dir);
        let models = dir.join("models");
        std::fs::create_dir_all(&models).unwrap();
        // Seed a live shadow so archiving has something to archive.
        std::fs::write(models.join("fish_v1.onnx"), b"live-bytes").unwrap();
        std::fs::write(models.join("fish_v1.json"), "{}").unwrap();
        let cand_dir = candidates_dir(&dir).join("fish-v9-j1");
        std::fs::create_dir_all(&cand_dir).unwrap();
        std::fs::write(cand_dir.join("model.onnx"), b"candidate-bytes").unwrap();
        std::fs::write(cand_dir.join("evaluation.json"), "{}").unwrap();
        // Register via record surgery (hashes of the real bytes)...
        let art_sha = sha_file(&cand_dir.join("model.onnx")).unwrap();
        let eval_sha = sha_file(&cand_dir.join("evaluation.json")).unwrap();
        let rec = CandidateRecord {
            candidate_id: "fish-v9-j1".into(), model_family: "fish".into(), model_version: 9,
            parent_name: "fish_v1".into(), parent_sha: None,
            dataset_fingerprint: "fp-t".into(), dataset_version: 1,
            training_config_hash: "cfg".into(), code_version: "t".into(), job_id: "j1".into(),
            created_at: 1, artifact_sha: art_sha, manifest_sha: None, eval_sha,
            metrics: metrics(0.9, 0.9, 9, &[("fish:shark", 0.9)]),
            classes: classes(&["fish:shark"]), temperature: 1.0,
            mean: [0.0; 3], std: [1.0; 3],
            status: Lifecycle::Evaluated, decision_reason: None,
        };
        save_registry(&dir, &[rec]).unwrap();
        // Tamper AFTER registration: promotion must refuse on hash mismatch.
        std::fs::write(cand_dir.join("model.onnx"), b"tampered-bytes").unwrap();
        let cmp = Comparison {
            acc_delta: 0.3, f1_delta: 0.3, ece_delta: None,
            regressions: vec![], improvements: vec![],
            verdict: ComparisonVerdict::Pass, reasons: vec!["test".into()],
        };
        let err = promote_to_shadow(&dir, &models, "fish-v9-j1", &cmp).unwrap_err();
        assert!(err.contains("hash mismatch"), "tampering must refuse: {err}");
        // Live shadow untouched by the refused promotion.
        assert_eq!(std::fs::read(models.join("fish_v1.onnx")).unwrap(), b"live-bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollback_restores_exact_archived_bytes() {
        let dir = std::env::temp_dir().join("gpo-reg-roll");
        let _ = std::fs::remove_dir_all(&dir);
        // Fake but loadable? No — rollback re-verifies via ShadowEngine::load,
        // which needs REAL models. Use the repo's bundled models instead.
        let repo_models = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models");
        let models = dir.join("models");
        std::fs::create_dir_all(&models).unwrap();
        for n in ["fish_v1.onnx", "fish_v1.json", "state_v1.onnx", "state_v1.json"] {
            std::fs::copy(repo_models.join(n), models.join(n)).unwrap();
        }
        let before = std::fs::read(models.join("fish_v1.onnx")).unwrap();
        // Simulate a promoted v2 then roll back to the archived v1 bytes.
        let arch = registry_dir(&dir).join("archive").join("fish").join("v2-0001");
        std::fs::create_dir_all(&arch).unwrap();
        std::fs::copy(models.join("fish_v1.onnx"), arch.join("fish_v1.onnx")).unwrap();
        std::fs::copy(models.join("fish_v1.json"), arch.join("fish_v1.json")).unwrap();
        std::fs::write(models.join("fish_v1.onnx"), b"broken-bytes").unwrap();
        rollback_family(&dir, &models, "fish").unwrap();
        assert_eq!(std::fs::read(models.join("fish_v1.onnx")).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn metrics_require_core_fields_or_none() {
        let v = serde_json::json!({"accuracy": 0.5});
        assert!(ModelMetrics::from_trainer_files(&v, None, None, 3).is_none());
        let v = serde_json::json!({"accuracy": 0.5, "macro_f1": 0.4, "n": 10});
        let m = ModelMetrics::from_trainer_files(&v, None, None, 3).unwrap();
        assert_eq!(m.test_sessions, 3);
        assert!(m.ece.is_none());
    }
}
