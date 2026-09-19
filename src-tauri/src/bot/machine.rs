use std::time::{Duration, Instant};

use crate::core::controller::Tracker;
use crate::core::fruit;
use crate::core::types::{Frame, Key, PxRect, RelRect};
use crate::core::vision;
use crate::events::{BotEvent, BotState, TrackFrame};

use super::actions;
use super::ctx::Ctx;
use super::trace::Trace;

enum Outcome {
    Ended,
    Timeout,
    Lost,
    Stopped,
    Disconnected(String),
}

pub fn run(ctx: &Ctx, skip_setup: bool) {
    ctx.log_debug("Loop thread started");
    let mut tracker = Tracker::default();
    let mut last_hash: u64 = 0;
    let mut spawn_checked_at = Instant::now() - Duration::from_secs(3600);
    let mut rod_equipped = false;

    if !wait_for_roblox(ctx, false) {
        return;
    }
    if !ensure_front(ctx) {
        return;
    }
    if !skip_setup && !actions::initial_setup(ctx, &mut rod_equipped) {
        return;
    }

    while ctx.alive() {
        ctx.touch();
        if ctx.roblox_rect().is_none() && !wait_for_roblox(ctx, true) {
            return;
        }
        if let Some(reason) = check_disconnect(ctx) {
            ctx.set_state(BotState::Paused, Some(format!("Roblox disconnected: {reason}")));
            ctx.log_warn(&format!("⚠️ Roblox disconnect detected: {reason}"));
            let photo = capture_screenshot_bytes(ctx);
            ctx.webhook.disconnect(&reason, photo);
            return;
        }
        if !ensure_front(ctx) {
            return;
        }
        if !actions::ensure_rod_equipped(ctx, &mut rod_equipped) {
            return;
        }
        if ctx.settings.read().features.auto_bait {
            if !actions::select_bait(ctx) {
                if ctx.state() == BotState::Paused || !ctx.alive() {
                    return;
                }
            }
        }
        if !actions::cast(ctx) {
            return;
        }
        if !ctx.sleep_ms(30) {
            return;
        }
        tracker.reset();
        let outcome = fish_cycle(ctx, &mut tracker, &mut last_hash, &mut spawn_checked_at);
        ctx.release_mouse();
        match outcome {
            Outcome::Stopped => return,
            Outcome::Disconnected(reason) => {
                ctx.set_state(BotState::Paused, Some(format!("Roblox disconnected: {reason}")));
                ctx.log_warn(&format!("⚠️ Roblox disconnect detected: {reason}"));
                let photo = capture_screenshot_bytes(ctx);
                ctx.webhook.disconnect(&reason, photo);
                return;
            }
            Outcome::Ended => {
                // 1. Direct fast reset immediately after ending minigame to cancel catch animation on frame 1
                let s = ctx.settings.read();
                let fast_reset_enabled = s.features.fast_reset;
                let reset_key = s.keys.reset_slot;
                let rod_key = s.keys.rod;
                drop(s);

                if fast_reset_enabled {
                    ctx.log_info(&format!("⚡ Fast reset: immediately canceling animation with slot [{reset_key}] -> rod [{rod_key}]"));
                    actions::key_tap(ctx, Key::Char(reset_key));
                    if !ctx.sleep_ms(35) {
                        return;
                    }
                    actions::key_tap(ctx, Key::Char(rod_key));
                    if !ctx.sleep_ms(80) {
                        return;
                    }
                    rod_equipped = true;
                }

                let (verdict, text) = verify_catch(ctx);
                match verdict {
                    fruit::CatchVerdict::Failed => {
                        ctx.session.lock().record_failed();
                        ctx.emit_stats();
                        ctx.log_warn(&format!("Reel failed: {}", text.trim()));
                    }
                    verdict => {
                        let (kind, item_name) = fruit::parse_catch_item(&ctx.settings.read().lexicon, &text);
                        let is_fruit = kind == "fruit";
                        {
                            let mut s = ctx.session.lock();
                            s.record(true);
                            if is_fruit {
                                s.fruits += 1;
                                s.last_fruit = Some(item_name.clone());
                            } else {
                                s.last_fish = Some(item_name.clone());
                            }
                            let n = s.fish;
                            let tag = if verdict == fruit::CatchVerdict::Caught { "" } else { " (unverified)" };
                            ctx.log_info(&format!("Caught {kind}: {item_name} (#{n}){tag}"));
                        }
                        ctx.record_catch(&kind, &item_name, &text);
                        ctx.emit_stats();
                        if !post_catch(ctx, &text, &mut rod_equipped) {
                            return;
                        }
                        // Fast recast: If normal fish caught, immediately continue straight to bait selection and recast!
                        if !is_fruit {
                            continue;
                        }
                    }
                }
            }
            Outcome::Timeout => {
                if let Some(reason) = check_disconnect(ctx) {
                    ctx.set_state(BotState::Paused, Some(format!("Roblox disconnected: {reason}")));
                    ctx.log_warn(&format!("⚠️ Roblox disconnect detected: {reason}"));
                    let photo = capture_screenshot_bytes(ctx);
                    ctx.webhook.disconnect(&reason, photo);
                    return;
                }
                ctx.session.lock().record(false);
                ctx.emit_stats();
                ctx.log_debug("No bite; recasting");
            }
            Outcome::Lost => {
                if let Some(reason) = check_disconnect(ctx) {
                    ctx.set_state(BotState::Paused, Some(format!("Roblox disconnected: {reason}")));
                    ctx.log_warn(&format!("⚠️ Roblox disconnect detected: {reason}"));
                    let photo = capture_screenshot_bytes(ctx);
                    ctx.webhook.disconnect(&reason, photo);
                    return;
                }
                ctx.session.lock().record(false);
                ctx.emit_stats();
                ctx.log_debug("Bar lost during tracking; recasting");
            }
        }
        let wait = ctx.settings.read().fishing.wait_after_catch_s;
        if !ctx.sleep(Duration::from_secs_f32(wait)) {
            return;
        }
    }
}

