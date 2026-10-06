//! ML Training Center command surface (v5.5).
//!
//! Every command reads real backend state (dataset rows, job files,
//! registry, manifests) and returns it. Nothing here fabricates metrics,
//! and nothing here can enable macro control (no such path exists).

use std::collections::HashSet;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use tauri::State;

use crate::app::AppState;
use crate::bot::Bot;
use crate::config::{Store, TrainingSettings};
use crate::core::ml_capability::{assess_capabilities, qualify_entities};
use crate::core::ml_dataset::{MlDatasetStore, DATASET_VERSION};
use crate::core::registry;
use crate::core::training;

fn rows_of(store: &Store) -> Vec<crate::core::ml_dataset::MlAnnotation> {
    MlDatasetStore::new(store.dir().to_path_buf()).annotations()
}

/// Fail-closed variant for any decision that shapes TRAINING.
///
/// An unreadable row means a row no filter can see, so it could be silently
/// frozen into (or out of) a snapshot. Any caller that derives eligibility,
/// a split, or a snapshot must use this instead of `rows_of`.
fn rows_of_checked(store: &Store) -> Result<Vec<crate::core::ml_dataset::MlAnnotation>, String> {
    MlDatasetStore::new(store.dir().to_path_buf()).annotations_checked()
}

fn trainer_module_for(family: &str) -> Option<&'static str> {
    match family {
        "fish" => Some("gpo_train.fish_train"),
        "state" => Some("gpo_train.train"),
        // No fruit trainer exists; the fruit gate blocks first anyway.
        _ => None,
    }
}

// ---- overview ----

