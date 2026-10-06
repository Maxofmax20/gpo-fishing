//! Persistent human-review store (`reviews.jsonl` + `review_audit.jsonl`).
//!
//! One line per `image_id` in `reviews.jsonl` holds the CURRENT review state;
//! every mutation also appends one event line to `review_audit.jsonl`
//! (append-only history: `REVIEW_CREATED` / `REVIEW_CHANGED` /
//! `REVIEW_CORRECTED` / `ENTITY_REMAP` / `REVIEW_EXCLUDED` / `REVIEW_RESTORED`
//! / `CONFLICT` / `RESOLVED`).
//!
//! Storage layout (directly under `data_dir`, no subdirectories):
//! ```text
//! <data_dir>/
//!   reviews.jsonl       (one ReviewRecord per line, keyed by image_id)
//!   review_audit.jsonl  (one AuditEvent per line, append-only)
//! ```
//!
//! Conflict rule: a second human verdict that disagrees with a previously
//! REVIEWED record never overwrites it — the record flips to `Conflict` and
//! must be settled explicitly via [`resolve_conflict`].

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

/// Reviewer tool version stamped on new/updated records.
pub const REVIEWER_VERSION: &str = "v5.6";

/// Audit event names (values stored in [`AuditEvent::event`]).
pub const AUDIT_CREATED: &str = "REVIEW_CREATED";
pub const AUDIT_CHANGED: &str = "REVIEW_CHANGED";
pub const AUDIT_CORRECTED: &str = "REVIEW_CORRECTED";
pub const AUDIT_REMAP: &str = "ENTITY_REMAP";
pub const AUDIT_EXCLUDED: &str = "REVIEW_EXCLUDED";
pub const AUDIT_RESTORED: &str = "REVIEW_RESTORED";
pub const AUDIT_CONFLICT: &str = "CONFLICT";
pub const AUDIT_RESOLVED: &str = "RESOLVED";

/// Review lifecycle state. Serialized SCREAMING_SNAKE_CASE for stable logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewStatus {
    /// No human has looked at this image yet.
    #[default]
    Unreviewed,
    /// Human confirmed the model prediction.
    ReviewedCorrect,
    /// Human supplied a different (corrected) entity.
    ReviewedCorrected,
    /// Human could not identify the entity (no `human_entity_id`).
    ReviewedUnknown,
    /// Human explicitly skipped this image.
    ReviewedSkipped,
    /// Two human verdicts disagree; needs [`resolve_conflict`].
    Conflict,
}

impl ReviewStatus {
    /// Canonical SCREAMING_SNAKE_CASE name (matches serde rendering).
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewStatus::Unreviewed => "UNREVIEWED",
            ReviewStatus::ReviewedCorrect => "REVIEWED_CORRECT",
            ReviewStatus::ReviewedCorrected => "REVIEWED_CORRECTED",
            ReviewStatus::ReviewedUnknown => "REVIEWED_UNKNOWN",
            ReviewStatus::ReviewedSkipped => "REVIEWED_SKIPPED",
            ReviewStatus::Conflict => "CONFLICT",
        }
    }

    /// Parse a wire/UI status name. Accepts the canonical
    /// SCREAMING_SNAKE_CASE form and the Rust variant form, case
    /// insensitively. `None` for anything unrecognised, so a bad filter
    /// value is detectable instead of silently matching nothing.
    pub fn parse(s: &str) -> Option<Self> {
        const ALL: [ReviewStatus; 6] = [
            ReviewStatus::Unreviewed,
            ReviewStatus::ReviewedCorrect,
            ReviewStatus::ReviewedCorrected,
            ReviewStatus::ReviewedUnknown,
            ReviewStatus::ReviewedSkipped,
            ReviewStatus::Conflict,
        ];
        let norm = s.trim().to_ascii_lowercase();
        ALL.into_iter().find(|st| {
            st.as_str().to_ascii_lowercase() == norm
                || format!("{st:?}").to_ascii_lowercase() == norm
        })
    }
}

/// True for verdicts that count as "reviewed" in [`coverage`]:
/// correct, corrected, or unknown. Skipped/conflict/unreviewed are
/// reported in their own buckets.
pub fn is_reviewed(status: ReviewStatus) -> bool {
    matches!(
        status,
        ReviewStatus::ReviewedCorrect
            | ReviewStatus::ReviewedCorrected
            | ReviewStatus::ReviewedUnknown
    )
}

/// Current review state for one image. Every `Option` field carries
/// `#[serde(default)]` so records written before a field existed still parse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewRecord {
    pub review_id: String,
    pub image_id: String,
    #[serde(default)]
    pub event_id: Option<String>,
    pub session_id: String,
    #[serde(default)]
    pub review_status: ReviewStatus,
    #[serde(default)]
    pub model_prediction: Option<String>,
    #[serde(default)]
    pub model_confidence: Option<f32>,
    #[serde(default)]
    pub human_entity_id: Option<String>,
    #[serde(default)]
    pub human_canonical_name: Option<String>,
    #[serde(default)]
    pub ocr_text: Option<String>,
    #[serde(default)]
    pub ocr_prediction: Option<String>,
    #[serde(default)]
    pub vision_prediction: Option<String>,
    #[serde(default)]
    pub vision_ocr_agreement: Option<bool>,
    #[serde(default)]
    pub is_hard_example: bool,
    #[serde(default)]
    pub reviewed_at: Option<u64>,
    #[serde(default)]
    pub reviewer_version: String,
    #[serde(default)]
    pub correction_reason: Option<String>,
    #[serde(default)]
    pub training_eligible: bool,
    #[serde(default)]
    pub excluded_reason: Option<String>,
    #[serde(default)]
    pub dataset_version: u32,
}

