use std::time::Duration;

use crate::core::types::{Key, MouseButton, PxPoint, PxRect, RelPoint, RelRect};
use crate::events::BotState;

use super::ctx::Ctx;

#[path = "../core/rod_state.rs"]
pub(crate) mod rod_state;

const WHEEL_STEP: i32 = 120;

fn rod_trackers() -> &'static parking_lot::Mutex<std::collections::HashMap<(std::thread::ThreadId, usize), rod_state::RodTracker>> {
    static TRACKERS: std::sync::OnceLock<parking_lot::Mutex<std::collections::HashMap<(std::thread::ThreadId, usize), rod_state::RodTracker>>> = std::sync::OnceLock::new();
    TRACKERS.get_or_init(Default::default)
}

fn rod_context_key(ctx: &Ctx) -> (std::thread::ThreadId, usize) {
    (std::thread::current().id(), ctx as *const Ctx as usize)
}

pub fn reset_rod_evidence(ctx: &Ctx) {
    rod_trackers().lock().remove(&rod_context_key(ctx));
}

fn learn_rod_outline(ctx: &Ctx) {
    if !rod_action_allowed(ctx) { return; }
    let Some(client)=ctx.roblox_rect() else { return; };
    let band=PxRect{x:client.x,y:client.y+client.h*9/10,w:client.w,h:client.h-client.h*9/10};
    let Ok(frame)=ctx.platform.capture.grab(band) else { return; };
    if !rod_action_allowed(ctx) { return; }
    let min=(client.h as usize*45/1000).max(12);
    let max=(client.h as usize*80/1000).max(min);
    if let Some(rect)=rod_state::find_outline(frame.w,frame.h,min,max,|x,y|frame.px(x,y)) {
        let mut trackers=rod_trackers().lock();
        let tracker=trackers.entry(rod_context_key(ctx)).or_default();
        tracker.learned=Some((rect,client.w as usize,client.h as usize,ctx.settings.read().keys.rod));
        tracker.recovery_attempts=0;
        tracker.last_recovery=None;
    }
}

fn learned_rod_selected(ctx:&Ctx)->bool {
    let learned=rod_trackers().lock().get(&rod_context_key(ctx)).and_then(|t|t.learned);
    let Some((r,w,h,key))=learned else { return false; };
    let Some(client)=ctx.roblox_rect() else { return false; };
    if (w,h,key)!=(client.w as usize,client.h as usize,ctx.settings.read().keys.rod) { return false; }
    let rect=PxRect{x:client.x+r.x as i32,y:client.y+client.h*9/10+r.y as i32,w:r.w as i32,h:r.h as i32};
    let Ok(frame)=ctx.platform.capture.grab(rect) else { return false; };
    if frame.w!=r.w || frame.h!=r.h || !rod_action_allowed(ctx) { return false; }
    rod_state::outlined(rod_state::SlotRect{x:0,y:0,w:r.w,h:r.h},&|x,y|frame.px(x,y))
}

/// Positive selection of another learned toolbar slot, not a missing border.
pub fn rod_visibly_unequipped(ctx:&Ctx)->bool {
    if !rod_action_allowed(ctx) { return false; }
    let learned=rod_trackers().lock().get(&rod_context_key(ctx)).and_then(|t|t.learned);
    let Some((rod,w,h,key))=learned else { return false; };
    let Some(client)=ctx.roblox_rect() else { return false; };
    if (w,h,key)!=(client.w as usize,client.h as usize,ctx.settings.read().keys.rod) { return false; }
    let band=PxRect{x:client.x,y:client.y+client.h*9/10,w:client.w,h:client.h-client.h*9/10};
    let Ok(frame)=ctx.platform.capture.grab(band) else { return false; };
    if !rod_action_allowed(ctx) { return false; }
    let min=(client.h as usize*45/1000).max(12);
    let max=(client.h as usize*80/1000).max(min);
    let selected=rod_state::find_outline(frame.w,frame.h,min,max,|x,y|frame.px(x,y));
    rod_state::different_selected_slot(rod,selected)
}

fn rod_action_allowed(ctx: &Ctx) -> bool {
    ctx.alive() && ctx.state() != BotState::Paused && ctx.roblox_in_front()
}

/// Local-only positive menu evidence; absence is deliberately unknown. Checks
/// cancellation again after capture/OCR so a late result cannot trigger input.
fn observed_bait_region(ctx: &Ctx) -> Option<RelRect> {
    rod_trackers().lock().get(&rod_context_key(ctx)).and_then(|t| t.menu_region)
        .map(|(x,y,w,h)|RelRect{x,y,w,h})
}

pub fn bait_menu_open(ctx: &Ctx) -> bool {
    if !rod_action_allowed(ctx) { return false; }
    let Some(client) = ctx.roblox_rect() else { return false; };
    let configured=ctx.settings.read().regions.bait_menu;
    // GPO's centered menu is a bounded fallback, not positive evidence itself.
    // Identity still requires the title/two distinct rows or local recognition.
    let centered=RelRect{x:0.43,y:0.704,w:0.14,h:0.142};
    let candidates=[observed_bait_region(ctx).unwrap_or(configured),centered];
    for rel in candidates {
        let region=rel.to_px(&client);
        if region.w < 20 || region.h < 20 { continue; }
        let Ok(frame) = ctx.platform.capture.grab(region) else { continue; };
        if !rod_action_allowed(ctx) { return false; }
        let neural=crate::core::bait::scan_bait_stock_neural(&frame).is_some_and(|stock|
            [stock.legendary,stock.rare,stock.common].iter().filter(|n|n.unwrap_or(0)>0).count()>=2);
        let mut visible=neural;
        if !visible && ctx.platform.ocr.available() {
            if let Ok(text)=ctx.platform.ocr.read_with_timeout(&frame,Duration::from_millis(120)) {
                visible=rod_state::bait_menu_text(&text);
            }
        }
        if !rod_action_allowed(ctx) { return false; }
        if visible {
            rod_trackers().lock().entry(rod_context_key(ctx)).or_default().menu_region=Some((rel.x,rel.y,rel.w,rel.h));
            return true;
        }
    }
    false
}

pub fn fishing_point(ctx: &Ctx) -> Option<PxPoint> {
    let rect = ctx.roblox_rect()?;
    let s = ctx.settings.read();
    let rel = if s.features.auto_mouse_position { RelPoint { x: 0.5, y: 0.33 } } else { s.points.fishing };
    Some(rel.to_px(&rect))
}

fn rel_to_px(ctx: &Ctx, p: RelPoint) -> Option<PxPoint> {
    ctx.roblox_rect().map(|r| p.to_px(&r))
}

pub fn click(ctx: &Ctx, p: PxPoint) -> bool {
    if !ctx.alive() || !ctx.roblox_in_front() { ctx.release_mouse(); return false; }
    let input = &ctx.platform.input;
    input.move_to(p);
    if !ctx.sleep_ms(35) {
        return false;
    }
    if !ctx.alive() || !ctx.roblox_in_front() { return false; }
    input.button(MouseButton::Left, true);
    if !ctx.sleep_ms(45) {
        input.button(MouseButton::Left, false);
        return false;
    }
    input.button(MouseButton::Left, false);
    true
}

/// A rock-solid, deliberate click designed specifically for Roblox UI dialogs and shop prompts.
/// Guarantees that mouse down/up states are registered by Roblox across multiple game engine ticks.
pub fn ui_click(ctx: &Ctx, p: PxPoint) -> bool {
    if !ctx.alive() || !ctx.roblox_in_front() { ctx.release_mouse(); return false; }
    let input = &ctx.platform.input;
    input.move_to(p);
    if !ctx.sleep_ms(60) {
        return false;
    }
    if !ctx.alive() || !ctx.roblox_in_front() { return false; }
    input.button(MouseButton::Left, true);
    if !ctx.sleep_ms(70) {
        input.button(MouseButton::Left, false);
        return false;
    }
    input.button(MouseButton::Left, false);
    ctx.sleep_ms(80)
}

pub fn right_click(ctx: &Ctx, p: PxPoint) -> bool {
    if !ctx.alive() || !ctx.roblox_in_front() { ctx.release_mouse(); return false; }
    let input = &ctx.platform.input;
    input.move_to(p);
    if !ctx.sleep_ms(40) {
        return false;
    }
    if !ctx.alive() || !ctx.roblox_in_front() { return false; }
    input.button(MouseButton::Right, true);
    if !ctx.sleep_ms(40) {
        input.button(MouseButton::Right, false);
        return false;
    }
    input.button(MouseButton::Right, false);
    true
}

pub fn key_tap(ctx: &Ctx, k: Key) -> bool {
    if !ctx.alive() || !ctx.roblox_in_front() { ctx.release_mouse(); return false; }
    ctx.platform.input.key(k, true);
    let ok = ctx.sleep_ms(30);
    ctx.platform.input.key(k, false);
    ok
}

pub fn key_hold(ctx: &Ctx, k: Key, d: Duration) -> bool {
    if !ctx.alive() || !ctx.roblox_in_front() { ctx.release_mouse(); return false; }
    ctx.platform.input.key(k, true);
    let ok = ctx.sleep(d);
    ctx.platform.input.key(k, false);
    ok
}

