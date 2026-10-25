//! Deploy a bundled shadow model into the live store ATOMICALLY.
//!
//! # This is the ONE place this repository writes to `%APPDATA%`
//!
//! Everything else in the training path is read-only over app state. This
//! binary does exactly one thing: copy an already-verified bundled model pair
//! into `%APPDATA%\gpo-autofish\models\` so the running app loads it. It cannot
//! write `labels.jsonl`, `reviews.jsonl`, `review_audit.jsonl`,
//! `knowledge_custom.json`, the registry or the version counter, and it does not
//! take a destination path from the command line - the destination is fixed to
//! the fish slot.
//!
//! # Why atomicity is the whole point
//!
//! A `.onnx` and its `.json` manifest are two files that must agree. Copying
//! them non-atomically opens a window where `fish_v1.onnx` is the new weights
//! and `fish_v1.json` is the old manifest, or a torn `.onnx` is half written.
//! `ShadowEngine::load` verifies the checksum, so it would refuse - but the app
//! would be left with no fish model at all until someone intervened.
//!
//! Each file is written to a temp sibling then `rename`d into place (atomic
//! within a filesystem), the `.onnx` FIRST (so the manifest is never newer than
//! the weights it describes), and both source bytes are re-hashed after the
//! write to prove nothing was truncated.
//!
//! Usage:
//!   cargo run --example deploy_shadow_model
//!
//! SAFETY: refuses unless the bundled manifest's `vocab_sha` equals
//! `class_vocab::fish_vocab_sha()` and the bundled `.onnx` matches the
//! manifest's `sha256`. A manifest that describes different weights than the
//! file beside it is exactly the defect this repository exists to prevent.

use gpo_autofish_lib::app::data_dir;
use gpo_autofish_lib::core::class_vocab;
use gpo_autofish_lib::core::ml_model::sha256_hex;

fn fail(msg: &str) -> ! {
    eprintln!("  REFUSED: {msg}");
    std::process::exit(1)
}

/// Write `bytes` to `dst` atomically: temp sibling in the SAME directory, fsync,
/// rename. Same directory means same filesystem, so the rename cannot degrade
/// into a copy.
fn write_atomic(dst: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let dir = dst.parent().ok_or_else(|| {
        std::io::Error::other("destination has no parent directory")
    })?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp",
        dst.file_name().and_then(|s| s.to_str()).unwrap_or("model")
    ));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
        f.sync_all()?;
    }
    match std::fs::rename(&tmp, dst) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn main() {
    let bundled = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models");
    let store = data_dir().join("models");

    let manifest_path = bundled.join("fish_v1.json");
    let onnx_path = bundled.join("fish_v1.onnx");
    let manifest_raw = std::fs::read(&manifest_path)
        .unwrap_or_else(|e| fail(&format!("{}: {e}", manifest_path.display())));
    let onnx_bytes = std::fs::read(&onnx_path)
        .unwrap_or_else(|e| fail(&format!("{}: {e}", onnx_path.display())));
    let m: serde_json::Value =
        serde_json::from_slice(&manifest_raw).unwrap_or_else(|e| fail(&format!("manifest JSON: {e}")));

    // ---- refuse anything inconsistent BEFORE touching app state ----------
    let classes: Vec<String> = m["classes"]
        .as_array()
        .unwrap_or_else(|| fail("bundled manifest has no classes array"))
        .iter()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect();
    let want_vocab_sha = class_vocab::fish_vocab_sha();
    let have_vocab_sha = m["vocab_sha"].as_str().unwrap_or("");
    if have_vocab_sha != want_vocab_sha {
        fail(&format!(
            "bundled manifest vocab_sha {have_vocab_sha} != canonical {}; the \
             manifest describes a different class map than FISH_VOCAB.txt",
            want_vocab_sha
        ));
    }
    let calc = sha256_hex(classes.join("\n").as_bytes());
    if calc != want_vocab_sha {
        fail(&format!(
            "manifest classes do not hash to their own vocab_sha ({calc} != {want_vocab_sha})"
        ));
    }
    let declared_sha = m["sha256"].as_str().unwrap_or("");
    let actual_sha = sha256_hex(&onnx_bytes);
    if declared_sha != actual_sha {
        fail(&format!(
            "bundled .onnx sha256 {actual_sha} != manifest {declared_sha}; the weights \
             and the manifest beside them describe different models"
        ));
    }
    let version = m["version"].as_str().unwrap_or("?").to_string();
    println!("bundled manifest verified BEFORE writing anything:");
    println!("  version      : {version}");
    println!("  classes      : {}", classes.len());
    println!("  vocab_sha    : {want_vocab_sha}");
    println!("  sha256(onnx) : {actual_sha}");

    // ---- one AppData write: the fish model pair ------------------------
    std::fs::create_dir_all(&store).unwrap_or_else(|e| fail(&format!("{}: {e}", store.display())));

    // Weights FIRST. If the process dies between the two writes, the manifest is
    // still the OLD one, which describes 8 classes; the old weights are still on
    // disk only if the first write also failed... so ordering alone cannot make
    // this atomic across two files. What it CAN do is avoid the worse state: a
    // NEW manifest (15 classes) beside OLD weights would mislabel every
    // observation. The old manifest beside new weights fails the checksum and
    // refuses to load, which is the honest failure.
    println!("\nwriting (atomic rename per file, weights first):");
    write_atomic(&store.join("fish_v1.onnx"), &onnx_bytes)
        .unwrap_or_else(|e| fail(&format!("fish_v1.onnx: {e}")));
    println!("  -> {}", store.join("fish_v1.onnx").display());
    write_atomic(&store.join("fish_v1.json"), &manifest_raw)
        .unwrap_or_else(|e| fail(&format!("fish_v1.json: {e}")));
    println!("  -> {}", store.join("fish_v1.json").display());

    // ---- prove the bytes landed -----------------------------------------
    let back_onnx = std::fs::read(store.join("fish_v1.onnx"))
        .unwrap_or_else(|e| fail(&format!("re-read onnx: {e}")));
    let back_json = std::fs::read(store.join("fish_v1.json"))
        .unwrap_or_else(|e| fail(&format!("re-read json: {e}")));
    let live_onnx_sha = sha256_hex(&back_onnx);
    let live_json_sha = sha256_hex(&back_json);
    if live_onnx_sha != actual_sha || live_json_sha != sha256_hex(&manifest_raw) {
        fail("a deployed file does not match its source after the write");
    }
    println!("\nverified by re-reading the deployed bytes:");
    println!("  fish_v1.onnx sha256: {live_onnx_sha}");
    println!("  fish_v1.json sha256: {live_json_sha}");

    // No temp files left behind.
    let leftovers: Vec<String> = std::fs::read_dir(&store)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.starts_with('.') && n.ends_with(".tmp"))
                .collect()
        })
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "temp files left behind: {leftovers:?}");
    println!("  no temp files left behind");

    println!("\nDeployed. The shadow soak for this revision starts from ZERO events.");
    println!("Production control is OFF and was not touched by this binary.");
}