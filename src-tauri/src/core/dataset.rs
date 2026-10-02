//! Learning dataset pipeline (collection + labeling, NO auto-training).
//!
//! When the user enables trace recording, uncertain observations can be
//! persisted as labeled-training-example candidates:
//!
//! ```text
//! screenshot (.png)
//! + detected region + OCR result + vision result
//! + game state + timestamp + confidence + event/entity type
//! → dataset/<id>.png + manifest.jsonl
//! ```
//!
//! Low-confidence samples are identifiable (`needs_label`), a user label
//! (`label_sample`) turns one into a training example, and NOTHING trains a
//! model automatically — training is a separate, future, explicitly-run step.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::core::perception::Observation;
use crate::core::types::Frame;

/// Maximum stored samples; oldest unlabeled samples are pruned first.
pub const MAX_SAMPLES: usize = 200;
/// Below this observation confidence a sample is flagged `needs_label`.
pub const NEEDS_LABEL_BELOW: f32 = 0.80;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetSample {
    pub id: String,
    pub timestamp_ms: u64,
    pub event_type: String,
    pub region: String,
    pub game_state: String,
    pub ocr_text: String,
    pub observation: Observation,
    pub needs_label: bool,
    pub label: Option<String>,
    pub label_correct: Option<bool>,
    pub png_file: String,
}

pub struct DatasetStore {
    dir: PathBuf,
}