fn wait_for_roblox(ctx: &Ctx, notify_disconnect: bool) -> bool {
    if ctx.roblox_rect().is_some() {
        return true;
    }
    ctx.set_state(BotState::WaitingForRoblox, None);
    ctx.log_warn("Waiting for Roblox window");
    if notify_disconnect {
        let photo = capture_screenshot_bytes(ctx);
        ctx.webhook.disconnect("Roblox disconnected or window closed", photo);
    }
    while ctx.alive() {
        if ctx.roblox_rect().is_some() {
            ctx.log_info("Roblox found");
            return true;
        }
        if !ctx.sleep_ms(500) {
            return false;
        }
    }
    false
}

fn ensure_front(ctx: &Ctx) -> bool {
    if ctx.ensure_roblox_focus() {
        return true;
    }
    ctx.set_state(BotState::WaitingForRoblox, Some("Roblox is not in front".into()));
    ctx.log_warn("Roblox is not the active window; waiting");
    while ctx.alive() {
        if ctx.roblox_rect().is_none() {
            return wait_for_roblox(ctx, true);
        }
        if let Some(reason) = check_disconnect(ctx) {
            ctx.set_state(BotState::Paused, Some(format!("Roblox disconnected: {reason}")));
            ctx.log_warn(&format!("⚠️ Roblox disconnect detected: {reason}"));
            let photo = capture_screenshot_bytes(ctx);
            ctx.webhook.disconnect(&reason, photo);
            return false;
        }
        if ctx.ensure_roblox_focus() {
            ctx.log_info("Roblox is back in front");
            return true;
        }
        if !ctx.sleep_ms(500) {
            return false;
        }
    }
    false
}

const BAR_PAD: f32 = 0.5;
const STUCK_HOLD_AFTER: Duration = Duration::from_millis(900);
const STUCK_MIN_SPEED: f32 = 0.05;

struct HoldWatch {
    since: Instant,
    progress: f32,
}

pub(super) fn bar_capture_rect(client: &PxRect, region: RelRect) -> (PxRect, i32) {
    let base = region.to_px(client);
    let pad = (base.w as f32 * BAR_PAD).round() as i32;
    let x0 = (base.x - pad).max(client.x);
    let x1 = (base.x + base.w + pad).min(client.right());
    (PxRect { x: x0, y: base.y, w: (x1 - x0).max(1), h: base.h }, x0 - base.x)
}