#[derive(Debug, Serialize)]
pub struct GateCheck {
    pub ok: bool,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct FamilyEligibility {
    pub family: String,
    pub eligible: bool,
    pub checks: Vec<GateCheck>,
}

#[derive(Debug, Serialize)]
pub struct DeployedModelInfo {
    pub family: String,
    pub name: String,
    pub version: String,
    pub test_accuracy: Option<f32>,
    pub macro_f1: Option<f32>,
}

#[derive(Debug, Serialize)]
pub struct TrainingOverview {
    pub dataset_version: u32,
    pub rows: usize,
    pub sessions: usize,
    pub last_collection_ms: u64,
    pub last_training_ms: Option<u64>,
    pub new_since_training: usize,
    pub models: Vec<DeployedModelInfo>,
    pub shadow_enabled: bool,
    pub shadow_events: usize,
    pub eligibility: Vec<FamilyEligibility>,
    pub auto_enabled: bool,
    pub backend_available: bool,
    pub backend_detail: String,
    pub production_control: String,
}

fn deployed_models(store: &Store) -> Vec<DeployedModelInfo> {
    let mut out = Vec::new();
    for (family, stem) in [("state", "state_v1"), ("fish", "fish_v1")] {
        let p = store.dir().join("models").join(format!("{stem}.json"));
        let (version, acc, f1) = std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .map(|v| {
                (
                    v.get("version").and_then(|x| x.as_str()).unwrap_or("?").to_string(),
                    v.get("test_accuracy").and_then(|x| x.as_f64()).map(|x| x as f32),
                    v.get("macro_f1").and_then(|x| x.as_f64()).map(|x| x as f32),
                )
            })
            .unwrap_or_else(|| ("?".to_string(), None, None));
        out.push(DeployedModelInfo {
            family: family.to_string(),
            name: stem.to_string(),
            version,
            test_accuracy: acc,
            macro_f1: f1,
        });
    }
    out
}

fn last_training_ms(store: &Store) -> Option<u64> {
    training::list_jobs(store.dir())
        .iter()
        .filter_map(|j| j.finished_at)
        .max()
}

#[allow(clippy::too_many_arguments)]
fn family_eligibility(
    family: &str,
    rows: &[crate::core::ml_dataset::MlAnnotation],
    settings: &TrainingSettings,
    backend_ok: bool,
    backend_detail: &str,
    running: usize,
    store_dir: Option<&std::path::Path>,
    review_floor: f64,
) -> FamilyEligibility {
    let mut checks: Vec<GateCheck> = Vec::new();
    let mut ok = true;
    macro_rules! fail {
        ($text:expr) => {{
            ok = false;
            checks.push(GateCheck { ok: false, text: $text.to_string() });
        }};
    }
    macro_rules! pass {
        ($text:expr) => {{
            checks.push(GateCheck { ok: true, text: $text.to_string() });
        }};
    }
    if !backend_ok {
        fail! {format!("training backend unavailable: {backend_detail}")};
    } else {
        pass! {"training backend reachable (python + torch + trainer dir)".to_string()};
    }
    let allowed = match family {
        "fish" => settings.allow_fish,
        "fruit" => settings.allow_fruit,
        "state" => true,
        _ => false,
    };
    if !allowed {
        fail! {format!("{family} training disabled in settings")};
    }
    if running > 0 {
        fail! {format!("{running} job(s) already running (limit {})", settings.max_concurrent_jobs)};
    } else {
        pass! {"no competing training job".to_string()};
    }
    match family {
        "fish" => {
            let q: Vec<_> =
                qualify_entities(rows, "fish:").into_iter().filter(|e| e.qualified).collect();
            if q.len() >= 8 {
                pass! {format!("{} qualified fish (scope: 8-class model)", q.len())};
            } else {
                fail! {format!("only {}/8 qualified fish for the scoped model", q.len())};
            }
        }
        "fruit" => {
            let q: Vec<_> =
                qualify_entities(rows, "fruit:").into_iter().filter(|e| e.qualified).collect();
            if q.len() >= 10 {
                pass! {format!("{} qualified fruits", q.len())};
            } else {
                fail! {format!(
                    "only {}/10 qualified fruits; no fruit trainer runs until the gate passes",
                    q.len()
                )};
            }
        }
        "state" => {
            pass! {"state gate READY (solved model; retrain rarely needed)".to_string()};
        }
        _ => fail! {format!("unknown family '{family}'")},
    }

    // HUMAN REVIEW GATE. Every check above is about COLLECTED data; none of it
    // says a human ever looked at it. v5.6.x would report NOT_ENOUGH_REVIEW in
    // the Readiness tab while happily training a model on 100%
    // collector-labelled rows - the displayed blocker was decorative.
    //
    // Machine labels (annotator: "collector") are what the bot's own OCR+KB
    // guess produced. Training on them is possible; training on them WITHOUT a
    // human having checked the data is how a wrong label becomes a permanently
    // learned "fact". This gate makes the review requirement enforceable.
    let scoped: Vec<&crate::core::ml_dataset::MlAnnotation> = match family {
        "fish" => rows.iter().filter(|r| r.entity_id.as_deref().is_some_and(|e| e.starts_with("fish:"))).collect(),
        "fruit" => rows.iter().filter(|r| r.entity_id.as_deref().is_some_and(|e| e.starts_with("fruit:"))).collect(),
        _ => rows
            .iter()
            .filter(|r| matches!(r.game_state.as_ref().map(|g| g.as_str()),
                Some("waiting_for_bite") | Some("bite") | Some("catch_result")))
            .collect(),
    };
    if scoped.is_empty() {
        fail! {"no scoped rows for this family yet"};
    } else {
        let dir = store_dir.unwrap_or(std::path::Path::new("."));
        let reviewed = crate::core::review::ReviewStore::new(dir.to_path_buf())
            .list()
            .into_iter()
            .filter(|r| r.training_eligible)
            .filter(|r| scoped.iter().any(|s| s.image_id == r.image_id))
            .count();
        let share = reviewed as f64 / scoped.len() as f64;
        if share >= review_floor {
            pass! {format!(
                "{reviewed}/{} scoped rows human-reviewed and eligible ({:.0}% >= {:.0}%)",
                scoped.len(),
                share * 100.0,
                review_floor * 100.0
            )};
        } else {
            fail! {format!(
                "only {reviewed}/{} scoped rows are human-reviewed and training-eligible \
                 ({:.0}% < {:.0}% floor); collector labels alone are not training evidence",
                scoped.len(),
                share * 100.0,
                review_floor * 100.0
            )};
        }
    }

    FamilyEligibility { family: family.to_string(), eligible: ok, checks }
}

/// Run a blocking aggregate off the UI thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn training_overview(st: State<'_, AppState>) -> Result<TrainingOverview, String> {
    // Snapshot everything the closure needs BEFORE blocking: `State` is a
    // borrow of app state and cannot cross into a blocking task.
    let store_dir = st.store.dir().to_path_buf();
    let python_path = st.settings.read().training.python_path.clone();
    let trainer_dir = st.settings.read().training.trainer_dir.clone();
    let shadow_flag = st.settings.read().features.ml_shadow;
    let tsettings = st.settings.read().training.clone();
    blocking(move || {
        // One parse, reused for every aggregate below. v5.6.x called
        // `rows_of(store)` four times on this path - four full
        // labels.jsonl parses per poll.
        // Read-only view of the same directory the app already uses.
        let store = crate::config::Store::new(store_dir.clone());
        let rows = rows_of(&store);
        let sessions = rows
            .iter()
            .map(|r| r.session_id.as_str())
            .collect::<HashSet<_>>()
            .len();
        let last_collection_ms = rows.iter().map(|r| r.timestamp_ms).max().unwrap_or(0);
        let backend = training::check_backend(&python_path, &trainer_dir);
        let jobs = training::list_jobs(store.dir());
        let running = jobs
            .iter()
            .filter(|j| matches!(j.status, training::JobStatus::Running | training::JobStatus::Evaluating))
            .count();
        let last_ms = last_training_ms(&store);
        let last_rows = jobs
            .iter()
            .filter_map(|j| if j.finished_at == last_ms { Some(j.frozen_rows) } else { None })
            .max()
            .unwrap_or(0);
        let shadow_models = crate::core::shadow_infer::engine(&store.dir().join("models"))
            .map(|e| e.model_names().len())
            .unwrap_or(0);
        // Real count, not the hardcoded 0 v5.6.x reported.
        let shadow_events = registry::soak_stats(store.dir(), "fish_v1", usize::MAX).events
            + registry::soak_stats(store.dir(), "state_v1", usize::MAX).events;
        Ok(TrainingOverview {
            dataset_version: DATASET_VERSION,
            rows: rows.len(),
            sessions,
            last_collection_ms,
            last_training_ms: last_ms,
            new_since_training: rows.len().saturating_sub(last_rows),
            models: deployed_models(&store),
            shadow_enabled: shadow_flag && shadow_models > 0,
            shadow_events,
            eligibility: ["fish", "fruit", "state"]
                .iter()
                .map(|f| family_eligibility(f, &rows, &tsettings, backend.available, &backend.detail, running, Some(store.dir()), tsettings.readiness_min_review_coverage as f64))
                .collect(),
            auto_enabled: tsettings.auto_enabled,
            backend_available: backend.available,
            backend_detail: backend.detail,
            production_control: "OFF".to_string(),
        })
    })
    .await
}

