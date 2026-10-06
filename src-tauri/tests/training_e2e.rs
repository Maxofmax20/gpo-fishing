//! Training Center end-to-end lifecycle (v5.5, §36).
//!
//! Windows-only (stub trainer is a .cmd like the repo's other OS-coupled
//! tests). Uses a disposable dataset + a stub "trainer" that writes the
//! EXACT files the real pipeline writes (training_log.jsonl,
//! evaluation_test.json, onnx_check.json) plus a REAL onnx artifact copied
//! from the repo models (so loader verification is genuine, not mocked).
//!
//! Proves the safety-critical chain both ways:
//!   good candidate -> PASSED -> Pass -> shadow deployed, rollback restores
//!   bad candidate  -> PASSED job but REJECTED decision, live model untouched
//! At no point can the flow silently replace the running model.

#![cfg(windows)]

use std::path::PathBuf;

use gpo_autofish_lib::core::registry::{
    self, ComparisonVerdict, Lifecycle, ModelMetrics,
};
use gpo_autofish_lib::core::training::{self, JobStatus};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("training-stub")
}

fn repo_models() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models")
}

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gpo-train-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("datasets").join("gpo-vision").join("v1")).unwrap();
    d
}

/// Minimal disposable dataset: two sessions chosen to land in train and
/// test under the deterministic FNV-1a split.
fn mini_dataset(dir: &PathBuf) -> (String, String) {
    let mut train_sess = None;
    let mut test_sess = None;
    for i in 0..5000 {
        let cand = format!("e2e-sess-{i:04}");
        match gpo_autofish_lib::core::ml_dataset::MlDatasetStore::split_of(&cand).as_str() {
            "train" if train_sess.is_none() => train_sess = Some(cand),
            "test" if test_sess.is_none() => test_sess = Some(cand),
            _ => {}
        }
        if train_sess.is_some() && test_sess.is_some() {
            break;
        }
    }
    let (tr, te) = (train_sess.unwrap(), test_sess.unwrap());
    let row = |img: &str, sess: &str, ts: u64, ent: &str| {
        format!(
            r#"{{"image_id":"{img}","dataset_version":1,"session_id":"{sess}","task":"entity_recognition","ocr_text":"Golden Fish","region_name":"drop","ui_label":null,"bbox":null,"game_state":"catch_result","entity_id":"{ent}","annotator":"collector","timestamp_ms":{ts},"source":"gameplay","confidence":null,"hard_example":false,"hard_reason":null,"corrections":[]}}"#
        )
    };
    let body = vec![
        row("e2e-aa", &tr, 1000, "fish:golden"),
        row("e2e-bb", &tr, 2000, "fish:shark"),
        row("e2e-cc", &te, 3000, "fish:golden"),
    ]
    .join("\n")
        + "\n";
    let v1 = dir.join("datasets").join("gpo-vision").join("v1");
    std::fs::write(v1.join("labels.jsonl"), body).unwrap();
    std::fs::write(v1.join("manifest.json"), r#"{"version":1,"split_overrides":{}}"#).unwrap();
    (tr, te)
}

fn current_metrics(acc: f32, f1: f32) -> (ModelMetrics, Vec<String>) {
    let mut per = std::collections::HashMap::new();
    per.insert("fish:shark".to_string(), 0.7f32);
    per.insert("fish:golden".to_string(), 0.6f32);
    (
        ModelMetrics {
            accuracy: acc,
            macro_f1: f1,
            ece: Some(0.1),
            test_sessions: 2,
            test_n: 20,
            per_class_f1: per,
            per_class_test_support: Default::default(),
            kept_accuracy: None,
        },
        vec!["fish:shark".to_string(), "fish:golden".to_string()],
    )
}

/// Run one stub job to completion. Returns (data_dir, job, trainer_out).
fn run_stub_job(
    tag: &str,
    mode: &str,
) -> (PathBuf, training::TrainingJob, PathBuf) {
    let dir = fresh_dir(tag);
    mini_dataset(&dir);
    let store_rows = {
        let ds = gpo_autofish_lib::core::ml_dataset::MlDatasetStore::new(dir.clone());
        ds.annotations()
    };
    assert_eq!(store_rows.len(), 3);
    let snap_dir = dir.join("training").join("snapshots").join("snap-1");
    let snap = training::snapshot_dataset(&dir, &snap_dir, &store_rows, 1).unwrap();
    let mut job = training::create_job(
        &dir, "fish", "manual", &snap, 1, "stub-trainer", 2, 7, "e2e", false,
    )
    .unwrap();
    // The stub writes the trainer-shaped output tree the runner watches.
    let trainer_root = dir.join("troot");
    let out = trainer_root.join("ml").join("output").join(&job.run_id);
    std::fs::create_dir_all(&out).unwrap();
    job.trainer_dir = trainer_root.to_string_lossy().to_string();
    training::save_job(&dir, &job).unwrap();
    let stub = fixtures().join("stub.cmd");
    let real_onnx = repo_models().join("fish_v1.onnx");
    // NOTE: cmd.exe mangles quoted /C tails when Rust's own argv quoting
    // stacks on top, so the stub + payload are staged into the space-free
    // temp dir and invoked with NO quotes at all. Production launch()
    // never uses a shell (direct argv, no quoting need).
    let stage = dir.join("stubstage");
    std::fs::create_dir_all(&stage).unwrap();
    for (name, src) in [("stub.cmd", stub), ("payload.onnx", real_onnx)] {
        std::fs::copy(src, stage.join(name)).unwrap();
    }
    let argv = vec![
        "cmd".to_string(),
        "/C".to_string(),
        stage.join("stub.cmd").to_string_lossy().to_string(),
        out.to_string_lossy().to_string(),
        mode.to_string(),
        stage.join("payload.onnx").to_string_lossy().to_string(),
    ];
    let mut sup = training::Supervisor::new();
    sup.launch_argv(&dir, &mut job, &argv, &[], &dir, 1).unwrap();
    // Poll until the stub finishes (fast) or time out loudly.
    let mut finished = false;
    for _ in 0..200 {
        if sup.poll(&dir, &mut job, &trainer_root.to_string_lossy().to_string()) {
            finished = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(finished, "stub job never finished (mode={mode})");
    (dir, job, out)
}

fn candidate_inputs_from_stub(
    out: &PathBuf,
) -> registry::CandidateInputs {
    let read = |n: &str| {
        serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(out.join(n)).unwrap(),
        )
        .unwrap()
    };
    let eval = read("evaluation_test.json");
    let metrics =
        ModelMetrics::from_trainer_files(&eval, None, None, 2).expect("stub eval must parse");
    let mut classes: Vec<String> = eval["per_entity"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    classes.sort();
    let vocab_sha = {
    // Local stand-in for `core::ml_model::sha256_hex` (private to the crate).
    // Only needs to be stable, not cryptographic, for the test.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in classes.join("\n").as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
};
registry::CandidateInputs {
        onnx_path: out.join("model.onnx"),
        vocab_sha,
        classes,
        temperature: 1.0,
        mean: [0.0; 3],
        std: [1.0; 3],
        metrics,
        eval_json: eval,
    }
}

#[test]
fn e2e_good_candidate_promotes_and_rolls_back() {
    let (dir, job, out) = run_stub_job("promote", "good");
    assert_eq!(job.status, JobStatus::Passed, "trainer succeeded: {:?}", job.error);
    assert!(job.evaluation.is_some(), "real evaluation must be attached");
    // Seed the "running" shadow with the real repo models.
    let models = dir.join("models");
    std::fs::create_dir_all(&models).unwrap();
    for n in ["fish_v1.onnx", "fish_v1.json", "state_v1.onnx", "state_v1.json"] {
        std::fs::copy(repo_models().join(n), models.join(n)).unwrap();
    }
    let before = std::fs::read(models.join("fish_v1.onnx")).unwrap();
    // Register + compare against a deliberately worse current.
    let inputs = candidate_inputs_from_stub(&out);
    let rec = registry::register_candidate(
        &dir, &job.job_id, "fish", &job.dataset_fingerprint, 1, "cfghash", "e2e", &inputs,
    )
    .unwrap();
    assert_eq!(rec.status, Lifecycle::Evaluated);
    let (cur, cur_classes) = current_metrics(0.50, 0.45);
    let qualified = vec!["fish:shark".to_string(), "fish:golden".to_string()];
    let cmp = registry::compare(&cur, &rec.metrics, &cur_classes, &rec.classes, &qualified);
    assert_eq!(cmp.verdict, ComparisonVerdict::Pass, "{:?}", cmp.reasons);
    // Promote: live bytes become the candidate; archive created.
    let manifest =
        registry::promote_to_shadow(&dir, &models, &rec.candidate_id, &cmp).unwrap();
    assert!(manifest.exists());
    assert_eq!(std::fs::read(models.join("fish_v1.onnx")).unwrap(), std::fs::read(out.join("model.onnx")).unwrap());
    let records = registry::load_registry(&dir);
    assert_eq!(
        records.iter().find(|r| r.candidate_id == rec.candidate_id).unwrap().status,
        Lifecycle::Shadow
    );
    // Rollback restores the exact previous bytes.
    registry::rollback_family(&dir, &models, "fish").unwrap();
    assert_eq!(std::fs::read(models.join("fish_v1.onnx")).unwrap(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn e2e_bad_candidate_rejects_and_live_model_untouched() {
    let (dir, job, out) = run_stub_job("reject", "bad");
    assert_eq!(job.status, JobStatus::Passed, "stub trainer itself succeeded");
    let models = dir.join("models");
    std::fs::create_dir_all(&models).unwrap();
    for n in ["fish_v1.onnx", "fish_v1.json", "state_v1.onnx", "state_v1.json"] {
        std::fs::copy(repo_models().join(n), models.join(n)).unwrap();
    }
    let live_before = std::fs::read(models.join("fish_v1.onnx")).unwrap();
    let inputs = candidate_inputs_from_stub(&out);
    let rec = registry::register_candidate(
        &dir, &job.job_id, "fish", &job.dataset_fingerprint, 1, "cfghash", "e2e", &inputs,
    )
    .unwrap();
    // Current model is BETTER: comparison must Reject.
    let (cur, cur_classes) = current_metrics(0.95, 0.93);
    let qualified = vec!["fish:shark".to_string(), "fish:golden".to_string()];
    let cmp = registry::compare(&cur, &rec.metrics, &cur_classes, &rec.classes, &qualified);
    assert_eq!(cmp.verdict, ComparisonVerdict::Reject, "{:?}", cmp.reasons);
    // Promotion refuses on a non-Pass verdict; live bytes provably untouched.
    let err = registry::promote_to_shadow(&dir, &models, &rec.candidate_id, &cmp).unwrap_err();
    assert!(err.contains("refused"), "{err}");
    assert_eq!(std::fs::read(models.join("fish_v1.onnx")).unwrap(), live_before);
    let _ = std::fs::remove_dir_all(&dir);
}
