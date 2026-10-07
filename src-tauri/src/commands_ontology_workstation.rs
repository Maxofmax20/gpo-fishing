//! Read and record for the ontology review workstation.
//!
//! # What this module is for
//!
//! The Python pipeline (`ml/gpo_ontology_review`) produces the evidence: 77
//! concepts, their representative screenshots, the KB entity list, and the
//! structural signals showing which decisions are not independent. This module
//! serves that evidence to the UI quickly, and records the ONE write the review
//! process needs: a human decision.
//!
//! # The single privileged action
//!
//! [`ontology_decide`] appends to the append-only `ontology_decisions.jsonl`.
//! That is the entire write surface.
//!
//! It does **not**, under any circumstance:
//!
//! * create or modify a knowledge-base entity (`knowledge.rs`)
//! * modify `FISH_VOCAB.txt` or the class map
//! * set `training_eligible`, or touch any dataset row
//! * change a train/validation/test split
//! * retrain, replace, register, or promote a model
//! * read or write readiness state
//! * reach game input
//!
//! A decision here is a PROPOSAL. The authoritative order remains:
//!
//! ```text
//! human decision -> validate -> proposed patch -> HUMAN APPLICATION -> KB update
//! ```
//! and no command in this file performs the application step.
//!
//! # Durability
//!
//! Decisions are appended by writing a full copy with the new line added, then
//! `fsync`-ing and atomically renaming over the original. A plain append could be
//! torn by a crash and leave a half-written JSON line, which the reader must then
//! reject - losing the whole log to protect one record. Copy-and-rename cannot
//! leave a partial log: either the old one is intact or the new one is.
//!
//! # Validation
//!
//! Shape validation is driven by `ml/gpo_ontology_review/validation_fixtures.json`,
//! which the Python test suite also reads, so the fast gate here and the
//! authoritative gate there cannot drift apart unnoticed. Cross-decision and
//! knowledge-base checks remain in Python's `validate` command, which still gates
//! patch export.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

// ===========================================================================
// Cached index
// ===========================================================================

/// The compact workstation index, parsed once per process.
///
/// Parsed lazily and held for the life of the process: re-reading 190 KB of JSON
/// on every keystroke would be the difference between a workstation that feels
/// instant and one that does not.
static INDEX: LazyLock<Option<serde_json::Value>> = LazyLock::new(|| {
    find_report_dir().and_then(|d| read_json(&d.join("workstation").join("index.json")))
});

static KB_ENTITIES: LazyLock<Option<serde_json::Value>> = LazyLock::new(|| {
    find_report_dir().and_then(|d| read_json(&d.join("workstation").join("kb_entities.json")))
});

static STRUCTURAL: LazyLock<Option<serde_json::Value>> = LazyLock::new(|| {
    find_report_dir().and_then(|d| read_json(&d.join("workstation").join("structural.json")))
});

/// Decision log re-read on each command. Deliberately NOT cached: decisions are
/// written constantly during a review session, and a cached log would show stale
/// progress and, worse, let an undo appear to fail.
static DECISION_LOG: LazyLock<PathBuf> = LazyLock::new(|| {
    find_report_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ontology_decisions.jsonl")
});

fn read_json(p: &Path) -> Option<serde_json::Value> {
    std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok())
}