#[tauri::command]
pub async fn training_backend(st: State<'_, AppState>) -> Result<training::BackendStatus, String> {
    let python_path = st.settings.read().training.python_path.clone();
    let trainer_dir = st.settings.read().training.trainer_dir.clone();
    blocking(move || Ok(training::check_backend(&python_path, &trainer_dir))).await
}

// ---- jobs ----

#[tauri::command]
pub fn training_start(
    st: State<'_, AppState>,
    family: String,
    epochs: Option<usize>,
    seed: Option<u64>,
) -> Result<training::TrainingJob, String> {
    let settings = st.settings.read().training.clone();
    let backend = training::check_backend(&settings.python_path, &settings.trainer_dir);
    if !backend.available {
        return Err(format!("training backend unavailable: {}", backend.detail));
    }
    let rows = rows_of_checked(&st.store)?;
    let running = training::list_jobs(st.store.dir())
        .iter()
        .filter(|j| matches!(j.status, training::JobStatus::Running | training::JobStatus::Evaluating))
        .count();
    let elig = family_eligibility(&family, &rows, &settings, true, "", running, Some(st.store.dir()), settings.readiness_min_review_coverage as f64);
    if !elig.eligible {
        return Err(format!(
            "{} training blocked: {}",
            family,
            elig.checks.iter().filter(|c| !c.ok).map(|c| c.text.clone()).collect::<Vec<_>>().join("; ")
        ));
    }
    if trainer_module_for(&family).is_none() {
        return Err(format!("no trainer implemented for family '{family}'"));
    }
    let macro_running = st.bot.is_running();
    let mut warning = None;
    if macro_running && settings.defer_while_fishing {
        warning = Some("macro is running; training competes for CPU (recorded; explicit manual request proceeds)".to_string());
    }
    let snap_dir = st.store.dir().join("training").join("snapshots").join(format!("snap-{}", crate::events::now_ms()));
    let snap = training::snapshot_dataset(st.store.dir(), &snap_dir, &rows, DATASET_VERSION)?;
    let mut job = training::create_job(
        st.store.dir(),
        &family,
        "manual",
        &snap,
        DATASET_VERSION,
        trainer_module_for(&family).unwrap(),
        epochs.unwrap_or(40).clamp(1, 200),
        seed.unwrap_or(7),
        env!("CARGO_PKG_VERSION"),
        macro_running,
    )?;
    job.trainer_dir = settings.trainer_dir.clone();
    training::save_job(st.store.dir(), &job)?;
    st.training.lock().launch(st.store.dir(), &mut job, &settings.python_path, &settings.trainer_dir, settings.max_concurrent_jobs)?;
    training::append_history(
        st.store.dir(),
        "job_started",
        serde_json::json!({"job_id": job.job_id, "family": family, "requested_by": "manual", "warning": warning}),
    );
    Ok(job)
}

#[tauri::command]
pub fn training_cancel(st: State<'_, AppState>, job_id: String) -> Result<training::TrainingJob, String> {
    let mut job = training::load_job(st.store.dir(), &job_id)?;
    st.training.lock().cancel(st.store.dir(), &mut job)?;
    training::append_history(st.store.dir(), "job_cancelled", serde_json::json!({"job_id": job_id}));
    Ok(job)
}

#[tauri::command]
pub fn training_jobs(st: State<'_, AppState>) -> Vec<training::TrainingJob> {
    let mut sup = st.training.lock();
    let mut jobs = training::list_jobs(st.store.dir());
    for j in jobs.iter_mut().filter(|j| !j.status.finished()) {
        let before = j.status;
        sup.poll(st.store.dir(), j, &st.settings.read().training.trainer_dir);
        if j.status != before && j.status == training::JobStatus::Passed {
            training::append_history(st.store.dir(), "job_passed", serde_json::json!({"job_id": j.job_id}));
        }
    }
    jobs
}

#[tauri::command]
pub fn training_job(st: State<'_, AppState>, job_id: String) -> Result<training::TrainingJob, String> {
    let mut job = training::load_job(st.store.dir(), &job_id)?;
    st.training.lock().poll(st.store.dir(), &mut job, &st.settings.read().training.trainer_dir);
    Ok(job)
}

#[tauri::command]
pub fn training_restart(st: State<'_, AppState>, job_id: String) -> Result<training::TrainingJob, String> {
    // Honest restart: a NEW job with a FRESH snapshot (data may have grown).
    // Resume is not offered: torch training is not reproducibly resumable.
    let old = training::load_job(st.store.dir(), &job_id)?;
    if !old.status.finished() {
        return Err("only finished jobs restart (cancel it first)".to_string());
    }
    let settings = st.settings.read().training.clone();
    let rows = rows_of_checked(&st.store)?;
    let snap_dir = st.store.dir().join("training").join("snapshots").join(format!("snap-{}", crate::events::now_ms()));
    let snap = training::snapshot_dataset(st.store.dir(), &snap_dir, &rows, DATASET_VERSION)?;
    let mut job = training::create_job(
        st.store.dir(),
        &old.model_family,
        &format!("{}(restart)", old.requested_by),
        &snap,
        DATASET_VERSION,
        &old.trainer_module,
        old.epochs,
        old.seed,
        env!("CARGO_PKG_VERSION"),
        st.bot.is_running(),
    )?;
    job.trainer_dir = settings.trainer_dir.clone();
    training::save_job(st.store.dir(), &job)?;
    st.training.lock().launch(st.store.dir(), &mut job, &settings.python_path, &settings.trainer_dir, settings.max_concurrent_jobs)?;
    training::append_history(st.store.dir(), "job_restarted", serde_json::json!({"from": job_id, "to": job.job_id}));
    Ok(job)
}

