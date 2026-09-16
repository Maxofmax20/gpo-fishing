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
    if !ctx.sleep_ms(40) {
        return false;
    }
    input.button(MouseButton::Left, true);
    if !ctx.sleep_ms(40) {
        input.button(MouseButton::Left, false);
        return false;
    }
    input.button(MouseButton::Left, false);
    true
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
    if !key_tap(ctx, Key::Char(s.keys.rod)) || !ctx.sleep_ms(400) {
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

pub fn select_bait(ctx: &Ctx) -> bool {
    let s = ctx.settings();
    if !s.features.auto_bait {
        return true;
    }
    let Some(primary) = s.points.bait[0] else {
        ctx.log_warn("Auto bait enabled but bait point not set");
        return true;
    };
    if s.features.zero_bait_failsafe && is_bait_depleted(ctx) {
        ctx.log_warn("🎣 Bait depleted (zero bait detected)! Safely pausing macro.");
        ctx.webhook.bait_depleted();
        return false;
    }
    ctx.log_debug("Selecting bait");
    click_pair(ctx, primary, s.points.bait[1], 300)
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
    if s.features.auto_purchase && !purchase(ctx) {
        return false;
    }
    if !ensure_rod_equipped(ctx, rod_equipped) {
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
    ctx.platform.input.move_to(p);
    if !ctx.sleep_ms(80) {
        return false;
    }
    if !right_click(ctx, p) || !ctx.sleep_ms(250) {
        return false;
    }
    ctx.hold_mouse(true);
    let ok = ctx.sleep_ms(hold);
    ctx.hold_mouse(false);
    ok
}

pub fn purchase(ctx: &Ctx) -> bool {
    let s = ctx.settings();
    let (Some(confirm), Some(quantity)) = (s.points.purchase[0].and_then(|p| rel_to_px(ctx, p)), s.points.purchase[1].and_then(|p| rel_to_px(ctx, p))) else {
        ctx.log_warn("Auto purchase: confirm and quantity points not both set");
        return true;
    };
    ctx.set_state(BotState::Purchasing, None);
    ctx.log_info(&format!("Buying {} bait", s.purchase.amount));
    let p = &s.purchase;
    let delay = p.click_delay_ms;

    ctx.ensure_roblox_focus();
    if !key_hold(ctx, Key::Char(s.keys.shop), Duration::from_millis(p.hold_shop_key_ms as u64)) {
        return false;
    }
    if !ctx.sleep_ms(p.after_key_ms) {
        return false;
    }
    if !click(ctx, confirm) || !ctx.sleep_ms(delay) {
        return false;
    }
    if !click(ctx, quantity) || !ctx.sleep_ms(delay + 300) {
        return false;
    }
    ctx.platform.input.key(Key::Control, true);
    key_tap(ctx, Key::Char('a'));
    ctx.platform.input.key(Key::Control, false);
    if !ctx.sleep_ms(100) {
        return false;
    }
    key_tap(ctx, Key::Delete);
    if !ctx.sleep_ms(100) {
        return false;
    }
    for c in p.amount.to_string().chars() {
        if !key_tap(ctx, Key::Char(c)) || !ctx.sleep_ms(50) {
            return false;
        }
    }
    if !ctx.sleep_ms(p.after_type_ms) {
        return false;
    }
    if !click(ctx, confirm) || !ctx.sleep_ms(delay) {
        return false;
    }
    if let Some(cancel) = s.points.purchase[2].and_then(|p| rel_to_px(ctx, p)) {
        if !click(ctx, cancel) || !ctx.sleep_ms(delay) {
            return false;
        }
    }
    if !click(ctx, quantity) || !ctx.sleep_ms(delay) {
        return false;
    }
    if let Some(fp) = fishing_point(ctx) {
        if !right_click(ctx, fp) || !ctx.sleep_ms(delay) {
            return false;
        }
    }
    {
        let mut sess = ctx.session.lock();
        sess.bait_purchased += p.amount;
        sess.since_purchase = 0;
    }
    ctx.emit_stats();
    ctx.emit(crate::events::BotEvent::Purchase { amount: p.amount });
    if s.webhook.purchase {
        ctx.webhook.purchase(p.amount);
    }
    true
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
        if detected_banner.is_none() && ctx.platform.ocr.available() {
            if let Some(r) = ctx.roblox_rect() {
                let px_box = banner_rect.to_px(&r);
                if let Ok(frame) = ctx.platform.capture.grab(px_box) {
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

        if !protect_drop {
            if !key_hold(ctx, Key::Backspace, Duration::from_millis(100)) {
                return false;
            }
            if !ctx.sleep_ms(fs.after_drop_ms) {
                return false;
            }

            // Check if drop banner appeared right after Backspace
            if detected_banner.is_none() && ctx.platform.ocr.available() {
                if let Some(r) = ctx.roblox_rect() {
                    let px_box = banner_rect.to_px(&r);
                    if let Ok(frame) = ctx.platform.capture.grab(px_box) {
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

/// Parses strings looking for timestamps like "01:43:48" or "1:43:48" or "43:48"
pub fn extract_timestamp(text: &str) -> Option<(String, i64)> {
    for line in text.lines() {
        for word in line.split_whitespace() {
            let clean = word.trim_matches(|c: char| !c.is_ascii_digit() && c != ':');
            if clean.contains(':') {
                let parts: Vec<&str> = clean.split(':').collect();
                if parts.len() == 3 {
                    if let (Ok(h), Ok(m), Ok(s)) = (
                        parts[0].parse::<i64>(),
                        parts[1].parse::<i64>(),
                        parts[2].parse::<i64>(),
                    ) {
                        if m < 60 && s < 60 {
                            let total = h * 3600 + m * 60 + s;
                            return Some((clean.to_string(), total));
                        }
                    }
                } else if parts.len() == 2 {
                    if let (Ok(m), Ok(s)) = (
                        parts[0].parse::<i64>(),
                        parts[1].parse::<i64>(),
                    ) {
                        if m < 60 && s < 60 {
                            let total = m * 60 + s;
                            return Some((clean.to_string(), total));
                        }
                    }
                }
            }
        }
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

    // In Roblox GPO, the server age timer is in the bottom-right corner
    // We capture a box of width 140px, height 60px anchored at the bottom-right
    let scan_w = 140.min(rect.w);
    let scan_h = 60.min(rect.h);
    let scan_x = rect.x + rect.w.saturating_sub(scan_w + 5);
    let scan_y = rect.y + rect.h.saturating_sub(scan_h + 15);

    let scan_rect = PxRect {
        x: scan_x,
        y: scan_y,
        w: scan_w,
        h: scan_h,
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