/// Locate `reports/ontology_review/`.
fn find_report_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("GPO_ONTOLOGY_REVIEW_DIR") {
        let p = PathBuf::from(p);
        if p.join("workstation").join("index.json").is_file() {
            return Some(p);
        }
    }
    if let Ok(root) = std::env::var("GPO_REPO_ROOT") {
        let c = PathBuf::from(root).join("reports").join("ontology_review");
        if c.join("workstation").join("index.json").is_file() {
            return Some(c);
        }
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        let mut p = exe.as_path().parent();
        for _ in 0..8 {
            match p {
                Some(d) => {
                    roots.push(d.to_path_buf());
                    p = d.parent();
                }
                None => break,
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut p = Some(cwd.as_path());
        for _ in 0..8 {
            match p {
                Some(d) => {
                    roots.push(d.to_path_buf());
                    p = d.parent();
                }
                None => break,
            }
        }
    }
    roots
        .into_iter()
        .map(|r| r.join("reports").join("ontology_review"))
        .find(|c| c.join("workstation").join("index.json").is_file())
}

/// The dataset image directory, resolved the same way the Python pipeline does.
fn find_images_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("GPO_DATASET_ROOT") {
        let p = PathBuf::from(p).join("images");
        if p.is_dir() {
            return Some(p);
        }
    }
    let appdata = std::env::var("APPDATA").ok()?;
    let p = PathBuf::from(appdata)
        .join("gpo-autofish")
        .join("datasets")
        .join("gpo-vision")
        .join("v1")
        .join("images");
    p.is_dir().then_some(p)
}

/// Dataset image id -> true. Built once, so the thumbnail endpoint can reject a
/// path-traversal attempt in constant time rather than trusting the caller.
static KNOWN_IMAGE_IDS: LazyLock<Option<std::collections::HashSet<String>>> = LazyLock::new(|| {
    let idx = INDEX.as_ref()?;
    let ids = std::collections::HashSet::new();
    let mut out = ids;
    if let Some(cs) = idx.get("concepts").and_then(|c| c.as_array()) {
        for c in cs {
            if let Some(ev) = c.get("evidence").and_then(|e| e.as_array()) {
                for e in ev {
                    if let Some(id) = e.get("imageId").and_then(|x| x.as_str()) {
                        out.insert(id.to_string());
                    }
                }
            }
        }
    }
    Some(out)
});

/// An image id is a fixed 16-char lowercase hex string in this dataset. Anything
/// else cannot have come from the index, and rejecting it is what makes a
/// `..`-laden id impossible to turn into a path.
fn is_plausible_image_id(id: &str) -> bool {
    id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

// ===========================================================================
// Read-side payloads
// ===========================================================================

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkstationStatus {
    pub available: bool,
    pub detail: String,
    pub total: usize,
    pub counts: HashMap<String, usize>,
    pub remaining: HashMap<String, usize>,
    pub decided: usize,
    pub pending: usize,
    pub by_decision: HashMap<String, usize>,
    /// Concept ids still unresolved, already in review order.
    pub queue: Vec<String>,
    /// Concept ids in review order regardless of decision state.
    pub order: Vec<String>,
}

fn decision_kind_of(v: &serde_json::Value) -> Option<String> {
    v.get("decision").and_then(|d| d.as_str()).map(str::to_string)
}

/// Every recorded decision, keyed by concept, last one winning.
///
/// A malformed or unparseable line is SKIPPED rather than fatal: a reviewer
/// mid-session should never lose the screen because one earlier record is odd.
/// The Python reader is strict; this one is forgiving, and only for display.
fn read_decisions() -> (HashMap<String, serde_json::Value>, usize, HashMap<String, usize>) {
    let mut current: HashMap<String, serde_json::Value> = HashMap::new();
    let mut by_kind: HashMap<String, usize> = HashMap::new();
    let mut total = 0usize;
    let path = &*DECISION_LOG;
    if let Ok(text) = std::fs::read_to_string(path) {
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(cid) = v.get("concept_id").and_then(|x| x.as_str()) else {
                continue;
            };
            // A reversal record replaces its predecessor in the view.
            let kind = decision_kind_of(&v).unwrap_or_default();
            if kind == "REVERSAL" {
                if let Some(prev) = v.get("reverses_decision").and_then(|x| x.as_str()) {
                    if let Some(existing) = current.get(cid) {
                        if decision_kind_of(existing).as_deref() == Some(prev) {
                            current.remove(cid);
                            continue;
                        }
                    }
                }
            }
            if let Some(k) = decision_kind_of(&v) {
                *by_kind.entry(k).or_insert(0) += 1;
            }
            current.insert(cid.to_string(), v);
            total += 1;
        }
    }
    (current, total, by_kind)
}

