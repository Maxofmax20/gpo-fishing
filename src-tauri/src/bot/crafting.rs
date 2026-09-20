use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use parking_lot::RwLock;

use crate::bot::ctx::Ctx;
use crate::core::types::{Frame, PxPoint, PxRect};

static CRAFTING_ACTIVE: AtomicBool = AtomicBool::new(false);
static CRAFTING_STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CraftTier {
    Rare,
    Legendary,
    Common,
    All,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CraftStatus {
    pub is_crafting: bool,
    pub tier: String,
    pub crafted_count: u32,
    pub message: String,
}

static STATUS: RwLock<CraftStatus> = RwLock::new(CraftStatus {
    is_crafting: false,
    tier: String::new(),
    crafted_count: 0,
    message: String::new(),
});

pub fn get_craft_status() -> CraftStatus {
    STATUS.read().clone()
}

pub fn is_crafting_running() -> bool {
    CRAFTING_ACTIVE.load(Ordering::SeqCst)
}

pub fn stop_auto_craft() {
    if CRAFTING_ACTIVE.load(Ordering::SeqCst) {
        CRAFTING_STOP_REQUESTED.store(true, Ordering::SeqCst);
        let mut s = STATUS.write();
        s.message = "Stopping auto-craft...".into();
    }
}

use std::sync::Arc;

pub fn start_auto_craft(ctx: Arc<Ctx>, tier: CraftTier) -> Result<(), String> {
    if CRAFTING_ACTIVE.swap(true, Ordering::SeqCst) {
        return Err("Auto-crafting is already running".into());
    }
    CRAFTING_STOP_REQUESTED.store(false, Ordering::SeqCst);

    {
        let mut s = STATUS.write();
        s.is_crafting = true;
        s.tier = format!("{tier:?}");
        s.crafted_count = 0;
        s.message = "Starting auto-craft...".into();
    }

    std::thread::spawn(move || {
        let res = run_crafting_loop(&ctx, tier);
        CRAFTING_ACTIVE.store(false, Ordering::SeqCst);
        let mut s = STATUS.write();
        s.is_crafting = false;
        match res {
            Ok(count) => {
                s.message = format!("Auto-craft finished! Crafted {count} batches.");
                ctx.log_info(&format!("🔨 Auto-craft complete: {count} batches crafted."));
            }
            Err(e) => {
                s.message = format!("Auto-craft stopped: {e}");
                ctx.log_warn(&format!("🔨 Auto-craft: {e}"));
            }
        }
    });

    Ok(())
}

fn craft_sleep(ms: u64) -> bool {
    let step = 40;
    let mut elapsed = 0;
    while elapsed < ms {
        if CRAFTING_STOP_REQUESTED.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(step.min(ms - elapsed)));
        elapsed += step;
    }
    !CRAFTING_STOP_REQUESTED.load(Ordering::SeqCst)
}

