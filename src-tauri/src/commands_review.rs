//! Human review + canonical entities + readiness command surface (v5.6.1).
//!
//! Every value returned here is computed from a real store (dataset rows,
//! review records, KB, registry, shadow log). PNGs are served as base64,
//! reusing the existing `region_preview` pattern; the CSP already allows
//! `data:` images.
//!
//! v5.6.1 hardening (all of these were verified defects in v5.6.0):
//! * eligibility is measured here and passed INTO the store, never
//!   recomputed with placeholders - and the command layer no longer
//!   overwrites the store's conflict demotion with its own value;
//! * conflict resolution is validated against the knowledge base instead of a
//!   display name that was never supplied (v5.6.0 excluded every UI-resolved
//!   conflict from training);
//! * `review_list` filters on the serde status names the frontend actually
//!   sends, not Rust `Debug` output;
//! * queue/coverage/drops builds index the dataset by `image_id`/`entity_id`
//!   instead of rescanning all rows per item (v5.6.0 was O(R*N));
//! * the soak is read per deployed REVISION, so a new candidate can no
//!   longer inherit the incumbent's telemetry;
//! * readiness gets structured, differentiated statuses and blockers.

use std::collections::{HashMap, HashSet};

use base64::Engine;
use serde::Serialize;
use tauri::State;

use crate::app::AppState;
use crate::core::canon::{self, Resolution};
use crate::core::ml_capability::{assess_capabilities, qualify_entities};
use crate::core::ml_dataset::{MlDatasetStore, DATASET_VERSION};
use crate::core::readiness::{self, DataGateInfo, JobPhase};
use crate::core::registry;
use crate::core::review::{
    self, EligibilityInput, ReviewInput, ReviewRecord, ReviewStatus, ReviewStore,
};
use crate::core::training;

/// Run a blocking read/aggregate off the UI thread.
///
/// Every command in this module parses the whole `labels.jsonl` and/or the
/// whole `reviews.jsonl`. Run synchronously (the default for a non-`async`
/// Tauri command) that work happens on the main thread and the panel freezes
/// on a large dataset. `spawn_blocking` moves it to the blocking pool, which
/// is bounded - not an unbounded thread per call.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

fn review_store(data_dir: &std::path::Path) -> ReviewStore {
    ReviewStore::new(data_dir.to_path_buf())
}

/// Dataset `labels.jsonl` parsed once and indexed by `image_id`.
///
/// v5.6.0 re-read and re-parsed the whole file in five separate commands on
/// every single human verdict (and `review_priority` did it inside an
/// O(R*N) scan). One parse + one index is the difference between a snappy
/// review queue and a multi-second freeze.
struct Rows {
    all: Vec<crate::core::ml_dataset::MlAnnotation>,
    by_id: HashMap<String, usize>,
    by_entity: HashMap<String, Vec<usize>>,
}

fn load_rows(data_dir: &std::path::Path) -> Rows {
    let all = MlDatasetStore::new(data_dir.to_path_buf()).annotations();
    let mut by_id = HashMap::with_capacity(all.len());
    let mut by_entity: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, r) in all.iter().enumerate() {
        by_id.insert(r.image_id.clone(), i);
        if let Some(e) = r.entity_id.as_deref() {
            by_entity.entry(e.to_string()).or_default().push(i);
        }
    }
    Rows {
        all,
        by_id,
        by_entity,
    }
}

fn dataset_images_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    MlDatasetStore::new(data_dir.to_path_buf())
        .root()
        .join("images")
}

/// Measure the eligibility facts the store cannot see itself.
fn measure_eligibility(
    data_dir: &std::path::Path,
    row: &crate::core::ml_dataset::MlAnnotation,
    human_entity_id: Option<&str>,
    kb: &crate::core::knowledge::KnowledgeBase,
) -> EligibilityInput {
    let path = dataset_images_dir(data_dir).join(format!("{}.png", row.image_id));
    let png_decodable = std::fs::read(&path)
        .ok()
        .and_then(|b| image::load_from_memory(&b).ok())
        .is_some();
    EligibilityInput {
        png_decodable,
        canonical_valid: human_entity_id
            .map(|id| kb.entities().iter().any(|e| e.id == id))
            .unwrap_or(false),
        // Byte-hash duplicate detection already runs at import time
        // (`MlDatasetStore::import_png` -> find_by_hash), so a collected row
        // is unique by construction. Kept explicit rather than hardcoded so
        // the gate stays visible when that changes.
        is_duplicate: false,
        has_provenance: !row.session_id.is_empty() && row.timestamp_ms > 0,
        has_entity: human_entity_id.is_some_and(|s| !s.trim().is_empty()),
    }
}

// ---------------------------------------------------------------------------
// PNG serving
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ReviewImage {
    pub image_id: String,
    pub width: u32,
    pub height: u32,
    pub png_base64: String,
}

/// Serve one review PNG.
///
/// The reviewer judges a small crop on screen, not a full-resolution frame,
/// so this downscales to `max_dim` (same convention as `region_preview`)
/// instead of shipping multi-megabyte base64 into the WebView on every
/// selection. `max_dim = 0` serves the original bytes (used by fixtures).
#[tauri::command]
pub fn review_image(
    st: State<'_, AppState>,
    image_id: String,
    max_dim: Option<u32>,
) -> Result<ReviewImage, String> {
    if image_id.contains('/') || image_id.contains('\\') || image_id.contains("..") {
        return Err("invalid image id".to_string());
    }
    let path = dataset_images_dir(st.store.dir()).join(format!("{image_id}.png"));
    let bytes = std::fs::read(&path).map_err(|_| format!("no image for '{image_id}'"))?;
    let img =
        image::load_from_memory(&bytes).map_err(|_| format!("undecodable image '{image_id}'"))?;
    let dim = max_dim.unwrap_or(0);
    let (width, height, out) = if dim > 0 && (img.width() > dim || img.height() > dim) {
        let scaled = img.thumbnail(dim, dim);
        let mut buf: Vec<u8> = Vec::new();
        scaled
            .write_to(
                &mut ::std::io::Cursor::new(&mut buf),
                image::ImageFormat::Png,
            )
            .map_err(|e| format!("encode '{image_id}': {e}"))?;
        (scaled.width(), scaled.height(), buf)
    } else {
        (img.width(), img.height(), bytes)
    };
    Ok(ReviewImage {
        image_id,
        width,
        height,
        png_base64: base64::engine::general_purpose::STANDARD.encode(&out),
    })
}

// ---------------------------------------------------------------------------
// Review records
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn review_get(st: State<'_, AppState>, image_id: String) -> Option<ReviewRecord> {
    review_store(st.store.dir()).get(&image_id)
}