/// One append-only audit event in `review_audit.jsonl`.
///
/// v5.6.1: the event now carries the resulting [`ReviewRecord`] in full
/// (`record`), which is what makes the audit log **authoritative** rather
/// than decorative: [`ReviewStore::rebuild_from_audit`] replays these
/// payloads to reconstruct the effective state, so losing `reviews.jsonl` is
/// recoverable. Older lines without `record` still parse; they simply cannot
/// contribute to a rebuild (reported, not guessed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub at_ms: u64,
    #[serde(default)]
    pub image_id: String,
    /// One of the `AUDIT_*` constants.
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub detail: Option<String>,
    /// Post-transition state. Absent on pre-v5.6.1 lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<ReviewRecord>,
}

/// Real eligibility inputs. v5.6.0 hardcoded `true/false/true` inside the
/// store, so the recorded verdict never reflected the actual image, the
/// actual canonical mapping, or the actual provenance. The caller (which can
/// see the dataset row, the decoded PNG and the KB) now supplies all of them,
/// and this struct is the contract.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EligibilityInput {
    /// The PNG for this `image_id` exists and actually decodes.
    pub png_decodable: bool,
    /// `human_entity_id` literally exists as a KB entity id.
    pub canonical_valid: bool,
    /// Byte-identical / near-duplicate of an already-collected image.
    pub is_duplicate: bool,
    /// Session id and capture timestamp are present.
    pub has_provenance: bool,
    /// The human set an entity (independent of whether it is canonical).
    pub has_entity: bool,
}

impl EligibilityInput {
    /// An input that legitimately has nothing to check: used by pure
    /// library callers that have no dataset/PNG/KB access. Everything is
    /// treated as present EXCEPT canonical validity, which fails closed when
    /// no canonical name was supplied.
    pub fn library_default(has_entity: bool, canonical_name: Option<&str>) -> Self {
        Self {
            png_decodable: true,
            canonical_valid: canonical_name.is_some_and(|s| !s.trim().is_empty()),
            is_duplicate: false,
            has_provenance: true,
            has_entity,
        }
    }
}

/// Everything a human review needs to apply. Bundled so the store's own
/// eligibility cannot be bypassed by passing placeholders.
#[derive(Debug, Clone)]
pub struct ReviewInput<'a> {
    pub image_id: &'a str,
    pub session_id: &'a str,
    pub human_entity_id: Option<&'a str>,
    pub canonical_name: Option<&'a str>,
    pub correction_reason: Option<&'a str>,
    pub model_prediction: Option<&'a str>,
    pub model_confidence: Option<f32>,
    pub dataset_version: u32,
    pub eligibility: EligibilityInput,
}


/// Aggregate counts over a slice of review records.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageReport {
    pub total: usize,
    pub reviewed: usize,
    pub correct: usize,
    pub corrected: usize,
    pub unknown: usize,
    pub skipped: usize,
    pub conflicts: usize,
    pub sessions: usize,
    pub eligible: usize,
    pub excluded: usize,
}

/// File-backed review store: `reviews.jsonl` (current state, one line per
/// `image_id`) + `review_audit.jsonl` (append-only events).
pub struct ReviewStore {
    dir: PathBuf,
}

