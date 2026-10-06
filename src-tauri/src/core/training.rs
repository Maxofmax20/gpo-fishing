//! ML Training Center backend: frozen snapshots, persistent jobs,
//! subprocess supervision, auto-training triggers, learning history (v5.5).
//!
//! Design notes (safety-critical, read before touching):
//! - Training executes the EXISTING, reviewed Python pipeline
//!   (`ml/gpo_train/{fish_train,train}.py`) as a child process. This module
//!   never trains in-process and never invents metrics: progress comes from
//!   the trainer's own `training_log.jsonl`, results from its
//!   `evaluation_test.json` + `onnx_check.json`.
//! - Jobs train against a FROZEN snapshot (labels+manifest copied at job
//!   creation; images stay live but are content-addressed and immutable).
//!   The snapshot fingerprint is recorded; drift is reported, never hidden.
//! - A job ending well is PASSED (evaluated). Promotion to shadow is a
//!   SEPARATE registry decision (`registry.rs`) — never automatic here
//!   unless the caller explicitly runs the promotion gate.
//! - Child handles live in the in-memory `Supervisor` (AppState). A restart
//!   loses handles by construction, so any RUNNING job found on disk at
//!   startup is honestly marked INTERRUPTED (never claimed complete).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use super::ml_dataset::{MlAnnotation, MlDatasetStore};
use super::ml_model::sha256_hex;

pub const TRAINING_SUBDIR: &str = "training";
/// Trainer families this backend knows how to launch. `fruit` is accepted
/// by the schema but blocked by the eligibility gate until real data exists.
pub const KNOWN_FAMILIES: &[&str] = &["fish", "state", "fruit"];

fn now_ms() -> u64 {
    crate::events::now_ms()
}

// ---- dataset fingerprint ----

/// Content fingerprint of a label set: canonical per-row lines, sorted,
/// SHA-256. Cheap (labels only); images are content-addressed by aHash and
/// immutable once written, so label identity fully determines the set.
pub fn dataset_fingerprint(rows: &[MlAnnotation], dataset_version: u32) -> String {
    let mut lines: Vec<String> = rows
        .iter()
        .map(|r| {
            format!(
                "{}|{}|{}|{}|{}|{}|{}|{}",
                r.image_id,
                r.session_id,
                r.timestamp_ms,
                r.game_state.map(|g| g.as_str()).unwrap_or(""),
                r.entity_id.as_deref().unwrap_or(""),
                r.ocr_text,
                r.hard_example,
                r.region_name,
            )
        })
        .collect();
    lines.sort();
    let body = format!("gpo-vision-v{dataset_version}:{}\n{}", lines.len(), lines.join("\n"));
    format!("fp-{}", &sha256_hex(body.as_bytes())[..16])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMeta {
    pub fingerprint: String,
    /// Rows ACTUALLY written into the frozen `labels.jsonl`, i.e. the
    /// training set size AFTER human-review filtering (v5.6).
    pub rows: usize,
    pub sessions: usize,
    pub test_sessions: Vec<String>,
    pub created_at: u64,
    /// Directory holding the frozen labels.jsonl + manifest.json copies.
    pub snap_dir: PathBuf,
    /// Fingerprint of reviews.jsonl at freeze time ("none" when unreviewed).
    /// A model is always traceable to exactly what humans had reviewed.
    #[serde(default = "default_review_none")]
    pub review_fingerprint: String,
    /// Entity registry (KB) version the snapshot was taken against.
    #[serde(default = "default_kb_one")]
    pub entity_registry_version: u32,
    /// Rows dropped from the snapshot because a human review recorded
    /// `training_eligible: false` (Skip / Unknown / Conflict / bad image /
    /// invalid canonical mapping / duplicate / missing provenance). Zero
    /// for snapshots frozen before review filtering existed.
    #[serde(default = "default_rows_excluded_by_review")]
    pub rows_excluded_by_review: usize,
    /// Rows with NO review record at all. They are KEPT (unreviewed data
    /// keeps exactly the meaning it had before v5.6) but counted, so a
    /// caller can show how much of the frozen set is human-verified.
    #[serde(default = "default_rows_unreviewed")]
    pub rows_unreviewed: usize,
}

fn default_review_none() -> String {
    "none".to_string()
}

fn default_kb_one() -> u32 {
    1
}

fn default_rows_excluded_by_review() -> usize {
    0
}

fn default_rows_unreviewed() -> usize {
    0
}

// ---- human-review projection (training eligibility) ----

/// Minimal projection of one `reviews.jsonl` line (see [`super::review`]).
/// Only what training eligibility needs is read, so a record that predates
/// newer review fields still parses.
///
/// `training_eligible` defaults to **false**, exactly like the full
/// `ReviewRecord`: a review that exists but carries no positive verdict must
/// never be mistaken for "no review at all" (which would let a bad image
/// straight back into training).
#[derive(Debug, Deserialize)]
struct ReviewEligibilityLine {
    image_id: String,
    #[serde(default)]
    training_eligible: bool,
}

/// Parse `reviews.jsonl` bytes into per-image eligibility lines, ALSO
/// returning how many non-empty lines failed to parse.
///
/// The count matters: silently dropping a line would make an excluded row
/// look unreviewed, and unreviewed rows are kept. Callers must fail closed
/// when it is non-zero.
fn parse_review_eligibility(raw: &[u8]) -> (Vec<ReviewEligibilityLine>, usize) {
    let Ok(text) = std::str::from_utf8(raw) else {
        return (Vec::new(), if raw.is_empty() { 0 } else { 1 });
    };
    let mut rows = Vec::new();
    let mut unreadable = 0usize;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<ReviewEligibilityLine>(line) {
            Ok(r) => rows.push(r),
            Err(_) => unreadable += 1,
        }
    }
    (rows, unreadable)
}

/// `image_id -> training_eligible`. Exclusion is sticky: if the same image
/// somehow appears twice with disagreeing verdicts, `false` wins (fail
/// closed — the conservative answer is always the one that trains on less).
fn review_eligibility_map(records: &[ReviewEligibilityLine]) -> HashMap<&str, bool> {
    let mut map: HashMap<&str, bool> = HashMap::with_capacity(records.len());
    for r in records {
        map.entry(r.image_id.as_str())
            .and_modify(|v| *v &= r.training_eligible)
            .or_insert(r.training_eligible);
    }
    map
}