fn run_crafting_loop(ctx: &Ctx, tier: CraftTier) -> Result<u32, String> {
    let mut total_crafted = 0u32;
    let tiers_to_run = match tier {
        CraftTier::All => vec![CraftTier::Legendary, CraftTier::Rare],
        other => vec![other],
    };

    for current_tier in tiers_to_run {
        if CRAFTING_STOP_REQUESTED.load(Ordering::SeqCst) {
            break;
        }

        {
            let mut s = STATUS.write();
            s.tier = format!("{current_tier:?}");
            s.message = format!("Crafting {current_tier:?} Fish Bait...");
        }
        ctx.log_info(&format!("🔨 Starting auto-craft for {current_tier:?} Fish Bait"));

        let mut consecutive_failures = 0;
        loop {
            if CRAFTING_STOP_REQUESTED.load(Ordering::SeqCst) {
                return Ok(total_crafted);
            }

            // 1. Ensure Roblox is focused
            ctx.ensure_roblox_focus();
            if !craft_sleep(100) { return Ok(total_crafted); }

            // 2. Grab current frame
            let rect = ctx.roblox_rect().ok_or("Roblox window not found")?;
            let frame = ctx.platform.capture.grab(rect).map_err(|e| e.to_string())?;

            // 3. Detect Blacksmith Sen dialog
            let layout = match detect_blacksmith_ui(&frame, &rect) {
                Some(l) => l,
                None => {
                    // Try pressing 'E' once to interact if dialog closed
                    ctx.log_info("🔨 Blacksmith Sen UI not visible, pressing 'E' to interact...");
                    ctx.platform.input.key(crate::core::types::Key::Char('e'), true);
                    std::thread::sleep(Duration::from_millis(60));
                    ctx.platform.input.key(crate::core::types::Key::Char('e'), false);
                    if !craft_sleep(600) { return Ok(total_crafted); }

                    let frame2 = ctx.platform.capture.grab(rect).map_err(|e| e.to_string())?;
                    match detect_blacksmith_ui(&frame2, &rect) {
                        Some(l) => l,
                        None => {
                            consecutive_failures += 1;
                            if consecutive_failures >= 3 {
                                return Err("Blacksmith Sen dialog not found. Please talk to Sen first!".into());
                            }
                            continue;
                        }
                    }
                }
            };

            // 4. Select the target bait in the left list
            let target_bait_pt = match current_tier {
                CraftTier::Rare => layout.rare_bait_pos,
                CraftTier::Legendary => layout.legendary_bait_pos,
                CraftTier::Common => layout.common_bait_pos,
                CraftTier::All => layout.rare_bait_pos,
            };

            ctx.log_info(&format!("🔨 Selecting {current_tier:?} Fish Bait at ({}, {})", target_bait_pt.x, target_bait_pt.y));
            click_point(ctx, target_bait_pt);
            if !craft_sleep(250) { return Ok(total_crafted); }

            // 5. Fill material slots:
            // Click the Material List '+' (plus) button to open fish list
            ctx.log_info(&format!("🔨 Clicking Material '+' at ({}, {})", layout.material_plus_pos.x, layout.material_plus_pos.y));
            click_point(ctx, layout.material_plus_pos);
            if !craft_sleep(300) { return Ok(total_crafted); }

            // Grab updated frame to see right-side fish list
            let frame_after_plus = ctx.platform.capture.grab(rect).map_err(|e| e.to_string())?;
            let fish_items = detect_fish_list(&frame_after_plus, &rect, layout.craft_button_pos.x);

            if fish_items.is_empty() {
                ctx.log_info(&format!("🔨 No available fish found for {current_tier:?} bait. Moving to next."));
                break;
            }

            // Click the first available fish in the list to fill slot 1
            let fish_1 = fish_items[0];
            ctx.log_info(&format!("🔨 Selecting fish #1 at ({}, {})", fish_1.x, fish_1.y));
            click_point(ctx, fish_1);
            if !craft_sleep(220) { return Ok(total_crafted); }

            // For recipes requiring 2 materials (e.g. 0/2), click plus again and select next fish if needed
            click_point(ctx, layout.material_plus_pos);
            if !craft_sleep(250) { return Ok(total_crafted); }

            let frame_slot2 = ctx.platform.capture.grab(rect).map_err(|e| e.to_string())?;
            let fish_items_slot2 = detect_fish_list(&frame_slot2, &rect, layout.craft_button_pos.x);
            if !fish_items_slot2.is_empty() {
                let fish_2 = fish_items_slot2[0];
                ctx.log_info(&format!("🔨 Selecting fish #2 at ({}, {})", fish_2.x, fish_2.y));
                click_point(ctx, fish_2);
                if !craft_sleep(220) { return Ok(total_crafted); }
            }

            // 6. Click green CRAFT button
            ctx.log_info(&format!("🔨 Clicking CRAFT button at ({}, {})", layout.craft_button_pos.x, layout.craft_button_pos.y));
            click_point(ctx, layout.craft_button_pos);
            if !craft_sleep(400) { return Ok(total_crafted); }

            // 7. Check if Quantity Dialog appeared ("Craft Selected" / "Craft 1")
            let frame_popup = ctx.platform.capture.grab(rect).map_err(|e| e.to_string())?;
            if let Some(q) = detect_quantity_dialog(&frame_popup, &rect) {
                ctx.log_info("🔨 Quantity dialog detected! Maxing slider and crafting...");
                // Click right end of slider to select maximum quantity
                click_point(ctx, q.slider_max_pos);
                if !craft_sleep(180) { return Ok(total_crafted); }

                // Click green "Craft Selected" button
                click_point(ctx, q.craft_selected_pos);
                if !craft_sleep(500) { return Ok(total_crafted); }
            } else {
                ctx.log_info("🔨 Single craft executed directly (no quantity dialog).");
                if !craft_sleep(400) { return Ok(total_crafted); }
            }

            total_crafted += 1;
            {
                let mut s = STATUS.write();
                s.crafted_count = total_crafted;
                s.message = format!("Crafted {total_crafted} batches so far...");
            }

            consecutive_failures = 0;
            // Short rest between crafts
            if !craft_sleep(350) { return Ok(total_crafted); }
        }
    }

    Ok(total_crafted)
}