fn grab_region(ctx: &Ctx, region: RelRect) -> Option<Frame> {
    grab_rect(ctx, region.to_px(&ctx.roblox_rect()?))
}

fn grab_rect(ctx: &Ctx, rect: PxRect) -> Option<Frame> {
    match ctx.platform.capture.grab(rect) {
        Ok(f) => Some(f),
        Err(e) => {
            ctx.log_debug(&format!("capture: {e}"));
            None
        }
    }
}

pub(super) fn check_disconnect(ctx: &Ctx) -> Option<String> {
    if !ctx.platform.ocr.available() {
        return None;
    }
    let client = ctx.roblox_rect()?;
    // The Roblox disconnect modal is centered in the client area
    let center_region = RelRect {
        x: 0.20,
        y: 0.20,
        w: 0.60,
        h: 0.60,
    };
    let px_rect = center_region.to_px(&client);
    let frame = grab_rect(ctx, px_rect)?;
    let text = ctx.platform.ocr.read(&frame).ok()?;
    fruit::parse_disconnect_text(&text)
}

pub(super) fn capture_screenshot_bytes(ctx: &Ctx) -> Option<Vec<u8>> {
    ctx.roblox_rect()
        .and_then(|r| ctx.platform.capture.grab(r).ok())
        .map(|f| f.downscale(1280))
        .and_then(|f| f.to_png_bytes().ok())
}

