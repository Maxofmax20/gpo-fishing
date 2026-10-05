//! Capability-specific readiness gates + shadow-mode event schema (v5.2.0).
//!
//! The old single `ready` flag cannot express "STATE is trained but FRUIT
//! recognition has ten examples". Each capability below gets an independent
//! gate computed from dataset rows (never from predictions, never from the
//! KB alone). Gates are REPORTING ONLY: nothing here can drive the macro.
//!
//! Shadow mode (§19-20 of the v5.2.0 plan) is schema-first: the event shape,
//! the append-only log writer, and the OFF status exist now; wiring live
//! vision inference in is a separate, explicitly-approved step. There is no
//! `Vision -> Macro` path anywhere in this module.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::ml_dataset::{GameStateLabel, MlAnnotation, MlDatasetStore};

// ---- gate thresholds (documented heuristics, all reported, none hidden) ----
/// Minimum rows of one STATE class, spread over enough sessions, to call the
/// STATE capability data-ready.
pub const STATE_MIN_ROWS_PER_STATE: usize = 300;
pub const STATE_MIN_SESSIONS: usize = 4;
/// Per-entity example counts for the entity capabilities.
pub const ENTITY_MIN_EXAMPLES: usize = 20;
/// Per-entity session diversity: examples concentrated in fewer sessions
/// may share one background/camera setup and overstate coverage.
pub const ENTITY_MIN_SESSIONS: usize = 3;
/// How many entities must clear ENTITY_MIN_EXAMPLES for the capability gate.
pub const FISH_MIN_ENTITIES: usize = 10;
pub const FRUIT_MIN_ENTITIES: usize = 10;
/// How many of those must also have held-out TEST coverage.
pub const ENTITY_MIN_TEST_COVERED: usize = 5;
/// RESULT rows needed to call result-screen data collection sufficient.
pub const RESULT_UI_MIN_ROWS: usize = 500;
/// Complete WAITING->BITE->RESULT sessions in TEST for workflow readiness.
pub const WORKFLOW_MIN_TEST_SESSIONS: usize = 3;

/// One independently-gated capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityGate {
    pub id: String,
    pub name: String,
    pub ready: bool,
    pub evidence_rows: usize,
    pub evidence_sessions: usize,
    pub detail: String,
    pub blocking_requirement: Option<String>,
}

fn gate(
    id: &str,
    name: &str,
    ready: bool,
    evidence_rows: usize,
    evidence_sessions: usize,
    detail: String,
    blocking: Option<String>,
) -> CapabilityGate {
    CapabilityGate {
        id: id.to_string(),
        name: name.to_string(),
        ready,
        evidence_rows,
        evidence_sessions,
        detail,
        blocking_requirement: blocking,
    }
}

fn sessions_of(rows: &[&MlAnnotation]) -> usize {
    rows.iter().map(|r| r.session_id.as_str()).collect::<HashSet<_>>().len()
}

/// Sessions whose timestamp-ordered states contain a WAITING->BITE->RESULT
/// subsequence (a complete observable fishing workflow).
pub fn complete_workflow_sessions(rows: &[MlAnnotation]) -> HashMap<String, Vec<String>> {
    let mut by_session: HashMap<&str, Vec<&MlAnnotation>> = HashMap::new();
    for r in rows {
        by_session.entry(r.session_id.as_str()).or_default().push(r);
    }
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for (sess, mut v) in by_session {
        v.sort_by_key(|r| r.timestamp_ms);
        let mut stage = 0;
        for r in v {
            let s = r.game_state.map(|g| g.as_str()).unwrap_or("");
            if stage == 0 && s == "waiting_for_bite" {
                stage = 1;
            } else if stage == 1 && s == "bite" {
                stage = 2;
            } else if stage == 2 && s == "catch_result" {
                stage = 3;
                break;
            }
        }
        if stage == 3 {
            let split = MlDatasetStore::split_of(sess);
            out.entry(split.as_str().to_string()).or_default().push(sess.to_string());
        }
    }
    out
}

