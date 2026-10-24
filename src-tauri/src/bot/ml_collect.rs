//! Macro-triggered ML data collection sessions.
//!
//! Lifecycle (driven by `Bot::start` / `pause` / `stop`):
//!
//! ```text
//! START MACRO → begin_session() → COLLECTING (async, bounded queue)
//! STOP MACRO  → end_session(): stop intake → flush → finalize → validate
//! ```
//!
//! Guarantees:
//! - The timing-critical loop never blocks on I/O: capture hands a PNG to a
//!   bounded channel; a background thread encodes/persists. A full queue
//!   drops (counted) instead of stalling the macro.
//! - Sessions append; nothing is overwritten, deleted, or reset (no reset
//!   API exists by design).
//! - Crash safety: `sessions/<id>.json` starts `complete:false`; startup
//!   recovery marks leftovers `recovered-incomplete` without touching rows.
//! - Secrets: only game-UI crops and game text are stored. No settings,
//!   tokens, cookies, chat, or system information ever enters a sample.

use crossbeam_channel::{bounded, Receiver, Sender};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::core::ml_dataset::{GameStateLabel, MlDatasetStore, MlTask, UiLabel};

/// Bounded async queue depth. Small on purpose: backpressure must shed load
/// (counted as dropped), never stall the macro loop.
pub const QUEUE_CAP: usize = 256;
/// Flush wait at session end before finalizing with what's done.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct SampleJob {
    pub png: Vec<u8>,
    pub session_id: String,
    pub task: MlTask,
    pub ui_label: Option<UiLabel>,
    pub game_state: Option<GameStateLabel>,
    pub ocr_text: String,
    pub region: String,
    pub entity_id: Option<String>,
    pub hard_reason: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: String,
    pub started_ms: u64,
    pub stopped_ms: Option<u64>,
    pub app_version: String,
    pub capture: CaptureConfig,
    pub samples: u64,
    pub reels: u64,
    pub hard_examples: u64,
    pub dropped: u64,
    pub complete: bool,
    pub recovered_incomplete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    pub task: String,
    pub regions: Vec<String>,
    pub trace_gated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub samples: u64,
    pub reels: u64,
    pub hard_examples: u64,
    pub dropped: u64,
    pub total_samples: u64,
    pub pending_annotation: u64,
    pub quality_ok: bool,
    pub quality_warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CollectorStats {
    pub collecting: bool,
    pub session_id: Option<String>,
    pub submitted: u64,
    pub written: u64,
    pub dropped: u64,
    pub write_errors: u64,
    pub queue_depth: usize,
    pub reels: u64,
    pub hard_examples: u64,
    /// Wall-clock ms of the last successful frame write (0 = none yet).
    /// Drives the "last capture … ago" health signal.
    pub last_write_ms: u64,
}

struct Inner {
    tx: Sender<SampleJob>,
    accepting: AtomicBool,
    pending: AtomicUsize,
    submitted: AtomicUsize,
    written: AtomicUsize,
    dropped: AtomicUsize,
    write_errors: AtomicUsize,
    reels: AtomicUsize,
    hard: AtomicUsize,
    session: Mutex<Option<SessionMeta>>,
    session_images: Mutex<Vec<String>>,
    last_write_ms: AtomicU64,
}

pub struct MlCollector {
    dataset: MlDatasetStore,
    sessions_dir: PathBuf,
    inner: Arc<Inner>,
}

