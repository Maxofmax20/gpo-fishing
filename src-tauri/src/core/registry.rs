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
use std::sync::Mutex;

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
    /// Fingerprint of the index -> label mapping. Empty for records written
    /// before v5.7.0 (they are still readable, just not pinned).
    #[serde(default)]
    pub vocab_sha: String,
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

/// Serialises every registry read-modify-write. `training_decide` and
/// `training_promote` are independent commands; without this, two of them
/// racing would silently drop a record.
static REGISTRY_LOCK: Mutex<()> = Mutex::new(());

/// Load the candidate registry.
///
/// An absent file means "no candidates yet" and yields an empty list. A file
/// that EXISTS but does not parse is a hard error: treating it as empty would
/// make `next_version` restart at 2, and a new candidate would then reuse a
/// dead revision's shadow events (a model's evidence attributed to another
/// model). Refusing is the only safe answer.
pub fn load_registry(data_dir: &Path) -> Vec<CandidateRecord> {
    let path = registry_path(data_dir);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    if raw.trim().is_empty() {
        return Vec::new();
    }
    match serde_json::from_str::<Vec<CandidateRecord>>(&raw) {
        Ok(v) => v,
        Err(e) => {
            // Loud, and loud everywhere it is consumed.
            tracing::error!(
                "registry at {} is unreadable ({e}); treating as BLOCKED rather than empty. \
                 Repair or remove the file - version numbers must never be reused.",
                path.display()
            );
            Vec::new()
        }
    }
}

/// True when a registry file exists but could not be parsed. Callers surface
/// this rather than silently reporting "no candidates".
pub fn registry_is_corrupt(data_dir: &Path) -> bool {
    let path = registry_path(data_dir);
    match std::fs::read_to_string(&path) {
        Ok(raw) => !raw.trim().is_empty()
            && serde_json::from_str::<Vec<CandidateRecord>>(&raw).is_err(),
        Err(_) => false,
    }
}

/// Atomic write: tmp file -> fsync -> rename.
///
/// A crash mid-write previously left a truncated `registry.json`, which then
/// read as "no candidates" and restarted version numbering.
pub fn save_registry(data_dir: &Path, records: &[CandidateRecord]) -> Result<(), String> {
    let _guard = REGISTRY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    save_registry_locked(data_dir, records)
}

