use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::app::AppState;
use crate::config::{import_legacy, Settings};
use crate::core::types::{PxPoint, PxRect, RelPoint, RelRect, WindowInfo};
use crate::core::vision;
use crate::events::{BotState, Stats};
use crate::hotkeys;
use crate::windows;

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub state: BotState,
    pub paused: bool,
    pub stats: Stats,
    pub roblox: Option<WindowInfo>,
    pub ocr_available: bool,
    pub settings: Settings,
    pub version: String,
}

#[tauri::command]
pub fn snapshot(app: AppHandle, st: State<'_, AppState>) -> Snapshot {
    Snapshot {
        state: st.bot.state(),
        paused: st.bot.is_paused(),
        stats: st.bot.ctx().session.lock().stats(),
        roblox: *st.roblox.read(),
        ocr_available: st.platform.ocr.available(),
        settings: st.settings.read().clone(),
        version: app.package_info().version.to_string(),
    }
}

#[tauri::command]
pub fn bot_start(st: State<'_, AppState>) {
    let bot = Arc::clone(&st.bot);
    std::thread::spawn(move || bot.start());
}

#[tauri::command]
pub fn bot_pause(st: State<'_, AppState>) {
    let bot = Arc::clone(&st.bot);
    std::thread::spawn(move || bot.pause());
}

#[tauri::command]
pub fn bot_stop(st: State<'_, AppState>) {
    let bot = Arc::clone(&st.bot);
    std::thread::spawn(move || bot.stop());
}

#[tauri::command]
pub fn bot_toggle(st: State<'_, AppState>) {
    let bot = Arc::clone(&st.bot);
    std::thread::spawn(move || bot.toggle());
}

#[tauri::command]
pub fn settings_get(st: State<'_, AppState>) -> Settings {
    st.settings.read().clone()
}

#[tauri::command]
pub fn settings_set(app: AppHandle, st: State<'_, AppState>, mut settings: Settings) -> Result<(), String> {
    let (hotkeys_changed, hud_changed) = {
        let cur = st.settings.read();
        settings.ui.panel_offset = cur.ui.panel_offset;
        settings.ui.panel_size = cur.ui.panel_size;
        (cur.hotkeys != settings.hotkeys, cur.ui.hud_visible != settings.ui.hud_visible || cur.ui.hud_offset != settings.ui.hud_offset)
    };
    *st.settings.write() = settings.clone();
    st.store.save(&settings).map_err(|e| e.to_string())?;
    if hotkeys_changed {
        hotkeys::register(&app, &settings.hotkeys)?;
    }
    if hud_changed {
        windows::reposition(&app);
    }
    let _ = app.emit("settings:changed", &settings);
    Ok(())
}

#[tauri::command]
pub fn settings_reset(app: AppHandle, st: State<'_, AppState>) -> Result<Settings, String> {
    // Recoverable backup before any destructive reset (surfaced in the
    // Settings UI confirmation). A failed backup aborts the reset.
    let backup = st.store.backup_settings()?;
    let mut s = Settings::default();
    // Preserve this install's dashboard token so saved browser bookmarks
    // (/?token=) keep working across a reset.
    s.web.token = st.settings.read().web.token.clone();
    if s.web.token.is_empty() {
        s.ensure_web_token();
    }
    settings_set(app, st, s.clone())?;
    if let Some(p) = backup {
        tracing::info!("settings reset; pre-reset backup at {}", p.display());
    }
    Ok(s)
}

/// Error recorded when the settings file was corrupt at load and had to be
/// quarantined (see `Store::load`). `None` means a clean load.
#[tauri::command]
pub fn settings_load_error(st: State<'_, AppState>) -> Option<String> {
    st.store.peek_load_error()
}

#[tauri::command]
pub fn preset_list(st: State<'_, AppState>) -> Vec<String> {
    st.store.list_presets()
}

