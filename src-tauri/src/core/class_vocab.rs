//! # Canonical model vocabularies
//!
//! There is exactly ONE place a fish class list is defined: [`FISH_VOCAB.txt`]
//! at the repository root. This module embeds it at COMPILE TIME and exposes it
//! to every consumer, so the trainer, the readiness engine, the training
//! eligibility gate and the candidate registry cannot disagree about what the
//! fish model is supposed to output.
//!
//! ## Why this exists
//!
//! Before v5.7.1 the repository carried two independent numbers for the same
//! quantity:
//!
//! ```text
//! ml/gpo_train/fish_train.py   FISH_VOCAB = 8 entries
//!                              -> FishNet(8) -> an ONNX head hard-wired to
//!                                 8 logits. A 10-class fish model was
//!                                 impossible to build.
//! ml_capability.rs             FISH_MIN_ENTITIES = 10
//!                              -> readiness demanded 10 qualified fish.
//! commands_training.rs         ">= 8 ... scope: 8-class model"
//!                              -> the training gate demanded 8.
//! ```
//!
//! Because readiness counted a *cardinality* rather than comparing a *set*,
//! it could report `qualified_classes 10/10` next to an artifact with 8
//! output classes and never notice. There was also no list naming which 10
//! fish were required, so the scalar could be satisfied by any 10 of the 41 KB
//! fish species.
//!
//! ## The rule now
//!
//! The expected class set is [`fish_vocab()`]. The expected class count is
//! [`fish_vocab()`].len(). Every gate compares SETS, never counts.
//!
//! ## Order is load-bearing
//!
//! Position in the file IS the output-logit index. [`fish_vocab_sha`] is the
//! fingerprint carried in the candidate record and the deployed manifest, and
//! `shadow_infer` verifies it against the live `classes` array before loading
//! weights. Reordering the file changes every class index, which is why the
//! fingerprint exists and why changing the file requires a retrain.
//!
//! ## Scope
//!
//! A class may only be added if it is a verified canonical KB entity AND real
//! gameplay frames of it can be collected and human-reviewed. Never add an
//! entity to reach a class count. See the header of `FISH_VOCAB.txt` for the
//! full rules and the excluded-entity list.

use std::collections::BTreeSet;

/// The canonical fish vocabulary, embedded at compile time.
///
/// Parsed once, lazily. A malformed file (duplicate entry, empty class id,
/// wrong prefix) is a hard error rather than a silent skip: a class map that
/// differs from what the trainer used is exactly the failure this module
/// exists to prevent.
const FISH_VOCAB_RAW: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../FISH_VOCAB.txt"));

/// Parse a vocabulary file: one class per line, `#` comments, blank lines
/// ignored, order preserved.
///
/// Slices are taken from `raw` itself rather than copied, so a `&'static` input
/// yields `&'static` class ids with no allocation and no leak.
fn parse_vocab(raw: &'static str) -> Result<Vec<&'static str>, String> {
    let mut out: Vec<&'static str> = Vec::new();
    for (n, line) in raw.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if !t.starts_with("fish:") {
            return Err(format!(
                "FISH_VOCAB.txt:{}: '{t}' is not a fish class id (expected 'fish:...')",
                n + 1
            ));
        }
        if out.contains(&t) {
            return Err(format!("FISH_VOCAB.txt:{}: duplicate class '{t}'", n + 1));
        }
        out.push(t);
    }
    if out.is_empty() {
        return Err("FISH_VOCAB.txt defines no classes".to_string());
    }
    Ok(out)
}

/// The canonical fish class list, in output-logit order.
///
/// Panics only if `FISH_VOCAB.txt` itself is malformed, which is a build-time
/// defect rather than a runtime condition.
pub fn fish_vocab() -> &'static [&'static str] {
    use std::sync::OnceLock;
    static V: OnceLock<Vec<&'static str>> = OnceLock::new();
    let v = V.get_or_init(|| {
        parse_vocab(FISH_VOCAB_RAW)
            .expect("FISH_VOCAB.txt is malformed; see its own header for the rules")
    });
    v.as_slice()
}