impl ReviewStore {
    pub fn new(data_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&data_dir);
        Self { dir: data_dir }
    }

    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    fn reviews_path(&self) -> PathBuf {
        self.dir.join("reviews.jsonl")
    }

    fn audit_path(&self) -> PathBuf {
        self.dir.join("review_audit.jsonl")
    }

    /// Read the state file, returning parsed records AND the raw text of any
    /// line that failed to parse.
    ///
    /// v5.6.1: v5.6.0 discarded unparseable lines and then rewrote the file
    /// from the survivors, so ONE corrupt line silently destroyed that review
    /// forever. Unparseable lines are now carried through untouched and
    /// re-emitted on the next write; they are also reported to the UI via
    /// [`ReviewStore::corrupt_lines`] so the operator can see them.
    fn read_state_raw(&self) -> (Vec<ReviewRecord>, Vec<String>) {
        let Ok(content) = std::fs::read_to_string(self.reviews_path()) else {
            return (Vec::new(), Vec::new());
        };
        let mut rows = Vec::new();
        let mut bad = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ReviewRecord>(line) {
                Ok(r) => rows.push(r),
                Err(_) => bad.push(line.to_string()),
            }
        }
        (rows, bad)
    }

    fn read_state(&self) -> Vec<ReviewRecord> {
        self.read_state_raw().0
    }

    /// Raw lines in `reviews.jsonl` that do not parse. Non-zero means part
    /// of the review history is unreadable (usually a crash mid-write or a
    /// record written by a newer version).
    pub fn corrupt_lines(&self) -> Vec<String> {
        self.read_state_raw().1
    }

    /// Current record for `image_id`, if any.
    pub fn get(&self, image_id: &str) -> Option<ReviewRecord> {
        self.read_state().into_iter().find(|r| r.image_id == image_id)
    }

    /// All current records, sorted by `image_id` for determinism.
    pub fn list(&self) -> Vec<ReviewRecord> {
        let mut rows = self.read_state();
        rows.sort_by(|a, b| a.image_id.cmp(&b.image_id));
        rows
    }

    /// Insert or replace `record` (keyed by `image_id`), append one audit
    /// event describing the transition (carrying the resulting record), and
    /// rewrite the state file atomically (tmp file + rename).
    ///
    /// The tmp name includes the process id so two writers can never stage
    /// into the same file and splice two states together.
    pub fn upsert(&self, record: &ReviewRecord) -> Result<(), String> {
        let (mut rows, unparsed) = self.read_state_raw();
        let old = rows.iter().find(|r| r.image_id == record.image_id).cloned();
        let (event, detail) = classify_transition(old.as_ref(), record);
        append_event(&self.audit_path(), &record.image_id, event, detail, Some(record))?;
        match rows.iter_mut().find(|r| r.image_id == record.image_id) {
            Some(slot) => *slot = record.clone(),
            None => rows.push(record.clone()),
        }
        rows.sort_by(|a, b| a.image_id.cmp(&b.image_id));
        let mut out = String::with_capacity(rows.len() * 512);
        for r in &rows {
            let line = serde_json::to_string(r).map_err(|e| e.to_string())?;
            out.push_str(&line);
            out.push('\n');
        }
        // Preserve unparseable history verbatim rather than deleting it.
        for l in &unparsed {
            out.push_str(l);
            out.push('\n');
        }
        if let Some(parent) = self.reviews_path().parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let tmp = self.dir.join(format!("reviews.jsonl.{}.tmp", std::process::id()));
        std::fs::write(&tmp, out.as_bytes()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, self.reviews_path()).map_err(|e| e.to_string())?;
        Ok(())
    }


    /// All audit events in append order ("tail" of the log).
    pub fn audit(&self) -> Vec<AuditEvent> {
        let Ok(content) = std::fs::read_to_string(self.audit_path()) else {
            return Vec::new();
        };
        content
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    /// Last `limit` audit events in append order.
    pub fn audit_tail(&self, limit: usize) -> Vec<AuditEvent> {
        let all = self.audit();
        if all.len() <= limit {
            all
        } else {
            all[all.len() - limit..].to_vec()
        }
    }

    /// Reconstruct the effective review state from the audit log.
    ///
    /// Each event carries the resulting record, so replaying the log in
    /// order reproduces `reviews.jsonl` exactly. This makes the append-only
    /// audit the recovery source of truth, and it is how a human can verify
    /// that the state file matches its own history.
    ///
    /// Returns the number of images reconstructed. Fails if any event lacks
    /// a `record` payload (pre-v5.6.1 log) rather than guessing.
    pub fn rebuild_from_audit(&self) -> Result<usize, String> {
        let events = self.audit();
        if events.is_empty() {
            return Ok(0);
        }
        let mut replay: std::collections::BTreeMap<String, ReviewRecord> =
            std::collections::BTreeMap::new();
        let mut missing = 0usize;
        for e in &events {
            match &e.record {
                Some(r) => {
                    replay.insert(r.image_id.clone(), r.clone());
                }
                None => missing += 1,
            }
        }
        if missing > 0 {
            return Err(format!(
                "audit log has {missing} event(s) without a record payload (pre-v5.6.1); \
                 cannot rebuild without guessing"
            ));
        }
        let rows: Vec<ReviewRecord> = replay.into_values().collect();
        let mut out = String::new();
        for r in &rows {
            out.push_str(&serde_json::to_string(r).map_err(|e| e.to_string())?);
            out.push('\n');
        }
        let tmp = self.dir.join(format!("reviews.jsonl.{}.rebuild", std::process::id()));
        std::fs::write(&tmp, out.as_bytes()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, self.reviews_path()).map_err(|e| e.to_string())?;
        Ok(rows.len())
    }

    /// Deterministic content fingerprint: FNV-1a hex over
    /// `image_id|STATUS|entity` lines sorted by `image_id`.
    pub fn fingerprint(&self) -> String {
        let rows = self.list();
        let mut lines: Vec<String> = rows
            .iter()
            .map(|r| {
                format!(
                    "{}|{}|{}",
                    r.image_id,
                    r.review_status.as_str(),
                    r.human_entity_id.as_deref().unwrap_or("")
                )
            })
            .collect();
        lines.sort();
        fnv1a_hex(lines.join("\n").as_bytes())
    }
}

/// Pick the audit event name + detail for an old → new transition.
fn classify_transition(
    old: Option<&ReviewRecord>,
    new: &ReviewRecord,
) -> (&'static str, Option<String>) {
    let Some(o) = old else {
        return (AUDIT_CREATED, Some(new.review_status.as_str().to_string()));
    };
    let transition = format!(
        "{}->{}",
        o.review_status.as_str(),
        new.review_status.as_str()
    );
    if new.review_status == ReviewStatus::Conflict {
        (AUDIT_CONFLICT, Some(transition))
    } else if o.review_status == ReviewStatus::Conflict {
        (AUDIT_RESOLVED, Some(transition))
    } else if o.training_eligible && !new.training_eligible {
        (
            AUDIT_EXCLUDED,
            Some(
                new.excluded_reason
                    .clone()
                    .unwrap_or_else(|| transition.clone()),
            ),
        )
    } else if !o.training_eligible && new.training_eligible {
        (AUDIT_RESTORED, Some(transition))
    } else if new.review_status == ReviewStatus::ReviewedCorrected {
        (AUDIT_CORRECTED, Some(transition))
    } else if o.human_entity_id != new.human_entity_id
        && o.human_entity_id.is_some()
        && new.human_entity_id.is_some()
    {
        (AUDIT_REMAP, Some(transition))
    } else {
        (AUDIT_CHANGED, Some(transition))
    }
}

fn append_event(
    path: &std::path::Path,
    image_id: &str,
    event: &str,
    detail: Option<String>,
    record: Option<&ReviewRecord>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let ev = AuditEvent {
        at_ms: now_ms(),
        image_id: image_id.to_string(),
        event: event.to_string(),
        detail,
        record: record.cloned(),
    };
    let mut line = serde_json::to_string(&ev).map_err(|e| e.to_string())?;
    line.push('\n');
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}