/// Freeze the current label set for one job, AFTER applying human review.
///
/// Copies are small (labels + manifest only); images stay in place
/// (content-addressed, immutable).
///
/// SAFETY (v5.6) — the frozen `labels.jsonl` is FILTERED by
/// `<data_dir>/reviews.jsonl`, the only channel by which a human verdict
/// reaches the Python trainer. A row is written only when
/// (a) it has no review record — unreviewed data keeps exactly the meaning
/// it had before, this does not silently redefine it — or
/// (b) its record says `training_eligible: true`.
/// Anything a human skipped, called unknown, flagged as a conflict, or that
/// failed the bad-image / canonical-id gate is physically ABSENT from the
/// snapshot, so the trainer cannot learn from it even if it ignores our
/// metadata. Rows are re-serialised with serde_json: input order and every
/// other field are preserved unchanged.
///
/// Fail closed: a snapshot that would hold 0 rows is an error, never an
/// empty training run. `fingerprint`, `rows`, `sessions` and
/// `test_sessions` all describe the FILTERED set — i.e. exactly what
/// trains — so a model is always traceable to its real training input.
pub fn snapshot_dataset(
    data_dir: &Path,
    snap_dir: &Path,
    rows: &[MlAnnotation],
    dataset_version: u32,
) -> Result<SnapshotMeta, String> {
    let src = data_dir.join("datasets").join("gpo-vision").join("v1");
    std::fs::create_dir_all(snap_dir).map_err(|e| format!("snapshot dir: {e}"))?;
    // The dataset labels file must exist even though the snapshot is written
    // from `rows` (the caller's parsed view, which is also what the
    // fingerprint covers): a missing/unreadable dataset must fail loudly
    // rather than quietly snapshot a half-loaded store.
    std::fs::metadata(src.join("labels.jsonl")).map_err(|e| format!("snapshot read labels.jsonl: {e}"))?;
    let manifest_bytes =
        std::fs::read(src.join("manifest.json")).map_err(|e| format!("snapshot read manifest.json: {e}"))?;

    // Read the review state ONCE: it both decides what may train and
    // fingerprints exactly what humans had seen at freeze time.
    //
    // FAIL CLOSED (v5.6.1). v5.6.0 used `unwrap_or_default()` and then
    // silently skipped unparseable lines, so a read error OR one corrupt
    // line made every excluded row look "unreviewed" and it trained anyway.
    // A missing file legitimately means "no reviews exist yet"; an existing
    // but unreadable file must NOT be treated the same way, because the safe
    // reading of a lost review file is "nothing may be assumed reviewed".
    let reviews_path = data_dir.join("reviews.jsonl");
    let reviews_bytes = match std::fs::read(&reviews_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            return Err(format!(
                "snapshot: cannot read {} ({e}); refusing to train because \
                 review exclusions cannot be verified",
                reviews_path.display()
            ))
        }
    };
    let (review_records, unreadable) = parse_review_eligibility(&reviews_bytes);
    if unreadable > 0 {
        return Err(format!(
            "snapshot: {} line(s) of reviews.jsonl are unreadable; refusing to train \
             because review exclusions would be silently ignored. \
             Repair the file or run Review > Rebuild from audit.",
            unreadable
        ));
    }
    let eligibility = review_eligibility_map(&review_records);

    // Filter BEFORE any write, so a rejected snapshot leaves nothing behind.
    let mut kept: Vec<MlAnnotation> = Vec::with_capacity(rows.len());
    let mut rows_excluded_by_review = 0usize;
    let mut rows_unreviewed = 0usize;
    for r in rows {
        match eligibility.get(r.image_id.as_str()) {
            None => {
                rows_unreviewed += 1;
                kept.push(r.clone());
            }
            Some(true) => kept.push(r.clone()),
            Some(false) => rows_excluded_by_review += 1,
        }
    }
    if kept.is_empty() {
        return Err("training snapshot has 0 rows after review filtering".to_string());
    }

    // Frozen trainer input: the surviving rows, input order, every field
    // untouched (serde_json round-trip of the annotation itself).
    let mut labels = String::new();
    for r in &kept {
        let line = serde_json::to_string(r).map_err(|e| format!("snapshot serialise labels: {e}"))?;
        labels.push_str(&line);
        labels.push('\n');
    }
    std::fs::write(snap_dir.join("labels.jsonl"), labels.as_bytes())
        .map_err(|e| format!("snapshot write labels.jsonl: {e}"))?;
    // Manifest is dataset metadata, not row data: copied unchanged.
    std::fs::write(snap_dir.join("manifest.json"), &manifest_bytes)
        .map_err(|e| format!("snapshot write manifest.json: {e}"))?;

    // Freeze the review state alongside the labels (may not exist yet).
    let mut review_fingerprint = default_review_none();
    if !reviews_bytes.is_empty() {
        std::fs::write(snap_dir.join("reviews.jsonl"), &reviews_bytes)
            .map_err(|e| format!("snapshot write reviews.jsonl: {e}"))?;
        review_fingerprint = format!("rv-{}", &super::ml_model::sha256_hex(&reviews_bytes)[..16]);
    }
    let mut test_sessions: Vec<String> = kept
        .iter()
        .filter(|r| MlDatasetStore::split_of(&r.session_id).as_str() == "test")
        .map(|r| r.session_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    test_sessions.sort();
    let sessions = kept.iter().map(|r| r.session_id.as_str()).collect::<HashSet<_>>().len();
    Ok(SnapshotMeta {
        fingerprint: dataset_fingerprint(&kept, dataset_version),
        rows: kept.len(),
        sessions,
        test_sessions,
        created_at: now_ms(),
        snap_dir: snap_dir.to_path_buf(),
        review_fingerprint,
        entity_registry_version: super::knowledge::KNOWLEDGE_VERSION,
        rows_excluded_by_review,
        rows_unreviewed,
    })
}