pub fn align_camera_shift_lock(ctx: &Ctx) -> bool {
    ctx.log_info("🔄 Snapping character orientation with Shift Lock");
    if !key_tap(ctx, Key::Shift) {
        return false;
    }
    if !ctx.sleep_ms(100) {
        return false;
    }
    if !key_tap(ctx, Key::Shift) {
        return false;
    }
    ctx.sleep_ms(60)
}

pub fn wheel_steps(ctx: &Ctx, steps: u32, dir: i32, step_delay_ms: u32) -> bool {
    for _ in 0..steps {
        if !ctx.alive() || !ctx.roblox_in_front() { return false; }
        ctx.platform.input.wheel(WHEEL_STEP * dir);
        if !ctx.sleep_ms(step_delay_ms) {
            return false;
        }
    }
    true
}

fn click_pair(ctx: &Ctx, primary: RelPoint, backup: Option<RelPoint>, settle_ms: u32) -> bool {
    let Some(p) = rel_to_px(ctx, primary) else { return false };
    if !click(ctx, p) || !ctx.sleep_ms(settle_ms) {
        return false;
    }
    if let Some(b) = backup.and_then(|b| rel_to_px(ctx, b)) {
        if !click(ctx, b) || !ctx.sleep_ms(settle_ms) {
            return false;
        }
        if !click(ctx, p) || !ctx.sleep_ms(settle_ms) {
            return false;
        }
    }
    true
}

pub fn is_rod_equipped(ctx: &Ctx) -> Option<bool> {
    if !rod_action_allowed(ctx) { return None; }
    if learned_rod_selected(ctx) { return Some(true); }
    if bait_menu_open(ctx) {
        learn_rod_outline(ctx);
        return if rod_action_allowed(ctx) { Some(true) } else { None };
    }
    let s = ctx.settings();
    let rect = ctx.roblox_rect()?;

    // A configured indicator identifies the rod slot, unlike arbitrary hotbar pixels.
    // 1. If custom rod slot indicator point is set, inspect around it:
    if let Some(rel) = s.points.rod_slot {
        let px = rel.to_px(&rect);
        if rect.w < 5 || rect.h < 5 { return None; }
        let patch = PxRect {
            x: (px.x - 2).clamp(rect.x, rect.x.saturating_add(rect.w - 5)),
            y: (px.y - 2).clamp(rect.y, rect.y.saturating_add(rect.h - 5)),
            w: 5,
            h: 5,
        };
        if let Ok(frame) = ctx.platform.capture.grab(patch) {
            let mut bright = 0;
            for y in 0..frame.h {
                for x in 0..frame.w {
                    let (r, g, b) = frame.px(x, y);
                    if r > 180 && g > 180 && b > 180 {
                        bright += 1;
                    }
                }
            }
            if !rod_action_allowed(ctx) { return None; }
            return if bright >= 3 { Some(true) } else { None };
        }
    }

    // An uncalibrated bright line cannot identify the configured rod slot:
    // item text, other hotbar slots and ability icons produce the same pixels.
    // Retain the state of our own validated input instead of guessing slot 1.

    None
}

pub fn ensure_rod_equipped(ctx: &Ctx, rod_equipped: &mut bool) -> bool {
    if !rod_action_allowed(ctx) { return false; }
    if is_rod_equipped(ctx) == Some(true) && rod_action_allowed(ctx) {
        *rod_equipped = true;
        let mut trackers = rod_trackers().lock();
        let tracker = trackers.entry(rod_context_key(ctx)).or_default();
        tracker.recovery_attempts = 0;
        tracker.last_recovery = None;
        return true;
    }
    // A verified rod stays valid across normal reels unless a *different*
    // selected slot is positively observed. Missing/OCR-obscured UI is unknown.
    if *rod_equipped && !rod_visibly_unequipped(ctx) && rod_action_allowed(ctx) {
        return true;
    }
    *rod_equipped = false;
    let s = ctx.settings();
    if s.keys.reset_slot == s.keys.rod {
        ctx.release_mouse();
        ctx.log_warn("Rod slot and reset slot are identical; cannot safely recover");
        ctx.set_state(BotState::Paused, Some("Configure different rod and reset slots".into()));
        return false;
    }
    let attempt = {
        let mut trackers = rod_trackers().lock();
        let tracker = trackers.entry(rod_context_key(ctx)).or_default();
        if tracker.recovery_attempts >= 3 { None }
        else if tracker.last_recovery.is_some_and(|t| t.elapsed() < Duration::from_millis(800)) { Some(false) }
        else {
            tracker.recovery_attempts += 1;
            tracker.last_recovery = Some(std::time::Instant::now());
            Some(true)
        }
    };
    if attempt.is_none() {
        ctx.release_mouse();
        ctx.log_warn("Rod still unverified after three recovery attempts; pausing instead of recovering forever");
        ctx.set_state(BotState::Paused, Some("Rod cannot be verified; check rod hotbar slot, bait menu and Roblox focus".into()));
        return false;
    }
    if attempt == Some(true) {
        ctx.log_info("Rod not confirmed; selecting reset slot then rod and checking locally");
        if !key_tap(ctx, Key::Char(s.keys.reset_slot)) || !ctx.sleep_ms(80) { return false; }
        if !rod_action_allowed(ctx) || !key_tap(ctx, Key::Char(s.keys.rod)) { return false; }
        for _ in 0..3 {
            if !ctx.sleep_ms(180) || !rod_action_allowed(ctx) { return false; }
            if is_rod_equipped(ctx) == Some(true) && rod_action_allowed(ctx) {
                *rod_equipped = true;
                let mut trackers = rod_trackers().lock();
                let tracker = trackers.entry(rod_context_key(ctx)).or_default();
                tracker.recovery_attempts = 0;
                tracker.last_recovery = None;
                return true;
            }
        }
    }
    ctx.release_mouse();
    if rod_action_allowed(ctx) {
        ctx.set_state(BotState::Recovering, Some("Checking rod selection; bounded retry, then pause if still unverified".into()));
    }
    false
}

pub fn equip_rod(ctx: &Ctx) -> bool {
    let mut dummy = false;
    ensure_rod_equipped(ctx, &mut dummy)
}

pub fn is_bait_depleted(ctx: &Ctx) -> bool {
    let s = ctx.settings();
    let Some(primary) = s.points.bait[0] else {
        return false;
    };
    let Some(rect) = ctx.roblox_rect() else {
        return false;
    };
    let pt = primary.to_px(&rect);
    let scan_rect = PxRect {
        x: (pt.x - 25).max(rect.x),
        y: (pt.y - 25).max(rect.y),
        w: 50.min(rect.w),
        h: 50.min(rect.h),
    };
    if ctx.platform.ocr.available() {
        if let Ok(frame) = ctx.platform.capture.grab(scan_rect) {
            let text = ctx.platform.ocr.read(&frame).unwrap_or_default().to_lowercase();
            let trimmed = text.trim();
            if trimmed == "x0" || trimmed == "0" || trimmed == "x 0" || trimmed.ends_with(" 0") {
                return true;
            }
        }
    }
    false
}

/// Scans the in-game bait menu using the trained neural network (with Windows OCR fallback).
/// Returns: (BaitStock, is_menu_visible)
pub fn scan_bait_stock_raw(ctx: &Ctx) -> Result<(crate::core::bait::BaitStock, bool), String> {
    scan_bait_stock_for(ctx, false)
}

fn scan_bait_stock_for(ctx: &Ctx, automatic: bool) -> Result<(crate::core::bait::BaitStock, bool), String> {
    if automatic && !ctx.alive() { return Err("Bait scan cancelled".into()); }
    let rect = ctx.roblox_rect().ok_or("Roblox window not found")?;
    let scan_rect = {
        let s = ctx.settings();
        if automatic { observed_bait_region(ctx).unwrap_or(s.regions.bait_menu).to_px(&rect) }
        else { s.regions.bait_menu.to_px(&rect) }
    };

    if scan_rect.w < 10 || scan_rect.h < 10 {
        return Err("Bait menu region is too small or not configured".into());
    }

    let frame = ctx
        .platform
        .capture
        .grab(scan_rect)
        .map_err(|e| format!("Screen capture failed: {e}"))?;

    // Prefer the local bait model; no network request belongs in this loop.
    if let Some(stock) = crate::core::bait::scan_bait_stock_neural(&frame) {
        ctx.log_info(&format!(
            "🐟 Bait stock recognized via neural model: Leg={:?}, Rare={:?}, Com={:?}",
            stock.legendary, stock.rare, stock.common
        ));
        return Ok((stock, true));
    }

    // Automatic fishing must not wait on a network vision service.
    // A missing menu is normal after a cast, not a reason to call Gemini.
    if automatic && !ctx.alive() { return Err("Bait scan cancelled".into()); }

    let s = ctx.settings();
    // 2. Try Gemini Vision if enabled and configured (fallback)
    if !automatic && s.gemini.enabled && !s.gemini.api_key.trim().is_empty() {
        match crate::core::gemini::scan_bait_stock_gemini(&frame, &s.gemini.api_key, &s.gemini.model) {
            Ok(stock) => {
                ctx.log_info(&format!(
                    "🐟 Bait stock recognized via Gemini Vision ({}): Leg={:?}, Rare={:?}, Com={:?}",
                    s.gemini.model, stock.legendary, stock.rare, stock.common
                ));
                return Ok((stock, true));
            }
            Err(e) => {
                ctx.log_warn(&format!("Gemini Vision scan failed ({e}); falling back to Windows OCR"));
            }
        }
    }

    // 3. Fallback to Windows OCR if available
    if ctx.platform.ocr.available() {
        if let Ok(text) = ctx.platform.ocr.read_with_timeout(&frame, std::time::Duration::from_millis(200)) {
            if automatic && !ctx.alive() { return Err("Bait scan cancelled".into()); }
            ctx.log_debug(&format!("Bait menu OCR raw text:\n{text}"));
            let is_visible = crate::core::bait::is_bait_menu_visible(&text);
            let stock = crate::core::bait::parse_bait_stock_with_frame(&text, &frame);
            return Ok((stock, is_visible));
        }
    }

    Ok((crate::core::bait::BaitStock::default(), false))
}