fn click_point(ctx: &Ctx, pt: PxPoint) {
    ctx.platform.input.move_to(pt);
    std::thread::sleep(Duration::from_millis(25));
    ctx.platform.input.button(crate::core::types::MouseButton::Left, true);
    std::thread::sleep(Duration::from_millis(50));
    ctx.platform.input.button(crate::core::types::MouseButton::Left, false);
}

#[derive(Debug, Clone)]
struct BlacksmithUiLayout {
    craft_button_pos: PxPoint,
    material_plus_pos: PxPoint,
    rare_bait_pos: PxPoint,
    legendary_bait_pos: PxPoint,
    common_bait_pos: PxPoint,
}

#[derive(Debug, Clone)]
struct QuantityDialogLayout {
    slider_max_pos: PxPoint,
    craft_selected_pos: PxPoint,
}

/// Detects the Blacksmith Sen dialog window by locating the bright green CRAFT button
fn detect_blacksmith_ui(frame: &Frame, window: &PxRect) -> Option<BlacksmithUiLayout> {
    if frame.w == 0 || frame.h == 0 {
        return None;
    }

    // Look for green CRAFT button: G is dominant, R and B are significantly lower
    let mut min_x = frame.w;
    let mut max_x = 0;
    let mut min_y = frame.h;
    let mut max_y = 0;
    let mut count = 0;

    for y in (frame.h / 3)..frame.h {
        let row_start = y * frame.w * 4;
        for x in (frame.w / 4)..frame.w {
            let idx = row_start + x * 4;
            let r = frame.rgba[idx];
            let g = frame.rgba[idx + 1];
            let b = frame.rgba[idx + 2];

            if g > 150 && (g as u32 > (r as u32 * 3 / 2)) && b < 110 {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
                count += 1;
            }
        }
    }

    if count < 40 || (max_x - min_x) < 30 {
        return None;
    }

    let craft_cx = ((min_x + max_x) / 2) as i32;
    let craft_cy = ((min_y + max_y) / 2) as i32;

    // Relative offsets derived from GPO Blacksmith Sen UI:
    let button_w = (max_x - min_x) as f32;
    let scale = (button_w / 110.0).clamp(0.6, 2.2);

    let craft_screen_pt = PxPoint {
        x: window.x + (craft_cx as f32 * (window.w as f32 / frame.w as f32)).round() as i32,
        y: window.y + (craft_cy as f32 * (window.h as f32 / frame.h as f32)).round() as i32,
    };

    // Material '+' is directly above CRAFT button (~380px above at 1.0 scale)
    let plus_screen_pt = PxPoint {
        x: craft_screen_pt.x - (12.0 * scale).round() as i32,
        y: craft_screen_pt.y - (380.0 * scale).round() as i32,
    };

    // Baits list is in the left pane (~300px to the left of CRAFT button)
    let list_x = craft_screen_pt.x - (300.0 * scale).round() as i32;
    let rare_screen_pt = PxPoint {
        x: list_x,
        y: craft_screen_pt.y - (70.0 * scale).round() as i32,
    };
    let common_screen_pt = PxPoint {
        x: list_x,
        y: craft_screen_pt.y - (36.0 * scale).round() as i32,
    };
    let leg_screen_pt = PxPoint {
        x: list_x,
        y: craft_screen_pt.y,
    };

    Some(BlacksmithUiLayout {
        craft_button_pos: craft_screen_pt,
        material_plus_pos: plus_screen_pt,
        rare_bait_pos: rare_screen_pt,
        legendary_bait_pos: leg_screen_pt,
        common_bait_pos: common_screen_pt,
    })
}

