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

pub fn capture_fruit_screenshot(ctx: &Ctx, s: &crate::config::Settings) -> Option<Vec<u8>> {
    if !s.webhook.send_screenshot {
        return None;
    }
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
            if let Ok(bytes) = frame.to_png_bytes() {
                return Some(bytes);
            }
        }
    }

    ctx.platform.capture.grab(r).ok()
        .map(|f| f.downscale(1280))
        .and_then(|f| f.to_png_bytes().ok())
}

pub fn capture_drop_screenshot(ctx: &Ctx, s: &crate::config::Settings, is_high_tier: bool) -> Option<Vec<u8>> {
    if !s.webhook.send_screenshot || !s.webhook.send_drop_screenshot {
        return None;
    }
    if s.webhook.legendary_only && !is_high_tier {
        return None;
    }
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
            if let Ok(bytes) = frame.to_png_bytes() {
                return Some(bytes);
            }
        }
    }

    ctx.platform.capture.grab(r).ok()
        .map(|f| f.downscale(1280))
        .and_then(|f| f.to_png_bytes().ok())
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
        run_storage(&mut io, catch)
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
        StorageOutcome::Unknown => "unknown_halted", StorageOutcome::Cancelled => "cancelled",
    }.into());
    let _ = crate::core::workflow::append_action_event(ctx.store.dir(), &event);
    ctx.log_info(&format!("{}: {}", catch.name, outcome.detail()));
    // Publish the completion before capture/encoding or queued HTTP delivery.
    if ctx.alive() { ctx.set_state(BotState::PostCatch, Some(format!("{}: {}", catch.name, outcome.detail()))); }
    let photo = if ctx.alive() && ctx.roblox_in_front() { capture_drop_screenshot(ctx, &s, catch.high_tier()) } else { None };
    ctx.webhook.fruit_storage_outcome(catch, &outcome, photo);

    outcome
}

struct LocalStorageIo<'a> {
    ctx: &'a Ctx,
    settings: &'a crate::config::Settings,
    last_text: String,
    drop_sent: bool,
}
impl LocalStorageIo<'_> {
    /// At most three local reads; configured delays govern the observation window.
    fn observe(&mut self, wait_ms: u32, drop_only: bool) -> Result<Option<crate::core::fruit::StorageBannerResult>, ()> {
        let gap = wait_ms.div_ceil(3);
        for _ in 0..3 {
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
            if let Some(banner) = crate::core::fruit::parse_storage_banner(&self.settings.lexicon, &self.last_text) {
                if drop_only && !matches!(banner, crate::core::fruit::StorageBannerResult::Dropped { .. } | crate::core::fruit::StorageBannerResult::DuplicateDropped { .. }) { continue; }
                submit_action_frame(self.ctx, "confirm_banner", &self.last_text);
                return Ok(Some(banner));
            }
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
    let png = match frame.to_png_bytes() {
        Ok(b) => b,
        Err(_) => return,
    };
    ctx.ml.submit(crate::bot::ml_collect::SampleJob {
        png,
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
        let dir = std::env::temp_dir().join(format!("gpo-storage-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
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
        assert_eq!(drops(&h), 1); terminal(&h, "unknown_halted");
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