fn blank_record(image_id: &str, session_id: &str, dataset_version: u32) -> ReviewRecord {
    ReviewRecord {
        review_id: format!("rev-{image_id}"),
        image_id: image_id.to_string(),
        event_id: None,
        session_id: session_id.to_string(),
        review_status: ReviewStatus::Unreviewed,
        model_prediction: None,
        model_confidence: None,
        human_entity_id: None,
        human_canonical_name: None,
        ocr_text: None,
        ocr_prediction: None,
        vision_prediction: None,
        vision_ocr_agreement: None,
        is_hard_example: false,
        reviewed_at: None,
        reviewer_version: REVIEWER_VERSION.to_string(),
        correction_reason: None,
        training_eligible: false,
        excluded_reason: None,
        dataset_version,
    }
}

fn status_for(human_entity_id: Option<&str>, model_prediction: Option<&str>) -> ReviewStatus {
    match human_entity_id {
        None => ReviewStatus::ReviewedUnknown,
        Some(h) if model_prediction == Some(h) => ReviewStatus::ReviewedCorrect,
        Some(_) => ReviewStatus::ReviewedCorrected,
    }
}

/// Record (or refresh) a human verdict for `image_id`.
///
/// * First review stores `model_prediction`/`model_confidence` as the
///   immutable baseline; later calls never overwrite them once set.
/// * A verdict that disagrees with a previously REVIEWED record (different
///   `human_entity_id` over an existing one) flips the record to `Conflict`
///   and preserves the original entity — never a silent overwrite.
///   Settle it with [`resolve_conflict`].
/// * `None` entity records `ReviewedUnknown` (with the matching exclusion).
///
/// Eligibility is NEVER computed here from placeholders: the caller supplies
/// real measurements through [`ReviewInput::eligibility`].
pub fn apply_human_review(store: &ReviewStore, input: ReviewInput<'_>) -> Result<ReviewRecord, String> {
    let (eligible, excluded) = evaluate_input(&input.eligibility);
    let image_id = input.image_id;
    let rec = match store.get(image_id) {
        None => {
            let mut rec = blank_record(image_id, input.session_id, input.dataset_version);
            rec.review_status =
                status_for(input.human_entity_id, input.model_prediction);
            rec.model_prediction = input.model_prediction.map(str::to_string);
            rec.model_confidence = input.model_confidence;
            rec.human_entity_id = input.human_entity_id.map(str::to_string);
            rec.human_canonical_name = input.canonical_name.map(str::to_string);
            rec.reviewed_at = Some(now_ms());
            rec.correction_reason = input.correction_reason.map(str::to_string);
            rec.training_eligible = eligible;
            rec.excluded_reason = excluded;
            rec
        }
        Some(mut cur) => {
            let prev_reviewed = is_reviewed(cur.review_status);
            let differs = cur.human_entity_id.as_deref() != input.human_entity_id;
            if cur.review_status == ReviewStatus::Conflict {
                // Conflicted records stay conflicted until resolve_conflict.
                if input.correction_reason.is_some() {
                    cur.correction_reason = input.correction_reason.map(str::to_string);
                }
                cur.reviewed_at = Some(now_ms());
                cur.session_id = input.session_id.to_string();
                if cur.model_prediction.is_none() {
                    cur.model_prediction = input.model_prediction.map(str::to_string);
                }
                if cur.model_confidence.is_none() {
                    cur.model_confidence = input.model_confidence;
                }
                // A conflict stays OUT of training no matter what the new
                // submission computes. v5.6.0 let the caller's recomputed
                // `eligible` overwrite this demotion.
                cur.training_eligible = false;
                cur.excluded_reason = Some("conflict".to_string());
            } else if prev_reviewed && differs && cur.human_entity_id.is_some() {
                // Competing verdict (or retraction) over a past judgment.
                cur.review_status = ReviewStatus::Conflict;
                if input.correction_reason.is_some() {
                    cur.correction_reason = input.correction_reason.map(str::to_string);
                }
                cur.reviewed_at = Some(now_ms());
                cur.session_id = input.session_id.to_string();
                if cur.model_prediction.is_none() {
                    cur.model_prediction = input.model_prediction.map(str::to_string);
                }
                if cur.model_confidence.is_none() {
                    cur.model_confidence = input.model_confidence;
                }
                cur.training_eligible = false;
                cur.excluded_reason = Some("conflict".to_string());
            } else {
                // Fresh verdict (first label over unknown, reaffirmation, or
                // review of an unreviewed/skipped row). The original model
                // prediction is preserved; status + eligibility recompute.
                if cur.model_prediction.is_none() {
                    cur.model_prediction = input.model_prediction.map(str::to_string);
                }
                if cur.model_confidence.is_none() {
                    cur.model_confidence = input.model_confidence;
                }
                cur.human_entity_id = input.human_entity_id.map(str::to_string);
                if input.canonical_name.is_some() {
                    cur.human_canonical_name = input.canonical_name.map(str::to_string);
                }
                cur.review_status =
                    status_for(input.human_entity_id, cur.model_prediction.as_deref());
                cur.session_id = input.session_id.to_string();
                cur.reviewed_at = Some(now_ms());
                cur.reviewer_version = REVIEWER_VERSION.to_string();
                if input.correction_reason.is_some() {
                    cur.correction_reason = input.correction_reason.map(str::to_string);
                }
                cur.training_eligible = eligible;
                cur.excluded_reason = excluded;
                cur.dataset_version = input.dataset_version;
            }
            cur
        }
    };
    store.upsert(&rec)?;
    Ok(rec)
}

