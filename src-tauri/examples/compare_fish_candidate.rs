//! Attempt promotion of a registered fish candidate through the REAL gate.
//!
//! # What this is
//!
//! One call to `registry::compare("fish", current, candidate, ...)` and an
//! honest print of the verdict and EVERY reason the gate emitted. Nothing is
//! smoothed, weighted away, or reinterpreted: `compare` is the gate, and this
//! example is a lens on it.
//!
//! # What it deliberately does NOT do
//!
//! It never promotes. Promotion is a human decision (FINAL_FEATURE_FREEZE §3.2);
//! this tool's job ends at reporting the verdict. It also never adjusts the
//! incumbent's metrics to make a comparison look better, which matters more than
//! usual here - see the caveat printed at the end.
//!
//! # The cross-vocabulary caveat, stated up front
//!
//! The incumbent `fish_v1.json` was trained on the OLD 8 bare-stem class map.
//! The candidate is trained on the 15 confirmed species. `compare` walks
//! `candidate_classes` and judges a class only when BOTH sides carry it, so
//! every species class is SKIPPED: the per-class regression gate has nothing to
//! compare, and the incumbent's `test_support` is empty, so no thin-support
//! signal exists either.
//!
//! That is a real limitation of comparing across a vocabulary change, not a
//! defect in the gate: there is no honest per-class comparison available between
//! two models that do not share a class map. The macro metrics below are
//! therefore a comparison of DIFFERENT TASKS at different accuracy scales, and
//! this example says so rather than letting 0.99-vs-0.58 read as "4x better at
//! the same job".
//!
//! Usage:
//!   cargo run --example compare_fish_candidate -- <candidate-id>

use gpo_autofish_lib::app::data_dir;
use gpo_autofish_lib::core::ml_capability;
use gpo_autofish_lib::core::ml_dataset::MlDatasetStore;
use gpo_autofish_lib::core::registry::{self, ComparisonVerdict, ModelMetrics};
use gpo_autofish_lib::core::review::ReviewStore;
use gpo_autofish_lib::core::training;

fn fail(msg: &str) -> ! {
    eprintln!("  REFUSED: {msg}");
    std::process::exit(1)
}

fn read_manifest(path: &std::path::Path) -> serde_json::Value {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| fail(&format!("{} unreadable: {e}", path.display())));
    serde_json::from_str(&raw).unwrap_or_else(|e| fail(&format!("{} is not JSON: {e}", path.display())))
}

/// The incumbent's metrics, rebuilt from the DEPLOYED manifest - the same bytes
/// `ShadowEngine` verifies - rather than from anything in the registry. A
/// comparison whose "current" comes from a different source than the running
/// model is not a comparison of deployed models.
fn incumbent_from_deployed(live: bool) -> (ModelMetrics, Vec<String>, String) {
    let path = if live {
        data_dir().join("models").join("fish_v1.json")
    } else {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models").join("fish_v1.json")
    };
    let m = read_manifest(&path);
    let classes: Vec<String> = m["classes"]
        .as_array()
        .unwrap_or_else(|| fail("deployed manifest has no classes"))
        .iter()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect();
    let metrics = ModelMetrics {
        accuracy: m["test_accuracy"].as_f64().unwrap_or(0.0) as f32,
        macro_f1: m["macro_f1"].as_f64().unwrap_or(0.0) as f32,
        ece: m["ece"].as_f64().map(|v| v as f32),
        test_sessions: m["test_sessions"].as_u64().unwrap_or(0) as usize,
        test_n: m["test_n"].as_u64().unwrap_or(0) as usize,
        per_class_f1: m["per_class_f1"]
            .as_object()
            .map(|o| o.iter().filter_map(|(k, v)| v.as_f64().map(|f| (k.clone(), f as f32))).collect())
            .unwrap_or_default(),
        per_class_test_support: m["test_support"]
            .as_object()
            .map(|o| o.iter().filter_map(|(k, v)| v.as_u64().map(|n| (k.clone(), n as usize))).collect())
            .unwrap_or_default(),
        kept_accuracy: None,
    };
    (metrics, classes, path.display().to_string())
}