fn fish_cycle(ctx: &Ctx, tracker: &mut Tracker, last_hash: &mut u64, spawn_checked_at: &mut Instant) -> Outcome {
    ctx.set_state(BotState::WaitingForBite, None);
    let started = Instant::now();
    let mut disconnect_checked_at = Instant::now() - Duration::from_secs(3600);
    let mut tracking_since: Option<Instant> = None;
    let mut confirm = 0u32;
    let mut misses = 0u32;
    let mut edge_warned = false;
    let mut bar_lock: Option<vision::Bbox> = None;
    let mut hold_watch: Option<HoldWatch> = None;
    let mut trace: Option<Trace> = None;
    let trace_enabled = ctx.settings.read().fishing.trace;
    let finish_trace = |trace: Option<Trace>, outcome: &str, ctx: &Ctx| {
        if let Some(t) = trace {
            let (name, _) = t.finish(outcome);
            ctx.log_info(&format!("Reel logged: {name}"));
        }
    };

    loop {
        if !ctx.alive() {
            return Outcome::Stopped;
        }
        ctx.touch();
        let s = ctx.settings.read();
        let scan_timeout = ctx.session.lock().adaptive_timeout(s.fishing.scan_timeout_s);
        let track_timeout = s.fishing.track_timeout_s;
        let palette = s.fishing.palette;
        let gains = s.fishing.control;
        let min_track = s.fishing.min_track_s;
        let confirm_frames = s.fishing.bite_confirm_frames.max(1);
        let lost_frames = s.fishing.lost_frames.max(1);
        let bar_region = s.regions.bar;
        let period = if tracking_since.is_some() {
            Duration::from_secs_f32(1.0 / s.fishing.track_hz.max(1) as f32)
        } else {
            Duration::from_secs_f32(1.0 / s.fishing.scan_hz.max(1) as f32)
        };
        let spawn_interval = Duration::from_secs_f32(s.ocr.spawn_check_interval_s);
        let alerts = s.fruit_alerts();
        drop(s);

        if tracking_since.is_none() {
            if started.elapsed().as_secs_f32() > scan_timeout {
                finish_trace(trace.take(), "timeout", ctx);
                return Outcome::Timeout;
            }
            if disconnect_checked_at.elapsed() > Duration::from_secs(3) {
                disconnect_checked_at = Instant::now();
                if let Some(reason) = check_disconnect(ctx) {
                    finish_trace(trace.take(), "disconnect", ctx);
                    return Outcome::Disconnected(reason);
                }
            }
            if alerts && spawn_checked_at.elapsed() > spawn_interval {
                *spawn_checked_at = Instant::now();
                check_spawn(ctx, last_hash);
            }
        } else if let Some(t) = tracking_since {
            if t.elapsed().as_secs_f32() > track_timeout {
                finish_trace(trace.take(), "track_timeout", ctx);
                return Outcome::Lost;
            }
        }

        let grab_started = Instant::now();
        let Some(client) = ctx.roblox_rect() else {
            if !ctx.sleep_ms(100) {
                return Outcome::Stopped;
            }
            continue;
        };
        let (capture_rect, origin_dx) = bar_capture_rect(&client, bar_region);
        let Some(frame) = grab_rect(ctx, capture_rect) else {
            if !ctx.sleep_ms(100) {
                return Outcome::Stopped;
            }
            continue;
        };
        let grab_took = grab_started.elapsed();

        let now = Instant::now();
        let reading = vision::read_locked(&frame, &palette, bar_lock);
        let read_took = now.elapsed();
        match reading {
            Some(r) => {
                misses = 0;
                if tracking_since.is_none() {
                    confirm += 1;
                    if confirm < confirm_frames {
                        if !ctx.sleep(period) {
                            return Outcome::Stopped;
                        }
                        continue;
                    }
                    tracking_since = Some(now);
                    tracker.reset();
                    bar_lock = None;
                    hold_watch = None;
                    ctx.set_state(BotState::Tracking, None);
                    ctx.log_debug("Bite confirmed; tracking");
                    if trace_enabled {
                        match Trace::start(&ctx.store.logs_dir(), gains) {
                            Ok(t) => trace = Some(t),
                            Err(e) => ctx.log_warn(&format!("trace: {e}")),
                        }
                    }
                }
                if bar_lock.is_none() && r.fish.start > 0 && r.fish.end + 1 < r.bar.h() && r.marker.start > 0 && r.marker.end + 1 < r.bar.h() {
                    bar_lock = Some(r.bar);
                }
                tracker.set_zone_half(r.fish.len() as f32 / 2.0 / r.bar.h().max(1) as f32);
                let decision = tracker.step(&gains, r.fish_center, r.marker_center, now);
                ctx.hold_mouse(decision.hold);
                if decision.hold {
                    let dir = if gains.physics.accel_hold < 0.0 { -1.0 } else { 1.0 };
                    let pinned = if dir < 0.0 { r.fish.start == 0 } else { r.fish.end + 1 >= r.bar.h() };
                    let w = hold_watch.get_or_insert(HoldWatch { since: now, progress: 0.0 });
                    w.progress = w.progress.max(dir * decision.fish_velocity);
                    if !pinned && now.duration_since(w.since) > STUCK_HOLD_AFTER && w.progress < STUCK_MIN_SPEED {
                        ctx.log_debug("Hold not registering; re-pressing at the cast point");
                        ctx.hold_mouse(false);
                        if let Some(p) = actions::fishing_point(ctx) {
                            ctx.platform.input.move_to(p);
                        }
                        hold_watch = None;
                    }
                } else {
                    hold_watch = None;
                }
                if !edge_warned && (r.bar.x0 == 0 || r.bar.x1 + 1 >= frame.w) {
                    edge_warned = true;
                    ctx.log_warn("Fishing bar touches the edge of its capture area; re-pick the Fishing bar area in Setup");
                }
                let tf = TrackFrame { reading: r, decision, origin_dx };
                if let Some(t) = trace.as_mut() {
                    t.frame(&tf, grab_took, read_took);
                }
                ctx.emit(BotEvent::Reading(Some(tf)));
            }
            None => {
                confirm = 0;
                if let Some(t) = tracking_since {
                    let bar_present = vision::has_bar(&frame, &palette);
                    if let Some(tr) = trace.as_mut() {
                        tr.blind(bar_present);
                    }
                    if bar_present {
                        misses = 0;
                    } else {
                        misses += 1;
                        if misses >= lost_frames {
                            ctx.emit(BotEvent::Reading(None));
                            let long_enough = t.elapsed().as_secs_f32() >= min_track;
                            finish_trace(trace.take(), if long_enough { "ended" } else { "lost" }, ctx);
                            return if long_enough { Outcome::Ended } else { Outcome::Lost };
                        }
                    }
                }
            }
        }

        if !ctx.sleep(period) {
            finish_trace(trace.take(), "stopped", ctx);
            return Outcome::Stopped;
        }
    }
}