pub fn scan_bait_stock(ctx: &Ctx) -> Result<crate::core::bait::BaitStock, String> {
    scan_bait_stock_raw(ctx).map(|(s, _)| s)
}

pub fn select_bait(ctx: &Ctx) -> bool {
    let s = ctx.settings();
    if !s.features.auto_bait {
        return true;
    }

    if s.features.smart_bait {
        let (stock, menu_visible) = match scan_bait_stock_for(ctx, true) {
            Ok(res) => res,
            Err(e) => {
                ctx.log_debug(&format!("Smart Bait: scan skipped: {e}"));
                return true;
            }
        };

        if !ctx.alive() { return false; }
        if !menu_visible {
            // Menu is not open on screen. If rod is already in hand from previous cast, it is already baited!
            ctx.log_debug("Smart Bait: bait menu not visible (rod already baited); skipping selection.");
            return true;
        }

        let leg_str = stock.legendary.map(|n| n.to_string()).unwrap_or_else(|| "?".into());
        let rare_str = stock.rare.map(|n| n.to_string()).unwrap_or_else(|| "?".into());
        let com_str = stock.common.map(|n| n.to_string()).unwrap_or_else(|| "?".into());
        ctx.log_info(&format!("🐟 Bait Stock: [Legendary: {leg_str} | Rare: {rare_str} | Common: {com_str}]"));

        // Dynamic auto-purchase trigger:
        // In GPO, ONLY Common Fish Bait can be purchased from the NPC shop.
        // Calculates the exact missing amount to reach max capacity (300).
        if s.features.auto_purchase {
            if let Some(c_qty) = stock.common {
                if c_qty <= s.purchase.low_bait_threshold {
                    let max_cap = s.purchase.max_bait.clamp(1, 300);
                    let to_buy = max_cap.saturating_sub(c_qty).clamp(1, max_cap);
                    ctx.log_warn(&format!(
                        "🛒 Common bait stock ({c_qty}/{max_cap}) <= threshold ({})! Purchasing exact missing {to_buy} bait...",
                        s.purchase.low_bait_threshold
                    ));
                    if !purchase_amount(ctx, Some(to_buy)) {
                        ctx.log_warn("Auto purchase failed; continuing with available bait.");
                    } else {
                        let mut rod_eq = false;
                        if !ensure_rod_equipped(ctx, &mut rod_eq) {
                            return false;
                        }
                        if !ctx.sleep_ms(350) {
                            return false;
                        }
                        return select_bait(ctx);
                    }
                }
            }
        }

        let chosen_tier = crate::core::bait::resolve_tier(&stock, s.purchase.bait_tier, s.purchase.legendary_reserve);

        // Notify user if a high tier bait depleted or reached reserve limit
        if (s.purchase.bait_tier == crate::core::bait::BaitTier::Legendary || s.purchase.bait_tier == crate::core::bait::BaitTier::Highest)
            && chosen_tier != crate::core::bait::BaitTier::Legendary
        {
            if let Some(l_qty) = stock.legendary {
                if s.purchase.legendary_reserve > 0 && l_qty <= s.purchase.legendary_reserve {
                    ctx.log_warn(&format!(
                        "🛡️ Legendary bait ({l_qty}) reached reserve limit ({})! Protecting remaining Legendary bait and using {:?}.",
                        s.purchase.legendary_reserve, chosen_tier
                    ));
                } else if l_qty == 0 {
                    ctx.log_warn(&format!("⚠️ Legendary bait depleted! Automatically fell back to {:?}.", chosen_tier));
                }
            }
        } else if s.purchase.bait_tier == crate::core::bait::BaitTier::Rare && chosen_tier == crate::core::bait::BaitTier::Common && stock.rare == Some(0) {
            ctx.log_warn("⚠️ Rare bait depleted! Automatically fell back to Common bait.");
        }

        // Check failsafe: if chosen tier has 0 stock while menu is visibly open
        if s.features.zero_bait_failsafe {
            let is_zero = match chosen_tier {
                crate::core::bait::BaitTier::Legendary => stock.legendary == Some(0),
                crate::core::bait::BaitTier::Rare => stock.rare == Some(0),
                crate::core::bait::BaitTier::Common => stock.common == Some(0),
                crate::core::bait::BaitTier::Highest => {
                    stock.legendary == Some(0) && stock.rare == Some(0) && stock.common == Some(0)
                }
            };
            if is_zero {
                ctx.set_state(BotState::Paused, Some("Bait depleted".into()));
                ctx.log_warn("🎣 Bait depleted (zero bait detected)! Safely pausing macro.");
                ctx.webhook.bait_depleted();
                return false;
            }
        }

        let Some(rect) = ctx.roblox_rect() else {
            ctx.log_warn("Roblox window not found; cannot select bait");
            return false;
        };

        let menu = observed_bait_region(ctx).unwrap_or(s.regions.bait_menu);
        let (row_rx, row_ry) = stock.click_relative_pos(chosen_tier);
        let target_rel = RelPoint {
            x: menu.x + menu.w * row_rx,
            y: menu.y + menu.h * row_ry,
        };
        let target_px = target_rel.to_px(&rect);

        ctx.log_info(&format!(
            "🎯 Selecting {:?} bait at dynamic pos ({:.2}, {:.2}) -> px ({}, {})",
            chosen_tier, row_rx, row_ry, target_px.x, target_px.y
        ));
        if !click(ctx, target_px) || !ctx.sleep_ms(30) {
            return false;
        }

        return true;
    }

    let Some(primary) = s.points.bait[0] else {
        ctx.log_warn("Auto bait enabled but bait point not set");
        return true;
    };
    if s.features.zero_bait_failsafe && is_bait_depleted(ctx) {
        ctx.set_state(BotState::Paused, Some("Bait depleted".into()));
        ctx.log_warn("🎣 Bait depleted (zero bait detected)! Safely pausing macro.");
        ctx.webhook.bait_depleted();
        return false;
    }
    ctx.log_debug("Selecting bait");
    click_pair(ctx, primary, s.points.bait[1], 30)
}

pub fn zoom_reset(ctx: &Ctx) -> bool {
    let z = ctx.settings().zoom;
    ctx.log_debug("Zoom: resetting camera");
    if let Some(p) = fishing_point(ctx) {
        ctx.platform.input.move_to(p);
        if !ctx.sleep_ms(120) {
            return false;
        }
    }
    if !wheel_steps(ctx, z.out_steps, -1, z.step_delay_ms) {
        return false;
    }
    if !ctx.sleep_ms(z.sequence_delay_ms) {
        return false;
    }
    if !wheel_steps(ctx, z.in_steps, 1, z.step_delay_ms) {
        return false;
    }
    ctx.sleep_ms(z.sequence_delay_ms)
}

pub fn initial_setup(ctx: &Ctx, rod_equipped: &mut bool) -> bool {
    ctx.set_state(BotState::InitialSetup, None);
    let s = ctx.settings();
    if s.features.auto_zoom && (!zoom_reset(ctx) || !ctx.sleep_ms(800)) {
        return false;
    }
    if !ensure_rod_equipped(ctx, rod_equipped) {
        return false;
    }
    if !ctx.sleep_ms(400) {
        return false;
    }
    if s.features.auto_bait && !select_bait(ctx) {
        return false;
    }
    ctx.sleep_ms(1000)
}

pub fn cast(ctx: &Ctx) -> bool {
    if !rod_action_allowed(ctx) { ctx.release_mouse(); return false; }
    ctx.set_state(BotState::Casting, None);
    let Some(p) = fishing_point(ctx) else {
        ctx.log_warn("Roblox window not found; cannot cast");
        return false;
    };
    let hold = ctx.settings.read().fishing.cast_hold_ms;
    if !rod_action_allowed(ctx) { return false; }
    ctx.platform.input.move_to(p);
    if !ctx.sleep_ms(30) {
        return false;
    }
    if !rod_action_allowed(ctx) { ctx.release_mouse(); return false; }
    // Prevent right click when fishing to keep screen and camera steady
    ctx.hold_mouse(true);
    let ok = ctx.sleep_ms(hold);
    ctx.hold_mouse(false);
    ok
}