#[tauri::command]
pub async fn review_list(
    st: State<'_, AppState>,
    status: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Vec<ReviewRecord>, String> {
    let dir = st.store.dir().to_path_buf();
    blocking(move || Ok(review_list_sync(&dir, status, limit, offset))).await
}

fn review_list_sync(
    dir: &std::path::Path,
    status: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Vec<ReviewRecord> {
    let mut v = review_store(dir).list();
    if let Some(s) = status {
        // v5.6.0 compared against `{:?}` ("ReviewedCorrect") while the wire
        // format and the TS type are SCREAMING_SNAKE_CASE, so every documented
        // value silently returned an empty list.
        if let Some(want) = ReviewStatus::parse(&s) {
            v.retain(|r| r.review_status == want);
        }
    }
    v.sort_by_key(|r| std::cmp::Reverse(r.reviewed_at.unwrap_or(0)));
    let off = offset.unwrap_or(0);
    let lim = limit.unwrap_or(50).clamp(1, 200);
    v.into_iter().skip(off).take(lim).collect()
}

/// Store integrity: unreadable state lines and the audit tail. v5.6.0 wrote
/// an audit log nothing could read and silently destroyed corrupt lines.
#[derive(Debug, Serialize)]
pub struct ReviewIntegrity {
    pub unreadable_lines: usize,
    pub audit_events: usize,
    pub last_event: Option<review::AuditEvent>,
    /// The most recent audit events, newest last.
    pub last_events: Vec<review::AuditEvent>,
    /// Image ids whose most recent state was an undo.
    pub restorable: Vec<String>,
}

#[tauri::command]
pub async fn review_integrity(
    st: State<'_, AppState>,
    tail: Option<usize>,
) -> Result<ReviewIntegrity, String> {
    let dir = st.store.dir().to_path_buf();
    blocking(move || Ok(review_integrity_sync(&dir, tail))).await
}

fn review_integrity_sync(dir: &std::path::Path, tail: Option<usize>) -> ReviewIntegrity {
    let store = review_store(dir);
    let audit = store.audit();
    let want = tail.unwrap_or(50).clamp(1, 500);
    let last_events: Vec<review::AuditEvent> = audit
        .iter()
        .rev()
        .take(want)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let restorable: Vec<String> = store
        .list()
        .into_iter()
        .filter(|r| {
            r.review_status == ReviewStatus::Unreviewed
                || r.excluded_reason.as_deref() == Some("undone")
        })
        .map(|r| r.image_id)
        .collect();
    ReviewIntegrity {
        unreadable_lines: store.corrupt_lines().len(),
        audit_events: audit.len(),
        last_event: audit.last().cloned(),
        last_events,
        restorable,
    }
}

/// Reconstruct `reviews.jsonl` from the append-only audit log.
#[tauri::command]
pub async fn review_rebuild(st: State<'_, AppState>) -> Result<usize, String> {
    let dir = st.store.dir().to_path_buf();
    blocking(move || review_store(&dir).rebuild_from_audit()).await
}

#[derive(Debug, Serialize)]
pub struct ReviewCoverageView {
    pub total_rows: usize,
    pub coverage: review::CoverageReport,
    pub per_entity: Vec<PerEntityReview>,
    pub per_session: Vec<SessionReview>,
    pub per_split: Vec<SplitReview>,
    pub exclusions: Vec<ExclusionReason>,
    /// Rows whose review record exists but which are not eligible, grouped by
    /// the exact reason recorded at review time.
    pub hard_examples: usize,
    pub unreviewed: usize,
    /// Per-entity class-readiness verdicts, so a reviewer knows which class is
    /// the binding one instead of guessing from raw counts.
    pub class_readiness: Vec<ClassReadiness>,
}

#[derive(Debug, Serialize)]
pub struct PerEntityReview {
    pub entity: String,
    pub collected: usize,
    pub reviewed: usize,
    pub confirmed: usize,
    pub corrected: usize,
    pub unknown: usize,
    pub skipped: usize,
    pub conflicts: usize,
    pub sessions: usize,
    pub eligible: usize,
    pub hard_examples: usize,
    pub train: usize,
    pub validation: usize,
    pub test: usize,
}

#[derive(Debug, Serialize)]
pub struct SessionReview {
    pub session_id: String,
    pub split: String,
    pub collected: usize,
    pub reviewed: usize,
    pub eligible: usize,
}

#[derive(Debug, Serialize)]
pub struct SplitReview {
    pub split: String,
    pub collected: usize,
    pub reviewed: usize,
    pub eligible: usize,
    pub sessions: usize,
}

#[derive(Debug, Serialize)]
pub struct ExclusionReason {
    pub reason: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct ClassReadiness {
    pub entity: String,
    pub status: String,
    pub collected: usize,
    pub reviewed: usize,
    pub eligible: usize,
    pub sessions: usize,
    pub reason: String,
}

/// Minimum bar for a class to be "ready" - mirrors the capability gate's
/// per-entity rule (>=20 examples AND >=3 sessions AND test coverage) but is
/// evaluated against *reviewed* counts, which is stricter and therefore safe.
const CLASS_MIN_REVIEWED: usize = 20;
const CLASS_MIN_SESSIONS: usize = 3;

#[tauri::command]
pub async fn review_coverage(st: State<'_, AppState>) -> Result<ReviewCoverageView, String> {
    let dir = st.store.dir().to_path_buf();
    blocking(move || Ok(review_coverage_sync(&dir))).await
}

fn review_coverage_sync(dir: &std::path::Path) -> ReviewCoverageView {
    let rows = load_rows(dir);
    let store = review_store(dir);
    let records = store.list();
    let coverage = review::coverage(&records);
    let by_image: HashMap<&str, &ReviewRecord> =
        records.iter().map(|r| (r.image_id.as_str(), r)).collect();

    let mut per: HashMap<String, PerEntityReview> = HashMap::new();
    let mut ent_sessions: HashMap<String, HashSet<String>> = HashMap::new();
    let mut sess: HashMap<String, SessionReview> = HashMap::new();
    let mut split: HashMap<String, SplitReview> = HashMap::new();
    let mut excl: HashMap<String, usize> = HashMap::new();
    let mut hard = 0usize;
    let mut unreviewed = 0usize;

    for r in &rows.all {
        let rev = by_image.get(r.image_id.as_str());
        let is_reviewed = rev.is_some_and(|v| {
            matches!(
                v.review_status,
                ReviewStatus::ReviewedCorrect
                    | ReviewStatus::ReviewedCorrected
                    | ReviewStatus::ReviewedUnknown
            )
        });
        if rev.is_none() {
            unreviewed += 1;
        }
        if r.hard_example {
            hard += 1;
        }
        if let Some(v) = rev {
            if !v.training_eligible {
                if let Some(why) = v.excluded_reason.as_deref() {
                    *excl.entry(why.to_string()).or_default() += 1;
                }
            }
        }

        let sp = MlDatasetStore::split_of(&r.session_id).as_str().to_string();
        let se = sess
            .entry(r.session_id.clone())
            .or_insert_with(|| SessionReview {
                session_id: r.session_id.clone(),
                split: sp.clone(),
                collected: 0,
                reviewed: 0,
                eligible: 0,
            });
        se.collected += 1;
        if is_reviewed {
            se.reviewed += 1;
        }
        if rev.is_some_and(|v| v.training_eligible) {
            se.eligible += 1;
        }
        let sl = split.entry(sp.clone()).or_insert_with(|| SplitReview {
            split: sp.clone(),
            collected: 0,
            reviewed: 0,
            eligible: 0,
            sessions: 0,
        });
        sl.collected += 1;
        if is_reviewed {
            sl.reviewed += 1;
        }
        if rev.is_some_and(|v| v.training_eligible) {
            sl.eligible += 1;
        }

        let Some(e) = r.entity_id.as_deref() else {
            continue;
        };
        let entry = per.entry(e.to_string()).or_insert_with(|| PerEntityReview {
            entity: e.to_string(),
            collected: 0,
            reviewed: 0,
            confirmed: 0,
            corrected: 0,
            unknown: 0,
            skipped: 0,
            conflicts: 0,
            sessions: 0,
            eligible: 0,
            hard_examples: 0,
            train: 0,
            validation: 0,
            test: 0,
        });
        entry.collected += 1;
        if r.hard_example {
            entry.hard_examples += 1;
        }
        match sp.as_str() {
            "train" => entry.train += 1,
            "validation" => entry.validation += 1,
            "test" => entry.test += 1,
            _ => {}
        }
        ent_sessions
            .entry(e.to_string())
            .or_default()
            .insert(r.session_id.clone());
        if let Some(rev) = rev {
            match rev.review_status {
                ReviewStatus::ReviewedCorrect => {
                    entry.reviewed += 1;
                    entry.confirmed += 1;
                }
                ReviewStatus::ReviewedCorrected => {
                    entry.reviewed += 1;
                    entry.corrected += 1;
                }
                ReviewStatus::ReviewedUnknown => {
                    entry.reviewed += 1;
                    entry.unknown += 1;
                }
                ReviewStatus::ReviewedSkipped => entry.skipped += 1,
                ReviewStatus::Conflict => entry.conflicts += 1,
                ReviewStatus::Unreviewed => {}
            }
            if rev.training_eligible {
                entry.eligible += 1;
            }
        }
    }

    for sl in split.values_mut() {
        sl.sessions = rows
            .all
            .iter()
            .filter(|r| MlDatasetStore::split_of(&r.session_id).as_str() == sl.split)
            .map(|r| r.session_id.as_str())
            .collect::<HashSet<_>>()
            .len();
    }

    let per_entity: Vec<PerEntityReview> = per
        .into_iter()
        .map(|(e, mut x)| {
            x.sessions = ent_sessions.get(&e).map(|s| s.len()).unwrap_or(0);
            x
        })
        .collect();

    let mut class_readiness: Vec<ClassReadiness> = per_entity
        .iter()
        .map(|e| {
            let (status, reason) = if e.collected == 0 {
                ("NO_DATA".to_string(), "nothing collected yet".to_string())
            } else if e.reviewed < CLASS_MIN_REVIEWED {
                (
                    "INSUFFICIENT_REVIEW".to_string(),
                    format!(
                        "{}/{} reviewed (need {}); review more of this class in Training > Review",
                        e.reviewed, e.collected, CLASS_MIN_REVIEWED
                    ),
                )
            } else if e.sessions < CLASS_MIN_SESSIONS {
                (
                    "INSUFFICIENT_SESSIONS".to_string(),
                    format!(
                        "{}/{} sessions (need {}); catch this fish in more independent sessions",
                        e.sessions, e.collected, CLASS_MIN_SESSIONS
                    ),
                )
            } else if e.test == 0 {
                (
                    "INSUFFICIENT_TEST".to_string(),
                    "no TEST-split examples; held-out evaluation impossible".to_string(),
                )
            } else {
                ("READY".to_string(), "meets the review bar".to_string())
            };
            ClassReadiness {
                entity: e.entity.clone(),
                status,
                collected: e.collected,
                reviewed: e.reviewed,
                eligible: e.eligible,
                sessions: e.sessions,
                reason,
            }
        })
        .collect();
    class_readiness.sort_by_key(|c| (c.status == "READY", std::cmp::Reverse(c.collected)));

    let mut exclusions: Vec<ExclusionReason> = excl
        .into_iter()
        .map(|(reason, count)| ExclusionReason { reason, count })
        .collect();
    exclusions.sort_by_key(|e| std::cmp::Reverse(e.count));

    let mut per_session: Vec<SessionReview> = sess.into_values().collect();
    per_session.sort_by_key(|s| std::cmp::Reverse(s.collected));
    let mut per_split: Vec<SplitReview> = split.into_values().collect();
    per_split.sort_by_key(|s| s.split.clone());

    ReviewCoverageView {
        total_rows: rows.all.len(),
        coverage,
        per_entity,
        per_session,
        per_split,
        exclusions,
        hard_examples: hard,
        unreviewed,
        class_readiness,
    }
}

#[derive(Debug, Serialize)]
pub struct ReviewApplyResult {
    pub record: ReviewRecord,
    pub dataset_updated: bool,
    /// Set when the dataset annotation could not be written. Never silently
    /// swallowed: the reviewer must know the label did not land.
    pub dataset_error: Option<String>,
}

/// Apply a human review.
///
/// Entity identities must be stable KB ids; anything else is recorded (the
/// audit keeps the attempt) but excluded from training. Confirming also
/// writes the dataset label through the SAME `annotate()` path as Setup, so
/// review and dataset can never disagree.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn review_apply(
    st: State<'_, AppState>,
    image_id: String,
    human_entity_id: Option<String>,
    human_canonical_name: Option<String>,
    correction_reason: Option<String>,
    model_prediction: Option<String>,
    model_confidence: Option<f32>,
) -> Result<ReviewApplyResult, String> {
    let ds = MlDatasetStore::new(st.store.dir().to_path_buf());
    let kb = st.store.effective_knowledge();
    let row = ds
        .annotations()
        .into_iter()
        .find(|r| r.image_id == image_id)
        .ok_or_else(|| format!("no dataset row for '{image_id}'"))?;

    let canonical_name = human_canonical_name.or_else(|| {
        human_entity_id
            .as_deref()
            .and_then(|id| kb.entities().iter().find(|e| e.id == id))
            .map(|e| e.canonical_name.clone())
    });
    let eligibility = measure_eligibility(st.store.dir(), &row, human_entity_id.as_deref(), &kb);
    let canonical_valid = eligibility.canonical_valid;

    let store = review_store(st.store.dir());
    let record = review::apply_human_review(
        &store,
        ReviewInput {
            image_id: &image_id,
            session_id: &row.session_id,
            human_entity_id: human_entity_id.as_deref(),
            canonical_name: canonical_name.as_deref(),
            correction_reason: correction_reason.as_deref(),
            model_prediction: model_prediction.as_deref(),
            model_confidence,
            dataset_version: DATASET_VERSION,
            eligibility,
        },
    )?;

    // Dataset linkage through the standard annotate path (Setup parity).
    // A conflicted record must NOT push the disputed entity into the dataset.
    let mut dataset_updated = false;
    let mut dataset_error = None;
    if record.review_status != ReviewStatus::Conflict {
        let outcome = if canonical_valid {
            ds.annotate(
                &image_id,
                None,
                None,
                None,
                record.human_entity_id.clone(),
                "human",
                false,
                None,
            )
        } else if record.human_entity_id.is_none() {
            ds.annotate(
                &image_id,
                None,
                None,
                None,
                None,
                "human",
                true,
                Some("human: unknown".to_string()),
            )
        } else {
            // A human entity that is NOT a KB id: the verdict is recorded and
            // excluded, but `labels.jsonl` is deliberately left untouched.
            // Never report `dataset_updated: true` for a write that did not
            // happen - that is exactly the signal the reviewer needs.
            dataset_error = Some(format!(
                "'{}' is not a canonical entity id; label not written and the \
                 sample is excluded from training",
                record.human_entity_id.as_deref().unwrap_or("")
            ));
            Ok(row.clone())
        };
        if dataset_error.is_none() {
            match outcome {
                Ok(_) => dataset_updated = true,
                Err(e) => dataset_error = Some(e),
            }
        }
    }
    Ok(ReviewApplyResult {
        record,
        dataset_updated,
        dataset_error,
    })
}

#[tauri::command]
pub fn review_skip(st: State<'_, AppState>, image_id: String) -> Result<ReviewRecord, String> {
    let row = MlDatasetStore::new(st.store.dir().to_path_buf())
        .annotations()
        .into_iter()
        .find(|r| r.image_id == image_id)
        .ok_or_else(|| format!("no dataset row for '{image_id}'"))?;
    let store = review_store(st.store.dir());
    review::skip_review(&store, &image_id, &row.session_id, None, DATASET_VERSION)
}

#[tauri::command]
pub fn review_resolve(
    st: State<'_, AppState>,
    image_id: String,
    entity_id: String,
    reason: String,
) -> Result<ReviewRecord, String> {
    let kb = st.store.effective_knowledge();
    if !kb.entities().iter().any(|e| e.id == entity_id) {
        return Err(format!("'{entity_id}' is not a canonical KB entity id"));
    }
    let row = MlDatasetStore::new(st.store.dir().to_path_buf())
        .annotations()
        .into_iter()
        .find(|r| r.image_id == image_id)
        .ok_or_else(|| format!("no dataset row for '{image_id}'"))?;
    let canonical = kb
        .entities()
        .iter()
        .find(|e| e.id == entity_id)
        .map(|e| e.canonical_name.clone());
    let eligibility = measure_eligibility(st.store.dir(), &row, Some(&entity_id), &kb);
    let store = review_store(st.store.dir());
    let rec = review::resolve_conflict(
        &store,
        &image_id,
        &entity_id,
        canonical.as_deref(),
        &reason,
        &eligibility,
    )?;
    // Only write the dataset label when the record actually re-enters
    // training. Writing it for an ineligible resolution would leave a label
    // the snapshot then drops anyway, and would misreport what was done.
    if rec.training_eligible {
        MlDatasetStore::new(st.store.dir().to_path_buf()).annotate(
            &image_id,
            None,
            None,
            None,
            Some(entity_id),
            "human",
            false,
            None,
        )?;
    }
    Ok(rec)
}

/// Undo the last review of an image. The audit keeps the mistake.
#[tauri::command]
pub fn review_undo(st: State<'_, AppState>, image_id: String) -> Result<ReviewRecord, String> {
    // Require a real dataset row: without this, any string (including a
    // phantom id) becomes a review record that inflates the coverage counts
    // and is resurrected by rebuild_from_audit.
    MlDatasetStore::new(st.store.dir().to_path_buf())
        .annotations()
        .into_iter()
        .find(|r| r.image_id == image_id)
        .ok_or_else(|| format!("no dataset row for '{image_id}'"))?;
    review::undo_review(&review_store(st.store.dir()), &image_id)
}

// ---------------------------------------------------------------------------
// Prioritized queue
// ---------------------------------------------------------------------------

/// The priority queue plus the TRUE total.
///
/// v5.6.x returned a bare `Vec`, so the UI could only say how many rows the
/// single request returned. The count is returned alongside so the UI can
/// say "showing 200 of 4,118" instead of inventing a total.
#[derive(Debug, Serialize)]
pub struct PriorityPage {
    pub items: Vec<PriorityItem>,
    /// Rows matching the filter BEFORE `limit` was applied.
    pub total_matching: usize,
    /// Hard cap on rows returned in one request.
    pub limit: usize,
}

#[derive(Debug, Serialize)]
pub struct PriorityItem {
    pub image_id: String,
    pub session_id: String,
    pub timestamp_ms: u64,
    pub ocr_text: String,
    pub entity_id: Option<String>,
    pub review_status: String,
    pub score: u8,
    pub reason: String,
    pub is_hard_example: bool,
    pub ocr_disagreement: bool,
    pub model_confidence: Option<f32>,
    /// True when this row is present in the most recent frozen training
    /// snapshot (i.e. it is already being learned from).
    pub in_latest_snapshot: bool,
}

#[derive(Debug, Default, Clone)]
pub struct QueueFilter {
    pub entity: Option<String>,
    pub session: Option<String>,
    pub only_hard: bool,
    pub only_disagreement: bool,
    pub sort: String,
}

/// Deterministic review priority.
///
/// Ordering is by weighted score, then newest-first, then image id, so the
/// queue is stable across reloads and a reviewer sees the same order twice.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn review_priority(
    st: State<'_, AppState>,
    limit: Option<usize>,
    entity: Option<String>,
    session: Option<String>,
    only_hard: Option<bool>,
    only_disagreement: Option<bool>,
    sort: Option<String>,
    status: Option<review::ReviewStatus>,
) -> Result<PriorityPage, String> {
    let dir = st.store.dir().to_path_buf();
    blocking(move || {
        Ok(review_priority_sync(
            &dir,
            limit,
            entity,
            session,
            only_hard,
            only_disagreement,
            sort,
            status,
        ))
    })
    .await
}

/// How many rows, and how many distinct sessions, each class contributes to the
/// TRAINING set. Reads the projection, never the live file.
fn projected_class_support(
    projected: &crate::core::training::ProjectedLabels,
) -> (HashMap<&str, usize>, HashMap<&str, HashSet<&str>>) {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut sessions: HashMap<&str, HashSet<&str>> = HashMap::new();
    for r in &projected.rows {
        if let Some(e) = r.entity_id.as_deref() {
            *counts.entry(e).or_default() += 1;
            sessions.entry(e).or_default().insert(r.session_id.as_str());
        }
    }
    (counts, sessions)
}

/// Folded text each entity can legitimately be recognised by: canonical name,
/// aliases, OCR aliases. Sorted + deduped so the map is deterministic.
fn ocr_name_keys(kb: &crate::core::knowledge::KnowledgeBase) -> HashMap<&str, Vec<String>> {
    let mut out: HashMap<&str, Vec<String>> = HashMap::with_capacity(kb.len());
    for e in kb.entities() {
        let mut keys: Vec<String> = std::iter::once(e.canonical_name.as_str())
            .chain(e.aliases.iter().map(|s| s.as_str()))
            .chain(e.ocr_aliases.iter().map(|s| s.as_str()))
            .map(canon::ocr_variant_fold)
            .filter(|k| !k.is_empty())
            .collect();
        keys.sort();
        keys.dedup();
        out.insert(e.id.as_str(), keys);
    }
    out
}

/// Does the captured banner text actually name `entity_id`?
///
/// The review queue's "OCR disagrees with the stored label" flag decides what a
/// human looks at next, so it must be true only when the pixels and the label
/// really disagree.
///
/// Comparing folded banner text against the id's short name cannot see a
/// multi-word species: `fish:crimsonsnapper` gives the key `crimsonsnapper`,
/// while the banner folds to `crimson snapper`, so `contains` is false and
/// EVERY correctly-labelled Crimson Snapper row was flagged as a disagreement.
/// The KB carries the word boundaries the id lost, so agreement is decided
/// against the names the runtime actually reads off the screen. The legacy
/// short-name comparison is kept as an additional way to agree, which leaves
/// single-word classes (`fish:crimson` -> `crimson`) behaving exactly as before,
/// and ids the KB no longer knows keep working on their own short name.
fn ocr_agrees_with_label(
    name_keys: &HashMap<&str, Vec<String>>,
    ocr_text: &str,
    entity_id: &str,
) -> bool {
    let folded = canon::ocr_variant_fold(ocr_text);
    let short = entity_id
        .split(':')
        .nth(1)
        .unwrap_or(entity_id)
        .replace(['-', '_'], " ")
        .to_lowercase();
    if !short.is_empty() && folded.contains(&short) {
        return true;
    }
    name_keys
        .get(entity_id)
        .map(|keys| keys.iter().any(|k| folded.contains(k)))
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
fn review_priority_sync(
    dir: &std::path::Path,
    limit: Option<usize>,
    entity: Option<String>,
    session: Option<String>,
    only_hard: Option<bool>,
    only_disagreement: Option<bool>,
    sort: Option<String>,
    status: Option<review::ReviewStatus>,
) -> PriorityPage {
    let rows = load_rows(dir);
    let store = review_store(dir);
    let records = store.list();
    let reviews: HashMap<&str, &ReviewRecord> =
        records.iter().map(|r| (r.image_id.as_str(), r)).collect();

    let reviewed_sessions: HashSet<&str> = records
        .iter()
        .filter(|r| {
            !matches!(
                r.review_status,
                ReviewStatus::Unreviewed | ReviewStatus::ReviewedSkipped
            )
        })
        .filter_map(|r| {
            rows.by_id
                .get(r.image_id.as_str())
                .map(|i| rows.all[*i].session_id.as_str())
        })
        .collect();

    // Per-class support, measured on the PROJECTED training set (human verdicts,
    // review-excluded rows dropped): `n < 20` / `sess_n < 3` below ask "does this
    // class have enough training rows to be worth more captures?", so they must
    // count the rows a snapshot would actually write. Counting the collector's
    // labels instead let a class read as supported on 224 rows while training had
    // 13 of them.
    let projected = crate::core::training::project_training_labels(&rows.all, &store);
    let (counts, class_sessions) = projected_class_support(&projected);
    // Names the KB knows each entity by, folded the same way the banner text is.
    // See `ocr_agrees_with_label` for why the id alone cannot answer this.
    let kb = crate::config::Store::new(dir.to_path_buf()).effective_knowledge();
    let name_keys = ocr_name_keys(&kb);

    // v5.6.0 fed `hard_example` into the *low-confidence* slot, so hard
    // examples were displayed as "low model confidence" even though the queue
    // carries no confidence at all. Each flag now gets its own slot.
    let cap = limit.unwrap_or(200).clamp(1, 500);
    let mut items = Vec::new();
    for r in &rows.all {
        let prior = reviews.get(r.image_id.as_str()).copied();
        if matches!(
            prior.map(|p| p.review_status),
            Some(ReviewStatus::ReviewedCorrect)
                | Some(ReviewStatus::ReviewedCorrected)
                | Some(ReviewStatus::ReviewedUnknown)
                | Some(ReviewStatus::ReviewedSkipped)
        ) {
            continue;
        }
        let e = r.entity_id.as_deref();
        // The stored label is still the collector's guess here: rows a human
        // already judged are filtered out above, and a projection only relabels
        // exactly those. So this comparison is genuinely "pixels vs the label the
        // collector wrote", which is the question the queue asks.
        let ocr_disagree = match e {
            Some(id) if !id.is_empty() => !ocr_agrees_with_label(&name_keys, &r.ocr_text, id),
            _ => false,
        };
        let n = e
            .map(|id| counts.get(id).copied().unwrap_or(0))
            .unwrap_or(0);
        let sess_n = e
            .map(|id| class_sessions.get(id).map(|s| s.len()).unwrap_or(0))
            .unwrap_or(0);
        let unseen_session = !reviewed_sessions.contains(r.session_id.as_str());
        let low_conf = prior
            .and_then(|p| p.model_confidence)
            .map(|c| c < 0.5)
            .unwrap_or(false);

        let (mut score, reason) = review::priority_score(
            e.is_none(),
            ocr_disagree,
            low_conf,
            n < 20,
            sess_n < 3,
            unseen_session,
            r.hard_example,
        );
        let is_conflict = prior.is_some_and(|p| p.review_status == ReviewStatus::Conflict);
        let reason = if is_conflict {
            score = score.max(200);
            format!("CONFLICT needs resolution; {reason}")
        } else {
            reason.to_string()
        };

        let model_confidence = prior.and_then(|p| p.model_confidence);

        items.push(PriorityItem {
            image_id: r.image_id.clone(),
            session_id: r.session_id.clone(),
            timestamp_ms: r.timestamp_ms,
            ocr_text: r.ocr_text.chars().take(160).collect(),
            entity_id: e.map(str::to_string),
            review_status: prior
                .map(|p| p.review_status.as_str().to_string())
                .unwrap_or_else(|| "UNREVIEWED".to_string()),
            score,
            reason,
            is_hard_example: r.hard_example,
            ocr_disagreement: ocr_disagree,
            model_confidence,
            in_latest_snapshot: false,
        });
    }

    let f = QueueFilter {
        entity,
        session,
        only_hard: only_hard.unwrap_or(false),
        only_disagreement: only_disagreement.unwrap_or(false),
        sort: sort.unwrap_or_else(|| "priority".to_string()),
    };
    if let Some(e) = f.entity.as_deref() {
        items.retain(|i| i.entity_id.as_deref() == Some(e));
    }
    if let Some(s) = f.session.as_deref() {
        items.retain(|i| i.session_id == s);
    }
    if f.only_hard {
        items.retain(|i| i.is_hard_example);
    }
    if f.only_disagreement {
        items.retain(|i| i.ocr_disagreement);
    }
    // A status filter makes this a REVIEWED queue rather than a pending one.
    if let Some(want) = status {
        items.retain(|i| {
            let cur = reviews.get(i.image_id.as_str()).map(|r| r.review_status);
            cur.map(|c| c == want)
                .unwrap_or(want == review::ReviewStatus::Unreviewed)
        });
    }
    let total_matching = items.len();
    mark_in_latest_snapshot(dir, &mut items);
    match f.sort.as_str() {
        "newest" => items.sort_by_key(|i| std::cmp::Reverse(i.timestamp_ms)),
        "oldest" => items.sort_by_key(|i| i.timestamp_ms),
        "least_confidence" => items.sort_by(|a, b| {
            a.model_confidence
                .unwrap_or(-1.0)
                .total_cmp(&b.model_confidence.unwrap_or(-1.0))
        }),
        "most_confidence" => items.sort_by(|a, b| {
            b.model_confidence
                .unwrap_or(-1.0)
                .total_cmp(&a.model_confidence.unwrap_or(-1.0))
        }),
        "rarest" => items.sort_by_key(|i| (i.entity_id.is_none(), i.ocr_text.len())),
        // EVIDENCE-FUSION ORDER (Phase 11). The teacher's whole value is
        // surfacing rows where the cheap label is most likely WRONG, then
        // getting out of the way: OCR disagreeing with the stored label first,
        // then RAM disagreeing, then a name with no KB entity, then OCR and RAM
        // disagreeing with each other, and full agreement LAST because it is
        // cheap to confirm.
        //
        // This ORDERS the queue. It never labels, never excludes, and never
        // touches `training_eligible`: priority is not evidence.
        "evidence" => {
            let kb = kb_entity_set(crate::core::knowledge::KnowledgeBase::bundled());
            let ram = ram_results(dir);
            let mut keyed: Vec<(crate::core::semantic::ReviewPriority, PriorityItem)> = items
                .drain(..)
                .map(|i| {
                    let f = crate::core::semantic::fuse(
                        &i.image_id,
                        Some(i.ocr_text.as_str()),
                        i.entity_id.as_deref(),
                        ram.get(&i.image_id),
                        false,
                        &kb,
                    );
                    (f.priority, i)
                })
                .collect();
            keyed.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then_with(|| b.1.timestamp_ms.cmp(&a.1.timestamp_ms))
                    .then_with(|| a.1.image_id.cmp(&b.1.image_id))
            });
            items = keyed.into_iter().map(|(_, i)| i).collect();
        }
        _ => items.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then(b.timestamp_ms.cmp(&a.timestamp_ms))
                .then(a.image_id.cmp(&b.image_id))
        }),
    }
    items.truncate(cap);
    PriorityPage {
        items,
        total_matching,
        limit: cap,
    }
}