#[tauri::command]
pub fn training_discard(st: State<'_, AppState>, job_id: String) -> Result<training::TrainingJob, String> {
    let mut job = training::load_job(st.store.dir(), &job_id)?;
    if !matches!(job.status, training::JobStatus::Interrupted | training::JobStatus::Queued) {
        return Err("only INTERRUPTED or QUEUED jobs can be discarded".to_string());
    }
    job.status = training::JobStatus::Cancelled;
    job.finished_at = Some(crate::events::now_ms());
    training::save_job(st.store.dir(), &job)?;
    Ok(job)
}

// ---- candidates / promotion ----

fn trainer_out_for(job: &training::TrainingJob) -> std::path::PathBuf {
    std::path::Path::new(&job.trainer_dir).join("ml").join("output").join(&job.run_id)
}

fn candidate_inputs_for(
    job: &training::TrainingJob,
) -> Result<registry::CandidateInputs, String> {
    let out = trainer_out_for(job);
    let read = |n: &str| {
        std::fs::read_to_string(out.join(n))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .ok_or_else(|| format!("{n} missing/unreadable in trainer output"))
    };
    let eval = read("evaluation_test.json")?;
    let cal = read("calibration.json").ok();
    let rej = read("rejection.json").ok();
    let cfg = read("config.json")?;
    let snap = read("dataset_snapshot.json").ok();
    let test_sessions = snap
        .as_ref()
        .and_then(|s| s.get("test_sessions"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let metrics = registry::ModelMetrics::from_trainer_files(&eval, cal.as_ref(), rej.as_ref(), test_sessions)
        .ok_or_else(|| "evaluation files lack core accuracy/macro-F1: refusing".to_string())?;
    // CLASS MAP (v5.7.0). The trainer's OWN vocabulary is the authority for
    // output-index -> label. v5.6.x derived this from the evaluation file's
    // keys and then SORTED them, which only coincided with the trainer's order
    // because the vocabulary happened to be alphabetical. Any insertion or
    // reorder would silently permute every label while the artifact checksum
    // and output width stayed identical - unobservable, and unrecoverable.
    // Prefer `config.json["vocab"]` and refuse when it disagrees with the
    // evaluated classes.
    let per = eval.get("per_entity").or_else(|| eval.get("per_class"));
    let evaluated: Vec<String> = per
        .and_then(|m| m.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    let classes: Vec<String> = match cfg.get("vocab").and_then(|v| v.as_array()) {
        Some(a) => {
            let vocab: Vec<String> = a
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect();
            if vocab.is_empty() {
                return Err("config.json vocab is empty: refusing".to_string());
            }
            // The two must describe the same class SET. A different ORDER is
            // fine (the trainer defines the order); a different SET is not.
            let mut a = vocab.clone();
            let mut b = evaluated.clone();
            a.sort();
            b.sort();
            if !evaluated.is_empty() && a != b {
                return Err(format!(
                    "class map mismatch: trainer vocab {:?} does not match evaluated classes {:?}; refusing",
                    vocab, evaluated
                ));
            }
            vocab
        }
        None => {
            // No vocabulary recorded (legacy run): fall back, but say so.
            tracing::warn!("config.json has no vocab; falling back to sorted evaluation keys");
            let mut c = evaluated;
            c.sort();
            c
        }
    };
    if classes.is_empty() {
        return Err("no class map available: refusing to register".to_string());
    }
    // Stable fingerprint of the index -> label mapping, carried into the
    // registry and the manifest so a later permutation is detectable.
    let vocab_sha = crate::core::ml_model::sha256_hex(classes.join("\n").as_bytes());
    let norm = cfg
        .get("normalization")
        .ok_or_else(|| "config.json lacks normalization: refusing".to_string())?;
    let arr = |k: &str| {
        norm.get(k)
            .and_then(|v| v.as_array())
            .filter(|a| a.len() == 3)
            .and_then(|a| {
                Some([
                    a[0].as_f64()? as f32,
                    a[1].as_f64()? as f32,
                    a[2].as_f64()? as f32,
                ])
            })
            .ok_or_else(|| format!("normalization.{k} malformed: refusing"))
    };
    let temperature = read("calibration.json").ok().and_then(|c| c.get("temperature").and_then(|t| t.as_f64())).unwrap_or(1.0) as f32;
    Ok(registry::CandidateInputs {
        vocab_sha,
        onnx_path: out.join(if job.model_family == "fish" { "fish_vision_v1.onnx" } else { "gpo_vision_v1.onnx" }),
        classes,
        temperature,
        mean: arr("mean")?,
        std: arr("std")?,
        metrics,
        eval_json: eval,
    })
}

/// Per-class TEST support, used to floor the per-class regression gate.
///
/// A class with 2 test examples can single-handedly trigger a reject, or clear
/// one with a lucky flip. `compare()` needs the support to make that judgement
/// honestly, and the trainer now writes it (`test_support`).
fn deployed_test_support(v: &serde_json::Value) -> std::collections::HashMap<String, usize> {
    v.get("test_support")
        .and_then(|m| m.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, n)| n.as_u64().map(|n| (k.clone(), n as usize)))
                .collect()
        })
        .unwrap_or_default()
}