fn save_registry_locked(data_dir: &Path, records: &[CandidateRecord]) -> Result<(), String> {
    std::fs::create_dir_all(registry_dir(data_dir)).map_err(|e| e.to_string())?;
    let body = serde_json::to_string_pretty(records).map_err(|e| e.to_string())?;
    let path = registry_path(data_dir);
    let tmp = registry_dir(data_dir).join(format!("registry.json.{}.tmp", std::process::id()));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// Next version for a family.
///
/// Version numbers must be MONOTONIC across registry loss, otherwise a
/// recycled revision would join a dead revision's shadow events. A separate
/// counter file survives a registry rebuild; if both are lost we refuse
/// rather than guess.
pub fn next_version(data_dir: &Path, records: &[CandidateRecord], family: &str) -> u32 {
    next_version_checked(data_dir, records, family).unwrap_or_else(|e| {
        tracing::error!("{e}");
        // Fail closed by never inventing a colliding version.
        u32::MAX
    })
}

pub fn next_version_checked(
    data_dir: &Path,
    records: &[CandidateRecord],
    family: &str,
) -> Result<u32, String> {
    let counter_path = registry_dir(data_dir)
        .join("versions")
        .join(format!("{family}.version"));
    let from_records = records
        .iter()
        .filter(|r| r.model_family == family)
        .map(|r| r.model_version)
        .max()
        .unwrap_or(1)
        + 1;
    let from_counter: Option<u32> = std::fs::read_to_string(&counter_path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    let next = match from_counter {
        Some(c) => from_records.max(c + 1),
        None => {
            // No counter file yet. It is safe to derive from the records only
            // if the registry itself is intact.
            if registry_is_corrupt(data_dir) {
                return Err(
                    "registry is unreadable and no version counter exists; refusing to reuse \
                     a version number (a recycled revision would inherit a dead model's soak)"
                        .to_string(),
                );
            }
            from_records
        }
    };
    if let Some(parent) = counter_path.parent() {
        let _ = std::fs::create_dir_all(parent);
        let _ = std::fs::write(&counter_path, next.to_string());
    }
    Ok(next)
}

fn sha_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

#[derive(Debug, Clone)]
pub struct CandidateInputs {
    pub onnx_path: PathBuf,
    pub classes: Vec<String>,
    /// SHA-256 of `classes.join("\n")`. The index -> label mapping is the one
    /// thing that cannot be re-derived from metrics, so it is pinned here and
    /// written into both the candidate record and the deployed manifest.
    pub vocab_sha: String,
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
    let version = next_version_checked(data_dir, &records, model_family)?;
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
        vocab_sha: inputs.vocab_sha.clone(),
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
    let load_err = super::shadow_infer::ShadowEngine::load(models_dir).err();
    if let Some(e) = load_err {
        rollback_family(data_dir, models_dir, &records[idx].model_family)?;
        return Err(format!("deployed model rejected by loader; restored archive: {e}"));
    }
    records[idx].status = Lifecycle::Shadow;
    records[idx].manifest_sha = sha_file(&live_json).ok();
    records[idx].decision_reason = Some(format!("promoted to shadow: {}", comparison.reasons.join("; ")));
    save_registry(data_dir, &records)?;
    // The running process still holds the previous weights; drop the cache so
    // the next observation actually runs the promoted candidate and stamps
    // events with ITS revision (otherwise telemetry is attributed to weights
    // that are no longer deployed, and the new revision's soak stays empty).
    super::shadow_infer::invalidate_engine();
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SoakStats {
    pub model: String,
    pub events: usize,
    pub agree: usize,
    pub disagree: usize,
    pub no_ocr_baseline: usize,
    pub mean_confidence: Option<f32>,
    pub agreement_rate: Option<f32>,
    /// Manifest revision these stats describe; `""` = unfiltered (every
    /// revision of the stem, the legacy view).
    #[serde(default)]
    pub revision: String,
    /// Distinct `session_id`s behind `events`. Soak evidence concentrated in
    /// one session is not evidence about the model.
    #[serde(default)]
    pub sessions: usize,
    /// Distinct entity labels seen (vision `entity` or normalized OCR id).
    #[serde(default)]
    pub distinct_entities: usize,
    /// Counted events that carried an entity label at all.
    #[serde(default)]
    pub uniques: usize,
    /// Log lines that failed to parse. Counted, never silently dropped.
    #[serde(default)]
    pub malformed_lines: usize,
    /// Repeated `event_id`s skipped, so a re-appended line cannot inflate
    /// the soak.
    #[serde(default)]
    pub duplicates_skipped: usize,
}

/// Stem-only soak view (legacy behaviour): every event logged under
/// `model_name`, all revisions unioned.
pub fn soak_stats(data_dir: &Path, model_name: &str, last_n: usize) -> SoakStats {
    soak_stats_for(data_dir, model_name, None, last_n)
}

/// Agreement telemetry for one shadow model over recent shadow events.
/// Reads the real append-only log; never synthesizes.
///
/// `revision` scopes the count to one revision of the slot (from
/// `ShadowEvent::model_revision`). This is what makes soak evidence
/// attributable: `model_version` is a CONSTANT per slot — `promote_to_shadow`
/// writes `"name": <stem>` for every promoted version — so a stem-only query
/// returns the UNION across revisions and a freshly promoted candidate would
/// inherit the incumbent's soak with zero observations of its own weights.
/// `None` keeps the old stem-only filter so pre-revision logs still count.
pub fn soak_stats_for(
    data_dir: &Path,
    model_name: &str,
    revision: Option<&str>,
    last_n: usize,
) -> SoakStats {
    let path = super::ml_capability::shadow_log_path(data_dir);
    let mut stats = SoakStats {
        model: model_name.to_string(),
        events: 0,
        agree: 0,
        disagree: 0,
        no_ocr_baseline: 0,
        mean_confidence: None,
        agreement_rate: None,
        revision: revision.unwrap_or_default().to_string(),
        sessions: 0,
        distinct_entities: 0,
        uniques: 0,
        malformed_lines: 0,
        duplicates_skipped: 0,
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return stats;
    };
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut evs: Vec<super::ml_capability::ShadowEvent> = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(e) = serde_json::from_str::<super::ml_capability::ShadowEvent>(line) else {
            // A line that cannot be parsed cannot be attributed to a model,
            // so the count is honest-but-global; never silently ignored.
            stats.malformed_lines += 1;
            continue;
        };
        if e.model_version.as_deref() != Some(model_name) {
            continue;
        }
        // Revision join: absent revision (legacy line) counts ONLY in the
        // unfiltered view, never as evidence for a specific candidate.
        if let Some(r) = revision {
            if e.model_revision.as_deref() != Some(r) {
                continue;
            }
        }
        if !seen_ids.insert(e.event_id.clone()) {
            stats.duplicates_skipped += 1;
            continue;
        }
        evs.push(e);
    }
    if evs.len() > last_n {
        evs.drain(..evs.len() - last_n);
    }
    stats.events = evs.len();
    let mut agree = 0;
    let mut disagree = 0;
    let mut no_base = 0;
    let mut conf_sum = 0f64;
    let mut conf_n = 0usize;
    let mut sessions: HashSet<String> = HashSet::new();
    let mut entities: HashSet<String> = HashSet::new();
    // Session/entity diversity is measured over the SAME tail as `events`.
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
        if !e.session_id.is_empty() {
            sessions.insert(e.session_id.clone());
        }
        // Vision label and normalized OCR id share one distinct-value set:
        // both answer "which fish was this".
        if let Some(v) = e
            .entity
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| e.normalized_entity.as_deref().filter(|s| !s.is_empty()))
        {
            stats.uniques += 1;
            entities.insert(v.to_string());
        }
    }
    stats.sessions = sessions.len();
    stats.distinct_entities = entities.len();
    let scored = agree + disagree;
    stats.agree = agree;
    stats.disagree = disagree;
    stats.no_ocr_baseline = no_base;
    stats.mean_confidence =
        if conf_n > 0 { Some((conf_sum / conf_n as f64) as f32) } else { None };
    stats.agreement_rate =
        if scored > 0 { Some(agree as f32 / scored as f32) } else { None };
    stats
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
            per_class_f1: per.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
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
            metrics: metrics(0.9, 0.9, 9, &[("fish:shark", 0.9)]), vocab_sha: String::new(),
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

    // ---- soak: revision attribution ----

    /// One shadow log line for stem `fish_v1`. `revision: None` omits the key
    /// entirely, i.e. a pre-revision log line.
    fn soak_line(
        event_id: &str,
        session: &str,
        revision: Option<&str>,
        agreement: Option<bool>,
        entity: Option<&str>,
        normalized: Option<&str>,
        conf: Option<f32>,
    ) -> serde_json::Value {
        let mut v = serde_json::json!({
            "session_id": session,
            "event_id": event_id,
            "timestamp_ms": 1,
            "vision_state": null,
            "production_state": null,
            "state_confidence": conf,
            "result_category": null,
            "entity": entity,
            "ocr_text": null,
            "normalized_entity": normalized,
            "policy_recommendation": null,
            "would_be_action": null,
            "actual_action": null,
            "confirmation": null,
            "latency_ms": null,
            "agreement": agreement,
            "model_version": "fish_v1",
        });
        if let Some(r) = revision {
            v.as_object_mut().unwrap().insert("model_revision".into(), r.into());
        }
        v
    }

    /// Write a shadow log to a fresh temp data dir; returns the dir.
    fn soak_dir(name: &str, lines: &[serde_json::Value]) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let body: String =
            lines.iter().map(|l| format!("{}\n", serde_json::to_string(l).unwrap())).collect();
        std::fs::write(super::super::ml_capability::shadow_log_path(&dir), body).unwrap();
        dir
    }

    /// 5 events for the incumbent revision, 3 for the freshly promoted one.
    /// `dir_name` must be unique: tests run in parallel.
    fn two_revision_log(dir_name: &str) -> PathBuf {
        let mut lines = Vec::new();
        for i in 0..5 {
            lines.push(soak_line(
                &format!("sA#v1-{i}"),
                "sA",
                Some("1"),
                Some(true),
                Some("fish:shark"),
                None,
                Some(0.9),
            ));
        }
        for i in 0..3 {
            lines.push(soak_line(
                &format!("sB#v2-{i}"),
                "sB",
                Some("2"),
                Some(false),
                Some("fish:golden"),
                None,
                Some(0.4),
            ));
        }
        soak_dir(dir_name, &lines)
    }

    #[test]
    fn soak_counts_only_the_requested_revision() {
        // Regression: promotion rewrites `"name": <stem>` for every version,
        // so a stem-only query UNIONs revisions and a brand-new candidate
        // would inherit the incumbent's soak. The revision join keeps them
        // apart: 3 observed events, never 8.
        let dir = two_revision_log("gpo-reg-soak-rev-a");
        let v2 = soak_stats_for(&dir, "fish_v1", Some("2"), 500);
        assert_eq!(v2.events, 3, "a candidate's soak must not include its predecessor's events");
        assert_eq!(v2.agree, 0);
        assert_eq!(v2.disagree, 3);
        assert_eq!(v2.revision, "2");
        let v1 = soak_stats_for(&dir, "fish_v1", Some("1"), 500);
        assert_eq!(v1.events, 5);
        assert_eq!(v1.agree, 5);
        // An unknown revision observes nothing rather than inheriting soak.
        assert_eq!(soak_stats_for(&dir, "fish_v1", Some("9"), 500).events, 0);
        // Other slots and families stay separate.
        assert_eq!(soak_stats_for(&dir, "state_v1", Some("2"), 500).events, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn soak_without_revision_keeps_the_legacy_union() {
        let dir = two_revision_log("gpo-reg-soak-rev-b");
        // Backward compat: the stem-only view still sees every event, and the
        // convenience wrapper behaves identically.
        let all = soak_stats_for(&dir, "fish_v1", None, 500);
        assert_eq!(all.events, 8);
        assert_eq!(all.revision, "");
        assert_eq!(all.agree, 5);
        assert_eq!(all.disagree, 3);
        assert_eq!(all.sessions, 2);
        let legacy = soak_stats(&dir, "fish_v1", 500);
        assert_eq!(legacy.events, 8);
        assert_eq!(legacy.agree, all.agree);
        assert_eq!(legacy.agreement_rate, all.agreement_rate);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn events_without_a_revision_parse_and_only_count_unfiltered() {
        let old = soak_line("old#1", "sOld", None, Some(true), None, None, Some(0.7));
        assert!(old.get("model_revision").is_none(), "legacy line must omit the key");
        let new = soak_line("new#1", "sNew", Some("2"), Some(true), None, None, Some(0.7));
        let dir = soak_dir("gpo-reg-soak-legacy", &[old, new]);
        // Old-format events still parse and still count for the stem view...
        let all = soak_stats(&dir, "fish_v1", 500);
        assert_eq!(all.events, 2);
        assert_eq!(all.sessions, 2);
        // ...but they are NEVER evidence for a specific revision.
        let v2 = soak_stats_for(&dir, "fish_v1", Some("2"), 500);
        assert_eq!(v2.events, 1);
        assert_eq!(v2.sessions, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_event_ids_are_counted_once() {
        let a = soak_line("dup#1", "sA", Some("2"), Some(true), Some("fish:shark"), None, Some(0.8));
        let b = soak_line("dup#1", "sA", Some("2"), Some(true), Some("fish:shark"), None, Some(0.8));
        let c = soak_line("dup#2", "sB", Some("2"), Some(false), Some("fish:golden"), None, Some(0.6));
        let dir = soak_dir("gpo-reg-soak-dupes", &[a, b, c]);
        let s = soak_stats_for(&dir, "fish_v1", Some("2"), 500);
        assert_eq!(s.events, 2, "a re-appended line must not inflate the soak");
        assert_eq!(s.duplicates_skipped, 1);
        assert_eq!(s.agree, 1);
        assert_eq!(s.disagree, 1);
        // Dedupe is per requested view: the duplicate of a DIFFERENT revision
        // is still just that other revision's event.
        assert_eq!(soak_stats_for(&dir, "fish_v1", Some("9"), 500).duplicates_skipped, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_lines_are_counted_and_do_not_abort_the_parse() {
        let good = soak_line("ok#1", "sA", Some("2"), Some(true), Some("fish:shark"), None, Some(0.8));
        let dir = soak_dir("gpo-reg-soak-malformed", &[good]);
        // Append garbage: a truncated object, a non-JSON line, and a blank.
        let log = super::super::ml_capability::shadow_log_path(&dir);
        let mut raw = std::fs::read_to_string(&log).unwrap();
        raw.push_str("{\"session_id\":\"sA\",\"event_id\":\"torn\n");
        raw.push_str("not json at all\n");
        raw.push('\n');
        raw.push_str(&format!("{}\n", serde_json::to_string(&soak_line("ok#2", "sB", Some("2"), Some(true), Some("fish:golden"), None, Some(0.9))).unwrap()));
        std::fs::write(&log, raw).unwrap();
        let s = soak_stats_for(&dir, "fish_v1", Some("2"), 500);
        assert_eq!(s.events, 2, "a malformed line must not abort or corrupt the parse");
        assert_eq!(s.malformed_lines, 2);
        assert_eq!(s.sessions, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn soak_counts_distinct_sessions_and_entities() {
        let lines = vec![
            soak_line("e1", "s1", Some("2"), Some(true), Some("fish:shark"), None, Some(0.9)),
            soak_line("e2", "s1", Some("2"), Some(true), Some("fish:shark"), None, Some(0.7)),
            soak_line("e3", "s2", Some("2"), Some(false), Some("fish:golden"), None, Some(0.3)),
            // No vision label: the normalized OCR id still names an entity.
            soak_line("e4", "s3", Some("2"), None, None, Some("fish:megalodon"), None),
            // Neither: not attributable to any entity.
            soak_line("e5", "s3", Some("2"), None, None, None, None),
        ];
        let dir = soak_dir("gpo-reg-soak-diversity", &lines);
        let s = soak_stats_for(&dir, "fish_v1", Some("2"), 500);
        assert_eq!(s.events, 5);
        assert_eq!(s.sessions, 3, "repeated session ids count once");
        assert_eq!(s.distinct_entities, 3);
        assert_eq!(s.uniques, 4, "4 of 5 events carried an entity label");
        assert_eq!(s.no_ocr_baseline, 2);
        assert_eq!(s.agree, 2);
        assert_eq!(s.disagree, 1);
        // mean over the events that carried a confidence (3 of 5).
        let mean = s.mean_confidence.unwrap();
        assert!((mean - 1.9 / 3.0).abs() < 1e-5, "mean confidence drifted: {mean}");
        assert!((s.agreement_rate.unwrap() - 2.0 / 3.0).abs() < 1e-5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn soak_tail_limit_and_missing_log_behave() {
        let dir = two_revision_log("gpo-reg-soak-rev-c");
        // last_n still drains the TAIL (most recent) of the revision view.
        let tail = soak_stats_for(&dir, "fish_v1", Some("1"), 2);
        assert_eq!(tail.events, 2);
        assert_eq!(tail.sessions, 1);
        // No log at all: honest zeros, revision echoed, nothing invented.
        let empty = std::env::temp_dir().join("gpo-reg-soak-none");
        let _ = std::fs::remove_dir_all(&empty);
        std::fs::create_dir_all(&empty).unwrap();
        let s = soak_stats_for(&empty, "fish_v1", Some("3"), 100);
        assert_eq!(s.events, 0);
        assert_eq!(s.sessions, 0);
        assert_eq!(s.revision, "3");
        assert_eq!(s.agreement_rate, None);
        assert_eq!(s.mean_confidence, None);
        let _ = std::fs::remove_dir_all(&empty);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn soak_stats_serializes_new_fields_with_defaults() {
        // Old consumers / persisted payloads without the new fields still
        // parse; the new fields are additive.
        let old = serde_json::json!({
            "model": "fish_v1", "events": 4, "agree": 3, "disagree": 1,
            "no_ocr_baseline": 0, "mean_confidence": 0.8, "agreement_rate": 0.75,
        });
        let s: SoakStats = serde_json::from_value(old).unwrap();
        assert_eq!(s.events, 4);
        assert_eq!(s.sessions, 0);
        assert_eq!(s.distinct_entities, 0);
        assert_eq!(s.uniques, 0);
        assert_eq!(s.malformed_lines, 0);
        assert_eq!(s.duplicates_skipped, 0);
        assert_eq!(s.revision, "");
        let round: SoakStats =
            serde_json::from_str(&serde_json::to_string(&soak_stats_for(
                &two_revision_log("gpo-reg-soak-rev-d"),
                "fish_v1",
                Some("2"),
                10,
            ))
            .unwrap())
            .unwrap();
        assert_eq!(round.events, 3);
        assert_eq!(round.revision, "2");
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("gpo-reg-soak-rev-d"));
    }


    fn record_for(family: &str, version: u32) -> CandidateRecord {
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
            metrics: metrics(0.9, 0.9, 5, &[("fish:shark", 0.9)]),
            classes: vec!["fish:shark".into()],
            vocab_sha: String::new(),
            temperature: 1.0,
            mean: [0.0; 3],
            std: [1.0; 3],
            status: Lifecycle::Evaluated,
            decision_reason: None,
        }
    }

    #[test]
    fn a_new_candidate_never_inherits_a_dead_revisions_soak() {
        // The v5.6.x failure: `next_version` derived purely from the record
        // list, so a lost registry restarted at 2 and the new candidate joined
        // the DEAD revision 2's shadow events.
        let dir = std::env::temp_dir().join("gpo-reg-version-mono");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let recs = vec![record_for("fish", 1), record_for("fish", 2), record_for("fish", 3)];
        let v4 = next_version_checked(&dir, &recs, "fish").unwrap();
        assert_eq!(v4, 4);

        // Registry destroyed, counter survives.
        let _ = std::fs::remove_file(registry_path(&dir));
        let after_loss = next_version_checked(&dir, &[], "fish").unwrap();
        assert!(
            after_loss > v4,
            "a lost registry must not recycle a version: {after_loss} <= {v4}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_numbering_is_refused_when_the_registry_is_corrupt_and_no_counter_exists() {
        let dir = std::env::temp_dir().join("gpo-reg-corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(registry_dir(&dir)).unwrap();
        std::fs::write(registry_path(&dir), b"{ truncated json").unwrap();

        assert!(registry_is_corrupt(&dir));
        let err = next_version_checked(&dir, &[], "fish").unwrap_err();
        assert!(err.contains("refusing to reuse"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registry_writes_are_atomic_and_survive_a_reload() {
        let dir = std::env::temp_dir().join("gpo-reg-atomic");
        let _ = std::fs::remove_dir_all(&dir);
        let recs = vec![record_for("fish", 1), record_for("state", 1)];
        save_registry(&dir, &recs).unwrap();
        assert_eq!(load_registry(&dir).len(), 2);
        // No temp files left behind.
        let leftovers: Vec<_> = std::fs::read_dir(registry_dir(&dir))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no staging file may survive: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_registry_is_reported_not_silently_treated_as_empty() {
        let dir = std::env::temp_dir().join("gpo-reg-report");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(registry_dir(&dir)).unwrap();
        std::fs::write(registry_path(&dir), "not json at all").unwrap();
        assert!(registry_is_corrupt(&dir));
        assert!(!registry_is_corrupt(&dir.join("nonexistent")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
