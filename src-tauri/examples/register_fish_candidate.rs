//! Register a trained fish-vision run as an immutable CANDIDATE in the model
//! registry, through the same contract the in-app training flow uses
//! (`registry::register_candidate`).
//!
//! # What this does NOT do
//!
//! It never promotes. Promotion for family "fish" goes through
//! `registry::compare`, which requires the candidate's class set to equal
//! `FISH_VOCAB.txt` exactly. Since `FISH_VOCAB.txt` now IS the 15-class
//! human-reviewed ontology (see the dated note in that file's header), a
//! candidate trained on that ontology is a DIRECT successor to the incumbent —
//! so `registry::compare` is applicable and this example refuses to call it.
//! Promotion is still a deliberate human decision: run the comparison, read the
//! verdict, and only then deploy.
//!
//! Usage:
//!   cargo run --example register_fish_candidate -- <run-dir> <job-id>
//! e.g.
//!   cargo run --example register_fish_candidate -- ml/output/fish-vision-v2-s7 fish-v2-s7

use gpo_autofish_lib::app::data_dir;
use gpo_autofish_lib::core::registry::{self, CandidateInputs, ModelMetrics};
use std::path::{Path, PathBuf};

fn fail(msg: &str) -> ! {
    eprintln!("  REFUSED: {msg}");
    std::process::exit(1)
}

fn read(out: &Path, name: &str) -> Result<serde_json::Value, String> {
    std::fs::read_to_string(out.join(name))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .ok_or_else(|| format!("{name} missing/unreadable in trainer output"))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (run_dir, job_id) = match (args.get(1), args.get(2)) {
        (Some(r), Some(j)) => (PathBuf::from(r), j.clone()),
        _ => fail("usage: register_fish_candidate <run-dir> <job-id> [--init-counter <n>]"),
    };
    let out = run_dir;

    // One-time counter seeding. The fish family never went through
    // register_candidate (fish_v1 was deployed as a bundled manifest), so the
    // monotonic counter file does not exist and next_version_checked refuses to
    // guess while shadow telemetry exists. Seeding it to the deployed manifest's
    // TRUE version (fish_v1 = 1) lets the counter do its job: the next candidate
    // is v2, and v1 can never be recycled.
    if let Some(pos) = args.iter().position(|a| a == "--init-counter") {
        let n: u32 = args
            .get(pos + 1)
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| fail("--init-counter needs a number"));
        let counter = data_dir()
            .join("training")
            .join("versions")
            .join("fish.version");
        if counter.is_file() {
            fail(&format!(
                "counter already exists at {}; refusing to reseed",
                counter.display()
            ));
        }
        if n != 1 {
            fail("the only honest seed is 1: the deployed fish_v1 manifest is version \"1\"");
        }
        std::fs::create_dir_all(counter.parent().unwrap())
            .map_err(|e| e.to_string())
            .unwrap_or_else(|e| fail(&e));
        std::fs::write(&counter, n.to_string())
            .unwrap_or_else(|e| fail(&format!("seed write: {e}")));
        println!("  seeded fish version counter to {n} (deployed fish_v1 is version 1)");
    }

    // The SAME input assembly as commands_training::candidate_inputs_for.
    let eval = read(&out, "evaluation_test.json").unwrap_or_else(|e| fail(&e));
    let cal = read(&out, "calibration.json").ok();
    let rej = read(&out, "rejection.json").ok();
    let cfg = read(&out, "config.json").unwrap_or_else(|e| fail(&e));
    let snap = read(&out, "dataset_snapshot.json").ok();
    let onnx_check = read(&out, "onnx_check.json").unwrap_or_else(|e| fail(&e));
    if !onnx_check
        .get("pass")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        fail("onnx_check.json pass != true: the export did not verify");
    }

    let test_sessions = snap
        .as_ref()
        .and_then(|s| s.get("test_sessions"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let metrics =
        ModelMetrics::from_trainer_files(&eval, cal.as_ref(), rej.as_ref(), test_sessions)
            .unwrap_or_else(|| fail("evaluation files lack core accuracy/macro-F1"));

    let per = eval.get("per_entity").or_else(|| eval.get("per_class"));
    let evaluated: Vec<String> = per
        .and_then(|m| m.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    let vocab: Vec<String> = cfg
        .get("vocab")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if vocab.is_empty() {
        fail("config.json vocab is empty");
    }
    let mut a = vocab.clone();
    let mut b = evaluated.clone();
    a.sort();
    b.sort();
    if !evaluated.is_empty() && a != b {
        fail("trainer vocab and evaluated classes disagree as SETS");
    }
    let vocab_sha = gpo_autofish_lib::core::ml_model::sha256_hex(vocab.join("\n").as_bytes());
    let norm = cfg
        .get("normalization")
        .unwrap_or_else(|| fail("config.json lacks normalization"));
    let arr = |k: &str| -> [f32; 3] {
        norm.get(k)
            .and_then(|v| v.as_array())
            .filter(|a| a.len() == 3)
            .map(|a| {
                [
                    a[0].as_f64().unwrap_or(0.0) as f32,
                    a[1].as_f64().unwrap_or(0.0) as f32,
                    a[2].as_f64().unwrap_or(0.0) as f32,
                ]
            })
            .unwrap_or_else(|| fail("normalization malformed"))
    };
    let temperature = cal
        .as_ref()
        .and_then(|c| c.get("temperature"))
        .and_then(|t| t.as_f64())
        .unwrap_or(1.0) as f32;

    let onnx_path = out.join("fish_vision_v1.onnx");
    if !onnx_path.is_file() {
        fail(&format!("{} not found", onnx_path.display()));
    }

    let dataset_fingerprint = cfg
        .get("dataset_sha256")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let dataset_version = snap
        .as_ref()
        .and_then(|s| s.get("dataset_version"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;
    let code_version = cfg
        .get("source_commit")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let cfg_hash = format!(
        "gpo_train.fish_train_v2:{}:{}",
        cfg.get("epochs").and_then(|v| v.as_u64()).unwrap_or(0),
        cfg.get("seed").and_then(|v| v.as_u64()).unwrap_or(0)
    );

    let inputs = CandidateInputs {
        onnx_path,
        classes: vocab,
        vocab_sha,
        temperature,
        mean: arr("mean"),
        std: arr("std"),
        metrics,
        eval_json: eval,
    };

    let rec = match registry::register_candidate(
        &data_dir(),
        &job_id,
        "fish",
        &dataset_fingerprint,
        dataset_version,
        &cfg_hash,
        &code_version,
        &inputs,
    ) {
        Ok(r) => r,
        Err(e) => fail(&e),
    };

    println!("  registered candidate: {}", rec.candidate_id);
    println!(
        "  family/version      : {} v{}",
        rec.model_family, rec.model_version
    );
    println!("  status              : {:?}", rec.status);
    println!("  classes             : {}", rec.classes.len());
    println!("  artifact sha256     : {}", rec.artifact_sha);
    println!("  dataset fingerprint : {}", rec.dataset_fingerprint);
    println!(
        "  test: acc={:.4} macroF1={:.4} n={} sessions={}",
        rec.metrics.accuracy, rec.metrics.macro_f1, rec.metrics.test_n, rec.metrics.test_sessions
    );
    println!("  NOT promoted. `compare()` is not called here: promotion is a");
    println!("  human decision, and this example's job ends at REGISTERED. Run the");
    println!("  comparison, read the verdict, and only then deploy.");
}
