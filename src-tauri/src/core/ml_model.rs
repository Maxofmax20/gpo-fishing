//! GPO vision model management: versioning, verification, rollback.
//!
//! Status: the provider, manifest format, checksum verification, and
//! rollback machinery are IMPLEMENTED and tested. No trained weights ship
//! with this codebase, and no ML framework is linked (a heavy ORT
//! dependency without a model would be unjustified):
//!
//! ```text
//! MODEL TRAINING BLOCKED: insufficient real labeled GPO data
//! ```
//!
//! Selected future runtime: ONNX Runtime (Windows CPU/GPU, portable `.onnx`,
//! small footprint, reproducible). See `docs/ML_TRAINING.md` for the training
//! contract this manager will load.
//!
//! Fallback chain (never leave the bot unusable):
//! loaded model → previous validated model → OCR + heuristic perception.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::core::types::Frame;
use crate::core::vision_provider::{RelBox, VisionObservation, VisionProvider};

/// On-disk model manifest (`models/<name>.json` next to `<name>.onnx`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelManifest {
    pub name: String,
    pub version: String,
    pub dataset: String,
    pub dataset_version: u32,
    pub trained_at: String,
    /// Inference runtime id. Only `"none"` (no weights) loads today;
    /// `"ort"` requires an ORT-linked build (see docs/ML_TRAINING.md).
    pub runtime: String,
    pub input_width: u32,
    pub input_height: u32,
    pub classes: Vec<String>,
    /// Lowercase hex sha256 of the weights file (`<name>.onnx`).
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub enum ModelLoadError {
    MissingManifest(String),
    InvalidManifest(String),
    MissingWeights(String),
    ChecksumMismatch { expected: String, actual: String },
    UnsupportedRuntime(String),
}

impl std::fmt::Display for ModelLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelLoadError::MissingManifest(m) => write!(f, "model manifest missing: {m}"),
            ModelLoadError::InvalidManifest(m) => write!(f, "invalid model manifest: {m}"),
            ModelLoadError::MissingWeights(m) => write!(f, "model weights missing: {m}"),
            ModelLoadError::ChecksumMismatch { expected, actual } => {
                write!(f, "model checksum mismatch: expected {expected}, got {actual}")
            }
            ModelLoadError::UnsupportedRuntime(m) => write!(f, "unsupported model runtime: {m}"),
        }
    }
}

/// A validated, loadable model reference (weights verified by checksum).
/// Inference sessions attach here once an ORT-linked build exists; until
/// then the provider reports availability honestly and yields nothing.
#[derive(Debug, Clone)]
pub struct LoadedModel {
    pub manifest: ModelManifest,
    pub weights_path: PathBuf,
}

/// Select the model to use: `current` first, previous validated on failure.
/// Returns `None` when no usable model exists (caller falls back to
/// OCR + heuristics — never an error for the bot).
pub fn select_model(models_dir: PathBuf) -> Option<LoadedModel> {
    let current = models_dir.join("current.json");
    if let Some(m) = try_load(&current) {
        return Some(m);
    }
    let previous = models_dir.join("previous.json");
    if let Some(m) = try_load(&previous) {
        tracing::warn!("ML model: current unavailable, fell back to previous validated model");
        return Some(m);
    }
    None
}

fn try_load(manifest_path: &PathBuf) -> Option<LoadedModel> {
    let manifest = load_manifest(manifest_path).ok()?;
    let weights = manifest_path.with_file_name(format!("{}.onnx", manifest.name));
    verify_weights(&manifest, &weights).ok()?;
    if manifest.runtime != "none" {
        tracing::warn!(
            "ML model '{}' needs runtime '{}' which is not linked into this build; ignoring",
            manifest.name,
            manifest.runtime
        );
        return None;
    }
    Some(LoadedModel { manifest, weights_path: weights })
}

pub fn load_manifest(path: &PathBuf) -> Result<ModelManifest, ModelLoadError> {
    let raw =
        std::fs::read_to_string(path).map_err(|_| ModelLoadError::MissingManifest(path.display().to_string()))?;
    let m: ModelManifest =
        serde_json::from_str(&raw).map_err(|e| ModelLoadError::InvalidManifest(e.to_string()))?;
    if m.name.trim().is_empty() {
        return Err(ModelLoadError::InvalidManifest("empty name".into()));
    }
    if m.input_width == 0 || m.input_height == 0 {
        return Err(ModelLoadError::InvalidManifest("input dimensions must be > 0".into()));
    }
    if m.classes.is_empty() {
        return Err(ModelLoadError::InvalidManifest("classes must not be empty".into()));
    }
    if m.sha256.len() != 64 || !m.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ModelLoadError::InvalidManifest("sha256 must be 64 hex chars".into()));
    }
    Ok(m)
}

