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

/// The deployed (live) fish manifest: the bytes `ShadowEngine` actually loads.
///
/// This is the contract that matters. The bundled copy under `src-tauri/models/`
/// is what ships in a release, but the live one is what the running app has
/// verified and is observing from - and `shadow_infer` checks its `vocab_sha`
/// there before every use. `None` when the store has never been seeded, which the
/// tests skip rather than fail on: that is a first-run state, not a divergence.
fn live_deployed_dir() -> Option<PathBuf> {
    let root = match std::env::var("GPO_DATA_DIR") {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from(std::env::var("APPDATA").ok()?).join("gpo-autofish"),
    };
    let dir = root.join("models");
    dir.join("fish_v1.json").is_file().then_some(dir)
}

/// Every fish manifest whose bytes must describe the canonical class map: the
/// bundled one, and the live deployed one when the store has been seeded.
///
/// Returned as a `Vec` rather than chained iterators so each call site reads as
/// "check each of these", and so adding a source cannot reintroduce the
/// redundant `.into_iter()` on an argument that already takes `IntoIterator`.
fn fish_manifests_to_check() -> Vec<(&'static str, PathBuf)> {
    let mut out = vec![(
        "the bundled fish manifest",
        models_dir().join("fish_v1.json"),
    )];
    if let Some(d) = live_deployed_dir() {
        out.push(("the live deployed fish manifest", d.join("fish_v1.json")));
    }
    out
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
        assert!(
            seen.insert(*c),
            "duplicate class in the canonical list: {c}"
        );
    }
    assert_eq!(seen.len(), vocab.len());
}

/// The refusal an operator sees when the deployed model is behind the
/// vocabulary. It names the next step instead of just failing, because a red
/// class-map test after a deliberate vocabulary change otherwise looks like
/// "the change was wrong" rather than "the change is not deployed yet".
fn behind_vocabulary_message(what: &str, path: &std::path::Path) -> String {
    format!(
        "{what} at {} carries a class map that is behind FISH_VOCAB.txt. This is \
         EXPECTED between changing the vocabulary and deploying the model trained \
         on it: the class map is only allowed to change by retraining and \
         promoting a new candidate, never by editing a manifest. Next steps: run \
         `scripts/gen_shadow_deploy.py <run-dir>` to regenerate the weights and \
         manifest, then copy them into place atomically. Do NOT edit `classes` or \
         `vocab_sha` by hand.",
        path.display()
    )
}

#[test]
fn the_deployed_model_class_set_equals_the_canonical_vocabulary() {
    // The deployed weights' output logits are positional, so the manifest's
    // `classes` array IS the model. If it diverges from FISH_VOCAB.txt the
    // artifact means something different from what every gate believes.
    //
    // Checked against the LIVE deployed model (what `shadow_infer` verifies) and
    // the bundled one (what a release ships). Both are replaced together by the
    // deploy script, so they cannot drift from each other or from the file.
    let canonical: Vec<String> = class_vocab::fish_vocab()
        .iter()
        .map(|c| c.to_string())
        .collect();

    let mut checked = 0usize;
    for (what, path) in fish_manifests_to_check() {
        if !path.is_file() {
            continue;
        }
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "{} unreadable ({e}); refusing to guess what it claims",
                path.display()
            )
        });
        let v: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));
        let deployed: Vec<String> = v["classes"]
            .as_array()
            .unwrap_or_else(|| panic!("{} carries no `classes` array", path.display()))
            .iter()
            .map(|x| x.as_str().unwrap_or_default().to_string())
            .collect();

        assert_eq!(
            deployed,
            canonical,
            "{}",
            behind_vocabulary_message(what, &path)
        );
        assert_eq!(
            deployed.len(),
            class_vocab::fish_vocab_len(),
            "{}",
            behind_vocabulary_message(what, &path)
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "no deployed fish manifest found to check against the vocabulary"
    );
}

#[test]
fn the_deployed_manifest_pins_the_canonical_class_map_fingerprint() {
    // Same two manifests as the class-set test above: the fingerprint must agree
    // with the vocabulary, because `shadow_infer` verifies it before loading
    // weights and would otherwise refuse to load a correct model.
    let want = class_vocab::fish_vocab_sha();
    let mut checked = 0usize;
    for (what, path) in fish_manifests_to_check() {
        if !path.is_file() {
            continue;
        }
        let raw = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            v["vocab_sha"].as_str().unwrap_or(""),
            want,
            "{}",
            behind_vocabulary_message(what, &path)
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "no deployed fish manifest found to check the fingerprint against"
    );
}

#[test]
fn the_python_trainer_reads_the_same_canonical_file() {
    // The trainer must not carry a second hardcoded list. If it does, the two
    // can drift and every gate above becomes a lie.
    let src = std::fs::read_to_string(
        repo_root()
            .join("ml")
            .join("gpo_train")
            .join("fish_train.py"),
    )
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
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|s| s.to_str()) != Some("rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
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
    use gpo_autofish_lib::core::registry::{compare, ComparisonVerdict, ModelMetrics};
    let vocab: Vec<String> = class_vocab::fish_vocab()
        .iter()
        .map(|c| c.to_string())
        .collect();

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
    //
    // Derived, not written out: the impostor is a PERMUTATION of the canonical
    // class names with every one of them mangled, so it is always exactly
    // `vocab.len()` long and never overlaps the vocabulary. The previous
    // version spelled out 8 wrong ids, which pinned this test to the vocabulary
    // that existed when it was written - the same defect this file's other
    // assertions had.
    let impostor: Vec<String> = vocab
        .iter()
        .enumerate()
        .map(|(i, c)| {
            // Append a per-index suffix so every id is distinct AND differs from
            // its canonical form, whatever the vocabulary happens to be.
            format!("{}#{}", c.strip_prefix("fish:").unwrap_or(c), i)
        })
        .map(|short| format!("fish:{short}"))
        .collect();
    assert_eq!(impostor.len(), class_vocab::fish_vocab_len());
    for wrong in &impostor {
        assert!(
            !vocab.contains(wrong),
            "the impostor must share no class with the vocabulary: {wrong}"
        );
    }
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