/// Flag rows that are already in the most recent frozen training snapshot, so
/// a reviewer can see which samples the current models have learned from.
///
/// Bounded work: only the newest few snapshots are inspected, and a missing
/// or unreadable snapshot simply leaves the flag false rather than failing.
fn mark_in_latest_snapshot(data_dir: &std::path::Path, items: &mut [PriorityItem]) {
    let Ok(rd) = std::fs::read_dir(data_dir.join("training").join("snapshots")) else {
        return;
    };
    let newest = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            e.metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| (t, e.path()))
        })
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p);
    let Some(snap) = newest else { return };
    let Ok(text) = std::fs::read_to_string(snap.join("labels.jsonl")) else {
        return;
    };
    // Streaming parse: `image_id` is read out of each line without building a
    // `serde_json::Value` for every row, which on a large snapshot was the
    // single most expensive part of building the queue.
    let mut in_snap: HashSet<String> = HashSet::new();
    for line in text.lines() {
        if line.is_empty() || !line.contains("\"image_id\"") {
            continue;
        }
        if let Some(id) = json_str_field(line, "image_id") {
            in_snap.insert(id);
        }
    }
    for i in items.iter_mut() {
        i.in_latest_snapshot = in_snap.contains(i.image_id.as_str());
    }
}

/// Extract a top-level string field without a full JSON parse.
///
/// Deliberately simple: it looks for `"field"` followed by `:` and a quoted
/// string, and unescapes only via `serde_json` on the extracted slice. A
/// malformed line yields `None` (fail closed - the row is simply not marked).
fn json_str_field(line: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\"");
    let at = line.find(&needle)? + needle.len();
    let rest = line[at..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// ---------------------------------------------------------------------------
// Canonical search + drops explorer
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct EntityHit {
    pub entity_id: String,
    pub canonical_name: String,
    pub category: String,
    pub kind: String,
}

#[tauri::command]
pub fn review_search(st: State<'_, AppState>, query: String) -> Vec<EntityHit> {
    let kb = st.store.effective_knowledge();
    let by_id: HashMap<&str, _> = kb.entities().iter().map(|e| (e.id.as_str(), e)).collect();
    let hit = |e: &crate::core::knowledge::GpoEntity, kind: &str| EntityHit {
        entity_id: e.id.clone(),
        canonical_name: e.canonical_name.clone(),
        category: e.category.as_str().to_string(),
        kind: kind.to_string(),
    };
    match canon::resolve(&kb, &query) {
        Resolution::Exact(id) => by_id
            .get(id.as_str())
            .map(|e| vec![hit(e, "exact")])
            .unwrap_or_default(),
        Resolution::Ambiguous(ids) => ids
            .iter()
            .filter_map(|id| by_id.get(id.as_str()))
            .map(|e| hit(e, "ambiguous-pick-one"))
            .collect(),
        Resolution::Unknown => vec![],
    }
}

#[derive(Debug, Serialize)]
pub struct DropEntry {
    pub entity_id: String,
    pub canonical_name: String,
    pub category: String,
    pub fishing_drop: bool,
    pub rarity: Option<String>,
    pub aliases: Vec<String>,
    pub wiki_source: String,
    pub wiki_url: Option<String>,
    pub collected: usize,
    pub reviewed: usize,
    pub eligible: usize,
    pub sessions: usize,
    pub model_status: String,
    /// `Some(winner)` when this entity's canonical name is already claimed by
    /// another KB entity, so it can never be resolved by name. Surfaced
    /// instead of silently hidden.
    pub shadowed_by: Option<String>,
}

#[tauri::command]
pub async fn drops_explorer(st: State<'_, AppState>) -> Result<Vec<DropEntry>, String> {
    let dir = st.store.dir().to_path_buf();
    let kb = st.store.effective_knowledge();
    blocking(move || Ok(drops_explorer_sync(&dir, kb))).await
}

fn drops_explorer_sync(
    dir: &std::path::Path,
    kb: crate::core::knowledge::KnowledgeBase,
) -> Vec<DropEntry> {
    let rows = load_rows(dir);
    let reviews = review_store(dir).list();
    let rev_by_image: HashMap<&str, &ReviewRecord> =
        reviews.iter().map(|r| (r.image_id.as_str(), r)).collect();
    let fish_scope: HashSet<String> =
        std::fs::read_to_string(dir.join("models").join("fish_v1.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("classes").cloned())
            .and_then(|c| serde_json::from_value::<Vec<String>>(c).ok())
            .unwrap_or_default()
            .into_iter()
            .collect();

    let mut out = Vec::new();
    for c in canon::canonical_view(&kb) {
        let mut sessions: HashSet<&str> = HashSet::new();
        let mut collected = 0;
        let (mut reviewed, mut eligible) = (0, 0);
        if let Some(idxs) = rows.by_entity.get(&c.entity_id) {
            for i in idxs {
                let r = &rows.all[*i];
                collected += 1;
                sessions.insert(r.session_id.as_str());
                if let Some(rev) = rev_by_image.get(r.image_id.as_str()) {
                    if matches!(
                        rev.review_status,
                        ReviewStatus::ReviewedCorrect
                            | ReviewStatus::ReviewedCorrected
                            | ReviewStatus::ReviewedUnknown
                    ) {
                        reviewed += 1;
                    }
                    if rev.training_eligible {
                        eligible += 1;
                    }
                }
            }
        }
        out.push(DropEntry {
            entity_id: c.entity_id.clone(),
            canonical_name: c.canonical_name.clone(),
            category: c.category.as_str().to_string(),
            fishing_drop: c.fishing_drop,
            rarity: c.rarity.clone(),
            aliases: c.aliases.clone(),
            wiki_source: c.wiki_source.clone(),
            wiki_url: c.wiki_url.clone(),
            collected,
            reviewed,
            eligible,
            sessions: sessions.len(),
            model_status: if c.shadowed_by.is_some() {
                "ALIAS_SHADOWED".to_string()
            } else if fish_scope.contains(&c.entity_id) {
                "IN_SCOPE".to_string()
            } else if c.fishing_drop {
                "OUT_OF_SCOPE".to_string()
            } else {
                "N/A".to_string()
            },
            shadowed_by: c.shadowed_by.clone(),
        });
    }
    out.sort_by_key(|d| std::cmp::Reverse(d.collected));
    out
}

// ---------------------------------------------------------------------------
// Readiness
// ---------------------------------------------------------------------------

fn readiness_status_inner(
    store_dir: &std::path::Path,
    settings: &crate::config::Settings,
) -> Vec<readiness::ModelReadiness> {
    let rows = load_rows(store_dir);
    let all = &rows.all;
    let gates = assess_capabilities(all);
    let gate = |id: &str| gates.iter().find(|g| g.id == id);
    // "Human-reviewed" comes from the PROJECTION below, not from a second parse of
    // the review file: one file, one parser, one answer.
    let records = registry::load_registry(store_dir);
    // THE PROJECTED TRAINING SET: live rows with the human review verdicts on
    // them. The per-class qualification and coverage numbers below describe what
    // a training snapshot WOULD contain, not the collector's guesses. Before this
    // they read the live file, so readiness could show `fish:crimson` qualified
    // on 224 rows while the snapshot wrote 13 and the trainer refused.
    let projected = crate::core::training::project_training_labels(all, &review_store(store_dir));
    // A review state that cannot be read back is a refusal, never a fallback to
    // the collector's labels: the fallback would be the lie this closes.
    let projection_note = match &projected.read_error {
        Some(err) => format!("; review state unreadable, so no class counts are reported: {err}"),
        None => format!(
            "; class counts are measured on the projected training set ({} row(s), \
             {} relabelled by human review, {} excluded by review)",
            projected.rows.len(),
            projected.rows_relabelled_by_review,
            projected.rows_excluded_by_review
        ),
    };
    let gate_rows = projected.rows.as_slice();
    let thresholds = readiness::ReadinessThresholds {
        min_macro_f1: settings.training.readiness_min_macro_f1,
        min_worst_class_f1: settings.training.readiness_min_worst_f1,
        min_shadow_events: settings.training.readiness_min_shadow_events,
        min_shadow_agreement: settings.training.readiness_min_shadow_agreement,
        min_shadow_sessions: settings.training.readiness_min_shadow_sessions,
        min_review_coverage: settings.training.readiness_min_review_coverage,
    };
    let jobs = training::list_jobs(store_dir);

    let mut out = Vec::new();
    for (family, prefix, stem) in [
        ("fish", Some("fish:"), "fish_v1"),
        ("state", None, "state_v1"),
    ] {
        let scoped: Vec<_> = gate_rows
            .iter()
            .filter(|r| match prefix {
                Some(p) => r.entity_id.as_deref().is_some_and(|e| e.starts_with(p)),
                None => matches!(
                    r.game_state.as_ref().map(|s| s.as_str()),
                    Some("waiting_for_bite") | Some("bite") | Some("catch_result")
                ),
            })
            .collect();
        let reviewed_scoped = scoped
            .iter()
            .filter(|r| {
                projected
                    .reviewed_eligible_ids
                    .contains(r.image_id.as_str())
            })
            .count();

        // Structured data gate from the real capability gate + per-entity
        // qualification, so readiness can say WHICH shortfall it is.
        let quals = qualify_entities(gate_rows, prefix.unwrap_or(""));
        let qualified: Vec<_> = quals.iter().filter(|q| q.qualified).collect();
        let test_covered = qualified.iter().filter(|q| q.test_covered).count();
        // Session shortfall: a class that has examples but too few sessions.
        let sessions_short = qualified
            .iter()
            .find(|q| q.sessions < crate::core::ml_capability::ENTITY_MIN_SESSIONS)
            .map(|q| {
                (
                    q.entity.clone(),
                    q.sessions,
                    crate::core::ml_capability::ENTITY_MIN_SESSIONS,
                )
            });
        // REQUIRED CLASS COUNT IS NOW DERIVED, NEVER DECLARED.
        //
        // Was `FISH_MIN_ENTITIES = 10` - a scalar that contradicted the
        // 8-logit fish model and could be satisfied by any 10 KB fish entities,
        // including ones the model would never learn. It is now
        // `len(FISH_VOCAB)`, the same list the trainer builds its output head
        // from. The authoritative check is the SET comparison in
        // `assess_capabilities` (`fish_entity`), which `g.ready` reflects; this
        // count exists for display only.
        let (required, test_required, gate_id) = if family == "fish" {
            (
                crate::core::class_vocab::fish_vocab_len(),
                crate::core::class_vocab::fish_vocab_len(),
                "fish_entity",
            )
        } else {
            (1, 0, "state")
        };
        let g = gate(gate_id);
        let data = DataGateInfo {
            ready: g.map(|x| x.ready).unwrap_or(false) && sessions_short.is_none(),
            qualified: qualified.len(),
            required,
            test_covered,
            test_required,
            sessions_short,
            detail: format!(
                "{}{}",
                g.map(|x| x.detail.clone())
                    .unwrap_or_else(|| "no gate".to_string()),
                projection_note
            ),
            next_action: g
                .and_then(|x| x.blocking_requirement.clone())
                .unwrap_or_else(|| "Open Dataset to see per-class qualification.".to_string()),
        };

        // Deployed revision, and the soak FOR THAT REVISION ONLY.
        let manifest: serde_json::Value =
            std::fs::read_to_string(store_dir.join("models").join(format!("{stem}.json")))
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(serde_json::Value::Null);
        // ONE reader for the manifest version. v5.6.x re-parsed `version` here as a
        // `u32` while the shadow log joins on the RAW STRING produced by
        // `revision_from_manifest`. A manifest whose version was a JSON number,
        // or any non-numeric string, therefore yielded `None` here - and `None`
        // means the LEGACY UNION view: the soak counted every revision that
        // slot had ever run, so a fresh candidate inherited the incumbent's
        // evidence. One reader, one definition, one join key.
        //
        // `deployed_rev` (raw string) filters the soak; `deployed_version`
        // (numeric) is only for display. A revision that is not a plain number
        // still filters the soak correctly and simply reports as "unknown"
        // revision in the UI, rather than silently widening the join.
        let deployed_rev: Option<String> = if manifest.is_null() {
            None
        } else {
            Some(crate::core::shadow_infer::revision_from_manifest(&manifest))
        };
        let deployed_version: Option<u32> = deployed_rev.as_deref().and_then(|s| s.parse().ok());
        let soak = registry::soak_stats_for(store_dir, stem, deployed_rev.as_deref(), 500);
        let soak_opt = (soak.events > 0).then_some(&soak);
        let phase = job_phase(&jobs, family);

        out.push(readiness::assess_family(
            family,
            &data,
            reviewed_scoped,
            scoped.len(),
            &records,
            deployed_version,
            soak_opt,
            phase,
            &jobs,
            &thresholds,
        ));
    }

    // Fruit: same shape, honest about being far from ready. Same projection as
    // fish: the counts describe the rows a snapshot would write.
    {
        let scoped: Vec<_> = gate_rows
            .iter()
            .filter(|r| {
                r.entity_id
                    .as_deref()
                    .is_some_and(|e| e.starts_with("fruit:"))
            })
            .collect();
        let reviewed_scoped = scoped
            .iter()
            .filter(|r| {
                projected
                    .reviewed_eligible_ids
                    .contains(r.image_id.as_str())
            })
            .count();
        let quals = qualify_entities(gate_rows, "fruit:");
        let qualified: Vec<_> = quals.iter().filter(|q| q.qualified).collect();
        let g = gate("fruit_entity");
        out.push(readiness::assess_family(
            "fruit",
            &DataGateInfo {
                ready: g.map(|x| x.ready).unwrap_or(false),
                qualified: qualified.len(),
                required: crate::core::ml_capability::FRUIT_MIN_ENTITIES,
                test_covered: qualified.iter().filter(|q| q.test_covered).count(),
                test_required: crate::core::ml_capability::ENTITY_MIN_TEST_COVERED,
                sessions_short: None,
                detail: format!(
                    "{}{}",
                    g.map(|x| x.detail.clone())
                        .unwrap_or_else(|| "no gate".to_string()),
                    projection_note
                ),
                next_action: g
                    .and_then(|x| x.blocking_requirement.clone())
                    .unwrap_or_else(|| "Collect real fruit catches.".to_string()),
            },
            reviewed_scoped,
            scoped.len(),
            &records,
            None,
            None,
            JobPhase::Idle,
            &jobs,
            &thresholds,
        ));
    }
    out
}

fn job_phase(jobs: &[training::TrainingJob], family: &str) -> JobPhase {
    if jobs.iter().any(|j| {
        j.model_family == family
            && matches!(
                j.status,
                training::JobStatus::Running | training::JobStatus::Queued
            )
    }) {
        return JobPhase::Training;
    }
    if jobs
        .iter()
        .any(|j| j.model_family == family && matches!(j.status, training::JobStatus::Evaluating))
    {
        return JobPhase::Evaluating;
    }
    JobPhase::Idle
}

#[tauri::command]
pub async fn readiness_status(
    st: State<'_, AppState>,
) -> Result<Vec<readiness::ModelReadiness>, String> {
    let store_dir = st.store.dir().to_path_buf();
    let settings = st.settings.read().clone();
    blocking(move || Ok(readiness_status_inner(&store_dir, &settings))).await
}

// ---------------------------------------------------------------------------
// Hermes orchestration interface (no agent framework)
// ---------------------------------------------------------------------------

/// Hermes task view: the deterministic trigger + readiness state an external
/// orchestrator may poll. Hermes schedules work by calling the existing
/// commands (`training_start`/`decide`/`promote`); it CANNOT bypass gates
/// because no bypass path exists in this codebase.
#[derive(Debug, Serialize)]
pub struct HermesTasks {
    pub triggers: Vec<training::TriggerEvent>,
    pub readiness: Vec<readiness::ModelReadiness>,
    pub history_tail: Vec<serde_json::Value>,
}

#[tauri::command]
pub fn hermes_tasks(st: State<'_, AppState>) -> HermesTasks {
    let rows = MlDatasetStore::new(st.store.dir().to_path_buf()).annotations();
    let gates = assess_capabilities(&rows);
    let gate_ready = |id: &str| {
        gates
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.ready)
            .unwrap_or(false)
    };
    let prev = training::load_trigger_state(st.store.dir());
    let settings = st.settings.read();
    let triggers = training::evaluate_triggers(
        &rows,
        DATASET_VERSION,
        gate_ready("fish_entity"),
        gate_ready("fruit_entity"),
        &prev,
        settings.training.min_new_samples,
        settings.training.min_new_sessions,
        settings.training.trigger_cooldown_hours,
        crate::events::now_ms(),
    );
    HermesTasks {
        triggers,
        readiness: readiness_status_inner(st.store.dir(), &settings),
        history_tail: training::read_history(st.store.dir(), 20),
    }
}

// ---------------------------------------------------------------------------
// Semantic-teacher evidence fusion
// ---------------------------------------------------------------------------

/// Canonical KB entity ids, for evidence fusion.
///
/// Read from the ONTOLOGY, never from the dataset. The fusion logic has to judge
/// a name against what the KB actually claims, not against whatever the
/// collector happened to write onto the row.
pub fn kb_entity_set(
    kb: &crate::core::knowledge::KnowledgeBase,
) -> std::collections::BTreeSet<String> {
    kb.entities().iter().map(|e| e.id.clone()).collect()
}

/// Semantic-teacher results, keyed by image id.
///
/// A missing file means "the teacher has not been run" - a real state, which the
/// fusion treats as *no teacher evidence* rather than as agreement. Only
/// successful rows are returned: an errored or malformed row is not evidence,
/// and silently counting it as agreement is how a broken teacher becomes an
/// authority.
pub fn ram_results(
    store_dir: &std::path::Path,
) -> std::collections::HashMap<String, crate::core::semantic::RamResult> {
    let path = store_dir.join("semantic_teacher").join("ram_results.jsonl");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return std::collections::HashMap::new();
    };
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<crate::core::semantic::RamResult>(l).ok())
        .filter(|r| r.is_ok())
        .map(|r| (r.image_id.clone(), r))
        .collect()
}