impl DatasetStore {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { dir: data_dir.join("dataset") }
    }

    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    fn manifest_path(&self) -> PathBuf {
        self.dir.join("manifest.jsonl")
    }

    /// Persist one sample (frame PNG + manifest row). Returns the sample id,
    /// or `None` when storage fails (never panics, never blocks the caller
    /// with an error — dataset collection is best-effort by design).
    pub fn record(
        &self,
        event_type: &str,
        region: &str,
        game_state: &str,
        ocr_text: &str,
        observation: &Observation,
        frame_png: Option<Vec<u8>>,
    ) -> Option<String> {
        if std::fs::create_dir_all(&self.dir).is_err() {
            return None;
        }
        let id = format!("{:x}", observation.timestamp_ms);
        let png_file = format!("{id}.png");
        if let Some(bytes) = frame_png {
            if std::fs::write(self.dir.join(&png_file), &bytes).is_err() {
                return None;
            }
        }
        let sample = DatasetSample {
            id: id.clone(),
            timestamp_ms: observation.timestamp_ms,
            event_type: event_type.to_string(),
            region: region.to_string(),
            game_state: game_state.to_string(),
            ocr_text: ocr_text.to_string(),
            observation: observation.clone(),
            needs_label: observation.confidence < NEEDS_LABEL_BELOW,
            label: None,
            label_correct: None,
            png_file,
        };
        let mut line = serde_json::to_string(&sample).ok()?;
        line.push('\n');
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.manifest_path()).ok()?;
            f.write_all(line.as_bytes()).ok()?;
        }
        self.prune();
        Some(id)
    }

    /// All samples, oldest first. Corrupt manifest rows are skipped, never
    /// fatal (a broken row must not hide the rest of the dataset).
    pub fn list(&self) -> Vec<DatasetSample> {
        let Ok(content) = std::fs::read_to_string(self.manifest_path()) else {
            return Vec::new();
        };
        content
            .lines()
            .filter_map(|l| serde_json::from_str::<DatasetSample>(l).ok())
            .collect()
    }

    pub fn needs_label(&self) -> Vec<DatasetSample> {
        self.list().into_iter().filter(|s| s.needs_label && s.label.is_none()).collect()
    }

    /// Attach a user label to a sample. `correct` records whether the
    /// observation's top candidate was right (`Confirm`) or not (`Correct`
    /// with the true label). Rewrites the manifest atomically.
    pub fn label(&self, id: &str, label: &str, correct: bool) -> Result<DatasetSample, String> {
        let label = label.trim();
        if label.is_empty() {
            return Err("Label cannot be empty".to_string());
        }
        let mut samples = self.list();
        let updated = {
            let s = samples
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or_else(|| format!("Dataset sample '{id}' not found"))?;
            s.label = Some(label.to_string());
            s.label_correct = Some(correct);
            s.needs_label = false;
            s.clone()
        };
        let out: String = samples
            .iter()
            .filter_map(|s| serde_json::to_string(s).ok())
            .map(|mut l| {
                l.push('\n');
                l
            })
            .collect();
        let tmp = self.dir.join("manifest.jsonl.tmp");
        std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, self.manifest_path()).map_err(|e| e.to_string())?;
        Ok(updated)
    }

    fn prune(&self) {
        let samples = self.list();
        if samples.len() <= MAX_SAMPLES {
            return;
        }
        // Keep labeled samples; drop oldest unlabeled first.
        let (mut keep_labeled, mut unlabeled): (Vec<_>, Vec<_>) =
            samples.into_iter().partition(|s| s.label.is_some());
        unlabeled.sort_by_key(|s| s.timestamp_ms);
        let drop_n = keep_labeled.len() + unlabeled.len() - MAX_SAMPLES;
        let drop_ids: std::collections::HashSet<String> =
            unlabeled.iter().take(drop_n).map(|s| s.id.clone()).collect();
        for id in &drop_ids {
            let _ = std::fs::remove_file(self.dir.join(format!("{id}.png")));
        }
        keep_labeled.extend(unlabeled.into_iter().filter(|s| !drop_ids.contains(&s.id)));
        keep_labeled.sort_by_key(|s| s.timestamp_ms);
        let out: String = keep_labeled
            .iter()
            .filter_map(|s| serde_json::to_string(s).ok())
            .map(|mut l| {
                l.push('\n');
                l
            })
            .collect();
        let _ = std::fs::write(self.manifest_path(), out);
    }

    /// Build a sample frame payload from a live frame (PNG bytes), if any.
    pub fn png_of(frame: &Frame) -> Option<Vec<u8>> {
        frame.to_png_bytes().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::knowledge::KnowledgeBase;
    use crate::core::perception::{correlate_text, ScreenKind};

    fn store(name: &str) -> (DatasetStore, PathBuf) {
        // Unique per test: the harness runs tests in parallel threads.
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("gpo-dataset-{}-{}-{n}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        (DatasetStore::new(dir.clone()), dir)
    }

    fn obs(confirmed: bool) -> Observation {
        let text = if confirmed { "Suna fruit" } else { "xqz wobble" };
        correlate_text(KnowledgeBase::bundled(), text, "drop", ScreenKind::Fishing, 0.85, 0.80, None)
    }

    #[test]
    fn record_and_list_round_trip() {
        let (ds, _dir) = store("t");
        let o = obs(true);
        let id = ds.record("drop", "drop", "tracking", "Suna fruit", &o, Some(vec![1, 2, 3])).expect("record");
        let list = ds.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].ocr_text, "Suna fruit");
        assert!(!list[0].needs_label);
        assert!(list[0].label.is_none());
    }

    #[test]
    fn low_confidence_samples_are_flagged() {
        let (ds, _dir) = store("t");
        let o = obs(false);
        assert!(o.entity.is_none());
        ds.record("drop", "drop", "tracking", "xqz wobble", &o, None).expect("record");
        let flagged = ds.needs_label();
        assert_eq!(flagged.len(), 1);
    }

    #[test]
    fn labeling_confirms_or_corrects() {
        let (ds, _dir) = store("t");
        let o = obs(false);
        let id = ds.record("drop", "drop", "tracking", "xqz wobble", &o, None).expect("record");
        let labeled = ds.label(&id, "Kraken", false).expect("label");
        assert_eq!(labeled.label.as_deref(), Some("Kraken"));
        assert_eq!(labeled.label_correct, Some(false));
        assert!(ds.needs_label().is_empty());
        assert!(ds.label("missing", "X", true).is_err());
        assert!(ds.label(&id, "   ", true).is_err());
    }

    #[test]
    fn corrupt_manifest_rows_are_skipped_not_fatal() {
        let (ds, dir) = store("t");
        let o = obs(true);
        ds.record("drop", "drop", "tracking", "Suna fruit", &o, None).expect("record");
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(dir.join("dataset").join("manifest.jsonl")).unwrap();
            writeln!(f, "{{not valid json").unwrap();
        }
        assert_eq!(ds.list().len(), 1);
    }

    #[test]
    fn prune_keeps_labeled_samples() {
        let (ds, _dir) = store("t");
        for i in 0..(MAX_SAMPLES + 10) {
            let mut o = obs(i % 2 == 0);
            o.timestamp_ms = 1000 + i as u64;
            ds.record("drop", "drop", "tracking", "t", &o, None).expect("record");
        }
        assert!(ds.list().len() <= MAX_SAMPLES);
    }
}