pub fn sha256_file(path: &PathBuf) -> Result<String, ModelLoadError> {
    // No sha2 dependency: shell out to nothing — use a compact pure-Rust
    // SHA-256 (public domain algorithm, ~60 lines) so checksums stay honest
    // without new crates.
    let bytes =
        std::fs::read(path).map_err(|_| ModelLoadError::MissingWeights(path.display().to_string()))?;
    Ok(sha256_hex(&bytes))
}

pub fn verify_weights(manifest: &ModelManifest, weights: &PathBuf) -> Result<(), ModelLoadError> {
    let actual = sha256_file(weights)?;
    if !actual.eq_ignore_ascii_case(&manifest.sha256) {
        return Err(ModelLoadError::ChecksumMismatch { expected: manifest.sha256.clone(), actual });
    }
    Ok(())
}

// --- Minimal SHA-256 (FIPS 180-4). Used only for model checksums. --------

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    h.iter().map(|w| format!("{w:08x}")).collect()
}

/// Real ML vision provider shell: loads and verifies a versioned model when
/// one exists, yields its observations when inference is available, and
/// yields NOTHING (honest absence) otherwise. Without an ORT-linked build
/// and real weights, `available()` is false and detection is empty — the bot
/// continues on OCR + heuristics.
#[derive(Debug)]
pub struct GpoMlProvider {
    models_dir: PathBuf,
    loaded: Option<LoadedModel>,
    unavailable_reason: Option<String>,
}

impl GpoMlProvider {
    pub fn load(models_dir: PathBuf) -> Self {
        match select_model(models_dir.clone()) {
            Some(m) => Self { models_dir, loaded: Some(m), unavailable_reason: None },
            None => Self {
                models_dir,
                loaded: None,
                unavailable_reason: Some(
                    "no verified model in models/ (see docs/ML_TRAINING.md)".to_string(),
                ),
            },
        }
    }

    pub fn available(&self) -> bool {
        self.loaded.is_some()
    }

    pub fn models_dir(&self) -> &std::path::Path {
        &self.models_dir
    }

    pub fn model_info(&self) -> Option<ModelManifest> {
        self.loaded.as_ref().map(|l| l.manifest.clone())
    }

    pub fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable_reason.as_deref()
    }

    /// ROI preprocessing for future inference: crop + scale to model input
    /// dims. Real code path (used by tests); inference attaches here later.
    pub fn preprocess(&self, frame: &Frame, roi: Option<RelBox>) -> Option<Frame> {
        let m = self.loaded.as_ref()?;
        let cropped = match roi {
            Some(r) => {
                let x = (r.x.clamp(0.0, 1.0) * frame.w as f32) as usize;
                let y = (r.y.clamp(0.0, 1.0) * frame.h as f32) as usize;
                let w = ((r.w.clamp(0.0, 1.0) * frame.w as f32) as usize).max(1);
                let h = ((r.h.clamp(0.0, 1.0) * frame.h as f32) as usize).max(1);
                frame.crop(x, y, w, h)
            }
            None => frame.clone(),
        };
        // Scale longest side to model input (simple proportional fit).
        let target = m.manifest.input_width.max(m.manifest.input_height) as usize;
        if target == 0 {
            return None;
        }
        Some(cropped.downscale(target))
    }
}

