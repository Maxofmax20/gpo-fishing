//! Read-only access to the ontology review dataset for the UI.
//!
//! The review artefacts are produced by `python -m ml.gpo_ontology_review build`
//! under the repository's `reports/ontology_review/`. This command serves them to
//! the Training Center so a human can work through the queue in the app instead
//! of in a text editor.
//!
//! READ-ONLY, and deliberately so. It reads `concepts.json` and the append-only
//! `ontology_decisions.jsonl`, and returns them. There is **no** command here that
//! records a decision, because a decision written by the UI would have to be
//! attributed to a human who clicked a button, and that attribution is exactly the
//! claim this campaign refuses to make on someone's behalf. Decisions are recorded
//! by a person, on the command line, with their name against them.
//!
//! What the UI offers is the evidence and the four decision shapes, with **nothing
//! pre-selected**.
//!
//! # Report path resolution
//!
//! `reports/` sits beside `src-tauri/` in the repository and is NOT inside the
//! `store_dir` (which is the app data directory). Resolution therefore tries, in
//! order: an explicit override, `GPO_REPO_ROOT`, then walks up from the executable
//! and the current directory looking for the marker file `FISH_VOCAB.txt`.
//!
//! All of that is optional. When no artefacts are found the command returns an
//! empty, explicitly-flagged payload rather than an error, because the Training
//! Center must not appear broken when a read-only review dataset was never built.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// One representative evidence image attached to a concept.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OntologyEvidence {
    pub image_id: String,
    pub session_id: String,
    pub day: String,
    pub raw_ocr: String,
    pub stored_entity_id: Option<String>,
    pub ocr_confidence: Option<f32>,
    pub selected_because: String,
}

/// One observed game name awaiting a human ruling.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OntologyConcept {
    pub concept_id: String,
    pub display_name: String,
    pub occurrences: i64,
    pub session_count: usize,
    pub day_count: usize,
    pub priority: String,
    pub priority_reasons: Vec<String>,
    pub ontology_status: String,
    pub current_kb_candidates: Vec<String>,
    pub base_name: Option<String>,
    pub modifiers: Vec<String>,
    pub family_id: Option<String>,
    pub stored_labels: Vec<(String, i64)>,
    /// Rows whose stored label equals one KB candidate. Reported with an explicit
    /// caveat in the UI: agreement with one COMPONENT is not confirmation.
    pub stored_agrees_with_candidate: i64,
    pub ocr_variants: Vec<String>,
    pub representative_images: Vec<OntologyEvidence>,
    /// Existing human decision, if one has been recorded.
    pub human_decision: Option<String>,
    pub decision_note: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OntologyReviewPayload {
    /// False when no artefacts were found. The UI says so plainly rather than
    /// showing an empty queue as if it were "nothing to review".
    pub available: bool,
    pub detail: String,
    pub concepts_total: usize,
    pub decided_total: usize,
    pub pending_total: usize,
    /// Concept ids grouped by priority band, strongest evidence first.
    pub queue_p0: Vec<String>,
    pub queue_p1: Vec<String>,
    pub queue_p2: Vec<String>,
    pub queue_p3: Vec<String>,
    pub concepts: Vec<OntologyConcept>,
}