fn current_metrics_for(store: &Store, family: &str) -> Result<(registry::ModelMetrics, Vec<String>), String> {
    let stem = registry::shadow_stem(family).ok_or("unknown family")?;
    let raw = std::fs::read_to_string(store.dir().join("models").join(format!("{stem}.json")))
        .map_err(|_| format!("no deployed {family} manifest"))?;
    let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let per: Vec<String> = v
        .get("per_class_f1")
        .and_then(|m| m.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    let classes: Vec<String> = v
        .get("classes")
        .and_then(|m| m.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let m = registry::ModelMetrics {
        accuracy: v.get("test_accuracy").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
        macro_f1: v.get("macro_f1").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
        ece: v.get("ece").and_then(|x| x.as_f64()).map(|x| x as f32),
        test_sessions: v.get("test_sessions").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
        test_n: v.get("test_n").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
        per_class_f1: per
            .iter()
            .filter_map(|c| {
                v.get("per_class_f1")?.get(c)?.as_f64().map(|f| (c.clone(), f as f32))
            })
            .collect(),
        per_class_test_support: deployed_test_support(&v),
        kept_accuracy: None,
    };
    Ok((m, if per.is_empty() { classes } else { per }))
}

fn qualified_now(store: &Store, family: &str) -> Vec<String> {
    let prefix = match family {
        "fish" => "fish:",
        "fruit" => "fruit:",
        _ => return vec![],
    };
    qualify_entities(&rows_of(store), prefix).into_iter().filter(|q| q.qualified).map(|q| q.entity).collect()
}

fn decide_for_job(
    store: &Arc<Store>,
    job: &mut training::TrainingJob,
    auto: bool,
    auto_promote: bool,
) -> String {
    // Register candidate from the real trainer outputs, compare, and either
    // promote (manual always allowed on Pass; auto only with the flag) or
    // record REJECTED. Returns a human-readable outcome.
    let inputs = match candidate_inputs_for(job) {
        Ok(i) => i,
        Err(e) => {
            job.status = training::JobStatus::Failed;
            job.error = Some(e.clone());
            let _ = training::save_job(store.dir(), job);
            return format!("FAILED: {e}");
        }
    };
    let cfg_hash = {
        use std::fmt::Write as _;
        let mut s = String::new();
        let _ = write!(s, "{}:{}:{}", job.trainer_module, job.epochs, job.seed);
        s
    };
    let rec = match registry::register_candidate(
        store.dir(),
        &job.job_id,
        &job.model_family,
        &job.dataset_fingerprint,
        job.dataset_version,
        &cfg_hash,
        &job.code_version,
        &inputs,
    ) {
        Ok(r) => r,
        Err(e) => {
            job.status = training::JobStatus::Failed;
            job.error = Some(e.clone());
            let _ = training::save_job(store.dir(), job);
            return format!("FAILED: {e}");
        }
    };
    job.candidate_id = Some(rec.candidate_id.clone());
    let (cur_metrics, cur_classes) = match current_metrics_for(store, &job.model_family) {
        Ok(v) => v,
        Err(e) => {
            let _ = training::save_job(store.dir(), job);
            return format!("candidate {} registered; current metrics unavailable: {e}", rec.candidate_id);
        }
    };
    let cmp = registry::compare(&cur_metrics, &rec.metrics, &cur_classes, &rec.classes, &qualified_now(store, &job.model_family));
    if cmp.verdict != registry::ComparisonVerdict::Pass {
        job.status = training::JobStatus::Rejected;
        job.error = Some(format!("candidate {} rejected: {}", rec.candidate_id, cmp.reasons.join("; ")));
        let mut records = registry::load_registry(store.dir());
        if let Some(r) = records.iter_mut().find(|r| r.candidate_id == rec.candidate_id) {
            r.status = registry::Lifecycle::Rejected;
            r.decision_reason = Some(cmp.reasons.join("; "));
        }
        let _ = registry::save_registry(store.dir(), &records);
        let _ = training::save_job(store.dir(), job);
        training::append_history(
            store.dir(),
            "candidate_rejected",
            serde_json::json!({"candidate": rec.candidate_id, "reasons": cmp.reasons}),
        );
        return format!("REJECTED: {}", cmp.reasons.join("; "));
    }
    if auto && !auto_promote {
        let _ = training::save_job(store.dir(), job);
        return format!(
            "candidate {} PASSED evaluation ({}); auto-promotion disabled, awaiting manual review",
            rec.candidate_id,
            cmp.reasons.join("; ")
        );
    }
    match registry::promote_to_shadow(store.dir(), &store.dir().join("models"), &rec.candidate_id, &cmp) {
        Ok(manifest) => {
            training::append_history(
                store.dir(),
                "candidate_promoted",
                serde_json::json!({"candidate": rec.candidate_id, "manifest": manifest.to_string_lossy(), "auto": auto}),
            );
            format!("PROMOTED {} to shadow ({})", rec.candidate_id, manifest.to_string_lossy())
        }
        Err(e) => {
            job.status = training::JobStatus::Failed;
            job.error = Some(e.clone());
            let _ = training::save_job(store.dir(), job);
            format!("promotion FAILED (rollback attempted): {e}")
        }
    }
}

#[tauri::command]
pub fn training_candidates(st: State<'_, AppState>) -> Vec<registry::CandidateRecord> {
    registry::load_registry(st.store.dir())
}

#[derive(Debug, Serialize)]
pub struct CompareView {
    pub candidate: registry::CandidateRecord,
    pub current_metrics: registry::ModelMetrics,
    pub comparison: registry::Comparison,
}

#[tauri::command]
pub fn training_compare(st: State<'_, AppState>, candidate_id: String) -> Result<CompareView, String> {
    let records = registry::load_registry(st.store.dir());
    let rec = records.iter().find(|r| r.candidate_id == candidate_id).ok_or("candidate not found")?.clone();
    let (cur_metrics, cur_classes) = current_metrics_for(&st.store, &rec.model_family)?;
    let cmp = registry::compare(
        &cur_metrics,
        &rec.metrics,
        &cur_classes,
        &rec.classes,
        &qualified_now(&st.store, &rec.model_family),
    );
    Ok(CompareView { candidate: rec, current_metrics: cur_metrics, comparison: cmp })
}

#[tauri::command]
pub fn training_promote(st: State<'_, AppState>, candidate_id: String) -> Result<String, String> {
    let records = registry::load_registry(st.store.dir());
    let rec = records.iter().find(|r| r.candidate_id == candidate_id).ok_or("candidate not found")?.clone();
    if rec.status != registry::Lifecycle::Evaluated {
        return Err(format!("candidate is {:?}, only EVALUATED promotes", rec.status));
    }
    let (cur_metrics, cur_classes) = current_metrics_for(&st.store, &rec.model_family)?;
    let cmp = registry::compare(
        &cur_metrics,
        &rec.metrics,
        &cur_classes,
        &rec.classes,
        &qualified_now(&st.store, &rec.model_family),
    );
    if cmp.verdict != registry::ComparisonVerdict::Pass {
        // Record the rejection on the candidate (manual review reached the
        // same fail-closed verdict).
        let mut records = records;
        if let Some(r) = records.iter_mut().find(|r| r.candidate_id == candidate_id) {
            r.status = registry::Lifecycle::Rejected;
            r.decision_reason = Some(cmp.reasons.join("; "));
        }
        let _ = registry::save_registry(st.store.dir(), &records);
        return Err(format!("promotion refused: {}", cmp.reasons.join("; ")));
    }
    let manifest = registry::promote_to_shadow(st.store.dir(), &st.store.dir().join("models"), &candidate_id, &cmp)?;
    training::append_history(
        st.store.dir(),
        "candidate_promoted",
        serde_json::json!({"candidate": candidate_id, "manifest": manifest.to_string_lossy(), "auto": false}),
    );
    Ok(format!("promoted to shadow: {}", manifest.to_string_lossy()))
}

#[tauri::command]
pub fn training_rollback(st: State<'_, AppState>, family: String) -> Result<String, String> {
    let manifest = registry::rollback_family(st.store.dir(), &st.store.dir().join("models"), &family)?;
    training::append_history(
        st.store.dir(),
        "rollback",
        serde_json::json!({"family": family, "manifest": manifest.to_string_lossy()}),
    );
    Ok(format!("rolled back to archive: {}", manifest.to_string_lossy()))
}

#[tauri::command]
pub fn training_history(st: State<'_, AppState>, tail: Option<usize>) -> Vec<serde_json::Value> {
    training::read_history(st.store.dir(), tail.unwrap_or(100).clamp(1, 1000))
}

// ---- explorer + review queue ----

#[derive(Debug, Serialize)]
pub struct ExplorerEntity {
    pub entity: String,
    pub examples: usize,
    pub sessions: usize,
    pub train: usize,
    pub validation: usize,
    pub test: usize,
    pub ocr_agree: usize,
    pub ocr_disagree: usize,
    pub qualified: bool,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct DatasetExplorer {
    pub rows: usize,
    pub sessions: usize,
    pub states: std::collections::HashMap<String, usize>,
    pub entities: usize,
    pub hard_examples: usize,
    pub ocr_bearing: usize,
    pub ocr_empty_result: usize,
    pub null_entity_result: usize,
    pub fish: Vec<ExplorerEntity>,
    pub fruits: Vec<ExplorerEntity>,
}

fn qual_reason(prefix: &str, q: &crate::core::ml_capability::EntityQual) -> String {
    let _ = prefix;
    let mut r = Vec::new();
    if q.examples < 20 {
        r.push(format!("need {} more real examples (>=20)", 20 - q.examples));
    }
    if q.sessions < 3 {
        r.push(format!("need session diversity (>=3, have {})", q.sessions));
    }
    if !q.test_covered {
        r.push("need TEST coverage".to_string());
    }
    if r.is_empty() { "qualified".to_string() } else { r.join("; ") }
}

#[tauri::command]
pub fn dataset_explorer(st: State<'_, AppState>) -> DatasetExplorer {
    let rows = rows_of(&st.store);
    let mut states = std::collections::HashMap::new();
    let mut hard = 0;
    let mut ocr = 0;
    let mut ocr_empty_result = 0;
    let mut null_result = 0;
    let mut entities = HashSet::new();
    for r in &rows {
        *states.entry(r.game_state.map(|g| g.as_str()).unwrap_or("none").to_string()).or_insert(0) += 1;
        if r.hard_example {
            hard += 1;
        }
        if !r.ocr_text.trim().is_empty() {
            ocr += 1;
        }
        if let Some(g) = r.game_state {
            if g.as_str() == "catch_result" {
                if r.ocr_text.trim().is_empty() {
                    ocr_empty_result += 1;
                }
                if r.entity_id.is_none() {
                    null_result += 1;
                }
            }
        }
        if let Some(e) = r.entity_id.as_deref() {
            entities.insert(e);
        }
    }
    let fish: Vec<ExplorerEntity> = qualify_entities(&rows, "fish:")
        .into_iter()
        .map(|q| {
            let reason = qual_reason("", &q);
            ExplorerEntity {
                entity: q.entity, examples: q.examples, sessions: q.sessions,
                train: q.train, validation: q.validation, test: q.test,
                ocr_agree: q.ocr_agree, ocr_disagree: q.ocr_disagree,
                qualified: q.qualified, reason,
            }
        })
        .collect();
    let fruits: Vec<ExplorerEntity> = qualify_entities(&rows, "fruit:")
        .into_iter()
        .map(|q| {
            let reason = qual_reason("", &q);
            ExplorerEntity {
                entity: q.entity, examples: q.examples, sessions: q.sessions,
                train: q.train, validation: q.validation, test: q.test,
                ocr_agree: q.ocr_agree, ocr_disagree: q.ocr_disagree,
                qualified: q.qualified, reason,
            }
        })
        .collect();
    DatasetExplorer {
        sessions: rows.iter().map(|r| r.session_id.as_str()).collect::<HashSet<_>>().len(),
        rows: rows.len(),
        states,
        entities: entities.len(),
        hard_examples: hard,
        ocr_bearing: ocr,
        ocr_empty_result,
        null_entity_result: null_result,
        fish,
        fruits,
    }
}

#[derive(Debug, Serialize)]
pub struct ReviewItem {
    pub image_id: String,
    pub session_id: String,
    pub timestamp_ms: u64,
    pub ocr_text: String,
    pub entity_id: Option<String>,
    pub reasons: Vec<String>,
}

#[tauri::command]
pub fn review_queue(st: State<'_, AppState>) -> Vec<ReviewItem> {
    // Real uncertain pools only: hard examples, OCR-empty RESULTs,
    // entity-less RESULTs. Newest first, capped. Review actions reuse the
    // existing ml_annotate command (annotations, never silent rewrites).
    let mut rows = rows_of(&st.store);
    rows.sort_by_key(|r| std::cmp::Reverse(r.timestamp_ms));
    let mut out = Vec::new();
    for r in rows {
        if out.len() >= 100 {
            break;
        }
        let mut reasons = Vec::new();
        if r.hard_example {
            reasons.push(format!("hard: {}", r.hard_reason.as_deref().unwrap_or("flagged")));
        }
        if let Some(g) = r.game_state {
            if g.as_str() == "catch_result" {
                if r.ocr_text.trim().is_empty() {
                    reasons.push("OCR empty on RESULT".to_string());
                }
                if r.entity_id.is_none() {
                    reasons.push("no entity linked".to_string());
                }
            }
        }
        if reasons.is_empty() {
            continue;
        }
        out.push(ReviewItem {
            image_id: r.image_id.clone(),
            session_id: r.session_id.clone(),
            timestamp_ms: r.timestamp_ms,
            ocr_text: r.ocr_text.chars().take(160).collect(),
            entity_id: r.entity_id.clone(),
            reasons,
        });
    }
    out
}

// ---- settings ----

#[tauri::command]
pub fn training_settings_get(st: State<'_, AppState>) -> crate::config::TrainingSettings {
    st.settings.read().training.clone()
}

#[tauri::command]
pub fn training_settings_set(
    st: State<'_, AppState>,
    settings: crate::config::TrainingSettings,
) -> Result<crate::config::TrainingSettings, String> {
    if settings.max_concurrent_jobs == 0 || settings.max_concurrent_jobs > 2 {
        return Err("max_concurrent_jobs must be 1 or 2".to_string());
    }
    if settings.min_new_samples < 50 {
        return Err("min_new_samples must be >= 50".to_string());
    }
    if settings.trigger_cooldown_hours == 0 || settings.trigger_cooldown_hours > 24 * 30 {
        return Err("trigger_cooldown_hours must be 1..720".to_string());
    }
    if settings.python_path.len() > 512 || settings.trainer_dir.len() > 512 {
        return Err("path too long".to_string());
    }
    // Readiness bars are user-visible configuration, but they are safety
    // gates: a 0 (or an absurd value) would let a mediocre model satisfy
    // SHADOW_READY. Valid ranges keep the meaning of "ready" meaningful
    // without hiding the threshold from the user.
    if !(0.0..=1.0).contains(&settings.readiness_min_macro_f1)
        || settings.readiness_min_macro_f1 < 0.3
    {
        return Err("readiness_min_macro_f1 must be 0.30..=1.00".to_string());
    }
    if !(0.0..=1.0).contains(&settings.readiness_min_worst_f1)
        || settings.readiness_min_worst_f1 < 0.1
    {
        return Err("readiness_min_worst_f1 must be 0.10..=1.00".to_string());
    }
    if !(0.0..=1.0).contains(&settings.readiness_min_shadow_agreement)
        || settings.readiness_min_shadow_agreement < 0.5
    {
        return Err("readiness_min_shadow_agreement must be 0.50..=1.00".to_string());
    }
    if !(0.0..=1.0).contains(&settings.readiness_min_review_coverage)
        || settings.readiness_min_review_coverage < 0.1
    {
        return Err("readiness_min_review_coverage must be 0.10..=1.00".to_string());
    }
    if settings.readiness_min_shadow_events < 20 {
        return Err("readiness_min_shadow_events must be >= 20".to_string());
    }
    if settings.readiness_min_shadow_sessions < 1 || settings.readiness_min_shadow_sessions > 50 {
        return Err("readiness_min_shadow_sessions must be 1..=50".to_string());
    }
    // No shell metacharacters: these values become subprocess argv[0]/cwd,
    // never shell strings, but reject outright anyway (defense in depth).
    for v in [&settings.python_path, &settings.trainer_dir] {
        if v.chars().any(|c| matches!(c, ';' | '&' | '|' | '`' | '$' | '\n' | '\r')) {
            return Err("paths must not contain shell metacharacters".to_string());
        }
    }
    st.settings.write().training = settings.clone();
    st.store.save(&st.settings.read()).map_err(|e| e.to_string())?;
    Ok(settings)
}

// ---- auto-training tick ----

/// Evaluate triggers and act on them. Called every 15 min from setup and on
/// demand from the UI. Spawns NOTHING unless auto_enabled; promotes NOTHING
/// unless auto_promote_to_shadow; never touches production control.
pub(crate) fn training_auto_tick(
    bot: &Arc<Bot>,
    settings: &Arc<RwLock<crate::config::Settings>>,
    store: &Arc<crate::config::Store>,
    training: &Arc<Mutex<training::Supervisor>>,
) {
    let s = settings.read().training.clone();
    let rows = rows_of(store);
    let gates = assess_capabilities(&rows);
    let gate_ready = |id: &str| gates.iter().find(|g| g.id == id).map(|g| g.ready).unwrap_or(false);
    let prev = training::load_trigger_state(store.dir());
    let now = crate::events::now_ms();
    let triggers = training::evaluate_triggers(
        &rows,
        DATASET_VERSION,
        gate_ready("fish_entity"),
        gate_ready("fruit_entity"),
        &prev,
        s.min_new_samples,
        s.min_new_sessions,
        s.trigger_cooldown_hours,
        now,
    );
    // Always advance the baseline so deltas measure since the last check.
    training::save_trigger_state(
        store.dir(),
        &training::TriggerState {
            last_fingerprint: Some(training::dataset_fingerprint(&rows, DATASET_VERSION)),
            last_rows: rows.len(),
            last_sessions: rows.iter().map(|r| r.session_id.as_str()).collect::<HashSet<_>>().len(),
            last_hard: rows.iter().filter(|r| r.hard_example).count(),
            last_trigger_ms: if triggers.is_empty() {
                prev.last_trigger_ms
            } else {
                Some(now)
            },
            last_fish_ready: gate_ready("fish_entity"),
            last_fruit_ready: gate_ready("fruit_entity"),
        },
    );
    for t in &triggers {
        training::append_history(
            store.dir(),
            "trigger",
            serde_json::json!({"type": t.trigger_type, "reason": t.reason, "evidence": t.evidence}),
        );
    }
    if !s.auto_enabled || triggers.is_empty() {
        return;
    }
    if s.defer_while_fishing && bot.is_running() {
        training::append_history(
            store.dir(),
            "auto_deferred",
            serde_json::json!({"reason": "macro running; training deferred until idle"}),
        );
        return;
    }
    // One auto job per tick at most; fish first (fruit gate blocks anyway).
    let family = "fish";
    let running = training::list_jobs(store.dir())
        .iter()
        .filter(|j| matches!(j.status, training::JobStatus::Running | training::JobStatus::Evaluating))
        .count();
    if running >= s.max_concurrent_jobs.max(1) {
        return;
    }
    let elig = family_eligibility(family, &rows, &s, true, "", running, Some(store.dir()), s.readiness_min_review_coverage as f64);
    if !elig.eligible {
        training::append_history(
            store.dir(),
            "auto_skipped",
            serde_json::json!({"family": family, "reasons": elig.checks.iter().filter(|c| !c.ok).map(|c| &c.text).collect::<Vec<_>>()}),
        );
        return;
    }
    let backend = training::check_backend(&s.python_path, &s.trainer_dir);
    if !backend.available {
        return;
    }
    let snap_dir =
        store.dir().join("training").join("snapshots").join(format!("snap-{now}"));
    let Ok(snap) = training::snapshot_dataset(store.dir(), &snap_dir, &rows, DATASET_VERSION) else {
        return;
    };
    let module = match trainer_module_for(family) {
        Some(m) => m,
        None => return,
    };
    let Ok(mut job) = training::create_job(
        store.dir(), family, "auto:new_data", &snap, DATASET_VERSION, module, 40, 7,
        env!("CARGO_PKG_VERSION"), true,
    ) else {
        return;
    };
    job.trainer_dir = s.trainer_dir.clone();
    let _ = training::save_job(store.dir(), &job);
    if training.lock().launch(store.dir(), &mut job, &s.python_path, &s.trainer_dir, s.max_concurrent_jobs).is_ok() {
        training::append_history(
            store.dir(),
            "job_started",
            serde_json::json!({"job_id": job.job_id, "requested_by": "auto:new_data"}),
        );
    }
    // Passed auto jobs are decided (promote-or-reject) on subsequent ticks
    // once finalize() marks them PASSED.
    for mut j in training::list_jobs(store.dir())
        .into_iter()
        .filter(|j| j.status == training::JobStatus::Passed && j.candidate_id.is_none())
    {
        let outcome = decide_for_job(store, &mut j, true, s.auto_promote_to_shadow);
        training::append_history(store.dir(), "auto_decision", serde_json::json!({"job_id": j.job_id, "outcome": outcome}));
    }
}

/// Manual review step for a PASSED job: register the candidate, compare
/// against the current model, and promote on Pass (the click IS the
/// approval) or record REJECTED. Never touches production control.
#[tauri::command]
pub fn training_decide(st: State<'_, AppState>, job_id: String) -> Result<String, String> {
    let mut job = training::load_job(st.store.dir(), &job_id)?;
    if job.status != training::JobStatus::Passed {
        return Err(format!("job is {:?}, only PASSED jobs are decided", job.status));
    }
    if job.candidate_id.is_some() {
        return Err("job already decided (candidate exists)".to_string());
    }
    Ok(decide_for_job(&st.store, &mut job, false, true))
}