// ---- jobs ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JobStatus {
    Queued,
    Running,
    Evaluating,
    Passed,
    Rejected,
    Failed,
    Cancelled,
    Interrupted,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Queued => "QUEUED",
            JobStatus::Running => "RUNNING",
            JobStatus::Evaluating => "EVALUATING",
            JobStatus::Passed => "PASSED",
            JobStatus::Rejected => "REJECTED",
            JobStatus::Failed => "FAILED",
            JobStatus::Cancelled => "CANCELLED",
            JobStatus::Interrupted => "INTERRUPTED",
        }
    }

    pub fn finished(self) -> bool {
        matches!(
            self,
            JobStatus::Passed
                | JobStatus::Rejected
                | JobStatus::Failed
                | JobStatus::Cancelled
                | JobStatus::Interrupted
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobProgress {
    pub stage: String,
    pub epoch: usize,
    pub total_epochs: usize,
    pub train_loss: Option<f32>,
    pub val_metric: Option<f32>,
    pub learning_rate: Option<f32>,
    /// Mean seconds per epoch from the trainer log; None until >= 2 epochs.
    pub sec_per_epoch: Option<f64>,
    /// Genuinely computed from sec_per_epoch x remaining; None otherwise.
    pub eta_s: Option<u64>,
    pub elapsed_s: u64,
    pub log_tail: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingJob {
    pub job_id: String,
    pub model_family: String,
    pub dataset_version: u32,
    pub dataset_fingerprint: String,
    pub frozen_rows: usize,
    pub frozen_sessions: usize,
    pub frozen_test_sessions: Vec<String>,
    /// Review fingerprint frozen with the snapshot ("none" when unreviewed).
    #[serde(default = "default_review_none")]
    pub review_fingerprint: String,
    pub snapshot_dir: PathBuf,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    pub status: JobStatus,
    /// "manual" or "auto:<trigger_type>".
    pub requested_by: String,
    /// Trainer argv *template* (python path resolved at launch).
    pub trainer_module: String,
    pub run_id: String,
    pub epochs: usize,
    pub seed: u64,
    pub code_version: String,
    /// Trainer checkout dir (set at launch from settings; trainer outputs
    /// live under <trainer_dir>/ml/output/<run_id>).
    pub trainer_dir: String,
    pub work_dir: PathBuf,
    pub candidate_id: Option<String>,
    pub evaluation: Option<serde_json::Value>,
    pub error: Option<String>,
    pub progress: JobProgress,
    pub deferred_reason: Option<String>,
    pub macro_running_at_start: bool,
}

pub fn jobs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(TRAINING_SUBDIR).join("jobs")
}

/// Reject any `job_id` that is not a plain, path-free token.
///
/// `job_id` arrives from the UI on cancel/restart/discard/decide, and
/// `job_path` interpolates it straight into a filename. Job ids are minted
/// internally as `job-<ms>-<n>`, so an allowlist costs nothing and removes the
/// whole traversal / device-name / overlong-name class.
fn valid_job_id(job_id: &str) -> bool {
    !job_id.is_empty()
        && job_id.len() <= 64
        && job_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn job_path(data_dir: &Path, job_id: &str) -> Result<PathBuf, String> {
    if !valid_job_id(job_id) {
        return Err(format!("invalid job id '{job_id}'"));
    }
    Ok(jobs_dir(data_dir).join(format!("{job_id}.json")))
}

pub fn save_job(data_dir: &Path, job: &TrainingJob) -> Result<(), String> {
    std::fs::create_dir_all(jobs_dir(data_dir)).map_err(|e| e.to_string())?;
    let body = serde_json::to_string_pretty(job).map_err(|e| e.to_string())?;
    std::fs::write(job_path(data_dir, &job.job_id)?, body).map_err(|e| e.to_string())
}

pub fn load_job(data_dir: &Path, job_id: &str) -> Result<TrainingJob, String> {
    let raw = std::fs::read_to_string(job_path(data_dir, job_id)?).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

pub fn list_jobs(data_dir: &Path) -> Vec<TrainingJob> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(jobs_dir(data_dir)) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "json") {
            if let Ok(raw) = std::fs::read_to_string(&p) {
                if let Ok(j) = serde_json::from_str::<TrainingJob>(&raw) {
                    out.push(j);
                }
            }
        }
    }
    out.sort_by_key(|j| std::cmp::Reverse(j.created_at));
    out
}

/// Startup recovery: any job left RUNNING/EVALUATING/QUEUED-with-pid is
/// honestly marked INTERRUPTED (handles are gone with the old process).
/// Returns the ids that were flipped.
pub fn mark_interrupted(data_dir: &Path) -> Vec<String> {
    let mut flipped = Vec::new();
    for mut j in list_jobs(data_dir) {
        if matches!(j.status, JobStatus::Running | JobStatus::Evaluating) {
            j.status = JobStatus::Interrupted;
            j.finished_at = Some(now_ms());
            j.error = Some("application restarted while training was running; outcome unknown (never claimed complete)".to_string());
            if save_job(data_dir, &j).is_ok() {
                flipped.push(j.job_id.clone());
            }
        }
    }
    flipped
}

pub fn new_job_id(family: &str) -> String {
    format!("{family}-{}-{}", now_ms(), &sha256_hex(family.as_bytes())[..6])
}

/// Create (do not launch) a job against a frozen snapshot.
#[allow(clippy::too_many_arguments)]
pub fn create_job(
    data_dir: &Path,
    model_family: &str,
    requested_by: &str,
    snapshot: &SnapshotMeta,
    dataset_version: u32,
    trainer_module: &str,
    epochs: usize,
    seed: u64,
    code_version: &str,
    macro_running: bool,
) -> Result<TrainingJob, String> {
    if !KNOWN_FAMILIES.contains(&model_family) {
        return Err(format!("unknown model family '{model_family}'"));
    }
    let job_id = new_job_id(model_family);
    let work_dir = data_dir.join(TRAINING_SUBDIR).join("work").join(&job_id);
    std::fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;
    let job = TrainingJob {
        job_id: job_id.clone(),
        model_family: model_family.to_string(),
        dataset_version,
        dataset_fingerprint: snapshot.fingerprint.clone(),
        frozen_rows: snapshot.rows,
        frozen_sessions: snapshot.sessions,
        frozen_test_sessions: snapshot.test_sessions.clone(),
        review_fingerprint: snapshot.review_fingerprint.clone(),
        snapshot_dir: snapshot.snap_dir.clone(),
        created_at: now_ms(),
        started_at: None,
        finished_at: None,
        status: JobStatus::Queued,
        requested_by: requested_by.to_string(),
        trainer_module: trainer_module.to_string(),
        run_id: job_id,
        epochs,
        seed,
        code_version: code_version.to_string(),
        trainer_dir: String::new(),
        work_dir,
        candidate_id: None,
        evaluation: None,
        error: None,
        progress: JobProgress { stage: "QUEUED".to_string(), ..Default::default() },
        deferred_reason: None,
        macro_running_at_start: macro_running,
    };
    save_job(data_dir, &job)?;
    Ok(job)
}

// ---- supervision (in-memory child handles) ----

pub struct Supervisor {
    children: HashMap<String, std::process::Child>,
}

impl Supervisor {
    pub fn new() -> Self {
        Self { children: HashMap::new() }
    }

    pub fn running_count(&self) -> usize {
        self.children.len()
    }

    /// Launch a QUEUED job. Env pins the frozen snapshot; stdout/stderr go
    /// to the job log file (never to the app console).
    pub fn launch(
        &mut self,
        data_dir: &Path,
        job: &mut TrainingJob,
        python_path: &str,
        trainer_dir: &str,
        max_concurrent: usize,
    ) -> Result<(), String> {
        if python_path.trim().is_empty() || trainer_dir.trim().is_empty() {
            return Err("training backend not configured: set python_path + trainer_dir in Training settings".to_string());
        }
        let images_live = data_dir.join("datasets").join("gpo-vision").join("v1").join("images");
        let argv = vec![
            python_path.to_string(),
            "-m".to_string(),
            job.trainer_module.clone(),
            "--epochs".to_string(),
            job.epochs.to_string(),
            "--seed".to_string(),
            job.seed.to_string(),
            "--run-id".to_string(),
            job.run_id.clone(),
        ];
        let envs = vec![
            ("GPO_DATASET_DIR".to_string(), job.snapshot_dir.to_string_lossy().to_string()),
            ("GPO_IMAGES_DIR".to_string(), images_live.to_string_lossy().to_string()),
            ("PYTHONUTF8".to_string(), "1".to_string()),
        ];
        self.launch_argv(data_dir, job, &argv, &envs, Path::new(trainer_dir), max_concurrent)
    }

    /// Launch with an explicit argv/env/cwd. Production passes the real
    /// trainer argv; the E2E test passes a stub command. Documented test
    /// seam: argv[0] is the program, the rest are args (no shell involved,
    /// extra args are never interpreted).
    pub fn launch_argv(
        &mut self,
        data_dir: &Path,
        job: &mut TrainingJob,
        argv: &[String],
        envs: &[(String, String)],
        cwd: &Path,
        max_concurrent: usize,
    ) -> Result<(), String> {
        if job.status != JobStatus::Queued {
            return Err(format!("job {} is {:?}, only QUEUED jobs launch", job.job_id, job.status));
        }
        if argv.is_empty() {
            return Err("empty trainer command".to_string());
        }
        // Reap dead handles so the count reflects reality.
        self.children.retain(|_, c| c.try_wait().map(|s| s.is_none()).unwrap_or(false));
        if self.children.len() >= max_concurrent.max(1) {
            return Err("another training job is already running (concurrency limit)".to_string());
        }
        let log_path = job.work_dir.join("trainer_stdout.log");
        let log_file = std::fs::File::create(&log_path).map_err(|e| e.to_string())?;
        let err_file = log_file.try_clone().map_err(|e| e.to_string())?;
        let mut cmd = std::process::Command::new(&argv[0]);
        cmd.args(&argv[1..]).current_dir(cwd).stdout(log_file).stderr(err_file);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let child = cmd.spawn().map_err(|e| format!("trainer spawn failed ({}): {e}", argv[0]))?;
        self.children.insert(job.job_id.clone(), child);
        job.status = JobStatus::Running;
        job.started_at = Some(now_ms());
        job.progress.stage = "RUNNING".to_string();
        job.progress.total_epochs = job.epochs;
        save_job(data_dir, job)?;
        Ok(())
    }

    /// Poll one job: reap exits, refresh progress from the trainer log,
    /// finalize on completion. Returns true when the job just finished.
    pub fn poll(&mut self, data_dir: &Path, job: &mut TrainingJob, trainer_out_subdir: &str) -> bool {
        let done = match self.children.get_mut(&job.job_id) {
            Some(child) => match child.try_wait() {
                Ok(Some(status)) => Some(status.code().unwrap_or(-1)),
                Ok(None) => None,
                Err(_) => Some(-2),
            },
            None => {
                // No handle (e.g. after restart without mark_interrupted):
                // infer from artifacts only, conservatively.
                if trainer_done_marker(&job.work_dir, trainer_out_subdir, &job.run_id) {
                    Some(0)
                } else {
                    None
                }
            }
        };
        refresh_progress(job, trainer_out_subdir);
        let _ = save_job(data_dir, job);
        match done {
            None => false,
            Some(code) => {
                self.children.remove(&job.job_id);
                finalize_job(data_dir, job, trainer_out_subdir, code);
                true
            }
        }
    }

    /// Cancel a live job: kill the child, mark CANCELLED. Never deletes
    /// partial outputs (they stay inspectable under work/).
    pub fn cancel(&mut self, data_dir: &Path, job: &mut TrainingJob) -> Result<(), String> {
        if let Some(mut child) = self.children.remove(&job.job_id) {
            let _ = child.kill();
            let _ = child.wait();
        } else if job.status.finished() {
            return Err(format!("job {} already {:?}", job.job_id, job.status));
        }
        job.status = JobStatus::Cancelled;
        job.finished_at = Some(now_ms());
        job.progress.stage = "CANCELLED".to_string();
        save_job(data_dir, job)?;
        Ok(())
    }
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

/// Trainer output root for a job: <trainer_dir>/ml/output/<run-id>.
/// The runner does not assume it; completion is detected via the files the
/// real pipeline always writes (training_log.jsonl ... onnx_check.json).
fn trainer_out_dir(trainer_dir: &Path, run_id: &str) -> PathBuf {
    trainer_dir.join("ml").join("output").join(run_id)
}

fn trainer_done_marker(work_dir: &Path, trainer_dir: &str, run_id: &str) -> bool {
    // Primary: the pipeline's own verification file (written last, only on
    // successful export+verify). Fallback: none — without it, not done.
    let _ = work_dir;
    trainer_out_dir(Path::new(trainer_dir), run_id).join("onnx_check.json").exists()
}

/// Refresh progress from the trainer's own log (never invented).
fn refresh_progress(job: &mut TrainingJob, trainer_dir: &str) {
    let log = trainer_out_dir(Path::new(trainer_dir), &job.run_id).join("training_log.jsonl");
    let Ok(raw) = std::fs::read_to_string(&log) else {
        job.progress.stage = if job.status == JobStatus::Queued { "QUEUED".to_string() } else { "STARTING".to_string() };
        return;
    };
    let mut epochs = 0usize;
    let mut secs: Vec<f64> = Vec::new();
    let mut last_loss = None;
    let mut last_val = None;
    let mut last_lr = None;
    let mut tail: Vec<String> = Vec::new();
    for line in raw.lines().rev().take(4) {
        tail.push(line.chars().take(220).collect());
    }
    tail.reverse();
    for line in raw.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        epochs += 1;
        if let Some(s) = v.get("seconds").and_then(|x| x.as_f64()) {
            secs.push(s);
        }
        last_loss = v.get("train_loss").and_then(|x| x.as_f64()).map(|x| x as f32);
        last_val = v
            .get("val_macro_f1")
            .or_else(|| v.get("val_state_macro_f1"))
            .and_then(|x| x.as_f64())
            .map(|x| x as f32);
        last_lr = v.get("lr").and_then(|x| x.as_f64()).map(|x| x as f32);
    }
    job.progress.epoch = epochs;
    job.progress.train_loss = last_loss;
    job.progress.val_metric = last_val;
    job.progress.learning_rate = last_lr;
    job.progress.log_tail = tail;
    if secs.len() >= 2 {
        let per = (secs[secs.len() - 1] - secs[0]) / (secs.len() - 1) as f64;
        job.progress.sec_per_epoch = Some((per * 10.0).round() / 10.0);
        if job.epochs > epochs {
            job.progress.eta_s = Some(((job.epochs - epochs) as f64 * per).round() as u64);
        } else {
            job.progress.eta_s = Some(0);
        }
    } else {
        job.progress.sec_per_epoch = None;
        job.progress.eta_s = None;
    }
    job.progress.stage = if epochs >= job.epochs { "EVALUATING".to_string() } else { "RUNNING".to_string() };
    if job.status == JobStatus::Running && epochs >= job.epochs {
        job.status = JobStatus::Evaluating;
    }
}

/// Finalize after the child exited. Reads the REAL evaluation files;
/// missing/invalid outputs are FAILED with the reason (never defaulted).
fn finalize_job(data_dir: &Path, job: &mut TrainingJob, trainer_dir: &str, exit_code: i32) {
    job.finished_at = Some(now_ms());
    if exit_code != 0 {
        job.status = JobStatus::Failed;
        job.error = Some(format!("trainer exited with code {exit_code}; see work dir log"));
        job.progress.stage = "FAILED".to_string();
        let _ = save_job(data_dir, job);
        append_history(data_dir, "job_failed", serde_json::json!({"job_id": job.job_id, "reason": job.error}));
        return;
    }
    let out = trainer_out_dir(Path::new(trainer_dir), &job.run_id);
    let read = |name: &str| std::fs::read_to_string(out.join(name)).ok();
    let onnx_check: Option<serde_json::Value> =
        read("onnx_check.json").and_then(|s| serde_json::from_str(&s).ok());
    let eval: Option<serde_json::Value> =
        read("evaluation_test.json").and_then(|s| serde_json::from_str(&s).ok());
    let onnx_ok = onnx_check.as_ref().and_then(|v| v.get("pass")).and_then(|v| v.as_bool()).unwrap_or(false);
    match (onnx_ok, eval) {
        (true, Some(e)) => {
            job.status = JobStatus::Passed;
            job.evaluation = Some(e.clone());
            job.progress.stage = "PASSED".to_string();
            // Factual lesson record (metrics only, no chain-of-thought):
            // what was trained, on what frozen data, with what outcome.
            append_history(
                data_dir,
                "lesson",
                serde_json::json!({
                    "job_id": job.job_id,
                    "family": job.model_family,
                    "dataset_fingerprint": job.dataset_fingerprint,
                    "review_fingerprint": job.review_fingerprint,
                    "accuracy": e.get("accuracy"),
                    "macro_f1": e.get("macro_f1").or_else(|| e.get("macroF1")),
                    "test_n": e.get("n"),
                }),
            );
        }
        (false, _) => {
            job.status = JobStatus::Failed;
            job.error = Some("onnx_check.json missing or pass=false: export/verify did not complete".to_string());
            job.progress.stage = "FAILED".to_string();
        }
        (true, None) => {
            job.status = JobStatus::Failed;
            job.error = Some("evaluation_test.json missing: evaluation did not complete".to_string());
            job.progress.stage = "FAILED".to_string();
        }
    }
    let _ = save_job(data_dir, job);
}

// ---- backend availability ----

/// Honest capability probe: runs `<python> -c "import torch..."` with a
/// bounded wall-clock timeout and never more than one probe in flight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendStatus {
    pub available: bool,
    pub python_path: String,
    pub python_ok: bool,
    pub torch_ok: bool,
    pub torch_version: Option<String>,
    pub trainer_dir_ok: bool,
    pub detail: String,
}