impl VisionProvider for GpoMlProvider {
    fn name(&self) -> &'static str {
        "gpo_ml"
    }

    fn detect(&self, _frame: &Frame) -> Vec<VisionObservation> {
        // No inference runtime is linked into this build: honest absence.
        // With an ORT-linked build, verified weights would be classified
        // here and observations returned with measured scores.
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(name: &str, runtime: &str, sha: &str) -> ModelManifest {
        ModelManifest {
            name: name.into(),
            version: "0.1.0".into(),
            dataset: "gpo-vision".into(),
            dataset_version: 1,
            trained_at: "2026-01-01".into(),
            runtime: runtime.into(),
            input_width: 320,
            input_height: 320,
            classes: vec!["fishing_bar".into(), "bait_menu".into()],
            sha256: sha.into(),
        }
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn manifest_validation_rejects_garbage() {
        let dir = std::env::temp_dir().join("gpo-ml-manifest");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("m.json");
        std::fs::write(&p, b"{nope").unwrap();
        assert!(matches!(load_manifest(&p), Err(ModelLoadError::InvalidManifest(_))));
        let mut m = manifest("gpo-v1", "none", &"0".repeat(64));
        m.classes.clear();
        std::fs::write(&p, serde_json::to_string(&m).unwrap()).unwrap();
        assert!(matches!(load_manifest(&p), Err(ModelLoadError::InvalidManifest(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checksum_mismatch_is_detected() {
        let dir = std::env::temp_dir().join("gpo-ml-checksum");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let w = dir.join("gpo-v1.onnx");
        std::fs::write(&w, b"fake-weights").unwrap();
        let mut m = manifest("gpo-v1", "none", &sha256_hex(b"different-bytes"));
        m.name = "gpo-v1".into();
        assert!(matches!(
            verify_weights(&m, &w),
            Err(ModelLoadError::ChecksumMismatch { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn select_model_falls_back_and_prefers_current() {
        let dir = std::env::temp_dir().join("gpo-ml-select");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Nothing present → None (bot falls back to OCR+heuristics).
        assert!(select_model(dir.clone()).is_none());
        // Only previous, runtime none, valid checksum → loads with warning.
        let weights = b"model-bytes-v0";
        std::fs::write(dir.join("gpo-v0.onnx"), weights).unwrap();
        let mut m = manifest("gpo-v0", "none", &sha256_hex(weights));
        m.name = "gpo-v0".into();
        std::fs::write(dir.join("previous.json"), serde_json::to_string(&m).unwrap()).unwrap();
        let sel = select_model(dir.clone()).expect("previous must load");
        assert_eq!(sel.manifest.version, "0.1.0");
        // Corrupt current → still falls back to previous, never fails hard.
        std::fs::write(dir.join("current.json"), b"{broken").unwrap();
        assert!(select_model(dir.clone()).is_some());
        // Unsupported runtime is refused even with valid checksum.
        let mut m2 = manifest("gpo-v1", "ort", &sha256_hex(weights));
        m2.name = "gpo-v1".into();
        std::fs::write(dir.join("gpo-v1.onnx"), weights).unwrap();
        std::fs::write(dir.join("current.json"), serde_json::to_string(&m2).unwrap()).unwrap();
        let sel2 = select_model(dir.clone()).expect("previous fallback");
        assert_eq!(sel2.manifest.name, "gpo-v0");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn provider_without_model_is_honestly_unavailable() {
        let dir = std::env::temp_dir().join("gpo-ml-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = GpoMlProvider::load(dir.clone());
        assert!(!p.available());
        assert!(p.model_info().is_none());
        assert!(p.unavailable_reason().is_some());
        let f = Frame::new(10, 10, vec![0u8; 400]);
        assert!(p.detect(&f).is_empty(), "no model → no predictions, never fakes");
        assert!(p.preprocess(&f, None).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preprocess_crops_and_scales_to_input_dims() {
        let dir = std::env::temp_dir().join("gpo-ml-pre");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let weights = b"w";
        std::fs::write(dir.join("gpo-v0.onnx"), weights).unwrap();
        let mut m = manifest("gpo-v0", "none", &sha256_hex(weights));
        m.name = "gpo-v0".into();
        m.input_width = 160;
        m.input_height = 160;
        std::fs::write(dir.join("current.json"), serde_json::to_string(&m).unwrap()).unwrap();
        let p = GpoMlProvider::load(dir.clone());
        assert!(p.available());
        let f = Frame::new(320, 240, vec![9u8; 320 * 240 * 4]);
        let out = p.preprocess(&f, None).expect("preprocess");
        assert!(out.w.max(out.h) <= 160);
        let roi = RelBox { x: 0.25, y: 0.25, w: 0.5, h: 0.5 };
        let out2 = p.preprocess(&f, Some(roi)).expect("roi");
        assert!(out2.w <= 160 && out2.h <= 160);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
