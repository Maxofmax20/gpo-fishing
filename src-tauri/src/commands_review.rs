//! Human review + canonical entities + readiness command surface (v5.6).
//!
//! All values come from real stores (dataset rows, review records, KB,
//! registry, shadow log). PNGs are served as base64 (same pattern as the
//! existing region_preview); the CSP already allows data: images.

use std::collections::{HashMap, HashSet};

use base64::Engine;
use serde::Serialize;
use tauri::State;

use crate::app::AppState;
use crate::core::canon::{self, Resolution};
use crate::core::ml_capability::assess_capabilities;
use crate::core::ml_dataset::{MlDatasetStore, DATASET_VERSION};
use crate::core::readiness;
use crate::core::registry;
use crate::core::review::{self, ReviewRecord, ReviewStore};
use crate::core::training;

fn local_normalize(s: &str) -> String {
    // Heuristic port of fruit::normalize (see ml_capability for the twin).
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn review_store(data_dir: &std::path::Path) -> ReviewStore {
    ReviewStore::new(data_dir.to_path_buf())
}

// ---- PNG serving ----

#[derive(Debug, Serialize)]
pub struct ReviewImage {
    pub image_id: String,
    pub width: u32,
    pub height: u32,
    pub png_base64: String,
}

#[tauri::command]
pub fn review_image(st: State<'_, AppState>, image_id: String) -> Result<ReviewImage, String> {
    if image_id.contains('/') || image_id.contains('\\') || image_id.contains("..") {
        return Err("invalid image id".to_string());
    }
    let images = MlDatasetStore::new(st.store.dir().to_path_buf()).root().join("images");
    let path = images.join(format!("{image_id}.png"));
    let bytes = std::fs::read(&path).map_err(|_| format!("no image for '{image_id}'"))?;
    let img = image::load_from_memory(&bytes).map_err(|_| format!("undecodable image '{image_id}'"))?;
    let rgba = img.to_rgba8();
    Ok(ReviewImage {
        image_id,
        width: rgba.width(),
        height: rgba.height(),
        png_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
    })
}

// ---- review records ----

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
        v.retain(|r| format!("{:?}", r.review_status).eq_ignore_ascii_case(&s));
    }
    v.sort_by_key(|r| std::cmp::Reverse(r.reviewed_at.unwrap_or(0)));
    let off = offset.unwrap_or(0);
    let lim = limit.unwrap_or(50).clamp(1, 200);
    v.into_iter().skip(off).take(lim).collect()
}

#[derive(Debug, Serialize)]
pub struct ReviewCoverageView {
    pub total_rows: usize,
    pub coverage: review::CoverageReport,
    pub per_entity: Vec<PerEntityReview>,
}

#[derive(Debug, Serialize)]
pub struct PerEntityReview {
    pub entity: String,
    pub collected: usize,
    pub reviewed: usize,
    pub confirmed: usize,
    pub corrected: usize,
    pub unknown: usize,
    pub sessions: usize,
    pub eligible: usize,
}

#[tauri::command]
pub fn review_coverage(st: State<'_, AppState>) -> ReviewCoverageView {
    let rows = MlDatasetStore::new(st.store.dir().to_path_buf()).annotations();
    let store = review_store(st.store.dir());
    let records = store.list();
    let coverage = review::coverage(&records);
    let by_image: HashMap<&str, &ReviewRecord> =
        records.iter().map(|r| (r.image_id.as_str(), r)).collect();
    let mut per: HashMap<String, PerEntityReview> = HashMap::new();
    let mut ent_sessions: HashMap<String, HashSet<String>> = HashMap::new();
    for r in &rows {
        let Some(e) = r.entity_id.as_deref() else { continue };
        let entry = per.entry(e.to_string()).or_insert(PerEntityReview {
            entity: e.to_string(), collected: 0, reviewed: 0, confirmed: 0,
            corrected: 0, unknown: 0, sessions: 0, eligible: 0,
        });
        entry.collected += 1;
        ent_sessions.entry(e.to_string()).or_default().insert(r.session_id.clone());
        if let Some(rev) = by_image.get(r.image_id.as_str()) {
            use review::ReviewStatus::*;
            match rev.review_status {
                ReviewedCorrect => {
                    entry.reviewed += 1;
                    entry.confirmed += 1;
                }
                ReviewedCorrected => {
                    entry.reviewed += 1;
                    entry.corrected += 1;
                }
                ReviewedUnknown => {
                    entry.reviewed += 1;
                    entry.unknown += 1;
                }
                _ => {}
            }
            if rev.training_eligible {
                entry.eligible += 1;
            }
        }
    }
    let mut v: Vec<PerEntityReview> = per
        .into_iter()
        .map(|(e, mut x)| {
            x.sessions = ent_sessions.get(&e).map(|s| s.len()).unwrap_or(0);
            x
        })
        .collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.collected));
    ReviewCoverageView { total_rows: rows.len(), coverage, per_entity: v }
}