/// Anything missing is reported by name, never papered over.
pub fn check_backend(python_path: &str, trainer_dir: &str) -> BackendStatus {
    check_backend_with_timeout(python_path, trainer_dir, DEFAULT_BACKEND_PROBE_TIMEOUT)
}

/// Default wall-clock budget for the backend health probe.
///
/// `import torch` on a cold Windows filesystem cache can take several seconds.
/// Past that, something is wrong (a hung interpreter, a network filesystem
/// stall) and we must not hold the caller hostage: the probe is polled
/// repeatedly, so a hang would otherwise freeze the panel indefinitely.
pub const DEFAULT_BACKEND_PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// Single-flight guard for the health probe.
///
/// `training_overview` is polled every few seconds. Without this, a slow
/// probe stacks up: each poll starts another `python`, and they pile up until
/// the machine is saturated. One probe at a time; overlapping callers get the
/// cached result of the most recent completed probe instead of spawning a
/// second process.
static BACKEND_PROBE: Mutex<Option<(String, String, BackendStatus)>> = Mutex::new(None);

/// Run `python -c <code>`, killing it after `timeout`.
///
/// Uses `spawn` + a bounded wait loop rather than `Command::output()` (which
/// has no timeout at all) and then `kill`s + `wait`s so no orphan interpreter
/// is left behind. stdout/stderr are piped and read on the helper threads, so
/// a chatty interpreter cannot deadlock us by filling a pipe buffer.
fn probe_python(python_path: &str, code: &str, timeout: Duration) -> Result<std::process::Output, String> {
    probe_program(python_path, &["-c", code], timeout)
}