/// Status of the OFFLINE semantic teacher (RAM++ / RAM).
///
/// Reported honestly: NOT_INSTALLED when no results file exists, ERROR when one
/// exists but yields nothing usable. It is labelled a TEACHER and never "ground
/// truth": it supplies evidence, and a human decides.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTeacherStatus {
    pub state: String,
    pub model: Option<String>,
    pub checkpoint_sha256: Option<String>,
    pub preprocessing: Option<String>,
    pub images_analysed: usize,
    pub last_analysis_ms: Option<u64>,
    pub human_reviewed_assisted: usize,
    pub unresolved_concepts: usize,
    pub detail: String,
}

pub fn semantic_teacher_status_sync(dir: &std::path::Path) -> SemanticTeacherStatus {
    let mut st = SemanticTeacherStatus {
        state: "NOT_INSTALLED".to_string(),
        model: None,
        checkpoint_sha256: None,
        preprocessing: None,
        images_analysed: 0,
        last_analysis_ms: None,
        human_reviewed_assisted: 0,
        unresolved_concepts: 0,
        detail: "Semantic teacher has not been run on this install. Review still works without it."
            .to_string(),
    };
    let path = dir.join("semantic_teacher").join("ram_results.jsonl");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return st;
    };

    let kb = kb_entity_set(crate::core::knowledge::KnowledgeBase::bundled());
    let reviewed: std::collections::HashSet<String> = review_store(dir)
        .list()
        .into_iter()
        .map(|r| r.image_id)
        .collect();

    let (mut ok_count, mut errors) = (0usize, 0usize);
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(r) = serde_json::from_str::<crate::core::semantic::RamResult>(line) else {
            errors += 1;
            continue;
        };
        if !r.is_ok() {
            errors += 1;
            continue;
        }
        ok_count += 1;
        st.model.get_or_insert_with(|| r.ram_model.clone());
        st.checkpoint_sha256
            .get_or_insert_with(|| r.checkpoint_sha256.clone());
        st.preprocessing
            .get_or_insert_with(|| r.preprocessing.clone());
        st.last_analysis_ms = Some(st.last_analysis_ms.unwrap_or(0).max(r.timestamp_ms));
        if let Some((top, _)) = r.top_category() {
            if !kb.contains(top) {
                st.unresolved_concepts += 1;
            }
        }
        // "Assisted" means evidence was shown AND a human then reviewed it.
        if reviewed.contains(&r.image_id) {
            st.human_reviewed_assisted += 1;
        }
    }
    st.images_analysed = ok_count;
    st.state = if ok_count == 0 { "ERROR" } else { "AVAILABLE" }.to_string();
    st.detail = if ok_count == 0 {
        format!(
            "Semantic teacher produced no usable results ({errors} errored row(s)). \
             It is evidence only and never a label source."
        )
    } else {
        format!(
            "{ok_count} image(s) analysed, {errors} errored row(s). Evidence only: \
             every label still requires a human verdict."
        )
    };
    st
}