/// Assess all eight capability gates from real rows. Pure function.
pub fn assess_capabilities(rows: &[MlAnnotation]) -> Vec<CapabilityGate> {
    // STATE: per-class rows + sessions.
    let mut state_counts: HashMap<&str, Vec<&MlAnnotation>> = HashMap::new();
    for r in rows {
        if let Some(g) = r.game_state {
            let s = g.as_str();
            if matches!(s, "waiting_for_bite" | "bite" | "catch_result") {
                state_counts.entry(s).or_default().push(r);
            }
        }
    }
    let state_ok = ["waiting_for_bite", "bite", "catch_result"].iter().all(|s| {
        state_counts
            .get(*s)
            .map(|v| v.len() >= STATE_MIN_ROWS_PER_STATE && sessions_of(v) >= STATE_MIN_SESSIONS)
            .unwrap_or(false)
    });
    let state_rows: usize = state_counts.values().map(|v| v.len()).sum();
    let state_detail = ["waiting_for_bite", "bite", "catch_result"]
        .iter()
        .map(|s| {
            let v = state_counts.get(*s);
            format!("{}:{}/{}sess", s, v.map(|x| x.len()).unwrap_or(0), v.map(|x| sessions_of(x)).unwrap_or(0))
        })
        .collect::<Vec<_>>()
        .join(" ");

    // ACTION UI: frames tagged by the v5.3.0+ collector hooks (unreviewed).
    let action_region_rows: Vec<&MlAnnotation> = rows
        .iter()
        .filter(|r| matches!(r.region_name.as_str(), "store_banner" | "drop_banner" | "confirm_banner"))
        .collect();

    // ENTITY: per-prefix coverage.
    let mut ent_counts: HashMap<&str, Vec<&MlAnnotation>> = HashMap::new();
    for r in rows {
        if let Some(e) = r.entity_id.as_deref() {
            ent_counts.entry(e).or_default().push(r);
        }
    }
    // Heuristic port of fruit::normalize for OCR/entity agreement: lowercase,
    // non-alphanumeric to space, collapse whitespace. Agreement = normalized
    // OCR contains the entity key (e.g. "golden", "you got").
    fn normalize_ocr(s: &str) -> String {
        s.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { ' ' })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
    fn entity_key(entity_id: &str) -> String {
        normalize_ocr(&entity_id.split(':').nth(1).unwrap_or(entity_id).replace('-', " "))
    }
    struct Qualified<'a> {
        entity: &'a str,
        examples: usize,
        sessions: usize,
        test_covered: bool,
        ocr_agree: usize,
        ocr_disagree: usize,
    }
    let qualified = |prefix: &str| -> Vec<Qualified<'_>> {
        let mut v: Vec<Qualified> = ent_counts
            .iter()
            .filter(|(e, _)| e.starts_with(prefix))
            .map(|(e, rows)| {
                let test = rows.iter().any(|r| MlDatasetStore::split_of(&r.session_id).as_str() == "test");
                let sessions = rows.iter().map(|r| r.session_id.as_str()).collect::<HashSet<_>>().len();
                let key = entity_key(e);
                let mut agree = 0;
                let mut disagree = 0;
                for r in rows.iter() {
                    if normalize_ocr(&r.ocr_text).contains(&key as &str) {
                        agree += 1;
                    } else {
                        disagree += 1;
                    }
                }
                Qualified { entity: *e, examples: rows.len(), sessions, test_covered: test, ocr_agree: agree, ocr_disagree: disagree }
            })
            .filter(|q| q.examples >= ENTITY_MIN_EXAMPLES && q.sessions >= ENTITY_MIN_SESSIONS)
            .collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.examples));
        v
    };
    let fish_q = qualified("fish:");
    let fruit_q = qualified("fruit:");
    let fish_test = fish_q.iter().filter(|q| q.test_covered).count();
    let fruit_test = fruit_q.iter().filter(|q| q.test_covered).count();
    let fish_ready = fish_q.len() >= FISH_MIN_ENTITIES && fish_test >= ENTITY_MIN_TEST_COVERED;
    let fruit_ready = fruit_q.len() >= FRUIT_MIN_ENTITIES && fruit_test >= ENTITY_MIN_TEST_COVERED;
    let qual_detail = |q: &[Qualified]| {
        q.iter()
            .map(|x| format!("{}(n={} sess={} ocr±={}/{}{})", x.entity, x.examples, x.sessions, x.ocr_agree, x.ocr_disagree, if x.test_covered { " T" } else { "" }))
            .collect::<Vec<_>>()
            .join(", ")
    };

    // RESULT UI: screen-level rows exist.
    let result_rows: Vec<&MlAnnotation> = rows
        .iter()
        .filter(|r| r.game_state == Some(GameStateLabel::CatchResult))
        .collect();
    let result_ready = result_rows.len() >= RESULT_UI_MIN_ROWS;

    // WORKFLOW: complete sessions per split.
    let workflows = complete_workflow_sessions(rows);
    let wf_test = workflows.get("test").map(|v| v.len()).unwrap_or(0);
    let wf_total: usize = workflows.values().map(|v| v.len()).sum();
    let wf_ready = wf_test >= WORKFLOW_MIN_TEST_SESSIONS;

    let mut gates = vec![
        gate(
            "state",
            "STATE (WAITING/BITE/RESULT)",
            state_ok,
            state_rows,
            state_counts.values().flatten().map(|r| r.session_id.as_str()).collect::<HashSet<_>>().len(),
            format!("per-class rows/sessions: {state_detail}"),
            (!state_ok).then(|| {
                format!("need >= {STATE_MIN_ROWS_PER_STATE} rows over >= {STATE_MIN_SESSIONS} sessions for EACH of waiting_for_bite/bite/catch_result")
            }),
        ),
        gate(
            "fish_entity",
            "FISH ENTITY (per-fish >=20 examples, >=3 sessions, TEST covered)",
            fish_ready,
            fish_q.iter().map(|q| q.examples).sum(),
            fish_q.iter().map(|q| q.sessions).sum(),
            format!("qualified fish {}/{} (test-covered {}/{}): {}",
                fish_q.len(), FISH_MIN_ENTITIES, fish_test, ENTITY_MIN_TEST_COVERED,
                qual_detail(&fish_q)),
            (!fish_ready).then(|| {
                format!("need >= {FISH_MIN_ENTITIES} fish with >= {ENTITY_MIN_EXAMPLES} examples over >= {ENTITY_MIN_SESSIONS} sessions and >= {ENTITY_MIN_TEST_COVERED} with TEST coverage; collect gameplay, never synthesize")
            }),
        ),
        gate(
            "fruit_entity",
            "DEVIL FRUIT ENTITY (per-fruit >=20 examples, >=3 sessions, TEST covered)",
            fruit_ready,
            fruit_q.iter().map(|q| q.examples).sum(),
            fruit_q.iter().map(|q| q.sessions).sum(),
            format!("qualified fruit {}/{} (test-covered {}/{}): {}",
                fruit_q.len(), FRUIT_MIN_ENTITIES, fruit_test, ENTITY_MIN_TEST_COVERED,
                qual_detail(&fruit_q)),
            (!fruit_ready).then(|| {
                format!("need >= {FRUIT_MIN_ENTITIES} fruits with >= {ENTITY_MIN_EXAMPLES} examples over >= {ENTITY_MIN_SESSIONS} sessions and >= {ENTITY_MIN_TEST_COVERED} with TEST coverage; only real catches count")
            }),
        ),
        gate(
            "other_drop",
            "OTHER DROP (non-fish, non-fruit results)",
            false,
            0,
            0,
            "no OTHER_DROP category exists in the KB and no dataset entity is unmapped to the KB".to_string(),
            Some("define the category from real game drops first, then collect tagged examples; Wiki entries do not count".to_string()),
        ),
        gate(
            "result_ui",
            "RESULT UI (result-screen recognition data)",
            result_ready,
            result_rows.len(),
            sessions_of(&result_rows),
            format!("catch_result rows: {}", result_rows.len()),
            (!result_ready).then(|| format!("need >= {RESULT_UI_MIN_ROWS} RESULT rows")),
        ),
        gate(
            "action_ui",
            "ACTION UI (DROP/STORE screens)",
            false,
            action_region_rows.len(),
            action_region_rows.iter().map(|r| r.session_id.as_str()).collect::<HashSet<_>>().len(),
            format!("{} action-region frames (store_banner/drop_banner/confirm_banner), all unreviewed; zero reviewed confirmation pairs",
                action_region_rows.len()),
            Some("review action-region frames + log CONFIRMED pairs from real store/drop flows before claiming verification".to_string()),
        ),
        gate(
            "confirmation",
            "CONFIRMATION (action verification pairs)",
            false,
            0,
            0,
            "no command-sent/game-confirmed pairs are logged anywhere".to_string(),
            Some("log COMMAND SENT + GAME CONFIRMED pairs from real store/drop flows before claiming verification".to_string()),
        ),
        gate(
            "workflow",
            "COMPLETE WORKFLOW (WAITING->BITE->RESULT sessions, TEST held-out)",
            wf_ready,
            wf_total,
            wf_total,
            format!("complete sessions train/val/test: {}/{}/{}",
                workflows.get("train").map(|v| v.len()).unwrap_or(0),
                workflows.get("validation").map(|v| v.len()).unwrap_or(0),
                wf_test),
            (!wf_ready).then(|| format!("need >= {WORKFLOW_MIN_TEST_SESSIONS} complete sessions in TEST; freeze them as the sequence test set")),
        ),
    ];
    // evidence_sessions for entity gates: distinct sessions across qualified entities.
    for g in gates.iter_mut().filter(|g| g.id == "fish_entity" || g.id == "fruit_entity") {
        let prefix = if g.id == "fish_entity" { "fish:" } else { "fruit:" };
        let sess: HashSet<&str> = ent_counts
            .iter()
            .filter(|(e, _)| e.starts_with(prefix))
            .flat_map(|(_, v)| v.iter().map(|r| r.session_id.as_str()))
            .collect();
        g.evidence_sessions = sess.len();
    }
    gates
}

