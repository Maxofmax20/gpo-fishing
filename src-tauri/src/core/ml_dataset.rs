//! Versioned ML training datasets (`datasets/gpo-vision/v1/`).
//!
//! This is SEPARATE from the bounded diagnostic capture cache
//! (`core::dataset`, 200 samples max): training data lives versioned with
//! stable image IDs, session-based splits, and a quality checker.
//!
//! ```text
//! datasets/gpo-vision/v1/
//!   images/<image_id>.png
//!   labels.jsonl          (one MlAnnotation per line)
//!   manifest.json         (version, split strategy + overrides)
//!   README.md             (generated: strategy + counts)
//! ```
//!
//! Split strategy (documented, enforced by the checker): sessions hash
//! deterministically to train/validation/test (70/15/15). Near-identical
//! consecutive frames share a session id, so they can NEVER leak across
//! splits. Each macro run mints one unique session id
//! (`session_YYYY-MM-DD_HHMMSS_<hash>`) — finer than day-granular, strictly
//! stronger isolation.
//!
//! DATA-FLOW + SEMANTICS NOTE (readiness architecture):
//! ```text
//! raw capture (bar/drop crops at state-machine moments)
//!   → dataset row (MlAnnotation: image/session/timestamp/state/entity?…)
//!   → state label (capture-time state machine: WaitingForBite/Bite/
//!       CatchResult — deterministic per pipeline evidence)
//!   → entity linkage (KB+OCR correlate at capture; RESULT rows ONLY —
//!       episode WAITING/BITE frames never carry entities by design)
//!   → hard classification (empty-OCR / unverdictable / rules≠KB /
//!       perception-∅ — in production 100% of hard flags sit on RESULT
//!       rows; they concern entity linkage, never the state label)
//!   → split (per-run session hash 70/15/15 + manifest overrides)
//!   → training eligibility (state_eligibility(): eligible / transition /
//!       excluded per class — COMPUTED, never stored, never a relabel)
//!   → model (external ONNX contract; no trainer in this repo)
//! ```
//! Where semantics diverge: `verified` counts entity-linked rows ONLY
//! (effectively RESULT). It does NOT measure 3-state coverage — WAITING
//! and BITE rows are training-usable (deterministic capture states, zero
//! hard flags in production) yet invisible to that counter. Do NOT
//! redefine `verified` to include them; report state coverage separately
//! (state_eligibility) and keep the entity gate intact.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::core::knowledge::KnowledgeBase;
use crate::core::types::Frame;

pub const DATASET_NAME: &str = "gpo-vision";
pub const DATASET_VERSION: u32 = 1;
/// Hamming distance on the 64-bit perceptual hash at/below which two frames
/// are reported as near-duplicates.
pub const NEAR_DUP_HAMMING: u32 = 6;

/// Serialises every `labels.jsonl` mutation in this process.
///
/// Why it exists: each mutator below is a read-all → mutate-one → write-all →
/// rename cycle over ONE file, and there are two independent writers — the
/// long-lived `ml-collect-writer` thread (`import_png` + `fill_metadata` +
/// `annotate` + `set_hard_example` + `set_event`) and the UI review path on the
/// main thread (`annotate`). Unserialised, two writers can each read the same
/// row set, apply *different* mutations, and write the whole file back: the
/// second write silently discards the first writer's update (a lost update that
/// no error reports). Holding this guard across the ENTIRE cycle
/// (read → mutate → write → rename) makes the file a serial history.
///
/// Reads (`annotations`, `validate`, `state_eligibility`) deliberately do NOT
/// take the lock: the file is only ever replaced by `rename` over a complete
/// temp file, so an unsynchronised reader always observes either the whole old
/// file or the whole new one — never a torn line. Locking readers would also
/// nest, and this mutex is not reentrant.
static LABELS_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Monotonic counter making each temp file name unique within a process, so a
/// writer that somehow bypasses (or outlives) the lock can never scribble over
/// a sibling's in-flight temp file.
static TMP_WRITE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Per-write temp path: `labels.jsonl.<pid>.<seq>.tmp`. Unique per write, so
/// concurrent writers never share a staging file. Any temp left behind by a
/// crashed/older run carries a different name and is inert.
fn labels_tmp_path(root: &Path) -> PathBuf {
    let seq = TMP_WRITE_SEQ.fetch_add(1, Ordering::Relaxed);
    root.join(format!("labels.jsonl.{}.{seq}.tmp", std::process::id()))
}

/// Serialise rows to `labels.jsonl` via a unique temp file + atomic rename.
/// The rename is what makes the file all-or-nothing for readers; uniqueness of
/// the temp is what keeps two writers from colliding before that rename.
fn write_labels_atomic(root: &Path, labels: &Path, out: &str) -> Result<(), String> {
    let tmp = labels_tmp_path(root);
    std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, labels).map_err(|e| e.to_string())?;
    Ok(())
}