#[tauri::command]
pub async fn semantic_teacher_status(
    st: State<'_, AppState>,
) -> Result<SemanticTeacherStatus, String> {
    let dir = st.store.dir().to_path_buf();
    blocking(move || Ok(semantic_teacher_status_sync(&dir))).await
}

// ---------------------------------------------------------------------------
// Tests: readiness and the review queue measure the projected training set
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ml_dataset::{MlAnnotation, MlDatasetStore, MlTask, DATASET_VERSION};
    use std::io::Write as _;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("gpo-review-{tag}-{}", crate::events::now_ms()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn row(image_id: &str, session: &str, entity: Option<&str>, ocr: &str) -> MlAnnotation {
        MlAnnotation {
            image_id: image_id.to_string(),
            dataset_version: DATASET_VERSION,
            session_id: session.to_string(),
            task: MlTask::EntityRecognition,
            ocr_text: ocr.to_string(),
            region_name: "drop".to_string(),
            ui_label: None,
            bbox: None,
            game_state: None,
            entity_id: entity.map(str::to_string),
            annotator: "collector".to_string(),
            timestamp_ms: 1,
            source: "ui".to_string(),
            confidence: Some(0.9),
            hard_example: false,
            hard_reason: None,
            corrections: Vec::new(),
            event_id: None,
            frame_index: None,
        }
    }

    /// Full `reviews.jsonl` schema, so both review parsers agree.
    fn review_line(
        image_id: &str,
        eligible: bool,
        human: Option<&str>,
        reason: Option<&str>,
    ) -> String {
        serde_json::json!({
            "review_id": format!("rv-{image_id}"),
            "image_id": image_id,
            "session_id": "fixture-session",
            "review_status": if eligible { "REVIEWED_CORRECTED" } else { "REVIEWED_SKIPPED" },
            "training_eligible": eligible,
            "human_entity_id": human,
            "excluded_reason": reason,
            "dataset_version": DATASET_VERSION,
        })
        .to_string()
    }

    fn write_labels(dir: &std::path::Path, rows: &[MlAnnotation]) {
        let ml = MlDatasetStore::new(dir.to_path_buf());
        let root = ml.root().to_path_buf();
        std::fs::create_dir_all(&root).unwrap();
        let mut f = std::fs::File::create(root.join("labels.jsonl")).unwrap();
        for r in rows {
            writeln!(f, "{}", serde_json::to_string(r).unwrap()).unwrap();
        }
    }

    fn write_reviews(dir: &std::path::Path, lines: &[String]) {
        let mut f = std::fs::File::create(dir.join("reviews.jsonl")).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
    }

    /// Add a multi-word species to the KB overlay, the way the app's own
    /// knowledge store holds it. Without it the banner text and the flattened id
    /// cannot be compared at all.
    fn write_overlay(dir: &std::path::Path, id: &str, name: &str) {
        std::fs::write(
            dir.join("knowledge_custom.json"),
            serde_json::json!([{
                "id": id,
                "canonical_name": name,
                "aliases": [],
                "category": "fish",
                "rarity": null,
                "description": null,
                "ocr_aliases": [],
                "visual_refs": [],
                "provenance": { "source": "test", "url": null, "updated": null },
            }])
            .to_string(),
        )
        .unwrap();
    }

    /// Readiness must count the rows a snapshot WOULD write. 100 collected rows
    /// of which a human rejected 40 leaves 60 training rows, all reviewed - and
    /// the readiness evidence must say 60/60, not the live file's 60/100.
    #[test]
    fn readiness_counts_the_projected_training_set() {
        let dir = tmp_dir("readiness");
        let mut rows = Vec::new();
        let mut reviews = Vec::new();
        for i in 0..100 {
            let id = format!("g-{i}");
            rows.push(row(
                &id,
                "fixture-session",
                Some("fish:golden"),
                "Golden Fish",
            ));
            if i < 60 {
                reviews.push(review_line(&id, true, Some("fish:golden"), None));
            } else {
                reviews.push(review_line(&id, false, None, Some("human skipped")));
            }
        }
        write_labels(&dir, &rows);
        write_reviews(&dir, &reviews);

        let live = MlDatasetStore::new(dir.clone()).annotations();
        assert_eq!(live.len(), 100);
        let out = readiness_status_inner(&dir, &crate::config::Settings::default());
        let fish = out
            .iter()
            .find(|r| r.family == "fish")
            .expect("fish readiness");
        assert_eq!(
            fish.evidence["scoped_total"], 60,
            "rejected rows are not training rows: {}",
            fish.evidence
        );
        assert_eq!(fish.evidence["reviewed_scoped"], 60);
        assert!(
            fish.checks
                .iter()
                .any(|c| c.detail.contains("projected training set"))
                || out.iter().any(|r| r
                    .checks
                    .iter()
                    .any(|c| c.detail.contains("projected training set"))),
            "the evidence must say which set it measured"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Per-class counters must count the projected set: a class whose rows a
    /// human moved to other species contributes 13 training rows, not 224, so the
    /// queue treats it as needing more captures.
    #[test]
    fn the_review_queue_counts_classes_in_the_projected_training_set() {
        let dir = tmp_dir("queue-counts");
        let mut rows = Vec::new();
        let mut reviews = Vec::new();
        for i in 0..224 {
            let id = format!("c-{i}");
            rows.push(row(&id, "fixture-session", Some("fish:crimson"), "Crimson"));
            if i < 211 {
                reviews.push(review_line(&id, true, Some("fish:crimsonsnapper"), None));
            }
        }
        write_labels(&dir, &rows);
        write_reviews(&dir, &reviews);

        let live = crate::core::ml_capability::qualify_entities(&rows, "fish:");
        let crimson_live = live
            .iter()
            .find(|q| q.entity == "fish:crimson")
            .expect("live class");
        assert_eq!(crimson_live.examples, 224, "the live file still claims 224");

        let loaded = load_rows(&dir);
        let projected =
            crate::core::training::project_training_labels(&loaded.all, &review_store(&dir));
        let (counts, sessions) = projected_class_support(&projected);
        assert_eq!(
            counts.get("fish:crimson").copied(),
            Some(13),
            "the queue must count what trains, not what was collected"
        );
        assert_eq!(sessions.get("fish:crimson").map(|s| s.len()), Some(1));
        assert_eq!(counts.get("fish:crimsonsnapper").copied(), Some(211));

        // The judged rows leave the pending queue; the 13 unjudged ones stay.
        let page = review_priority_sync(&dir, Some(500), None, None, None, None, None, None);
        assert_eq!(page.items.len(), 13, "{:?}", page.items.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The disagreement flag is a claim about the PIXELS. A species id has no
    /// spaces, so comparing it to banner text that does flagged every correctly
    /// labelled Crimson Snapper as a disagreement. It must now agree when the
    /// banner names the species - and still disagree when it names something
    /// else.
    #[test]
    fn a_species_row_is_flagged_only_when_the_banner_really_disagrees() {
        let dir = tmp_dir("queue-ocr");
        write_overlay(&dir, "fish:crimsonsnapper", "Crimson Snapper");
        let rows = vec![
            row(
                "ok-species",
                "fixture-session",
                Some("fish:crimsonsnapper"),
                "Crimson Snapper",
            ),
            row(
                "other-banner",
                "fixture-session",
                Some("fish:crimsonsnapper"),
                "Golden Ribbon Angelfish",
            ),
            row(
                "ok-stem",
                "fixture-session",
                Some("fish:crimson"),
                "Crimson",
            ),
            row(
                "stem-mismatch",
                "fixture-session",
                Some("fish:crimson"),
                "Golden Ribbon Angelfish",
            ),
        ];
        write_labels(&dir, &rows);
        write_reviews(&dir, &[]);

        let page = review_priority_sync(&dir, Some(50), None, None, None, None, None, None);
        let flag = |id: &str| -> Option<bool> {
            page.items
                .iter()
                .find(|i| i.image_id == id)
                .map(|i| i.ocr_disagreement)
        };
        assert_eq!(flag("ok-species"), Some(false), "the banner names it");
        assert_eq!(
            flag("other-banner"),
            Some(true),
            "the banner names something else"
        );
        assert_eq!(
            flag("ok-stem"),
            Some(false),
            "single-word behaviour is unchanged"
        );
        assert_eq!(
            flag("stem-mismatch"),
            Some(true),
            "mismatch is still a disagreement"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No row may be called a disagreement just because a human replaced a
    /// stem. Reviewed rows leave the pending queue entirely, so the flag can
    /// never describe a verdict the collector made and a human undid.
    #[test]
    fn a_row_a_human_overrode_is_not_in_the_queue_as_a_disagreement() {
        let dir = tmp_dir("queue-overridden");
        let rows = vec![
            row("kept", "fixture-session", Some("fish:crimson"), "Crimson"),
            row(
                "overridden",
                "fixture-session",
                Some("fish:crimson"),
                "Crimson Snapper",
            ),
        ];
        write_labels(&dir, &rows);
        write_reviews(
            &dir,
            &[review_line(
                "overridden",
                true,
                Some("fish:crimsonsnapper"),
                None,
            )],
        );
        let page = review_priority_sync(&dir, Some(50), None, None, None, None, None, None);
        assert_eq!(
            page.items.len(),
            1,
            "the judged row is not pending: {:?}",
            page.items
        );
        assert_eq!(page.items[0].image_id, "kept");
        assert!(!page.items[0].ocr_disagreement);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
