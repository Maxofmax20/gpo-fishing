use std::time::Duration;

use crate::core::types::{Key, MouseButton, PxPoint, PxRect, RelPoint, RelRect};
use crate::events::BotState;

use super::ctx::Ctx;

const WHEEL_STEP: i32 = 120;

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
    let input = &ctx.platform.input;
    input.move_to(p);
    if !ctx.sleep_ms(35) {
        return false;
    }
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
    let input = &ctx.platform.input;
    input.move_to(p);
    if !ctx.sleep_ms(60) {
        return false;
    }
    input.button(MouseButton::Left, true);
    if !ctx.sleep_ms(70) {
        input.button(MouseButton::Left, false);
        return false;
    }
    input.button(MouseButton::Left, false);
    ctx.sleep_ms(80)
}

pub fn right_click(ctx: &Ctx, p: PxPoint) -> bool {
    let input = &ctx.platform.input;
    input.move_to(p);
    if !ctx.sleep_ms(40) {
        return false;
    }
    input.button(MouseButton::Right, true);
    if !ctx.sleep_ms(40) {
        input.button(MouseButton::Right, false);
        return false;
    }
    input.button(MouseButton::Right, false);
    true
}

pub fn key_tap(ctx: &Ctx, k: Key) -> bool {
    ctx.platform.input.key(k, true);
    let ok = ctx.sleep_ms(30);
    ctx.platform.input.key(k, false);
    ok
}

pub fn key_hold(ctx: &Ctx, k: Key, d: Duration) -> bool {
    ctx.platform.input.key(k, true);
    let ok = ctx.sleep(d);
    ctx.platform.input.key(k, false);
    ok
}

