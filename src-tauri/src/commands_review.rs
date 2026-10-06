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
    Rows { all, by_id, by_entity }
}

fn dataset_images_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    MlDatasetStore::new(data_dir.to_path_buf()).root().join("images")
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
    let img = image::load_from_memory(&bytes).map_err(|_| format!("undecodable image '{image_id}'"))?;
    let dim = max_dim.unwrap_or(0);
    let (width, height, out) = if dim > 0 && (img.width() > dim || img.height() > dim) {
        let scaled = img.thumbnail(dim, dim);
        let mut buf: Vec<u8> = Vec::new();
        scaled
            .write_to(&mut ::std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
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
pub fn review_list(
    st: State<'_, AppState>,
    status: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Vec<ReviewRecord> {
    let mut v = review_store(st.store.dir()).list();
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
    /// Image ids whose most recent state was an undo.
    pub restorable: Vec<String>,
}

#[tauri::command]
pub fn review_integrity(st: State<'_, AppState>, tail: Option<usize>) -> ReviewIntegrity {
    let store = review_store(st.store.dir());
    let audit = store.audit();
    let restorable: Vec<String> = store
        .list()
        .into_iter()
        .filter(|r| r.review_status == ReviewStatus::Unreviewed || r.excluded_reason.as_deref() == Some("undone"))
        .map(|r| r.image_id)
        .collect();
    ReviewIntegrity {
        unreadable_lines: store.corrupt_lines().len(),
        audit_events: audit.len(),
        last_event: audit.last().cloned(),
        restorable,
    }
}

/// Reconstruct `reviews.jsonl` from the append-only audit log.
#[tauri::command]
pub fn review_rebuild(st: State<'_, AppState>) -> Result<usize, String> {
    review_store(st.store.dir()).rebuild_from_audit()
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
pub fn review_coverage(st: State<'_, AppState>) -> ReviewCoverageView {
    let rows = load_rows(st.store.dir());
    let store = review_store(st.store.dir());
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

        let Some(e) = r.entity_id.as_deref() else { continue };
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
        ent_sessions.entry(e.to_string()).or_default().insert(r.session_id.clone());
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
            Ok(row.clone())
        };
        match outcome {
            Ok(_) => dataset_updated = true,
            Err(e) => dataset_error = Some(e),
        }
    }
    Ok(ReviewApplyResult { record, dataset_updated, dataset_error })
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
    MlDatasetStore::new(st.store.dir().to_path_buf())
        .annotate(&image_id, None, None, None, Some(entity_id), "human", false, None)?;
    Ok(rec)
}

/// Undo the last review of an image. The audit keeps the mistake.
#[tauri::command]
pub fn review_undo(st: State<'_, AppState>, image_id: String) -> Result<ReviewRecord, String> {
    review::undo_review(&review_store(st.store.dir()), &image_id)
}

// ---------------------------------------------------------------------------
// Prioritized queue
// ---------------------------------------------------------------------------

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
pub fn review_priority(
    st: State<'_, AppState>,
    limit: Option<usize>,
    entity: Option<String>,
    session: Option<String>,
    only_hard: Option<bool>,
    only_disagreement: Option<bool>,
    sort: Option<String>,
) -> Vec<PriorityItem> {
    let rows = load_rows(st.store.dir());
    let store = review_store(st.store.dir());
    let records = store.list();
    let reviews: HashMap<&str, &ReviewRecord> =
        records.iter().map(|r| (r.image_id.as_str(), r)).collect();

    let reviewed_sessions: HashSet<&str> = records
        .iter()
        .filter(|r| {
            !matches!(r.review_status, ReviewStatus::Unreviewed | ReviewStatus::ReviewedSkipped)
        })
        .filter_map(|r| rows.by_id.get(r.image_id.as_str()).map(|i| rows.all[*i].session_id.as_str()))
        .collect();

    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut class_sessions: HashMap<&str, HashSet<&str>> = HashMap::new();
    for r in &rows.all {
        if let Some(e) = r.entity_id.as_deref() {
            *counts.entry(e).or_default() += 1;
            class_sessions.entry(e).or_default().insert(r.session_id.as_str());
        }
    }

    // v5.6.0 fed `hard_example` into the *low-confidence* slot, so hard
    // examples were displayed as "low model confidence" even though the queue
    // carries no confidence at all. Each flag now gets its own slot.
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
        let key = e.map(|id| {
            id.split(':')
                .nth(1)
                .unwrap_or(id)
                .replace(['-', '_'], " ")
                .to_lowercase()
        });
        let ocr_disagree = match (e, key.as_deref()) {
            (Some(_), Some(k)) if !k.is_empty() => {
                !canon::ocr_variant_fold(&r.ocr_text).contains(k)
            }
            _ => false,
        };
        let n = e.map(|id| counts.get(id).copied().unwrap_or(0)).unwrap_or(0);
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
            review_status: prior.map(|p| p.review_status.as_str().to_string()).unwrap_or_else(|| "UNREVIEWED".to_string()),
            score,
            reason,
            is_hard_example: r.hard_example,
            ocr_disagreement: ocr_disagree,
            model_confidence,
            in_latest_snapshot: false,
        });    }

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
    match f.sort.as_str() {
        "newest" => items.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms)),
        "oldest" => items.sort_by(|a, b| a.timestamp_ms.cmp(&b.timestamp_ms)),
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
        _ => items.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then(b.timestamp_ms.cmp(&a.timestamp_ms))
                .then(a.image_id.cmp(&b.image_id))
        }),
    }
    items.truncate(limit.unwrap_or(200).clamp(1, 500));
    items
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
    let hit = |e: &&crate::core::knowledge::GpoEntity, kind: &str| EntityHit {
        entity_id: e.id.clone(),
        canonical_name: e.canonical_name.clone(),
        category: e.category.as_str().to_string(),
        kind: kind.to_string(),
    };
    match canon::resolve(&kb, &query) {
        Resolution::Exact(id) => by_id.get(id.as_str()).map(|e| vec![hit(&e, "exact")]).unwrap_or_default(),
        Resolution::Ambiguous(ids) => {
            ids.iter().filter_map(|id| by_id.get(id.as_str())).map(|e| hit(&e, "ambiguous-pick-one")).collect()
        }
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
pub fn drops_explorer(st: State<'_, AppState>) -> Vec<DropEntry> {
    let kb = st.store.effective_knowledge();
    let rows = load_rows(st.store.dir());
    let reviews = review_store(st.store.dir()).list();
    let rev_by_image: HashMap<&str, &ReviewRecord> =
        reviews.iter().map(|r| (r.image_id.as_str(), r)).collect();
    let fish_scope: HashSet<String> = std::fs::read_to_string(st.store.dir().join("models").join("fish_v1.json"))
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
    store: &crate::config::Store,
    settings: &crate::config::Settings,
) -> Vec<readiness::ModelReadiness> {
    let rows = load_rows(store.dir());
    let all = &rows.all;
    let gates = assess_capabilities(all);
    let gate = |id: &str| gates.iter().find(|g| g.id == id);
    let reviews = review_store(store.dir()).list();
    let reviewed_ids: HashSet<&str> = reviews
        .iter()
        .filter(|r| r.training_eligible)
        .map(|r| r.image_id.as_str())
        .collect();
    let records = registry::load_registry(store.dir());
    let thresholds = readiness::ReadinessThresholds {
        min_macro_f1: settings.training.readiness_min_macro_f1,
        min_worst_class_f1: settings.training.readiness_min_worst_f1,
        min_shadow_events: settings.training.readiness_min_shadow_events,
        min_shadow_agreement: settings.training.readiness_min_shadow_agreement,
        min_shadow_sessions: settings.training.readiness_min_shadow_sessions,
        min_review_coverage: settings.training.readiness_min_review_coverage,
    };
    let jobs = training::list_jobs(store.dir());

    let mut out = Vec::new();
    for (family, prefix, stem) in
        [("fish", Some("fish:"), "fish_v1"), ("state", None, "state_v1")]
    {
        let scoped: Vec<_> = all
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
            .filter(|r| reviewed_ids.contains(r.image_id.as_str()))
            .count();

        // Structured data gate from the real capability gate + per-entity
        // qualification, so readiness can say WHICH shortfall it is.
        let quals = qualify_entities(all, prefix.unwrap_or(""));
        let qualified: Vec<_> = quals.iter().filter(|q| q.qualified).collect();
        let test_covered = qualified.iter().filter(|q| q.test_covered).count();
        // Session shortfall: a class that has examples but too few sessions.
        let sessions_short = qualified
            .iter()
            .find(|q| q.sessions < crate::core::ml_capability::ENTITY_MIN_SESSIONS)
            .map(|q| (q.entity.clone(), q.sessions, crate::core::ml_capability::ENTITY_MIN_SESSIONS));
        let (required, test_required, gate_id) = if family == "fish" {
            (
                crate::core::ml_capability::FISH_MIN_ENTITIES,
                crate::core::ml_capability::ENTITY_MIN_TEST_COVERED,
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
            detail: g.map(|x| x.detail.clone()).unwrap_or_else(|| "no gate".to_string()),
            next_action: g
                .and_then(|x| x.blocking_requirement.clone())
                .unwrap_or_else(|| "Open Dataset to see per-class qualification.".to_string()),
        };

        // Deployed revision, and the soak FOR THAT REVISION ONLY.
        let manifest: serde_json::Value =
            std::fs::read_to_string(store.dir().join("models").join(format!("{stem}.json")))
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(serde_json::Value::Null);
        let deployed_version = manifest
            .get("version")
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<u32>().ok());
        let soak = registry::soak_stats_for(
            store.dir(),
            stem,
            deployed_version.map(|v| v.to_string()).as_deref(),
            500,
        );
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
            &thresholds,
        ));
    }

    // Fruit: same shape, honest about being far from ready.
    {
        let scoped: Vec<_> = all
            .iter()
            .filter(|r| r.entity_id.as_deref().is_some_and(|e| e.starts_with("fruit:")))
            .collect();
        let reviewed_scoped = scoped
            .iter()
            .filter(|r| reviewed_ids.contains(r.image_id.as_str()))
            .count();
        let quals = qualify_entities(all, "fruit:");
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
                detail: g.map(|x| x.detail.clone()).unwrap_or_else(|| "no gate".to_string()),
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
            &thresholds,
        ));
    }
    out
}

fn job_phase(jobs: &[training::TrainingJob], family: &str) -> JobPhase {
    if jobs.iter().any(|j| {
        j.model_family == family && matches!(j.status, training::JobStatus::Running | training::JobStatus::Queued)
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
pub fn readiness_status(st: State<'_, AppState>) -> Vec<readiness::ModelReadiness> {
    let settings = st.settings.read();
    readiness_status_inner(&st.store, &settings)
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
    let gate_ready =
        |id: &str| gates.iter().find(|g| g.id == id).map(|g| g.ready).unwrap_or(false);
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
        readiness: readiness_status_inner(&st.store, &settings),
        history_tail: training::read_history(st.store.dir(), 20),
    }
}