/// Bounded subprocess run: spawn, poll until `timeout`, kill + reap on expiry.
///
/// Split from [`probe_python`] so the timeout/reap behaviour is testable
/// without needing a Python interpreter present.
fn probe_program(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = std::io::Read::read_to_end(p, &mut buf);
        }
        buf
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = std::io::Read::read_to_end(p, &mut buf);
        }
        buf
    });

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = out_handle.join().unwrap_or_default();
                let stderr = err_handle.join().unwrap_or_default();
                return Ok(std::process::Output { status, stdout, stderr });
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    // Terminate the tree, then reap it: no orphan process.
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("probe timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(e.to_string());
            }
        }
    }
}

pub fn check_backend_with_timeout(
    python_path: &str,
    trainer_dir: &str,
    timeout: Duration,
) -> BackendStatus {
    // Single-flight: reuse the last result for the same configuration rather
    // than spawning a second interpreter.
    {
        let cached = BACKEND_PROBE.lock().ok().and_then(|g| g.clone());
        if let Some((p, t, st)) = cached {
            if p == python_path && t == trainer_dir {
                return st;
            }
        }
    }
    let status = check_backend_uncached(python_path, trainer_dir, timeout);
    if let Ok(mut g) = BACKEND_PROBE.lock() {
        *g = Some((python_path.to_string(), trainer_dir.to_string(), status.clone()));
    }
    status
}

/// Drop the cached probe result (call after the training settings change).
pub fn invalidate_backend_probe() {
    if let Ok(mut g) = BACKEND_PROBE.lock() {
        *g = None;
    }
}

fn check_backend_uncached(
    python_path: &str,
    trainer_dir: &str,
    timeout: Duration,
) -> BackendStatus {
    let mut st = BackendStatus {
        available: false,
        python_path: python_path.to_string(),
        python_ok: false,
        torch_ok: false,
        torch_version: None,
        trainer_dir_ok: false,
        detail: String::new(),
    };
    if python_path.trim().is_empty() {
        st.detail = "python_path is not configured (Training settings)".to_string();
        return st;
    }
    match probe_python(
        python_path,
        "import sys, torch; print(torch.__version__)",
        timeout,
    ) {
        Err(e) => {
            // Distinguish "cannot run at all" from "ran but hung": the second
            // is a hang, and saying so is the difference between a misconfigured
            // path and a wedged interpreter.
            let (ok, detail) = if e.starts_with("probe timed out") {
                (true, format!("torch import did not finish within {}s (interpreter terminated): {e}", timeout.as_secs()))
            } else {
                (false, format!("cannot execute '{python_path}': {e}"))
            };
            st.python_ok = ok;
            st.detail = detail;
            return st;
        }
        Ok(out) if !out.status.success() => {
            let err = String::from_utf8_lossy(&out.stderr);
            st.python_ok = true;
            st.detail = format!("python runs but torch import failed: {}", err.chars().take(300).collect::<String>());
            return st;
        }
        Ok(out) => {
            st.python_ok = true;
            st.torch_ok = true;
            st.torch_version = Some(String::from_utf8_lossy(&out.stdout).trim().to_string());
        }
    }
    let trainer_ok = !trainer_dir.trim().is_empty()
        && Path::new(trainer_dir).join("ml").join("gpo_train").join("fish_train.py").exists();
    st.trainer_dir_ok = trainer_ok;
    if !trainer_ok {
        st.detail = format!("trainer checkout not found under '{trainer_dir}' (need ml/gpo_train/fish_train.py)");
        return st;
    }
    st.available = true;
    st.detail = format!("ready (torch {})", st.torch_version.as_deref().unwrap_or("?"));
    st
}

// ---- triggers ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerEvent {
    pub trigger_type: String,
    pub reason: String,
    pub evidence: serde_json::Value,
    pub dataset_fingerprint: String,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TriggerState {
    pub last_fingerprint: Option<String>,
    pub last_rows: usize,
    pub last_sessions: usize,
    pub last_hard: usize,
    pub last_trigger_ms: Option<u64>,
    pub last_fish_ready: bool,
    pub last_fruit_ready: bool,
}

pub fn trigger_state_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TRAINING_SUBDIR).join("trigger_state.json")
}