#[tauri::command]
pub fn ontology_review_status() -> WorkstationStatus {
    let Some(idx) = INDEX.as_ref() else {
        return WorkstationStatus {
            available: false,
            detail: "No ontology review index found. Build it with: \
                     python -m ml.gpo_ontology_review build"
                .to_string(),
            total: 0,
            counts: HashMap::new(),
            remaining: HashMap::new(),
            decided: 0,
            pending: 0,
            by_decision: HashMap::new(),
            queue: vec![],
            order: vec![],
        };
    };

    let (current, _records, by_kind) = read_decisions();

    let concepts = idx.get("concepts").and_then(|c| c.as_array()).cloned().unwrap_or_default();
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut remaining: HashMap<String, usize> = HashMap::new();
    let mut queue = Vec::new();
    let mut order = Vec::new();
    let mut decided = 0usize;

    // `concepts` is already in review order, so iteration preserves it.
    for c in &concepts {
        let Some(cid) = c.get("conceptId").and_then(|x| x.as_str()) else {
            continue;
        };
        order.push(cid.to_string());
        let band = c.get("priority").and_then(|x| x.as_str()).unwrap_or("P3").to_string();
        *counts.entry(band.clone()).or_insert(0) += 1;
        if current.get(cid).and_then(decision_kind_of).is_some() {
            decided += 1;
        } else {
            *remaining.entry(band).or_insert(0) += 1;
            queue.push(cid.to_string());
        }
    }

    let total = order.len();
    WorkstationStatus {
        available: true,
        total,
        detail: format!(
            "{total} observed game name(s). {decided} decided, {} pending. \
             Recording a decision appends to the append-only log; nothing here \
             modifies the knowledge base.",
            total - decided
        ),
        pending: total - decided,
        counts,
        remaining,
        decided,
        by_decision: by_kind,
        queue,
        order,
    }
}

/// One concept in full, for the review screen.
#[tauri::command]
pub fn ontology_review_concept(concept_id: String) -> Option<serde_json::Value> {
    let idx = INDEX.as_ref()?;
    let concepts = idx.get("concepts")?.as_array()?;
    let mut found = concepts
        .iter()
        .find(|c| c.get("conceptId").and_then(|x| x.as_str()) == Some(concept_id.as_str()))
        .cloned()?;
    // Attach the current decision so the screen can show it without a second call.
    let (current, _, _) = read_decisions();
    if let Some(d) = current.get(&concept_id) {
        if let Some(o) = found.as_object_mut() {
            o.insert(
                "humanDecision".into(),
                serde_json::Value::String(decision_kind_of(d).unwrap_or_default()),
            );
            o.insert(
                "decisionNote".into(),
                d.get("notes").cloned().unwrap_or(serde_json::Value::Null),
            );
            o.insert("decidedAt".into(), d.get("timestamp").cloned().unwrap_or(serde_json::Value::Null));
        }
    }
    Some(found)
}

/// Structural signals: recurring modifiers, families, base-name analysis.
///
/// Evidence only. Nothing here decides a role for a word.
#[tauri::command]
pub fn ontology_review_structural() -> Option<serde_json::Value> {
    // Returned as built by the Python pipeline: it already carries each
    // modifier's and family's concept ids, so the panel can link to a sibling
    // without a second round trip.
    STRUCTURAL.as_ref().cloned()
}

/// A family and its members, with each member's representative images, for the
/// sibling comparison view.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyView {
    pub family_id: String,
    pub base_name: Option<String>,
    pub member_count: usize,
    pub occurrences: usize,
    pub decision_required: String,
    pub members: Vec<serde_json::Value>,
}