/// How many classes the fish model must output.
pub fn fish_vocab_len() -> usize {
    fish_vocab().len()
}

/// Fingerprint of the index -> label mapping.
///
/// MUST equal `sha256(classes.join("\n"))`, which is what `shadow_infer`
/// verifies against a deployed manifest and what `registry` records on every
/// candidate. It is deliberately *not* a fingerprint of the set: reordering is
/// the failure mode that matters and that a set hash would hide.
pub fn fish_vocab_sha() -> String {
    super::ml_model::sha256_hex(fish_vocab().join("\n").as_bytes())
}

/// The canonical vocabulary as a set, for set comparisons.
pub fn fish_vocab_set() -> BTreeSet<&'static str> {
    fish_vocab().iter().copied().collect()
}

/// Which canonical classes are missing from `present`.
///
/// The gate is phrased this way on purpose. "Are at least N classes
/// qualified?" is satisfiable by any N entities and was the original defect;
/// "is every canonical class qualified?" cannot be satisfied by substituting an
/// entity the model was never going to learn.
pub fn missing_from<'a>(present: impl IntoIterator<Item = &'a str>) -> Vec<&'static str> {
    let have: BTreeSet<&str> = present.into_iter().collect();
    fish_vocab()
        .iter()
        .copied()
        .filter(|c| !have.contains(c))
        .collect()
}