#[derive(Debug, Serialize)]
pub struct ReviewApplyResult {
    pub record: ReviewRecord,
    pub dataset_updated: bool,
}

/// Apply a human review. Entity identities must be stable KB ids; anything
/// else is recorded but excluded from training. Confirming also writes the
/// dataset label through the SAME annotate() path as Setup (never a side
/// channel), so review and dataset can never disagree.
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
    let row = ds
        .annotations()
        .into_iter()
        .find(|r| r.image_id == image_id)
        .ok_or_else(|| format!("no dataset row for '{image_id}'"))?;
    let kb = st.store.effective_knowledge();
    // Canonical validity: the id must literally exist in the KB.
    let canonical_valid = human_entity_id
        .as_deref()
        .map(|id| kb.entities().iter().any(|e| e.id == id))
        .unwrap_or(false);
    // PNG must really decode (bad-image exclusion with evidence).
    let png_path = ds.root().join("images").join(format!("{image_id}.png"));
    let png_ok = png_path
        .metadata()
        .map(|m| m.len() > 0)
        .unwrap_or(false)
        && std::fs::read(&png_path)
            .ok()
            .and_then(|b| image::load_from_memory(&b).ok())
            .is_some();
    let has_provenance = !row.session_id.is_empty() && row.timestamp_ms > 0;
    let (eligible, excluded) = review::evaluate_eligibility(
        png_ok,
        human_entity_id.as_deref(),
        canonical_valid,
        false,
        has_provenance,
    );
    let store = review_store(st.store.dir());
    let mut record = review::apply_human_review(
        &store,
        &image_id,
        &row.session_id,
        human_entity_id.clone(),
        correction_reason,
        human_canonical_name.clone(),
        model_prediction,
        model_confidence,
        DATASET_VERSION,
    )?;
    // Enrich with real row context the store fn cannot see, and attach the
    // computed eligibility (its internal placeholders are conservative).
    record.ocr_text = Some(row.ocr_text.clone());
    record.event_id = row.event_id.clone();
    record.is_hard_example = row.hard_example;
    record.training_eligible = eligible;
    record.excluded_reason = excluded.clone();
    store.upsert(&record).map_err(|e| e.to_string())?;

    // Dataset linkage through the standard annotate path (Setup parity):
    // confirm/correct writes the HUMAN entity; unknown flags human review.
    let mut dataset_updated = false;
    if canonical_valid {
        let _ = ds.annotate(
            &image_id, None, None, None, human_entity_id, "human", false, None,
        );
        dataset_updated = true;
    } else if record.human_entity_id.is_none() {
        let _ = ds.annotate(&image_id, None, None, None, None, "human", true, Some("human: unknown".to_string()));
        dataset_updated = true;
    }
    Ok(ReviewApplyResult { record, dataset_updated })
}

#[tauri::command]
pub fn review_skip(st: State<'_, AppState>, image_id: String) -> Result<ReviewRecord, String> {
    let ds = MlDatasetStore::new(st.store.dir().to_path_buf());
    let row = ds
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
    let store = review_store(st.store.dir());
    let rec = review::resolve_conflict(&store, &image_id, &entity_id, &reason)?;
    let ds = MlDatasetStore::new(st.store.dir().to_path_buf());
    let _ = ds.annotate(&image_id, None, None, None, Some(entity_id), "human", false, None);
    Ok(rec)
}

// ---- prioritized queue ----

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
}