/// Undo the most recent review of `image_id`, restoring the state that
/// preceded it.
///
/// Safety model (never weakens anything):
/// * the audit log is APPEND-ONLY - undo adds an event, it never edits or
///   removes one, so the human mistake stays in the permanent history;
/// * the restored state is replayed from the audit's own `record` payloads,
///   so it is exactly the earlier effective state, not a reconstruction;
/// * if there is no earlier state, the record reverts to `Unreviewed` and
///   out of training;
/// * eligibility is re-evaluated conservatively (an undone review is not
///   training-eligible) and the reason says so.
pub fn undo_review(store: &ReviewStore, image_id: &str) -> Result<ReviewRecord, String> {
    let events = store.audit();
    // Last event for this image, and the one before that: the previous state.
    let mut prev: Option<ReviewRecord> = None;
    let mut found = false;
    for e in &events {
        if e.image_id != image_id {
            continue;
        }
        if let Some(r) = &e.record {
            if r.image_id == image_id {
                prev = Some(r.clone());
                found = true;
            }
        }
    }
    let mut cur = prev.clone().unwrap_or_else(|| {
        ReviewRecord {
            image_id: image_id.to_string(),
            review_status: ReviewStatus::Unreviewed,
            reviewer_version: REVIEWER_VERSION.to_string(),
            ..blank_record(image_id, "", 0)
        }
    });
    cur.review_status = ReviewStatus::Unreviewed;
    cur.training_eligible = false;
    cur.excluded_reason = Some("undone".to_string());
    cur.reviewed_at = Some(now_ms());
    cur.correction_reason = Some("undo".to_string());
    cur.reviewer_version = REVIEWER_VERSION.to_string();
    if !found && prev.is_none() {
        // Nothing at all in the audit for this image: still record the undo.
        cur.session_id = String::new();
    }
    let old = store.get(image_id);
    let (event, detail) = match &old {
        Some(o) => (
            AUDIT_RESTORED,
            Some(format!("undo {} -> UNREVIEWED", o.review_status.as_str())),
        ),
        None => (AUDIT_RESTORED, Some("undo: no prior state -> UNREVIEWED".to_string())),
    };
    append_event(
        &store.dir().join("review_audit.jsonl"),
        image_id,
        event,
        detail,
        Some(&cur),
    )?;
    store.upsert(&cur)?;
    Ok(cur)
}

/// Settle a `Conflict` record: the chosen entity wins with status
/// `ReviewedCorrected` (audit `RESOLVED`).
///
/// v5.6.1: eligibility is supplied by the caller, which checks the entity
/// against the knowledge base. v5.6.0 derived it from a stored *display
/// name* that the resolve command never supplied, so EVERY conflict resolved
/// through the UI was permanently excluded as "invalid canonical mapping".
pub fn resolve_conflict(
    store: &ReviewStore,
    image_id: &str,
    entity_id: &str,
    canonical_name: Option<&str>,
    reason: &str,
    eligibility: &EligibilityInput,
) -> Result<ReviewRecord, String> {
    let mut cur = store
        .get(image_id)
        .ok_or_else(|| format!("no review for image '{image_id}'"))?;
    cur.human_entity_id = Some(entity_id.to_string());
    if canonical_name.is_some() {
        cur.human_canonical_name = canonical_name.map(str::to_string);
    }
    cur.review_status = ReviewStatus::ReviewedCorrected;
    cur.correction_reason = Some(reason.to_string());
    cur.reviewed_at = Some(now_ms());
    cur.reviewer_version = REVIEWER_VERSION.to_string();
    let (ok, why) = evaluate_input(eligibility);
    cur.training_eligible = ok;
    cur.excluded_reason = why;
    store.upsert(&cur)?;
    Ok(cur)
}

/// Mark an image explicitly skipped (stays out of training with reason
/// `"skipped"`). Preserves any existing verdict fields.
pub fn skip_review(
    store: &ReviewStore,
    image_id: &str,
    session_id: &str,
    reason: Option<String>,
    dataset_version: u32,
) -> Result<ReviewRecord, String> {
    let mut cur = store
        .get(image_id)
        .unwrap_or_else(|| blank_record(image_id, session_id, dataset_version));
    cur.review_status = ReviewStatus::ReviewedSkipped;
    cur.session_id = session_id.to_string();
    cur.reviewed_at = Some(now_ms());
    cur.reviewer_version = REVIEWER_VERSION.to_string();
    if reason.is_some() {
        cur.correction_reason = reason;
    }
    cur.training_eligible = false;
    cur.excluded_reason = Some("skipped".to_string());
    cur.dataset_version = dataset_version;
    store.upsert(&cur)?;
    Ok(cur)
}

/// Training eligibility (pure, no I/O). Returns `(true, None)` when the row
/// may train, else `(false, Some(exact_reason))` with one of: `"bad image"`,
/// `"unknown (no entity)"`, `"invalid canonical mapping"`, `"duplicate"`,
/// `"insufficient provenance"`.
pub fn evaluate_eligibility(
    is_png_decodable: bool,
    human_entity_id: Option<&str>,
    canonical_valid: bool,
    is_duplicate: bool,
    has_provenance: bool,
) -> (bool, Option<String>) {
    if !is_png_decodable {
        return (false, Some("bad image".to_string()));
    }
    let entity = human_entity_id
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if entity.is_none() {
        return (false, Some("unknown (no entity)".to_string()));
    }
    if !canonical_valid {
        return (false, Some("invalid canonical mapping".to_string()));
    }
    if is_duplicate {
        return (false, Some("duplicate".to_string()));
    }
    if !has_provenance {
        return (false, Some("insufficient provenance".to_string()));
    }
    (true, None)
}