/// Canonical classes present in `present` that are NOT part of the target.
///
/// These are rows the resolver assigned to a real class id but which the model
/// will never be trained to distinguish. Seeing them is the signal that label
/// curation is incomplete - NOT something to resolve by adding classes.
pub fn not_in_target<'a>(present: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let want = fish_vocab_set();
    present.into_iter().filter(|c| !want.contains(c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vocabulary_parses_and_is_exactly_the_deployed_model() {
        let v = fish_vocab();
        assert!(!v.is_empty(), "vocabulary must not be empty");
        // The vocabulary is the 15 CONFIRMED SPECIES, not the 8 bare stems it
        // replaced. It used to assert the 8 stems inline here, which is exactly
        // the duplicated literal that let one vocabulary change land with a
        // stale test. It now asserts the PROPERTY that must hold - ascending id
        // order, one fish id per line, no duplicates - and the concrete class
        // list is verified against the deployed manifest below, which is the
        // thing the order actually has to agree with.
        let mut sorted = v.to_vec();
        sorted.sort_unstable();
        assert_eq!(
            v, sorted,
            "FISH_VOCAB.txt must be in ascending id order; that order IS the \
             output-logit index and must be defined by the file's own rule"
        );
        for c in v {
            assert!(c.starts_with("fish:"), "not a fish class id: {c}");
        }
        for m in ["fish_v1.json", "state_v1.json"] {
            let raw = std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("models")
                    .join(m),
            )
            .unwrap_or_default();
            if let Ok(vj) = serde_json::from_str::<serde_json::Value>(&raw) {
                if m == "fish_v1.json" {
                    let deployed: Vec<&str> = vj["classes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|x| x.as_str())
                        .collect();
                    assert_eq!(
                        deployed,
                        v.to_vec(),
                        "deployed fish_v1 class order diverges from the canonical vocabulary"
                    );
                }
            }
        }
    }

    #[test]
    fn the_deployed_manifest_pins_the_canonical_fingerprint() {
        // The bundled manifest carries `vocab_sha`; it must equal what this
        // module computes, or the class map is unpinned and a permutation
        // would be undetectable.
        let raw = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("models")
                .join("fish_v1.json"),
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            v["vocab_sha"].as_str().unwrap_or(""),
            fish_vocab_sha(),
            "bundled fish_v1.json must pin the canonical vocab fingerprint"
        );
    }

    #[test]
    fn a_permuted_vocabulary_changes_the_fingerprint() {
        // The whole point of pinning a fingerprint rather than a set: order is
        // load-bearing.
        let a = super::super::ml_model::sha256_hex("fish:a\nfish:b\nfish:c".as_bytes());
        let b = super::super::ml_model::sha256_hex("fish:b\nfish:a\nfish:c".as_bytes());
        assert_ne!(a, b, "a permuted class map must not hash the same");
    }

    #[test]
    fn the_gate_is_set_based_not_a_count() {
        let all = fish_vocab();
        assert!(
            missing_from(all.iter().copied()).is_empty(),
            "full vocabulary must pass"
        );

        // A DIFFERENT set of the same size must NOT pass. This is the exact
        // failure that produced `10/10` over an 8-class artifact.
        let impostors: Vec<&str> = (0..fish_vocab_len())
            .map(|i| match i {
                0 => "fish:polkaguffer",
                1 => "fish:zebraangel",
                _ => "fish:tiger",
            })
            .collect();
        assert_eq!(impostors.len(), fish_vocab_len());
        let missing = missing_from(impostors.iter().copied());
        assert_eq!(
            missing.len(),
            fish_vocab_len(),
            "an equal-sized but wrong set must not satisfy the gate"
        );
        assert!(!missing.is_empty());
    }

    #[test]
    fn a_partial_vocabulary_names_exactly_what_is_missing() {
        // Derived from the vocabulary itself, so this test does not pin a
        // class list that has to be edited again at the next vocabulary change.
        // Take two real classes and one id that is deliberately not a class.
        let vocab = fish_vocab();
        let real_a = vocab[0];
        let real_b = vocab[1];
        let partial: Vec<&str> = vec![real_a, real_b, "fish:not_a_class"];

        let missing = missing_from(partial.iter().copied());
        // Every canonical class except the two supplied ones.
        assert_eq!(missing.len(), fish_vocab_len() - 2);
        assert!(!missing.contains(&real_a));
        assert!(!missing.contains(&real_b));
        for c in vocab.iter().filter(|c| **c != real_a && **c != real_b) {
            assert!(missing.contains(c), "{c} should be reported missing");
        }
        let extra = not_in_target(partial.iter().copied());
        assert_eq!(
            extra,
            vec!["fish:not_a_class"],
            "unknown ids must be surfaced"
        );
    }

    #[test]
    fn malformed_vocabularies_are_rejected() {
        assert!(
            parse_vocab("").is_err(),
            "empty vocabulary must be an error"
        );
        assert!(parse_vocab("# only comments\n").is_err());
        assert!(
            parse_vocab("fish:a\nfish:a").is_err(),
            "a duplicate class is a silent label corruption; it must be an error"
        );
        assert!(
            parse_vocab("fruit:yami").is_err(),
            "a non-fish id in the fish vocabulary is a build defect"
        );
        assert!(parse_vocab("# c\n\nfish:a\nfish:b\n").is_ok());
    }

    #[test]
    fn the_vocabulary_is_the_single_source_for_count_and_set() {
        // Guards against someone reintroducing a scalar: the count and the set
        // must be the same fact viewed two ways.
        assert_eq!(fish_vocab_len(), fish_vocab_set().len());
        // The class COUNT was an 8 written into a test, which is how it stayed
        // green through a vocabulary change and turned red afterwards. What has
        // to hold is the invariant, not the number: the canonical list is the
        // confirmed-species ontology, every entry is a fish id, and the set and
        // the count are the same fact twice. The concrete list is asserted
        // against the DEPLOYED manifest in
        // `tests/deployed_shadow_model::the_bundled_pair_is_internally_consistent`.
        assert!(
            fish_vocab_len() >= 2,
            "a vocabulary of fewer than 2 classes is degenerate"
        );
        assert!(
            fish_vocab().iter().all(|c| c.starts_with("fish:")),
            "every canonical class must be a fish id"
        );
        assert!(
            !fish_vocab().contains(&"fish:anglerfish"),
            "anglerfish is documented but below the class gate; it re-enters \
             automatically at 20 rows / 3 sessions and must not be added to \
             reach a class count"
        );
        assert_eq!(fish_vocab_set(), fish_vocab().iter().copied().collect());
    }
}