#[tauri::command]
pub fn review_priority(st: State<'_, AppState>, limit: Option<usize>) -> Vec<PriorityItem> {
    let rows = MlDatasetStore::new(st.store.dir().to_path_buf()).annotations();
    let store = review_store(st.store.dir());
    let reviews: HashMap<String, review::ReviewStatus> =
        store.list().into_iter().map(|r| (r.image_id.clone(), r.review_status)).collect();
    let reviewed_sessions: HashSet<String> = store
        .list()
        .into_iter()
        .filter(|r| !matches!(r.review_status, review::ReviewStatus::Unreviewed | review::ReviewStatus::ReviewedSkipped))
        .filter_map(|r| {
            rows.iter().find(|row| row.image_id == r.image_id).map(|row| row.session_id.clone())
        })
        .collect();
    // Class sizes for new/underrepresented detection.
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut class_sessions: HashMap<&str, HashSet<&str>> = HashMap::new();
    for r in &rows {
        if let Some(e) = r.entity_id.as_deref() {
            *counts.entry(e).or_default() += 1;
            class_sessions.entry(e).or_default().insert(r.session_id.as_str());
        }
    }
    let mut items = Vec::new();
    for r in &rows {
        let prior = reviews.get(&r.image_id);
        // Parked work (reviewed or skipped) leaves the queue; conflicts stay
        // top-priority until resolved.
        if matches!(
            prior,
            Some(review::ReviewStatus::ReviewedCorrect)
                | Some(review::ReviewStatus::ReviewedCorrected)
                | Some(review::ReviewStatus::ReviewedUnknown)
                | Some(review::ReviewStatus::ReviewedSkipped)
        ) {
            continue;
        }
        let conflict_boost =
            matches!(prior, Some(review::ReviewStatus::Conflict));
        let e = r.entity_id.as_deref();
        let key = e.map(|id| {
            local_normalize(&id.split(':').nth(1).unwrap_or(id).replace('-', " "))
        });
        let ocr_disagree = match (e, key) {
            (Some(_), Some(k)) if !k.is_empty() => !local_normalize(&r.ocr_text).contains(&k as &str),
            _ => false,
        };
        let n = e.map(|id| counts.get(id).cloned().unwrap_or(0)).unwrap_or(0);
        let (mut score, reason) = review::priority_score(
            e.is_none(),
            ocr_disagree,
            r.hard_example && e.is_some(),
            e.map(|_| n < 20).unwrap_or(true),
            e.map(|id| class_sessions.get(id).map(|s| s.len() < 3).unwrap_or(true)).unwrap_or(false),
            !reviewed_sessions.contains(&r.session_id),
            r.hard_example,
        );
        let reason = if conflict_boost {
            score = score.max(200);
            format!("CONFLICT needs resolution; {reason}")
        } else {
            reason.to_string()
        };
        items.push(PriorityItem {
            image_id: r.image_id.clone(),
            session_id: r.session_id.clone(),
            timestamp_ms: r.timestamp_ms,
            ocr_text: r.ocr_text.chars().take(160).collect(),
            entity_id: e.map(|s| s.to_string()),
            review_status: reviews
                .get(&r.image_id)
                .map(|s| format!("{s:?}"))
                .unwrap_or_else(|| "UNREVIEWED".to_string()),
            score,
            reason: reason.to_string(),
        });
    }
    items.sort_by(|a, b| b.score.cmp(&a.score).then(b.timestamp_ms.cmp(&a.timestamp_ms)));
    items.into_iter().take(limit.unwrap_or(50).clamp(1, 200)).collect()
}

// ---- canonical search + drops explorer ----

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
    match canon::resolve(&kb, &query) {
        Resolution::Exact(id) => by_id
            .get(id.as_str())
            .map(|e| {
                vec![EntityHit {
                    entity_id: e.id.clone(),
                    canonical_name: e.canonical_name.clone(),
                    category: e.category.as_str().to_string(),
                    kind: "exact".to_string(),
                }]
            })
            .unwrap_or_default(),
        Resolution::Ambiguous(ids) => ids
            .iter()
            .filter_map(|id| by_id.get(id.as_str()))
            .map(|e| EntityHit {
                entity_id: e.id.clone(),
                canonical_name: e.canonical_name.clone(),
                category: e.category.as_str().to_string(),
                kind: "ambiguous-pick-one".to_string(),
            })
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
}

#[tauri::command]
pub fn drops_explorer(st: State<'_, AppState>) -> Vec<DropEntry> {
    let kb = st.store.effective_knowledge();
    let rows = MlDatasetStore::new(st.store.dir().to_path_buf()).annotations();
    let reviews = review_store(st.store.dir()).list();
    let rev_by_image: HashMap<&str, &review::ReviewRecord> =
        reviews.iter().map(|r| (r.image_id.as_str(), r)).collect();
    // Deployed fish scope for model coverage (empty when undeployed).
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
        let mut collected = 0;
        let mut sessions = HashSet::new();
        let (mut reviewed, mut eligible) = (0, 0);
        for r in rows.iter().filter(|r| r.entity_id.as_deref() == Some(c.entity_id.as_str())) {
            collected += 1;
            sessions.insert(r.session_id.as_str());
            if let Some(rev) = rev_by_image.get(r.image_id.as_str()) {
                use review::ReviewStatus::*;
                if matches!(rev.review_status, ReviewedCorrect | ReviewedCorrected | ReviewedUnknown) {
                    reviewed += 1;
                }
                if rev.training_eligible {
                    eligible += 1;
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
            model_status: if fish_scope.contains(&c.entity_id) {
                "IN_SCOPE".to_string()
            } else if c.fishing_drop {
                "OUT_OF_SCOPE".to_string()
            } else {
                "N/A".to_string()
            },
        });
    }
    out.sort_by_key(|d| std::cmp::Reverse(d.collected));
    out
}

// ---- readiness ----

#[tauri::command]
pub fn readiness_status(st: State<'_, AppState>) -> Vec<readiness::ModelReadiness> {
    let settings = st.settings.read();
    readiness_status_inner(&st.store, &settings)
}