impl MlCollector {
    pub fn new(data_dir: PathBuf) -> Arc<Self> {
        let dataset = MlDatasetStore::new(data_dir.clone());
        let sessions_dir = data_dir.join("sessions");
        let _ = std::fs::create_dir_all(&sessions_dir);
        let (tx, rx) = bounded::<SampleJob>(QUEUE_CAP);
        let inner = Arc::new(Inner {
            tx,
            accepting: AtomicBool::new(false),
            pending: AtomicUsize::new(0),
            submitted: AtomicUsize::new(0),
            written: AtomicUsize::new(0),
            dropped: AtomicUsize::new(0),
            write_errors: AtomicUsize::new(0),
            reels: AtomicUsize::new(0),
            hard: AtomicUsize::new(0),
            session: Mutex::new(None),
            session_images: Mutex::new(Vec::new()),
            last_write_ms: AtomicU64::new(0),
        });
        let c = Arc::new(Self { dataset, sessions_dir, inner });
        // Crash recovery BEFORE any new session: quarantine leftovers.
        let recovered = c.recover_incomplete();
        if recovered > 0 {
            tracing::warn!("ML collection: recovered {recovered} incomplete session(s) from previous run (samples kept, marked incomplete)");
        }
        // Background writer for the process lifetime.
        let writer = Arc::clone(&c);
        std::thread::Builder::new()
            .name("ml-collect-writer".into())
            .spawn(move || writer.writer_loop(rx))
            .ok();
        c
    }

    /// Deterministic construction for tests: no threads, no disk recovery.
    #[cfg(test)]
    fn new_isolated(data_dir: PathBuf) -> Self {
        let (tx, _rx) = bounded::<SampleJob>(QUEUE_CAP);
        Self {
            dataset: MlDatasetStore::new(data_dir.clone()),
            sessions_dir: data_dir.join("sessions"),
            inner: Arc::new(Inner {
                tx,
                accepting: AtomicBool::new(false),
                pending: AtomicUsize::new(0),
                submitted: AtomicUsize::new(0),
                written: AtomicUsize::new(0),
                dropped: AtomicUsize::new(0),
                write_errors: AtomicUsize::new(0),
                reels: AtomicUsize::new(0),
                hard: AtomicUsize::new(0),
                session: Mutex::new(None),
                session_images: Mutex::new(Vec::new()),
                last_write_ms: AtomicU64::new(0),
            }),
        }
    }

    fn session_path(&self, id: &str) -> PathBuf {
        self.sessions_dir.join(format!("{id}.json"))
    }

    fn write_meta(&self, meta: &SessionMeta) {
        let _ = std::fs::create_dir_all(&self.sessions_dir);
        let tmp = self.sessions_dir.join(format!("{}.tmp", meta.session_id));
        if let Ok(bytes) = serde_json::to_vec_pretty(meta) {
            if std::fs::write(&tmp, &bytes).is_ok() {
                let _ = std::fs::rename(&tmp, self.session_path(&meta.session_id));
            }
        }
    }

    /// Begin a collection session for one macro run. Idempotent-ish: if a
    /// session is already active (double start), the existing id is kept.
    pub fn begin_session(&self, app_version: &str) -> String {
        if let Some(active) = self.inner.session.lock().as_ref() {
            return active.session_id.clone();
        }
        let id = new_session_id();
        let meta = SessionMeta {
            session_id: id.clone(),
            started_ms: now_ms(),
            stopped_ms: None,
            app_version: app_version.to_string(),
            capture: CaptureConfig {
                task: "entity_recognition+ui_detection".to_string(),
                regions: vec!["drop".to_string(), "bar".to_string()],
                trace_gated: true,
            },
            samples: 0,
            reels: 0,
            hard_examples: 0,
            dropped: 0,
            complete: false,
            recovered_incomplete: false,
        };
        self.write_meta(&meta);
        *self.inner.session.lock() = Some(meta);
        self.inner.session_images.lock().clear();
        self.inner.accepting.store(true, Ordering::SeqCst);
        tracing::info!("ML collection: session {id} started (COLLECTING)");
        id
    }

    pub fn active_session_id(&self) -> Option<String> {
        self.inner.session.lock().as_ref().map(|m| m.session_id.clone())
    }

    pub fn is_collecting(&self) -> bool {
        self.inner.accepting.load(Ordering::SeqCst) && self.inner.session.lock().is_some()
    }