fn check_spawn(ctx: &Ctx, last_hash: &mut u64) {
    if !ctx.platform.ocr.available() {
        return;
    }
    let cooldown = Duration::from_secs_f32(ctx.settings.read().ocr.spawn_cooldown_s);
    if let Some(t) = ctx.session.lock().last_spawn_alert {
        if t.elapsed() < cooldown {
            return;
        }
    }
    let region = ctx.settings.read().regions.drop;
    let Some(frame) = grab_region(ctx, region) else { return };
    let hash = frame.average_hash();
    if hash == *last_hash {
        return;
    }
    *last_hash = hash;
    let Ok(text) = ctx.platform.ocr.read(&frame) else { return };
    if text.trim().is_empty() {
        return;
    }
    ctx.log_debug(&format!("OCR: {}", text.trim()));
    let lex = ctx.settings.read().lexicon.clone();
    if let Some(info) = fruit::detect_spawn(&lex, &text) {
        ctx.log_info(&format!("Fruit spawned: {}", info.label()));
        {
            let mut s = ctx.session.lock();
            s.last_spawn = Some(info.label());
            s.last_spawn_alert = Some(Instant::now());
        }
        ctx.emit_stats();
        ctx.emit(BotEvent::FruitSpawn(info.clone()));
        if ctx.settings.read().webhook.spawn {
            ctx.webhook.spawn(&info);
        }
    }
}

fn verify_catch(ctx: &Ctx) -> (fruit::CatchVerdict, String) {
    ctx.set_state(BotState::PostCatch, None);
    if !ctx.platform.ocr.available() {
        return (fruit::CatchVerdict::Unknown, String::new());
    }
    let s = ctx.settings();
    let wants_ocr = s.features.fruit_storage || (s.webhook.enabled && s.webhook.fruit_drop);
    let max_reads = if wants_ocr { s.ocr.post_catch_reads.max(1) } else { 1 };
    let mut last = String::new();
    for _ in 0..max_reads {
        if let Some(frame) = grab_region(ctx, s.regions.drop) {
            if let Ok(text) = ctx.platform.ocr.read(&frame) {
                if !text.trim().is_empty() {
                    last = text.clone();
                    ctx.log_info(&format!("Reel text: {}", text.trim()));
                    let v = fruit::detect_catch(&s.lexicon, &text);
                    if v != fruit::CatchVerdict::Unknown {
                        return (v, text);
                    }
                }
            }
        }
        if !ctx.sleep_ms(s.ocr.post_catch_read_gap_ms.min(40)) {
            break;
        }
    }
    if last.is_empty() {
        ctx.log_info("Reel text: (nothing read in drop area)");
    }
    (fruit::CatchVerdict::Unknown, last)
}