#[tauri::command]
pub fn ontology_review_family(family_id: String) -> Option<FamilyView> {
    let idx = INDEX.as_ref()?;
    let f = STRUCTURAL
        .as_ref()?
        .get("families")?
        .as_array()?
        .iter()
        .find(|x| x.get("familyId").and_then(|v| v.as_str()) == Some(family_id.as_str()))?
        .clone();

    let concepts = idx.get("concepts").and_then(|c| c.as_array())?;
    let (current, _, _) = read_decisions();

    let mut members = Vec::new();
    if let Some(fams) = f.get("members").and_then(|m| m.as_array()) {
        for m in fams {
            let Some(cid) = m.get("conceptId").and_then(|x| x.as_str()) else {
                continue;
            };
            let full = concepts
                .iter()
                .find(|c| c.get("conceptId").and_then(|x| x.as_str()) == Some(cid))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let mut entry = m.clone();
            if let Some(o) = entry.as_object_mut() {
                // Sibling comparison needs images, not just row counts.
                if let Some(ev) = full.get("evidence") {
                    o.insert("evidence".into(), ev.clone());
                }
                o.insert(
                    "modifiers".into(),
                    full.get("modifiers").cloned().unwrap_or(serde_json::Value::Array(vec![])),
                );
                o.insert(
                    "occurrences".into(),
                    full.get("occurrences").cloned().unwrap_or(serde_json::Value::from(0)),
                );
                o.insert(
                    "sessionCount".into(),
                    full.get("sessionCount").cloned().unwrap_or(serde_json::Value::from(0)),
                );
                o.insert(
                    "dayCount".into(),
                    full.get("dayCount").cloned().unwrap_or(serde_json::Value::from(0)),
                );
                o.insert(
                    "humanDecision".into(),
                    current
                        .get(cid)
                        .and_then(decision_kind_of)
                        .map(serde_json::Value::String)
                        .unwrap_or(serde_json::Value::Null),
                );
            }
            members.push(entry);
        }
    }

    Some(FamilyView {
        family_id,
        base_name: f.get("baseName").and_then(|x| x.as_str()).map(str::to_string),
        member_count: f.get("memberCount").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
        occurrences: f.get("occurrences").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
        decision_required: "are these one entity, several entities, a base fish plus \
                            modifiers, or unrelated names?"
            .to_string(),
        members,
    })
}

// ===========================================================================
// KB entity search
// ===========================================================================

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct KbEntityOption {
    pub id: String,
    pub canonical_name: String,
    pub aliases: Vec<String>,
    pub category: String,
    pub rarity: Option<String>,
    pub in_fish_vocab: bool,
}

/// Search canonical KB entities for the "existing entity" picker.
///
/// Returns suggestions only. The UI must not preselect one, and this command does
/// not choose: a suggestion is not a decision.
#[tauri::command]
pub fn ontology_review_kb_search(query: String, limit: Option<usize>) -> Vec<KbEntityOption> {
    let cap = limit.unwrap_or(50).min(200);
    let Some(kb) = KB_ENTITIES.as_ref() else {
        return vec![];
    };
    let Some(entities) = kb.get("entities").and_then(|e| e.as_array()) else {
        return vec![];
    };
    let q = query.trim().to_lowercase();
    let mut out: Vec<KbEntityOption> = Vec::new();

    for e in entities {
        let id = e.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let name = e.get("canonicalName").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let aliases: Vec<String> = e
            .get("aliases")
            .and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let hits = q.is_empty()
            || id.to_lowercase().contains(&q)
            || name.to_lowercase().contains(&q)
            || aliases.iter().any(|a| a.to_lowercase().contains(&q));
        if hits {
            out.push(KbEntityOption {
                id,
                canonical_name: name,
                aliases,
                category: e.get("category").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                rarity: e.get("rarity").and_then(|x| x.as_str()).map(str::to_string),
                in_fish_vocab: e.get("inFishVocab").and_then(|x| x.as_bool()).unwrap_or(false),
            });
        }
        if out.len() >= cap {
            break;
        }
    }
    out
}