fn main() {
    let store = data_dir();
    let wanted: Option<String> = std::env::args().nth(1);

    // ---- candidate, from the registry ------------------------------------
    let records = registry::load_registry(&store);
    let rec = match &wanted {
        Some(id) => records
            .iter()
            .find(|c| c.candidate_id == *id)
            .unwrap_or_else(|| fail(&format!("no candidate {id}"))),
        None => records
            .iter()
            .filter(|c| c.model_family == "fish")
            .max_by_key(|c| c.model_version)
            .unwrap_or_else(|| fail("no fish candidate in the registry")),
    };
    println!("candidate        : {}", rec.candidate_id);
    println!("  family/version : {} v{}", rec.model_family, rec.model_version);
    println!("  status         : {:?}", rec.status);
    println!("  classes        : {}", rec.classes.len());
    println!("  vocab_sha      : {}", rec.vocab_sha);
    println!("  artifact_sha256: {}", rec.artifact_sha);
    println!("  dataset        : {}", rec.dataset_fingerprint);
    println!(
        "  test: acc={:.4} macroF1={:.4} n={} sessions={}",
        rec.metrics.accuracy, rec.metrics.macro_f1, rec.metrics.test_n, rec.metrics.test_sessions
    );

    // ---- incumbent, from the DEPLOYED manifest --------------------------
    let live_path = store.join("models").join("fish_v1.json");
    let (current, current_classes, current_src) = incumbent_from_deployed(live_path.is_file());
    println!("\nincumbent        : {}", current_src);
    println!("  classes        : {}", current_classes.len());
    println!(
        "  test: acc={:.4} macroF1={:.4} n={} sessions={}",
        current.accuracy, current.macro_f1, current.test_n, current.test_sessions
    );
    println!("  per_class_f1   : {} entries", current.per_class_f1.len());
    println!("  test_support   : {} entries", current.per_class_test_support.len());

    // ---- qualified classes, from the PROJECTED training set --------------
    // Read through the same projection every gate uses, so "is this class
    // qualified" is one question with one answer.
    let rows = MlDatasetStore::new(store.clone())
        .annotations_checked()
        .unwrap_or_else(|e| fail(&format!("labels.jsonl unreadable: {e}")));
    let projected = training::project_training_labels(&rows, &ReviewStore::new(store.clone()));
    if let Some(err) = &projected.read_error {
        fail(&format!("cannot measure qualification: {err}"));
    }
    let qualified: Vec<String> = ml_capability::qualify_entities(&projected.rows, "fish:")
        .into_iter()
        .filter(|q| q.qualified)
        .map(|q| q.entity)
        .collect();
    println!("\nqualified classes: {} of {} canonical", qualified.len(), class_vocab_len());

    // ---- THE GATE -------------------------------------------------------
    let cand_classes = rec.classes.clone();
    let c = registry::compare(
        "fish",
        &current,
        &rec.metrics,
        &current_classes,
        &cand_classes,
        &qualified,
    );

    println!("\n================ registry::compare verdict ================");
    println!("  verdict       : {:?}", c.verdict);
    println!("  macro-F1 delta: {:+.4}", c.f1_delta);
    println!("  accuracy delta: {:+.4}", c.acc_delta);
    println!("  ece delta     : {:?}", c.ece_delta.map(|d| format!("{d:+.4}")));
    println!("  regressions   : {}", c.regressions.len());
    for (cls, a, b) in &c.regressions {
        println!("      {cls} {a:.3} -> {b:.3}");
    }
    println!("  improvements  : {}", c.improvements.len());
    println!("  reasons ({}):", c.reasons.len());
    for r in &c.reasons {
        println!("      - {r}");
    }

    // ---- what the verdict does and does not mean ------------------------
    let shared: Vec<&String> = cand_classes
        .iter()
        .filter(|c| current_classes.contains(c))
        .collect();
    println!("\n---------------- cross-vocabulary caveat ----------------");
    println!(
        "  candidate classes shared with the incumbent: {} of {}",
        shared.len(),
        cand_classes.len()
    );
    if shared.is_empty() {
        println!("  The two class maps are DISJOINT, so `compare` could not judge a single");
        println!("  per-class regression or thin-support signal: it walks candidate classes");
        println!("  and skips any the incumbent does not also carry. The macro numbers above");
        println!("  therefore compare two DIFFERENT TASKS, not two attempts at one task.");
        println!("  There is no honest per-class comparison available across a vocabulary");
        println!("  change, and this tool does not manufacture one.");
    }

    match c.verdict {
        ComparisonVerdict::Pass => {
            println!("\n  VERDICT IS PASS. Promotion is still a human decision: this tool");
            println!("  does not deploy. Shadow soak evidence starts from ZERO after a");
            println!("  version bump, so SHADOW_READY cannot be claimed today.");
        }
        other => {
            println!("\n  VERDICT IS {other:?}, NOT PASS. Do not deploy. Do not weaken the gate.");
        }
    }
}

fn class_vocab_len() -> usize {
    gpo_autofish_lib::core::class_vocab::fish_vocab_len()
}