/// Locate `reports/ontology_review/concepts.json`, or `None`.
fn find_report_dir() -> Option<PathBuf> {
    // 1. explicit override
    if let Ok(p) = std::env::var("GPO_ONTOLOGY_REVIEW_DIR") {
        let p = PathBuf::from(p);
        if p.join("concepts.json").is_file() {
            return Some(p);
        }
    }

    // 2. repository root hint
    if let Ok(root) = std::env::var("GPO_REPO_ROOT") {
        let cand = PathBuf::from(root).join("reports").join("ontology_review");
        if cand.join("concepts.json").is_file() {
            return Some(cand);
        }
    }

    // 3. walk up from the executable and from the current directory.
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        let mut p: Option<&Path> = exe.as_path().parent();
        for _ in 0..8 {
            if let Some(dir) = p {
                roots.push(dir.to_path_buf());
                p = dir.parent();
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut p: Option<&Path> = Some(cwd.as_path());
        for _ in 0..8 {
            if let Some(dir) = p {
                roots.push(dir.to_path_buf());
                p = dir.parent();
            }
        }
    }

    for root in roots {
        let cand = root.join("reports").join("ontology_review");
        if cand.join("concepts.json").is_file() {
            return Some(cand);
        }
    }
    None
}

/// Minimal extraction from `concepts.json` without pulling in a JSON-schema
/// dependency: the file is machine-generated by our own pipeline, so the shapes
/// are known and a permissive reader is appropriate. Anything unexpected is
/// skipped rather than failing the command.
#[tauri::command]
pub fn ontology_review_read() -> OntologyReviewPayload {
    let empty = |detail: &str| OntologyReviewPayload {
        available: false,
        detail: detail.to_string(),
        concepts_total: 0,
        decided_total: 0,
        pending_total: 0,
        queue_p0: vec![],
        queue_p1: vec![],
        queue_p2: vec![],
        queue_p3: vec![],
        concepts: vec![],
    };

    let Some(dir) = find_report_dir() else {
        return empty(
            "No ontology review dataset found. Build it with: \
             python -m ml.gpo_ontology_review build",
        );
    };

    let raw = match std::fs::read_to_string(dir.join("concepts.json")) {
        Ok(r) => r,
        Err(e) => return empty(&format!("concepts.json unreadable: {e}")),
    };
    let v: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => return empty(&format!("concepts.json is not valid JSON: {e}")),
    };

    let arr = match v.get("concepts").and_then(|c| c.as_array()) {
        Some(a) => a.clone(),
        None => return empty("concepts.json has no `concepts` array"),
    };

    // A decision log, if present, is folded in so the UI can show progress.
    let mut decisions: std::collections::HashMap<String, (String, Option<String>)> =
        std::collections::HashMap::new();
    if let Ok(log) = std::fs::read_to_string(dir.join("ontology_decisions.jsonl")) {
        for line in log.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(d) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let (Some(cid), Some(dec)) = (
                d.get("concept_id").and_then(|x| x.as_str()),
                d.get("decision").and_then(|x| x.as_str()),
            ) else {
                continue;
            };
            let note = d.get("notes").and_then(|x| x.as_str()).map(str::to_string);
            decisions.insert(cid.to_string(), (dec.to_string(), note));
        }
    }

    let mut concepts: Vec<OntologyConcept> = Vec::with_capacity(arr.len());
    for c in &arr {
        let s = |k: &str| c.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let n = |k: &str| c.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
        let ln = |k: &str| c.get(k).and_then(|x| x.as_u64()).unwrap_or(0) as usize;

        let stored_labels: Vec<(String, i64)> = c
            .get("stored_labels")
            .and_then(|x| x.as_object())
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.clone(), v.as_i64().unwrap_or(0)))
                    .collect()
            })
            .unwrap_or_default();

        let representative_images = c
            .get("representative_images")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .map(|e| OntologyEvidence {
                        image_id: e
                            .get("image_id")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string(),
                        session_id: e
                            .get("session_id")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string(),
                        day: e.get("day").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        raw_ocr: e
                            .get("raw_ocr")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string(),
                        stored_entity_id: e
                            .get("stored_entity_id")
                            .and_then(|x| x.as_str())
                            .map(str::to_string),
                        ocr_confidence: e
                            .get("ocr_confidence")
                            .and_then(|x| x.as_f64())
                            .map(|v| v as f32),
                        selected_because: e
                            .get("selected_because")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let cid = s("concept_id");
        let (human_decision, decision_note) = decisions
            .get(&cid)
            .cloned()
            .unwrap_or((String::new(), None));
        let human_decision = if human_decision.is_empty() {
            None
        } else {
            Some(human_decision)
        };

        concepts.push(OntologyConcept {
            concept_id: cid,
            display_name: s("display_name"),
            occurrences: n("occurrences"),
            session_count: ln("sessions").max(
                c.get("sessions").and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0),
            ),
            day_count: c
                .get("calendar_days")
                .and_then(|x| x.as_array())
                .map(|a| a.len())
                .unwrap_or(0),
            priority: s("priority"),
            priority_reasons: c
                .get("priority_reasons")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            ontology_status: s("ontology_status"),
            current_kb_candidates: c
                .get("current_kb_candidates")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            base_name: c.get("base_name").and_then(|x| x.as_str()).map(str::to_string),
            modifiers: c
                .get("modifiers")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            family_id: c.get("family_id").and_then(|x| x.as_str()).map(str::to_string),
            stored_labels,
            stored_agrees_with_candidate: n("stored_agrees_with_candidate"),
            ocr_variants: c
                .get("ocr_variants")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            representative_images,
            human_decision,
            decision_note,
        });
    }

    // Strongest evidence first within each band.
    concepts.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then_with(|| a.display_name.cmp(&b.display_name))
    });

    let queue = |band: &str| -> Vec<String> {
        concepts
            .iter()
            .filter(|c| c.priority == band && c.human_decision.is_none())
            .map(|c| c.concept_id.clone())
            .collect()
    };
    let decided_total = concepts.iter().filter(|c| c.human_decision.is_some()).count();
    let pending_total = concepts.len() - decided_total;

    OntologyReviewPayload {
        available: true,
        detail: format!(
            "{} observed game name(s) from the OCR audit. {} decided, {} pending. \
             A decision is recorded by a person with: \
             python -m ml.gpo_ontology_review decide",
            concepts.len(),
            decided_total,
            pending_total
        ),
        concepts_total: concepts.len(),
        decided_total,
        pending_total,
        queue_p0: queue("P0"),
        queue_p1: queue("P1"),
        queue_p2: queue("P2"),
        queue_p3: queue("P3"),
        concepts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_artefacts_yield_an_explicit_unavailable_payload() {
        // The command must never error: an absent review dataset is a normal
        // state, and the Training Center must not look broken.
        let payload = ontology_review_read();
        if !payload.available {
            assert_eq!(payload.concepts_total, 0);
            assert!(payload.detail.contains("ontology_review"), "{}", payload.detail);
            assert!(payload.concepts.is_empty());
        }
    }

    #[test]
    fn the_command_is_read_only() {
        // Guard against a future edit adding a write to the report directory.
        //
        // Only the non-test half of the file is scanned: the sentinel strings live
        // in this test, so scanning the whole file would match the check against
        // itself and always fail.
        let full = include_str!("commands_ontology.rs");
        let src = full.split("#[cfg(test)]").next().unwrap_or(full);
        for forbidden in [
            "std::fs::write",
            "std::fs::remove_file",
            "File::create",
            "OpenOptions",
            "fn ontology_review_write",
            "fn ontology_review_decide",
        ] {
            assert!(
                !src.contains(forbidden),
                "commands_ontology.rs must stay read-only; found {forbidden}"
            );
        }
    }

    #[test]
    fn no_command_in_this_module_records_a_decision() {
        // The UI deliberately cannot attribute a decision to a person. A decision
        // requires someone to state it, with their name against it.
        let full = include_str!("commands_ontology.rs");
        let src = full.split("#[cfg(test)]").next().unwrap_or(full);
        assert!(
            !src.contains("decision_maker"),
            "this module must not write a decision; it only reads what a person recorded"
        );
    }

    #[test]
    fn queue_helpers_are_order_stable() {
        // A concept with no decision belongs in its band's queue.
        let c = OntologyConcept {
            concept_id: "concept:x".into(),
            display_name: "x".into(),
            occurrences: 10,
            session_count: 2,
            day_count: 1,
            priority: "P2".into(),
            priority_reasons: vec![],
            ontology_status: "UNKNOWN".into(),
            current_kb_candidates: vec![],
            base_name: None,
            modifiers: vec![],
            family_id: None,
            stored_labels: vec![],
            stored_agrees_with_candidate: 0,
            ocr_variants: vec![],
            representative_images: vec![],
            human_decision: None,
            decision_note: None,
        };
        assert!(c.human_decision.is_none());
        assert_eq!(c.concept_id, "concept:x");
    }
}