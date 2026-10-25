//! The DEPLOYED shadow model must load, and must not be silently replaced.
//!
//! These are the contracts `ShadowEngine` enforces on the bytes the running app
//! actually reads. They are pinned here because every one of them has a failure
//! mode that is silent rather than loud:
//!
//! * **checksum** - weights that are not the manifest's `sha256` are refused, so
//!   a half-written `.onnx` cannot run;
//! * **`vocab_sha`** - the class map is pinned and compared against
//!   `class_vocab::fish_vocab_sha()`, so a permuted or stale `classes` array is
//!   refused even though the weights would load fine;
//! * **slot/name** - a manifest that declares a different model than the slot it
//!   occupies is refused, which is what stops a fish manifest evicting the state
//!   entry;
//! * **output width** - inference must return exactly one probability per
//!   declared class, or the observation is dropped rather than mislabelled;
//! * **seed does not clobber** - `seed_shadow_models` on the next launch must
//!   leave a deployed model alone.
//!
//! The tamper cases run against a temp directory, so they are hermetic. The
//! "deployed" cases read the live store and SKIP when it is absent (a fresh
//! clone has no store), which is a first-run state, not a divergence.

#![cfg(windows)]

use gpo_autofish_lib::core::class_vocab;
use gpo_autofish_lib::core::shadow_infer::{seed_shadow_models, ShadowEngine};
use std::path::{Path, PathBuf};

fn live_models_dir() -> Option<PathBuf> {
    let root = match std::env::var("GPO_DATA_DIR") {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from(std::env::var("APPDATA").ok()?).join("gpo-autofish"),
    };
    let dir = root.join("models");
    dir.join("fish_v1.json").is_file().then_some(dir)
}

fn bundled_models_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models")
}

/// A directory holding copies of the bundled pair, so a tamper case cannot
/// touch the real ones.
fn sandbox(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gpo-shadow-{tag}-{}",
        gpo_autofish_lib::events::now_ms()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for f in ["fish_v1.onnx", "fish_v1.json"] {
        std::fs::copy(bundled_models_dir().join(f), dir.join(f))
            .unwrap_or_else(|e| panic!("copy {f}: {e}"));
    }
    dir
}

#[test]
fn the_deployed_engine_loads_and_answers_fifteen_classes() {
    let Some(dir) = live_models_dir() else {
        eprintln!(
            "SKIP: no live deployed models in {}",
            live_models_dir().unwrap_or_default().display()
        );
        return;
    };
    let engine = ShadowEngine::load(&dir)
        .unwrap_or_else(|e| panic!("ShadowEngine::load refused the deployed model: {e}"));
    assert!(
        engine.model_names().contains(&"fish_v1".to_string()),
        "the fish slot must be loaded; got {:?}",
        engine.model_names()
    );

    // A synthetic 96x96 frame: the output width is the contract under test.
    let spec_size = 96usize;
    let rgba = vec![128u8; spec_size * spec_size * 4];
    let spec = gpo_autofish_lib::core::shadow_infer::PreprocessSpec {
        size: spec_size,
        mean: [0.0, 0.0, 0.0],
        std: [1.0, 1.0, 1.0],
        pad: "square-black".into(),
        resize: "bilinear".into(),
    };
    let chw =
        gpo_autofish_lib::core::shadow_infer::preprocess_rgba(&rgba, spec_size, spec_size, &spec)
            .expect("preprocess must produce a tensor");
    assert_eq!(chw.len(), 3 * 96 * 96, "input fact is [1,3,96,96]");

    let probs = engine
        .infer_probs("fish_v1", &chw)
        .unwrap_or_else(|| panic!("inference returned nothing for a valid 96x96 input"));
    assert_eq!(
        probs.len(),
        class_vocab::fish_vocab_len(),
        "one probability per canonical class, or the observation must be dropped"
    );
    let sum: f32 = probs.iter().map(|(_, p)| *p).sum();
    assert!(
        (sum - 1.0).abs() < 1e-4,
        "probabilities must sum to 1, got {sum}"
    );
    let ids: Vec<&str> = probs.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(
        ids,
        class_vocab::fish_vocab(),
        "the deployed class map must be the canonical vocabulary, in order"
    );
}