    /// Non-blocking submit from the macro loop. Full queue → counted drop,
    /// never a stall. No active session → counted drop (never an error).
    pub fn submit(&self, mut job: SampleJob) -> bool {
        let session = match self.inner.session.lock().as_ref() {
            Some(m) => m.session_id.clone(),
            None => {
                self.inner.dropped.fetch_add(1, Ordering::SeqCst);
                return false;
            }
        };
        if !self.inner.accepting.load(Ordering::SeqCst) {
            self.inner.dropped.fetch_add(1, Ordering::SeqCst);
            return false;
        }
        job.session_id = session;
        self.inner.submitted.fetch_add(1, Ordering::SeqCst);
        // Claimed BEFORE the send so the writer can never complete (and
        // decrement) ahead of us — pending only over-counts transiently,
        // which delays flush but can never early-exit it or underflow.
        self.inner.pending.fetch_add(1, Ordering::SeqCst);
        match self.inner.tx.try_send(job) {
            Ok(()) => true,
            Err(_) => {
                self.inner.pending.fetch_sub(1, Ordering::SeqCst);
                self.inner.dropped.fetch_add(1, Ordering::SeqCst);
                false
            }
        }
    }

    pub fn count_reel(&self) {
        self.inner.reels.fetch_add(1, Ordering::SeqCst);
    }