// ===========================================================================
// Images
// ===========================================================================

/// A generated thumbnail as a data URL, or null when absent.
///
/// Returned inline rather than as a binary response so the browser can put it
/// straight into an ``img src`` with no second round trip per image and no blob
/// lifecycle to manage. Thumbnails are ~5 KB, so twelve per concept is ~60 KB of
/// base64 - small enough that fetching the whole concept's evidence at once beats
/// twelve sequential requests.
#[tauri::command]
pub fn ontology_review_thumb(image_id: String) -> Option<String> {
    if !is_plausible_image_id(&image_id) {
        return None;
    }
    let known = KNOWN_IMAGE_IDS.as_ref()?;
    if !known.contains(&image_id) {
        return None;
    }
    let dir = find_report_dir()?.join("workstation").join("thumbs");
    let bytes = std::fs::read(dir.join(format!("{image_id}.jpg"))).ok()?;
    Some(format!("data:image/jpeg;base64,{}", base64_encode(&bytes)))
}

/// Full-resolution source image, fetched only when the reviewer asks for it.
#[tauri::command]
pub fn ontology_review_full(image_id: String) -> Option<String> {
    if !is_plausible_image_id(&image_id) {
        return None;
    }
    let known = KNOWN_IMAGE_IDS.as_ref()?;
    if !known.contains(&image_id) {
        return None;
    }
    let dir = find_images_dir()?;
    let bytes = std::fs::read(dir.join(format!("{image_id}.png"))).ok()?;
    Some(format!("data:image/png;base64,{}", base64_encode(&bytes)))
}

fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

// ===========================================================================
// The one privileged action: record a human decision
// ===========================================================================

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionInput {
    pub concept_id: String,
    pub decision: String,
    #[serde(default)]
    pub existing_entity_id: Option<String>,
    #[serde(default)]
    pub proposed_entity_name: Option<String>,
    #[serde(default)]
    pub variant_descriptor: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub evidence_image_ids: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionResult {
    pub ok: bool,
    /// Populated on success; the next concept to review, already in order.
    pub next_concept_id: Option<String>,
    pub decided_total: usize,
    pub pending_total: usize,
    pub message: String,
}

/// A required-field problem.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionError {
    pub ok: bool,
    pub error: String,
}

/// Minimal validation mirroring `HumanDecision.validate()` in Python.
///
/// Driven by the shared fixture table so this fast gate and the authoritative
/// Python gate cannot drift; `tests/` asserts the agreement.
fn validate_shape(
    concept_id: &str,
    decision: &str,
    existing_entity_id: Option<&str>,
    proposed_entity_name: Option<&str>,
    variant_descriptor: Option<&str>,
    decision_maker: &str,
) -> Vec<String> {
    let mut problems = Vec::new();

    if decision_maker != "human" {
        problems.push(format!(
            "decisionMaker must be \"human\", got {decision_maker:?}; only a human may \
             resolve ontology ambiguity"
        ));
    }
    if concept_id.trim().is_empty() {
        problems.push("conceptId is required".into());
    }
    // A free function rather than a closure: the closure's return borrows its
    // argument, which the borrow checker cannot infer for a closure.
    fn non_empty(o: Option<&str>) -> Option<&str> {
        o.map(str::trim).filter(|s| !s.is_empty())
    }
    match decision {
        "UNKNOWN" => {}
        "EXISTING_ENTITY" => {
            if non_empty(existing_entity_id).is_none() {
                problems.push(
                    "EXISTING_ENTITY requires existingEntityId naming the entity the game \
                     refers to"
                        .into(),
                );
            }
        }
        "NEW_ENTITY" => {
            if non_empty(proposed_entity_name).is_none() {
                problems.push("NEW_ENTITY requires proposedEntityName".into());
            }
        }
        "VARIANT_OF_EXISTING" => {
            if non_empty(existing_entity_id).is_none() {
                problems.push(
                    "VARIANT_OF_EXISTING requires existingEntityId naming the base entity"
                        .into(),
                );
            }
            if non_empty(variant_descriptor).is_none() {
                problems.push(
                    "VARIANT_OF_EXISTING requires variantDescriptor, because calling \
                     something a variant is a claim about the base entity"
                        .into(),
                );
            }
        }
        other => problems.push(format!(
            "decision must be one of UNKNOWN, EXISTING_ENTITY, NEW_ENTITY, \
             VARIANT_OF_EXISTING; got {other:?}"
        )),
    }
    problems
}