#[test]
fn the_seeder_does_not_replace_a_deployed_model_with_the_bundled_one() {
    // The exact scenario from `seed_shadow_models`: the bundled model is offered
    // as a seed on the next launch. A deployed model must survive it.
    let dir = sandbox("seed");
    let deployed = PathBuf::from(std::env::var("APPDATA").unwrap_or_default())
        .join("gpo-autofish")
        .join("models");
    if deployed.join("fish_v1.json").is_file() {
        // Copy the LIVE deployed pair in, so this case is about a real
        // deployed model rather than two copies of the bundled one.
        for f in ["fish_v1.onnx", "fish_v1.json"] {
            std::fs::copy(deployed.join(f), dir.join(f)).unwrap();
        }
    }
    let before: Vec<u8> = std::fs::read(dir.join("fish_v1.json")).unwrap();
    let before_onnx: Vec<u8> = std::fs::read(dir.join("fish_v1.onnx")).unwrap();
    let before_version: serde_json::Value =
        serde_json::from_slice(&before).expect("deployed manifest must parse");

    // Offer the BUNDLED directory as the candidate source.
    let candidates = vec![bundled_models_dir()];
    seed_shadow_models(&candidates, &dir);

    let after: Vec<u8> = std::fs::read(dir.join("fish_v1.json")).unwrap();
    let after_onnx: Vec<u8> = std::fs::read(dir.join("fish_v1.onnx")).unwrap();
    let after_version: serde_json::Value = serde_json::from_slice(&after).unwrap();

    if before_version["version"] == after_version["version"]
        && before_version["sha256"] == after_version["sha256"]
    {
        assert_eq!(
            before, after,
            "identical seed must not rewrite the manifest"
        );
        assert_eq!(
            before_onnx, after_onnx,
            "identical seed must not rewrite the weights"
        );
    } else {
        // Versions genuinely differ, so the seeder is entitled to write - but
        // only DOWNWARD is forbidden. Never to an older manifest.
        let b = before_version["version"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok());
        let a = after_version["version"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok());
        assert!(
            match (a, b) {
                (Some(a), Some(b)) => a >= b,
                _ => true,
            },
            "seeding must never downgrade a deployed model: {:?} -> {:?}",
            before_version.get("version"),
            after_version.get("version")
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_manifest_that_disagrees_with_its_weights_is_refused() {
    // Checksum. Weights and manifest beside them that describe different models
    // must not load.
    let dir = sandbox("tamper-sha");
    let p = dir.join("fish_v1.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    m["sha256"] = serde_json::json!("0".repeat(64));
    std::fs::write(&p, serde_json::to_vec(&m).unwrap()).unwrap();
    let err = match ShadowEngine::load(&dir) {
        Ok(_) => panic!("a wrong checksum must refuse to load"),
        Err(e) => e,
    };
    assert!(
        err.to_lowercase().contains("sha") || err.to_lowercase().contains("checksum"),
        "the refusal must name the checksum: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_manifest_with_a_foreign_class_map_is_refused() {
    // `vocab_sha`. The weights are fine; the CLASS MAP is wrong. This is the
    // failure that is invisible in the artifact: a permuted `classes` array
    // loads perfectly and attributes every observation to the wrong label.
    let dir = sandbox("tamper-vocab");
    let p = dir.join("fish_v1.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    m["vocab_sha"] = serde_json::json!("f".repeat(64));
    std::fs::write(&p, serde_json::to_vec(&m).unwrap()).unwrap();
    let err = match ShadowEngine::load(&dir) {
        Ok(_) => panic!("a foreign class map must refuse to load"),
        Err(e) => e,
    };
    assert!(
        err.to_lowercase().contains("vocab") || err.to_lowercase().contains("class"),
        "the refusal must name the class map: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_manifest_in_the_wrong_slot_is_refused() {
    let dir = sandbox("tamper-slot");
    let p = dir.join("fish_v1.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    m["name"] = serde_json::json!("state_v1");
    std::fs::write(&p, serde_json::to_vec(&m).unwrap()).unwrap();
    let err = match ShadowEngine::load(&dir) {
        Ok(_) => panic!("a cross-slot manifest must refuse to load"),
        Err(e) => e,
    };
    assert!(
        err.contains("slot") || err.contains("cross slots"),
        "the refusal must name the slot rule: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_bundled_pair_is_internally_consistent() {
    // What ships in a release must satisfy the same contract as what runs.
    let dir = bundled_models_dir();
    ShadowEngine::load(&dir).expect("the bundled shadow models must load");
    let raw = std::fs::read(dir.join("fish_v1.json")).unwrap();
    let m: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(
        m["vocab_sha"].as_str().unwrap(),
        class_vocab::fish_vocab_sha(),
        "the bundled manifest must pin the canonical class-map fingerprint"
    );
    assert_eq!(
        m["sha256"].as_str().unwrap(),
        gpo_autofish_lib::core::ml_model::sha256_hex(
            &std::fs::read(dir.join("fish_v1.onnx")).unwrap()
        ),
        "the bundled manifest's sha256 must describe the bundled weights"
    );
    let classes: Vec<&str> = m["classes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    assert_eq!(
        classes,
        class_vocab::fish_vocab(),
        "the bundled class map must equal FISH_VOCAB.txt, in order"
    );
}

#[test]
fn shadow_inference_cannot_reach_a_game_input() {
    // Structural, not behavioural: the production path must have no edge from a
    // prediction to an input. `FINAL_FEATURE_FREEZE` §3.1 states this as an
    // ABSENCE, so it is asserted as one.
    let src = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/core/shadow_infer.rs"),
    )
    .unwrap();
    for forbidden in [
        "bot::actions",
        "platform::windows::input",
        "controller",
        "send_key",
        "click",
    ] {
        assert!(
            !src.contains(forbidden),
            "shadow_infer must not reference {forbidden}: a shadow observation has no \
             path to a game input"
        );
    }
    let _ = Path::new(".");
}