pub fn purchase_amount(ctx: &Ctx, amount_override: Option<u32>) -> bool {
    let s = ctx.settings();
    let (Some(confirm), Some(quantity)) = (s.points.purchase[0].and_then(|p| rel_to_px(ctx, p)), s.points.purchase[1].and_then(|p| rel_to_px(ctx, p))) else {
        ctx.log_warn("Auto purchase: confirm and quantity points not both set");
        return true;
    };
    let max_cap = s.purchase.max_bait.clamp(1, 300);
    let amount = amount_override.unwrap_or(s.purchase.amount).clamp(1, max_cap);
    ctx.set_state(BotState::Purchasing, None);
    ctx.log_info(&format!("Buying {amount} bait (up to {max_cap} max capacity)"));
    let p = &s.purchase;
    let delay = p.click_delay_ms;

    ctx.ensure_roblox_focus();
    if !key_hold(ctx, Key::Char(s.keys.shop), Duration::from_millis(p.hold_shop_key_ms.max(800) as u64)) {
        return false;
    }
    // Wait for the barrel merchant dialog to open
    if !ctx.sleep_ms(p.after_key_ms.max(500)) {
        return false;
    }

    // 1. Click Confirm option in merchant dialog to open the quantity prompt
    ctx.log_info(&format!("🛒 Auto purchase: clicking Confirm button at ({}, {})", confirm.x, confirm.y));
    if !ui_click(ctx, confirm) || !ctx.sleep_ms((delay + 300).max(500)) {
        return false;
    }

    // 2. Click the middle button (quantity text input box) to focus it
    ctx.log_info(&format!("🛒 Auto purchase: focusing quantity textbox at ({}, {})", quantity.x, quantity.y));
    if !ui_click(ctx, quantity) || !ctx.sleep_ms(150) {
        return false;
    }

    // 3. Select all and clear existing text
    ctx.platform.input.key(Key::Control, true);
    key_tap(ctx, Key::Char('a'));
    ctx.platform.input.key(Key::Control, false);
    if !ctx.sleep_ms(60) {
        return false;
    }
    key_tap(ctx, Key::Backspace);
    if !ctx.sleep_ms(60) {
        return false;
    }

    // 4. Type the purchase amount with clean keypresses
    for c in amount.to_string().chars() {
        if !key_tap(ctx, Key::Char(c)) || !ctx.sleep_ms(50) {
            return false;
        }
    }
    if !ctx.sleep_ms(p.after_type_ms.max(300)) {
        return false;
    }

    // Press Enter to submit textbox
    key_tap(ctx, Key::Enter);
    if !ctx.sleep_ms(150) {
        return false;
    }

    // 5. Click the Buy button to finalize and execute purchase
    let buy_button = s.points.purchase[2].or(s.points.purchase[0]).and_then(|p| rel_to_px(ctx, p));
    if let Some(buy_btn) = buy_button {
        ctx.log_info(&format!("🛒 Auto purchase: clicking final Buy button at ({}, {})", buy_btn.x, buy_btn.y));
        if !ui_click(ctx, buy_btn) || !ctx.sleep_ms((delay + 300).max(500)) {
            return false;
        }
    }

    // 6. Click Cancel button after "yes" in case user doesn't have enough Peli (safely dismisses prompt)
    let cancel_pt = RelPoint {
        x: 0.588,
        y: s.points.purchase[0].map(|p| p.y).unwrap_or(0.906),
    };
    if let Some(cancel_btn) = rel_to_px(ctx, cancel_pt) {
        ctx.log_info(&format!("🛒 Auto purchase: clicking Cancel button at ({}, {}) to ensure prompt dismisses", cancel_btn.x, cancel_btn.y));
        let _ = ui_click(ctx, cancel_btn);
        let _ = ctx.sleep_ms((delay + 200).max(400));
    }

    // 7. Click the final middle button (OK / Close) (purchase[3], or fallback to middle point purchase[1])
    let middle_button = s.points.purchase[3].or(s.points.purchase[1]).and_then(|p| rel_to_px(ctx, p));
    if let Some(mid_btn) = middle_button {
        ctx.log_info(&format!("🛒 Auto purchase: clicking final middle button (OK / Close) at ({}, {})", mid_btn.x, mid_btn.y));
        if !ui_click(ctx, mid_btn) || !ctx.sleep_ms((delay + 200).max(500)) {
            return false;
        }
    }
    {
        let mut sess = ctx.session.lock();
        sess.bait_purchased += amount;
        sess.since_purchase = 0;
    }
    ctx.emit_stats();
    ctx.emit(crate::events::BotEvent::Purchase { amount });
    if s.webhook.purchase {
        ctx.webhook.purchase(amount);
    }
    true
}

pub fn purchase(ctx: &Ctx) -> bool {
    let s = ctx.settings();
    let max_cap = s.purchase.max_bait.clamp(1, 300);

    ctx.ensure_roblox_focus();

    // 1. Ensure fishing rod is equipped so the in-game bait menu is open on screen
    let mut dummy = false;
    let _ = ensure_rod_equipped(ctx, &mut dummy);
    let _ = ctx.sleep_ms(450);

    // 2. Read common bait stock using neural network
    let to_buy = match scan_bait_stock_raw(ctx) {
        Ok((stock, visible)) if visible => {
            if let Some(c_qty) = stock.common {
                if c_qty >= max_cap {
                    ctx.log_info(&format!("🛒 Bait stock is already full ({c_qty}/{max_cap}); skipping purchase."));
                    ctx.session.lock().since_purchase = 0;
                    ctx.emit_stats();
                    return true;
                }
                let missing = max_cap.saturating_sub(c_qty).clamp(1, max_cap);
                ctx.log_info(&format!("🛒 Neural model detected {c_qty}/{max_cap} common bait. Purchasing exact missing {missing} bait..."));
                Some(missing)
            } else {
                None
            }
        }
        _ => None,
    };

    if to_buy.is_none() {
        ctx.log_warn(&format!("🛒 Could not detect common bait count from menu; purchasing configured default amount ({})", s.purchase.amount));
    }

    purchase_amount(ctx, to_buy)
}

fn fruit_notification_png(frame: &crate::core::types::Frame) -> Option<Vec<u8>> {
    // Preserve a readable PNG where it fits; resize unusually complex frames
    // before enqueue, rather than silently discarding the requested screenshot.
    for max_dimension in [1280, 640, 480] {
        let bytes = frame.downscale(max_dimension).to_png_bytes().ok()?;
        if bytes.len() <= 1024 * 1024 { return Some(bytes); }
    }
    None
}

pub fn capture_fruit_screenshot(ctx: &Ctx, s: &crate::config::Settings) -> Option<Vec<u8>> {
    if !s.webhook.send_screenshot {
        return None;
    }
    if !ctx.roblox_in_front() { return None; }
    let r = ctx.roblox_rect()?;

    if s.webhook.crop_fruit_screenshot {
        let drop_r = s.regions.drop;
        let center_x = drop_r.x + drop_r.w / 2.0;
        let center_y = drop_r.y + drop_r.h / 2.0;

        let target_w = (drop_r.w * 2.2).max(0.60).min(0.96);
        let target_h = (drop_r.h * 2.4).max(0.30).min(0.70);

        let crop_x = (center_x - target_w / 2.0).clamp(0.0, 1.0 - target_w);
        let crop_y = (center_y - target_h / 2.0).clamp(0.0, 1.0 - target_h);

        let crop_rel = RelRect {
            x: crop_x,
            y: crop_y,
            w: target_w,
            h: target_h,
        };
        let px_box = crop_rel.to_px(&r);
        if let Ok(frame) = ctx.platform.capture.grab(px_box) {
            if !ctx.roblox_in_front() { return None; }
            if let Some(bytes) = fruit_notification_png(&frame) {
                return Some(bytes);
            }
        }
    }

    ctx.platform.capture.grab(r).ok().filter(|_| ctx.roblox_in_front())
        .and_then(|f| fruit_notification_png(&f))
}

pub fn capture_drop_screenshot(ctx: &Ctx, s: &crate::config::Settings, is_high_tier: bool) -> Option<Vec<u8>> {
    if !s.webhook.send_screenshot || !s.webhook.send_drop_screenshot {
        return None;
    }
    if s.webhook.legendary_only && !is_high_tier {
        return None;
    }
    if !ctx.roblox_in_front() { return None; }
    let r = ctx.roblox_rect()?;

    if s.webhook.crop_fruit_screenshot {
        // Crop wide banner region across top-middle of the screen where GPO displays:
        // 1) "Dropped [Fruit] will despawn in 10 minutes"
        // 2) "You can only store one of each fruit!"
        let drop_r = s.regions.drop;
        let min_x = (drop_r.x - 0.10).min(0.15).max(0.0);
        let max_x = (drop_r.x + drop_r.w + 0.10).max(0.85).min(1.0);
        let min_y = 0.01;
        let max_y = (drop_r.y + drop_r.h + 0.18).max(0.33).min(1.0);

        let crop_rel = RelRect {
            x: min_x,
            y: min_y,
            w: max_x - min_x,
            h: max_y - min_y,
        };
        let px_box = crop_rel.to_px(&r);
        if let Ok(frame) = ctx.platform.capture.grab(px_box) {
            if !ctx.roblox_in_front() { return None; }
            if let Some(bytes) = fruit_notification_png(&frame) {
                return Some(bytes);
            }
        }
    }

    ctx.platform.capture.grab(r).ok().filter(|_| ctx.roblox_in_front())
        .and_then(|f| fruit_notification_png(&f))
}

