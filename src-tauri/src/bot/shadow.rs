//! Live shadow observation (v5.4.0): tract inference + agreement telemetry.
//!
//! Hard rule: these functions OBSERVE and LOG. They take `&Ctx` only to
//! read settings/store/ml-session and to append events. They never call
//! input APIs, never change macro state, and their return values are `()`
//! so no caller can branch control flow on a vision prediction. The
//! production macro behaves identically with shadow on or off.
//!
//! Each logged event carries the observing model's stem AND its manifest
//! revision, so a candidate promoted over a slot (stem unchanged, weights
//! replaced) is measured on its own observations, never the incumbent's.

use std::time::Instant;

use crate::core::ml_capability::{ShadowEvent, append_shadow_event};

/// Sample every Nth scan frame so the shadow log stays small at 30-60 Hz.
pub const STATE_SAMPLE_EVERY: u64 = 30;

/// Shadow STATE inference on a bar frame.
/// `production_bar_present`: the production detector's verdict (bar locked).
/// Agreement is coarse by design: production sees bar/no-bar, vision names
/// waiting_for_bite/bite/catch_result. Agreement = (vision in
/// {waiting,bite}) == bar_present.
fn infer_state(ctx: &Observation, frame: &crate::core::types::Frame, production_bar_present: bool) {
    let t0 = Instant::now();
    let models_dir = ctx.store.dir().join("models");
    let Some(eng) = crate::core::shadow_infer::engine(&models_dir) else { return };
    let Some(m) = eng.models.get("state_v1") else { return };
    let Some(chw) =
        crate::core::shadow_infer::preprocess_rgba(&frame.rgba, frame.w, frame.h, &m.spec)
    else {
        return;
    };
    let Some(probs) = eng.infer_probs("state_v1", &chw) else { return };
    let (top, conf) = probs
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(c, p)| (c.clone(), *p))
        .unwrap_or_default();
    let vision_is_live = top == "waiting_for_bite" || top == "bite";
    let session = ctx.session.clone();
    let now = ctx.timestamp_ms;
    let ev = ShadowEvent {
        session_id: session.clone(),
        event_id: format!("{session}#sh{now}"),
        timestamp_ms: now,
        vision_state: Some(top),
        production_state: Some(if production_bar_present { "bar_present" } else { "bar_absent" }.to_string()),
        state_confidence: Some(conf),
        result_category: None,
        entity: None,
        ocr_text: None,
        normalized_entity: None,
        policy_recommendation: None,
        would_be_action: None,
        actual_action: None,
        confirmation: None,
        latency_ms: Some(t0.elapsed().as_millis() as u64),
        agreement: Some(vision_is_live == production_bar_present),
        vision_confidence: None,
        model_version: Some(m.name.clone()),
        // Stem + revision: promotion overwrites a slot's weights under the
        // same stem, so stem alone would merge this candidate's telemetry
        // with the incumbent's.
        model_revision: Some(m.identity_revision()),
    };
    let _ = append_shadow_event(ctx.store.dir(), &ev);
}

/// Shadow FISH inference on a RESULT drop frame.
/// `ocr_entity`: the OCR+KB entity id (if any) for agreement scoring.
/// `ocr_none_agreement` is None (nothing to agree with) — logged, not hidden.
fn infer_fish(
    ctx: &Observation,
    frame: &crate::core::types::Frame,
    ocr_entity: Option<&str>,
    ocr_text: &str,
) {
    let t0 = Instant::now();
    let models_dir = ctx.store.dir().join("models");
    let Some(eng) = crate::core::shadow_infer::engine(&models_dir) else { return };
    let Some(m) = eng.models.get("fish_v1") else { return };
    let Some(chw) =
        crate::core::shadow_infer::preprocess_rgba(&frame.rgba, frame.w, frame.h, &m.spec)
    else {
        return;
    };
    let Some(probs) = eng.infer_probs("fish_v1", &chw) else { return };
    let (top, conf) = probs
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(c, p)| (c.clone(), *p))
        .unwrap_or_default();
    let session = ctx.session.clone();
    let now = ctx.timestamp_ms;
    let agreement = ocr_entity.map(|o| o == top);
    let ev = ShadowEvent {
        session_id: session.clone(),
        event_id: format!("{session}#sh{now}"),
        timestamp_ms: now,
        vision_state: Some("catch_result".to_string()),
        production_state: Some("catch_result".to_string()),
        state_confidence: None,
        result_category: Some("fish_candidate".to_string()),
        entity: Some(top),
        ocr_text: Some(ocr_text.to_string()),
        normalized_entity: ocr_entity.map(|s| s.to_string()),
        policy_recommendation: Some("advisory-only: vision never decides".to_string()),
        would_be_action: None,
        actual_action: None,
        confirmation: None,
        latency_ms: Some(t0.elapsed().as_millis() as u64),
        agreement,
        // Fish-vision confidence is informational only: no rejection
        // threshold was established on validation (model report), so the
        // confidence travels with the event and policy ignores it.
        vision_confidence: Some(conf),
        model_version: Some(m.name.clone()),
        model_revision: Some(m.identity_revision()),
    };
    let _ = append_shadow_event(ctx.store.dir(), &ev);
}


struct Observation {
    store: std::sync::Arc<crate::config::Store>,
    session: String,
    timestamp_ms: u64,
}
enum Sample {
    State(Observation, crate::core::types::Frame, bool),
    Fish(Observation, crate::core::types::Frame, Option<String>, String),
}
fn queue() -> &'static crossbeam_channel::Sender<Sample> {
    static QUEUE: std::sync::OnceLock<crossbeam_channel::Sender<Sample>> = std::sync::OnceLock::new();
    QUEUE.get_or_init(|| {
        let (tx, rx) = crossbeam_channel::bounded::<Sample>(2);
        std::thread::Builder::new().name("shadow-observation".into()).spawn(move || {
            for sample in rx {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match sample {
                    Sample::State(ctx, frame, bar) => infer_state(&ctx, &frame, bar),
                    Sample::Fish(ctx, frame, entity, text) => infer_fish(&ctx, &frame, entity.as_deref(), &text),
                }));
                if result.is_err() { tracing::warn!("shadow observation failed; macro unaffected"); }
            }
        }).expect("spawn shadow observation worker");
        tx
    })
}
fn observation(ctx: &super::ctx::Ctx) -> Observation {
    Observation { store: std::sync::Arc::clone(&ctx.store),
        session: ctx.ml.active_session_id().unwrap_or_else(|| "no-session".into()),
        timestamp_ms: crate::events::now_ms() }
}
fn enqueue(sample: Sample) {
    static DROPPED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if queue().try_send(sample).is_err() {
        let count = DROPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if count == 1 || count % 100 == 0 { tracing::warn!("shadow queue busy: {count} observations dropped"); }
    }
}
pub fn observe_state(ctx: &super::ctx::Ctx, frame: &crate::core::types::Frame, bar: bool) {
    if ctx.settings.read().features.ml_shadow { enqueue(Sample::State(observation(ctx), frame.clone(), bar)); }
}
pub fn observe_fish(ctx: &super::ctx::Ctx, frame: &crate::core::types::Frame, entity: Option<&str>, text: &str) {
    if ctx.settings.read().features.ml_shadow { enqueue(Sample::Fish(observation(ctx), frame.clone(), entity.map(str::to_string), text.to_string())); }
}