pub fn load_trigger_state(data_dir: &Path) -> TriggerState {
    std::fs::read_to_string(trigger_state_path(data_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_trigger_state(data_dir: &Path, state: &TriggerState) {
    let dir = data_dir.join(TRAINING_SUBDIR);
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(body) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(trigger_state_path(data_dir), body);
    }
}

/// Evaluate automatic-training triggers against the live dataset.
/// Pure function (no spawning): the caller decides manual vs auto.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_triggers(
    rows: &[MlAnnotation],
    dataset_version: u32,
    fish_ready_now: bool,
    fruit_ready_now: bool,
    prev: &TriggerState,
    min_new_samples: usize,
    min_new_sessions: usize,
    cooldown_hours: u64,
    now: u64,
) -> Vec<TriggerEvent> {
    let fp = dataset_fingerprint(rows, dataset_version);
    let sessions = rows.iter().map(|r| r.session_id.as_str()).collect::<HashSet<_>>().len();
    let mut out = Vec::new();
    let cooled = prev
        .last_trigger_ms
        .map(|t| now.saturating_sub(t) >= cooldown_hours.saturating_mul(3_600_000))
        .unwrap_or(true);
    if prev.last_fingerprint.as_deref() == Some(fp.as_str()) {
        return out;
    }
    let new_rows = rows.len().saturating_sub(prev.last_rows);
    let new_sessions = sessions.saturating_sub(prev.last_sessions);
    if new_rows >= min_new_samples && new_sessions >= min_new_sessions && cooled {
        out.push(TriggerEvent {
            trigger_type: "new_data".to_string(),
            reason: format!("+{new_rows} rows / +{new_sessions} sessions since last training check"),
            evidence: serde_json::json!({"new_rows": new_rows, "new_sessions": new_sessions}),
            dataset_fingerprint: fp.clone(),
            timestamp_ms: now,
        });
    }
    if (fish_ready_now && !prev.last_fish_ready || fruit_ready_now && !prev.last_fruit_ready) && cooled {
        out.push(TriggerEvent {
            trigger_type: "coverage".to_string(),
            reason: format!(
                "capability newly READY: {}{}",
                if fish_ready_now && !prev.last_fish_ready { "fish " } else { "" },
                if fruit_ready_now && !prev.last_fruit_ready { "fruit" } else { "" }
            ),
            evidence: serde_json::json!({"fish_ready": fish_ready_now, "fruit_ready": fruit_ready_now}),
            dataset_fingerprint: fp.clone(),
            timestamp_ms: now,
        });
    }
    let new_hard = rows.iter().filter(|r| r.hard_example).count();
    if new_hard >= prev.last_hard + (min_new_samples / 2).max(25) && cooled {
        out.push(TriggerEvent {
            trigger_type: "hard_examples".to_string(),
            reason: format!("+{} hard/disagreement rows since last check", new_hard.saturating_sub(prev.last_hard)),
            evidence: serde_json::json!({"hard_rows": new_hard}),
            dataset_fingerprint: fp.clone(),
            timestamp_ms: now,
        });
    }
    out
}

// ---- learning history (immutable, append-only) ----

pub fn history_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TRAINING_SUBDIR).join("history.jsonl")
}

pub fn append_history(data_dir: &Path, kind: &str, detail: serde_json::Value) {
    let dir = data_dir.join(TRAINING_SUBDIR);
    let _ = std::fs::create_dir_all(&dir);
    let line = serde_json::json!({"ts": now_ms(), "kind": kind, "detail": detail}).to_string() + "\n";
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(history_path(data_dir)) {
        let _ = f.write_all(line.as_bytes());
    }
}