// ---- shadow mode: event schema + log writer + live status ----
// (inference itself lives in shadow_infer.rs; status reflects it).

/// One shadow-mode comparison event (§20). Every field but the ids is
/// optional: UNKNOWN is a valid result, never a guessed one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowEvent {
    pub session_id: String,
    pub event_id: String,
    pub timestamp_ms: u64,
    pub vision_state: Option<String>,
    pub production_state: Option<String>,
    pub state_confidence: Option<f32>,
    pub result_category: Option<String>,
    pub entity: Option<String>,
    pub ocr_text: Option<String>,
    pub normalized_entity: Option<String>,
    pub policy_recommendation: Option<String>,
    pub would_be_action: Option<String>,
    pub actual_action: Option<String>,
    pub confirmation: Option<String>,
    pub latency_ms: Option<u64>,
    pub agreement: Option<bool>,
    /// Vision classifier confidence for non-state predictions (e.g. fish
    /// top-1 probability). Informational: no fish rejection threshold was
    /// established, so policy ignores it. Additive (default None) so old
    /// log lines keep parsing.
    #[serde(default)]
    pub vision_confidence: Option<f32>,
}

pub fn shadow_log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("shadow_events.jsonl")
}

/// Append one shadow event (JSONL, never rewrites history).
pub fn append_shadow_event(data_dir: &Path, event: &ShadowEvent) -> Result<(), String> {
    let mut line = serde_json::to_string(event).map_err(|e| e.to_string())?;
    line.push('\n');
    std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(shadow_log_path(data_dir))
        .and_then(|mut f| f.write_all(line.as_bytes()))
        .map_err(|e| e.to_string())
}

