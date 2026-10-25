//! Workflow tracking: RESULT -> ENTITY -> POLICY -> ACTION -> CONFIRMATION
//! (§12, §14-15 of v5.3.0).
//!
//! - `ActionEvent` is the additive, backwards-compatible event schema (§12):
//!   every field except ids/workflow is optional; old data keeps parsing.
//! - `WorkflowTracker` is a pure in-memory state machine (no I/O, no macro
//!   control) correlating one workflow's events by `workflow_id`.
//! - `append_action_event` / `summarize_action_log` persist and report the
//!   event stream (`action_events.jsonl`, append-only).
//!
//! Confirmation vocabulary: CONFIRMED / FAILED / TIMEOUT / UNKNOWN.
//! Missing confirmation is NEVER success: see `finalize()`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Game-observed confirmation of a sent action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConfirmationState {
    Pending,
    Confirmed,
    Failed,
    Timeout,
    Unknown,
}

impl ConfirmationState {
    pub fn as_str(self) -> &'static str {
        match self {
            ConfirmationState::Pending => "PENDING",
            ConfirmationState::Confirmed => "CONFIRMED",
            ConfirmationState::Failed => "FAILED",
            ConfirmationState::Timeout => "TIMEOUT",
            ConfirmationState::Unknown => "UNKNOWN",
        }
    }
}

/// One correlated action/confirmation event (§12 schema).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionEvent {
    pub workflow_id: String,
    pub event_id: String,
    #[serde(default)]
    pub frame_index: Option<u64>,
    pub session_id: String,
    #[serde(default)]
    pub result_event_id: Option<String>,
    #[serde(default)]
    pub entity_id: Option<String>,
    #[serde(default)]
    pub entity_type: Option<String>,
    #[serde(default)]
    pub policy_decision: Option<String>,
    #[serde(default)]
    pub action_requested: Option<String>,
    #[serde(default)]
    pub action_sent_at: Option<u64>,
    pub confirmation_state: ConfirmationState,
    #[serde(default)]
    pub confirmation_evidence: Option<String>,
    #[serde(default)]
    pub confirmation_at: Option<u64>,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub final_outcome: Option<String>,
}

/// Workflow lifecycle states (§14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkflowState {
    Waiting,
    Bite,
    ResultPending,
    EntityPending,
    EntityConfirmed,
    PolicyPending,
    ActionPending,
    ConfirmationPending,
    Confirmed,
    Failed,
    Unknown,
    Recovery,
}

impl WorkflowState {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkflowState::Waiting => "WAITING",
            WorkflowState::Bite => "BITE",
            WorkflowState::ResultPending => "RESULT_PENDING",
            WorkflowState::EntityPending => "ENTITY_PENDING",
            WorkflowState::EntityConfirmed => "ENTITY_CONFIRMED",
            WorkflowState::PolicyPending => "POLICY_PENDING",
            WorkflowState::ActionPending => "ACTION_PENDING",
            WorkflowState::ConfirmationPending => "CONFIRMATION_PENDING",
            WorkflowState::Confirmed => "CONFIRMED",
            WorkflowState::Failed => "FAILED",
            WorkflowState::Unknown => "UNKNOWN",
            WorkflowState::Recovery => "RECOVERY",
        }
    }

    pub fn terminal(self) -> bool {
        matches!(self, WorkflowState::Confirmed | WorkflowState::Failed | WorkflowState::Unknown)
    }
}

/// Pure tracker: feed correlated events, read the workflow state.
/// Observation-only — advancing the tracker never sends an action.
#[derive(Debug, Clone)]
pub struct WorkflowTracker {
    pub workflow_id: String,
    pub session_id: String,
    pub state: WorkflowState,
    pub events: Vec<ActionEvent>,
}

impl WorkflowTracker {
    pub fn new(workflow_id: &str, session_id: &str) -> Self {
        Self {
            workflow_id: workflow_id.to_string(),
            session_id: session_id.to_string(),
            state: WorkflowState::Waiting,
            events: Vec::new(),
        }
    }