/// Store/drop locally, then publish one terminal outcome before any notification work.
pub fn store_fruit(ctx: &Ctx, catch: &crate::core::fruit_storage::FruitCatchContext, workflow_id: &str) -> crate::core::fruit_storage::StorageOutcome {
    use crate::core::fruit_storage::{run_storage, StorageOutcome};
    use crate::core::workflow::{ActionEvent, ConfirmationState};
    let s = ctx.settings();
    if !s.features.fruit_storage { return StorageOutcome::Cancelled; }
    let now = crate::events::now_ms();
    let mut event = ActionEvent {
        workflow_id: workflow_id.into(), event_id: format!("{workflow_id}#store"),
        frame_index: None, session_id: ctx.ml.active_session_id().unwrap_or_else(|| "no-session".into()),
        result_event_id: None, entity_id: None, entity_type: Some("fruit".into()),
        policy_decision: Some(catch.policy.action.as_str().into()), action_requested: Some("store_attempt".into()),
        action_sent_at: Some(now), confirmation_state: ConfirmationState::Pending,
        confirmation_evidence: None, confirmation_at: None, retry_count: 0, final_outcome: None,
    };
    let _ = crate::core::workflow::append_action_event(ctx.store.dir(), &event);
    ctx.set_state(BotState::StoringFruit, Some(format!("Storing {}", catch.name)));
    let outcome = if s.points.fruit[0].is_none() {
        StorageOutcome::Failed("Store button point is not configured".into())
    } else {
        let mut io = LocalStorageIo { ctx, settings: &s, last_text: String::new(), drop_sent: false };
        let result = run_storage(&mut io, catch);
        // A delayed drop banner deserves read-only confirmation even when the
        // user has *not* enabled continuing after an unconfirmed outcome.
        // Never repeat Backspace or infer success from the key being sent.
        if result == StorageOutcome::Unknown && io.drop_sent {
            ctx.log_info("Fruit outcome unconfirmed; rechecking without sending another drop");
            match io.observe(5000, true) {
                Ok(Some(crate::core::fruit::StorageBannerResult::Dropped { .. }
                    | crate::core::fruit::StorageBannerResult::DuplicateDropped { .. })) => StorageOutcome::Dropped,
                Ok(_) => StorageOutcome::Unknown,
                Err(()) => StorageOutcome::Cancelled,
            }
        } else { result }
    };
    event.confirmation_state = match &outcome {
        StorageOutcome::Stored | StorageOutcome::ProtectedKept | StorageOutcome::Dropped => ConfirmationState::Confirmed,
        StorageOutcome::Failed(_) => ConfirmationState::Failed,
        StorageOutcome::Unknown | StorageOutcome::Cancelled => ConfirmationState::Unknown,
    };
    event.confirmation_at = Some(crate::events::now_ms());
    event.confirmation_evidence = Some(outcome.detail().into());
    event.final_outcome = Some(match &outcome {
        StorageOutcome::Stored => "stored", StorageOutcome::ProtectedKept => "protected_kept",
        StorageOutcome::Dropped => "dropped_confirmed", StorageOutcome::Failed(_) => "store_failed",
        StorageOutcome::Unknown => "unconfirmed", StorageOutcome::Cancelled => "cancelled",
    }.into());
    let _ = crate::core::workflow::append_action_event(ctx.store.dir(), &event);
    ctx.log_info(&format!("{}: {}", catch.name, outcome.detail()));
    // Publish the completion before capture/encoding or queued HTTP delivery.
    if ctx.alive() { ctx.set_state(BotState::PostCatch, Some(format!("{}: {}", catch.name, outcome.detail()))); }
    let notify = crate::webhook::should_notify_fruit_outcome(&s, catch, &outcome);
    let photo = if notify && ctx.alive() && ctx.roblox_in_front() { capture_drop_screenshot(ctx, &s, catch.high_tier()) } else { None };
    ctx.webhook.fruit_storage_outcome_with_id(catch, &outcome, photo, workflow_id);

    outcome
}

/// High-contrast white result text, used only after an ordinary OCR read is unknown.
/// This does not classify the catch: the same strict result parser remains authoritative.
pub(crate) fn catch_banner_ocr_frame(frame: &crate::core::types::Frame) -> crate::core::types::Frame {
    let mut mask = frame.clone();
    for px in mask.rgba.chunks_exact_mut(4) {
        let min = px[0].min(px[1]).min(px[2]);
        let max = px[0].max(px[1]).max(px[2]);
        let value = if min >= 170 && max - min <= 55 { 0 } else { 255 };
        px.copy_from_slice(&[value, value, value, 255]);
    }
    let factor = if mask.w.max(mask.h) <= 1280 { 2 } else { 1 };
    mask.upscale(factor)
}

// Fallback only for the yellow post-drop banner. A high-contrast copy avoids
// blue water, the red duplicate message and the white catch message dominating
// Windows OCR. It never authorizes input and is never sent to a remote service.
fn drop_banner_ocr_frame(frame: &crate::core::types::Frame) -> crate::core::types::Frame {
    let mut mask = frame.clone();
    for px in mask.rgba.chunks_exact_mut(4) {
        let yellow = px[0] >= 130 && px[1] >= 110
            && (px[2] as u16 + 55) < (px[0].min(px[1]) as u16);
        let value = if yellow { 0 } else { 255 };
        px.copy_from_slice(&[value, value, value, 255]);
    }
    // Keep the longest side below 2560 for the native OCR engine.
    let factor = if mask.w.max(mask.h) <= 1280 { 2 } else { 1 };
    mask.upscale(factor)
}

struct LocalStorageIo<'a> {
    ctx: &'a Ctx,
    settings: &'a crate::config::Settings,
    last_text: String,
    drop_sent: bool,
}
impl LocalStorageIo<'_> {
    /// Store prompts get three reads. After Backspace, GPO may display the
    /// drop confirmation later than the default 350 ms; wait longer before
    /// treating the result as unconfirmed. This only reads the screen:
    /// it never retries a destructive input or assumes a drop succeeded.
    fn observe(&mut self, wait_ms: u32, drop_only: bool) -> Result<Option<crate::core::fruit::StorageBannerResult>, ()> {
        let reads: u32 = if drop_only { 9 } else { 3 };
        let window_ms = if drop_only { wait_ms.max(1400) } else { wait_ms };
        let gap = window_ms.div_ceil(reads);
        for _ in 0..reads {
            if !self.ctx.sleep_ms(gap) || !self.ctx.roblox_in_front() { return Err(()); }
            let Some(r) = self.ctx.roblox_rect() else { return Err(()); };
            let rect = RelRect { x: 0.18, y: 0.02, w: 0.64, h: 0.28 }.to_px(&r);
            let frame = match self.ctx.platform.capture.grab(rect) {
                Ok(f) => f,
                Err(_) => return Ok(Some(crate::core::fruit::StorageBannerResult::Failed { reason: "Storage capture failed".into() })),
            };
            let text = match self.ctx.platform.ocr.read(&frame) {
                Ok(t) => t,
                Err(_) => return Ok(Some(crate::core::fruit::StorageBannerResult::Failed { reason: "Storage OCR failed".into() })),
            };
            self.last_text = text;
            let mut parsed = crate::core::fruit::parse_storage_banner(&self.settings.lexicon, &self.last_text);
            if drop_only && !matches!(parsed, Some(crate::core::fruit::StorageBannerResult::Dropped { .. } | crate::core::fruit::StorageBannerResult::DuplicateDropped { .. })) {
                // One bounded fallback on the SAME captured frame: no extra
                // Backspace, click or other game input.
                if let Ok(enhanced) = self.ctx.platform.ocr.read(&drop_banner_ocr_frame(&frame)) {
                    if let Some(banner) = crate::core::fruit::parse_storage_banner(&self.settings.lexicon, &enhanced) {
                        if matches!(banner, crate::core::fruit::StorageBannerResult::Dropped { .. } | crate::core::fruit::StorageBannerResult::DuplicateDropped { .. }) {
                            self.last_text = enhanced;
                            parsed = Some(banner);
                        }
                    }
                }
            }
            if let Some(banner) = parsed {
                if drop_only && !matches!(banner, crate::core::fruit::StorageBannerResult::Dropped { .. } | crate::core::fruit::StorageBannerResult::DuplicateDropped { .. }) { continue; }
                submit_action_frame(self.ctx, "confirm_banner", &self.last_text);
                return Ok(Some(banner));
            }
        }
        if drop_only {
            let excerpt: String = self.last_text.chars().take(180).collect();
            self.ctx.log_warn(&format!("Drop confirmation OCR remained unrecognized: {excerpt:?}"));
        }
        Ok(None)
    }
}
impl crate::core::fruit_storage::StorageIo for LocalStorageIo<'_> {
    fn active(&self) -> bool { self.ctx.alive() && self.ctx.roblox_in_front() }
    fn attempt_store(&mut self) -> Result<Option<crate::core::fruit::StorageBannerResult>, ()> {
        let Some(primary) = self.settings.points.fruit[0] else { return Err(()); };
        for slot in [self.settings.keys.fruit_slot_1, self.settings.keys.fruit_slot_2] {
            if !key_tap(self.ctx, Key::Char(slot)) || !self.ctx.sleep_ms(self.settings.fruit_storage.key_settle_ms)
                || !click_pair(self.ctx, primary, self.settings.points.fruit[1], self.settings.fruit_storage.click_settle_ms) { return Err(()); }
            if let Some(banner) = self.observe(self.settings.fruit_storage.dialog_wait_ms, false)? { return Ok(Some(banner)); }
            submit_action_frame(self.ctx, "store_banner", &self.last_text);
        }
        Ok(None)
    }
    fn drop_once(&mut self) -> bool {
        if self.drop_sent { return false; }
        self.drop_sent = true;
        key_hold(self.ctx, Key::Backspace, Duration::from_millis(180))
    }
    fn observe_drop(&mut self) -> Result<Option<crate::core::fruit::StorageBannerResult>, ()> {
        self.observe(self.settings.fruit_storage.after_drop_ms, true)
    }
}