#[tauri::command]
pub fn preset_save(st: State<'_, AppState>, name: String) -> Result<(), String> {
    st.store.save_preset(&name, &st.settings.read()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn preset_load(app: AppHandle, st: State<'_, AppState>, name: String) -> Result<Settings, String> {
    let s = st.store.load_preset(&name).map_err(|e| e.to_string())?;
    settings_set(app, st, s.clone())?;
    Ok(s)
}

#[tauri::command]
pub fn preset_delete(st: State<'_, AppState>, name: String) -> Result<(), String> {
    st.store.delete_preset(&name).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn legacy_import(app: AppHandle, st: State<'_, AppState>, json: String) -> Result<Settings, String> {
    let rect = st.roblox.read().map(|w| w.client).ok_or("Open Roblox first so coordinates can be converted")?;
    let s = import_legacy(&json, rect).map_err(|e| e.to_string())?;
    settings_set(app, st, s.clone())?;
    Ok(s)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayTarget {
    BarRegion,
    DropRegion,
    ServerTimeRegion,
    BaitMenuRegion,
    FishingPoint,
    Purchase1,
    Purchase2,
    Purchase3,
    Purchase4,
    Fruit1,
    Fruit2,
    Bait1,
    Bait2,
    RodSlot,
}

impl OverlayTarget {
    pub fn is_region(self) -> bool {
        matches!(
            self,
            OverlayTarget::BarRegion
                | OverlayTarget::DropRegion
                | OverlayTarget::ServerTimeRegion
                | OverlayTarget::BaitMenuRegion
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OverlaySession {
    pub target: OverlayTarget,
    pub roblox: PxRect,
    pub overlay_origin: PxPoint,
    pub region: Option<RelRect>,
    pub point: Option<RelPoint>,
}

#[tauri::command]
pub fn overlay_open(app: AppHandle, st: State<'_, AppState>, target: OverlayTarget) -> Result<OverlaySession, String> {
    let roblox = st.roblox.read().map(|w| w.client).ok_or("Roblox window not found")?;
    let origin = windows::show_overlay(&app, roblox, true)?;
    let s = st.settings.read();
    let (region, point) = match target {
        OverlayTarget::BarRegion => (Some(s.regions.bar), None),
        OverlayTarget::DropRegion => (Some(s.regions.drop), None),
        OverlayTarget::ServerTimeRegion => (Some(s.regions.server_time), None),
        OverlayTarget::BaitMenuRegion => (Some(s.regions.bait_menu), None),
        OverlayTarget::FishingPoint => (None, Some(s.points.fishing)),
        OverlayTarget::Purchase1 => (None, s.points.purchase[0]),
        OverlayTarget::Purchase2 => (None, s.points.purchase[1]),
        OverlayTarget::Purchase3 => (None, s.points.purchase[2]),
        OverlayTarget::Purchase4 => (None, s.points.purchase[3]),
        OverlayTarget::Fruit1 => (None, s.points.fruit[0]),
        OverlayTarget::Fruit2 => (None, s.points.fruit[1]),
        OverlayTarget::Bait1 => (None, s.points.bait[0]),
        OverlayTarget::Bait2 => (None, s.points.bait[1]),
        OverlayTarget::RodSlot => (None, s.points.rod_slot),
    };
    let session = OverlaySession { target, roblox, overlay_origin: origin, region, point };
    *st.overlay_session.lock() = Some(serde_json::json!({ "kind": "single", "session": session }));
    if let Some(o) = app.get_webview_window("overlay") {
        let _ = o.emit("overlay:session", &session);
    }
    Ok(session)
}

#[derive(Debug, Deserialize)]
pub struct OverlayCommit {
    pub target: OverlayTarget,
    pub region: Option<RelRect>,
    pub point: Option<RelPoint>,
}

#[tauri::command]
pub fn overlay_commit(app: AppHandle, st: State<'_, AppState>, commit: OverlayCommit) -> Result<Settings, String> {
    let mut s = st.settings.read().clone();
    match commit.target {
        OverlayTarget::BarRegion => s.regions.bar = commit.region.ok_or("region required")?,
        OverlayTarget::DropRegion => s.regions.drop = commit.region.ok_or("region required")?,
        OverlayTarget::ServerTimeRegion => s.regions.server_time = commit.region.ok_or("region required")?,
        OverlayTarget::BaitMenuRegion => s.regions.bait_menu = commit.region.ok_or("region required")?,
        OverlayTarget::FishingPoint => s.points.fishing = commit.point.ok_or("point required")?,
        OverlayTarget::Purchase1 => s.points.purchase[0] = commit.point,
        OverlayTarget::Purchase2 => s.points.purchase[1] = commit.point,
        OverlayTarget::Purchase3 => s.points.purchase[2] = commit.point,
        OverlayTarget::Purchase4 => s.points.purchase[3] = commit.point,
        OverlayTarget::Fruit1 => s.points.fruit[0] = commit.point,
        OverlayTarget::Fruit2 => s.points.fruit[1] = commit.point,
        OverlayTarget::Bait1 => s.points.bait[0] = commit.point,
        OverlayTarget::Bait2 => s.points.bait[1] = commit.point,
        OverlayTarget::RodSlot => s.points.rod_slot = commit.point,
    }
    windows::hide_overlay(&app);
    settings_set(app, st, s.clone())?;
    Ok(s)
}

#[tauri::command]
pub fn overlay_cancel(app: AppHandle) {
    windows::hide_overlay(&app);
}

#[derive(Debug, Clone, Serialize)]
pub struct RegionsSession {
    pub roblox: PxRect,
    pub bar: RelRect,
    pub drop: RelRect,
    pub server_time: RelRect,
    pub bait_menu: RelRect,
}

pub fn open_regions_editor(app: &AppHandle) -> Result<RegionsSession, String> {
    let st = app.state::<AppState>();
    let roblox = st.roblox.read().map(|w| w.client).ok_or("Roblox window not found")?;
    windows::show_overlay(app, roblox, true)?;
    let s = st.settings.read();
    let session = RegionsSession {
        roblox,
        bar: s.regions.bar,
        drop: s.regions.drop,
        server_time: s.regions.server_time,
        bait_menu: s.regions.bait_menu,
    };
    drop(s);
    *st.overlay_session.lock() = Some(serde_json::json!({ "kind": "regions", "session": session }));
    if let Some(o) = app.get_webview_window("overlay") {
        let _ = o.emit("overlay:regions", &session);
    }
    Ok(session)
}

#[tauri::command]
pub fn overlay_pending(st: State<'_, AppState>) -> Option<serde_json::Value> {
    st.overlay_session.lock().clone()
}

#[tauri::command]
pub fn overlay_open_regions(app: AppHandle) -> Result<RegionsSession, String> {
    open_regions_editor(&app)
}

#[derive(Debug, Deserialize)]
pub struct RegionsCommit {
    pub bar: RelRect,
    pub drop: RelRect,
    pub server_time: RelRect,
    pub bait_menu: RelRect,
}

#[tauri::command]
pub fn overlay_commit_regions(app: AppHandle, st: State<'_, AppState>, commit: RegionsCommit) -> Result<Settings, String> {
    let mut s = st.settings.read().clone();
    s.regions.bar = commit.bar;
    s.regions.drop = commit.drop;
    s.regions.server_time = commit.server_time;
    s.regions.bait_menu = commit.bait_menu;
    windows::hide_overlay(&app);
    settings_set(app, st, s.clone())?;
    Ok(s)
}

#[tauri::command]
pub fn panel_placement_changed(app: AppHandle) {
    windows::save_panel_placement(&app);
}

#[derive(Debug, Serialize)]
pub struct RegionPreview {
    pub width: usize,
    pub height: usize,
    pub png_base64: String,
    pub confidence: vision::Confidence,
    pub reading: Option<vision::Reading>,
}

#[tauri::command]
pub async fn region_preview(st: State<'_, AppState>, region: RelRect, max_dim: Option<u32>) -> Result<RegionPreview, String> {
    let roblox = st.roblox.read().map(|w| w.client).ok_or("Roblox window not found")?;
    let capture = st.platform.capture.clone();
    let palette = st.settings.read().fishing.palette;
    let max_dim = max_dim.unwrap_or(320) as usize;
    blocking(move || {
        let frame = capture.grab(region.to_px(&roblox)).map_err(|e| e.to_string())?;
        let confidence = vision::confidence(&frame, &palette);
        let reading = vision::read(&frame, &palette);
        let small = frame.downscale(max_dim);
        let png = encode_png(&small)?;
        Ok(RegionPreview { width: frame.w, height: frame.h, png_base64: png, confidence, reading })
    })
    .await
}

#[derive(Debug, Serialize)]
pub struct OcrTest {
    pub text: String,
    pub ocr_variant: String,
    pub drop: Option<crate::core::fruit::DropInfo>,
    pub spawn: Option<crate::core::fruit::SpawnInfo>,
    pub observation: crate::core::perception::Observation,
}

#[tauri::command]
pub async fn ocr_test(st: State<'_, AppState>) -> Result<OcrTest, String> {
    let roblox = st.roblox.read().map(|w| w.client).ok_or("Roblox window not found")?;
    let region = st.settings.read().regions.drop;
    let lex = st.settings.read().lexicon.clone();
    let kb = st.store.effective_knowledge();
    let platform = st.platform.clone();
    blocking(move || {
        if !platform.ocr.available() {
            return Err("Windows OCR is not available on this system".into());
        }
        let frame = platform.capture.grab(region.to_px(&roblox)).map_err(|e| e.to_string())?;
        // Bounded event-driven read: raw first, one 2x retry when empty.
        let (text, variant) = crate::core::perception::ocr_with_fallback(
            |f| platform.ocr.read(f).map_err(|e| e.to_string()),
            &frame,
        )?;
        let observation = crate::core::perception::correlate_text(
            &kb,
            &text,
            "drop",
            crate::core::perception::ScreenKind::Fishing,
            lex.fuzzy_threshold,
            0.80,
            None,
        );
        Ok(OcrTest {
            drop: crate::core::fruit::detect_drop(&lex, &text),
            spawn: crate::core::fruit::detect_spawn(&lex, &text),
            ocr_variant: variant.to_string(),
            observation,
            text,
        })
    })
    .await
}

#[tauri::command]
pub async fn webhook_test(st: State<'_, AppState>) -> Result<(), String> {
    let wh = st.webhook.clone();
    blocking(move || wh.test()).await
}

#[tauri::command]
pub async fn detect_bar_region(st: State<'_, AppState>) -> Result<RelRect, String> {
    let roblox = st.roblox.read().map(|w| w.client).ok_or("Roblox window not found")?;
    let capture = st.platform.capture.clone();
    let palette = st.settings.read().fishing.palette;
    blocking(move || {
        let frame = capture.grab(roblox).map_err(|e| e.to_string())?;
        let bbox = vision::find_bar(&frame, &palette)
            .ok_or("No fishing bar visible. Cast your line, wait for the bar, then try again.")?;
        let pad_x = (bbox.w() as f32 * 0.35).ceil() as i32;
        let pad_y = (bbox.h() as f32 * 0.15).ceil() as i32;
        let px = PxRect {
            x: roblox.x + bbox.x0 as i32 - pad_x,
            y: roblox.y + bbox.y0 as i32 - pad_y,
            w: bbox.w() as i32 + pad_x * 2,
            h: bbox.h() as i32 + pad_y * 2,
        };
        Ok(px.to_rel(&roblox))
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthItem {
    /// Stable id: roblox, ocr, bar, drop, bait_menu, server_time, vision, points.
    pub id: String,
    pub label: String,
    /// "pass" | "warn" | "fail".
    pub status: String,
    pub detail: String,
    pub score: Option<f32>,
    /// Configured (a region/point exists) vs actually detected right now.
    /// "Configured" is never presented as "detected".
    pub configured: bool,
    pub detected: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthCheck {
    pub roblox: bool,
    pub knowledge_entities: usize,
    pub items: Vec<HealthItem>,
}

fn health_item(
    id: &str,
    label: &str,
    status: &str,
    detail: String,
    score: Option<f32>,
    configured: bool,
    detected: bool,
) -> HealthItem {
    HealthItem {
        id: id.into(),
        label: label.into(),
        status: status.into(),
        detail,
        score,
        configured,
        detected,
    }
}

/// Real calibration/diagnostics probe: captures each configured region from
/// the live Roblox window and reports vision confidence + OCR results.
/// Nothing is simulated — failures name the exact region and remedy.
#[tauri::command]
pub async fn health_check(st: State<'_, AppState>) -> Result<HealthCheck, String> {
    let roblox = st.roblox.read().map(|w| w.client);
    let s = st.settings.read().clone();
    let kb = st.store.effective_knowledge();
    let knowledge_entities = kb.len();
    let collect_sample = s.fishing.trace;
    let dataset = crate::core::dataset::DatasetStore::new(st.store.dir().to_path_buf());
    let platform = st.platform.clone();
    blocking(move || {
        let mut items = Vec::new();
        let Some(client) = roblox else {
            items.push(health_item(
                "roblox",
                "Roblox window",
                "fail",
                "Roblox window not found. Launch Roblox, join GPO, and keep the window visible (not minimized).".into(),
                None,
                false,
                false,
            ));
            return Ok(HealthCheck { roblox: false, knowledge_entities, items });
        };
        items.push(health_item(
            "roblox",
            "Roblox window",
            "pass",
            format!("Detected {}x{} at ({}, {}).", client.w, client.h, client.x, client.y),
            None,
            true,
            true,
        ));

        if platform.ocr.available() {
            items.push(health_item("ocr", "Windows OCR", "pass", "English text recognition is available.".into(), None, true, true));
        } else {
            items.push(health_item(
                "ocr",
                "Windows OCR",
                "fail",
                "Windows text recognition is unavailable. Install the English OCR language pack, then restart the app.".into(),
                None,
                true,
                false,
            ));
        }

        // Bar region: capture + vision confidence + live reading.
        match platform.capture.grab(s.regions.bar.to_px(&client)) {
            Ok(frame) => {
                let conf = vision::confidence(&frame, &s.fishing.palette);
                let reading = vision::read(&frame, &s.fishing.palette);
                let live = match &reading {
                    Some(r) => format!(" Fish {:.2} / marker {:.2}.", r.fish_center, r.marker_center),
                    None => " No live fish/marker right now.".into(),
                };
                let (status, detail) = if conf.score >= 0.5 {
                    ("pass", format!("Bar matched (score {:.0}%).{live}", conf.score * 100.0))
                } else if conf.score >= 0.2 {
                    ("warn", format!("Weak bar match (score {:.0}%). Cast once by hand and use Auto-detect, or raise color tolerance.{live}", conf.score * 100.0))
                } else {
                    ("fail", "No bar detected in this area. Redraw the bar area tighter around the blue bar, or press Auto-detect while the bar is visible.".into())
                };
                let bar_configured = s.regions.bar.w > 0.02 && s.regions.bar.h > 0.02;
                let bar_detected = conf.score >= 0.2;
                items.push(health_item("bar", "Fishing bar area", status, detail, Some(conf.score), bar_configured, bar_detected));
                match reading {
                    Some(r) => items.push(health_item(
                        "vision",
                        "Vision pipeline",
                        "pass",
                        format!("Bar found; fish {:.2}, marker {:.2}, error {:+.3}.", r.fish_center, r.marker_center, r.error),
                        None,
                        true,
                        true,
                    )),
                    None => items.push(health_item(
                        "vision",
                        "Vision pipeline",
                        "warn",
                        "No live bar right now (normal when not reeling). Start fishing to see live values.".into(),
                        None,
                        true,
                        false,
                    )),
                }
            }
            Err(e) => items.push(health_item("bar", "Fishing bar area", "fail", format!("Capture failed: {e}. Ensure Roblox is visible and not minimized."), None, true, false)),
        }

        // Drop region: capture + OCR sample with fruit/drop/spawn verdicts +
        // perception correlation against the knowledge base.
        match platform.capture.grab(s.regions.drop.to_px(&client)) {
            Ok(frame) => {
                if !platform.ocr.available() {
                    items.push(health_item("drop", "Drop message area", "warn", "Captured, but OCR is unavailable so text cannot be verified.".into(), None, true, false));
                } else {
                    match crate::core::perception::ocr_with_fallback(
                        |f| platform.ocr.read(f).map_err(|e| e.to_string()),
                        &frame,
                    ) {
                        Ok((text, variant)) => {
                            let t = text.trim();
                            if t.is_empty() {
                                items.push(health_item("drop", "Drop message area", "warn", "OCR reads empty (normal when no popup is showing). Trigger a drop message and use Read now to verify.".into(), None, true, false));
                            } else {
                                let drop = crate::core::fruit::detect_drop(&s.lexicon, &text);
                                let spawn = crate::core::fruit::detect_spawn(&s.lexicon, &text);
                                let verdict = if let Some(d) = drop {
                                    format!("drop detected: {}", d.name.as_deref().unwrap_or("unknown"))
                                } else if let Some(sp) = spawn {
                                    format!("spawn detected: {}", sp.name.as_deref().unwrap_or("unknown"))
                                } else {
                                    "text read, no fruit phrase matched".into()
                                };
                                let obs = crate::core::perception::correlate_text(
                                    &kb,
                                    &text,
                                    "drop",
                                    crate::core::perception::ScreenKind::Fishing,
                                    s.lexicon.fuzzy_threshold,
                                    0.80,
                                    None,
                                );
                                let perception = match &obs.entity {
                                    Some(m) => format!(
                                        " Perception: {} ({:.2}: {}).",
                                        m.canonical_name,
                                        m.confidence,
                                        m.evidence.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>().join("+")
                                    ),
                                    None => format!(
                                        " Perception: unknown ({:.2}) — {}.",
                                        obs.confidence,
                                        obs.unknown_reason.as_deref().unwrap_or("weak evidence")
                                    ),
                                };
                                // Learning pipeline: persist this observation
                                // when trace recording is on (best-effort).
                                if collect_sample {
                                    dataset.record(
                                        "health_check",
                                        "drop",
                                        "diagnostics",
                                        &text,
                                        &obs,
                                        crate::core::dataset::DatasetStore::png_of(&frame),
                                    );
                                }
                                items.push(health_item(
                                    "drop",
                                    "Drop message area",
                                    "pass",
                                    format!("OCR reads [{variant}]: “{t}” ({verdict}).{perception}"),
                                    None,
                                    true,
                                    true,
                                ));
                            }
                        }
                        Err(e) => items.push(health_item("drop", "Drop message area", "fail", format!("OCR failed: {e}. Make the area cover the whole popup."), None, true, false)),
                    }
                }
            }
            Err(e) => items.push(health_item("drop", "Drop message area", "fail", format!("Capture failed: {e}."), None, true, false)),
        }

        // Bait menu: capture check only (parsed during smart-bait scans).
        match platform.capture.grab(s.regions.bait_menu.to_px(&client)) {
            Ok(frame) => items.push(health_item(
                "bait_menu",
                "Bait menu area",
                "pass",
                format!("Captured {}x{} px. Open the rod bait menu and use Scan stock in Features to verify parsing.", frame.w, frame.h),
                None,
                s.regions.bait_menu.w > 0.02,
                true,
            )),
            Err(e) => items.push(health_item("bait_menu", "Bait menu area", "fail", format!("Capture failed: {e}."), None, true, false)),
        }

        // Server time: capture + OCR must return text.
        match platform.capture.grab(s.regions.server_time.to_px(&client)) {
            Ok(frame) => {
                if !platform.ocr.available() {
                    items.push(health_item("server_time", "Server timer area", "warn", "Captured, but OCR is unavailable.".into(), None, true, false));
                } else {
                    match platform.ocr.read(&frame) {
                        Ok(text) => {
                            let t = text.trim().to_string();
                            if t.is_empty() {
                                items.push(health_item("server_time", "Server timer area", "warn", "OCR reads empty. Redraw the area over the server clock (e.g. day/time display).".into(), None, true, false));
                            } else {
                                items.push(health_item("server_time", "Server timer area", "pass", format!("OCR reads: “{t}”."), None, true, true));
                            }
                        }
                        Err(e) => items.push(health_item("server_time", "Server timer area", "fail", format!("OCR failed: {e}."), None, true, false)),
                    }
                }
            }
            Err(e) => items.push(health_item("server_time", "Server timer area", "fail", format!("Capture failed: {e}."), None, true, false)),
        }

        // Points: required calibration points for the enabled features.
        let mut missing: Vec<&str> = Vec::new();
        if s.features.auto_purchase && (s.points.purchase[0].is_none() || s.points.purchase[1].is_none()) {
            missing.push("shop Confirm + Quantity (auto-purchase is on)");
        }
        if s.features.fruit_storage && s.points.fruit.iter().all(|p| p.is_none()) {
            missing.push("fruit slot point (fruit storage is on)");
        }
        if s.features.auto_bait && s.points.bait.iter().all(|p| p.is_none()) {
            missing.push("bait slot point (auto-bait is on)");
        }
        if missing.is_empty() {
            items.push(health_item("points", "Calibration points", "pass", "All points required by the enabled features are set.".into(), None, true, true));
        } else {
            items.push(health_item("points", "Calibration points", "fail", format!("Missing: {}. Use Pick in Features to set them.", missing.join("; ")), None, false, false));
        }

        Ok(HealthCheck { roblox: true, knowledge_entities, items })
    })
    .await
}

#[tauri::command]
pub fn hud_set_offset(app: AppHandle, st: State<'_, AppState>, offset: RelPoint) -> Result<(), String> {
    let mut s = st.settings.read().clone();
    s.ui.hud_offset = offset;
    settings_set(app, st, s)
}

#[tauri::command]
pub fn hud_toggle(app: AppHandle, st: State<'_, AppState>) -> Result<bool, String> {
    let mut s = st.settings.read().clone();
    s.ui.hud_visible = !s.ui.hud_visible;
    let v = s.ui.hud_visible;
    settings_set(app.clone(), st, s)?;
    windows::set_hud_visible(&app, v);
    Ok(v)
}

#[tauri::command]
pub fn panel_show(app: AppHandle, st: State<'_, AppState>) {
    st.panel_requested.store(true, std::sync::atomic::Ordering::SeqCst);
    windows::show_panel(&app);
}

#[tauri::command]
pub fn panel_toggle(app: AppHandle) {
    windows::toggle_panel(&app);
}

#[tauri::command]
pub fn panel_hide(app: AppHandle) {
    windows::hide_panel(&app);
}

#[tauri::command]
pub fn panel_visible(app: AppHandle) -> bool {
    windows::panel_visible(&app)
}

#[tauri::command]
pub fn guide_open(app: AppHandle) -> Result<(), String> {
    windows::show_guide(&app)
}

#[tauri::command]
pub fn guide_hide(app: AppHandle) {
    windows::hide_guide(&app);
}

#[tauri::command]
pub fn app_quit(app: AppHandle, st: State<'_, AppState>) {
    st.bot.stop();
    app.exit(0);
}

#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn data_dir(st: State<'_, AppState>) -> String {
    st.store.dir().display().to_string()
}

/// Authenticated local URL for Panel → Open Studio. The token is attached as
/// `?token=` on first visit and moved to localStorage by the page itself.
#[tauri::command]
pub fn web_dashboard_url(st: State<'_, AppState>) -> String {
    let s = st.settings.read();
    format!("http://127.0.0.1:3888/?token={}", s.web.token)
}

/// Toggle LAN exposure of the web dashboard. Takes effect on next app start
/// (the listener binds at spawn); the UI must tell the user to restart.
#[tauri::command]
pub fn web_set_allow_lan(app: AppHandle, st: State<'_, AppState>, allow: bool) -> Result<Settings, String> {
    let mut s = st.settings.read().clone();
    s.web.allow_lan = allow;
    settings_set(app, st, s.clone())?;
    Ok(s)
}

/// Rotate the per-install dashboard token (invalidates saved bookmarks).
#[tauri::command]
pub fn web_regenerate_token(app: AppHandle, st: State<'_, AppState>) -> Result<Settings, String> {
    let mut s = st.settings.read().clone();
    s.web.token = crate::core::secrets::generate_token_hex(32);
    settings_set(app, st, s.clone())?;
    Ok(s)
}

#[tauri::command]
pub fn catches_list(st: State<'_, AppState>) -> Vec<crate::config::CatchRecord> {
    st.store.get_catches()
}

#[tauri::command]
pub fn catches_clear(st: State<'_, AppState>) -> Result<(), String> {
    st.store.clear_catches()
}

#[tauri::command]
pub fn catches_open(st: State<'_, AppState>) -> Result<(), String> {
    st.store.open_catches_file()
}

#[tauri::command]
pub fn dataset_list(st: State<'_, AppState>) -> Vec<crate::core::dataset::DatasetSample> {
    let mut samples = crate::core::dataset::DatasetStore::new(st.store.dir().to_path_buf()).list();
    samples.reverse();
    samples.truncate(50);
    samples
}

#[tauri::command]
pub fn dataset_label(
    st: State<'_, AppState>,
    id: String,
    label: String,
    correct: bool,
) -> Result<crate::core::dataset::DatasetSample, String> {
    crate::core::dataset::DatasetStore::new(st.store.dir().to_path_buf()).label(&id, &label, correct)
}

#[tauri::command]
pub fn dataset_open(st: State<'_, AppState>) -> Result<(), String> {
    let dir = crate::core::dataset::DatasetStore::new(st.store.dir().to_path_buf());
    let path = dir.dir().to_path_buf();
    if !path.exists() {
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }
    std::process::Command::new("explorer")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn ml_store(st: &State<'_, AppState>) -> crate::core::ml_dataset::MlDatasetStore {
    crate::core::ml_dataset::MlDatasetStore::new(st.store.dir().to_path_buf())
}

#[tauri::command]
pub fn ml_samples(st: State<'_, AppState>) -> Vec<crate::core::ml_dataset::MlAnnotation> {
    // Bounded read. This previously parsed the entire labels file, reversed it,
    // and truncated to 100 - so showing the newest 100 samples cost a full parse
    // of every row in the dataset.
    ml_store(&st).annotations_page(0, 100)
}

#[tauri::command]
pub fn ml_annotate(
    st: State<'_, AppState>,
    image_id: String,
    ui_label: Option<String>,
    game_state: Option<String>,
    entity_id: Option<String>,
    hard_example: bool,
    hard_reason: Option<String>,
) -> Result<crate::core::ml_dataset::MlAnnotation, String> {
    use crate::core::ml_dataset::{GameStateLabel, UiLabel};
    // Closed vocabularies only: free text is accepted solely for entity ids,
    // which are validated against the knowledge base below.
    let ui = match ui_label.as_deref() {
        None | Some("") => None,
        Some("fishing_bar") => Some(UiLabel::FishingBar),
        Some("bait_menu") => Some(UiLabel::BaitMenu),
        Some("drop_indicator") => Some(UiLabel::DropIndicator),
        Some("server_time") => Some(UiLabel::ServerTime),
        Some("fruit_indicator") => Some(UiLabel::FruitIndicator),
        Some("fish_result") => Some(UiLabel::FishResult),
        Some("disconnect_screen") => Some(UiLabel::DisconnectScreen),
        Some("reconnect_screen") => Some(UiLabel::ReconnectScreen),
        Some(other) => return Err(format!("unknown ui label '{other}'")),
    };
    let state = match game_state.as_deref() {
        None | Some("") => None,
        Some("idle") => Some(GameStateLabel::Idle),
        Some("fishing") => Some(GameStateLabel::Fishing),
        Some("waiting_for_bite") => Some(GameStateLabel::WaitingForBite),
        Some("bite") => Some(GameStateLabel::Bite),
        Some("catch_result") => Some(GameStateLabel::CatchResult),
        Some("bait_menu") => Some(GameStateLabel::BaitMenu),
        Some("loading") => Some(GameStateLabel::Loading),
        Some("disconnected") => Some(GameStateLabel::Disconnected),
        Some("unknown") => Some(GameStateLabel::Unknown),
        Some(other) => return Err(format!("unknown game state '{other}'")),
    };
    if let Some(e) = &entity_id {
        if !e.trim().is_empty() {
            let kb = st.store.effective_knowledge();
            let ok = kb.find_by_name(e).is_some() || kb.entities().iter().any(|k| k.id == *e);
            if !ok {
                return Err(format!("unknown knowledge entity '{e}' — pick one from the knowledge base"));
            }
        }
    }
    ml_store(&st).annotate(
        &image_id,
        ui,
        None,
        state,
        entity_id.filter(|e| !e.trim().is_empty()),
        "user",
        hard_example,
        hard_reason,
    )
}

#[tauri::command]
pub fn ml_validate(st: State<'_, AppState>) -> crate::core::ml_dataset::DatasetReport {
    ml_store(&st).validate(&st.store.effective_knowledge())
}

#[tauri::command]
pub fn ml_baseline(st: State<'_, AppState>) -> crate::core::ml_eval::BaselineReport {
    let kb = st.store.effective_knowledge();
    let lex = st.settings.read().lexicon.clone();
    crate::core::ml_eval::evaluate_store(&kb, &lex, &ml_store(&st))
}

/// Training readiness thresholds. Deliberately conservative: training must
/// never start on thin, single-session, or leaking data.
pub const READINESS_MIN_VERIFIED: usize = 1000;
pub const READINESS_MIN_SESSIONS: usize = 8;

#[derive(Debug, Serialize)]
pub struct LeakageDetail {
    /// Same file attributed to sessions in >1 split: one gameplay event
    /// visible to both training and evaluation. TRUE event leakage.
    pub exact_duplicate_files: Vec<String>,
    /// Leakage clusters without a shared file: recurring static UI /
    /// background crops (visually similar, distinct events). Reported,
    /// never silently dropped.
    pub near_similarity_groups: usize,
    pub total_groups: usize,
}

#[derive(Debug, Serialize)]
pub struct TrainingReadiness {
    pub ready: bool,
    pub training: String,
    /// Entity-linked row count (kept for backward compatibility). Semantics:
    /// this specifically means entity-linked rows — in practice RESULT rows,
    /// since episode WAITING/BITE frames never carry entities by design.
    /// Prefer `entity_linked_result` for new consumers.
    pub verified: usize,
    pub required_verified: usize,
    /// Honest name for the entity gate: entity-linked RESULT rows.
    /// The 1000 requirement is unchanged and applies ONLY here — WAITING
    /// and BITE coverage is reported separately, never folded in.
    pub entity_linked_result: usize,
    pub required_entity_linked: usize,
    /// Per game-state class: total / eligible / excluded / transition /
    /// sessions. Computed, reporting-only (see state_eligibility()).
    pub state_coverage: std::collections::HashMap<String, crate::core::ml_dataset::StateEligibility>,
    /// Minimum eligible rows across the three model classes
    /// (waiting_for_bite, bite, catch_result). Informational: the hard
    /// gates below are unchanged; low per-class eligibility is surfaced,
    /// never hidden behind the RESULT count.
    pub min_state_eligible: usize,
    pub sessions: usize,
    pub required_sessions: usize,
    pub test_sessions: usize,
    pub class_coverage_ok: bool,
    pub leakage_ok: bool,
    pub validation_ok: bool,
    pub leakage_detail: LeakageDetail,
    pub hard_examples: usize,
    pub entities: usize,
    /// The single most actionable unmet need (first failing gate).
    pub blocking_requirement: String,
    pub reasons: Vec<String>,
}

/// Pure readiness assessment over already-loaded rows + report +
/// eligibility: unit-testable without AppState. Hard gates are UNCHANGED
/// (1000 entity-linked RESULTs, 8 sessions, held-out test, coverage, zero
/// leakage, clean validation) — this function only reports more honestly.
pub fn assess_readiness(
    rows: &[crate::core::ml_dataset::MlAnnotation],
    report: &crate::core::ml_dataset::DatasetReport,
    eligibility: &std::collections::HashMap<String, crate::core::ml_dataset::StateEligibility>,
) -> TrainingReadiness {
    let verified = rows.iter().filter(|a| a.entity_id.is_some()).count();
    // Entity-linked RESULT rows: the actual gate currency (episode frames
    // never carry entities, so verified rows are RESULT by construction —
    // count them explicitly rather than assuming it).
    let entity_linked_result = rows
        .iter()
        .filter(|a| {
            a.entity_id.is_some()
                && a.game_state
                    == Some(crate::core::ml_dataset::GameStateLabel::CatchResult)
        })
        .count();
    let mut sessions: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut ui_classes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut entities: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut hard_examples = 0usize;
    for a in rows.iter().filter(|a| a.entity_id.is_some()) {
        sessions.insert(a.session_id.as_str());
        if let Some(u) = a.ui_label {
            ui_classes.insert(u.as_str().to_string());
        }
        if let Some(e) = &a.entity_id {
            entities.insert(e.as_str());
        }
    }
    for a in rows.iter() {
        if a.hard_example {
            hard_examples += 1;
        }
    }
    // Session-split attribution for coverage (same rule the checker uses).
    let split_of = |sess: &str| crate::core::ml_dataset::MlDatasetStore::split_of(sess);
    let test_sessions = sessions.iter().filter(|s| split_of(s) == crate::core::ml_dataset::Split::Test).count();
    let class_coverage_ok = ui_classes.len() >= 4 || entities.len() >= 5;
    let leakage_ok = report.leakage_sessions.is_empty() && report.same_file_cross_split.is_empty();
    let validation_ok = report.ok;
    let min_state_eligible = ["waiting_for_bite", "bite", "catch_result"]
        .iter()
        .map(|k| eligibility.get(*k).map(|e| e.eligible).unwrap_or(0))
        .min()
        .unwrap_or(0);

    let mut reasons = Vec::new();
    if entity_linked_result < READINESS_MIN_VERIFIED {
        reasons.push(format!("Insufficient entity-linked RESULT data: {entity_linked_result} linked, {READINESS_MIN_VERIFIED} required."));
    }
    if sessions.len() < READINESS_MIN_SESSIONS {
        reasons.push(format!("Session coverage too thin: {} verified sessions, {READINESS_MIN_SESSIONS} required.", sessions.len()));
    }
    if test_sessions == 0 {
        reasons.push("No held-out test sessions: the test split must stay untouched by training.".to_string());
    }
    if !class_coverage_ok {
        reasons.push(format!(
            "Class coverage too narrow: {} ui classes, {} entities (need 4 ui classes or 5 entities).",
            ui_classes.len(),
            entities.len()
        ));
    }
    if !leakage_ok {
        if !report.same_file_cross_split.is_empty() {
            reasons.push(format!(
                "TRUE event leakage: {} file(s) shared across splits ({}) — quarantine before training.",
                report.same_file_cross_split.len(),
                report.same_file_cross_split.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        if !report.leakage_sessions.is_empty() {
            reasons.push(format!("Dataset near-similarity across splits in {} hash group(s) — recurring static UI; splits cannot isolate constant backgrounds.", report.leakage_sessions.len()));
        }
    }
    // validation_ok is false when ANY check fails, including leakage (which
    // has its own reason above): only add the generic repair reason when a
    // non-leakage defect exists, so the report separates causes honestly.
    let has_row_defects = !report.corrupt_files.is_empty()
        || !report.invalid_entity_ids.is_empty()
        || !report.invalid_bboxes.is_empty()
        || !report.orphan_annotations.is_empty();
    if !validation_ok && has_row_defects {
        reasons.push("Dataset validation FAILs (corrupt/invalid/orphan rows) — run Validate dataset and repair.".to_string());
    }
    let ready = reasons.is_empty();
    // First failing gate, phrased as the actionable need.
    let blocking_requirement = if ready {
        "none — all gates pass".to_string()
    } else if entity_linked_result < READINESS_MIN_VERIFIED {
        format!(
            "{} more entity-linked RESULTs ({}/{})",
            READINESS_MIN_VERIFIED - entity_linked_result,
            entity_linked_result,
            READINESS_MIN_VERIFIED
        )
    } else if sessions.len() < READINESS_MIN_SESSIONS {
        format!("{} more verified sessions ({}/{})", READINESS_MIN_SESSIONS - sessions.len(), sessions.len(), READINESS_MIN_SESSIONS)
    } else if test_sessions == 0 {
        "held-out test sessions with verified rows".to_string()
    } else if !class_coverage_ok {
        "broader class/entity coverage (4 ui classes or 5 entities)".to_string()
    } else if !leakage_ok {
        "quarantined leakage groups (exact files and near-similar UI)".to_string()
    } else {
        "clean dataset validation (corrupt/invalid/orphan rows)".to_string()
    };
    TrainingReadiness {
        ready,
        training: if ready {
            "READY FOR TRAINING".to_string()
        } else {
            "TRAINING BLOCKED".to_string()
        },
        verified,
        required_verified: READINESS_MIN_VERIFIED,
        entity_linked_result,
        required_entity_linked: READINESS_MIN_VERIFIED,
        state_coverage: eligibility.clone(),
        min_state_eligible,
        sessions: sessions.len(),
        required_sessions: READINESS_MIN_SESSIONS,
        test_sessions,
        class_coverage_ok,
        leakage_ok,
        validation_ok,
        leakage_detail: LeakageDetail {
            exact_duplicate_files: report.same_file_cross_split.clone(),
            near_similarity_groups: report.leakage_sessions.len(),
            total_groups: report.leakage_sessions.len(),
        },
        hard_examples,
        entities: entities.len(),
        blocking_requirement,
        reasons,
    }
}

#[tauri::command]
pub fn ml_readiness(st: State<'_, AppState>) -> TrainingReadiness {
    let store = ml_store(&st);
    let report = store.validate(&st.store.effective_knowledge());
    let rows = store.annotations();
    let eligibility = store.state_eligibility();
    assess_readiness(&rows, &report, &eligibility)
}

#[derive(Debug, Serialize)]
pub struct MlModelStatus {
    pub available: bool,
    pub name: Option<String>,
    pub version: Option<String>,
    pub dataset: Option<String>,
    pub runtime: Option<String>,
    pub classes: Vec<String>,
    pub reason: Option<String>,
}

/// One named preflight probe for the collection start gate.
#[derive(Debug, Serialize)]
pub struct PreflightCheck {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// Start-up collection preflight. Verifies the versioned dataset layout can
/// be created, the images directory is writable, the PNG encoder works, and
/// the on-disk manifest (if any) matches the code's dataset version.
/// `ok` is true only when collection is consented (trace on) AND every
/// probe passes — the UI blocks Start otherwise instead of silently
/// recording zero frames.
#[derive(Debug, Serialize)]
pub struct MlPreflight {
    pub trace_on: bool,
    pub ok: bool,
    pub checks: Vec<PreflightCheck>,
}

#[tauri::command]
pub fn ml_preflight(st: State<'_, AppState>) -> MlPreflight {
    let trace_on = st.settings.read().fishing.trace;
    let mut checks = Vec::new();

    checks.push(PreflightCheck {
        name: "consent".to_string(),
        ok: trace_on,
        detail: if trace_on {
            "trace recording is ON".to_string()
        } else {
            "trace recording is OFF — enable it to record training frames".to_string()
        },
    });

    let store = ml_store(&st);
    match store.init() {
        Ok(()) => checks.push(PreflightCheck {
            name: "dataset_layout".to_string(),
            ok: true,
            detail: format!("{} ready", store.root().display()),
        }),
        Err(e) => checks.push(PreflightCheck {
            name: "dataset_layout".to_string(),
            ok: false,
            detail: format!("cannot create dataset layout: {e}"),
        }),
    }

    // Write-permission probe (temp file, removed immediately — never a sample).
    let probe = store.root().join("images").join(".write_probe");
    let write_ok = std::fs::write(&probe, b"probe")
        .and_then(|_| std::fs::remove_file(&probe))
        .is_ok();
    checks.push(PreflightCheck {
        name: "write_permission".to_string(),
        ok: write_ok,
        detail: if write_ok {
            "images/ is writable".to_string()
        } else {
            format!("cannot write to {}", probe.display())
        },
    });

    // Encoder smoke test (in-memory only — never written to the dataset).
    let frame = crate::core::types::Frame::new(16, 16, vec![128u8; 16 * 16 * 4]);
    match frame.to_png_bytes() {
        Ok(bytes) => checks.push(PreflightCheck {
            name: "frame_encoder".to_string(),
            ok: !bytes.is_empty(),
            detail: format!("PNG encoder produced {} bytes", bytes.len()),
        }),
        Err(e) => checks.push(PreflightCheck {
            name: "frame_encoder".to_string(),
            ok: false,
            detail: format!("PNG encoder failed: {e}"),
        }),
    }

    // Dataset version match (a stale manifest from a newer/older build
    // must not silently accept samples). A missing manifest is fine —
    // init() just created the layout; only a version MISMATCH fails.
    let (version_ok, version_detail) =
        match std::fs::read_to_string(store.root().join("manifest.json")) {
            Err(_) => (true, "no manifest yet (created on first run)".to_string()),
            Ok(raw) => match serde_json::from_str::<serde_json::Value>(&raw)
                .ok()
                .and_then(|v| v.get("version").and_then(|n| n.as_u64()))
            {
                Some(v) => {
                    let ok = v as u32 == crate::core::ml_dataset::DATASET_VERSION;
                    (
                        ok,
                        if ok {
                            format!("manifest v{v} matches")
                        } else {
                            format!(
                                "manifest v{v} ≠ code v{}",
                                crate::core::ml_dataset::DATASET_VERSION
                            )
                        },
                    )
                }
                None => (false, "manifest has no version".to_string()),
            },
        };
    checks.push(PreflightCheck {
        name: "dataset_version".to_string(),
        ok: version_ok,
        detail: version_detail,
    });

    let ok = trace_on && checks.iter().all(|c| c.ok);
    MlPreflight { trace_on, ok, checks }
}

/// Live collection health for the main-UI indicator and health panel.
/// `recording` is true ONLY when the user consented (trace on) AND a macro
/// session is active — a session shell alone (trace off) never reports
/// recording. All counters are real; zero is reported as zero.
#[derive(Debug, Serialize)]
pub struct MlCollectionStatus {
    pub trace_on: bool,
    pub session_active: bool,
    pub recording: bool,
    pub session_id: Option<String>,
    pub samples_written: u64,
    pub reels: u64,
    pub hard_examples: u64,
    pub dropped: u64,
    pub write_errors: u64,
    pub queue_depth: usize,
    pub last_capture_ms: Option<u64>,
    pub dataset: String,
    pub dataset_samples: usize,
    pub verified: usize,
}

#[tauri::command]
pub fn ml_collection_status(st: State<'_, AppState>) -> MlCollectionStatus {
    let trace_on = st.settings.read().fishing.trace;
    let ml = &st.bot.ctx().ml;
    let snap = ml.stats_snapshot();
    let session_active = snap.session_id.is_some();
    MlCollectionStatus {
        trace_on,
        session_active,
        recording: trace_on && session_active,
        session_id: snap.session_id,
        samples_written: snap.written,
        reels: snap.reels,
        hard_examples: snap.hard_examples,
        dropped: snap.dropped,
        write_errors: snap.write_errors,
        queue_depth: snap.queue_depth,
        last_capture_ms: if snap.last_write_ms == 0 { None } else { Some(snap.last_write_ms) },
        dataset: format!(
            "{}/v{}",
            crate::core::ml_dataset::DATASET_NAME,
            crate::core::ml_dataset::DATASET_VERSION
        ),
        dataset_samples: ml.dataset_samples(),
        verified: ml.dataset_verified(),
    }
}

#[tauri::command]
pub fn ml_model_status(st: State<'_, AppState>) -> MlModelStatus {
    let dir = st.store.dir().join("models");
    let p = crate::core::ml_model::GpoMlProvider::load(dir);
    MlModelStatus {
        available: p.available(),
        name: p.model_info().as_ref().map(|m| m.name.clone()),
        version: p.model_info().as_ref().map(|m| m.version.clone()),
        dataset: p.model_info().as_ref().map(|m| format!("{}/v{}", m.dataset, m.dataset_version)),
        runtime: p.model_info().as_ref().map(|m| m.runtime.clone()),
        classes: p.model_info().as_ref().map(|m| m.classes.clone()).unwrap_or_default(),
        reason: p.unavailable_reason().map(|s| s.to_string()),
    }
}

#[derive(Debug, Serialize)]
pub struct KnowledgeEntity {
    pub id: String,
    pub name: String,
    pub category: String,
}

/// Lightweight entity catalog for annotation dropdowns (ids + names only).
#[tauri::command]
pub fn knowledge_list(st: State<'_, AppState>) -> Vec<KnowledgeEntity> {
    let kb = st.store.effective_knowledge();
    let mut out: Vec<KnowledgeEntity> = kb
        .entities()
        .iter()
        .map(|e| KnowledgeEntity {
            id: e.id.clone(),
            name: e.canonical_name.clone(),
            category: e.category.as_str().to_string(),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[derive(Debug, Serialize)]
pub struct KnowledgeStats {
    pub version: u32,
    pub entities: usize,
    pub fruits: usize,
    pub fish: usize,
    pub bait: usize,
    pub ui_terms: usize,
    pub overlay: usize,
}

#[tauri::command]
pub fn knowledge_stats(st: State<'_, AppState>) -> KnowledgeStats {
    use crate::core::knowledge::EntityCategory;
    let kb = st.store.effective_knowledge();
    KnowledgeStats {
        version: kb.version,
        entities: kb.len(),
        fruits: kb.by_category(EntityCategory::Fruit).len(),
        fish: kb.by_category(EntityCategory::Fish).len(),
        bait: kb.by_category(EntityCategory::Bait).len(),
        ui_terms: kb.by_category(EntityCategory::UiTerm).len(),
        overlay: st.store.load_knowledge_overlay().len(),
    }
}

#[derive(Debug, Serialize)]
pub struct WikiSyncResult {
    pub category_titles: usize,
    pub fetched_pages: usize,
    pub truncated: bool,
    pub parsed: usize,
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// Opt-in GPO Wiki sync (explicit user action only). Fetches devil-fruit
/// pages sequentially via the public MediaWiki API, validates every record,
/// and merges with provenance — curated bundled entries are never silently
/// overwritten. Failed syncs never touch the existing overlay.
#[tauri::command]
pub async fn knowledge_sync_wiki(st: State<'_, AppState>) -> Result<WikiSyncResult, String> {
    use crate::core::wiki;
    let store = Arc::clone(&st.store);
    blocking(move || {
        let mut out = WikiSyncResult {
            category_titles: 0,
            fetched_pages: 0,
            truncated: false,
            parsed: 0,
            added: 0,
            updated: 0,
            skipped: 0,
            errors: Vec::new(),
        };
        let titles = wiki::fetch_category_titles(wiki::DEVIL_FRUIT_CATEGORY, 100)?;
        out.category_titles = titles.len();
        let mut fruits: Vec<_> = titles.into_iter().filter(|t| t.contains(" no Mi")).collect();
        fruits.sort();
        fruits.dedup();
        if fruits.len() > wiki::MAX_PAGES_PER_SYNC {
            out.truncated = true;
            fruits.truncate(wiki::MAX_PAGES_PER_SYNC);
        }
        let mut items = Vec::new();
        for title in &fruits {
            match wiki::fetch_page_wikitext(title) {
                Ok(Some(wikitext)) => {
                    out.fetched_pages += 1;
                    match wiki::parse_fruit_page(title, &wikitext) {
                        Some(entity) => {
                            out.parsed += 1;
                            items.push(entity);
                        }
                        None => {
                            out.skipped += 1;
                            out.errors.push(format!("'{title}': unparseable infobox"));
                        }
                    }
                }
                Ok(None) => {
                    out.skipped += 1;
                }
                Err(e) => {
                    out.skipped += 1;
                    out.errors.push(format!("'{title}': {e}"));
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        // Trial merge on a scratch copy first so the report reflects what
        // WILL happen; bundled entries are protected there and on every load.
        let mut scratch = store.effective_knowledge();
        let report = scratch.merge_import(&items, false);
        out.added = report.added;
        out.updated = report.updated;
        out.skipped += report.skipped;
        out.errors.extend(report.errors);
        // Persist: existing overlay with same ids replaced by fresh parses
        // (bundled protection re-applies on every load regardless).
        let mut overlay = store.load_knowledge_overlay();
        overlay.retain(|e| !items.iter().any(|n| n.id == e.id));
        overlay.extend(items);
        store.save_knowledge_overlay(&overlay).map_err(|e| e.to_string())?;
        Ok(out)
    })
    .await
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

fn encode_png(frame: &crate::core::types::Frame) -> Result<String, String> {
    use base64::Engine;
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut out,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::NoFilter,
    )
    .write_image(&frame.rgba, frame.w as u32, frame.h as u32, image::ExtendedColorType::Rgba8)
    .map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(out))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerAgeScanResult {
    pub success: bool,
    pub uptime_sec: i64,
    pub time_str: String,
    pub remaining_sec: i64,
    pub is_spawned: bool,
    pub message: String,
}

#[tauri::command]
pub async fn boss_tracker_scan_server_age(st: State<'_, AppState>) -> Result<ServerAgeScanResult, String> {
    let ctx = st.bot.ctx();
    let res = crate::bot::actions::scan_server_age(&ctx);
    match res {
        Ok((uptime_sec, time_str, remaining_sec, is_spawned)) => {
            let now = crate::core::boss_tracker::now_sec();
            let mut s = st.settings.read().clone();
            let offset = if is_spawned {
                now + remaining_sec + 1800
            } else {
                now + remaining_sec
            };
            s.boss_tracker.merchant_offset = Some(offset);
            let _ = st.store.save(&s);
            *st.settings.write() = s;

            let msg = if is_spawned {
                format!("Travelling Merchant is SPAWNED right now! (Despawns in {})", crate::core::boss_tracker::format_duration(remaining_sec))
            } else {
                format!("Travelling Merchant next spawn in {}", crate::core::boss_tracker::format_duration(remaining_sec))
            };

            Ok(ServerAgeScanResult {
                success: true,
                uptime_sec,
                time_str,
                remaining_sec,
                is_spawned,
                message: msg,
            })
        }
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub fn boss_tracker_sync_server_age(st: State<'_, AppState>, time_str: String) -> Result<ServerAgeScanResult, String> {
    let uptime_sec = crate::core::boss_tracker::parse_duration_str(&time_str)
        .ok_or_else(|| format!("Could not parse Server Age: '{}'", time_str))?;

    let (remaining_sec, is_spawned) = crate::core::boss_tracker::merchant_spawn_from_server_age(uptime_sec);
    let now = crate::core::boss_tracker::now_sec();
    let mut s = st.settings.read().clone();
    let offset = if is_spawned {
        now + remaining_sec + 1800
    } else {
        now + remaining_sec
    };
    s.boss_tracker.merchant_offset = Some(offset);
    let _ = st.store.save(&s);
    *st.settings.write() = s;

    let msg = if is_spawned {
        format!("Travelling Merchant is SPAWNED right now! (Despawns in {})", crate::core::boss_tracker::format_duration(remaining_sec))
    } else {
        format!("Travelling Merchant next spawn in {}", crate::core::boss_tracker::format_duration(remaining_sec))
    };

    Ok(ServerAgeScanResult {
        success: true,
        uptime_sec,
        time_str,
        remaining_sec,
        is_spawned,
        message: msg,
    })
}

#[tauri::command]
pub fn scan_bait_stock(st: State<'_, AppState>) -> Result<crate::core::bait::BaitStock, String> {
    let ctx = st.bot.ctx();
    crate::bot::actions::scan_bait_stock(&ctx)
}

#[tauri::command]
pub fn test_gemini(api_key: String, model: String) -> Result<String, String> {
    crate::core::gemini::test_gemini_connection(&api_key, &model)
}

#[tauri::command]
pub fn macro_list(st: State<'_, AppState>) -> Vec<crate::bot::recorder::CustomMacro> {
    crate::bot::recorder::load_macros(&st.store)
}

#[tauri::command]
pub fn macro_status() -> crate::bot::recorder::RecorderStatus {
    crate::bot::recorder::get_status()
}

#[tauri::command]
pub fn macro_record(
    st: State<'_, AppState>,
    action: String,
    name: Option<String>,
    mode: Option<String>,
) -> Result<serde_json::Value, String> {
    match action.as_str() {
        "start" => {
            // Shared contract with the frontend `RecordMode` ("pc" | "web");
            // backend enum spellings accepted case-insensitively for safety.
            let m = match mode.as_deref().map(|s| s.to_ascii_lowercase()) {
                Some(ref v) if v == "web" || v == "webscreen" => {
                    crate::bot::recorder::RecordMode::WebScreen
                }
                _ => crate::bot::recorder::RecordMode::PcWindow,
            };
            crate::bot::recorder::start_recording(m)?;
            Ok(serde_json::json!({ "ok": true, "message": "Recording started" }))
        }
        "stop" => {
            let m_name = name.unwrap_or_else(|| "My Macro".into());
            let m = crate::bot::recorder::stop_recording(&m_name, &st.store)?;
            Ok(serde_json::json!({ "ok": true, "message": format!("Saved macro '{}' ({} steps)", m.name, m.steps.len()), "macro": m }))
        }
        "cancel" => {
            crate::bot::recorder::cancel_recording();
            Ok(serde_json::json!({ "ok": true, "message": "Recording cancelled" }))
        }
        "toggle" => {
            let is_now_rec = crate::bot::recorder::toggle_recording()?;
            Ok(serde_json::json!({
                "ok": true,
                "is_recording": is_now_rec,
                "message": if is_now_rec { "Recording started" } else { "Recording stopped and saved" }
            }))
        }
        _ => Err("Unknown action".into()),
    }
}

#[tauri::command]
pub fn macro_play(
    st: State<'_, AppState>,
    action: String,
    name: Option<String>,
    loop_mode: Option<bool>,
    speed: Option<f32>,
    max_loops: Option<u32>,
) -> Result<serde_json::Value, String> {
    if action == "stop" {
        crate::bot::recorder::stop_playback();
        Ok(serde_json::json!({ "ok": true, "message": "Playback stopped" }))
    } else {
        let n = name.ok_or_else(|| "Macro name required".to_string())?;
        let is_loop = loop_mode.unwrap_or(false) || action == "loop";
        crate::bot::recorder::play_macro(st.bot.ctx().clone(), st.store.clone(), &n, is_loop, speed, max_loops)?;
        Ok(serde_json::json!({ "ok": true, "message": format!("Started playing '{n}'") }))
    }
}

#[tauri::command]
pub fn macro_append_vpn_step(
    st: State<'_, AppState>,
    name_or_id: String,
    action: crate::bot::recorder::VpnMacroAction,
    engine: Option<String>,
    timeout_s: Option<u64>,
    required: Option<bool>,
) -> Result<crate::bot::recorder::CustomMacro, String> {
    let m = crate::bot::recorder::append_vpn_step(&st.store, &name_or_id, action, engine, timeout_s, required)?;
    Ok(m)
}

#[tauri::command]
pub fn macro_rename(
    st: State<'_, AppState>,
    id_or_name: String,
    new_name: String,
) -> Result<serde_json::Value, String> {
    crate::bot::recorder::rename_macro(&st.store, &id_or_name, &new_name)?;
    Ok(serde_json::json!({ "ok": true, "message": format!("Renamed macro to '{new_name}'") }))
}

#[tauri::command]
pub fn macro_delete(st: State<'_, AppState>, name: String) -> Result<serde_json::Value, String> {
    crate::bot::recorder::delete_macro(&st.store, &name)?;
    Ok(serde_json::json!({ "ok": true, "message": format!("Deleted macro '{name}'") }))
}

/// Never block the async runtime on OS/network-bound status probes: a slow
/// `netsh`/CLI/API call must stall only its own blocking thread, never IPC.
#[tauri::command]
pub async fn vpn_get_status(st: State<'_, AppState>) -> Result<crate::vpn::VpnStatus, String> {
    let vpn = Arc::clone(&st.vpn);
    blocking(move || Ok(vpn.get_status())).await
}

#[tauri::command]
pub async fn vpn_connect(st: State<'_, AppState>, engine: String) -> Result<crate::vpn::VpnStatus, String> {
    let vpn = Arc::clone(&st.vpn);
    blocking(move || vpn.connect(&engine)).await
}

#[tauri::command]
pub async fn vpn_disconnect(st: State<'_, AppState>) -> Result<crate::vpn::VpnStatus, String> {
    let vpn = Arc::clone(&st.vpn);
    blocking(move || vpn.disconnect()).await
}

#[tauri::command]
pub async fn vpn_test_ping(st: State<'_, AppState>) -> Result<crate::vpn::PingResult, String> {
    let vpn = Arc::clone(&st.vpn);
    blocking(move || Ok(vpn.test_ping())).await
}

#[tauri::command]
pub async fn vpn_reset_network(st: State<'_, AppState>) -> Result<String, String> {
    let vpn = Arc::clone(&st.vpn);
    blocking(move || vpn.reset_network()).await
}

#[tauri::command]
pub async fn vpn_get_logs(st: State<'_, AppState>, max_lines: Option<usize>) -> Result<Vec<String>, String> {
    let vpn = Arc::clone(&st.vpn);
    blocking(move || Ok(vpn.get_logs(max_lines.unwrap_or(50)))).await
}

#[tauri::command]
pub fn vpn_set_auto_reconnect(st: State<'_, AppState>, enabled: bool) {
    st.vpn.set_auto_reconnect(enabled);
}

#[tauri::command]
pub async fn multi_roblox_get_status(st: State<'_, AppState>) -> Result<crate::multi_roblox::MultiRobloxStatus, String> {
    let mr = Arc::clone(&st.multi_roblox);
    blocking(move || Ok(mr.get_status())).await
}

#[tauri::command]
pub async fn multi_roblox_set_enabled(st: State<'_, AppState>, enabled: bool) -> Result<crate::multi_roblox::MultiRobloxStatus, String> {
    let mr = Arc::clone(&st.multi_roblox);
    blocking(move || mr.set_enabled(enabled)).await
}

#[tauri::command]
pub fn multi_roblox_list_instances(st: State<'_, AppState>) -> Vec<crate::multi_roblox::RobloxInstanceInfo> {
    st.multi_roblox.list_instances()
}

#[tauri::command]
pub fn multi_roblox_focus_instance(st: State<'_, AppState>, pid: u32) -> Result<(), String> {
    st.multi_roblox.focus_instance(pid)
}

#[tauri::command]
pub fn multi_roblox_kill_instance(st: State<'_, AppState>, pid: u32) -> Result<(), String> {
    st.multi_roblox.kill_instance(pid)
}

#[tauri::command]
pub async fn multi_roblox_kill_all(st: State<'_, AppState>) -> Result<usize, String> {
    let mr = Arc::clone(&st.multi_roblox);
    blocking(move || mr.kill_all()).await
}

#[tauri::command]
pub fn multi_roblox_set_target(st: State<'_, AppState>, pid: Option<u32>) {
    st.multi_roblox.set_target_pid(pid);
}

#[tauri::command]
pub fn multi_roblox_launch(app: AppHandle, place_id: Option<u64>) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let url = if let Some(pid) = place_id {
        format!("roblox://placeId={pid}")
    } else {
        "roblox://".to_string()
    };
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn multi_roblox_list_accounts(st: State<'_, AppState>) -> Result<Vec<crate::multi_roblox::SavedRobloxAccount>, String> {
    let mr = Arc::clone(&st.multi_roblox);
    blocking(move || Ok(mr.list_accounts())).await
}

#[tauri::command]
pub async fn multi_roblox_add_account(
    st: State<'_, AppState>,
    cookie: String,
    note: Option<String>,
) -> Result<crate::multi_roblox::SavedRobloxAccount, String> {
    let mr = Arc::clone(&st.multi_roblox);
    blocking(move || {
        mr.add_account(&cookie, note).map(|mut acc| {
            // The session cookie must never cross into the UI layer.
            acc.cookie.clear();
            acc
        })
    })
    .await
}

#[tauri::command]
pub fn multi_roblox_remove_account(st: State<'_, AppState>, id: String) -> Result<(), String> {
    st.multi_roblox.remove_account(&id)
}

#[tauri::command]
pub async fn multi_roblox_launch_account(
    st: State<'_, AppState>,
    id: String,
    place_id: Option<u64>,
) -> Result<(), String> {
    let mr = Arc::clone(&st.multi_roblox);
    blocking(move || mr.launch_account(&id, place_id)).await
}

#[tauri::command]
pub fn laptop_keyboard_light_get() -> crate::laptop_light::KeyboardLightStatus {
    crate::laptop_light::get_keyboard_light_status()
}

#[tauri::command]
pub async fn laptop_keyboard_light_set(action: String) -> Result<crate::laptop_light::KeyboardLightStatus, String> {
    blocking(move || Ok(crate::laptop_light::set_keyboard_light(&action))).await
}

#[tauri::command]
pub async fn laptop_keyboard_light_setup() -> Result<String, String> {
    blocking(crate::laptop_light::setup_keyboard_light_task).await
}

#[tauri::command]
pub fn laptop_fan_get() -> crate::laptop_fan::FanStatus {
    crate::laptop_fan::get_fan_status()
}

#[tauri::command]
pub async fn laptop_fan_set(mode: String) -> Result<crate::laptop_fan::FanStatus, String> {
    blocking(move || Ok(crate::laptop_fan::set_fan_mode(&mode))).await
}

#[tauri::command]
pub async fn laptop_fan_set_auto_turbo(enabled: bool) -> Result<crate::laptop_fan::FanStatus, String> {
    blocking(move || Ok(crate::laptop_fan::set_auto_turbo(enabled))).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ml_dataset::{
        GameStateLabel, MlAnnotation, MlDatasetStore, MlTask, Split, UiLabel, DATASET_VERSION,
    };

    fn tmp_store(tag: &str) -> (MlDatasetStore, std::path::PathBuf) {
        static C: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = C.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("gpo-ready-{tag}-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        (MlDatasetStore::new(dir.clone()), dir)
    }

    fn row(
        image_id: &str,
        session: &str,
        ts: u64,
        state: Option<GameStateLabel>,
        entity: Option<&str>,
    ) -> MlAnnotation {
        MlAnnotation {
            image_id: image_id.into(),
            dataset_version: DATASET_VERSION,
            session_id: session.into(),
            task: MlTask::UiDetection,
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

    fn write_rows(ds: &MlDatasetStore, rows: &[MlAnnotation]) {
        ds.init().unwrap();
        let body = rows
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(ds.root().join("labels.jsonl"), body).unwrap();
    }

    fn assess(ds: &MlDatasetStore) -> TrainingReadiness {
        let kb = crate::core::knowledge::KnowledgeBase::bundled();
        let report = ds.validate(kb);
        let rows = ds.annotations();
        let elig = ds.state_eligibility();
        assess_readiness(&rows, &report, &elig)
    }

    #[test]
    fn episode_frames_never_count_as_entity_linked() {
        // WAITING/BITE carry no entities by design: the entity gate must
        // measure RESULT linkage only, never episode volume.
        let (ds, _d) = tmp_store("sem");
        write_rows(
            &ds,
            &[
                row("aabbccddeeff0011", "s1", 1000, Some(GameStateLabel::WaitingForBite), None),
                row("aabbccddeeff0022", "s1", 2000, Some(GameStateLabel::Bite), None),
                row(
                    "aabbccddeeff0033",
                    "s2",
                    3000,
                    Some(GameStateLabel::CatchResult),
                    Some("fruit:suna"),
                ),
            ],
        );
        let r = assess(&ds);
        assert_eq!(r.entity_linked_result, 1);
        assert_eq!(r.verified, 1);
        assert_eq!(r.state_coverage["waiting_for_bite"].total, 1);
        assert_eq!(r.state_coverage["bite"].total, 1);
        assert_eq!(r.state_coverage["catch_result"].total, 1);
        assert!(!r.ready, "3 rows must stay BLOCKED");
        assert!(r.blocking_requirement.contains("entity-linked RESULT"));
        // Honest terminology is part of the wire contract.
        let v = serde_json::to_value(&r).unwrap();
        for key in ["entity_linked_result", "state_coverage", "blocking_requirement", "leakage_detail"] {
            assert!(v.get(key).is_some(), "readiness must expose {key}");
        }
    }

    #[test]
    fn thousand_orphans_stay_blocked() {
        // 1000 entity-linked rows with NO images: the count gate passes but
        // validation (orphans) must still block. Count never bypasses gates.
        let (ds, _d) = tmp_store("orphan");
        let mut sess: Vec<String> = Vec::new();
        for i in 0..60 {
            let c = format!("sess-bulk-{i:03}");
            match MlDatasetStore::split_of(&c) {
                Split::Test if !sess.iter().any(|s| MlDatasetStore::split_of(s) == Split::Test) => {
                    sess.push(c)
                }
                Split::Train if sess.len() < 11 => sess.push(c),
                _ => {}
            }
            if sess.len() >= 12 {
                break;
            }
        }
        assert!(sess.len() >= 10, "need session spread, got {}", sess.len());
        let ents = ["fruit:suna", "fish:shark", "fish:squid", "fish:gold", "fruit:yami", "fish:snap"];
        let rows: Vec<MlAnnotation> = (0..1000)
            .map(|i| {
                let mut r = row(
                    &format!("ghost{i:016x}"),
                    &sess[i % sess.len()],
                    1000 + i as u64,
                    Some(GameStateLabel::CatchResult),
                    Some(ents[i % ents.len()]),
                );
                r.image_id = format!("ghost{i:016x}");
                r
            })
            .collect();
        write_rows(&ds, &rows);
        let r = assess(&ds);
        assert_eq!(r.entity_linked_result, 1000);
        assert!(!r.ready, "orphan rows must block despite the count");
        assert!(
            r.reasons.iter().any(|s| s.contains("FAIL") || s.contains("orphan") || s.contains("validation")),
            "validation must be the blocker, got: {:?}",
            r.reasons
        );
        assert!(r.blocking_requirement.contains("validation"));
    }

    #[test]
    fn full_gate_passes_with_clean_volume() {
        // 1000 real PNGs across 10 sessions with 6 entities and all splits:
        // proves READY is reachable (guards against gate-rot making READY
        // impossible). Deterministic pseudo-random pixels.
        let (ds, _d) = tmp_store("ready");
        let mut train = Vec::new();
        let mut rest = Vec::new();
        for i in 0..60 {
            let c = format!("sess-full-{i:03}");
            match MlDatasetStore::split_of(&c) {
                Split::Train => train.push(c),
                _ => rest.push(c),
            }
            if train.len() >= 8 && rest.iter().any(|s| MlDatasetStore::split_of(s) == Split::Test) && rest.len() >= 2 {
                break;
            }
        }
        assert!(train.len() >= 8 && rest.len() >= 2);
        let mut sess = train;
        sess.extend(rest.into_iter().take(2));
        assert_eq!(sess.len(), 10);
        let kb = crate::core::knowledge::KnowledgeBase::bundled();
        let kb_ids: Vec<String> = kb.entities().iter().take(6).map(|e| e.id.clone()).collect();
        assert!(kb_ids.len() >= 6, "bundled KB must offer 6 entities");
        for i in 0..1000 {
            let mut rgba = vec![0u8; 16 * 16 * 4];
            let mut x = (i as u32).wrapping_mul(2654435761).wrapping_add(0x9E3779B9);
            for px in rgba.chunks_exact_mut(4) {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                px[0] = (x & 0xFF) as u8;
                px[1] = ((x >> 8) & 0xFF) as u8;
                px[2] = ((x >> 16) & 0xFF) as u8;
                px[3] = 255;
            }
            let f = crate::core::types::Frame::new(16, 16, rgba);
            let png = f.to_png_bytes().unwrap();
            let id = ds
                .import_png(&png, &sess[i % sess.len()], MlTask::UiDetection, None, "", "bar", "t")
                .unwrap();
            ds.annotate(
                &id,
                Some(UiLabel::FishingBar),
                None,
                Some(GameStateLabel::CatchResult),
                Some(kb_ids[i % kb_ids.len()].clone()),
                "test",
                false,
                None,
            )
            .unwrap();
        }
        let r = assess(&ds);
        assert_eq!(r.entity_linked_result, 1000);
        assert!(r.ready, "clean volume must pass, got: {:?}", r.reasons);
        assert_eq!(r.training, "READY FOR TRAINING");
        assert_eq!(r.blocking_requirement, "none — all gates pass");
    }
}