fn now_iso8601() -> String {
    // Epoch milliseconds -> ISO-8601 UTC without pulling in a time crate.
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let secs = ms / 1000;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60,
        ms % 1000
    )
}

/// Append one decision durably: write a full copy, fsync, rename over.
///
/// Copy-and-rename rather than a plain append, because a plain append torn by a
/// crash leaves a half-written JSON line and the strict Python reader then rejects
/// the entire log - losing every decision to protect one. A rename is atomic, so
/// the log is either wholly the old version or wholly the new one.
fn append_decision(record: &serde_json::Value) -> Result<(), String> {
    let path = &*DECISION_LOG;
    let existing = std::fs::read_to_string(path).unwrap_or_default();

    let mut next = existing.clone();
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(&serde_json::to_string(record).map_err(|e| e.to_string())?);
    next.push('\n');

    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;

    // Same directory, so the rename stays on one filesystem and is therefore
    // atomic. A temp file elsewhere could degrade to a non-atomic copy.
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("decisions")
    ));
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(next.as_bytes()).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })?;
    Ok(())
}

/// Record a human ontology decision. THE ONLY WRITE IN THIS MODULE.
///
/// This is a PROPOSAL. It writes one line to an append-only log and nothing else:
/// no KB entity, no vocabulary entry, no label, no split, no model, no readiness
/// state. Applying the accumulated proposals is a separate, human-driven step
/// performed outside this codebase.
#[tauri::command]
pub fn ontology_decide(input: DecisionInput) -> Result<DecisionResult, DecisionError> {
    let concept_id = input.concept_id.trim().to_string();
    let decision = input.decision.trim().to_string();
    // The caller cannot choose the attribution; the UI has no parameter for it.
    let decision_maker = "human";

    // The concept must actually exist, or a typo would create a phantom decision.
    let exists = INDEX
        .as_ref()
        .and_then(|i| i.get("concepts"))
        .and_then(|c| c.as_array())
        .map(|cs| {
            cs.iter()
                .any(|c| c.get("conceptId").and_then(|x| x.as_str()) == Some(concept_id.as_str()))
        })
        .unwrap_or(false);
    if !exists {
        return Err(DecisionError {
            ok: false,
            error: format!("unknown concept {concept_id:?}; it is not in the review index"),
        });
    }

    let problems = validate_shape(
        &concept_id,
        &decision,
        input.existing_entity_id.as_deref(),
        input.proposed_entity_name.as_deref(),
        input.variant_descriptor.as_deref(),
        decision_maker,
    );
    if !problems.is_empty() {
        return Err(DecisionError {
            ok: false,
            error: format!("decision rejected:\n  - {}", problems.join("\n  - ")),
        });
    }

    // A NEW_ENTITY whose proposed id already exists is caught here rather than at
    // export time, so the reviewer learns immediately instead of after a batch.
    if decision == "NEW_ENTITY" {
        if let Some(name) = input.proposed_entity_name.as_deref() {
            if let Some(kb) = KB_ENTITIES.as_ref() {
                if let Some(ents) = kb.get("entities").and_then(|e| e.as_array()) {
                    let low = name.trim().to_lowercase();
                    let slug: String = low
                        .chars()
                        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                        .collect::<String>()
                        .trim_matches('-')
                        .to_string();
                    let want = format!("fish:{slug}");
                    if let Some(hit) = ents.iter().find(|e| {
                        e.get("id").and_then(|x| x.as_str()) == Some(want.as_str())
                            || e.get("canonicalName")
                                .and_then(|x| x.as_str())
                                .map(str::to_lowercase)
                                .as_deref()
                                == Some(low.as_str())
                    }) {
                        return Err(DecisionError {
                            ok: false,
                            error: format!(
                                "the knowledge base already has {} as {:?}; use \
                                 EXISTING_ENTITY instead of proposing a new entity",
                                hit.get("canonicalName").and_then(|x| x.as_str()).unwrap_or(&want),
                                hit.get("id").and_then(|x| x.as_str()).unwrap_or("")
                            ),
                        });
                    }
                }
            }
        }
    }

    let record = serde_json::json!({
        "concept_id": concept_id,
        "decision": decision,
        "existing_entity_id": input.existing_entity_id,
        "proposed_entity_name": input.proposed_entity_name,
        "variant_descriptor": input.variant_descriptor,
        "decision_maker": decision_maker,
        "timestamp": now_iso8601(),
        "evidence_image_ids": input.evidence_image_ids,
        "notes": input.notes.unwrap_or_default(),
        "source": "workstation-ui",
    });

    append_decision(&record).map_err(|e| DecisionError { ok: false, error: e })?;

    let status = ontology_review_status();
    let next = status
        .queue
        .iter()
        .find(|c| **c != concept_id)
        .cloned();

    Ok(DecisionResult {
        ok: true,
        next_concept_id: next,
        decided_total: status.decided,
        pending_total: status.pending,
        message: "Recorded. This is a PROPOSAL — the knowledge base is unchanged. \
                  Run `python -m ml.gpo_ontology_review validate` and then `export`."
            .to_string(),
    })
}

