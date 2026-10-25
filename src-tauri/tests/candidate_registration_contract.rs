//! Candidate registration contract: the per-class gate must be armed.
//!
//! # What is guarded
//!
//! `registry::compare` decides, per class, whether a candidate's F1 is
//! judgeable at all. It asks `per_class_test_support` for the number of held-out
//! TEST examples behind that F1 and refuses to let a class on fewer than
//! `PER_CLASS_MIN_TEST_SUPPORT` move the verdict.
//!
//! That map is a field on the candidate's metrics. A candidate whose record
//! carried `per_class_f1` but NOT `per_class_test_support` had the per-class
//! regression gate silently disabled - a historical bug in this repository.
//! An absent support map reads as "unknown", and "unknown" is not "too thin",
//! so a class measured on two examples could pass a comparison.
//!
//! So: both maps must be populated, `from_trainer_files` must read the support
//! out of the trainer's own `per_entity[*].support`, and the gate must actually
//! fire when support IS thin. These tests use only the public API, so they also
//! pin that nothing needed for this is private.
//!
//! Read-only: no registry, no store, no files.

use gpo_autofish_lib::core::class_vocab;
use gpo_autofish_lib::core::registry::{self, ComparisonVerdict, ModelMetrics};

fn fish_classes() -> Vec<String> {
    class_vocab::fish_vocab()
        .iter()
        .map(|c| c.to_string())
        .collect()
}

/// A metrics record over the canonical classes with real per-class support.
///
/// `macro_f1` and `accuracy` are parameters because the comparison's bars
/// matter: `compare` evaluates thin support BEFORE the macro-F1 delta, and a
/// candidate that fails the delta is Rejected outright, which would mask the
/// thin-support signal this file is about. So the candidate here must clear the
/// +0.01 macro-F1 bar and keep accuracy flat, leaving the per-class regression
/// as the only open question.
fn metrics_with_support(
    sessions: usize,
    macro_f1: f32,
    over: &[(&str, f32, usize)],
) -> ModelMetrics {
    let per_class_f1: Vec<(String, f32)> =
        fish_classes().iter().map(|c| (c.clone(), 0.80)).collect();
    let per_class_test_support: Vec<(String, usize)> = fish_classes()
        .iter()
        .map(|c| (c.clone(), 40usize))
        .collect();
    let mut m = ModelMetrics {
        holdout_fingerprint: Some("a".repeat(64)),
        accuracy: 0.80,
        macro_f1,
        ece: Some(0.1),
        test_sessions: sessions,
        test_n: 100,
        per_class_f1: per_class_f1.into_iter().collect(),
        per_class_test_support: per_class_test_support.into_iter().collect(),
        kept_accuracy: Some(0.95),
    };
    for (k, v, n) in over {
        m.per_class_f1.insert((*k).to_string(), *v);
        m.per_class_test_support.insert((*k).to_string(), *n);
    }
    m
}

#[test]
fn a_class_on_thin_test_support_cannot_decide_a_verdict() {
    let cls = fish_classes();
    // A class that IS in the canonical vocabulary: `compare` only judges a
    // class both sides share, so a name outside the class map would be skipped
    // and this test would pass for the wrong reason.
    let victim = cls[0].clone();
    // Clears the +0.01 macro-F1 bar and keeps accuracy flat, so the ONLY open
    // question is the per-class regression.
    let cur = metrics_with_support(5, 0.70, &[]);
    // `victim` regresses hard - 0.80 -> 0.10 - but is measured on 3 TEST rows,
    // so its F1 cannot be trusted to mean anything.
    let cand = metrics_with_support(6, 0.74, &[(victim.as_str(), 0.10, 3)]);
    let c = registry::compare("fish", &cur, &cand, &cls, &cls, &cls);
    assert_eq!(
        c.verdict,
        ComparisonVerdict::Inconclusive,
        "a class on 3 held-out examples must not move the verdict: {:?}",
        c.reasons
    );
    assert!(
        c.reasons.iter().any(|r| r.contains("too thin")),
        "the reason must name the thin-support class: {:?}",
        c.reasons
    );
    assert!(
        c.reasons.iter().any(|r| r.contains(victim.as_str())),
        "the reason must name WHICH class is too thin: {:?}",
        c.reasons
    );
}

#[test]
fn the_same_regression_on_supported_examples_is_judged() {
    let cls = fish_classes();
    let victim = cls[0].clone();
    let cur = metrics_with_support(5, 0.70, &[]);
    let cand = metrics_with_support(6, 0.74, &[(victim.as_str(), 0.10, 40)]);
    let c = registry::compare("fish", &cur, &cand, &cls, &cls, &cls);
    assert_ne!(
        c.verdict,
        ComparisonVerdict::Pass,
        "a 0.70 per-class F1 regression on 40 supported examples must not pass: {:?}",
        c.reasons
    );
    assert!(
        c.reasons.iter().any(|r| r.contains("per-class regression")),
        "the regression must be named as the blocker: {:?}",
        c.reasons
    );
}

#[test]
fn test_support_is_read_from_the_trainer_file_not_defaulted() {
    // This is the exact shape `fish_train_v2.py` writes.
    let eval = serde_json::json!({
        "accuracy": 0.80,
        "macro_f1": 0.70,
        "n": 100,
        "per_entity": {
            "fish:shark": {"f1": 0.50, "support": 7},
            "fish:swordfish": {"f1": 0.90, "support": 33},
        },
    });
    let m = ModelMetrics::from_trainer_files(&eval, None, None, 5)
        .expect("a complete evaluation file must load");
    assert_eq!(m.per_class_test_support.get("fish:shark"), Some(&7));
    assert_eq!(m.per_class_test_support.get("fish:swordfish"), Some(&33));
    assert!(
        !m.per_class_test_support.is_empty(),
        "an empty support map silently disables the per-class gate, which is the \
         bug this file exists to keep from returning"
    );
    assert_eq!(m.per_class_f1.get("fish:shark"), Some(&0.50));
}

#[test]
fn the_canonical_vocabulary_is_what_a_candidate_must_cover() {
    // The class set a candidate is judged against is FISH_VOCAB, read live - so
    // this also fails loudly if the vocabulary and the registry's idea of it
    // ever diverge, rather than silently judging against a stale list.
    let cls = fish_classes();
    assert_eq!(cls.len(), class_vocab::fish_vocab_len());
    assert!(
        cls.iter().all(|c| c.starts_with("fish:")),
        "the canonical vocabulary must be fish classes"
    );
    // Coverage is exact: a subset is rejected, naming the class.
    let cur = metrics_with_support(5, 0.70, &[]);
    let cand = metrics_with_support(6, 0.74, &[]);
    let mut partial = cls.clone();
    partial.pop();
    let c = registry::compare("fish", &cur, &cand, &cls, &partial, &cls);
    assert_eq!(c.verdict, ComparisonVerdict::Reject);
    assert!(
        c.reasons
            .iter()
            .any(|r| r.contains("does not cover the canonical vocabulary")),
        "{:?}",
        c.reasons
    );
}