/// Evaluate a real [`EligibilityInput`]. Same fail-closed ordering as
/// [`evaluate_eligibility`], but the caller supplies the facts instead of
/// the store assuming them.
pub fn evaluate_input(e: &EligibilityInput) -> (bool, Option<String>) {
    evaluate_eligibility(
        e.png_decodable,
        if e.has_entity { Some("x") } else { None },
        e.canonical_valid,
        e.is_duplicate,
        e.has_provenance,
    )
}

/// Aggregate counts. `reviewed` = correct + corrected + unknown;
/// skipped and conflicts live in their own buckets; `eligible` counts
/// `training_eligible` rows and `excluded` is the remainder.
pub fn coverage(records: &[ReviewRecord]) -> CoverageReport {
    let mut rep = CoverageReport {
        total: records.len(),
        ..CoverageReport::default()
    };
    let mut sessions = HashSet::new();
    for r in records {
        sessions.insert(r.session_id.as_str());
        match r.review_status {
            ReviewStatus::ReviewedCorrect => rep.correct += 1,
            ReviewStatus::ReviewedCorrected => rep.corrected += 1,
            ReviewStatus::ReviewedUnknown => rep.unknown += 1,
            ReviewStatus::ReviewedSkipped => rep.skipped += 1,
            ReviewStatus::Conflict => rep.conflicts += 1,
            ReviewStatus::Unreviewed => {}
        }
        if r.training_eligible {
            rep.eligible += 1;
        }
    }
    rep.reviewed = rep.correct + rep.corrected + rep.unknown;
    rep.sessions = sessions.len();
    rep.excluded = rep.total.saturating_sub(rep.eligible);
    rep
}