/// Undo the most recent decision for a concept.
///
/// Writes a REVERSAL record; it never deletes or edits the original. The audit
/// trail must show what was decided and that it was later reversed - silently
/// removing history would hide exactly the kind of change this process needs to be
/// able to account for.
#[tauri::command]
pub fn ontology_undo(concept_id: String) -> Result<WorkstationStatus, DecisionError> {
    let path = &*DECISION_LOG;
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut last: Option<(usize, serde_json::Value)> = None;
    for (i, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("concept_id").and_then(|x| x.as_str()) == Some(concept_id.as_str()) {
            last = Some((i, v));
        }
    }
    let Some((_, prev)) = last else {
        return Err(DecisionError {
            ok: false,
            error: format!("no decision recorded for {concept_id:?}"),
        });
    };
    let prev_kind = decision_kind_of(&prev).unwrap_or_default();
    if prev_kind == "REVERSAL" {
        return Err(DecisionError {
            ok: false,
            error: "the last record for this concept is already a reversal".into(),
        });
    }

    let reversal = serde_json::json!({
        "concept_id": concept_id,
        "decision": "REVERSAL",
        "reverses_decision": prev_kind,
        "reverses_timestamp": prev.get("timestamp"),
        "decision_maker": "human",
        "timestamp": now_iso8601(),
        "notes": "reversed via the workstation UI",
        "source": "workstation-ui",
    });
    append_decision(&reversal).map_err(|e| DecisionError { ok: false, error: e })?;
    Ok(ontology_review_status())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> serde_json::Value {
        let raw = include_str!("../../ml/gpo_ontology_review/validation_fixtures.json");
        serde_json::from_str(raw).expect("validation fixtures must be valid JSON")
    }

    /// The fast Rust gate and the authoritative Python gate read the SAME table,
    /// so a rule can only change in one place without a test failing here.
    #[test]
    fn rust_validation_agrees_with_the_shared_fixture_table() {
        let f = fixtures();
        let cases = f["cases"].as_array().expect("cases array");
        assert!(cases.len() >= 10, "the fixture table should be substantial");
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let fields = &case["fields"];
            let expected = case["valid"].as_bool().unwrap();
            let problems = validate_shape(
                fields["concept_id"].as_str().unwrap_or(""),
                fields["decision"].as_str().unwrap_or(""),
                fields.get("existing_entity_id").and_then(|v| v.as_str()),
                fields.get("proposed_entity_name").and_then(|v| v.as_str()),
                fields.get("variant_descriptor").and_then(|v| v.as_str()),
                fields["decision_maker"].as_str().unwrap_or(""),
            );
            assert_eq!(
                problems.is_empty(),
                expected,
                "fixture {name:?}: Rust says valid={} but the table says {expected}; \
                 the fast gate has drifted from the authoritative rules",
                problems.is_empty()
            );
            if let Some(must) = case.get("mustMention").and_then(|v| v.as_str()) {
                // The fixture names the canonical field (`existing_entity_id`);
                // the Rust message uses the UI's spelling (`existingEntityId`).
                // They are the same field, so compare with case and separators
                // removed rather than forcing the message to carry both.
                let norm = |x: &str| x.to_lowercase().chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .collect::<String>();
                let joined = norm(&problems.join(" | "));
                assert!(
                    joined.contains(&norm(must)),
                    "fixture {name:?}: expected the problem text to mention {must:?}, got {:?}",
                    problems.join(" | ")
                );
            }
        }
    }

    #[test]
    fn image_ids_are_validated_before_touching_the_filesystem() {
        // The traversal defence: a hostile id must be refused by shape, so it
        // never becomes a path at all.
        for bad in ["../secrets", "..", "a/../../b", "", "ZZZZZZZZZZZZZZZZ",
                    "0123456789abcde", "0123456789abcdefg"] {
            assert!(!is_plausible_image_id(bad), "{bad:?} must be refused");
        }
        assert!(is_plausible_image_id("606020203e34beb6"));
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn this_module_never_writes_the_knowledge_base_or_the_vocabulary() {
        // The guarantee that matters most, enforced against the source itself.
        // Only CODE is scanned, and only the non-test half:
        //
        //  * the sentinel strings live in this test, so scanning the whole file
        //    would match the check against itself;
        //  * the module's documentation deliberately NAMES these files, in the
        //    list of things the module does not do. That list is the point, so
        //    comments must be excluded or the guarantee would forbid the very
        //    sentence that states it.
        let full = include_str!("commands_ontology_workstation.rs");
        let src = full.split("#[cfg(test)]").next().unwrap_or(full);
        let code: String = src
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("//") && !t.starts_with("/*") && !t.starts_with('*')
            })
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "FISH_VOCAB",
            "knowledge.rs",
            "training_eligible",
            "class_vocab",
            "fish_v1",
            "registry",
            "readiness",
            "MARKETING",
        ] {
            assert!(
                !code.contains(forbidden),
                "the workstation must not reference {forbidden} in code; ontology review \
                 is not allowed to touch production artefacts"
            );
        }
    }

    #[test]
    fn the_only_write_target_is_the_decision_log() {
        let full = include_str!("commands_ontology_workstation.rs");
        let src = full.split("#[cfg(test)]").next().unwrap_or(full);
        let code: String = src
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // Exactly one write helper, and it is the append-and-rename.
        assert_eq!(code.matches("fn append_decision").count(), 1);
        assert!(code.contains("sync_all"), "the write must be fsynced");
        assert!(code.contains("std::fs::rename"), "the write must be atomic");
        // And the only file the append helper names is the decision log.
        let body: Vec<&str> = code
            .lines()
            .skip_while(|l| !l.contains("fn append_decision"))
            .take_while(|l| !l.trim_start().starts_with('}'))
            .collect();
        assert!(
            body.iter().any(|l| l.contains("DECISION_LOG")),
            "append_decision must write the decision log"
        );
    }
}