pub fn wheel_steps(ctx: &Ctx, steps: u32, dir: i32, step_delay_ms: u32) -> bool {
    for _ in 0..steps {
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
    let s = ctx.settings();
    let rect = ctx.roblox_rect()?;

    // 1. If custom rod slot indicator point is set, inspect around it:
    if let Some(rel) = s.points.rod_slot {
        let px = rel.to_px(&rect);
        let patch = PxRect {
            x: (px.x - 2).max(rect.x),
            y: (px.y - 2).max(rect.y),
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
            return Some(bright >= 3);
        }
    }

    // 2. Auto-detect on standard Roblox bottom hotbar slot 1:
    if rect.h > 100 && rect.w > 200 {
        let scan_h = 50.min(rect.h - 10);
        let scan_w = 260.min(rect.w);
        let scan_y = rect.y + rect.h - scan_h - 5;
        let scan_x = rect.x + (rect.w / 2).saturating_sub(260);
        let scan_rect = PxRect {
            x: scan_x.max(rect.x),
            y: scan_y.max(rect.y),
            w: scan_w,
            h: scan_h,
        };
        if let Ok(frame) = ctx.platform.capture.grab(scan_rect) {
            for y in 0..frame.h {
                let mut run = 0;
                for x in 0..frame.w {
                    let (r, g, b) = frame.px(x, y);
                    if r > 210 && g > 210 && b > 210 {
                        run += 1;
                        if run >= 15 {
                            return Some(true);
                        }
                    } else {
                        run = 0;
                    }
                }
            }
            return Some(false);
        }
    }

    None
}

pub fn ensure_rod_equipped(ctx: &Ctx, rod_equipped: &mut bool) -> bool {
    let s = ctx.settings();

    // Check visual detection
    if let Some(visual) = is_rod_equipped(ctx) {
        if visual {
            ctx.log_debug("Fishing rod already equipped (visual check confirmed)");
            *rod_equipped = true;
            return true;
        } else {
            ctx.log_debug("Visual check detected rod is unequipped");
            *rod_equipped = false;
        }
    } else if *rod_equipped {
        ctx.log_debug("Fishing rod already equipped (state tracking)");
        return true;
    }

    ctx.log_info(&format!("Equipping rod ({})", s.keys.rod));
    if !key_tap(ctx, Key::Char(s.keys.rod)) || !ctx.sleep_ms(120) {
        return false;
    }
    *rod_equipped = true;
    true
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
    let rect = ctx.roblox_rect().ok_or("Roblox window not found")?;
    let scan_rect = {
        let s = ctx.settings();
        s.regions.bait_menu.to_px(&rect)
    };

    if scan_rect.w < 10 || scan_rect.h < 10 {
        return Err("Bait menu region is too small or not configured".into());
    }

    let frame = ctx
        .platform
        .capture
        .grab(scan_rect)
        .map_err(|e| format!("Screen capture failed: {e}"))?;

    let s = ctx.settings();

    // 1. Dedicated neural network classifier (100% accurate, dedicated GPO model, sub-millisecond local execution)
    if let Some(stock) = crate::core::bait::scan_bait_stock_neural(&frame) {
        ctx.log_info(&format!(
            "🐟 Bait stock recognized via neural model: Leg={:?}, Rare={:?}, Com={:?}",
            stock.legendary, stock.rare, stock.common
        ));
        return Ok((stock, true));
    }

    // 2. Try Gemini Vision if enabled and configured (fallback)
    if s.gemini.enabled && !s.gemini.api_key.trim().is_empty() {
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
        if let Ok(text) = ctx.platform.ocr.read(&frame) {
            ctx.log_info(&format!("Bait menu OCR raw text:\n{text}"));
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
        let (stock, menu_visible) = match scan_bait_stock_raw(ctx) {
            Ok(res) => res,
            Err(e) => {
                ctx.log_debug(&format!("Smart Bait: scan skipped: {e}"));
                return true;
            }
        };

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

        let chosen_tier = crate::core::bait::resolve_tier(&stock, s.purchase.bait_tier);

        // Notify user if a high tier bait depleted and we fell back to Common
        match s.purchase.bait_tier {
            crate::core::bait::BaitTier::Legendary if chosen_tier == crate::core::bait::BaitTier::Common && stock.legendary == Some(0) => {
                ctx.log_warn("⚠️ Legendary bait depleted! Automatically fell back to Common bait.");
            }
            crate::core::bait::BaitTier::Rare if chosen_tier == crate::core::bait::BaitTier::Common && stock.rare == Some(0) => {
                ctx.log_warn("⚠️ Rare bait depleted! Automatically fell back to Common bait.");
            }
            _ => {}
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

        let menu = s.regions.bait_menu;
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

        // If secondary backup point is also set, click it too (legacy support)
        if let Some(backup) = s.points.bait[1].and_then(|p| rel_to_px(ctx, p)) {
            if !click(ctx, backup) || !ctx.sleep_ms(30) {
                return false;
            }
            if !click(ctx, target_px) || !ctx.sleep_ms(30) {
                return false;
            }
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
    ctx.set_state(BotState::Casting, None);
    let Some(p) = fishing_point(ctx) else {
        ctx.log_warn("Roblox window not found; cannot cast");
        return false;
    };
    let hold = ctx.settings.read().fishing.cast_hold_ms;
    ctx.ensure_roblox_focus();
    ctx.platform.input.move_to(p);
    if !ctx.sleep_ms(30) {
        return false;
    }
    // Prevent right click when fishing to keep screen and camera steady
    ctx.hold_mouse(true);
    let ok = ctx.sleep_ms(hold);
    ctx.hold_mouse(false);
    ok
}

pub fn is_shop_dialog_visible(ctx: &Ctx) -> bool {
    let Some(rect) = ctx.roblox_rect() else {
        return false;
    };
    let dialog_rect = RelRect {
        x: 0.35,
        y: 0.85,
        w: 0.30,
        h: 0.12,
    }
    .to_px(&rect);

    if dialog_rect.w < 10 || dialog_rect.h < 10 {
        return false;
    }

    let Ok(frame) = ctx.platform.capture.grab(dialog_rect) else {
        return false;
    };

    let w = frame.w as f32;
    let h = frame.h as f32;
    let mut green = 0;
    let mut red = 0;
    let y_start = (h * 0.20) as usize;
    let y_end = (h * 0.85) as usize;
    let gx_start = (w * 0.12) as usize;
    let gx_end = (w * 0.30) as usize;
    let rx_start = (w * 0.70) as usize;
    let rx_end = (w * 0.90) as usize;

    for y in y_start..y_end.min(frame.h) {
        for x in gx_start..gx_end.min(frame.w) {
            let idx = (y * frame.w + x) * 4;
            if idx + 3 < frame.rgba.len() {
                let r = frame.rgba[idx];
                let g = frame.rgba[idx + 1];
                let b = frame.rgba[idx + 2];
                if g > 150 && b < 70 && g > r.saturating_add(25) {
                    green += 1;
                }
            }
        }
        for x in rx_start..rx_end.min(frame.w) {
            let idx = (y * frame.w + x) * 4;
            if idx + 3 < frame.rgba.len() {
                let r = frame.rgba[idx];
                let g = frame.rgba[idx + 1];
                let b = frame.rgba[idx + 2];
                if r > 170 && g < 70 && b < 70 {
                    red += 1;
                }
            }
        }
    }
    green > 15 || red > 15
}

pub fn purchase_amount(ctx: &Ctx, amount_override: Option<u32>) -> bool {
    let s = ctx.settings();
    let Some(rect) = ctx.roblox_rect() else {
        ctx.log_warn("Auto purchase: Roblox window not found");
        return false;
    };
    let quantity = s.points.purchase[1]
        .and_then(|p| rel_to_px(ctx, p))
        .unwrap_or_else(|| RelPoint { x: 0.501, y: 0.906 }.to_px(&rect));

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
    // Wait for the merchant/barrel dialog to open
    if !ctx.sleep_ms(p.after_key_ms.max(400)) {
        return false;
    }

    // Check if the "How many do you want?" prompt is already visible (e.g. from the dock bait vendor "her")
    let prompt_already_open = is_shop_dialog_visible(ctx);
    if prompt_already_open {
        ctx.log_info("🛒 Shop dialog (How many do you want?) is already open - bypassing Confirm button");
    } else {
        // If barrel requires clicking Confirm first to open the quantity prompt
        if let Some(confirm) = s.points.purchase[0].and_then(|p| rel_to_px(ctx, p)) {
            ctx.log_info(&format!("🛒 Auto purchase: clicking Confirm button at ({}, {})", confirm.x, confirm.y));
            if !ui_click(ctx, confirm) || !ctx.sleep_ms((delay + 300).max(500)) {
                return false;
            }
        }
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
    if !ctx.sleep_ms(p.after_type_ms.max(250)) {
        return false;
    }

    // Press Enter to submit textbox
    key_tap(ctx, Key::Enter);
    if !ctx.sleep_ms(150) {
        return false;
    }

    // 5. Click the Buy button to finalize and execute purchase
    let buy_button = s.points.purchase[2]
        .or(s.points.purchase[0])
        .and_then(|p| rel_to_px(ctx, p))
        .unwrap_or_else(|| RelPoint { x: 0.414, y: 0.910 }.to_px(&rect));

    ctx.log_info(&format!("🛒 Auto purchase: clicking final Buy button at ({}, {})", buy_button.x, buy_button.y));
    if !ui_click(ctx, buy_button) || !ctx.sleep_ms(250) {
        return false;
    }

    // 6. Guarantee this screen is GONE before continuing!
    ctx.log_info("🛒 Waiting until shop screen is completely gone...");
    let mut screen_cleared = false;
    for attempt in 0..15 {
        if !ctx.sleep_ms(180) {
            return false;
        }
        if !is_shop_dialog_visible(ctx) {
            screen_cleared = true;
            ctx.log_info("🛒 Confirmed: shop screen is gone!");
            break;
        }
        if attempt == 4 || attempt == 8 {
            ctx.log_info("🛒 Shop screen still visible; retrying Buy button click...");
            let _ = ui_click(ctx, buy_button);
        }
        if attempt == 11 {
            ctx.log_warn("🛒 Shop screen still visible (amount may exceed Peli limit); dismissing with Cancel/Esc...");
            let cancel_btn = RelPoint { x: 0.588, y: 0.910 }.to_px(&rect);
            let _ = ui_click(ctx, cancel_btn);
            let _ = key_tap(ctx, Key::Escape);
        }
    }
    if !screen_cleared {
        let _ = key_tap(ctx, Key::Escape);
        let _ = ctx.sleep_ms(200);
    }

    // 7. Equip the rod and wait until baits are visible again!
    let rod_key = s.keys.rod;
    ctx.log_info(&format!("🎣 Equipping rod ({rod_key}) and waiting to see baits again..."));
    key_tap(ctx, Key::Char(rod_key));

    let mut baits_seen = false;
    for attempt in 0..14 {
        if !ctx.sleep_ms(250) {
            return false;
        }
        let (stock, menu_visible) = scan_bait_stock_raw(ctx).unwrap_or_default();
        if menu_visible {
            let leg_str = stock.legendary.map(|n| n.to_string()).unwrap_or_else(|| "?".into());
            let rare_str = stock.rare.map(|n| n.to_string()).unwrap_or_else(|| "?".into());
            let com_str = stock.common.map(|n| n.to_string()).unwrap_or_else(|| "?".into());
            ctx.log_info(&format!("🐟 Baits seen again! Stock: [Legendary: {leg_str} | Rare: {rare_str} | Common: {com_str}]"));
            baits_seen = true;
            break;
        }
        if attempt == 4 || attempt == 9 {
            ctx.log_debug("Bait menu not yet visible, re-pressing rod key...");
            key_tap(ctx, Key::Char(rod_key));
        }
    }
    if !baits_seen {
        ctx.log_warn("Bait menu did not appear within timeout; ensuring rod is equipped before continuing.");
        let mut rod_eq = false;
        let _ = ensure_rod_equipped(ctx, &mut rod_eq);
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
    purchase_amount(ctx, None)
}

pub fn store_fruit(ctx: &Ctx, fruit_name: &str, protect_drop: bool) -> bool {
    let s = ctx.settings();
    if !s.features.fruit_storage {
        return true;
    }
    let Some(fruit_primary) = s.points.fruit[0] else {
        ctx.log_warn("Fruit storage enabled but fruit point not set");
        return true;
    };
    ctx.set_state(BotState::StoringFruit, None);
    if protect_drop {
        ctx.log_info(&format!("🛡️ Storing protected {fruit_name} (drop/backspace disabled)"));
    } else {
        ctx.log_info(&format!("Storing {fruit_name} in inventory"));
    }
    let fs = &s.fruit_storage;

    if let Some(fp) = fishing_point(ctx) {
        if !click(ctx, fp) || !ctx.sleep_ms(1000) {
            return false;
        }
        if !click(ctx, fp) || !ctx.sleep_ms(1000) {
            return false;
        }
    }

    let mut detected_banner: Option<crate::core::fruit::StorageBannerResult> = None;
    let banner_rect = RelRect { x: 0.25, y: 0.04, w: 0.50, h: 0.22 };

    for slot in [s.keys.fruit_slot_1, s.keys.fruit_slot_2] {
        if !key_tap(ctx, Key::Char(slot)) || !ctx.sleep_ms(fs.key_settle_ms) {
            return false;
        }
        if !click_pair(ctx, fruit_primary, s.points.fruit[1], fs.click_settle_ms) {
            return false;
        }
        if !ctx.sleep_ms(fs.dialog_wait_ms) {
            return false;
        }

        // Check if storage duplicate/error banner appeared right after clicking store
        if detected_banner.is_none() {
            if let Some(r) = ctx.roblox_rect() {
                let px_box = banner_rect.to_px(&r);
                if let Ok(frame) = ctx.platform.capture.grab(px_box) {
                    if s.gemini.enabled && !s.gemini.api_key.trim().is_empty() {
                        if let Ok(analysis) = crate::core::gemini::analyze_fruit_event_gemini(&frame, &s.gemini.api_key, &s.gemini.model) {
                            ctx.log_info(&format!("✨ Gemini Storage Analysis: event={}, fruit={:?}", analysis.event, analysis.fruit_name));
                            if let Some(name) = analysis.fruit_name {
                                if analysis.event == "storage_full" || analysis.event == "dropped_ground" {
                                    detected_banner = Some(crate::core::fruit::StorageBannerResult::DuplicateDropped { fruit_name: name });
                                }
                            }
                        }
                    }
                    if detected_banner.is_none() && ctx.platform.ocr.available() {
                        if let Ok(text) = ctx.platform.ocr.read(&frame) {
                            if !text.trim().is_empty() {
                                ctx.log_debug(&format!("Storage check OCR: {}", text.trim()));
                                if let Some(res) = crate::core::fruit::parse_storage_banner(&s.lexicon, &text) {
                                    ctx.log_info(&format!("Storage banner detected: {res:?}"));
                                    detected_banner = Some(res);
                                }
                            }
                        }
                    }
                }
            }
        }

        if !protect_drop {
            if !key_hold(ctx, Key::Backspace, Duration::from_millis(100)) {
                return false;
            }
            if !ctx.sleep_ms(fs.after_drop_ms) {
                return false;
            }

            // Check if drop banner appeared right after Backspace
            if detected_banner.is_none() {
                if let Some(r) = ctx.roblox_rect() {
                    let px_box = banner_rect.to_px(&r);
                    if let Ok(frame) = ctx.platform.capture.grab(px_box) {
                        if s.gemini.enabled && !s.gemini.api_key.trim().is_empty() {
                            if let Ok(analysis) = crate::core::gemini::analyze_fruit_event_gemini(&frame, &s.gemini.api_key, &s.gemini.model) {
                                ctx.log_info(&format!("✨ Gemini Drop Analysis: event={}, fruit={:?}", analysis.event, analysis.fruit_name));
                                if let Some(name) = analysis.fruit_name {
                                    if analysis.event == "storage_full" || analysis.event == "dropped_ground" {
                                        detected_banner = Some(crate::core::fruit::StorageBannerResult::DuplicateDropped { fruit_name: name });
                                    }
                                }
                            }
                        }
                        if detected_banner.is_none() && ctx.platform.ocr.available() {
                            if let Ok(text) = ctx.platform.ocr.read(&frame) {
                                if !text.trim().is_empty() {
                                    ctx.log_debug(&format!("Post-drop OCR: {}", text.trim()));
                                    if let Some(res) = crate::core::fruit::parse_storage_banner(&s.lexicon, &text) {
                                        ctx.log_info(&format!("Drop banner detected: {res:?}"));
                                        detected_banner = Some(res);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else {
            ctx.log_info("🛡️ Protected fruit kept in slot (drop prevented)");
        }
    }

    let photo = if s.webhook.send_screenshot {
        ctx.roblox_rect()
            .and_then(|r| ctx.platform.capture.grab(r).ok())
            .map(|f| f.downscale(1280))
            .and_then(|f| f.to_png_bytes().ok())
    } else {
        None
    };

    if let Some(banner) = detected_banner {
        match banner {
            crate::core::fruit::StorageBannerResult::DuplicateDropped { fruit_name: detected_name } => {
                let name = if detected_name != "Devil Fruit" {
                    detected_name
                } else if fruit_name != "Devil Fruit" {
                    fruit_name.to_string()
                } else {
                    "Devil Fruit".to_string()
                };
                ctx.log_warn(&format!("⚠️ Could not store {name}: duplicate fruit already in inventory (dropped)"));
                {
                    let mut sess = ctx.session.lock();
                    sess.last_fruit = Some(name.clone());
                }
                ctx.emit_stats();
                ctx.webhook.fruit_storage_failed(
                    &name,
                    "You can only store one of each fruit (inventory limit reached) - dropped on ground.",
                    photo,
                );
            }
            crate::core::fruit::StorageBannerResult::Dropped { fruit_name: detected_name } => {
                let name = if detected_name != "Devil Fruit" {
                    detected_name
                } else if fruit_name != "Devil Fruit" {
                    fruit_name.to_string()
                } else {
                    "Devil Fruit".to_string()
                };
                ctx.log_warn(&format!("⚠️ Fruit dropped on ground: {name}"));
                {
                    let mut sess = ctx.session.lock();
                    sess.last_fruit = Some(name.clone());
                }
                ctx.emit_stats();
                ctx.webhook.fruit_storage_failed(
                    &name,
                    "Fruit was dropped on the ground.",
                    photo,
                );
            }
            crate::core::fruit::StorageBannerResult::Failed { reason } => {
                ctx.log_warn(&format!("⚠️ Fruit storage failed: {reason}"));
                ctx.webhook.fruit_storage_failed(
                    fruit_name,
                    &reason,
                    photo,
                );
            }
        }
    } else {
        ctx.webhook.fruit_stored(fruit_name, photo);
    }

    if let Some(fp) = fishing_point(ctx) {
        ctx.platform.input.move_to(fp);
    }
    ctx.sleep_ms(300)
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