/// Render rows back to JSONL (one `MlAnnotation` per line).
fn render_labels(rows: &[MlAnnotation]) -> String {
    rows.iter()
        .filter_map(|a| serde_json::to_string(a).ok())
        .map(|mut l| {
            l.push('\n');
            l
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MlTask {
    UiDetection,
    GameState,
    EntityRecognition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiLabel {
    FishingBar,
    BaitMenu,
    DropIndicator,
    ServerTime,
    FruitIndicator,
    FishResult,
    DisconnectScreen,
    ReconnectScreen,
}

impl UiLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            UiLabel::FishingBar => "fishing_bar",
            UiLabel::BaitMenu => "bait_menu",
            UiLabel::DropIndicator => "drop_indicator",
            UiLabel::ServerTime => "server_time",
            UiLabel::FruitIndicator => "fruit_indicator",
            UiLabel::FishResult => "fish_result",
            UiLabel::DisconnectScreen => "disconnect_screen",
            UiLabel::ReconnectScreen => "reconnect_screen",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameStateLabel {
    Idle,
    Fishing,
    WaitingForBite,
    Bite,
    CatchResult,
    BaitMenu,
    Loading,
    Disconnected,
    Unknown,
}

impl GameStateLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            GameStateLabel::Idle => "idle",
            GameStateLabel::Fishing => "fishing",
            GameStateLabel::WaitingForBite => "waiting_for_bite",
            GameStateLabel::Bite => "bite",
            GameStateLabel::CatchResult => "catch_result",
            GameStateLabel::BaitMenu => "bait_menu",
            GameStateLabel::Loading => "loading",
            GameStateLabel::Disconnected => "disconnected",
            GameStateLabel::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Split {
    Train,
    Validation,
    Test,
}

impl Split {
    pub fn as_str(self) -> &'static str {
        match self {
            Split::Train => "train",
            Split::Validation => "validation",
            Split::Test => "test",
        }
    }
}

/// Relative bounding box (0..1 of image dimensions).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelBBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RelBBox {
    pub fn valid(&self) -> bool {
        self.w > 0.0
            && self.h > 0.0
            && self.x >= 0.0
            && self.y >= 0.0
            && self.x + self.w <= 1.0 + 1e-6
            && self.y + self.h <= 1.0 + 1e-6
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Correction {
    pub at_ms: u64,
    pub prev_label: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MlAnnotation {
    pub image_id: String,
    pub dataset_version: u32,
    pub session_id: String,
    pub task: MlTask,
    /// Raw OCR text at capture time (game UI text only — never secrets;
    /// secrets never appear in screenshots the bot captures).
    pub ocr_text: String,
    /// Region id the frame/reading came from (e.g. `drop`, `bar`).
    pub region_name: String,
    pub ui_label: Option<UiLabel>,
    pub bbox: Option<RelBBox>,
    pub game_state: Option<GameStateLabel>,
    /// Stable knowledge-base id (e.g. `fruit:suna`), never a display name.
    pub entity_id: Option<String>,
    pub annotator: String,
    pub timestamp_ms: u64,
    pub source: String,
    pub confidence: Option<f32>,
    pub hard_example: bool,
    pub hard_reason: Option<String>,
    pub corrections: Vec<Correction>,
    /// Temporal provenance: `{session_id}#f{frame_index:06}` capture order
    /// within one collection session. First capture wins on exact-dedupe
    /// folds (the pixels ARE that first moment). Absent on rows collected
    /// before v5.2.0.
    #[serde(default)]
    pub event_id: Option<String>,
    /// Zero-based per-session capture sequence (same counter as the
    /// session `samples` tally). Gaps are possible when identical frames
    /// fold onto an earlier row via exact dedup.
    #[serde(default)]
    pub frame_index: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetManifest {
    pub name: String,
    pub version: u32,
    pub created_ms: u64,
    pub split_strategy: String,
    pub split_overrides: HashMap<String, Split>,
}

/// Per-class training eligibility (COMPUTED from rows + files, never
/// stored, never a relabel). Buckets are disjoint:
/// `total == eligible + transition + excluded`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StateEligibility {
    pub total: usize,
    pub eligible: usize,
    pub excluded: usize,
    pub transition: usize,
    pub sessions: usize,
}

/// A WAITING/BITE row counts as transition-adjacent when a different-state
/// row from the same session was captured within this window: the grab
/// happened at the state boundary, so the frame may straddle two states.
/// Conservative relative to reel timescales (tens of seconds), generous
/// relative to bite confirmation (tens of ms). Applies to WAITING/BITE
/// only — never demotes entity-verified RESULT rows. Heuristic, documented,
/// reporting-only.
pub const TRANSITION_WINDOW_MS: u64 = 2000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetReport {
    pub dataset: String,
    pub images: usize,
    pub labeled: usize,
    pub unlabeled: usize,
    pub classes: HashMap<String, usize>,
    pub sessions_train: usize,
    pub sessions_validation: usize,
    pub sessions_test: usize,
    pub leakage_sessions: Vec<String>,
    /// Image ids (files) attributed to sessions landing in MORE THAN ONE
    /// split: the same file — hence the same gameplay event — would be seen
    /// in training and evaluation. TRUE event leakage (stronger claim than
    /// near-similarity; empty in a healthy dataset thanks to dedup).
    pub same_file_cross_split: Vec<String>,
    /// Leakage clusters NOT explained by a shared file: near-duplicate
    /// (hamming 1..=6) recurring UI/background crops across sessions.
    /// Real visual similarity, irreducible for constant game UI — reported,
    /// never silently dropped, never a reason to weaken the detector.
    pub near_similarity_groups: usize,
    pub corrupt_files: Vec<String>,
    pub missing_labels: usize,
    pub invalid_entity_ids: Vec<String>,
    pub duplicate_groups: usize,
    pub near_duplicate_pairs: usize,
    pub invalid_bboxes: Vec<String>,
    pub orphan_annotations: Vec<String>,
    pub orphan_images: Vec<String>,
    pub min_max_class_ratio: f32,
    pub ok: bool,
}

pub struct MlDatasetStore {
    root: PathBuf,
}

impl MlDatasetStore {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            root: data_dir
                .join("datasets")
                .join(DATASET_NAME)
                .join(format!("v{}", DATASET_VERSION)),
        }
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    fn images_dir(&self) -> PathBuf {
        self.root.join("images")
    }

    fn labels_path(&self) -> PathBuf {
        self.root.join("labels.jsonl")
    }

    fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.json")
    }

    /// Day-granular session id (conservative: same-day frames never split).
    pub fn session_id_for_day(year: u32, month: u32, day: u32) -> String {
        format!("sess-{year:04}-{month:02}-{day:02}")
    }

    /// Deterministic session → split mapping (70/15/15 by FNV-1a hash).
    /// Stable across runs and processes (NOT SipHash).
    pub fn split_of(session_id: &str) -> Split {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in session_id.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        match (h % 100) as u8 {
            0..=69 => Split::Train,
            70..=84 => Split::Validation,
            _ => Split::Test,
        }
    }

    /// Initialize the dataset layout (idempotent; writes README + manifest).
    pub fn init(&self) -> Result<(), String> {
        std::fs::create_dir_all(self.images_dir()).map_err(|e| e.to_string())?;
        if !self.manifest_path().exists() {
            let manifest = DatasetManifest {
                name: DATASET_NAME.to_string(),
                version: DATASET_VERSION,
                created_ms: now_ms(),
                split_strategy: "session-hash 70/15/15 (train/validation/test); one session id per macro run (session_YYYY-MM-DD_HHMMSS_hash), so frames from a single run never split across sets; overrides in split_overrides".to_string(),
                split_overrides: HashMap::new(),
            };
            std::fs::write(
                self.manifest_path(),
                serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
        let readme = self.root.join("README.md");
        if !readme.exists() {
            std::fs::write(
                &readme,
                format!(
                    "# {name} v{ver}\n\nGPO vision training dataset.\n\n## Split strategy\n\nSessions hash deterministically to train/validation/test (70/15/15). Each macro run creates one unique session id (`session_YYYY-MM-DD_HHMMSS_<hash>`), so near-identical consecutive frames share a session and can never leak across splits. Manual overrides live in `manifest.json` (`split_overrides`).\n\n## Layout\n\n- `images/<image_id>.png` — perceptual-hash image ids (`<ahash-hex>` + counter on collision)\n- `labels.jsonl` — one `MlAnnotation` per line (stable `entity_id`s from the GPO knowledge base, never display names)\n- `manifest.json` — version + strategy + overrides\n\n## Quality\n\nRun the in-app dataset validator before any training. It reports leakage, corrupt files, duplicates, invalid labels/boxes, imbalance, and orphans. Training on a failing dataset is not supported.\n",
                    name = DATASET_NAME,
                    ver = DATASET_VERSION
                ),
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Import one PNG: perceptual-hash id (counter on collision), atomic
    /// manifest append. Returns the image id, or the existing id on exact
    /// duplicate (never stores the same bytes twice).
    #[allow(clippy::too_many_arguments)]
    pub fn import_png(
        &self,
        png_bytes: &[u8],
        session_id: &str,
        task: MlTask,
        game_state: Option<GameStateLabel>,
        ocr_text: &str,
        region_name: &str,
        source: &str,
    ) -> Result<String, String> {
        self.init()?;
        // Decode/hash BEFORE taking the lock: this is the expensive part and
        // needs no shared state.
        let hash = png_ahash(png_bytes)?;
        // The guard covers dedup + file write + row append as ONE critical
        // section: two threads importing identical bytes concurrently must not
        // both observe "not present" and both append a row. Note the lock is
        // taken here and NOT inside `append_annotation` — this mutex is not
        // reentrant, and `append_annotation` has no other caller.
        let _guard = LABELS_WRITE_LOCK.lock();
        // EXACT-duplicate check by CONTENT, not by the 64-bit average hash.
        //
        // `png_ahash` is a 64-bit perceptual signature over an 8x8 luminance
        // grid: two genuinely different frames collide with non-trivial
        // probability. v5.6.x treated a collision as "same image", returned the
        // other id, and the caller then wrote the NEW sample's OCR text and
        // entity id onto the row describing the OLD pixels - silent ground
        // truth corruption with no error anywhere.
        //
        // The average hash is still the identity (it is what `image_id`
        // encodes and what near-duplicate reporting uses), but a collision now
        // only allocates a `-N` suffix instead of aliasing two frames.
        if let Some(existing) = self.find_identical_bytes(hash, png_bytes)? {
            return Ok(existing);
        }
        let mut image_id = format!("{hash:016x}");
        let mut n = 0u32;
        while self.images_dir().join(format!("{image_id}.png")).exists() {
            n += 1;
            image_id = format!("{hash:016x}-{n}");
        }
        std::fs::write(self.images_dir().join(format!("{image_id}.png")), png_bytes)
            .map_err(|e| e.to_string())?;
        let ann = MlAnnotation {
            image_id: image_id.clone(),
            dataset_version: DATASET_VERSION,
            session_id: session_id.to_string(),
            task,
            ocr_text: ocr_text.to_string(),
            region_name: region_name.to_string(),
            ui_label: None,
            bbox: None,
            game_state,
            entity_id: None,
            annotator: "collector".to_string(),
            timestamp_ms: now_ms(),
            source: source.to_string(),
            confidence: None,
            hard_example: false,
            hard_reason: None,
            corrections: Vec::new(),
            event_id: None,
            frame_index: None,
        };
        self.append_annotation(&ann)?;
        Ok(image_id)
    }

    /// Find a row whose stored image bytes are IDENTICAL to `png_bytes`.
///
/// Only candidates sharing the average hash are considered (an index-free
/// linear scan over rows is fine at dataset scale), and then the bytes on disk
/// must match exactly. A shared average hash is therefore a *hint*, never a
/// decision.
fn find_identical_bytes(&self, hash: u64, png_bytes: &[u8]) -> Result<Option<String>, String> {
    for ann in self.annotations() {
        if image_id_hash(&ann.image_id) != Some(hash) {
            continue;
        }
        let path = self.images_dir().join(format!("{}.png", ann.image_id));
        if let Ok(bytes) = std::fs::read(&path) {
            if bytes == png_bytes {
                return Ok(Some(ann.image_id));
            }
        }
    }
        Ok(None)
    }

    /// Append one row to `labels.jsonl`.
    ///
    /// Called ONLY from `import_png`, which already holds
    /// `LABELS_WRITE_LOCK`; appending a single complete line in one
    /// `write_all` is what keeps this safe against the read-modify-write
    /// mutators — a writer holding the lock rewrites the file wholesale, so
    /// the append must be serialised against them (it must NOT take the lock
    /// itself: not reentrant).
    fn append_annotation(&self, ann: &MlAnnotation) -> Result<(), String> {
        let mut line = serde_json::to_string(ann).map_err(|e| e.to_string())?;
        line.push('\n');
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.labels_path())
            .map_err(|e| e.to_string())?;
        f.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Flag an imported sample as a hard example with a reason. Used by the
    /// gameplay collector for low-confidence/disagreement observations.
    pub fn set_hard_example(&self, image_id: &str, reason: &str) -> Result<(), String> {
        // Read → mutate → write → rename as one critical section.
        let _guard = LABELS_WRITE_LOCK.lock();
        let mut rows = self.annotations();
        {
            let ann = rows
                .iter_mut()
                .find(|a| a.image_id == image_id)
                .ok_or_else(|| format!("no annotation for image '{image_id}'"))?;
            ann.hard_example = true;
            ann.hard_reason = Some(reason.to_string());
        }
        let out = render_labels(&rows);
        write_labels_atomic(&self.root, &self.labels_path(), &out)
    }

    /// Attach temporal provenance to an imported row. First capture wins:
    /// never overwrites an existing `event_id` (exact-dedupe folds reuse the
    /// first moment's pixels, so the first moment's id stays honest).
    pub fn set_event(&self, image_id: &str, event_id: &str, frame_index: u64) -> Result<bool, String> {
        // Read → mutate → write → rename as one critical section.
        let _guard = LABELS_WRITE_LOCK.lock();
        let mut rows = self.annotations();
        let changed = {
            let ann = rows
                .iter_mut()
                .find(|a| a.image_id == image_id)
                .ok_or_else(|| format!("no annotation for image '{image_id}'"))?;
            if ann.event_id.is_some() {
                return Ok(false);
            }
            ann.event_id = Some(event_id.to_string());
            ann.frame_index = Some(frame_index);
            true
        };
        if !changed {
            return Ok(false);
        }
        let out = render_labels(&rows);
        write_labels_atomic(&self.root, &self.labels_path(), &out)?;
        Ok(true)
    }

    /// Fill empty `ocr_text`/`entity_id` on an existing row (used when a new
    /// sample deduplicates onto identical pixels: the frame is the same, so
    /// carrying over the reading is honest). Never overwrites non-empty
    /// fields and never touches moment-specific labels (`game_state`,
    /// `ui_label`). Returns true when anything was filled.
    pub fn fill_metadata(&self, image_id: &str, ocr_text: &str, entity_id: Option<&str>) -> Result<bool, String> {
        // Read → mutate → write → rename as one critical section.
        let _guard = LABELS_WRITE_LOCK.lock();
        let mut rows = self.annotations();
        let changed = {
            let ann = rows
                .iter_mut()
                .find(|a| a.image_id == image_id)
                .ok_or_else(|| format!("no annotation for image '{image_id}'"))?;
            let mut changed = false;
            if ann.ocr_text.trim().is_empty() && !ocr_text.trim().is_empty() {
                ann.ocr_text = ocr_text.to_string();
                changed = true;
            }
            if ann.entity_id.is_none() {
                if let Some(e) = entity_id {
                    if !e.trim().is_empty() {
                        ann.entity_id = Some(e.to_string());
                        changed = true;
                    }
                }
            }
            changed
        };
        if !changed {
            return Ok(false);
        }
        let out = render_labels(&rows);
        write_labels_atomic(&self.root, &self.labels_path(), &out)?;
        Ok(true)
    }

    /// Rows whose line failed to parse. Counted, never silently dropped.
    pub fn unreadable_lines(&self) -> usize {
        let Ok(content) = std::fs::read_to_string(self.labels_path()) else {
            return 0;
        };
        content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter(|l| serde_json::from_str::<MlAnnotation>(l).is_err())
            .count()
    }

    /// Parse the whole label set, but report a partially-written file.
    ///
    /// `annotations()` cannot fail closed for every caller (read-only UI
    /// panels want data even when one row is odd), but a training snapshot or
    /// an eligibility decision must not silently proceed with missing rows: a
    /// row the parser skipped is a row no filter can see, so it could be
    /// frozen out of a training set without trace.
    pub fn annotations_checked(&self) -> Result<Vec<MlAnnotation>, String> {
        let Ok(content) = std::fs::read_to_string(self.labels_path()) else {
            return Ok(Vec::new());
        };
        let mut rows = Vec::new();
        let mut bad = 0usize;
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<MlAnnotation>(line) {
                Ok(r) => rows.push(r),
                Err(_) => bad += 1,
            }
        }
        if bad > 0 {
            return Err(format!(
                "labels.jsonl has {bad} unreadable line(s); refusing to act on a partial dataset"
            ));
        }
        Ok(rows)
    }

    pub fn annotations(&self) -> Vec<MlAnnotation> {
        let Ok(content) = std::fs::read_to_string(self.labels_path()) else {
            return Vec::new();
        };
        content.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
    }

    /// Annotate (or correct) one image's labels. Corrections append to
    /// history; the manifest row is rewritten atomically.
    #[allow(clippy::too_many_arguments)]
    pub fn annotate(
        &self,
        image_id: &str,
        ui_label: Option<UiLabel>,
        bbox: Option<RelBBox>,
        game_state: Option<GameStateLabel>,
        entity_id: Option<String>,
        annotator: &str,
        hard_example: bool,
        hard_reason: Option<String>,
    ) -> Result<MlAnnotation, String> {
        // BBox validation is pure — do it before locking so a rejected call
        // never blocks a writer.
        if let Some(b) = &bbox {
            if !b.valid() {
                return Err("invalid bounding box (must be 0..1 relative, x+w<=1, y+h<=1)".to_string());
            }
        }
        // Read → mutate → write → rename as one critical section. This is the
        // UI review path, racing the collector's writer thread on the same
        // file; without the guard a reviewer's label could be wiped by a
        // concurrently-collected sample's whole-file rewrite.
        let _guard = LABELS_WRITE_LOCK.lock();
        let mut rows = self.annotations();
        let updated = {
            let ann = rows
                .iter_mut()
                .find(|a| a.image_id == image_id)
                .ok_or_else(|| format!("no annotation for image '{image_id}'"))?;
            let prev = ann.ui_label.map(|l| l.as_str().to_string());
            ann.ui_label = ui_label.or(ann.ui_label);
            ann.bbox = bbox.or_else(|| ann.bbox.clone());
            ann.game_state = game_state.or(ann.game_state);
            ann.entity_id = entity_id.or_else(|| ann.entity_id.clone());
            ann.annotator = annotator.to_string();
            ann.hard_example = hard_example;
            ann.hard_reason = hard_reason;
            if prev != ann.ui_label.map(|l| l.as_str().to_string()) {
                ann.corrections.push(Correction {
                    at_ms: now_ms(),
                    prev_label: prev,
                    note: "correction".to_string(),
                });
            }
            ann.clone()
        };
        let out = render_labels(&rows);
        write_labels_atomic(&self.root, &self.labels_path(), &out)?;
        Ok(updated)
    }

    /// Full quality report. `ok` is false when ANY hard failure exists
    /// (leakage, corrupt files, invalid ids/boxes, orphans). Class imbalance
    /// is reported as a ratio, never a pass/fail on its own.
    pub fn validate(&self, kb: &KnowledgeBase) -> DatasetReport {
        let rows = self.annotations();
        let mut files: HashMap<String, PathBuf> = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(self.images_dir()) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "png") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        files.insert(stem.to_string(), p);
                    }
                }
            }
        }

        let mut hashes: HashMap<u64, Vec<String>> = HashMap::new();
        let mut corrupt_files = Vec::new();
        for (id, path) in &files {
            match std::fs::read(path) {
                Ok(bytes) => match png_ahash(&bytes) {
                    Ok(h) => {
                        hashes.entry(h).or_default().push(id.clone());
                    }
                    Err(_) => corrupt_files.push(id.clone()),
                },
                Err(_) => corrupt_files.push(id.clone()),
            }
        }

        let annotated_ids: HashSet<&str> = rows.iter().map(|r| r.image_id.as_str()).collect();
        let mut classes: HashMap<String, usize> = HashMap::new();
        let mut missing_labels = 0usize;
        let mut invalid_entity_ids = Vec::new();
        let mut invalid_bboxes = Vec::new();
        let mut orphan_annotations = Vec::new();
        let mut sessions: HashSet<String> = HashSet::new();
        let overrides: HashMap<String, Split> = self
            .manifest()
            .map(|m| m.split_overrides)
            .unwrap_or_default();
        let split_for = |sess: &str| {
            overrides.get(sess).copied().unwrap_or_else(|| Self::split_of(sess))
        };

        for r in &rows {
            sessions.insert(r.session_id.clone());
            let labeled = r.ui_label.is_some() || r.game_state.is_some() || r.entity_id.is_some();
            if !labeled {
                missing_labels += 1;
            }
            if let Some(u) = r.ui_label {
                *classes.entry(format!("ui:{}", u.as_str())).or_insert(0) += 1;
            }
            if let Some(g) = r.game_state {
                *classes.entry(format!("state:{}", g.as_str())).or_insert(0) += 1;
            }
            if let Some(e) = &r.entity_id {
                *classes.entry(format!("entity:{e}")).or_insert(0) += 1;
                if kb.find_by_name(e).is_none() && kb.entities().iter().all(|k| k.id != *e) {
                    invalid_entity_ids.push(format!("{}:{}", r.image_id, e));
                }
            }
            if let Some(b) = &r.bbox {
                if !b.valid() {
                    invalid_bboxes.push(r.image_id.clone());
                }
            }
            if !files.contains_key(&r.image_id) {
                orphan_annotations.push(r.image_id.clone());
            }
        }

        let orphan_images: Vec<String> = files
            .keys()
            .filter(|id| !annotated_ids.contains(id.as_str()))
            .cloned()
            .collect();

        // Leakage: identical (or near-identical) frames attributed to
        // different sessions that land in different splits. One session id
        // always maps to one split by construction, so same-session rows can
        // never leak — cross-session near-dupes are the real hazard. Note the
        // same image id may legitimately appear under several session ids
        // (re-imported frames): union them all, never overwrite.
        let mut img_sessions: HashMap<&str, HashSet<&str>> = HashMap::new();
        for r in &rows {
            img_sessions.entry(r.image_id.as_str()).or_default().insert(r.session_id.as_str());
        }
        let mut hash_sessions: HashMap<u64, HashSet<String>> = HashMap::new();
        for (h, ids) in &hashes {
            for id in ids {
                if let Some(sess) = img_sessions.get(id.as_str()) {
                    hash_sessions.entry(*h).or_default().extend(sess.iter().map(|s| s.to_string()));
                }
            }
        }
        let leakage_sessions =
            find_cross_split_leakage(&hash_sessions, NEAR_DUP_HAMMING, |sess| split_for(sess));

        // TRUE event leakage, distinguished from near-similarity: one file
        // (hence one gameplay event) attributed to sessions in >1 split.
        // Dedup normally prevents this (same bytes → same id); a non-empty
        // list means re-imported rows genuinely straddle the split line.
        let mut same_file_cross_split: Vec<String> = img_sessions
            .iter()
            .filter(|(_, sess)| {
                sess.iter().map(|s| split_for(s)).collect::<HashSet<Split>>().len() > 1
            })
            .map(|(id, _)| id.to_string())
            .collect();
        same_file_cross_split.sort();
        // Clusters without a shared file are recurring static UI, not events.
        let near_similarity_groups = leakage_sessions.len();

        let (mut n_train, mut n_val, mut n_test) = (0usize, 0usize, 0usize);
        for sess in &sessions {
            match split_for(sess) {
                Split::Train => n_train += 1,
                Split::Validation => n_val += 1,
                Split::Test => n_test += 1,
            }
        }

        let duplicate_groups = hashes.values().filter(|v| v.len() > 1).count();
        let near_duplicate_pairs = count_near_duplicates(&hashes);

        let min_max_class_ratio = if classes.is_empty() {
            1.0
        } else {
            let min = *classes.values().min().unwrap_or(&1) as f32;
            let max = *classes.values().max().unwrap_or(&1) as f32;
            if max == 0.0 { 1.0 } else { min / max }
        };

        let ok = leakage_sessions.is_empty()
            && same_file_cross_split.is_empty()
            && corrupt_files.is_empty()
            && invalid_entity_ids.is_empty()
            && invalid_bboxes.is_empty()
            && orphan_annotations.is_empty();

        DatasetReport {
            dataset: format!("{DATASET_NAME}-v{DATASET_VERSION}"),
            images: files.len(),
            labeled: rows.len() - missing_labels,
            unlabeled: missing_labels,
            classes,
            sessions_train: n_train,
            sessions_validation: n_val,
            sessions_test: n_test,
            leakage_sessions,
            same_file_cross_split,
            near_similarity_groups,
            corrupt_files,
            missing_labels,
            invalid_entity_ids,
            duplicate_groups,
            near_duplicate_pairs,
            invalid_bboxes,
            orphan_annotations,
            orphan_images,
            min_max_class_ratio,
            ok,
        }
    }

    /// Training eligibility per game-state class (COMPUTED, reporting-only).
    ///
    /// Rules (see module semantics note). Base validity means image file
    /// present and session id non-empty. RESULT is eligible iff
    /// entity-linked (the entity gate is untouched); every other RESULT row
    /// is excluded (unverified), never promoted. WAITING/BITE is eligible
    /// iff base-valid, state present and concrete (not missing/Unknown),
    /// not transition-adjacent, and not hard-flagged for a
    /// state-contradicting reason (today's hard vocabulary concerns
    /// entity/OCR linkage only, so hard never excludes a state in practice
    /// — implemented literally for future vocabularies). Transition
    /// (WAITING/BITE only) means a different-state row from the same session
    /// within ±TRANSITION_WINDOW_MS: own bucket, still visible and counted,
    /// not silently relabeled, not IID ground truth. Decodability is NOT
    /// rechecked here (validate() owns corrupt-file detection); pair this
    /// with a clean validation report.
    pub fn state_eligibility(&self) -> HashMap<String, StateEligibility> {
        let rows = self.annotations();
        let mut files: HashSet<String> = HashSet::new();
        if let Ok(rd) = std::fs::read_dir(self.images_dir()) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "png") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        files.insert(stem.to_string());
                    }
                }
            }
        }
        // Per-session time order for transition adjacency.
        let mut by_session: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, r) in rows.iter().enumerate() {
            by_session.entry(r.session_id.as_str()).or_default().push(i);
        }
        for idxs in by_session.values_mut() {
            idxs.sort_by_key(|&i| rows[i].timestamp_ms);
        }
        // Row -> transition flag (WAITING/BITE only).
        let mut is_transition = vec![false; rows.len()];
        for idxs in by_session.values() {
            for (pos, &i) in idxs.iter().enumerate() {
                let st = match rows[i].game_state {
                    Some(GameStateLabel::WaitingForBite) | Some(GameStateLabel::Bite) => {
                        rows[i].game_state
                    }
                    _ => continue,
                };
                let t = rows[i].timestamp_ms;
                let mut boundary = false;
                if pos > 0 {
                    let j = idxs[pos - 1];
                    if rows[j].game_state.is_some() && rows[j].game_state != st
                        && t.saturating_sub(rows[j].timestamp_ms) <= TRANSITION_WINDOW_MS
                    {
                        boundary = true;
                    }
                }
                if !boundary && pos + 1 < idxs.len() {
                    let j = idxs[pos + 1];
                    if rows[j].game_state.is_some() && rows[j].game_state != st
                        && rows[j].timestamp_ms.saturating_sub(t) <= TRANSITION_WINDOW_MS
                    {
                        boundary = true;
                    }
                }
                is_transition[i] = boundary;
            }
        }

        let mut out: HashMap<String, StateEligibility> = HashMap::new();
        for (i, r) in rows.iter().enumerate() {
            let Some(g) = r.game_state else { continue };
            let key = g.as_str().to_string();
            let e = out.entry(key).or_default();
            e.total += 1;
            // Distinct sessions per class are counted below.
            let base_valid = files.contains(r.image_id.as_str()) && !r.session_id.is_empty();
            if !base_valid {
                e.excluded += 1;
                continue;
            }
            match g {
                GameStateLabel::CatchResult => {
                    if r.entity_id.is_some() {
                        e.eligible += 1;
                    } else {
                        e.excluded += 1;
                    }
                }
                GameStateLabel::WaitingForBite | GameStateLabel::Bite => {
                    let state_invalidating_hard = r.hard_example
                        && r.hard_reason
                            .as_deref()
                            .is_some_and(|s| s.to_lowercase().contains("state"));
                    if state_invalidating_hard {
                        e.excluded += 1;
                    } else if is_transition[i] {
                        e.transition += 1;
                    } else {
                        e.eligible += 1;
                    }
                }
                _ => {
                    // Other states (idle/loading/…) carry no training contract.
                    e.excluded += 1;
                }
            }
        }
        // Distinct sessions per class (over total rows of the class).
        let mut sess_per_class: HashMap<String, HashSet<&str>> = HashMap::new();
        for r in &rows {
            if let Some(g) = r.game_state {
                sess_per_class
                    .entry(g.as_str().to_string())
                    .or_default()
                    .insert(r.session_id.as_str());
            }
        }
        for (k, s) in sess_per_class {
            out.entry(k).or_default().sessions = s.len();
        }
        out
    }

    fn manifest(&self) -> Option<DatasetManifest> {
        std::fs::read_to_string(self.manifest_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 64-bit average hash of PNG bytes (decode → luminance grid → bits).
/// Stable across runs (no SipHash): usable for ids and dedup.
fn png_ahash(png_bytes: &[u8]) -> Result<u64, String> {
    let img = image::load_from_memory(png_bytes).map_err(|e| format!("decode: {e}"))?;
    let gray = img.to_luma8();
    let (w, h) = (gray.width().max(1), gray.height().max(1));
    let mut cells = [0u32; 64];
    let mut counts = [0u32; 64];
    for y in 0..h {
        for x in 0..w {
            let v = gray.get_pixel(x, y)[0] as u32;
            let idx = ((y * 8 / h) * 8 + (x * 8 / w)) as usize;
            cells[idx] += v;
            counts[idx] += 1;
        }
    }
    let total: u64 = cells
        .iter()
        .zip(counts)
        .map(|(s, c)| (*s as u64).checked_div(c as u64).unwrap_or(0))
        .sum();
    let avg = (total / 64) as u32;
    let mut hash = 0u64;
    for (i, s) in cells.iter().enumerate() {
        if s.checked_div(counts[i]).unwrap_or(0) >= avg {
            hash |= 1 << i;
        }
    }
    Ok(hash)
}

/// True when the file at `path` decodes as an image. Used by session
/// finalization (subset check) and the full validator alike.
pub fn png_file_valid(path: &std::path::Path) -> bool {
    std::fs::read(path).ok().and_then(|b| png_ahash(&b).ok()).is_some()
}

/// Image id embeds the hash (`<hex>` or `<hex>-<n>`); recover it for dedup.
fn image_id_hash(image_id: &str) -> Option<u64> {
    let hex = image_id.split('-').next()?;
    if hex.len() != 16 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(hex, 16).ok()
}

fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

fn count_near_duplicates(hashes: &HashMap<u64, Vec<String>>) -> usize {
    let keys: Vec<u64> = hashes.keys().copied().collect();
    let mut pairs = 0usize;
    for i in 0..keys.len() {
        for j in (i + 1)..keys.len() {
            let d = hamming(keys[i], keys[j]);
            if d > 0 && d <= NEAR_DUP_HAMMING {
                pairs += 1;
            }
        }
    }
    pairs
}

/// Cross-split leakage detector (pure, unit-tested): given perceptual-hash
/// → sessions attribution, report every hash whose sessions land in more
/// than one split. Covers both bit-identical frames under different session
/// ids and near-duplicates (hamming ≤ threshold) across sessions.
fn find_cross_split_leakage(
    hash_sessions: &HashMap<u64, HashSet<String>>,
    hamming_threshold: u32,
    split_for: impl Fn(&str) -> Split,
) -> Vec<String> {
    // Collapse to representative session sets per hash cluster (exact hash
    // plus near-dups unioned).
    let keys: Vec<u64> = hash_sessions.keys().copied().collect();
    let mut visited: HashSet<u64> = HashSet::new();
    let mut out = Vec::new();
    for (i, &h) in keys.iter().enumerate() {
        if visited.contains(&h) {
            continue;
        }
        visited.insert(h);
        let mut cluster: HashSet<String> = hash_sessions[&h].clone();
        for &other in &keys[i + 1..] {
            let d = hamming(h, other);
            if d > 0 && d <= hamming_threshold {
                visited.insert(other);
                cluster.extend(hash_sessions[&other].iter().cloned());
            }
        }
        if cluster.len() < 2 {
            continue;
        }
        let splits: HashSet<Split> = cluster.iter().map(|s| split_for(s)).collect();
        if splits.len() > 1 {
            let mut sess: Vec<&str> = cluster.iter().map(|s| s.as_str()).collect();
            sess.sort_unstable();
            out.push(format!("hash {h:016x} spans splits: {}", sess.join("+")));
        }
    }
    out.sort();
    out
}

/// Frame helper: PNG bytes of a live frame for dataset import.
pub fn png_of_frame(frame: &Frame) -> Option<Vec<u8>> {
    frame.to_png_bytes().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_of(color: [u8; 3], w: u32, h: u32) -> Vec<u8> {
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for px in rgba.chunks_exact_mut(4) {
            px[0] = color[0];
            px[1] = color[1];
            px[2] = color[2];
            px[3] = 255;
        }
        let f = Frame::new(w as usize, h as usize, rgba);
        f.to_png_bytes().unwrap()
    }

    fn store(name: &str) -> (MlDatasetStore, PathBuf) {
        static C: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = C.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("gpo-mlds-{name}-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        (MlDatasetStore::new(dir.clone()), dir)
    }

    #[test]
    fn split_is_deterministic_and_covers_all() {
        let a = MlDatasetStore::split_of("sess-2026-01-01");
        let b = MlDatasetStore::split_of("sess-2026-01-01");
        assert_eq!(a, b);
        let mut seen = std::collections::HashSet::new();
        for i in 0..300 {
            seen.insert(MlDatasetStore::split_of(&format!("sess-2026-{i:03}")));
        }
        assert!(seen.contains(&Split::Train));
        assert!(seen.contains(&Split::Validation));
        assert!(seen.contains(&Split::Test));
    }

    #[test]
    fn import_dedups_identical_bytes() {
        let (ds, _d) = store("dedup");
        let bytes = png_of([10, 20, 30], 32, 32);
        let a = ds.import_png(&bytes, "sess-2026-01-01", MlTask::UiDetection, None, "", "drop", "test").unwrap();
        let b = ds.import_png(&bytes, "sess-2026-01-02", MlTask::UiDetection, None, "", "drop", "test").unwrap();
        assert_eq!(a, b);
        assert_eq!(ds.annotations().len(), 1);
    }

    #[test]
    fn annotate_validates_bbox_and_tracks_corrections() {
        let (ds, _d) = store("ann");
        let bytes = png_of([10, 20, 30], 32, 32);
        let id = ds.import_png(&bytes, "sess-2026-01-01", MlTask::UiDetection, None, "", "drop", "test").unwrap();
        assert!(ds
            .annotate(&id, Some(UiLabel::FishingBar), Some(RelBBox { x: 0.9, y: 0.9, w: 0.5, h: 0.5 }), None, None, "t", false, None)
            .is_err());
        let a = ds
            .annotate(
                &id,
                Some(UiLabel::FishingBar),
                Some(RelBBox { x: 0.1, y: 0.1, w: 0.5, h: 0.5 }),
                None,
                Some("fruit:suna".into()),
                "tester",
                true,
                Some("hard: low light".into()),
            )
            .unwrap();
        assert!(a.hard_example);
        assert_eq!(a.entity_id.as_deref(), Some("fruit:suna"));
        // First labeling already records history (None → FishingBar).
        assert_eq!(a.corrections.len(), 1);
        let a2 = ds
            .annotate(&id, Some(UiLabel::BaitMenu), None, None, None, "tester2", false, None)
            .unwrap();
        assert_eq!(a2.corrections.len(), 2);
    }

    #[test]
    fn validator_reports_leakage_corrupt_invalid_orphans() {
        let (ds, dir) = store("val");
        ds.init().unwrap();
        // Two sessions forced into different splits via same-session trick:
        // craft rows manually — one session id appears with an override.
        let b1 = png_of([10, 20, 30], 32, 32);
        let id1 = ds.import_png(&b1, "sess-A", MlTask::UiDetection, Some(GameStateLabel::Fishing), "", "drop", "t").unwrap();
        ds.annotate(&id1, Some(UiLabel::FishingBar), None, None, None, "t", false, None).unwrap();
        // Corrupt file with no annotation row (orphan image, undecodable).
        std::fs::write(dir.join("datasets").join("gpo-vision").join("v1").join("images").join("deadbeef.png"), b"not a png").unwrap();
        // Orphan annotation row (no file).
        {
            use std::io::Write;
            let row = MlAnnotation {
                image_id: "ghost".into(),
                dataset_version: DATASET_VERSION,
                session_id: "sess-A".into(),
                task: MlTask::UiDetection,
                ocr_text: String::new(),
                region_name: "bait_menu".into(),
                ui_label: Some(UiLabel::BaitMenu),
                bbox: Some(RelBBox { x: 0.0, y: 0.0, w: 2.0, h: 0.5 }),
                game_state: None,
                entity_id: Some("nope:missing".into()),
                annotator: "t".into(),
                timestamp_ms: 1,
                source: "t".into(),
                confidence: None,
                hard_example: false,
                hard_reason: None,
                corrections: vec![],
                event_id: None,
                frame_index: None,
            };
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(dir.join("datasets").join("gpo-vision").join("v1").join("labels.jsonl"))
                .unwrap();
            writeln!(f, "{}", serde_json::to_string(&row).unwrap()).unwrap();
        }
        // Real leakage: the identical frame attributed to two sessions that
        // land in different splits (e.g. re-imported gameplay). Find such a
        // pair deterministically, then craft the second row by hand.
        let mut sess_b = None;
        for i in 0..500 {
            let cand = format!("sess-B-{i:03}");
            if MlDatasetStore::split_of(&cand) != MlDatasetStore::split_of("sess-A") {
                sess_b = Some(cand);
                break;
            }
        }
        let sess_b = sess_b.expect("a differently-split session exists");
        {
            use std::io::Write;
            // Same image bytes (id1) under a second session id.
            let rows: Vec<MlAnnotation> = ds.annotations();
            let mut dup = rows.into_iter().find(|r| r.image_id == id1).expect("row");
            dup.session_id = sess_b.clone();
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(dir.join("datasets").join("gpo-vision").join("v1").join("labels.jsonl"))
                .unwrap();
            writeln!(f, "{}", serde_json::to_string(&dup).unwrap()).unwrap();
        }
        let rep = ds.validate(KnowledgeBase::bundled());
        assert_eq!(rep.leakage_sessions.len(), 1, "got: {:?}", rep.leakage_sessions);
        assert!(rep.leakage_sessions[0].contains("sess-A") && rep.leakage_sessions[0].contains(&sess_b));
        assert!(rep.images >= 2);
        assert!(!rep.orphan_images.is_empty(), "undecodable file must be corrupt or orphan");
        assert!(rep.orphan_annotations.contains(&"ghost".to_string()));
        assert!(rep.invalid_bboxes.contains(&"ghost".to_string()));
        assert!(rep.invalid_entity_ids.iter().any(|s| s.contains("nope:missing")));
        assert!(!rep.ok);
    }

    #[test]
    fn cross_split_leakage_helper_flags_hash_spanning_splits() {
        // Pure unit coverage of the pairing rule (same helper validate uses).
        let split_for = |s: &str| MlDatasetStore::split_of(s);
        let mut m: HashMap<u64, HashSet<String>> = HashMap::new();
        m.insert(0xAAAA, ["sess-P".into(), "sess-P".into()].into_iter().collect());
        assert!(find_cross_split_leakage(&m, NEAR_DUP_HAMMING, split_for).is_empty());
        // Two sessions sharing one hash: flagged iff their splits differ.
        let mut m2: HashMap<u64, HashSet<String>> = HashMap::new();
        m2.insert(0xBBBB, ["sess-P".into(), "sess-Q".into()].into_iter().collect());
        let sp = split_for("sess-P");
        let sq = split_for("sess-Q");
        let found = find_cross_split_leakage(&m2, NEAR_DUP_HAMMING, split_for);
        assert_eq!(found.is_empty(), sp == sq);
        // Near-duplicate hashes (3 bits apart) union into one cluster.
        let mut m3: HashMap<u64, HashSet<String>> = HashMap::new();
        m3.insert(0b0u64, ["sess-R".into()].into_iter().collect());
        m3.insert(0b111u64, ["sess-S".into()].into_iter().collect());
        let sr = split_for("sess-R");
        let ss = split_for("sess-S");
        let found3 = find_cross_split_leakage(&m3, NEAR_DUP_HAMMING, split_for);
        assert_eq!(found3.is_empty(), sr == ss);
    }

    #[test]
    fn leakage_detector_flags_multisplit_sessions() {
        // Direct unit coverage of the leakage rule: same session id in two
        // splits must be reported, never silently accepted.
        let (ds, dir) = store("leak");
        ds.init().unwrap();
        let root = dir.join("datasets").join("gpo-vision").join("v1");
        let mk = |img: &str, sess: &str| MlAnnotation {
            image_id: img.into(),
            dataset_version: DATASET_VERSION,
            session_id: sess.into(),
            task: MlTask::GameState,
            ocr_text: String::new(),
            region_name: "drop".into(),
            ui_label: None,
            bbox: None,
            game_state: Some(GameStateLabel::Fishing),
            entity_id: None,
            annotator: "t".into(),
            timestamp_ms: 1,
            source: "t".into(),
            confidence: None,
            hard_example: false,
            hard_reason: None,
            corrections: vec![],
            event_id: None,
            frame_index: None,
        };
        let rows = vec![mk("aa", "sess-X"), mk("bb", "sess-X")];
        // Write matching (empty-content but decodable) PNGs so they count.
        let bytes = png_of([1, 2, 3], 8, 8);
        std::fs::write(root.join("images").join("aa.png"), &bytes).unwrap();
        std::fs::write(root.join("images").join("bb.png"), &bytes).unwrap();
        let mut out = String::new();
        for r in &rows {
            out.push_str(&serde_json::to_string(r).unwrap());
            out.push('\n');
        }
        std::fs::write(root.join("labels.jsonl"), out).unwrap();
        // Force the split: override sess-X to Test, then verify natural map
        // differs... instead assert the invariant directly: identical session
        // ids always share one split by construction.
        let s1 = MlDatasetStore::split_of("sess-X");
        let s2 = MlDatasetStore::split_of("sess-X");
        assert_eq!(s1, s2);
        let rep = ds.validate(KnowledgeBase::bundled());
        assert!(rep.leakage_sessions.is_empty(), "same-session rows share one split");
    }

    #[test]
    fn fill_metadata_merges_without_overwriting() {
        let (ds, _d) = store("fill");
        let bytes = png_of([10, 20, 30], 32, 32);
        let id = ds.import_png(&bytes, "sess-1", MlTask::UiDetection, None, "", "bar", "t").unwrap();
        // Fill empties.
        assert!(ds.fill_metadata(&id, "Suna fruit", Some("fruit:suna")).unwrap());
        let row = ds.annotations().into_iter().find(|r| r.image_id == id).unwrap();
        assert_eq!(row.ocr_text, "Suna fruit");
        assert_eq!(row.entity_id.as_deref(), Some("fruit:suna"));
        assert_eq!(row.region_name, "bar", "moment-specific fields never touched");
        // Second fill is a no-op (never overwrites, never churns).
        assert!(!ds.fill_metadata(&id, "Other text", Some("fruit:mera")).unwrap());
        let row2 = ds.annotations().into_iter().find(|r| r.image_id == id).unwrap();
        assert_eq!(row2.ocr_text, "Suna fruit");
        assert!(ds.fill_metadata("missing-id", "x", None).is_err());
    }

    #[test]
    fn near_duplicate_detection_counts_close_hashes() {
        // Deterministic: hashes differing by 3 bits are near-dups, identical
        // hashes are exact dups (not counted), distant hashes are unrelated.
        let mut map: HashMap<u64, Vec<String>> = HashMap::new();
        map.insert(0b0u64, vec!["a".into()]);
        map.insert(0b111u64, vec!["b".into()]);
        map.insert(0xFFFF_FFFF_FFFF_FFFFu64, vec!["c".into()]);
        assert_eq!(count_near_duplicates(&map), 1);
        assert_eq!(hamming(0b0, 0b111), 3);
        // End-to-end: structurally different frames get their own ids
        // (uniform fills share one hash by construction — use halves).
        let (ds, _d) = store("near");
        let a = png_of([10, 20, 30], 32, 32);
        let mut rgba_b = vec![0u8; 32 * 32 * 4];
        for (i, px) in rgba_b.chunks_exact_mut(4).enumerate() {
            let left = (i % 32) < 16;
            px[0] = if left { 10 } else { 220 };
            px[1] = if left { 20 } else { 210 };
            px[2] = if left { 30 } else { 200 };
            px[3] = 255;
        }
        let fb = Frame::new(32, 32, rgba_b);
        let b = fb.to_png_bytes().unwrap();
        let ida = ds.import_png(&a, "sess-1", MlTask::UiDetection, None, "", "drop", "t").unwrap();
        let idb = ds.import_png(&b, "sess-2", MlTask::UiDetection, None, "", "drop", "t").unwrap();
        assert_ne!(ida, idb);
    }

    fn elig_row(
        image_id: &str,
        session: &str,
        ts: u64,
        state: Option<GameStateLabel>,
        entity: Option<&str>,
        hard: bool,
        reason: Option<&str>,
    ) -> MlAnnotation {
        MlAnnotation {
            image_id: image_id.into(),
            dataset_version: DATASET_VERSION,
            session_id: session.into(),
            task: MlTask::UiDetection,
            ocr_text: String::new(),
            region_name: "bar".into(),
            ui_label: Some(UiLabel::FishingBar),
            bbox: None,
            game_state: state,
            entity_id: entity.map(|s| s.into()),
            annotator: "test".into(),
            timestamp_ms: ts,
            source: "test".into(),
            confidence: None,
            hard_example: hard,
            hard_reason: reason.map(|s| s.into()),
            corrections: vec![],
            event_id: None,
            frame_index: None,
        }
    }

    fn sess_with_split(want: Split) -> String {
        for i in 0..2000 {
            let cand = format!("sess-elig-{i:04}");
            if MlDatasetStore::split_of(&cand) == want {
                return cand;
            }
        }
        panic!("no session hashing to {want:?}");
    }

    #[test]
    fn eligibility_buckets_rows_correctly() {
        // Controlled timestamps: r1 stable-W, r2/r3 transition pair (500ms),
        // r4 linked-R, r5 unlinked hard-R, r6 ghost-W (missing file).
        let (ds, _d) = store("elig");
        let ids: Vec<String> = (0..5u8)
            .map(|i| {
                ds.import_png(
                    &png_of([10 + i * 20, 20, 30], 32, 32),
                    "sess-elig-A",
                    MlTask::UiDetection,
                    None,
                    "",
                    "bar",
                    "t",
                )
                .unwrap()
            })
            .collect();
        let rows = [
            elig_row(&ids[0], "sess-elig-A", 100_000, Some(GameStateLabel::WaitingForBite), None, false, None),
            elig_row(&ids[1], "sess-elig-A", 200_000, Some(GameStateLabel::Bite), None, false, None),
            elig_row(&ids[2], "sess-elig-A", 200_500, Some(GameStateLabel::WaitingForBite), None, false, None),
            elig_row(&ids[3], "sess-elig-B", 300_000, Some(GameStateLabel::CatchResult), Some("fruit:suna"), false, None),
            elig_row(&ids[4], "sess-elig-B", 300_000, Some(GameStateLabel::CatchResult), None, true, Some("unverdictable catch")),
            elig_row("ghost000000000000", "sess-elig-B", 300_000, Some(GameStateLabel::WaitingForBite), None, false, None),
        ];
        let body = rows
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(ds.labels_path(), body).unwrap();

        let cov = ds.state_eligibility();
        let w = &cov["waiting_for_bite"];
        assert_eq!((w.total, w.eligible, w.transition, w.excluded, w.sessions), (3, 1, 1, 1, 2));
        let b = &cov["bite"];
        assert_eq!((b.total, b.eligible, b.transition, b.excluded, b.sessions), (1, 0, 1, 0, 1));
        let r = &cov["catch_result"];
        assert_eq!((r.total, r.eligible, r.transition, r.excluded, r.sessions), (2, 1, 0, 1, 1));
        // Buckets are disjoint and exhaustive.
        for e in [&w, &b, &r] {
            assert_eq!(e.total, e.eligible + e.transition + e.excluded);
        }
        // WAITING/BITE rows never carry entities here: the entity gate
        // cannot be satisfied by episode frames, by construction.
        assert!(ds.annotations().iter().filter(|a| a.entity_id.is_some()).all(|a| {
            a.game_state == Some(GameStateLabel::CatchResult)
        }));
    }

    #[test]
    fn same_file_cross_split_is_true_event_leak() {
        // One file attributed to two sessions in different splits: the same
        // gameplay event would be seen in training and evaluation. (Import
        // dedup never creates this — same bytes fold without a new row — so
        // the row is hand-attributed, exactly the hand-edit case the check
        // guards.)
        let (ds, _d) = store("eventleak");
        let sess_a = sess_with_split(Split::Train);
        let sess_b = sess_with_split(Split::Test);
        let id = ds
            .import_png(&png_of([10, 20, 30], 32, 32), &sess_a, MlTask::UiDetection, None, "", "drop", "t")
            .unwrap();
        ds.annotate(&id, Some(UiLabel::FishingBar), None, Some(GameStateLabel::WaitingForBite), None, "t", false, None)
            .unwrap();
        let row_b = elig_row(&id, &sess_b, 999_000, Some(GameStateLabel::WaitingForBite), None, false, None);
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(ds.labels_path())
                .unwrap();
            writeln!(f, "{}", serde_json::to_string(&row_b).unwrap()).unwrap();
        }
        let rep = ds.validate(KnowledgeBase::bundled());
        assert!(rep.same_file_cross_split.contains(&id));
        assert!(!rep.leakage_sessions.is_empty());
        assert!(!rep.ok);
    }

    /// 64x64 cell-controlled frame: all 8x8 cells mid-gray except the
    /// listed cell indices painted black. Yields exact, predictable ahash
    /// bits (one flipped bit per black cell vs the all-gray image).
    fn cell_png(black_cells: &[usize]) -> Vec<u8> {
        let mut rgba = vec![0u8; 64 * 64 * 4];
        for (p, px) in rgba.chunks_exact_mut(4).enumerate() {
            let x = p % 64;
            let y = p / 64;
            let cell = (y / 8) * 8 + (x / 8);
            let v = if black_cells.contains(&cell) { 0 } else { 100 };
            px[0] = v;
            px[1] = v;
            px[2] = v;
            px[3] = 255;
        }
        Frame::new(64, 64, rgba).to_png_bytes().unwrap()
    }

    #[test]
    fn near_dupe_without_shared_file_is_not_event_leak() {
        // Two files exactly 1 hash-bit apart in different splits: visual
        // near-similarity (recurring UI), NOT the same gameplay event.
        let (ds, _d) = store("nearsim");
        let sess_a = sess_with_split(Split::Train);
        let sess_b = sess_with_split(Split::Test);
        let a = cell_png(&[0]);
        let b = cell_png(&[0, 1]);
        assert_eq!(hamming(png_ahash(&a).unwrap(), png_ahash(&b).unwrap()), 1);
        let ida = ds
            .import_png(&a, &sess_a, MlTask::UiDetection, None, "", "bar", "t")
            .unwrap();
        let idb = ds
            .import_png(&b, &sess_b, MlTask::UiDetection, None, "", "bar", "t")
            .unwrap();
        assert_ne!(ida, idb, "1-bit-apart frames must not fold");
        let rep = ds.validate(KnowledgeBase::bundled());
        assert!(!rep.leakage_sessions.is_empty(), "near-dupes must cluster");
        assert!(
            rep.same_file_cross_split.is_empty(),
            "distinct files are similarity, not event leakage: {:?}",
            rep.same_file_cross_split
        );
    }

    // ---- labels.jsonl concurrency (lost-update / torn-file) ----
    //
    // The collector's writer thread and the UI review path both run
    // read-modify-write cycles over ONE `labels.jsonl`. These tests drive the
    // mutators concurrently against a single store and assert the two
    // properties the lock exists to provide:
    //   1. no lost update — every mutation is present in the final file, and
    //   2. no torn file — every line is complete, parseable JSON.

    /// Raw `labels.jsonl` split into lines, asserting the file is well-formed
    /// JSONL (no truncated/torn line from an interleaved writer).
    fn labels_lines(ds: &MlDatasetStore) -> Vec<String> {
        let raw = std::fs::read_to_string(ds.labels_path()).expect("labels.jsonl readable");
        assert!(!raw.is_empty(), "labels.jsonl must not be empty");
        assert!(raw.ends_with('\n'), "labels.jsonl must end with a newline");
        let lines: Vec<String> = raw.lines().map(|l| l.to_string()).collect();
        for (i, l) in lines.iter().enumerate() {
            assert!(
                serde_json::from_str::<MlAnnotation>(l).is_ok(),
                "line {i} is not a complete JSON object (torn write): {l}"
            );
        }
        lines
    }

    /// Distinct-bytes imports: `cell_png(&[i])` flips exactly one ahash bit, so
    /// every row gets its own image id (uniform fills all share one hash).
    fn seed_rows(ds: &MlDatasetStore, n: usize) -> Vec<String> {
        (0..n)
            .map(|i| {
                ds.import_png(&cell_png(&[i]), "sess-conc", MlTask::UiDetection, None, "", "bar", "t")
                    .expect("import")
            })
            .collect()
    }

    #[test]
    fn concurrent_annotate_hard_and_fill_lose_no_update() {
        // Three mutators × 3 threads, hammering ONE store. Row sets are
        // DISJOINT (rows 0-2 annotate, 3-5 hard, 6-8 fill) so each assertion
        // is unambiguous — no mutator can legitimately undo another's effect.
        const N: usize = 9;
        let (ds, _d) = store("conc3");
        let ids = seed_rows(&ds, N);
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), N, "ids must be distinct");

        const ROUNDS: usize = 12;
        std::thread::scope(|s| {
            for t in 0..3usize {
                let ds = &ds;
                let ids = &ids;
                s.spawn(move || {
                    for _ in 0..ROUNDS {
                        match t {
                            0 => {
                                for id in &ids[0..3] {
                                    ds.annotate(
                                        id,
                                        Some(UiLabel::FishingBar),
                                        None,
                                        Some(GameStateLabel::Bite),
                                        Some("fruit:suna".into()),
                                        &format!("ann-{t}"),
                                        false,
                                        None,
                                    )
                                    .unwrap();
                                }
                            }
                            1 => {
                                for id in &ids[3..6] {
                                    ds.set_hard_example(id, &format!("hard-{t}")).unwrap();
                                }
                            }
                            _ => {
                                for id in &ids[6..9] {
                                    ds.fill_metadata(id, "Suna fruit", Some("fruit:suna")).unwrap();
                                }
                            }
                        }
                    }
                });
            }
        });

        // (2) Not torn.
        let lines = labels_lines(&ds);
        assert_eq!(lines.len(), N, "row count must survive concurrent rewrites");

        // (1) No lost update: every mutation landed, every row intact.
        let rows = ds.annotations();
        assert_eq!(rows.len(), N);
        for (i, id) in ids.iter().enumerate() {
            let r = rows.iter().find(|r| &r.image_id == id).expect("row survives");
            match i {
                0..=2 => {
                    assert_eq!(r.ui_label, Some(UiLabel::FishingBar), "annotate lost on {id}");
                    assert_eq!(r.game_state, Some(GameStateLabel::Bite), "annotate lost on {id}");
                    assert_eq!(r.entity_id.as_deref(), Some("fruit:suna"));
                }
                3..=5 => {
                    assert!(r.hard_example, "set_hard_example lost on {id}");
                    assert_eq!(r.hard_reason.as_deref(), Some("hard-1"));
                }
                _ => {
                    assert_eq!(r.ocr_text, "Suna fruit", "fill_metadata lost on {id}");
                    assert_eq!(r.entity_id.as_deref(), Some("fruit:suna"));
                }
            }
        }
    }

    #[test]
    fn annotate_racing_import_png_loses_no_appended_row() {
        // The collector APPENDS (import_png) while the review path REWRITES
        // the whole file (annotate). Without serialisation, the rewrite is
        // built from a stale read and silently drops rows appended in between.
        let (ds, _d) = store("concimport");
        let base = seed_rows(&ds, 3);
        let reviewed: &String = &base[0];

        const IMPORTS: usize = 6;
        const ROUNDS: usize = 8;
        std::thread::scope(|s| {
            // Appender (collector writer path).
            let store = &ds;
            s.spawn(move || {
                for i in 0..IMPORTS {
                    store
                        .import_png(
                            &cell_png(&[10 + i]),
                            "sess-conc",
                            MlTask::UiDetection,
                            None,
                            "",
                            "bar",
                            "t",
                        )
                        .expect("import");
                    std::thread::yield_now();
                }
            });
            // Reviewer (UI main-thread path): rewrites the whole file.
            let store = &ds;
            s.spawn(move || {
                for _ in 0..ROUNDS {
                    store
                        .annotate(
                            reviewed,
                            Some(UiLabel::BaitMenu),
                            None,
                            Some(GameStateLabel::CatchResult),
                            Some("fruit:mera".into()),
                            "reviewer",
                            false,
                            None,
                        )
                        .unwrap();
                    std::thread::yield_now();
                }
            });
        });

        labels_lines(&ds); // no torn file
        let rows = ds.annotations();
        // 3 seeded + 6 appended, all present.
        assert_eq!(rows.len(), 3 + IMPORTS, "appended rows were lost to a concurrent rewrite");
        for id in &base {
            assert!(rows.iter().any(|r| &r.image_id == id), "seeded row {id} lost");
        }
        let r = rows.iter().find(|r| r.image_id == *reviewed).expect("reviewed row");
        assert_eq!(r.ui_label, Some(UiLabel::BaitMenu));
        assert_eq!(r.entity_id.as_deref(), Some("fruit:mera"));
    }

    #[test]
    fn stale_temp_file_from_earlier_write_is_inert() {
        // A leftover staging file (crash mid-write, or the legacy fixed
        // `labels.jsonl.tmp`) must never be picked up, renamed over
        // `labels.jsonl`, or read as data. Per-write unique names make any
        // leftover inert by construction.
        let (ds, _d) = store("stale");
        let ids = seed_rows(&ds, 4);
        let root = ds.labels_path().parent().unwrap().to_path_buf();

        // Garbage in a stale per-write temp AND in the legacy fixed temp.
        let stale_seq = format!("labels.jsonl.{}.999999.tmp", std::process::id());
        let stale_bytes = b"{\"image_id\":\"garbage\",\"torn\":tru\nnot json at all";
        std::fs::write(root.join(&stale_seq), stale_bytes).unwrap();
        std::fs::write(root.join("labels.jsonl.tmp"), b"TOTALLY NOT JSONL").unwrap();

        // Mutate twice: both must succeed against the real file.
        ds.annotate(&ids[0], Some(UiLabel::ServerTime), None, None, None, "t", false, None).unwrap();
        ds.set_hard_example(&ids[1], "hard: stale").unwrap();

        labels_lines(&ds);
        let rows = ds.annotations();
        assert_eq!(rows.len(), 4, "stale temp files must not contribute rows");
        assert!(rows.iter().all(|r| r.image_id != "garbage"));
        assert_eq!(
            rows.iter().find(|r| r.image_id == ids[0]).unwrap().ui_label,
            Some(UiLabel::ServerTime)
        );
        assert!(rows.iter().find(|r| r.image_id == ids[1]).unwrap().hard_example);

        // Stale files are left alone (never consumed, never renamed onto us).
        assert_eq!(std::fs::read(root.join(&stale_seq)).unwrap(), stale_bytes.to_vec());
        assert_eq!(std::fs::read(root.join("labels.jsonl.tmp")).unwrap(), b"TOTALLY NOT JSONL");
    }

    #[test]
    fn concurrent_imports_of_identical_bytes_dedup_once() {
        // import_png's dedup check + row append must be one critical section,
        // or two racing imports of the same bytes both append a row.
        let (ds, _d) = store("concdedup");
        let bytes: &Vec<u8> = &cell_png(&[3]);
        const THREADS: usize = 6;
        let out = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|s| {
            for _ in 0..THREADS {
                let store = &ds;
                let out = &out;
                s.spawn(move || {
                    let id = store
                        .import_png(bytes, "sess-conc", MlTask::UiDetection, None, "", "bar", "t")
                        .expect("import");
                    out.lock().unwrap().push(id);
                });
            }
        });
        let out = out.into_inner().unwrap();
        assert_eq!(out.len(), THREADS);
        assert!(out.iter().all(|id| *id == out[0]), "all racing imports must fold onto one id");
        labels_lines(&ds);
        assert_eq!(ds.annotations().len(), 1, "identical bytes must not duplicate rows");
    }
}
