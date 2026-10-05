//! Live shadow observation (v5.4.0): tract inference + agreement telemetry.
//!
//! Hard rule: these functions OBSERVE and LOG. They take `&Ctx` only to
//! read settings/store/ml-session and to append events. They never call
//! input APIs, never change macro state, and their return values are `()`
//! so no caller can branch control flow on a vision prediction. The
//! production macro behaves identically with shadow on or off.

use std::time::Instant;

use crate::core::ml_capability::{ShadowEvent, append_shadow_event};

/// Sample every Nth scan frame so the shadow log stays small at 30-60 Hz.
pub const STATE_SAMPLE_EVERY: u64 = 30;

/// Shadow STATE inference on a bar frame.
/// `production_bar_present`: the production detector's verdict (bar locked).
/// Agreement is coarse by design: production sees bar/no-bar, vision names
/// waiting_for_bite/bite/catch_result. Agreement = (vision in
/// {waiting,bite}) == bar_present.
pub fn observe_state(ctx: &super::ctx::Ctx, frame: &crate::core::types::Frame, production_bar_present: bool) {
    if !ctx.settings.read().features.ml_shadow {
        return;
    }
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
    let session = ctx.ml.active_session_id().unwrap_or_else(|| "no-session".to_string());
    let now = crate::events::now_ms();
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
    };
    let _ = append_shadow_event(ctx.store.dir(), &ev);
}

/// Shadow FISH inference on a RESULT drop frame.
/// `ocr_entity`: the OCR+KB entity id (if any) for agreement scoring.
/// `ocr_none_agreement` is None (nothing to agree with) — logged, not hidden.
pub fn observe_fish(
    ctx: &super::ctx::Ctx,
    frame: &crate::core::types::Frame,
    ocr_entity: Option<&str>,
    ocr_text: &str,
) {
    if !ctx.settings.read().features.ml_shadow {
        return;
    }
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
    let session = ctx.ml.active_session_id().unwrap_or_else(|| "no-session".to_string());
    let now = crate::events::now_ms();
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
    };
    let _ = append_shadow_event(ctx.store.dir(), &ev);
}