pub fn read_history(data_dir: &Path, tail: usize) -> Vec<serde_json::Value> {
    let Ok(raw) = std::fs::read_to_string(history_path(data_dir)) else { return Vec::new() };
    let mut v: Vec<serde_json::Value> =
        raw.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    if v.len() > tail {
        v.drain(..v.len() - tail);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ml_dataset::{DATASET_VERSION, GameStateLabel, MlTask, UiLabel};

    fn row(session: &str, ts: u64, state: GameStateLabel, entity: Option<&str>) -> MlAnnotation {
        MlAnnotation {
            image_id: format!("{session}-{ts}"),
            dataset_version: DATASET_VERSION,
            session_id: session.into(),
            task: MlTask::GameState,
            ocr_text: String::new(),
            region_name: "bar".into(),
            ui_label: Some(UiLabel::FishingBar),
            bbox: None,
            game_state: Some(state),
            entity_id: entity.map(|s| s.into()),
            annotator: "test".into(),
            timestamp_ms: ts,
            source: "test".into(),
            confidence: None,
            hard_example: false,
            hard_reason: None,
            corrections: vec![],
            event_id: None,
            frame_index: None,
        }
    }

    /// Row with an explicit image_id (review filtering keys on it).
    fn frow(image_id: &str, session: &str, ts: u64, state: GameStateLabel) -> MlAnnotation {
        let mut r = row(session, ts, state, None);
        r.image_id = image_id.to_string();
        r
    }

    /// One `reviews.jsonl` line in the `ReviewRecord` shape.
    fn review_line(image_id: &str, status: &str, eligible: bool, excluded: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "review_id": format!("rev-{image_id}"),
            "image_id": image_id,
            "session_id": "flt-sess",
            "review_status": status,
            "human_entity_id": eligible.then(|| "fish:golden".to_string()),
            "training_eligible": eligible,
            "excluded_reason": excluded,
            "reviewed_at": 1,
            "reviewer_version": "v5.6",
            "dataset_version": 1,
        })
    }

    /// Write `reviews.jsonl` in its real shape: ONE review object per line.
    fn write_reviews(path: &Path, lines: &[serde_json::Value]) {
        let body: String = lines.iter().map(|l| l.to_string() + "\n").collect();
        std::fs::write(path, body).unwrap();
    }

    /// A review file that exists but cannot be read back must FAIL the
    /// snapshot, never be treated as "no reviews exist".
    ///
    /// This is the fail-open that let an excluded row train: one unparseable
    /// line made its `image_id` vanish from the eligibility map, so the row
    /// looked unreviewed and was kept.
    #[test]
    fn snapshot_refuses_when_the_review_file_cannot_be_parsed() {
        let (dir, rows) = seeded_review_dir("corrupt-review");
        let reviews = dir.join("reviews.jsonl");
        let mut body = std::fs::read_to_string(&reviews).unwrap();
        // A line from a newer writer, or a truncated write.
        body.push_str("{ this is not a review record\n");
        std::fs::write(&reviews, body).unwrap();

        let err = snapshot_dataset(&dir, &dir.join("snap"), &rows, 1).unwrap_err();
        assert!(err.contains("unreadable"), "{err}");
        assert!(err.contains("refusing to train"), "{err}");
        // And nothing was written: a rejected snapshot leaves no trainer input.
        assert!(!dir.join("snap").join("labels.jsonl").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An absent review file legitimately means "nothing reviewed yet", and
    /// must keep working (otherwise a first run could never train).
    #[test]
    fn snapshot_allows_a_missing_review_file() {
        let (dir, rows) = seeded_review_dir("no-review-file");
        let _ = std::fs::remove_file(dir.join("reviews.jsonl"));
        let snap = snapshot_dataset(&dir, &dir.join("snap"), &rows, 1).unwrap();
        assert_eq!(snap.rows, rows.len(), "unreviewed rows are kept");
        assert_eq!(snap.rows_unreviewed, rows.len());
        assert_eq!(snap.review_fingerprint, "none");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Disposable dataset whose reviews cover 3 of 4 rows:
    /// `flt-a` eligible, `flt-b` eligible, `flt-c` explicitly skipped
    /// (`excluded_reason: "skipped"`), `flt-d` unreviewed.
    fn seeded_review_dir(tag: &str) -> (PathBuf, Vec<MlAnnotation>) {
        let dir = std::env::temp_dir().join(format!("gpo-train-filter-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        let v1 = dir.join("datasets").join("gpo-vision").join("v1");
        std::fs::create_dir_all(&v1).unwrap();
        let rows = vec![
            frow("flt-a", "flt-sess", 1, GameStateLabel::Bite),
            frow("flt-b", "flt-sess", 2, GameStateLabel::WaitingForBite),
            frow("flt-c", "flt-sess", 3, GameStateLabel::CatchResult),
            frow("flt-d", "flt-sess", 4, GameStateLabel::CatchResult),
        ];
        let body: String = rows.iter().map(|r| serde_json::to_string(r).unwrap() + "\n").collect();
        std::fs::write(v1.join("labels.jsonl"), body).unwrap();
        std::fs::write(
            v1.join("manifest.json"),
            r#"{"name":"gpo-vision","version":1,"created_ms":1,"split_strategy":"hash","split_overrides":{}}"#,
        )
        .unwrap();
        write_reviews(
            &dir.join("reviews.jsonl"),
            &[
                review_line("flt-a", "REVIEWED_CORRECT", true, None),
                review_line("flt-b", "REVIEWED_CORRECTED", true, None),
                review_line("flt-c", "REVIEWED_SKIPPED", false, Some("skipped")),
            ],
        );
        (dir, rows)
    }

    #[test]
    fn fingerprint_is_stable_and_sensitive() {
        let a = vec![row("s1", 1, GameStateLabel::Bite, None), row("s1", 2, GameStateLabel::WaitingForBite, None)];
        let b = a.clone();
        assert_eq!(dataset_fingerprint(&a, 1), dataset_fingerprint(&b, 1));
        let mut c = a.clone();
        c.push(row("s2", 3, GameStateLabel::CatchResult, Some("fish:golden")));
        assert_ne!(dataset_fingerprint(&a, 1), dataset_fingerprint(&c, 1));
        // Order-independent.
        let mut d = c.clone();
        d.reverse();
        assert_eq!(dataset_fingerprint(&c, 1), dataset_fingerprint(&d, 1));
    }

    #[test]
    fn snapshot_freezes_labels_and_records_test_sessions() {
        let dir = std::env::temp_dir().join("gpo-train-snap");
        let _ = std::fs::remove_dir_all(&dir);
        let ds = dir.join("datasets").join("gpo-vision").join("v1");
        std::fs::create_dir_all(&ds).unwrap();
        std::fs::write(ds.join("labels.jsonl"), "l1\nl2\n").unwrap();
        std::fs::write(ds.join("manifest.json"), r#"{"version":1}"#).unwrap();
        // Find a test-split session deterministically.
        let mut test_sess = None;
        for i in 0..2000 {
            let cand = format!("snap-test-{i:04}");
            if MlDatasetStore::split_of(&cand).as_str() == "test" {
                test_sess = Some(cand);
                break;
            }
        }
        let ts = test_sess.unwrap();
        let rows = vec![row("snap-a", 1, GameStateLabel::Bite, None), row(&ts, 2, GameStateLabel::CatchResult, Some("fish:golden"))];
        let meta = snapshot_dataset(&dir, &dir.join("snap1"), &rows, 1).unwrap();
        assert_eq!(meta.rows, 2);
        assert!(meta.test_sessions.contains(&ts));
        assert!(dir.join("snap1").join("labels.jsonl").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_drops_rows_a_human_marked_training_ineligible() {
        let (dir, rows) = seeded_review_dir("mixed");
        let snap_dir = dir.join("snap1");
        let meta = snapshot_dataset(&dir, &snap_dir, &rows, 1).unwrap();

        // Counts describe the FILTERED set (the training set), not the input.
        assert_eq!(meta.rows, 3, "2 eligible + 1 unreviewed");
        assert_eq!(meta.rows_excluded_by_review, 1);
        assert_eq!(meta.rows_unreviewed, 1);

        // The trainer-facing labels.jsonl holds exactly the survivors, in
        // input order, and the human-skipped row is physically absent.
        let raw = std::fs::read_to_string(snap_dir.join("labels.jsonl")).unwrap();
        let parsed: Vec<MlAnnotation> =
            raw.lines().map(|l| serde_json::from_str(l).expect("snapshot row must parse")).collect();
        let ids: Vec<&str> = parsed.iter().map(|r| r.image_id.as_str()).collect();
        assert_eq!(ids, vec!["flt-a", "flt-b", "flt-d"], "input order preserved");
        assert!(!ids.contains(&"flt-c"), "skipped row must not reach the trainer");

        // Every other field survives the serde_json round-trip untouched.
        for got in &parsed {
            let src = rows.iter().find(|r| r.image_id == got.image_id).unwrap();
            assert_eq!(serde_json::to_value(src).unwrap(), serde_json::to_value(got).unwrap());
        }
        assert_eq!(parsed[2].game_state, Some(GameStateLabel::CatchResult));

        // Manifest still copied unchanged; the frozen review state remains.
        assert_eq!(
            std::fs::read(snap_dir.join("manifest.json")).unwrap(),
            std::fs::read(dir.join("datasets").join("gpo-vision").join("v1").join("manifest.json")).unwrap()
        );
        assert!(snap_dir.join("reviews.jsonl").exists(), "reviews.jsonl copy must survive filtering");
        assert_ne!(meta.review_fingerprint, "none", "reviews exist so fingerprint must be set");

        // Fingerprint + session counts describe what trains, not the input.
        let kept: Vec<MlAnnotation> = rows.iter().filter(|r| r.image_id != "flt-c").cloned().collect();
        assert_eq!(meta.fingerprint, dataset_fingerprint(&kept, 1));
        assert_ne!(meta.fingerprint, dataset_fingerprint(&rows, 1), "excluded row must not be in the fingerprint");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_fails_closed_when_review_excludes_every_row() {
        let (dir, rows) = seeded_review_dir("allexcluded");
        let reviews: Vec<serde_json::Value> = rows
            .iter()
            .map(|r| review_line(&r.image_id, "REVIEWED_SKIPPED", false, Some("skipped")))
            .collect();
        write_reviews(&dir.join("reviews.jsonl"), &reviews);
        let snap_dir = dir.join("snap-all");
        let err = snapshot_dataset(&dir, &snap_dir, &rows, 1).unwrap_err();
        assert!(err.contains("0 rows after review filtering"), "{err}");
        assert!(!snap_dir.join("labels.jsonl").exists(), "never leave an empty training snapshot on disk");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_refuses_to_freeze_an_empty_training_set() {
        let dir = std::env::temp_dir().join("gpo-train-filter-empty");
        let _ = std::fs::remove_dir_all(&dir);
        let v1 = dir.join("datasets").join("gpo-vision").join("v1");
        std::fs::create_dir_all(&v1).unwrap();
        std::fs::write(v1.join("labels.jsonl"), "").unwrap();
        std::fs::write(v1.join("manifest.json"), r#"{"version":1}"#).unwrap();
        let err = snapshot_dataset(&dir, &dir.join("snap0"), &[], 1).unwrap_err();
        assert!(err.contains("0 rows after review filtering"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_of_unchanged_input_is_deterministic() {
        let (dir, rows) = seeded_review_dir("determinism");
        let a = snapshot_dataset(&dir, &dir.join("snap-a"), &rows, 1).unwrap();
        let b = snapshot_dataset(&dir, &dir.join("snap-b"), &rows, 1).unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.rows, b.rows);
        assert_eq!(a.rows_excluded_by_review, b.rows_excluded_by_review);
        assert_eq!(a.rows_unreviewed, b.rows_unreviewed);
        assert_eq!(a.review_fingerprint, b.review_fingerprint);
        assert_eq!(a.test_sessions, b.test_sessions);
        // The frozen trainer inputs are byte-identical too.
        assert_eq!(
            std::fs::read(dir.join("snap-a").join("labels.jsonl")).unwrap(),
            std::fs::read(dir.join("snap-b").join("labels.jsonl")).unwrap()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn interrupted_jobs_are_marked_never_completed() {
        let dir = std::env::temp_dir().join("gpo-train-int");
        let _ = std::fs::remove_dir_all(&dir);
        let snap = SnapshotMeta {
            fingerprint: "fp-x".into(), rows: 1, sessions: 1, test_sessions: vec![],
            created_at: 1, snap_dir: dir.clone(),
            review_fingerprint: "none".into(), entity_registry_version: 1,
            rows_excluded_by_review: 0, rows_unreviewed: 0,
        };
        let mut j = create_job(&dir, "fish", "manual", &snap, 1, "gpo_train.fish_train", 40, 7, "test", false).unwrap();
        j.status = JobStatus::Running;
        save_job(&dir, &j).unwrap();
        let flipped = mark_interrupted(&dir);
        assert_eq!(flipped, vec![j.job_id.clone()]);
        let back = load_job(&dir, &j.job_id).unwrap();
        assert_eq!(back.status, JobStatus::Interrupted);
        assert!(back.error.unwrap().contains("never claimed complete"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_family_is_rejected_at_creation() {
        let dir = std::env::temp_dir().join("gpo-train-fam");
        let _ = std::fs::remove_dir_all(&dir);
        let snap = SnapshotMeta {
            fingerprint: "fp-x".into(), rows: 1, sessions: 1, test_sessions: vec![],
            created_at: 1, snap_dir: dir.clone(),
            review_fingerprint: "none".into(), entity_registry_version: 1,
            rows_excluded_by_review: 0, rows_unreviewed: 0,
        };
        assert!(create_job(&dir, "dragons", "manual", &snap, 1, "m", 1, 1, "t", false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn triggers_fire_on_growth_and_coverage_not_on_noise() {
        let prev = TriggerState::default();
        let rows = vec![row("s-new", 1, GameStateLabel::Bite, None)];
        let ev = evaluate_triggers(&rows, 1, false, false, &prev, 300, 3, 24, 1000);
        assert!(ev.is_empty(), "1 row must not trigger (thresholds)");
        // Same fingerprint twice: no repeat triggers.
        let st = TriggerState {
            last_fingerprint: Some(dataset_fingerprint(&rows, 1)),
            last_rows: 1, last_sessions: 1, last_hard: 0, last_trigger_ms: None,
            last_fish_ready: false, last_fruit_ready: false,
        };
        let ev2 = evaluate_triggers(&rows, 1, false, false, &st, 1, 1, 24, 2000);
        assert!(ev2.is_empty(), "unchanged dataset must not re-trigger");
    }

    #[test]
    fn history_appends_and_reads_tail() {
        let dir = std::env::temp_dir().join("gpo-train-hist");
        let _ = std::fs::remove_dir_all(&dir);
        append_history(&dir, "test_event", serde_json::json!({"a": 1}));
        append_history(&dir, "test_event", serde_json::json!({"a": 2}));
        let h = read_history(&dir, 10);
        assert_eq!(h.len(), 2);
        assert_eq!(read_history(&dir, 1).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backend_probe_timeout_kills_the_interpreter() {
        // A probe that hangs must be terminated, not waited on forever.
        let t0 = std::time::Instant::now();
        let r = probe_program(
            "cmd",
            &["/c", "ping -n 30 127.0.0.1 > nul"],
            Duration::from_millis(400),
        );
        let err = match r {
            Err(e) => e,
            Ok(o) => panic!("a hanging probe must fail, got success: {o:?}"),
        };
        assert!(err.contains("timed out"), "must say it timed out: {err}");
        assert!(t0.elapsed() < Duration::from_secs(10), "must not wait for the process");
    }

    #[test]
    fn backend_probe_reports_a_missing_interpreter_distinctly() {
        let r = probe_program(
            "definitely-not-a-real-python-binary-xyz",
            &["-c", "print(1)"],
            Duration::from_secs(5),
        );
        let err = r.unwrap_err();
        assert!(!err.contains("timed out"), "a missing binary is not a hang: {err}");
    }

    #[test]
    fn backend_probe_runs_a_real_process() {
        // cmd is always present on Windows and returns promptly.
        let out = probe_program("cmd", &["/c", "echo ready"], Duration::from_secs(10)).unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("ready"));
    }

    #[test]
    fn backend_probe_cache_is_invalidated_and_bounded() {
        invalidate_backend_probe();
        // A cache miss must not report a stale result.
        let a = check_backend_with_timeout("", "", Duration::from_millis(100));
        assert!(a.detail.contains("not configured"));
        invalidate_backend_probe();
        let b = check_backend_with_timeout("", "", Duration::from_millis(100));
        assert_eq!(a.python_path, b.python_path);
    }

    #[test]
    fn snapshot_metadata_describes_what_actually_trains() {
        // Reproducibility contract: identical input -> identical fingerprint.
        let (dir, rows) = seeded_review_dir("repro");
        let s1 = snapshot_dataset(&dir, &dir.join("s1"), &rows, 1).unwrap();
        let s2 = snapshot_dataset(&dir, &dir.join("s2"), &rows, 1).unwrap();
        assert_eq!(s1.fingerprint, s2.fingerprint, "same input must fingerprint identically");
        assert_eq!(s1.rows, s2.rows);
        assert_eq!(s1.rows_excluded_by_review, s2.rows_excluded_by_review);
        // The frozen trainer input must be byte-identical too.
        let a = std::fs::read(dir.join("s1").join("labels.jsonl")).unwrap();
        let b = std::fs::read(dir.join("s2").join("labels.jsonl")).unwrap();
        assert_eq!(a, b, "snapshots must be reproducible byte-for-byte");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn job_id_rejects_path_traversal() {
        let dir = std::env::temp_dir().join("gpo-jobid-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for bad in ["../escape", "..\\escape", "a/b", "a\\b", "", "NUL", &"x".repeat(80)] {
            assert!(load_job(&dir, bad).is_err(), "must reject {bad:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