/// Submit one action-UI frame (store/drop/confirmation screens) for future
/// ACTION_UI / CONFIRMATION training (§11, §18 of v5.3.0).
///
/// Trace-gated (explicit collection consent) and observation-only: re-grabs
/// the banner region, tags it (`store_banner` / `drop_banner` /
/// `confirm_banner`), and files it as an unreviewed hard example. NEVER
/// auto-labeled — a human promotes these; the game loop never reads them.
fn submit_action_frame(ctx: &Ctx, region_tag: &str, ocr_text: &str) {
    if !ctx.settings.read().fishing.trace {
        return;
    }
    if ctx.ml.active_session_id().is_none() {
        return;
    }
    let Some(r) = ctx.roblox_rect() else { return };
    let banner_rect = RelRect { x: 0.18, y: 0.02, w: 0.64, h: 0.28 };
    let frame = match ctx.platform.capture.grab(banner_rect.to_px(&r)) {
        Ok(f) => f,
        Err(_) => return,
    };
    ctx.ml.submit(crate::bot::ml_collect::SampleJob {
        png: Vec::new(),
        frame: Some(frame.clone()),
        session_id: String::new(),
        task: crate::core::ml_dataset::MlTask::UiDetection,
        ui_label: None,
        game_state: None,
        ocr_text: ocr_text.to_string(),
        region: region_tag.to_string(),
        entity_id: None,
        hard_reason: Some("action-ui: unreviewed, needs human labels".to_string()),
        source: "gameplay".to_string(),
    });
}

fn clean_ocr_digits(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'O' | 'o' | 'C' | 'c' | 'D' | 'Q' => '0',
            'I' | 'l' | '|' | '!' => '1',
            'S' | 's' => '5',
            'B' => '8',
            other => other,
        })
        .filter(|c| c.is_ascii_digit())
        .collect()
}

/// Parses strings looking for timestamps like "1 days 14:00:44", "dap 14•0C29", "01:43:48", "14:00:17"
pub fn extract_timestamp(text: &str) -> Option<(String, i64)> {
    let mut normalized = String::new();
    for c in text.chars() {
        match c {
            '•' | '·' | ';' => normalized.push(':'),
            _ => normalized.push(c),
        }
    }

    let lower = normalized.to_lowercase();

    // 1. Detect days: e.g. "1 days", "2 days", "dap", "1 dap", "dys"
    let mut days: i64 = 0;
    if let Some(idx) = lower.find("day") {
        let before = &lower[..idx];
        let digits: String = before.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>().chars().rev().collect();
        days = digits.parse::<i64>().unwrap_or(1);
    } else if lower.contains("dap") || lower.contains("dys") {
        if let Some(idx) = lower.find("dap").or_else(|| lower.find("dys")) {
            let before = &lower[..idx];
            let digits: String = before.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>().chars().rev().collect();
            days = digits.parse::<i64>().unwrap_or(1);
        } else {
            days = 1;
        }
    }

    // 2. Scan words for time chunks
    for word in normalized.split_whitespace() {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != ':');
        if clean.contains(':') {
            let parts: Vec<&str> = clean.split(':').collect();
            if parts.len() == 3 {
                let h_str = clean_ocr_digits(parts[0]);
                let m_str = clean_ocr_digits(parts[1]);
                let s_str = clean_ocr_digits(parts[2]);

                if let (Ok(h), Ok(m), Ok(s)) = (h_str.parse::<i64>(), m_str.parse::<i64>(), s_str.parse::<i64>()) {
                    if m < 60 && s < 60 {
                        let total = days * 86400 + h * 3600 + m * 60 + s;
                        let display = if days > 0 {
                            format!("{days}d {h:02}:{m:02}:{s:02}")
                        } else {
                            format!("{h:02}:{m:02}:{s:02}")
                        };
                        return Some((display, total));
                    }
                }
            } else if parts.len() == 2 {
                let p0 = clean_ocr_digits(parts[0]);
                let p1 = clean_ocr_digits(parts[1]);

                if p1.len() == 4 {
                    if let (Ok(h), Ok(m), Ok(s)) = (
                        p0.parse::<i64>(),
                        p1[..2].parse::<i64>(),
                        p1[2..].parse::<i64>(),
                    ) {
                        if m < 60 && s < 60 {
                            let total = days * 86400 + h * 3600 + m * 60 + s;
                            let display = if days > 0 {
                                format!("{days}d {h:02}:{m:02}:{s:02}")
                            } else {
                                format!("{h:02}:{m:02}:{s:02}")
                            };
                            return Some((display, total));
                        }
                    }
                } else if let (Ok(m), Ok(s)) = (p0.parse::<i64>(), p1.parse::<i64>()) {
                    if m < 60 && s < 60 {
                        let total = days * 86400 + m * 60 + s;
                        let display = if days > 0 {
                            format!("{days}d 00:{m:02}:{s:02}")
                        } else {
                            format!("{m:02}:{s:02}")
                        };
                        return Some((display, total));
                    }
                }
            }
        }
    }

    // 3. Fallback: also check core::boss_tracker::parse_duration_str
    if let Some(sec) = crate::core::boss_tracker::parse_duration_str(&normalized) {
        let total = days * 86400 + sec;
        return Some((crate::core::boss_tracker::format_duration(total), total));
    }

    None
}

/// Automatically captures the bottom-right corner of the Roblox window where the in-game
/// Server Age timer is displayed, runs Windows OCR, and calculates Travelling Merchant spawn.
/// Returns: (uptime_seconds, raw_time_string, remaining_seconds, is_currently_spawned)
pub fn scan_server_age(ctx: &Ctx) -> Result<(i64, String, i64, bool), String> {
    if !ctx.platform.ocr.available() {
        return Err("Windows OCR is not available on this system".into());
    }

    let rect = ctx.roblox_rect().ok_or("Roblox window not found")?;

    // In Roblox GPO, the server age timer is configured via s.regions.server_time
    // Default is bottom-right corner; user can adjust via F2 Overlay
    let scan_rect = {
        let s = ctx.settings();
        let r = s.regions.server_time.to_px(&rect);
        if r.w < 10 || r.h < 10 {
            let scan_w = 220.min(rect.w);
            let scan_h = 60.min(rect.h);
            let scan_x = rect.x + rect.w.saturating_sub(scan_w + 5);
            let scan_y = rect.y + rect.h.saturating_sub(scan_h + 15);
            PxRect {
                x: scan_x,
                y: scan_y,
                w: scan_w,
                h: scan_h,
            }
        } else {
            r
        }
    };

    let frame = ctx
        .platform
        .capture
        .grab(scan_rect)
        .map_err(|e| format!("Screen capture failed: {e}"))?;

    // Upscale 2x for optimal OCR readability on small fonts
    let upscaled = frame.upscale(2);

    let text = ctx
        .platform
        .ocr
        .read(&upscaled)
        .map_err(|e| format!("OCR read failed: {e}"))?;

    let (time_str, uptime_sec) = extract_timestamp(&text)
        .ok_or_else(|| format!("Could not find time (HH:MM:SS) in bottom-right corner. OCR detected: '{}'", text.trim()))?;

    let (rem, is_spawned) = crate::core::boss_tracker::merchant_spawn_from_server_age(uptime_sec);

    Ok((uptime_sec, time_str, rem, is_spawned))
}

#[cfg(test)]
mod storage_replay_tests {
    use super::*;
    use std::sync::{Arc, atomic::Ordering};
    use parking_lot::{Mutex, RwLock};
    use crate::core::platform::{Platform, mock::{MockWindow, ScriptedCapture, ScriptedOcr, RecordingInput, InputEvent}};
    use crate::core::types::{Frame, WindowInfo};
    use crate::core::fruit::{DropInfo};
    use crate::core::fruit_storage::{FruitCatchContext, StorageOutcome};