    /// Record one event and advance. Returns the new state.
    pub fn observe(&mut self, event: ActionEvent) -> WorkflowState {
        debug_assert_eq!(event.workflow_id, self.workflow_id, "cross-workflow event");
        let next = match self.state {
            WorkflowState::Waiting => {
                if event.result_event_id.is_some() { WorkflowState::ResultPending } else { WorkflowState::Waiting }
            }
            WorkflowState::Bite => {
                if event.result_event_id.is_some() { WorkflowState::ResultPending } else { WorkflowState::Bite }
            }
            WorkflowState::ResultPending => {
                if event.entity_id.is_some() { WorkflowState::EntityConfirmed } else { WorkflowState::EntityPending }
            }
            WorkflowState::EntityPending => {
                if event.entity_id.is_some() { WorkflowState::EntityConfirmed } else { WorkflowState::EntityPending }
            }
            WorkflowState::EntityConfirmed => {
                if event.policy_decision.is_some() { WorkflowState::PolicyPending } else { WorkflowState::EntityConfirmed }
            }
            WorkflowState::PolicyPending => {
                if event.action_sent_at.is_some() { WorkflowState::ConfirmationPending } else { WorkflowState::ActionPending }
            }
            WorkflowState::ActionPending => {
                if event.action_sent_at.is_some() { WorkflowState::ConfirmationPending } else { WorkflowState::ActionPending }
            }
            WorkflowState::ConfirmationPending => match event.confirmation_state {
                ConfirmationState::Confirmed => WorkflowState::Confirmed,
                ConfirmationState::Failed | ConfirmationState::Timeout => WorkflowState::Failed,
                ConfirmationState::Unknown => {
                    if event.retry_count > 0 { WorkflowState::Recovery } else { WorkflowState::Unknown }
                }
                ConfirmationState::Pending => WorkflowState::ConfirmationPending,
            },
            // Terminal states absorb further events without moving.
            WorkflowState::Confirmed | WorkflowState::Failed | WorkflowState::Unknown => self.state,
            WorkflowState::Recovery => match event.confirmation_state {
                ConfirmationState::Confirmed => WorkflowState::Confirmed,
                ConfirmationState::Failed | ConfirmationState::Timeout => WorkflowState::Failed,
                _ => WorkflowState::Recovery,
            },
        };
        // Bite is a macro-level observation, not an action event: allow an
        // explicit nudge via result-less events is unnecessary; keep Waiting
        // until RESULT evidence arrives.
        self.state = next;
        self.events.push(event);
        next
    }

    /// Bite observed on the production path (no event payload needed).
    pub fn observe_bite(&mut self) {
        if self.state == WorkflowState::Waiting {
            self.state = WorkflowState::Bite;
        }
    }
}

pub fn action_log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("action_events.jsonl")
}

pub fn append_action_event(data_dir: &Path, event: &ActionEvent) -> Result<(), String> {
    let mut line = serde_json::to_string(event).map_err(|e| e.to_string())?;
    line.push('\n');
    std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(action_log_path(data_dir))
        .and_then(|mut f| f.write_all(line.as_bytes()))
        .map_err(|e| e.to_string())
}