fn post_catch(ctx: &Ctx, first_text: &str, rod_equipped: &mut bool) -> bool {
    ctx.set_state(BotState::PostCatch, None);
    let s = ctx.settings();
    let wants_ocr = s.features.fruit_storage || (s.webhook.enabled && s.webhook.fruit_drop);
    if wants_ocr && ctx.platform.ocr.available() {
        let mut drop = fruit::detect_drop(&s.lexicon, first_text);
        if drop.is_none() && first_text.trim().is_empty() {
            for _ in 0..s.ocr.post_catch_reads.max(1) {
            if drop.is_some() {
                break;
            }
            if let Some(frame) = grab_region(ctx, s.regions.drop) {
                if let Ok(text) = ctx.platform.ocr.read(&frame) {
                    if !text.trim().is_empty() {
                        ctx.log_debug(&format!("OCR: {}", text.trim()));
                    }
                    if let Some(d) = fruit::detect_drop(&s.lexicon, &text) {
                        drop = Some(d);
                        break;
                    }
                }
            }
            if !ctx.sleep_ms(s.ocr.post_catch_read_gap_ms.min(40)) {
                return false;
            }
        }
    }
    if let Some(mut d) = drop {
        // 1. Instantly snap character orientation to camera look direction
        actions::align_camera_shift_lock(ctx);

        // Enhance drop detection and message with Gemini if enabled and configured
        if s.gemini.enabled && !s.gemini.api_key.trim().is_empty() {
            if let Some(frame) = grab_region(ctx, s.regions.drop) {
                if let Ok(analysis) = crate::core::gemini::analyze_fruit_event_gemini(&frame, &s.gemini.api_key, &s.gemini.model) {
                    ctx.log_info(&format!("✨ Gemini Drop Analysis: pity={:?}, fruit={:?}", analysis.pity, analysis.fruit_name));
                    if let Some(p) = analysis.pity {
                        d.pity = Some(p);
                    }
                    if let Some(fn_name) = analysis.fruit_name {
                        d.name = Some(fn_name);
                    }
                    if let Some(msg) = analysis.telegram_message {
                        d.custom_telegram_message = Some(msg);
                    }
                }
            }
            if d.custom_telegram_message.is_none() {
                let temp_name = d.name.as_deref().unwrap_or("Devil Fruit");
                let rarity = fruit::fruit_rarity(temp_name);
                let pity_str = d.pity.as_deref().unwrap_or("Check backpack");
                if let Ok(rewritten) = crate::core::gemini::rewrite_fruit_message_gemini(
                    temp_name,
                    rarity.as_str(),
                    pity_str,
                    "Caught from fishing",
                    &s.gemini.api_key,
                    &s.gemini.model,
                ) {
                    ctx.log_info("✨ Generated accurate fruit Telegram message via Gemini");
                    d.custom_telegram_message = Some(rewritten);
                }
            }
        }

        let fruit_name = d.name.clone().unwrap_or_else(|| {
            fruit::parse_catch_item(&s.lexicon, &d.text).1
        });
            let rarity = fruit::fruit_rarity(&fruit_name);
            let is_high_tier = d.is_legendary || rarity.is_high_tier();
            let is_protected = s.fruit_storage.never_drop_legendary_or_mythical && is_high_tier;

            let label = if rarity == fruit::FruitRarity::Mythical {
                format!("Mythical devil fruit ({fruit_name})")
            } else if d.is_legendary || rarity == fruit::FruitRarity::Legendary {
                format!("Legendary devil fruit ({fruit_name})")
            } else if rarity != fruit::FruitRarity::Unknown {
                format!("{} devil fruit ({fruit_name})", rarity.as_str())
            } else {
                format!("Devil fruit ({fruit_name})")
            };
            ctx.log_info(&format!("{label} dropped"));
            {
                let mut sess = ctx.session.lock();
                sess.fruits += 1;
                sess.last_fruit = Some(fruit_name.clone());
                sess.pity_fruit = 0;
                if is_high_tier {
                    sess.pity_legendary = 0;
                }
            }
            ctx.record_catch("fruit", &fruit_name, &d.text);
            ctx.emit_stats();
            ctx.emit(BotEvent::FruitDrop(d.clone()));
            let photo = if s.webhook.send_screenshot {
                ctx.roblox_rect()
                    .and_then(|r| ctx.platform.capture.grab(r).ok())
                    .map(|f| f.downscale(1280))
                    .and_then(|f| f.to_png_bytes().ok())
            } else {
                None
            };
            if s.webhook.fruit_drop && (is_high_tier || !s.webhook.legendary_only) {
                ctx.webhook.fruit_drop(&d, photo);
            }
            if is_protected {
                ctx.log_info(&format!("🛡️ Protected {label} - preventing drop"));
            }
            if !actions::store_fruit(ctx, &fruit_name, is_protected) {
                return false;
            }
            *rod_equipped = true;

            if s.fruit_storage.pause_on_protected_fruit && is_protected {
                ctx.log_info(&format!("🚨 Macro paused: Protected {label} caught! Safely inspect your inventory."));
                return false;
            }
        }
    }

    let (fish, since_wh, since_buy) = {
        let sess = ctx.session.lock();
        (sess.fish, sess.since_progress_webhook, sess.since_purchase)
    };
    if s.webhook.progress && since_wh >= s.webhook.progress_every_n.max(1) {
        ctx.session.lock().since_progress_webhook = 0;
        ctx.webhook.progress(ctx.session.lock().stats());
        let _ = fish;
    }
    if s.features.auto_purchase && since_buy >= s.purchase.every_n_catches.max(1) {
        *rod_equipped = false;
        if !actions::purchase(ctx) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::RwLock;

    use super::*;
    use crate::config::Settings;
    use crate::core::platform::mock::{RecordingInput, ScriptedCapture, ScriptedOcr};
    use crate::core::platform::Platform;
    use crate::core::types::{PxRect, Rgb, WindowInfo};
    use crate::webhook::WebhookQueue;

    fn solid(w: usize, h: usize, paint: impl Fn(usize, usize) -> Rgb) -> Frame {
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let c = paint(x, y);
                let i = (y * w + x) * 4;
                rgba[i] = c.r;
                rgba[i + 1] = c.g;
                rgba[i + 2] = c.b;
                rgba[i + 3] = 255;
            }
        }
        Frame::new(w, h, rgba)
    }

    fn bar_frame(fish_y: usize) -> Frame {
        let p = vision::Palette::default();
        solid(40, 200, move |_, y| {
            if (fish_y..fish_y + 20).contains(&y) {
                p.fish
            } else if (100..106).contains(&y) {
                p.marker
            } else {
                p.bar
            }
        })
    }

    fn blank() -> Frame {
        solid(40, 200, |_, _| Rgb::new(0, 0, 0))
    }

    fn make_ctx(cap: Arc<ScriptedCapture>, input: Arc<RecordingInput>) -> (Arc<Ctx>, crossbeam_channel::Receiver<BotEvent>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut settings = Settings::default();
        settings.fishing.scan_timeout_s = 1.0;
        settings.fishing.scan_hz = 200;
        settings.fishing.track_hz = 200;
        settings.fishing.min_track_s = 0.0;
        let platform = Platform {
            window: Arc::new(crate::core::platform::mock::MockWindow::default()),
            capture: cap,
            input,
            ocr: Arc::new(ScriptedOcr::default()),
        };
        let roblox = Arc::new(RwLock::new(Some(WindowInfo {
            client: PxRect { x: 0, y: 0, w: 1920, h: 1080 },
            is_foreground: true,
            visible: true,
            dpi: 96,
        })));
        let store = Arc::new(crate::config::Store::new(std::env::temp_dir().join("gpo-autofish-test")));
        let ctx = Arc::new(Ctx::new(platform, Arc::new(RwLock::new(settings)), roblox, tx, Arc::new(WebhookQueue::disabled()), store));
        ctx.running.store(true, std::sync::atomic::Ordering::SeqCst);
        (ctx, rx)
    }

    #[test]
    fn cycle_tracks_then_catches() {
        let cap = Arc::new(ScriptedCapture::default());
        let input = Arc::new(RecordingInput::default());
        for _ in 0..3 {
            cap.push(blank());
        }
        for _ in 0..5 {
            cap.push(bar_frame(40));
        }
        for _ in 0..3 {
            cap.push(bar_frame(140));
        }
        for _ in 0..4 {
            cap.push(blank());
        }
        let (ctx, _rx) = make_ctx(cap, input.clone());
        let mut tracker = Tracker::default();
        let mut hash = 0;
        let mut t = Instant::now();
        let out = fish_cycle(&ctx, &mut tracker, &mut hash, &mut t);
        assert!(matches!(out, Outcome::Ended));
        let ev = input.events.lock();
        assert!(ev.iter().any(|e| matches!(e, crate::core::platform::mock::InputEvent::Button(_, true))));
    }

    #[test]
    fn cycle_times_out_without_bar() {
        let cap = Arc::new(ScriptedCapture::default());
        cap.push(blank());
        let input = Arc::new(RecordingInput::default());
        let (ctx, _rx) = make_ctx(cap, input);
        let mut tracker = Tracker::default();
        let mut hash = 0;
        let mut t = Instant::now();
        let out = fish_cycle(&ctx, &mut tracker, &mut hash, &mut t);
        assert!(matches!(out, Outcome::Timeout));
    }
}