    struct Harness { ctx: Ctx, input: Arc<RecordingInput>, window: Arc<MockWindow>, rx: crossbeam_channel::Receiver<crate::events::BotEvent>, dir: std::path::PathBuf }
    impl Drop for Harness { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.dir); } }
    fn harness(texts: &[&str]) -> Harness {
        let mut settings = crate::config::Settings::default();
        settings.features.fruit_storage = true;
        settings.fishing.trace = false;
        settings.points.fruit[0] = Some(RelPoint { x: 0.5, y: 0.5 });
        settings.fruit_storage.key_settle_ms = 0;
        settings.fruit_storage.click_settle_ms = 0;
        settings.fruit_storage.dialog_wait_ms = 0;
        settings.fruit_storage.after_drop_ms = 0;
        settings.webhook.send_screenshot = false;
        // Even with Gemini enabled, the replay never contacts a network service.
        settings.gemini.enabled = true;
        settings.gemini.api_key = "test-not-a-real-key".into();
        let info = WindowInfo { client: PxRect { x: 0, y: 0, w: 1920, h: 1080 }, is_foreground: true, visible: true, dpi: 96 };
        let window = Arc::new(MockWindow { info: Mutex::new(Some(info)) });
        let cap = Arc::new(ScriptedCapture::default()); cap.push(Frame::new(4, 4, vec![255; 64]));
        let input = Arc::new(RecordingInput::default());
        let ocr = Arc::new(ScriptedOcr::default());
        ocr.texts.lock().extend(texts.iter().map(|s| s.to_string()));
        let platform = Platform { window: window.clone(), capture: cap, input: input.clone(), ocr };
        let (tx, rx) = crossbeam_channel::unbounded();
        // Windows clock resolution can give concurrent replays the same
        // timestamp. Each harness must have its own action-event log.
        static NEXT_REPLAY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let replay_id = NEXT_REPLAY.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("gpo-storage-{}-{replay_id}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let ctx = Ctx::new(platform, Arc::new(RwLock::new(settings)), Arc::new(RwLock::new(Some(info))), tx, Arc::new(crate::webhook::WebhookQueue::disabled()), Arc::new(crate::config::Store::new(dir.clone())));
        ctx.running.store(true, Ordering::SeqCst);
        Harness { ctx, input, window, rx, dir }
    }
    fn catch(raw: &str) -> FruitCatchContext {
        let d = DropInfo { name: None, pity: None, text: raw.into(), is_legendary: false, custom_telegram_message: None };
        FruitCatchContext::new(&d, raw, true, true)
    }
    fn drops(h: &Harness) -> usize { h.input.events.lock().iter().filter(|e| **e == InputEvent::Key(Key::Backspace, true)).count() }
    fn terminal(h: &Harness, expected: &str) {
        let lines = std::fs::read_to_string(h.dir.join("action_events.jsonl")).unwrap();
        let last: serde_json::Value = serde_json::from_str(lines.lines().last().unwrap()).unwrap();
        assert_eq!(last["final_outcome"], expected);
        assert!(last["confirmation_at"].is_number());
    }
    #[test] fn protected_full_storage_has_zero_drop_inputs_and_finishes_state() {
        let h = harness(&["You can only store one of each fruit!"]);
        assert_eq!(store_fruit(&h.ctx, &catch("0/40"), "test-protected"), StorageOutcome::ProtectedKept);
        assert_eq!(drops(&h), 0); assert_eq!(h.ctx.state(), BotState::PostCatch);
        terminal(&h, "protected_kept");
        assert!(h.rx.try_iter().any(|e| matches!(e, crate::events::BotEvent::State { state: BotState::PostCatch, .. })));
    }
    #[test] fn delayed_drop_ignores_old_duplicate_banner_and_drops_once() {
        let h = harness(&["You can only store one of each fruit!", "You can only store one of each fruit!", "", "Dropped Kilo will despawn in 10 minutes"]);
        assert_eq!(store_fruit(&h.ctx, &catch("unreadable"), "test-drop"), StorageOutcome::Dropped);
        assert_eq!(drops(&h), 1); terminal(&h, "dropped_confirmed");
    }
    #[test]
    fn enhanced_catch_frame_is_bounded_and_isolates_white_result_text() {
        let frame = Frame::new(3, 1, vec![255,255,255,255, 220,180,20,255, 20,90,160,255]);
        let enhanced = catch_banner_ocr_frame(&frame);
        assert_eq!((enhanced.w, enhanced.h), (6, 2));
        assert_eq!(&enhanced.rgba[..4], &[0,0,0,255]);
        assert_eq!(&enhanced.rgba[8..12], &[255,255,255,255]);
    }

    #[test]
    fn extended_recheck_confirms_late_drop_without_another_input() {
        let mut texts = vec!["You can only store one of each fruit!"];
        texts.extend(std::iter::repeat_n("", 18));
        texts.push("Dropped Bomb will despawn in 10 minutes");
        let h = harness(&texts);
        // Default is to pause on *still*-unconfirmed outcomes, but the late
        // banner must be checked before that decision is made.
        assert!(!h.ctx.settings().fruit_storage.continue_on_unconfirmed);
        assert_eq!(store_fruit(&h.ctx, &catch("4/40"), "late-recheck"), StorageOutcome::Dropped);
        assert_eq!(drops(&h), 1);
        terminal(&h, "dropped_confirmed");
    }

    #[test] fn late_drop_confirmation_is_not_mistaken_for_an_unconfirmed_drop() {
        // The old three-read drop window paused even though Roblox later
        // displayed its confirmation. A stale duplicate banner may linger.
        let h = harness(&[
            "You can only store one of each fruit!",
            "You can only store one of each fruit!",
            "", "", "", "",
            "Dropped Kilo will despawn in 10 minutes",
        ]);
        assert_eq!(store_fruit(&h.ctx, &catch("4/40"), "test-late-drop"), StorageOutcome::Dropped);
        assert_eq!(drops(&h), 1);
        terminal(&h, "dropped_confirmed");
    }
    #[test]
    fn yellow_banner_fallback_confirms_without_a_second_drop_input() {
        let h = harness(&[
            "You can only store one of each fruit!",
            "Dr0p text unreadable",
            "Drcpped Bomb will despawn in 10 minutes",
        ]);
        assert_eq!(store_fruit(&h.ctx, &catch("4/40"), "test-enhanced-drop"), StorageOutcome::Dropped);
        assert_eq!(drops(&h), 1);
        terminal(&h, "dropped_confirmed");
    }

    #[test]
    fn drop_banner_mask_is_bounded_and_excludes_red_and_white_text() {
        let source = crate::core::types::Frame::new(3, 1, vec![240, 220, 20, 255, 240, 10, 10, 255, 255, 255, 255, 255]);
        let mask = drop_banner_ocr_frame(&source);
        assert_eq!((mask.w, mask.h), (6, 2));
        assert_eq!(mask.px(0, 0), (0, 0, 0));
        assert_eq!(mask.px(2, 0), (255, 255, 255));
        assert_eq!(mask.px(4, 0), (255, 255, 255));
    }

    #[test] fn stored_banner_stops_slot_attempts() {
        let h = harness(&["Successfully stored Pika"]);
        assert_eq!(store_fruit(&h.ctx, &catch("0/100"), "test-stored"), StorageOutcome::Stored);
        assert_eq!(drops(&h), 0); terminal(&h, "stored");
        let events = h.input.events.lock();
        assert!(!events.iter().any(|e| *e == InputEvent::Key(Key::Char(h.ctx.settings().keys.fruit_slot_2), true)));
    }
    #[test] fn unconfirmed_drop_has_terminal_unknown() {
        let h = harness(&["You can only store one of each fruit!"]);
        assert_eq!(store_fruit(&h.ctx, &catch("4/40"), "test-unknown"), StorageOutcome::Unknown);
        assert_eq!(drops(&h), 1); terminal(&h, "unconfirmed");
    }
    #[test] fn focus_loss_never_sends_storage_inputs() {
        let h = harness(&[]); h.window.info.lock().as_mut().unwrap().is_foreground = false;
        assert_eq!(store_fruit(&h.ctx, &catch("0/40"), "test-focus"), StorageOutcome::Cancelled);
        assert!(h.input.events.lock().is_empty()); terminal(&h, "cancelled");
    }
    #[test] fn cancellation_never_leaves_a_pending_workflow() {
        let h = harness(&[]); h.ctx.running.store(false, Ordering::SeqCst);
        assert_eq!(store_fruit(&h.ctx, &catch("0/40"), "test-cancel"), StorageOutcome::Cancelled);
        assert!(h.input.events.lock().is_empty()); terminal(&h, "cancelled");
    }
}