/// Detects the available fish list items that pop up on the right side
fn detect_fish_list(frame: &Frame, window: &PxRect, craft_button_x: i32) -> Vec<PxPoint> {
    if frame.w == 0 || frame.h == 0 {
        return vec![];
    }

    let rel_craft_x = ((craft_button_x - window.x) as f32 * (frame.w as f32 / window.w as f32)).round() as usize;
    let search_start_x = (rel_craft_x + 90).min(frame.w.saturating_sub(60));
    let search_end_x = frame.w;

    let mut item_pts = Vec::new();

    // Fish items are horizontal rows with dark blue/grey background
    let mut in_item = false;
    let mut item_start_y = 0;

    for y in (frame.h / 8)..(frame.h * 3 / 4) {
        let row_start = y * frame.w * 4;
        let mut row_blue_count = 0;

        for x in search_start_x..search_end_x {
            let idx = row_start + x * 4;
            let r = frame.rgba[idx];
            let g = frame.rgba[idx + 1];
            let b = frame.rgba[idx + 2];

            if r >= 15 && r <= 110 && g >= 30 && g <= 140 && b >= 55 && b <= 180 {
                row_blue_count += 1;
            }
        }

        if row_blue_count > 40 {
            if !in_item {
                in_item = true;
                item_start_y = y;
            }
        } else if in_item {
            in_item = false;
            let item_h = y - item_start_y;
            if item_h >= 10 && item_h <= 70 {
                let center_y = (item_start_y + y) / 2;
                let center_x = (search_start_x + search_end_x) / 2;
                let pt = PxPoint {
                    x: window.x + (center_x as f32 * (window.w as f32 / frame.w as f32)).round() as i32,
                    y: window.y + (center_y as f32 * (window.h as f32 / frame.h as f32)).round() as i32,
                };
                item_pts.push(pt);
            }
        }
    }

    // Fallback standard points on right pane if visual edge detection found nothing
    if item_pts.is_empty() {
        let fallback_x = craft_button_x + 230;
        let scale_y = window.h as f32 / 685.0;
        for dy in [200.0, 245.0, 290.0, 335.0, 380.0] {
            item_pts.push(PxPoint {
                x: fallback_x,
                y: window.y + (dy * scale_y).round() as i32,
            });
        }
    }

    item_pts
}

/// Detects the quantity popup dialog ("Craft Selected" and "Craft 1")
fn detect_quantity_dialog(frame: &Frame, window: &PxRect) -> Option<QuantityDialogLayout> {
    if frame.w == 0 || frame.h == 0 {
        return None;
    }

    // Detect green "Craft Selected" button and red "Craft 1" button
    let mut min_gx = frame.w;
    let mut max_gx = 0;
    let mut min_gy = frame.h;
    let mut max_gy = 0;
    let mut g_count = 0;
    let mut r_count = 0;

    for y in (frame.h / 3)..(frame.h * 4 / 5) {
        let row_start = y * frame.w * 4;
        for x in (frame.w / 4)..(frame.w * 3 / 4) {
            let idx = row_start + x * 4;
            let r = frame.rgba[idx];
            let g = frame.rgba[idx + 1];
            let b = frame.rgba[idx + 2];

            if g > 100 && (g as u32 > r as u32 + 20) && (g as u32 > b as u32 + 20) {
                min_gx = min_gx.min(x);
                max_gx = max_gx.max(x);
                min_gy = min_gy.min(y);
                max_gy = max_gy.max(y);
                g_count += 1;
            } else if r > 100 && (r as u32 > g as u32 + 20) && (r as u32 > b as u32 + 20) {
                r_count += 1;
            }
        }
    }

    if g_count < 25 && r_count < 25 {
        return None;
    }

    let craft_sel_cx = ((min_gx + max_gx) / 2) as i32;
    let craft_sel_cy = ((min_gy + max_gy) / 2) as i32;

    let craft_selected_pt = PxPoint {
        x: window.x + (craft_sel_cx as f32 * (window.w as f32 / frame.w as f32)).round() as i32,
        y: window.y + (craft_sel_cy as f32 * (window.h as f32 / frame.h as f32)).round() as i32,
    };

    // Slider bar is located ~85px above the buttons, spanning horizontally
    // The right end (maximum quantity) is ~260px to the right of Craft Selected button center
    let slider_max_pt = PxPoint {
        x: craft_selected_pt.x + 260,
        y: craft_selected_pt.y - 84,
    };

    Some(QuantityDialogLayout {
        slider_max_pos: slider_max_pt,
        craft_selected_pos: craft_selected_pt,
    })
}