/// Deterministic review-priority score (0..=254) plus a human reason string.
///
/// Weights are powers of two so higher-priority flags strictly dominate any
/// combination of lower ones: unknown predictions first, then disagreement,
/// low confidence, new class, underrepresented class, unseen session, hard
/// example, ordinary last.
pub fn priority_score(
    has_model: bool,
    ocr_disagree: bool,
    low_conf: bool,
    is_new_class: bool,
    underrep: bool,
    unseen_session: bool,
    hard: bool,
) -> (u8, &'static str) {
    let score = (!has_model as u8) * 128
        + (ocr_disagree as u8) * 64
        + (low_conf as u8) * 32
        + (is_new_class as u8) * 16
        + (underrep as u8) * 8
        + (unseen_session as u8) * 4
        + (hard as u8) * 2;
    let reason = if !has_model {
        "unknown prediction"
    } else if ocr_disagree {
        "vision/OCR disagreement"
    } else if low_conf {
        "low model confidence"
    } else if is_new_class {
        "new class"
    } else if underrep {
        "underrepresented class"
    } else if unseen_session {
        "unseen session"
    } else if hard {
        "hard example"
    } else {
        "ordinary"
    };
    (score, reason)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// FNV-1a 64-bit hex (std only — no hash crates). Deterministic across runs
/// (unlike SipHash), matching the dataset module's split-hash approach.
fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store(name: &str) -> (ReviewStore, PathBuf) {
        static C: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = C.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("gpo-review-{name}-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        (ReviewStore::new(dir.clone()), dir)
    }

    fn sample_record(image: &str, session: &str, status: ReviewStatus, eligible: bool) -> ReviewRecord {
        ReviewRecord {
            review_id: format!("rev-{image}"),
            image_id: image.to_string(),
            event_id: None,
            session_id: session.to_string(),
            review_status: status,
            model_prediction: Some("fruit:suna".to_string()),
            model_confidence: Some(0.9),
            human_entity_id: Some("fruit:suna".to_string()),
            human_canonical_name: Some("Suna".to_string()),
            ocr_text: None,
            ocr_prediction: None,
            vision_prediction: None,
            vision_ocr_agreement: None,
            is_hard_example: false,
            reviewed_at: Some(1),
            reviewer_version: REVIEWER_VERSION.to_string(),
            correction_reason: None,
            training_eligible: eligible,
            excluded_reason: None,
            dataset_version: 1,
        }
    }


    /// Test convenience wrapper: applies a review with the library-default
    /// eligibility (canonical validity inferred from the supplied name).
    #[allow(clippy::too_many_arguments)]
    fn apply_test(
        store: &ReviewStore,
        image_id: &str,
        session_id: &str,
        human_entity_id: Option<&str>,
        correction_reason: Option<&str>,
        canonical_name: Option<&str>,
        model_prediction: Option<&str>,
        model_confidence: Option<f32>,
    ) -> ReviewRecord {
        apply_human_review(
            store,
            ReviewInput {
                image_id,
                session_id,
                human_entity_id,
                canonical_name,
                correction_reason,
                model_prediction,
                model_confidence,
                dataset_version: 1,
                eligibility: EligibilityInput::library_default(
                    human_entity_id.is_some(),
                    canonical_name,
                ),
            },
        )
        .expect("review must apply")
    }

    /// Eligibility with everything present (canonical mapping valid).
    fn elig(has_entity: bool) -> EligibilityInput {
        EligibilityInput {
            png_decodable: true,
            canonical_valid: true,
            is_duplicate: false,
            has_provenance: true,
            has_entity,
        }
    }

    #[test]
    fn create_get_roundtrip() {
        let (store, _dir) = test_store("roundtrip");
        let rec = apply_test(&store, "img-001", "sess-a", Some("fruit:suna"), None, Some("Suna"), Some("fruit:suna"), Some(0.9));
        assert_eq!(rec.review_status, ReviewStatus::ReviewedCorrect);
        assert!(rec.training_eligible);
        assert_eq!(rec.excluded_reason, None);
        let got = store.get("img-001").expect("record must exist");
        assert_eq!(got, rec);
        assert_eq!(store.list().len(), 1);
        assert!(store.get("missing").is_none());
    }

    #[test]
    fn corrected_preserves_prediction_and_conflicts_on_disagreement() {
        let (store, _dir) = test_store("conflict");
        // First review: human corrects model suna -> mera.
        let first = apply_test(&store, "img-010", "sess-a", Some("fruit:mera"), Some("looks like mera"), Some("Mera"), Some("fruit:suna"), Some(0.4));
        assert_eq!(first.review_status, ReviewStatus::ReviewedCorrected);
        assert_eq!(first.model_prediction.as_deref(), Some("fruit:suna"));
        // Reaffirming the same entity is not a conflict.
        let same = apply_test(&store, "img-010", "sess-a", Some("fruit:mera"), None, Some("Mera"), Some("fruit:suna"), Some(0.99));
        assert_eq!(same.review_status, ReviewStatus::ReviewedCorrected);
        assert_eq!(same.human_entity_id.as_deref(), Some("fruit:mera"));
        // A competing entity flips to Conflict and preserves the original.
        let conflict = apply_test(&store, "img-010", "sess-a", Some("fruit:suna"), Some("second opinion"), Some("Suna"), None, None);
        assert_eq!(conflict.review_status, ReviewStatus::Conflict);
        assert_eq!(conflict.human_entity_id.as_deref(), Some("fruit:mera"));
        assert_eq!(conflict.model_prediction.as_deref(), Some("fruit:suna"));
        assert_eq!(conflict.model_confidence, Some(0.4));
        assert!(!conflict.training_eligible);
        let stored = store.get("img-010").unwrap();
        assert_eq!(stored, conflict);
    }

    #[test]
    fn unknown_and_skip_flows() {
        let (store, _dir) = test_store("unknownskip");
        let unk = apply_test(&store, "img-020", "sess-a", None, None, None, Some("fruit:suna"), Some(0.2));
        assert_eq!(unk.review_status, ReviewStatus::ReviewedUnknown);
        assert!(!unk.training_eligible);
        assert_eq!(unk.excluded_reason.as_deref(), Some("unknown (no entity)"));
        let skipped = skip_review(
            &store,
            "img-020",
            "sess-a",
            Some("too blurry".to_string()),
            1,
        )
        .unwrap();
        assert_eq!(skipped.review_status, ReviewStatus::ReviewedSkipped);
        assert!(!skipped.training_eligible);
        // Unknown over a skipped row is a fresh verdict path, not a conflict.
        let relabeled = apply_test(&store, "img-021", "sess-a", None, None, None, None, None);
        assert_eq!(relabeled.review_status, ReviewStatus::ReviewedUnknown);
    }

    #[test]
    fn resolve_conflict_sets_corrected_with_resolved_audit() {
        let (store, _dir) = test_store("resolve");
        apply_test(&store, "img-030", "sess-a", Some("fruit:mera"), None, Some("Mera"), Some("fruit:suna"), Some(0.4));
        let conflict = apply_test(&store, "img-030", "sess-a", Some("fruit:suna"), None, Some("Suna"), None, None);
        assert_eq!(conflict.review_status, ReviewStatus::Conflict);
        let resolved = resolve_conflict(&store, "img-030", "fruit:suna", Some("Suna"), "curator pick", &elig(true)).unwrap();
        assert_eq!(resolved.review_status, ReviewStatus::ReviewedCorrected);
        assert_eq!(resolved.human_entity_id.as_deref(), Some("fruit:suna"));
        assert_eq!(resolved.correction_reason.as_deref(), Some("curator pick"));
        assert!(resolve_conflict(&store, "img-999", "fruit:suna", Some("Suna"), "x", &elig(true)).is_err());
        let events: Vec<String> = store.audit().iter().map(|e| e.event.clone()).collect();
        assert_eq!(events, vec!["REVIEW_CREATED", "CONFLICT", "RESOLVED"]);
    }

    #[test]
    fn eligibility_reasons_cover_each_exclusion() {
        assert_eq!(
            evaluate_eligibility(false, Some("fruit:suna"), true, false, true),
            (false, Some("bad image".to_string()))
        );
        assert_eq!(
            evaluate_eligibility(true, None, true, false, true),
            (false, Some("unknown (no entity)".to_string()))
        );
        assert_eq!(
            evaluate_eligibility(true, Some("  "), true, false, true),
            (false, Some("unknown (no entity)".to_string()))
        );
        assert_eq!(
            evaluate_eligibility(true, Some("fruit:suna"), false, false, true),
            (false, Some("invalid canonical mapping".to_string()))
        );
        assert_eq!(
            evaluate_eligibility(true, Some("fruit:suna"), true, true, true),
            (false, Some("duplicate".to_string()))
        );
        assert_eq!(
            evaluate_eligibility(true, Some("fruit:suna"), true, false, false),
            (false, Some("insufficient provenance".to_string()))
        );
        assert_eq!(
            evaluate_eligibility(true, Some("fruit:suna"), true, false, true),
            (true, None)
        );
    }

    #[test]
    fn coverage_counts_each_bucket() {
        let mut rows = vec![
            sample_record("a", "s1", ReviewStatus::ReviewedCorrect, true),
            sample_record("b", "s1", ReviewStatus::ReviewedCorrect, true),
            sample_record("c", "s2", ReviewStatus::ReviewedCorrected, true),
            sample_record("d", "s2", ReviewStatus::ReviewedUnknown, false),
            sample_record("e", "s3", ReviewStatus::ReviewedSkipped, false),
            sample_record("f", "s3", ReviewStatus::Conflict, false),
            sample_record("g", "s3", ReviewStatus::Unreviewed, false),
        ];
        rows[3].training_eligible = false;
        rows[3].excluded_reason = Some("unknown (no entity)".to_string());
        rows[3].human_entity_id = None;
        let rep = coverage(&rows);
        assert_eq!(rep.total, 7);
        assert_eq!(rep.reviewed, 4);
        assert_eq!(rep.correct, 2);
        assert_eq!(rep.corrected, 1);
        assert_eq!(rep.unknown, 1);
        assert_eq!(rep.skipped, 1);
        assert_eq!(rep.conflicts, 1);
        assert_eq!(rep.sessions, 3);
        assert_eq!(rep.eligible, 3);
        assert_eq!(rep.excluded, 4);
        assert!(coverage(&[]).total == 0 && coverage(&[]).excluded == 0);
    }

    #[test]
    fn priority_orders_unknown_above_disagreement_above_ordinary() {
        let (s_unknown, r_unknown) = priority_score(false, false, false, false, false, false, false);
        let (s_disagree, r_disagree) = priority_score(true, true, false, false, false, false, false);
        let (s_ordinary, r_ordinary) =
            priority_score(true, false, false, false, false, false, false);
        assert!(s_unknown > s_disagree, "{s_unknown} > {s_disagree}");
        assert!(s_disagree > s_ordinary, "{s_disagree} > {s_ordinary}");
        assert_eq!(s_ordinary, 0);
        assert_eq!(r_unknown, "unknown prediction");
        assert_eq!(r_disagree, "vision/OCR disagreement");
        assert_eq!(r_ordinary, "ordinary");
        // Unknown dominates every lower-flag combination.
        let (s_all_lower, _) = priority_score(true, true, true, true, true, true, true);
        assert!(s_unknown > s_all_lower);
        // Reason picks the highest-priority applicable flag.
        assert_eq!(priority_score(true, false, true, false, false, false, false).1, "low model confidence");
        assert_eq!(priority_score(true, false, false, true, false, false, false).1, "new class");
        assert_eq!(priority_score(true, false, false, false, true, false, false).1, "underrepresented class");
        assert_eq!(priority_score(true, false, false, false, false, true, false).1, "unseen session");
        assert_eq!(priority_score(true, false, false, false, false, false, true).1, "hard example");
        // Deterministic.
        assert_eq!(
            priority_score(true, true, false, true, false, false, true),
            priority_score(true, true, false, true, false, false, true)
        );
    }

    #[test]
    fn audit_log_appends_events_in_order() {
        let (store, _dir) = test_store("audit");
        assert!(store.audit().is_empty());
        apply_test(&store, "img-040", "sess-a", Some("fruit:suna"), None, Some("Suna"), Some("fruit:suna"), Some(0.9));
        apply_test(&store, "img-041", "sess-a", Some("fruit:mera"), None, Some("Mera"), None, None);
        let events = store.audit();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event, "REVIEW_CREATED");
        assert_eq!(events[0].image_id, "img-040");
        assert_eq!(events[1].image_id, "img-041");
        assert!(events[0].at_ms > 0);
        // Reloading from disk preserves order.
        let reopened = ReviewStore::new(store.dir().to_path_buf());
        let tail: Vec<String> = reopened.audit_tail(1).iter().map(|e| e.event.clone()).collect();
        assert_eq!(tail, vec!["REVIEW_CREATED"]);
    }

    #[test]
    fn old_json_without_new_option_fields_still_parses() {
        let old = r#"{"review_id":"rev-x","image_id":"x","session_id":"s","review_status":"REVIEWED_CORRECT","reviewer_version":"v5.0","dataset_version":1}"#;
        let rec: ReviewRecord = serde_json::from_str(old).expect("old record must parse");
        assert_eq!(rec.image_id, "x");
        assert_eq!(rec.review_status, ReviewStatus::ReviewedCorrect);
        assert_eq!(rec.event_id, None);
        assert_eq!(rec.model_prediction, None);
        assert_eq!(rec.model_confidence, None);
        assert_eq!(rec.human_entity_id, None);
        assert_eq!(rec.human_canonical_name, None);
        assert_eq!(rec.ocr_text, None);
        assert_eq!(rec.ocr_prediction, None);
        assert_eq!(rec.vision_prediction, None);
        assert_eq!(rec.vision_ocr_agreement, None);
        assert_eq!(rec.reviewed_at, None);
        assert_eq!(rec.correction_reason, None);
        assert_eq!(rec.excluded_reason, None);
        assert!(!rec.is_hard_example);
        assert!(!rec.training_eligible);
    }

    #[test]
    fn status_roundtrips_as_screaming_snake_case() {
        let s = serde_json::to_string(&ReviewStatus::ReviewedCorrected).unwrap();
        assert_eq!(s, "\"REVIEWED_CORRECTED\"");
        assert_eq!(
            serde_json::from_str::<ReviewStatus>("\"CONFLICT\"").unwrap(),
            ReviewStatus::Conflict
        );
    }

    #[test]
    fn fingerprint_is_stable_and_sensitive_to_state() {
        let (a, _d1) = test_store("fp-a");
        let (b, _d2) = test_store("fp-b");
        assert_eq!(a.fingerprint(), b.fingerprint(), "empty stores match");
        for store in [&a, &b] {
            store.upsert(&sample_record("x", "s1", ReviewStatus::ReviewedCorrect, true)).unwrap();
            store.upsert(&sample_record("y", "s1", ReviewStatus::ReviewedUnknown, false)).unwrap();
        }
        assert_eq!(a.fingerprint(), b.fingerprint());
        let mut changed = sample_record("y", "s1", ReviewStatus::ReviewedCorrected, true);
        changed.human_entity_id = Some("fruit:mera".to_string());
        b.upsert(&changed).unwrap();
        assert_ne!(a.fingerprint(), b.fingerprint());
    }
}