#[cfg(test)]
mod rod_state_tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
    use parking_lot::RwLock;
    use crate::config::{Settings, Store};
    use crate::core::platform::{Platform, mock::{MockWindow, ScriptedCapture, RecordingInput, NoOcr, InputEvent}};
    use crate::core::types::WindowInfo;
    use crate::webhook::WebhookQueue;

    fn fixture(calibrated: bool) -> (Ctx, Arc<RecordingInput>, std::path::PathBuf) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("gpo-rod-state-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
        let info = WindowInfo { client: PxRect { x:0,y:0,w:800,h:600 }, is_foreground:true, visible:true, dpi:96 };
        let mut settings = Settings::default();
        settings.keys.rod = '4';
        if calibrated { settings.points.rod_slot = Some(RelPoint {x:0.5,y:0.95}); }
        let input = Arc::new(RecordingInput::default());
        let capture=Arc::new(ScriptedCapture::default());
        capture.push(crate::core::types::Frame::new(5,5,vec![0;100]));
        let (events, _rx) = crossbeam_channel::unbounded();
        let ctx = Ctx::new(
            Platform { window:Arc::new(MockWindow {info:parking_lot::Mutex::new(Some(info))}),
                capture, input:input.clone(), ocr:Arc::new(NoOcr) },
            Arc::new(RwLock::new(settings)), Arc::new(RwLock::new(Some(info))), events,
            Arc::new(WebhookQueue::disabled()), Arc::new(Store::new(dir.clone())));
        ctx.running.store(true, Ordering::SeqCst);
        rod_trackers().lock().retain(|(thread,_),_|*thread!=std::thread::current().id());
        (ctx,input,dir)
    }

    #[test]
    fn uncalibrated_hotbar_does_not_claim_slot_one_is_the_rod() {
        let (ctx,input,dir) = fixture(false);
        assert_eq!(is_rod_equipped(&ctx), None);
        let mut equipped = false;
        assert!(!ensure_rod_equipped(&ctx, &mut equipped));
        assert!(!equipped);
        assert!(!ensure_rod_equipped(&ctx, &mut equipped));
        let events=input.events.lock();
        assert_eq!(events.iter().filter(|e| matches!(e,InputEvent::Key(Key::Char('4'),true))).count(),1);
        assert!(!events.iter().any(|e| matches!(e,InputEvent::Key(Key::Char('1'),true))));
        drop(events);
        let _=std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unknown_rod_selection_switches_away_before_equipping_once() {
        let (ctx,input,dir) = fixture(false);
        let mut equipped = false;
        assert!(!ensure_rod_equipped(&ctx, &mut equipped));
        assert!(!equipped, "a dispatched key is not confirmation");
        let events = input.events.lock();
        let keys: Vec<char> = events.iter().filter_map(|event| match event {
            InputEvent::Key(Key::Char(key), true) => Some(*key),
            _ => None,
        }).collect();
        assert_eq!(keys, vec![ctx.settings().keys.reset_slot, '4']);
        drop(events);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn same_reset_and_rod_key_cannot_toggle_an_unknown_selection() {
        let (ctx,input,dir) = fixture(false);
        ctx.settings.write().keys.reset_slot = '4';
        assert!(!ensure_rod_equipped(&ctx, &mut false));
        assert!(input.events.lock().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn paused_bait_scan_never_starts_capture_or_network_work() {
        let (ctx,input,dir) = fixture(false);
        ctx.running.store(false, Ordering::SeqCst);
        assert!(scan_bait_stock_for(&ctx, true).is_err());
        assert!(input.events.lock().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn notifications_discard_frames_if_focus_changes_during_capture() {
        struct SwitchingCapture { window: Arc<MockWindow> }
        impl crate::core::platform::Capture for SwitchingCapture {
            fn grab(&self,_rect:PxRect)->crate::core::platform::Result<crate::core::types::Frame> {
                *self.window.info.lock()=None;
                Ok(crate::core::types::Frame::new(5,5,vec![255;100]))
            }
        }
        for drop_photo in [false,true] {
            let (mut ctx,_input,dir)=fixture(false);
            let window=Arc::new(MockWindow {info: parking_lot::Mutex::new(*ctx.roblox.read())});
            ctx.platform.window=window.clone();
            ctx.platform.capture=Arc::new(SwitchingCapture {window});
            ctx.settings.write().webhook.send_screenshot=true;
            ctx.settings.write().webhook.send_drop_screenshot=true;
            let settings=ctx.settings();
            let photo=if drop_photo {capture_drop_screenshot(&ctx,&settings,true)} else {capture_fruit_screenshot(&ctx,&settings)};
            assert!(photo.is_none(),"an Alt-Tab frame must never be sent");
            let _=std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn previously_verified_rod_survives_unreadable_reel_overlay() {
        let (ctx,input,dir) = fixture(false);
        let mut equipped = true;
        assert!(ensure_rod_equipped(&ctx, &mut equipped));
        assert!(equipped);
        assert!(input.events.lock().is_empty(), "no duplicate rod toggle while overlay is unreadable");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_rod_evidence_pauses_after_bounded_recovery() {
        let (ctx,input,dir) = fixture(false);
        let mut equipped = false;
        for i in 0..3 {
            if i > 0 { std::thread::sleep(std::time::Duration::from_millis(850)); }
            assert!(!ensure_rod_equipped(&ctx, &mut equipped));
        }
        assert!(!ensure_rod_equipped(&ctx, &mut equipped));
        assert_eq!(ctx.state(), BotState::Paused);
        assert_eq!(input.events.lock().iter().filter(|e| **e == InputEvent::Key(Key::Char('4'),true)).count(),3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn weak_negative_indicator_does_not_claim_remembered_rod_is_equipped() {
        let (ctx,_input,dir) = fixture(true);
        let mut equipped = true;
        assert!(!ensure_rod_equipped(&ctx, &mut equipped));
        assert!(!equipped);
        let _=std::fs::remove_dir_all(dir);
    }

    #[test]
    fn positive_bait_menu_already_held_sends_no_rod_key() {
        let (mut ctx,input,dir)=fixture(false);
        let ocr=Arc::new(crate::core::platform::mock::ScriptedOcr::default());
        ocr.texts.lock().push_back("Fishing Baits".into());
        ctx.platform.ocr=ocr;
        let mut equipped=false;
        assert!(ensure_rod_equipped(&ctx,&mut equipped));
        assert!(equipped);
        assert!(input.events.lock().is_empty());
        let _=std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unknown_initial_selection_is_verified_after_single_recovery() {
        let (mut ctx,input,dir)=fixture(false);
        let ocr=Arc::new(crate::core::platform::mock::ScriptedOcr::default());
        ocr.texts.lock().extend(["".into(),"".into(),"Fishing Baits".into()]);
        ctx.platform.ocr=ocr;
        let mut equipped=false;
        assert!(ensure_rod_equipped(&ctx,&mut equipped));
        assert!(equipped);
        let keys:Vec<char>=input.events.lock().iter().filter_map(|e|match e {InputEvent::Key(Key::Char(k),true)=>Some(*k),_=>None}).collect();
        assert_eq!(keys,vec![ctx.settings().keys.reset_slot,'4']);
        let _=std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pause_during_ocr_discards_late_positive_and_sends_no_key() {
        struct PausingOcr { ctx:parking_lot::Mutex<Option<std::sync::Weak<Ctx>>> }
        impl crate::core::platform::Ocr for PausingOcr {
            fn available(&self)->bool { true }
            fn read(&self,_:&crate::core::types::Frame)->crate::core::platform::Result<String> {
                if let Some(ctx)=self.ctx.lock().as_ref().and_then(|w|w.upgrade()) {
                    ctx.set_state(BotState::Paused,Some("test late OCR".into()));
                }
                Ok("Fishing Baits".into())
            }
        }
        let (mut ctx,input,dir)=fixture(false);
        let ocr=Arc::new(PausingOcr{ctx:parking_lot::Mutex::new(None)});
        ctx.platform.ocr=ocr.clone();
        let ctx=Arc::new(ctx);
        *ocr.ctx.lock()=Some(Arc::downgrade(&ctx));
        assert!(!ensure_rod_equipped(&ctx,&mut false));
        assert!(!cast(&ctx));
        assert!(!input.events.lock().iter().any(|e|matches!(e,InputEvent::Key(_,true)|InputEvent::Button(_,true))));
        let _=std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pause_prevents_late_equipping_and_cast() {
        let (ctx,input,dir)=fixture(false);
        ctx.set_state(BotState::Paused,Some("test".into()));
        assert!(!ensure_rod_equipped(&ctx,&mut false));
        assert!(!cast(&ctx));
        assert!(!input.events.lock().iter().any(|e|matches!(e,InputEvent::Key(_,true)|InputEvent::Button(_,true))));
        let _=std::fs::remove_dir_all(dir);
    }
}


#[cfg(test)]
mod notification_image_tests {
    #[test]
    fn complex_screenshot_is_retained_within_queue_budget() {
        let mut state = 7u32;
        let pixels: Vec<u8> = (0..1024 * 1024 * 4).map(|_| {
            state ^= state << 13; state ^= state >> 17; state ^= state << 5;
            state as u8
        }).collect();
        let frame = crate::core::types::Frame::new(1024, 1024, pixels);
        let png = super::fruit_notification_png(&frame).expect("bounded screenshot");
        assert!(png.len() <= 1024 * 1024);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
