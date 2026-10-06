use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::core::controller::Tracker;
use crate::core::fruit;
use crate::core::types::{Frame, Key, MouseButton, PxPoint, PxRect, RelPoint, RelRect};
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

pub fn run(ctx: &Arc<Ctx>, skip_setup: bool) {
    ctx.log_debug("Loop thread started");
    let mut tracker = Tracker::default();
    let mut last_hash: u64 = 0;
    let mut spawn_checked_at = Instant::now() - Duration::from_secs(3600);
    let mut anti_afk_at = Instant::now();
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
        if anti_afk_at.elapsed() >= Duration::from_secs(480) {
            anti_afk_at = Instant::now();
            trigger_anti_afk(ctx);
        }
        if ctx.roblox_rect().is_none() && !wait_for_roblox(ctx, true) {
            return;
        }
        if let Some(reason) = check_disconnect(ctx) {
            if handle_disconnect_flow(ctx, &reason, &mut rod_equipped) {
                continue;
            }
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
                if handle_disconnect_flow(ctx, &reason, &mut rod_equipped) {
                    continue;
                }
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
                // Training-data collection: one sample per ended reel, only
                // when the user enabled trace recording. Best-effort and
                // never on the timing-critical path (single grab + write).
                maybe_collect_ml_sample(ctx, &verdict, &text);
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
                    if handle_disconnect_flow(ctx, &reason, &mut rod_equipped) {
                        continue;
                    }
                    return;
                }
                ctx.session.lock().record(false);
                ctx.emit_stats();
                ctx.log_debug("No bite; recasting");
            }
            Outcome::Lost => {
                if let Some(reason) = check_disconnect(ctx) {
                    if handle_disconnect_flow(ctx, &reason, &mut rod_equipped) {
                        continue;
                    }
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

/// Validate a VIP server URL before it is handed to a shell.
///
/// `cmd /C start "" <url>` runs the URL through cmd.exe's parser, so argv
/// quoting does NOT protect it: `x&calc` is passed unquoted by Rust (no
/// whitespace) and cmd executes `calc` as a second command. `vip_server_url` is
/// writable from the token-authenticated LAN dashboard, so this is an RCE
/// primitive guarded only by the dashboard token.
///
/// Only an `https://` Roblox game URL is accepted, with nothing after it that
/// cmd would interpret. Empty input is allowed through as empty (the caller
/// skips the launch).
pub fn validate_vip_server_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    if url.is_empty() {
        return Ok(String::new());
    }
    if url.len() > 512 {
        return Err("too long".to_string());
    }
    if !url.starts_with("https://") {
        return Err("must be https".to_string());
    }
    if !url.contains("roblox.com") {
        return Err("must be a roblox.com URL".to_string());
    }
    // Belt and braces: even with the prefix checks above, refuse any character
    // a shell could act on. A Roblox game URL needs none of these.
    if let Some(bad) = url
        .chars()
        .find(|c| matches!(c, '&' | '|' | '<' | '>' | '^' | '%' | '!' | '`' | '"' | '\'' | '(' | ')' | '{' | '}' | ';' | '\n' | '\r' | '\t' | ' '))
    {
        return Err(format!("contains an illegal character {bad:?}"));
    }
    Ok(url.to_string())
}

fn wait_for_roblox(ctx: &Arc<Ctx>, notify_disconnect: bool) -> bool {
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

fn ensure_front(ctx: &Arc<Ctx>) -> bool {
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
            let mut rod_dummy = false;
            if handle_disconnect_flow(ctx, &reason, &mut rod_dummy) {
                return true;
            }
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
    // (tunable via Settings → game.disconnect_region).
    let center_region = ctx.settings.read().game.disconnect_region;
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

pub(super) fn trigger_anti_afk(ctx: &Ctx) {
    if let Some(rect) = ctx.roblox_rect() {
        let center = RelPoint { x: 0.50, y: 0.50 }.to_px(&rect);
        ctx.platform.input.move_to(center);
        let _ = ctx.sleep_ms(30);
        let nudge = PxPoint { x: center.x + 2, y: center.y };
        ctx.platform.input.move_to(nudge);
        ctx.log_debug("Anti-AFK: idle kick prevention timer refreshed");
    }
}

pub(super) fn attempt_reconnect(ctx: &Arc<Ctx>, reason: &str, rod_equipped: &mut bool) -> bool {
    ctx.set_state(BotState::Recovering, Some(format!("Auto-reconnecting: {reason}")));

    // 1. Click Roblox 'Reconnect' button on disconnect modal
    // (tunable via Settings → game.reconnect_point).
    if let Some(rect) = ctx.roblox_rect() {
        let reconnect_pt = ctx.settings.read().game.reconnect_point;
        let px = reconnect_pt.to_px(&rect);
        ctx.log_info("Auto-reconnect: Clicking Roblox 'Reconnect' button...");
        ctx.platform.input.move_to(px);
        let _ = ctx.sleep_ms(30);
        ctx.platform.input.button(MouseButton::Left, true);
        let _ = ctx.sleep_ms(60);
        ctx.platform.input.button(MouseButton::Left, false);
    }

    // 2. Wait up to 15s to check if Roblox reconnects
    let mut reconnected = false;
    for _ in 0..30 {
        if !ctx.alive() {
            return false;
        }
        if !ctx.sleep_ms(500) {
            return false;
        }
        if check_disconnect(ctx).is_none() && ctx.roblox_rect().is_some() {
            reconnected = true;
            break;
        }
    }

    // 3. If modal still present, attempt VIP URL or rejoin relaunch if configured
    if !reconnected {
        let vip = ctx.settings.read().features.vip_server_url.trim().to_string();
        match validate_vip_server_url(&vip) {
            Err(e) => ctx.log_info(&format!("Auto-reconnect: VIP URL rejected ({e})")),
            Ok(url) => {
                ctx.log_info(&format!("Auto-reconnect: Launching via VIP URL: {url}"));
                // `cmd /C start` re-parses its argument string, so Rust's argv
                // quoting is not enough: an unquoted `&` makes cmd run a second
                // command. The URL is validated to be a bare https:// Roblox
                // game URL first, so nothing that cmd would interpret can reach
                // it.
                let _ = std::process::Command::new("cmd")
                    .args(["/C", "start", "", &url])
                    .spawn();
            }
        }
    }

    // 4. Wait for Roblox window to become available
    if !wait_for_roblox(ctx, false) {
        return false;
    }

    // 5. Execute recorded private server join macro or type private server code if configured
    let (rejoin_macro, ps_code) = {
        let s = ctx.settings.read();
        (s.features.rejoin_macro_name.trim().to_string(), s.features.private_server_code.trim().to_string())
    };

    if !rejoin_macro.is_empty() {
        ctx.log_info(&format!("Auto-reconnect: Executing private server rejoin macro '{rejoin_macro}'..."));
        let _ = ensure_front(ctx);
        let _ = ctx.sleep_ms(1500);
        let _ = crate::bot::recorder::play_macro(
            Arc::clone(ctx),
            ctx.store.clone(),
            &rejoin_macro,
            false,
            Some(1.0),
            Some(1),
        );
        // Wait for macro execution to finish
        for _ in 0..30 {
            if !ctx.alive() { return false; }
            if !crate::bot::recorder::is_playing() { break; }
            let _ = ctx.sleep_ms(500);
        }
    } else if !ps_code.is_empty() {
        ctx.log_info(&format!("Auto-reconnect: Entering private server code '{ps_code}'..."));
        let _ = ensure_front(ctx);
        let _ = ctx.sleep_ms(1000);
        ctx.platform.input.type_text(&ps_code);
        let _ = ctx.sleep_ms(250);
        ctx.platform.input.key(crate::core::types::Key::Enter, true);
        let _ = ctx.sleep_ms(80);
        ctx.platform.input.key(crate::core::types::Key::Enter, false);
    }

    // 6. Wait 12s for character assets to load
    ctx.log_info("Roblox window found! Waiting 12s for character to load...");
    for _ in 0..24 {
        if !ctx.alive() {
            return false;
        }
        if !ctx.sleep_ms(500) {
            return false;
        }
    }

    // 7. Ensure in front and equip rod
    if !ensure_front(ctx) {
        return false;
    }
    *rod_equipped = false;
    if !actions::ensure_rod_equipped(ctx, rod_equipped) {
        return false;
    }

    ctx.log_info("✅ Auto-reconnect successful! Resuming autofish.");
    ctx.webhook.reconnected(None);
    ctx.set_state(BotState::WaitingForBite, None);
    true
}

fn handle_disconnect_flow(ctx: &Arc<Ctx>, reason: &str, rod_equipped: &mut bool) -> bool {
    let photo = capture_screenshot_bytes(ctx);
    ctx.webhook.disconnect(reason, photo);

    let auto_reconnect = ctx.settings.read().features.auto_reconnect;
    if auto_reconnect && ctx.alive() {
        ctx.log_warn(&format!("⚠️ Roblox disconnect: {reason}. Auto-reconnect active!"));
        if attempt_reconnect(ctx, reason, rod_equipped) {
            return true;
        }
    }

    ctx.set_state(BotState::Paused, Some(format!("Roblox disconnected: {reason}")));
    ctx.log_warn(&format!("⚠️ Roblox disconnect detected: {reason} (Bot paused)"));
    false
}

fn fish_cycle(ctx: &Ctx, tracker: &mut Tracker, last_hash: &mut u64, spawn_checked_at: &mut Instant) -> Outcome {
    ctx.set_state(BotState::WaitingForBite, None);
    // Episode capture: WAITING state, once per cast (trace-gated, async).
    if ctx.settings.read().fishing.trace {
        if let Some(client) = ctx.roblox_rect() {
            let bar_region = ctx.settings.read().regions.bar;
            if let Some(frame) = grab_rect(ctx, bar_region.to_px(&client)) {
                submit_ml_frame(
                    ctx,
                    &frame,
                    crate::core::ml_dataset::MlTask::UiDetection,
                    Some(crate::core::ml_dataset::UiLabel::FishingBar),
                    Some(crate::core::ml_dataset::GameStateLabel::WaitingForBite),
                    "",
                    "bar",
                    None,
                    None,
                );
            }
        }
    }
    let started = Instant::now();
    let mut disconnect_checked_at = Instant::now() - Duration::from_secs(3600);
    let mut tracking_since: Option<Instant> = None;
    let mut confirm = 0u32;
    let mut misses = 0u32;
    // Shadow sampling counter (observation only; see bot::shadow).
    let mut shadow_n: u64 = 0;
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
        // Shadow STATE observation (sampled; never influences control).
        shadow_n += 1;
        if shadow_n % crate::bot::shadow::STATE_SAMPLE_EVERY == 0 {
            crate::bot::shadow::observe_state(ctx, &frame, reading.is_some());
        }
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
                    // Shadow: always observe the confirming frame (rare,
                    // high-value moment for agreement telemetry).
                    crate::bot::shadow::observe_state(ctx, &frame, true);
                    // Episode capture: BITE moment with the confirming frame
                    // (trace-gated, async — encode is a few ms, once per reel).
                    if ctx.settings.read().fishing.trace {
                        submit_ml_frame(
                            ctx,
                            &frame,
                            crate::core::ml_dataset::MlTask::UiDetection,
                            Some(crate::core::ml_dataset::UiLabel::FishingBar),
                            Some(crate::core::ml_dataset::GameStateLabel::Bite),
                            "",
                            "bar",
                            None,
                            None,
                        );
                    }
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

pub(crate) fn check_spawn(ctx: &Ctx, last_hash: &mut u64) {
    if !ctx.platform.ocr.available() {
        return;
    }
    let cooldown = Duration::from_secs_f32(ctx.settings.read().ocr.spawn_cooldown_s);
    if let Some(t) = ctx.session.lock().last_spawn_alert {
        if t.elapsed() < cooldown {
            return;
        }
    }
    // Use wide banner region across top of screen (tunable via
    // Settings → game.spawn_banner) to ensure wide ASE text is captured.
    let banner_region = ctx.settings.read().game.spawn_banner;
    let Some(frame) = grab_region(ctx, banner_region) else { return };
    let hash = frame.average_hash();
    if hash == *last_hash {
        return;
    }
    *last_hash = hash;
    let Ok(text) = ctx.platform.ocr.read(&frame) else { return };
    if text.trim().is_empty() {
        return;
    }
    ctx.log_debug(&format!("Spawn banner OCR: {}", text.trim()));
    let lex = ctx.settings.read().lexicon.clone();
    if let Some(mut info) = fruit::detect_spawn(&lex, &text) {
        ctx.log_info(&format!("Fruit spawned: {}", info.label()));
        {
            let mut s = ctx.session.lock();
            s.last_spawn = Some(info.label());
            s.last_spawn_alert = Some(Instant::now());
        }

        let photo_bytes = if ctx.settings.read().webhook.send_screenshot {
            frame.to_png_bytes().ok()
        } else {
            None
        };

        // If Gemini is enabled, rewrite message or resolve fruit name with Gemini
        let s = ctx.settings.read();
        if s.gemini.enabled && !s.gemini.api_key.trim().is_empty() {
            if let Ok(gemini_msg) = crate::core::gemini::rewrite_spawn_message_gemini(
                info.name.as_deref(),
                info.location.as_deref(),
                info.is_ase,
                &text,
                &s.gemini.api_key,
                &s.gemini.model,
            ) {
                info.custom_telegram_message = Some(gemini_msg);
            }
        }
        drop(s);

        ctx.emit_stats();
        ctx.emit(BotEvent::FruitSpawn(info.clone()));
        if ctx.settings.read().webhook.spawn {
            ctx.webhook.spawn(&info, photo_bytes);
        }
    }
}

/// Submit one frame to the async ML collector (never blocks: bounded queue
/// sheds load when full). No active session or trace off → counted drop.
#[allow(clippy::too_many_arguments)]
fn submit_ml_frame(
    ctx: &Ctx,
    frame: &Frame,
    task: crate::core::ml_dataset::MlTask,
    ui_label: Option<crate::core::ml_dataset::UiLabel>,
    game_state: Option<crate::core::ml_dataset::GameStateLabel>,
    ocr_text: &str,
    region: &str,
    entity_id: Option<String>,
    hard_reason: Option<String>,
) {
    if ctx.ml.active_session_id().is_none() {
        return;
    }
    let png = match frame.to_png_bytes() {
        Ok(b) => b,
        Err(_) => return,
    };
    ctx.ml.submit(crate::bot::ml_collect::SampleJob {
        png,
        session_id: String::new(),
        task,
        ui_label,
        game_state,
        ocr_text: ocr_text.to_string(),
        region: region.to_string(),
        entity_id,
        hard_reason,
        source: "gameplay".to_string(),
    });
}

/// Submit one RESULT sample per ended reel (async, trace-gated).
/// Hard-example mining: empty OCR, unverdictable catches, rules/knowledge
/// disagreement, and perception-unknown are retained with reasons so the
/// dataset naturally accumulates difficult cases during normal play.
fn maybe_collect_ml_sample(ctx: &Ctx, verdict: &fruit::CatchVerdict, text: &str) {
    let (trace_on, drop_region, fuzzy_threshold, lex) = {
        let s = ctx.settings.read();
        (s.fishing.trace, s.regions.drop, s.lexicon.fuzzy_threshold, s.lexicon.clone())
    };
    if !trace_on {
        return;
    }
    ctx.ml.count_reel();
    let frame = match grab_region(ctx, drop_region) {
        Some(f) => f,
        None => return,
    };
    let kb = ctx.store.effective_knowledge();
    let obs = crate::core::perception::correlate_text(
        &kb,
        text,
        "drop",
        crate::core::perception::ScreenKind::Fishing,
        fuzzy_threshold,
        0.80,
        None,
    );
    let rules_say_drop = fruit::detect_drop(&lex, text).is_some();
    let kb_says_fruit = obs.entity.as_ref().is_some_and(|m| m.category == "fruit");
    let mut hard_reasons: Vec<&str> = Vec::new();
    if text.trim().is_empty() {
        hard_reasons.push("empty OCR on catch");
    }
    if *verdict == fruit::CatchVerdict::Unknown {
        hard_reasons.push("unverdictable catch");
    }
    if rules_say_drop != kb_says_fruit {
        hard_reasons.push("rules/knowledge disagreement");
    }
    if obs.entity.is_none() {
        hard_reasons.push("perception unknown");
    }
    let hard = if hard_reasons.is_empty() { None } else { Some(hard_reasons.join("; ")) };
    // `EntityMatch.entity_id` is ALREADY the canonical KB id perception
    // resolved (it is `entity.id.clone()` of the entity it actually matched),
    // so it is the authoritative value for the label. Re-resolving
    // `canonical_name` is NOT equivalent: the KB name index is first-wins
    // (`or_insert` over canonical name + aliases + OCR aliases), so an alias
    // key can already belong to a DIFFERENT entity and would relabel this row
    // with the wrong id — silent ground-truth corruption in the training set.
    // Name lookup is therefore only a fallback for a match that carries no id
    // at all, and a failed lookup stays `None` (unknown) rather than inventing
    // one: a missing label is honestly downstream-handled (hard/unknown),
    // whereas a fabricated id is not detectable from the row alone.
    let entity: Option<String> = obs.entity.as_ref().and_then(|m| {
        if !m.entity_id.trim().is_empty() {
            Some(m.entity_id.clone())
        } else {
            kb.find_by_name(&m.canonical_name).map(|e| e.id.clone())
        }
    });
    submit_ml_frame(
        ctx,
        &frame,
        crate::core::ml_dataset::MlTask::EntityRecognition,
        None,
        Some(crate::core::ml_dataset::GameStateLabel::CatchResult),
        text,
        "drop",
        entity.clone(),
        hard,
    );
    // Shadow FISH observation on the same RESULT frame (once per reel;
    // agreement vs the OCR+KB entity above; vision never decides).
    crate::bot::shadow::observe_fish(ctx, &frame, entity.as_deref(), text);
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

        let is_pity_zero = fruit::is_pity_zero(d.pity.as_deref(), &d.text);
        let is_known_legendary_or_mythical = rarity == fruit::FruitRarity::Legendary
            || rarity == fruit::FruitRarity::Mythical;

        let is_high_tier = is_known_legendary_or_mythical || is_pity_zero;
        // v5.3.0: single source of truth for the protection rule (proven
        // equivalent to the legacy inline computation by policy tests).
        let policy = crate::core::policy::decide_fruit_policy(
            rarity,
            is_pity_zero,
            s.fruit_storage.never_drop_legendary_or_mythical,
            s.fruit_storage.keep_pity_zero_fruit,
        );
        let is_protected = policy.protects_valuable;

        // Correlate this RESULT into the workflow event stream
        // (observation + logging only — never influences control flow).
        let session_id = ctx.ml.active_session_id().unwrap_or_else(|| "no-session".to_string());
        let workflow_id = format!("wf-{session_id}-{}", crate::events::now_ms());
        {
            use crate::core::workflow::{ActionEvent, ConfirmationState};
            // `ActionEvent.entity_id` is a stable knowledge-base id
            // (`fruit:suna`), and this event is persisted into the workflow
            // log — so it must be RESOLVED, never synthesised. `fruit_name`
            // is an OCR/Gemini display string, and `format!("fruit:{…}")`
            // over it fabricated an id for any unknown or misspelled catch
            // (e.g. a Gemini-provided name), poisoning downstream linkage.
            // Resolve against the KB; an unrecognised catch stays `None`
            // (honest "unknown entity") instead of a plausible-looking lie.
            let canonical_entity_id: Option<String> =
                ctx.store.effective_knowledge().find_by_name(&fruit_name).map(|e| e.id.clone());
            let policy_event = ActionEvent {
                workflow_id: workflow_id.clone(),
                event_id: format!("{workflow_id}#result"),
                frame_index: None,
                session_id: session_id.clone(),
                result_event_id: None,
                entity_id: canonical_entity_id,
                entity_type: Some("fruit".to_string()),
                policy_decision: Some(policy.action.as_str().to_string()),
                action_requested: None,
                action_sent_at: None,
                confirmation_state: ConfirmationState::Pending,
                confirmation_evidence: Some(policy.reason.clone()),
                confirmation_at: None,
                retry_count: 0,
                final_outcome: None,
            };
            let _ = crate::core::workflow::append_action_event(ctx.store.dir(), &policy_event);
        }

        let label = if rarity == fruit::FruitRarity::Mythical {
            format!("Mythical devil fruit ({fruit_name})")
        } else if is_known_legendary_or_mythical {
            format!("Legendary devil fruit ({fruit_name})")
        } else if is_pity_zero {
            format!("Guaranteed Pity 0 devil fruit ({fruit_name})")
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
            if is_pity_zero {
                sess.pity_legendary = 0;
            }
        }
        ctx.record_catch("fruit", &fruit_name, &d.text);
        ctx.emit_stats();
        ctx.emit(BotEvent::FruitDrop(d.clone()));

        // Screenshot on catch: Sent ONLY when pity is actually 0, or when getting a verified Legendary/Mythical fruit!
        let wants_catch_photo = s.webhook.send_screenshot && (
            s.webhook.send_catch_screenshot
            || is_pity_zero
            || is_known_legendary_or_mythical
        );

        let photo = if wants_catch_photo {
            actions::capture_fruit_screenshot(ctx, &s)
        } else {
            None
        };
        if s.webhook.fruit_drop && (is_high_tier || !s.webhook.legendary_only) {
            ctx.webhook.fruit_drop(&d, photo);
        }
        if is_protected {
            if is_pity_zero {
                ctx.log_info(&format!("🛡️ Pity reached 0! Protecting {label} - drop/backspace strictly prevented"));
            } else {
                ctx.log_info(&format!("🛡️ Protected {label} - preventing drop"));
            }
        }
        if !actions::store_fruit(ctx, &fruit_name, is_protected, is_high_tier, &workflow_id) {
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
    use std::path::PathBuf;
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

    static ML_PIPE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn make_collect_ctx(
        frames: Vec<Frame>,
        ocr_texts: Vec<String>,
    ) -> (Arc<Ctx>, PathBuf) {
        let n = ML_PIPE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("gpo-mlpipe-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut settings = Settings::default();
        settings.fishing.scan_timeout_s = 5.0;
        settings.fishing.scan_hz = 200;
        settings.fishing.track_hz = 200;
        settings.fishing.min_track_s = 0.0;
        settings.fishing.trace = true; // collection armed
        settings.watchdog.enabled = false;
        let cap = Arc::new(ScriptedCapture::default());
        for f in frames {
            cap.push(f);
        }
        let ocr = Arc::new(ScriptedOcr::default());
        for t in ocr_texts {
            ocr.texts.lock().push_back(t);
        }
        let platform = Platform {
            window: Arc::new(crate::core::platform::mock::MockWindow::default()),
            capture: cap,
            input: Arc::new(RecordingInput::default()),
            ocr,
        };
        let roblox = Arc::new(RwLock::new(Some(WindowInfo {
            client: PxRect { x: 0, y: 0, w: 1920, h: 1080 },
            is_foreground: true,
            visible: true,
            dpi: 96,
        })));
        let store = Arc::new(crate::config::Store::new(dir.clone()));
        let ctx = Arc::new(Ctx::new(
            platform,
            Arc::new(RwLock::new(settings)),
            roblox,
            tx,
            Arc::new(WebhookQueue::disabled()),
            store,
        ));
        ctx.running.store(true, std::sync::atomic::Ordering::SeqCst);
        (ctx, dir)
    }

    fn reel_frames() -> Vec<Frame> {
        let mut v = Vec::new();
        for _ in 0..3 {
            v.push(blank());
        }
        for _ in 0..5 {
            v.push(bar_frame(40));
        }
        for _ in 0..3 {
            v.push(bar_frame(140));
        }
        for _ in 0..10 {
            v.push(blank());
        }
        v
    }

    #[test]
    fn reel_end_collects_real_pipeline_sample() {
        // Full production path with scripted frames (plumbing test, not
        // accuracy): fish_cycle WAITING/BITE hooks + verify + RESULT hook →
        // async writer → dataset rows + valid PNGs on disk.
        let texts = vec!["You caught a Suna devil fruit!".to_string(); 12];
        let (ctx, dir) = make_collect_ctx(reel_frames(), texts);
        ctx.ml.begin_session("test-4.3.0");
        let mut tracker = Tracker::default();
        let mut hash = 0;
        let mut t = Instant::now();
        let out = fish_cycle(&ctx, &mut tracker, &mut hash, &mut t);
        assert!(matches!(out, Outcome::Ended), "scripted reel must end");
        let (verdict, text) = verify_catch(&ctx);
        maybe_collect_ml_sample(&ctx, &verdict, &text);
        let summary = ctx.ml.end_session().expect("session summary");
        assert!(summary.samples >= 2, "WAITING + RESULT samples expected, got {:?}", summary);
        assert_eq!(summary.dropped, 0, "bounded queue must not shed at reel rate");
        // Rows on disk with valid PNGs; session finalized complete.
        let ds = crate::core::ml_dataset::MlDatasetStore::new(dir.clone());
        let rows = ds.annotations();
        assert!(!rows.is_empty());
        for row in &rows {
            let p = ds.root().join("images").join(format!("{}.png", row.image_id));
            assert!(crate::core::ml_dataset::png_file_valid(&p), "sample PNG must decode");
        }
        assert!(rows.iter().any(|r| r.region_name == "bar"), "episode WAITING/BITE frames expected");
        // The RESULT reading merges into the identical-pixel row (dedup):
        // its OCR text + entity must survive, not be lost.
        assert!(
            rows.iter().any(|r| r.ocr_text.contains("Suna")),
            "RESULT OCR text must persist, got: {:?}",
            rows.iter().map(|r| r.ocr_text.clone()).collect::<Vec<_>>()
        );
        assert!(
            rows.iter().any(|r| r.entity_id.as_deref() == Some("fruit:suna")),
            "RESULT entity must persist"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_ocr_reel_is_mined_as_hard_example() {
        // No OCR text at all: verdict Unknown + empty text must yield a
        // hard-flagged RESULT row (never silently dropped, never faked).
        let (ctx, dir) = make_collect_ctx(reel_frames(), vec![]);
        ctx.ml.begin_session("test-4.3.0");
        let mut tracker = Tracker::default();
        let mut hash = 0;
        let mut t = Instant::now();
        let out = fish_cycle(&ctx, &mut tracker, &mut hash, &mut t);
        assert!(matches!(out, Outcome::Ended));
        let (verdict, text) = verify_catch(&ctx);
        assert_eq!(verdict, fruit::CatchVerdict::Unknown);
        maybe_collect_ml_sample(&ctx, &verdict, &text);
        ctx.ml.end_session().expect("summary");
        let ds = crate::core::ml_dataset::MlDatasetStore::new(dir.clone());
        let rows = ds.annotations();
        assert!(!rows.is_empty(), "episode frames must be stored");
        let hard = rows.iter().find(|r| r.hard_example).expect("a hard-flagged row");
        assert!(hard.hard_reason.as_deref().unwrap_or("").contains("empty OCR"), "got: {:?}", hard.hard_reason);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn drain_ml_session(rx: &crossbeam_channel::Receiver<BotEvent>) -> Vec<crate::events::MlSessionState> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let BotEvent::MlSession(st) = ev {
                out.push(st);
            }
        }
        out
    }

    #[test]
    fn bot_start_stop_drives_ml_collection_session() {
        static C: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = C.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("gpo-bot-ml-{n}"));
        let _ = std::fs::remove_dir_all(&dir);

        let (tx, rx) = crossbeam_channel::unbounded();
        let mut settings = Settings::default();
        settings.fishing.scan_timeout_s = 1.0;
        settings.fishing.scan_hz = 200;
        settings.fishing.track_hz = 200;
        settings.watchdog.enabled = false;
        settings.fishing.trace = true; // collection armed
        let settings = Arc::new(RwLock::new(settings));
        let cap = Arc::new(ScriptedCapture::default());
        for _ in 0..200 {
            cap.push(blank());
        }
        let platform = Platform {
            window: Arc::new(crate::core::platform::mock::MockWindow::default()),
            capture: cap,
            input: Arc::new(RecordingInput::default()),
            ocr: Arc::new(ScriptedOcr::default()),
        };
        let roblox = Arc::new(RwLock::new(Some(WindowInfo {
            client: PxRect { x: 0, y: 0, w: 1920, h: 1080 },
            is_foreground: true,
            visible: true,
            dpi: 96,
        })));
        let store = Arc::new(crate::config::Store::new(dir.clone()));
        let bot = crate::bot::Bot::new(
            platform,
            Arc::clone(&settings),
            roblox,
            tx,
            Arc::new(WebhookQueue::disabled()),
            Arc::clone(&store),
        );

        // START MACRO → collector starts (no separate recording mode).
        bot.start();
        assert!(bot.ctx().ml.is_collecting());
        let session_id = bot.ctx().ml.active_session_id().expect("session id");
        let started = drain_ml_session(&rx);
        assert!(
            started.iter().any(|s| s.collecting && s.session_id.as_deref() == Some(session_id.as_str())),
            "must emit collecting state on start"
        );

        // Let a reel cycle run (blank frames → quick timeouts, WAITING
        // samples submitted each cycle).
        std::thread::sleep(std::time::Duration::from_millis(2500));

        // STOP MACRO → collector finalizes with a summary event.
        bot.stop();
        assert!(!bot.ctx().ml.is_collecting());
        let mut saw_final = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline && !saw_final {
            for st in drain_ml_session(&rx) {
                if !st.collecting && st.session_id.as_deref() == Some(session_id.as_str()) {
                    saw_final = true;
                }
            }
            if !saw_final {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        assert!(saw_final, "must emit finalized session on stop");

        // Session file finalized on disk; dataset rows appended, none lost.
        let meta_path = dir.join("sessions").join(format!("{session_id}.json"));
        let meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&meta_path).expect("session file")).unwrap();
        assert_eq!(meta.get("complete").and_then(|v| v.as_bool()), Some(true));

        // RESTART → brand-new session, old session file preserved.
        bot.start();
        let session2 = bot.ctx().ml.active_session_id().expect("session 2");
        assert_ne!(session_id, session2);
        bot.stop();
        assert!(dir.join("sessions").join(format!("{session_id}.json")).exists());
        assert!(dir.join("sessions").join(format!("{session2}.json")).exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bot_start_with_trace_off_reports_not_collecting() {
        // Regression: hours were fished with trace recording OFF while the
        // UI showed "ML DATA: COLLECTING". A session shell may still be
        // created, but the emitted UI state must never claim collecting
        // without consent — otherwise the macro silently runs as if
        // collection were active.
        static C: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = C.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("gpo-bot-ml-off-{n}"));
        let _ = std::fs::remove_dir_all(&dir);

        let (tx, rx) = crossbeam_channel::unbounded();
        let mut settings = Settings::default();
        settings.fishing.scan_timeout_s = 1.0;
        settings.fishing.scan_hz = 200;
        settings.fishing.track_hz = 200;
        settings.watchdog.enabled = false;
        assert!(!settings.fishing.trace, "trace defaults off (explicit consent)");
        let settings = Arc::new(RwLock::new(settings));
        let cap = Arc::new(ScriptedCapture::default());
        for _ in 0..5 {
            cap.push(blank());
        }
        let platform = Platform {
            window: Arc::new(crate::core::platform::mock::MockWindow::default()),
            capture: cap,
            input: Arc::new(RecordingInput::default()),
            ocr: Arc::new(ScriptedOcr::default()),
        };
        let roblox = Arc::new(RwLock::new(Some(WindowInfo {
            client: PxRect { x: 0, y: 0, w: 1920, h: 1080 },
            is_foreground: true,
            visible: true,
            dpi: 96,
        })));
        let store = Arc::new(crate::config::Store::new(dir.clone()));
        let bot = crate::bot::Bot::new(
            platform,
            Arc::clone(&settings),
            roblox,
            tx,
            Arc::new(WebhookQueue::disabled()),
            Arc::clone(&store),
        );

        bot.start();
        // Session shell exists (macro-run bookkeeping)…
        let session_id = bot.ctx().ml.active_session_id().expect("session shell");
        // …but the UI-facing state must not claim collecting.
        let states = drain_ml_session(&rx);
        assert!(!states.is_empty(), "must emit session state on start");
        assert!(
            states.iter().all(|s| !s.collecting),
            "trace off must never report collecting, got: {states:?}"
        );

        bot.stop();
        // Finalized shell on disk with zero samples (nothing faked).
        let meta_path = dir.join("sessions").join(format!("{session_id}.json"));
        let meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&meta_path).expect("session file")).unwrap();
        assert_eq!(meta.get("complete").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(meta.get("samples").and_then(|v| v.as_u64()), Some(0));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_vip_url_cannot_smuggle_a_second_command_into_cmd() {
        // `cmd /C start "" <url>` re-parses its argument string, so argv
        // quoting does NOT protect it. `x&calc` has no whitespace, so Rust
        // passes it unquoted and cmd runs `calc`. vip_server_url is writable
        // from the token-authenticated LAN dashboard.
        for bad in [
            "x&calc",
            "https://roblox.com/games/1&calc",
            "https://roblox.com/games/1|calc",
            "https://roblox.com/games/1^&calc",
            "https://roblox.com/games/1\ncalc",
            "https://roblox.com/games/1\rcalc",
            "https://roblox.com/games/1 calc",
            "https://roblox.com/games/1;calc",
            "https://roblox.com/games/1$(calc)",
            "https://roblox.com/games/1`calc`",
            "https://roblox.com/games/1\"calc\"",
        ] {
            assert!(
                validate_vip_server_url(bad).is_err(),
                "must refuse {bad:?}"
            );
        }
    }

    #[test]
    fn a_real_roblox_vip_url_is_accepted_and_empty_is_allowed() {
        assert_eq!(
            validate_vip_server_url("https://www.roblox.com/games/12345/Fish-It").unwrap(),
            "https://www.roblox.com/games/12345/Fish-It"
        );
        // Empty means "no VIP configured": the caller skips the launch, so it
        // must not be an error.
        assert_eq!(validate_vip_server_url("").unwrap(), "");
        assert_eq!(validate_vip_server_url("   ").unwrap(), "");
        // Non-HTTPS and non-Roblox are refused even without metacharacters.
        assert!(validate_vip_server_url("http://www.roblox.com/games/1").is_err());
        assert!(validate_vip_server_url("https://evil.example.com/games/1").is_err());
        assert!(validate_vip_server_url(&"h".repeat(600)).is_err());
    }
}