    fn writer_loop(&self, rx: Receiver<SampleJob>) {
        for job in rx.iter() {
            self.write_one(&job);
            // Decremented only after all counters/meta updates landed, so a
            // flush on pending==0 observes complete results (no TOCTOU gap
            // between queue-take and completion accounting).
            self.inner.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn write_one(&self, job: &SampleJob) {
        let hard = job.hard_reason.clone();
        let ocr = job.ocr_text.clone();
        let entity = job.entity_id.clone();
        let result = self.dataset.import_png(
            &job.png,
            &job.session_id,
            job.task,
            job.game_state,
            &job.ocr_text,
            &job.region,
            &job.source,
        );
        match result {
            Ok(image_id) => {
                self.inner.written.fetch_add(1, Ordering::SeqCst);
                self.inner.last_write_ms.store(now_ms(), Ordering::SeqCst);
                self.inner.session_images.lock().push(image_id.clone());
                // A DEDUP FOLD is not a new sample: `import_png` returned the
                // existing id because the bytes were already stored. Counting
                // it as written (and as a session sample) overstated collection
                // yield, and `set_event` below would attach this moment's frame
                // index to the other moment's row.
                let folded = !self.dataset.image_is_new(&image_id);
                if !folded {
                    self.inner.written.fetch_add(1, Ordering::SeqCst);
                    self.inner.last_write_ms.store(now_ms(), Ordering::SeqCst);
                }
                // Dedup may have folded this sample onto identical pixels
                // from an earlier moment: carry over the reading (OCR text
                // + entity) into empty fields rather than losing it.
                let _ = self.dataset.fill_metadata(&image_id, &ocr, entity.as_deref());
                // Machine-derived labels at capture time (game state from the
                // state machine, ui_label from the bot, entity from OCR+KB).
                // These are recorded with `annotator: "collector"` and are
                // NOT training-eligible: `snapshot_dataset` keeps unreviewed
                // rows but the review layer is what grants eligibility, so a
                // collector label alone never trains.
                if job.ui_label.is_some() || job.entity_id.is_some() || job.game_state.is_some() {
                    let _ = self.dataset.annotate(
                        &image_id,
                        job.ui_label,
                        None,
                        job.game_state,
                        job.entity_id.clone(),
                        "collector",
                        hard.is_some(),
                        hard.clone(),
                    );
                } else if let Some(reason) = &hard {
                    let _ = self.dataset.set_hard_example(&image_id, reason);
                }
                if hard.is_some() {
                    self.inner.hard.fetch_add(1, Ordering::SeqCst);
                }
                // Mirror counters into the session meta (best-effort) and
                // attach temporal provenance: the pre-increment sample count
                // is the zero-based per-session capture sequence.
                //
                // A DEDUP FOLD gets no frame index: the pixels belong to an
                // earlier moment, and stamping this moment's index onto that
                // row would assert a temporal provenance that never happened.
                if !folded {
                    if let Some(meta) = self.inner.session.lock().as_mut() {
                        let frame_index = meta.samples;
                        let event_id = format!("{}#f{frame_index:06}", meta.session_id);
                        let _ = self.dataset.set_event(&image_id, &event_id, frame_index);
                        meta.samples += 1;
                        if hard.is_some() {
                            meta.hard_examples += 1;
                        }
                    }
                }
            }
            Err(e) => {
                self.inner.write_errors.fetch_add(1, Ordering::SeqCst);
                tracing::warn!("ML collection: sample write failed ({e})");
            }
        }
    }

    /// Stop intake, flush pending writes (bounded wait), finalize metadata,
    /// run subset validation. Returns None when no session was active.
    /// Ordering matters: the session meta is taken AFTER the flush, so slow
    /// trailing writes (manifest rewrites) still land in the finalized
    /// counters. Taking first would freeze a partial snapshot.
    pub fn end_session(&self) -> Option<SessionSummary> {
        self.active_session_id()?;
        self.inner.accepting.store(false, Ordering::SeqCst);

        // Flush: wait until every accepted job is fully accounted
        // (pending hits 0 only after rows, counters, and meta updates all
        // landed). Bounded; a stuck writer yields a WARNING, not a hang.
        let deadline = std::time::Instant::now() + FLUSH_TIMEOUT;
        while std::time::Instant::now() < deadline {
            if self.inner.pending.load(Ordering::SeqCst) == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let drained = self.inner.pending.load(Ordering::SeqCst) == 0;

        let mut meta = self.inner.session.lock().take()?;
        meta.stopped_ms = Some(now_ms());

        meta.reels = self.inner.reels.load(Ordering::SeqCst) as u64;
        meta.dropped = self.inner.dropped.load(Ordering::SeqCst) as u64;
        meta.complete = true;
        self.write_meta(&meta);

        // Subset validation: only this session's images (fast, bounded).
        // Full-dataset validation stays on demand (Setup › Validate).
        let images = self.inner.session_images.lock().clone();
        let mut corrupt = 0usize;
        for id in &images {
            let p = self.dataset.root().join("images").join(format!("{id}.png"));
            if !crate::core::ml_dataset::png_file_valid(&p) {
                corrupt += 1;
            }
        }
        let mut warnings = Vec::new();
        if !drained {
            warnings.push("flush timeout: some queued samples may still be writing".to_string());
        }
        if corrupt > 0 {
            warnings.push(format!("{corrupt} session images failed validation"));
        }
        let quality_ok = warnings.is_empty();

        // Dataset-wide pending count (cheap: manifest rows only).
        let pending = self.dataset.annotations().iter().filter(|a| a.entity_id.is_none()).count() as u64;
        let total = self.dataset.annotations().len() as u64;

        tracing::info!(
            "ML collection: session {} complete — {} samples, {} reels, {} hard, quality {}",
            meta.session_id,
            meta.samples,
            meta.reels,
            meta.hard_examples,
            if quality_ok { "PASS" } else { "WARNING" }
        );
        Some(SessionSummary {
            session_id: meta.session_id.clone(),
            samples: meta.samples,
            reels: meta.reels,
            hard_examples: meta.hard_examples,
            dropped: meta.dropped,
            total_samples: total,
            pending_annotation: pending,
            quality_ok,
            quality_warnings: warnings,
        })
    }

    pub fn stats_snapshot(&self) -> CollectorStats {
        CollectorStats {
            collecting: self.is_collecting(),
            session_id: self.active_session_id(),
            submitted: self.inner.submitted.load(Ordering::SeqCst) as u64,
            written: self.inner.written.load(Ordering::SeqCst) as u64,
            dropped: self.inner.dropped.load(Ordering::SeqCst) as u64,
            write_errors: self.inner.write_errors.load(Ordering::SeqCst) as u64,
            queue_depth: self.inner.tx.len(),
            reels: self.inner.reels.load(Ordering::SeqCst) as u64,
            hard_examples: self.inner.hard.load(Ordering::SeqCst) as u64,
            last_write_ms: self.inner.last_write_ms.load(Ordering::SeqCst),
        }
    }

    /// Wall-clock ms of the last successful frame write (0 = none yet).
    pub fn last_write_ms(&self) -> u64 {
        self.inner.last_write_ms.load(Ordering::SeqCst)
    }

    /// Ensure the versioned dataset layout exists (idempotent). Used by the
    /// start-up preflight so a missing/unwritable dataset blocks Start with
    /// a visible error instead of silently recording zero frames.
    pub fn ensure_layout(&self) -> Result<(), String> {
        self.dataset.init()
    }

    /// Dataset samples recorded so far (all sessions).
    pub fn dataset_samples(&self) -> usize {
        self.dataset.annotations().len()
    }

    /// Samples carrying a knowledge-base entity (the readiness "verified").
    pub fn dataset_verified(&self) -> usize {
        self.dataset
            .annotations()
            .iter()
            .filter(|a| a.entity_id.is_some())
            .count()
    }

    /// Startup crash recovery: sessions left `complete:false` are marked
    /// recovered-incomplete (rows/images untouched). The currently active
    /// session (if any) is never touched. Returns the count.
    pub fn recover_incomplete(&self) -> usize {
        let active = self.active_session_id();
        let mut recovered = 0usize;
        let Ok(rd) = std::fs::read_dir(&self.sessions_dir) else {
            return 0;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json") {
                if let Ok(raw) = std::fs::read_to_string(&p) {
                    if let Ok(mut meta) = serde_json::from_str::<SessionMeta>(&raw) {
                        if Some(meta.session_id.as_str()) == active.as_deref() {
                            continue;
                        }
                        if !meta.complete && !meta.recovered_incomplete {
                            meta.recovered_incomplete = true;
                            meta.stopped_ms = Some(now_ms());
                            // Rewrite in place (atomic tmp+rename).
                            let tmp = p.with_extension("json.tmp");
                            if serde_json::to_vec_pretty(&meta)
                                .ok()
                                .and_then(|b| std::fs::write(&tmp, &b).ok())
                                .and_then(|_| std::fs::rename(&tmp, &p).ok())
                                .is_some()
                            {
                                recovered += 1;
                            }
                        }
                    }
                }
            }
        }
        recovered
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Unique session id: `session_YYYY-MM-DD_HHMMSS_<8hex>`.
/// Date/time from wall clock; suffix mixes wall time, pid, and a process
/// counter so back-to-back calls in the same millisecond still differ
/// (uniqueness only, not security).
pub fn new_session_id() -> String {
    static SALT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ms = now_ms();
    let secs = ms / 1000;
    let (y, mo, d, h, mi, s) = civil_datetime(secs);
    let salt = SALT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut hsh = secs
        ^ ((ms % 1000) << 32)
        ^ (std::process::id() as u64).wrapping_mul(0x9E3779B97F4A7C15)
        ^ salt.wrapping_mul(0xC2B2AE3D27D4EB4F);
    hsh ^= hsh >> 29;
    hsh = hsh.wrapping_mul(0xBF58476D1CE4E5B9);
    hsh ^= hsh >> 32;
    format!("session_{y:04}-{mo:02}-{d:02}_{h:02}{mi:02}{s:02}_{:08x}", (hsh & 0xFFFF_FFFF) as u32)
}

fn civil_datetime(mut secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let s = secs % 60;
    secs /= 60;
    let mi = secs % 60;
    secs /= 60;
    let h = secs % 24;
    let mut days = secs / 24;
    let mut y = 1970u64;
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
        let diy = if leap { 366 } else { 365 };
        if days < diy {
            break;
        }
        days -= diy;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
    let md = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mo = 1u64;
    for m in md {
        if days < m {
            break;
        }
        days -= m;
        mo += 1;
    }
    (y, mo, days + 1, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collector(name: &str) -> (MlCollector, PathBuf) {
        static C: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = C.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("gpo-mlsess-{name}-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        // Isolated: no writer thread; drive writes synchronously via dataset.
        let c = MlCollector::new_isolated(dir.clone());
        (c, dir)
    }

    fn png() -> Vec<u8> {
        // 16x16 gradient (non-uniform → stable distinct hash).
        let mut rgba = vec![0u8; 16 * 16 * 4];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            px[0] = (i % 256) as u8;
            px[1] = ((i * 3) % 256) as u8;
            px[2] = ((i * 7) % 256) as u8;
            px[3] = 255;
        }
        crate::core::types::Frame::new(16, 16, rgba).to_png_bytes().unwrap()
    }

    #[test]
    fn session_ids_are_unique_and_shaped() {
        let a = new_session_id();
        let b = new_session_id();
        assert_ne!(a, b);
        assert!(a.starts_with("session_20"));
        assert_eq!(a.split('_').count(), 4);
    }

    #[test]
    fn begin_is_idempotent_and_stop_finalizes() {
        let (c, _d) = collector("lifecycle");
        assert!(!c.is_collecting());
        let a = c.begin_session("9.9.9");
        let b = c.begin_session("9.9.9");
        assert_eq!(a, b, "double start keeps the session");
        assert!(c.is_collecting());
        let meta_path = c.sessions_dir.join(format!("{a}.json"));
        let raw: SessionMeta = serde_json::from_str(&std::fs::read_to_string(&meta_path).unwrap()).unwrap();
        assert!(!raw.complete);
        let summary = c.end_session().expect("summary");
        assert_eq!(summary.session_id, a);
        assert!(!c.is_collecting());
        // Second stop: no active session → None (no fake summary).
        assert!(c.end_session().is_none());
        let raw2: SessionMeta = serde_json::from_str(&std::fs::read_to_string(&meta_path).unwrap()).unwrap();
        assert!(raw2.complete);
    }

    #[test]
    fn restart_creates_new_session_and_appends() {
        let (c, _d) = collector("restart");
        let a = c.begin_session("1.0");
        c.end_session();
        let b = c.begin_session("1.0");
        assert_ne!(a, b);
        c.end_session();
        // Both session files persist (append-only history).
        assert!(c.sessions_dir.join(format!("{a}.json")).exists());
        assert!(c.sessions_dir.join(format!("{b}.json")).exists());
    }

    #[test]
    fn crash_recovery_marks_incomplete_without_deleting() {
        let (c, _dir) = collector("crash");
        let live = c.begin_session("1.0");
        // A stale session file from a killed run (complete:false, not active).
        let stale_id = "session_2001-01-01_000000_deadbeef";
        let stale = SessionMeta {
            session_id: stale_id.into(),
            started_ms: 1,
            stopped_ms: None,
            app_version: "1.0".into(),
            capture: CaptureConfig {
                task: "t".into(),
                regions: vec![],
                trace_gated: true,
            },
            samples: 7,
            reels: 2,
            hard_examples: 1,
            dropped: 0,
            complete: false,
            recovered_incomplete: false,
        };
        std::fs::create_dir_all(&c.sessions_dir).unwrap();
        std::fs::write(
            c.session_path(stale_id),
            serde_json::to_vec_pretty(&stale).unwrap(),
        )
        .unwrap();
        assert_eq!(c.recover_incomplete(), 1);
        // Live session untouched; stale marked recovered, counts preserved.
        assert!(c.is_collecting());
        assert_eq!(c.active_session_id().as_deref(), Some(live.as_str()));
        let back: SessionMeta =
            serde_json::from_str(&std::fs::read_to_string(c.session_path(stale_id)).unwrap()).unwrap();
        assert!(back.recovered_incomplete);
        assert!(!back.complete);
        assert_eq!(back.samples, 7);
        // Second recovery is a no-op (idempotent).
        assert_eq!(c.recover_incomplete(), 0);
    }

    #[test]
    fn async_submit_flushes_on_end() {
        // Full path with the real background writer: begin → submit →
        // end_session flushes and the rows/images exist on disk.
        // Nanos-unique dir: hermetic across repeated runs (no leftovers).
        let uniq = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("gpo-mlsess-e2e-{uniq}"));
        let _ = std::fs::remove_dir_all(&dir);
        let c = MlCollector::new(dir.clone());
        // Sanity: collector root exists (constructor ran fully).
        assert!(c.sessions_dir.exists(), "sessions dir must exist after new()");
        let id = c.begin_session("9.9.9");
        for i in 0..3 {
            // Structurally distinct frames per sample (varying gradient +
            // contrast block position) so perceptual dedup keeps all three.
            // Never touch the 8-byte PNG signature.
            let mut rgba = vec![0u8; 16 * 16 * 4];
            for (p, px) in rgba.chunks_exact_mut(4).enumerate() {
                let x = (p % 16) as u8;
                let bright = x.wrapping_mul((i as u8 + 1).wrapping_mul(37));
                px[0] = bright;
                px[1] = 200u8.wrapping_sub(bright / 2);
                px[2] = (i as u8).wrapping_mul(90);
                px[3] = 255;
            }
            let frame = crate::core::types::Frame::new(16, 16, rgba);
            let png = frame.to_png_bytes().unwrap();
            assert!(c.submit(SampleJob {
                png,
                session_id: String::new(),
                task: crate::core::ml_dataset::MlTask::EntityRecognition,
                ui_label: None,
                game_state: Some(crate::core::ml_dataset::GameStateLabel::CatchResult),
                ocr_text: format!("sample {i}"),
                region: "drop".into(),
                entity_id: None,
                hard_reason: if i == 2 { Some("perception unknown".into()) } else { None },
                source: "test".into(),
            }));
        }
        // end_session's flush (not this test) is responsible for draining;
        // if the writer is broken this assert times out via FLUSH_TIMEOUT
        // and the summary reports it instead of hanging.
        let summary = c.end_session().expect("summary");
        assert_eq!(summary.session_id, id);
        assert_eq!(summary.samples, 3, "all queued samples flushed, got {:?}", summary);
        assert_eq!(summary.hard_examples, 1);
        assert_eq!(summary.dropped, 0);
        assert!(summary.quality_ok);
        // Rows + images on disk; dataset listable.
        let ds = crate::core::ml_dataset::MlDatasetStore::new(dir);
        assert_eq!(ds.annotations().len(), 3);
    }

    #[test]
    fn finalized_session_validates_and_attributes_run() {
        // Stop → session finalized, manifest complete, validation sees the
        // samples under THIS run's session id (per-run attribution: splits
        // hash the full run id, so one run never leaks across sets).
        let uniq = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("gpo-mlsess-final-{uniq}"));
        let _ = std::fs::remove_dir_all(&dir);
        let c = MlCollector::new(dir.clone());
        let id = c.begin_session("9.9.9");
        assert!(c.submit(SampleJob {
            png: png(),
            session_id: String::new(),
            task: crate::core::ml_dataset::MlTask::UiDetection,
            ui_label: Some(crate::core::ml_dataset::UiLabel::FishingBar),
            game_state: Some(crate::core::ml_dataset::GameStateLabel::WaitingForBite),
            ocr_text: String::new(),
            region: "bar".into(),
            entity_id: None,
            hard_reason: None,
            source: "test".into(),
        }));
        let summary = c.end_session().expect("summary");
        assert_eq!(summary.samples, 1);
        assert!(c.last_write_ms() > 0, "first-frame health signal must be set");
        // Manifest complete on disk.
        let meta: SessionMeta = serde_json::from_str(
            &std::fs::read_to_string(c.session_path(&id)).expect("session file"),
        )
        .unwrap();
        assert!(meta.complete);
        assert_eq!(meta.samples, 1);
        // Validation sees exactly this run's sample; image decodes.
        let ds = crate::core::ml_dataset::MlDatasetStore::new(dir.clone());
        let rows = ds.annotations();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id, id, "annotation must carry the run session id");
        let p = ds.root().join("images").join(format!("{}.png", rows[0].image_id));
        assert!(crate::core::ml_dataset::png_file_valid(&p), "sample PNG must decode");
        let rep = ds.validate(crate::core::knowledge::KnowledgeBase::bundled());
        assert!(rep.ok, "single clean sample must validate: {rep:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn submit_without_session_drops_without_error() {
        let (c, _d) = collector("nosess");
        let job = SampleJob {
            png: png(),
            session_id: String::new(),
            task: crate::core::ml_dataset::MlTask::UiDetection,
            ui_label: None,
            game_state: None,
            ocr_text: String::new(),
            region: "bar".into(),
            entity_id: None,
            hard_reason: None,
            source: "test".into(),
        };
        assert!(!c.submit(job));
        assert_eq!(c.stats_snapshot().dropped, 1);
    }
}
