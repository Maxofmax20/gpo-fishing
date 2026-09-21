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
            if !craft_sleep(250) { return Ok(total_crafted); }

            // For recipes requiring 2 materials (e.g. 0/2), check if second fish needed
            let frame_check = ctx.platform.capture.grab(rect).map_err(|e| e.to_string())?;
            let mut fish_items_slot2 = detect_fish_list(&frame_check, &rect, layout.craft_button_pos.x);
            if fish_items_slot2.is_empty() {
                // List closed after first fish, click '+' again to reopen for slot 2
                click_point(ctx, layout.material_plus_pos);
                if !craft_sleep(250) { return Ok(total_crafted); }
                let frame_after_plus2 = ctx.platform.capture.grab(rect).map_err(|e| e.to_string())?;
                fish_items_slot2 = detect_fish_list(&frame_after_plus2, &rect, layout.craft_button_pos.x);
            }
            if !fish_items_slot2.is_empty() {
                let fish_2 = fish_items_slot2[0];
                ctx.log_info(&format!("🔨 Selecting fish #2 at ({}, {})", fish_2.x, fish_2.y));
                click_point(ctx, fish_2);
                if !craft_sleep(250) { return Ok(total_crafted); }
            }

            // 6. Click green CRAFT button
            ctx.log_info(&format!("🔨 Clicking CRAFT button at ({}, {})", layout.craft_button_pos.x, layout.craft_button_pos.y));
            click_point(ctx, layout.craft_button_pos);
            if !craft_sleep(500) { return Ok(total_crafted); }

            // 7. Check if Quantity Dialog appeared ("Craft Selected" / "Craft 1")
            let frame_popup = ctx.platform.capture.grab(rect).map_err(|e| e.to_string())?;
            if let Some(q) = detect_quantity_dialog(&frame_popup, &rect) {
                ctx.log_info("🔨 Quantity dialog detected! Maxing slider and crafting...");
                // Click right end of slider to select maximum quantity
                click_point(ctx, q.slider_max_pos);
                if !craft_sleep(180) { return Ok(total_crafted); }

                // Click green "Craft Selected" button
                click_point(ctx, q.craft_selected_pos);
                if !craft_sleep(600) { return Ok(total_crafted); }
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

    let search_top = frame.h / 3;
    let search_left = frame.w / 4;

    // Mask green pixels: bright green CRAFT button
    // CRAFT button has G > 175, R < 90, B < 80, and G > R * 2
    let mut visited = vec![false; frame.w * frame.h];
    let mut best_craft: Option<(usize, usize, usize, usize, usize)> = None;
    let mut best_count = 0;

    for y in search_top..frame.h {
        let row_start = y * frame.w;
        for x in search_left..frame.w {
            let idx = row_start + x;
            if visited[idx] {
                continue;
            }

            let p_idx = idx * 4;
            let r = frame.rgba[p_idx];
            let g = frame.rgba[p_idx + 1];
            let b = frame.rgba[p_idx + 2];

            if g > 170 && r < 95 && b < 85 && (g as u32 > r as u32 * 2) {
                // BFS to find connected component
                let mut queue = Vec::with_capacity(256);
                queue.push((y, x));
                visited[idx] = true;

                let mut min_x = x;
                let mut max_x = x;
                let mut min_y = y;
                let mut max_y = y;
                let mut count = 0;

                while let Some((cy, cx)) = queue.pop() {
                    count += 1;
                    min_x = min_x.min(cx);
                    max_x = max_x.max(cx);
                    min_y = min_y.min(cy);
                    max_y = max_y.max(cy);

                    // 4-neighborhood
                    for (dy, dx) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                        let ny = cy as i32 + dy;
                        let nx = cx as i32 + dx;
                        if ny >= search_top as i32 && ny < frame.h as i32 && nx >= search_left as i32 && nx < frame.w as i32 {
                            let n_idx = (ny as usize) * frame.w + (nx as usize);
                            if !visited[n_idx] {
                                visited[n_idx] = true;
                                let np_idx = n_idx * 4;
                                let nr = frame.rgba[np_idx];
                                let ng = frame.rgba[np_idx + 1];
                                let nb = frame.rgba[np_idx + 2];
                                if ng > 170 && nr < 95 && nb < 85 && (ng as u32 > nr as u32 * 2) {
                                    queue.push((ny as usize, nx as usize));
                                }
                            }
                        }
                    }
                }

                let w = max_x - min_x + 1;
                let h = max_y - min_y + 1;
                let ar = w as f32 / h.max(1) as f32;
                let density = count as f32 / (w * h) as f32;

                // Button criteria: width 60..220px, height 10..40px, aspect ratio 3.0..9.5, solid density > 0.55
                if (60..=220).contains(&w) && (10..=40).contains(&h) && ar >= 3.0 && ar <= 9.5 && density > 0.55 {
                    if count > best_count {
                        best_count = count;
                        best_craft = Some((min_x, max_x, min_y, max_y, count));
                    }
                }
            }
        }
    }

    let (min_x, max_x, min_y, max_y, _) = best_craft?;
    let craft_cx = ((min_x + max_x) / 2) as i32;
    let craft_cy = ((min_y + max_y) / 2) as i32;
    let craft_w = (max_x - min_x + 1) as f32;

    let scale_x = window.w as f32 / frame.w as f32;
    let scale_y = window.h as f32 / frame.h as f32;

    let craft_screen_pt = PxPoint {
        x: window.x + (craft_cx as f32 * scale_x).round() as i32,
        y: window.y + (craft_cy as f32 * scale_y).round() as i32,
    };

    // Material '+' is directly above CRAFT button: cy - (craft_w * 3.66)
    let raw_plus_y = (craft_cy as f32 - craft_w * 3.66).round() as i32;
    let raw_plus_x = craft_cx;

    // Visual snap: search a 35x35 neighborhood around raw_plus in frame for white '+' pixels
    let mut snap_x = raw_plus_x;
    let mut snap_y = raw_plus_y;
    let mut white_pts = Vec::new();
    let r_min_y = (raw_plus_y - 18).max(0) as usize;
    let r_max_y = (raw_plus_y + 18).min(frame.h as i32 - 1) as usize;
    let r_min_x = (raw_plus_x - 18).max(0) as usize;
    let r_max_x = (raw_plus_x + 18).min(frame.w as i32 - 1) as usize;

    for y in r_min_y..=r_max_y {
        for x in r_min_x..=r_max_x {
            let idx = (y * frame.w + x) * 4;
            let r = frame.rgba[idx];
            let g = frame.rgba[idx + 1];
            let b = frame.rgba[idx + 2];
            if r > 200 && g > 200 && b > 200 {
                white_pts.push((x, y));
            }
        }
    }
    if !white_pts.is_empty() {
        let avg_x = white_pts.iter().map(|p| p.0).sum::<usize>() / white_pts.len();
        let avg_y = white_pts.iter().map(|p| p.1).sum::<usize>() / white_pts.len();
        snap_x = avg_x as i32;
        snap_y = avg_y as i32;
    }

    let plus_screen_pt = PxPoint {
        x: window.x + (snap_x as f32 * scale_x).round() as i32,
        y: window.y + (snap_y as f32 * scale_y).round() as i32,
    };

    // Left pane bait selection list:
    // cx - (craft_w * 2.82)
    let list_frame_x = (craft_cx as f32 - craft_w * 2.82).round() as i32;
    let rare_frame_y = (craft_cy as f32 - craft_w * 0.84).round() as i32;
    let common_frame_y = (craft_cy as f32 - craft_w * 0.48).round() as i32;
    let leg_frame_y = (craft_cy as f32 - craft_w * 0.08).round() as i32;

    let rare_screen_pt = PxPoint {
        x: window.x + (list_frame_x as f32 * scale_x).round() as i32,
        y: window.y + (rare_frame_y as f32 * scale_y).round() as i32,
    };
    let common_screen_pt = PxPoint {
        x: window.x + (list_frame_x as f32 * scale_x).round() as i32,
        y: window.y + (common_frame_y as f32 * scale_y).round() as i32,
    };
    let leg_screen_pt = PxPoint {
        x: window.x + (list_frame_x as f32 * scale_x).round() as i32,
        y: window.y + (leg_frame_y as f32 * scale_y).round() as i32,
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

    let scale_x = window.w as f32 / frame.w as f32;
    let scale_y = window.h as f32 / frame.h as f32;

    let rel_craft_x = ((craft_button_x - window.x) as f32 / scale_x).round() as i32;

    // Right-panel fish rows are centered ~256px to the right of CRAFT button
    let fish_frame_x = rel_craft_x + 256;
    let search_start_x = (fish_frame_x - 80).clamp(0, frame.w as i32 - 1) as usize;
    let search_end_x = (fish_frame_x + 80).clamp(0, frame.w as i32 - 1) as usize;

    let mut item_pts = Vec::new();
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

            // Slate/blue pill background of fish rows
            if (15..=110).contains(&r) && (30..=140).contains(&g) && (55..=180).contains(&b) {
                row_blue_count += 1;
            }
        }

        if row_blue_count > 30 {
            if !in_item {
                in_item = true;
                item_start_y = y;
            }
        } else if in_item {
            in_item = false;
            let item_h = y - item_start_y;
            if (10..=65).contains(&item_h) {
                let center_y = (item_start_y + y) / 2;
                let center_x = (search_start_x + search_end_x) / 2;
                let pt = PxPoint {
                    x: window.x + (center_x as f32 * scale_x).round() as i32,
                    y: window.y + (center_y as f32 * scale_y).round() as i32,
                };
                item_pts.push(pt);
            }
        }
    }

    // If visual contour didn't separate rows, generate standard rows on right panel
    if item_pts.is_empty() {
        let base_y = (window.h as f32 * 0.18).round() as i32;
        let step_y = (window.h as f32 * 0.08).round() as i32;
        let screen_fish_x = window.x + (fish_frame_x as f32 * scale_x).round() as i32;
        for i in 0..5 {
            item_pts.push(PxPoint {
                x: screen_fish_x,
                y: window.y + base_y + (i * step_y),
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

    let search_top = frame.h / 3;
    let search_bottom = frame.h * 4 / 5;
    let search_left = frame.w / 4;
    let search_right = frame.w * 3 / 4;

    let mut visited = vec![false; frame.w * frame.h];
    let mut best_btn: Option<(usize, usize, usize, usize)> = None;
    let mut best_count = 0;

    for y in search_top..search_bottom {
        let row_start = y * frame.w;
        for x in search_left..search_right {
            let idx = row_start + x;
            if visited[idx] {
                continue;
            }

            let p_idx = idx * 4;
            let r = frame.rgba[p_idx];
            let g = frame.rgba[p_idx + 1];
            let b = frame.rgba[p_idx + 2];

            // Green button "Craft Selected": G is significantly higher than R and B
            if g > 110 && (g as u32 > r as u32 + 25) && (g as u32 > b as u32 + 25) {
                let mut queue = Vec::with_capacity(256);
                queue.push((y, x));
                visited[idx] = true;

                let mut min_x = x;
                let mut max_x = x;
                let mut min_y = y;
                let mut max_y = y;
                let mut count = 0;

                while let Some((cy, cx)) = queue.pop() {
                    count += 1;
                    min_x = min_x.min(cx);
                    max_x = max_x.max(cx);
                    min_y = min_y.min(cy);
                    max_y = max_y.max(cy);

                    for (dy, dx) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                        let ny = cy as i32 + dy;
                        let nx = cx as i32 + dx;
                        if ny >= search_top as i32 && ny < search_bottom as i32 && nx >= search_left as i32 && nx < search_right as i32 {
                            let n_idx = (ny as usize) * frame.w + (nx as usize);
                            if !visited[n_idx] {
                                visited[n_idx] = true;
                                let np_idx = n_idx * 4;
                                let nr = frame.rgba[np_idx];
                                let ng = frame.rgba[np_idx + 1];
                                let nb = frame.rgba[np_idx + 2];
                                if ng > 110 && (ng as u32 > nr as u32 + 25) && (ng as u32 > nb as u32 + 25) {
                                    queue.push((ny as usize, nx as usize));
                                }
                            }
                        }
                    }
                }

                let w = max_x - min_x + 1;
                let h = max_y - min_y + 1;
                let ar = w as f32 / h.max(1) as f32;

                // "Craft Selected" button: width 80..180px, height 20..50px, ar 2.2..5.0
                if (80..=180).contains(&w) && (20..=50).contains(&h) && ar >= 2.2 && ar <= 5.0 {
                    if count > best_count {
                        best_count = count;
                        best_btn = Some((min_x, max_x, min_y, max_y));
                    }
                }
            }
        }
    }

    let (min_x, max_x, min_y, max_y) = best_btn?;
    let sel_cx = ((min_x + max_x) / 2) as i32;
    let sel_cy = ((min_y + max_y) / 2) as i32;

    let scale_x = window.w as f32 / frame.w as f32;
    let scale_y = window.h as f32 / frame.h as f32;

    let craft_selected_pt = PxPoint {
        x: window.x + (sel_cx as f32 * scale_x).round() as i32,
        y: window.y + (sel_cy as f32 * scale_y).round() as i32,
    };

    // Slider track is ~85px above the buttons in frame coordinates.
    // The far right end of the slider track is ~290px to the right of Craft Selected center.
    let slider_frame_x = sel_cx + 290;
    let slider_frame_y = sel_cy - 85;

    let slider_max_pt = PxPoint {
        x: window.x + (slider_frame_x as f32 * scale_x).round() as i32,
        y: window.y + (slider_frame_y as f32 * scale_y).round() as i32,
    };

    Some(QuantityDialogLayout {
        slider_max_pos: slider_max_pt,
        craft_selected_pos: craft_selected_pt,
    })
}