fn readiness_status_inner(
    store: &crate::config::Store,
    settings: &crate::config::Settings,
) -> Vec<readiness::ModelReadiness> {
    let rows = MlDatasetStore::new(store.dir().to_path_buf()).annotations();
    let gates = crate::core::ml_capability::assess_capabilities(&rows);
    let gate_ready = |id: &str| gates.iter().find(|g| g.id == id).map(|g| g.ready).unwrap_or(false);
    let gate_detail = |id: &str| {
        gates.iter().find(|g| g.id == id).map(|g| g.detail.clone()).unwrap_or_default()
    };
    let reviews = review_store(store.dir()).list();
    let reviewed_ids: HashSet<&str> =
        reviews.iter().filter(|r| r.training_eligible).map(|r| r.image_id.as_str()).collect();
    let records = registry::load_registry(store.dir());
    let thresholds = readiness::ReadinessThresholds {
        min_macro_f1: settings.training.readiness_min_macro_f1,
        min_worst_class_f1: settings.training.readiness_min_worst_f1,
        min_shadow_events: settings.training.readiness_min_shadow_events,
        min_review_coverage: settings.training.readiness_min_review_coverage,
    };
    let mut out = Vec::new();
    for (family, gate_id, prefix) in [("fish", "fish_entity", "fish:"), ("state", "state", "")] {
        // Scoped rows: entity-linked rows of the family prefix (state: all
        // state rows). Reviewed-scoped: those with eligible reviews.
        let scoped: Vec<_> = rows
            .iter()
            .filter(|r| {
                if prefix.is_empty() {
                    r.game_state.map(|g| matches!(g.as_str(), "waiting_for_bite" | "bite" | "catch_result")).unwrap_or(false)
                } else {
                    r.entity_id.as_deref().map(|e| e.starts_with(prefix)).unwrap_or(false)
                }
            })
            .collect();
        let reviewed_scoped =
            scoped.iter().filter(|r| reviewed_ids.contains(r.image_id.as_str())).count();
        // Deployed version + metrics from the shadow manifest (if any).
        let stem = if family == "fish" { "fish_v1" } else { "state_v1" };
        let (deployed_version, deployed_metrics) = std::fs::read_to_string(
            store.dir().join("models").join(format!("{stem}.json")),
        )
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .map(|v| {
            let ver = v.get("version").and_then(|x| x.as_str()).and_then(|s| s.parse::<u32>().ok());
            let metrics = (
                v.get("test_accuracy").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                v.get("macro_f1").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
            );
            (ver, metrics)
        })
        .map(|(v, m)| (v, Some(m)))
        .unwrap_or((None, None));
        let soak = registry::soak_stats(store.dir(), stem, 500);
        let soak_opt = if soak.events > 0 { Some(&soak) } else { None };
        out.push(readiness::assess_family(
            family,
            gate_ready(gate_id),
            &gate_detail(gate_id),
            reviewed_scoped,
            scoped.len(),
            &records,
            deployed_version,
            deployed_metrics,
            soak_opt,
            &thresholds,
        ));
    }
    // Fruit: same shape, data gate almost surely red (honest).
    {
        let scoped: Vec<_> = rows
            .iter()
            .filter(|r| r.entity_id.as_deref().map(|e| e.starts_with("fruit:")).unwrap_or(false))
            .collect();
        let reviewed_scoped =
            scoped.iter().filter(|r| reviewed_ids.contains(r.image_id.as_str())).count();
        out.push(readiness::assess_family(
            "fruit",
            gate_ready("fruit_entity"),
            &gate_detail("fruit_entity"),
            reviewed_scoped,
            scoped.len(),
            &records,
            None,
            None,
            None,
            &thresholds,
        ));
    }
    out
}

// ---- Hermes orchestration interface (no agent framework) ----

/// Hermes task view: the deterministic trigger + readiness state an
/// external orchestrator may poll. Hermes schedules work by calling the
/// existing commands (training_start/decide/promote); it CANNOT bypass
/// gates because no bypass path exists in this codebase.
#[derive(Debug, Serialize)]
pub struct HermesTasks {
    pub triggers: Vec<training::TriggerEvent>,
    pub readiness: Vec<readiness::ModelReadiness>,
    pub history_tail: Vec<serde_json::Value>,
}

#[tauri::command]
pub fn hermes_tasks(st: State<'_, AppState>) -> HermesTasks {
    // Reuse the same tick evaluation (read-only part): triggers computed,
    // nothing spawned from here.
    let rows = MlDatasetStore::new(st.store.dir().to_path_buf()).annotations();
    let gates = assess_capabilities(&rows);
    let gate_ready = |id: &str| gates.iter().find(|g| g.id == id).map(|g| g.ready).unwrap_or(false);
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
