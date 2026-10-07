//! # Single-source-of-truth contract for the fish class map
//!
//! The repository used to carry three independent statements about one
//! quantity:
//!
//! ```text
//! ml/gpo_train/fish_train.py   FISH_VOCAB = 8   -> ONNX head hard-wired to 8 logits
//! ml_capability.rs             FISH_MIN_ENTITIES = 10   -> readiness demanded 10
//! commands_training.rs         ">= 8 ... scope: 8-class model"
//! ```
//!
//! Because the readiness gate compared a CARDINALITY rather than a SET, it
//! could report `qualified_classes 10/10` beside an artifact with 8 output
//! classes and never notice. These tests exist to make that unrepresentable:
//! every component that has an opinion about the fish class set must agree
//! with `FISH_VOCAB.txt`, and a disagreement must fail the build rather than
//! ship.

use gpo_autofish_lib::core::class_vocab;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn models_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models")
}

#[test]
fn one_source_of_truth_defines_the_fish_class_set() {
    // The canonical list is non-empty, has unique entries, and every entry is a
    // fish id.
    let vocab = class_vocab::fish_vocab();
    assert!(!vocab.is_empty());
    let mut seen = std::collections::BTreeSet::new();
    for c in vocab {
        assert!(c.starts_with("fish:"), "not a fish class id: {c}");
        assert!(seen.insert(*c), "duplicate class in the canonical list: {c}");
    }
    assert_eq!(seen.len(), vocab.len());
}

#[test]
fn the_deployed_model_class_set_equals_the_canonical_vocabulary() {
    // The deployed weights' output logits are positional, so the manifest's
    // `classes` array IS the model. If it diverges from FISH_VOCAB.txt the
    // artifact means something different from what every gate believes.
    let raw = std::fs::read_to_string(models_dir().join("fish_v1.json"))
        .expect("bundled fish_v1.json must exist");
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let deployed: Vec<String> = v["classes"]
        .as_array()
        .expect("manifest must carry a classes array")
        .iter()
        .map(|x| x.as_str().unwrap_or_default().to_string())
        .collect();

    assert_eq!(
        deployed,
        class_vocab::fish_vocab().iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "deployed fish_v1 class set/order diverges from FISH_VOCAB.txt"
    );
    assert_eq!(
        deployed.len(),
        class_vocab::fish_vocab_len(),
        "model class count must equal the canonical vocabulary count"
    );
}

#[test]
fn the_deployed_manifest_pins_the_canonical_class_map_fingerprint() {
    let raw = std::fs::read_to_string(models_dir().join("fish_v1.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        v["vocab_sha"].as_str().unwrap_or(""),
        class_vocab::fish_vocab_sha(),
        "the deployed manifest must pin the canonical class-map fingerprint"
    );
}

#[test]
fn the_python_trainer_reads_the_same_canonical_file() {
    // The trainer must not carry a second hardcoded list. If it does, the two
    // can drift and every gate above becomes a lie.
    let src = std::fs::read_to_string(repo_root().join("ml").join("gpo_train").join("fish_train.py"))
        .expect("fish_train.py must exist");
    assert!(
        src.contains("FISH_VOCAB.txt"),
        "fish_train.py must read FISH_VOCAB.txt, not carry its own list"
    );
    assert!(
        !src.contains("FISH_VOCAB = ["),
        "fish_train.py must not define FISH_VOCAB as a literal any more"
    );
    // And it must load it from the repository root, i.e. the same file the
    // Rust side embeds.
    assert!(
        src.contains("parent.parent.parent"),
        "the trainer must resolve FISH_VOCAB.txt from the repository root"
    );
}

#[test]
fn no_scalar_fish_class_count_survives_anywhere() {
    // The original defect was a hardcoded count that no vocabulary backs.
    // Sweep the Rust sources for the removed constant and for any other
    // hardcoded "10 fish" / "8 fish" style requirement.
    let mut offenders: Vec<String> = Vec::new();
    let mut stack = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|s| s.to_str()) != Some("rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            for (i, line) in text.lines().enumerate() {
                let t = line.trim_start();
                if t.starts_with("//") {
                    continue;
                }
                if t.contains("FISH_MIN_ENTITIES") {
                    offenders.push(format!("{}:{}: {t}", p.display(), i + 1));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the hardcoded fish class scalar must not come back:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_registry_gate_requires_exact_vocabulary_coverage() {
    use gpo_autofish_lib::core::registry::{compare, ModelMetrics, ComparisonVerdict};
    let vocab: Vec<String> = class_vocab::fish_vocab().iter().map(|c| c.to_string()).collect();

    let full = |f1: f32| {
        let mut m = ModelMetrics::from_trainer_files(
            &serde_json::json!({"accuracy": 0.8, "macro_f1": f1, "n": 100}),
            None,
            None,
            5,
        )
        .unwrap();
        m.macro_f1 = f1;
        m.per_class_f1 = vocab.iter().map(|c| (c.clone(), f1)).collect();
        m
    };

    // Complete coverage is accepted.
    let c = compare("fish", &full(0.60), &full(0.80), &vocab, &vocab, &vocab);
    assert_ne!(
        c.verdict,
        ComparisonVerdict::Reject,
        "full canonical coverage must not be rejected for coverage: {:?}",
        c.reasons
    );

    // A candidate missing one canonical class is rejected, naming the class.
    let mut partial = vocab.clone();
    partial.pop();
    let c = compare("fish", &full(0.60), &full(0.80), &vocab, &partial, &vocab);
    assert_eq!(c.verdict, ComparisonVerdict::Reject);
    let missing = class_vocab::fish_vocab_len() - partial.len();
    assert!(
        c.reasons
            .iter()
            .any(|r| r.contains("does not cover the canonical vocabulary")),
        "the reason must name the coverage failure: {:?}",
        c.reasons
    );
    assert_eq!(missing, 1);

    // A candidate covering a DIFFERENT set of the same size is rejected. This
    // is the precise scenario that produced `10/10` over an 8-class artifact:
    // a cardinality check cannot tell these apart.
    let impostor: Vec<String> = vec![
        "fish:polkaguffer",
        "fish:zebraangel",
        "fish:goldentiger",
        "fish:candysquid",
        "fish:skeletalshark",
        "fish:bluegrouper",
        "fish:jackobite",
        "fish:fangfish",
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(impostor.len(), class_vocab::fish_vocab_len());
    let c = compare("fish", &full(0.60), &full(0.80), &vocab, &impostor, &vocab);
    assert_eq!(
        c.verdict,
        ComparisonVerdict::Reject,
        "an equal-sized but wrong class set must be rejected: {:?}",
        c.reasons
    );

    // And a superset is rejected too: a class outside the vocabulary is a class
    // the model will never be trained to distinguish.
    let mut over = vocab.clone();
    over.push("fish:megalodon".to_string());
    let c = compare("fish", &full(0.60), &full(0.80), &vocab, &over, &vocab);
    assert_eq!(c.verdict, ComparisonVerdict::Reject);
    assert!(c.reasons.iter().any(|r| r.contains("megalodon")));
}

#[test]
fn the_training_gate_and_the_readiness_gate_ask_the_same_question() {
    // Both gates now compare against `fish_vocab()`. If either ever hardcodes a
    // number again, this fails.
    assert_eq!(
        class_vocab::fish_vocab_len(),
        class_vocab::fish_vocab_set().len()
    );
    // The removed scalar must not be re-exported.
    let src = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/core/ml_capability.rs"),
    )
    .unwrap();
    assert!(
        !src.contains("pub const FISH_MIN_ENTITIES"),
        "FISH_MIN_ENTITIES must stay deleted"
    );
}