/// Shadow status: ON only when the operator flag is set AND at least one
/// verified shadow model is loaded. Observation + telemetry either way;
/// macro control is unconditionally OFF (no path exists).
pub fn shadow_status(
    data_dir: &Path,
    flag_enabled: bool,
    models: &[(String, Option<f32>)],
) -> serde_json::Value {
    let logged = std::fs::read_to_string(shadow_log_path(data_dir))
        .map(|c| c.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    let enabled = flag_enabled && !models.is_empty();
    let reason = if enabled {
        "observation + telemetry only; macro control OFF".to_string()
    } else if models.is_empty() {
        "no verified shadow model in models/ (seeded from bundled copies at startup)".to_string()
    } else {
        "operator flag features.ml_shadow is off".to_string()
    };
    serde_json::json!({
        "enabled": enabled,
        "mode": if enabled { "ON" } else { "OFF" },
        "reason": reason,
        "models": models.iter().map(|(n, a)| serde_json::json!({"name": n, "test_accuracy": a})).collect::<Vec<_>>(),
        "vision_to_macro": "FORBIDDEN (no path exists)",
        "production_control": "OFF",
        "events_logged": logged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ml_dataset::{DATASET_VERSION, MlTask, UiLabel};

    fn row(session: &str, ts: u64, state: Option<GameStateLabel>, entity: Option<&str>) -> MlAnnotation {
        MlAnnotation {
            image_id: format!("{session}-{ts}"),
            dataset_version: DATASET_VERSION,
            session_id: session.into(),
            task: MlTask::GameState,
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
            hard_example: false,
            hard_reason: None,
            corrections: vec![],
            event_id: None,
            frame_index: None,
        }
    }

    #[test]
    fn state_gate_needs_all_three_classes() {
        use GameStateLabel::*;
        let mut rows = vec![];
        for i in 0..(STATE_MIN_ROWS_PER_STATE as u64) {
            rows.push(row(&format!("s-w{i}"), i, Some(WaitingForBite), None));
            rows.push(row(&format!("s-b{i}"), i, Some(Bite), None));
        }
        let gates = assess_capabilities(&rows);
        let state = gates.iter().find(|g| g.id == "state").unwrap();
        assert!(!state.ready, "missing catch_result must block");
        assert!(state.blocking_requirement.is_some());
    }

    #[test]
    fn workflow_gate_counts_complete_chains_only() {
        use GameStateLabel::*;
        let mut rows = vec![
            row("full-1", 1, Some(WaitingForBite), None),
            row("full-1", 2, Some(Bite), None),
            row("full-1", 3, Some(CatchResult), Some("fish:golden")),
            row("partial-1", 1, Some(WaitingForBite), None),
            row("partial-1", 2, Some(CatchResult), None),
        ];
        // find a test-split session deterministically
        let mut test_sess = None;
        for i in 0..2000 {
            let cand = format!("wf-test-{i:04}");
            if MlDatasetStore::split_of(&cand).as_str() == "test" {
                test_sess = Some(cand);
                break;
            }
        }
        let ts = test_sess.unwrap();
        rows.push(row(&ts, 1, Some(WaitingForBite), None));
        rows.push(row(&ts, 2, Some(Bite), None));
        rows.push(row(&ts, 3, Some(CatchResult), None));
        let wf = complete_workflow_sessions(&rows);
        assert_eq!(wf.get("test").map(|v| v.len()), Some(1));
        assert!(wf.values().flatten().any(|s| s == "full-1"));
        assert!(!wf.values().flatten().any(|s| s == "partial-1"), "bite-less chain is not complete");
        let gates = assess_capabilities(&rows);
        let g = gates.iter().find(|g| g.id == "workflow").unwrap();
        assert!(!g.ready, "1 test session < 3 required");
    }

    #[test]
    fn other_drop_action_confirmation_gates_stay_red() {
        let gates = assess_capabilities(&[]);
        for id in ["other_drop", "action_ui", "confirmation"] {
            let g = gates.iter().find(|g| g.id == id).unwrap();
            assert!(!g.ready, "{id} must stay red with no evidence");
            assert!(g.blocking_requirement.is_some(), "{id} must name its blocker");
        }
        assert_eq!(gates.len(), 8);
    }

    #[test]
    fn legacy_rows_without_event_fields_still_parse() {
        let raw = r#"{"image_id":"x","dataset_version":1,"session_id":"s","task":"game_state","ocr_text":"","region_name":"bar","ui_label":"fishing_bar","bbox":null,"game_state":"bite","entity_id":null,"annotator":"c","timestamp_ms":5,"source":"gameplay","confidence":null,"hard_example":false,"hard_reason":null,"corrections":[]}"#;
        let ann: MlAnnotation = serde_json::from_str(raw).unwrap();
        assert_eq!(ann.event_id, None);
        assert_eq!(ann.frame_index, None);
        assert_eq!(ann.game_state, Some(GameStateLabel::Bite));
    }

    #[test]
    fn set_event_first_capture_wins() {
        let dir = std::env::temp_dir().join("gpo-cap-ev");
        let _ = std::fs::remove_dir_all(&dir);
        let ds = MlDatasetStore::new(dir.clone());
        ds.init().unwrap();
        // distinct tiny pngs
        let mut rgba = vec![0u8; 16 * 16 * 4];
        rgba[0] = 7;
        let id = ds.import_png(&rgba_to_png(&rgba), "sess-e", MlTask::GameState, Some(GameStateLabel::Bite), "", "bar", "t").unwrap();
        assert!(ds.set_event(&id, "sess-e#f000003", 3).unwrap());
        assert!(!ds.set_event(&id, "sess-e#f000009", 9).unwrap(), "first capture wins");
        let back = ds.annotations().into_iter().find(|r| r.image_id == id).unwrap();
        assert_eq!(back.event_id.as_deref(), Some("sess-e#f000003"));
        assert_eq!(back.frame_index, Some(3));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn rgba_to_png(rgba: &[u8]) -> Vec<u8> {
        // minimal valid PNG via the image crate (same pattern as commands.rs)
        let mut buf = Vec::new();
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new(&mut buf)
            .write_image(rgba, 16, 16, image::ExtendedColorType::Rgba8)
            .unwrap();
        buf
    }

    #[test]
    fn shadow_event_roundtrips_and_stays_off() {
        let dir = std::env::temp_dir().join("gpo-cap-shadow");
        let _ = std::fs::remove_dir_all(&dir);
        let ev = ShadowEvent {
            session_id: "s".into(),
            event_id: "s#f000001".into(),
            timestamp_ms: 42,
            vision_state: None,
            production_state: Some("bite".into()),
            state_confidence: None,
            result_category: None,
            entity: None,
            ocr_text: None,
            normalized_entity: None,
            policy_recommendation: None,
            would_be_action: None,
            actual_action: Some("reel".into()),
            confirmation: Some("UNKNOWN".into()),
            latency_ms: None,
            agreement: None,
            vision_confidence: None,
        };
        append_shadow_event(&dir, &ev).unwrap();
        let st = shadow_status(&dir, false, &[]);
        assert_eq!(st["enabled"], false);
        assert_eq!(st["events_logged"], 1);
        assert_eq!(st["production_control"], "OFF");
        // Flag on but no models: still OFF (nothing to observe with).
        let st2 = shadow_status(&dir, true, &[]);
        assert_eq!(st2["enabled"], false);
        // Flag on + a model: ON, control still OFF.
        let st3 = shadow_status(&dir, true, &[("fish_v1".to_string(), Some(0.58))]);
        assert_eq!(st3["enabled"], true);
        assert_eq!(st3["mode"], "ON");
        assert_eq!(st3["production_control"], "OFF");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