/// Counts for the API/UI: outcomes + confirmation states + UNKNOWN rate.
pub fn summarize_action_log(data_dir: &Path) -> serde_json::Value {
    let mut outcomes: HashMap<String, usize> = HashMap::new();
    let mut confirmations: HashMap<String, usize> = HashMap::new();
    let mut total = 0usize;
    if let Ok(content) = std::fs::read_to_string(action_log_path(data_dir)) {
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            if let Ok(ev) = serde_json::from_str::<ActionEvent>(line) {
                total += 1;
                *outcomes.entry(ev.final_outcome.clone().unwrap_or_else(|| "open".to_string())).or_default() += 1;
                *confirmations.entry(ev.confirmation_state.as_str().to_string()).or_default() += 1;
            }
        }
    }
    serde_json::json!({ "events": total, "outcomes": outcomes, "confirmations": confirmations })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(workflow: &str, sess: &str) -> ActionEvent {
        ActionEvent {
            workflow_id: workflow.into(),
            event_id: format!("{workflow}#f000001"),
            frame_index: Some(1),
            session_id: sess.into(),
            result_event_id: None,
            entity_id: None,
            entity_type: None,
            policy_decision: None,
            action_requested: None,
            action_sent_at: None,
            confirmation_state: ConfirmationState::Pending,
            confirmation_evidence: None,
            confirmation_at: None,
            retry_count: 0,
            final_outcome: None,
        }
    }

    #[test]
    fn complete_success_path() {
        let mut t = WorkflowTracker::new("w1", "s1");
        t.observe_bite();
        assert_eq!(t.state, WorkflowState::Bite);
        let mut e = ev("w1", "s1");
        e.result_event_id = Some("s1#f000010".into());
        assert_eq!(t.observe(e), WorkflowState::ResultPending);
        let mut e = ev("w1", "s1");
        e.entity_id = Some("fish:golden".into());
        e.entity_type = Some("fish".into());
        assert_eq!(t.observe(e), WorkflowState::EntityConfirmed);
        let mut e = ev("w1", "s1");
        e.entity_id = Some("fish:golden".into());
        e.policy_decision = Some("no_action".into());
        assert_eq!(t.observe(e), WorkflowState::PolicyPending);
        // Action dispatch and confirmation observation are separate moments.
        let mut e = ev("w1", "s1");
        e.action_requested = Some("record_catch".into());
        e.action_sent_at = Some(100);
        assert_eq!(t.observe(e), WorkflowState::ConfirmationPending);
        let mut e = ev("w1", "s1");
        e.action_sent_at = Some(100);
        e.confirmation_state = ConfirmationState::Confirmed;
        e.confirmation_evidence = Some("catch recorded in session tally".into());
        e.final_outcome = Some("recorded".into());
        assert_eq!(t.observe(e), WorkflowState::Confirmed);
        assert!(t.state.terminal());
    }

    #[test]
    fn unknown_entity_never_reaches_action() {
        let mut t = WorkflowTracker::new("w2", "s1");
        let mut e = ev("w2", "s1");
        e.result_event_id = Some("s1#f000011".into());
        t.observe(e);
        // Entity stays unresolved: tracker parks in EntityPending, never acts.
        let e = ev("w2", "s1");
        assert_eq!(t.observe(e), WorkflowState::EntityPending);
        assert!(!matches!(t.state, WorkflowState::ActionPending | WorkflowState::ConfirmationPending));
    }

    #[test]
    fn missing_confirmation_is_unknown_not_success() {
        let mut t = WorkflowTracker::new("w3", "s1");
        let mut e = ev("w3", "s1");
        e.result_event_id = Some("r".into());
        assert_eq!(t.observe(e), WorkflowState::ResultPending);
        let mut e = ev("w3", "s1");
        e.entity_id = Some("fruit:kilo".into());
        assert_eq!(t.observe(e), WorkflowState::EntityConfirmed);
        let mut e = ev("w3", "s1");
        e.entity_id = Some("fruit:kilo".into());
        e.policy_decision = Some("allow_duplicate_drop".into());
        assert_eq!(t.observe(e), WorkflowState::PolicyPending);
        let mut e = ev("w3", "s1");
        e.action_requested = Some("backspace_drop".into());
        e.action_sent_at = Some(50);
        assert_eq!(t.observe(e), WorkflowState::ConfirmationPending);
        // Banner never observed: confirmation stays UNKNOWN, never success.
        let mut e = ev("w3", "s1");
        e.action_sent_at = Some(50);
        e.confirmation_state = ConfirmationState::Unknown;
        e.confirmation_evidence = Some("no banner after send + re-observation".into());
        e.retry_count = 0;
        assert_eq!(t.observe(e), WorkflowState::Unknown);
        assert_ne!(t.state, WorkflowState::Confirmed, "unconfirmed action must not read as success");
    }

    #[test]
    fn failed_confirmation_after_retry_is_failed() {
        let mut t = WorkflowTracker::new("w4", "s1");
        let mut e = ev("w4", "s1");
        e.result_event_id = Some("r".into());
        assert_eq!(t.observe(e), WorkflowState::ResultPending);
        let mut e = ev("w4", "s1");
        e.entity_id = Some("fruit:kilo".into());
        assert_eq!(t.observe(e), WorkflowState::EntityConfirmed);
        let mut e = ev("w4", "s1");
        e.entity_id = Some("fruit:kilo".into());
        e.policy_decision = Some("allow_duplicate_drop".into());
        assert_eq!(t.observe(e), WorkflowState::PolicyPending);
        let mut e = ev("w4", "s1");
        e.action_requested = Some("backspace_drop".into());
        e.action_sent_at = Some(50);
        assert_eq!(t.observe(e), WorkflowState::ConfirmationPending);
        // First observation inconclusive but a re-observation was spent.
        let mut e = ev("w4", "s1");
        e.action_sent_at = Some(50);
        e.confirmation_state = ConfirmationState::Unknown;
        e.retry_count = 1;
        assert_eq!(t.observe(e), WorkflowState::Recovery);
        let mut e2 = ev("w4", "s1");
        e2.confirmation_state = ConfirmationState::Timeout;
        assert_eq!(t.observe(e2), WorkflowState::Failed);
    }

    #[test]
    fn old_events_without_new_fields_parse() {
        let raw = r#"{"workflow_id":"w","event_id":"e","session_id":"s","confirmation_state":"UNKNOWN"}"#;
        let e: ActionEvent = serde_json::from_str(raw).unwrap();
        assert_eq!(e.retry_count, 0);
        assert_eq!(e.final_outcome, None);
        assert_eq!(e.confirmation_state, ConfirmationState::Unknown);
    }

    #[test]
    fn action_log_roundtrips_and_summarizes() {
        let dir = std::env::temp_dir().join("gpo-wf-log");
        let _ = std::fs::remove_dir_all(&dir);
        let mut e = ev("w9", "s9");
        e.confirmation_state = ConfirmationState::Confirmed;
        e.final_outcome = Some("stored".into());
        append_action_event(&dir, &e).unwrap();
        let mut e2 = ev("w9", "s9");
        e2.confirmation_state = ConfirmationState::Unknown;
        append_action_event(&dir, &e2).unwrap();
        let s = summarize_action_log(&dir);
        assert_eq!(s["events"], 2);
        assert_eq!(s["confirmations"]["CONFIRMED"], 1);
        assert_eq!(s["confirmations"]["UNKNOWN"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
