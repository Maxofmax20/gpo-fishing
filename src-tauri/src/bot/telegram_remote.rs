use std::sync::Arc;
use std::time::Duration;
use parking_lot::RwLock;
use serde_json::Value;

use crate::bot::Bot;
use crate::config::Settings;
use crate::webhook::{post_telegram, post_telegram_photo};

pub fn spawn(bot: Arc<Bot>, settings: Arc<RwLock<Settings>>) {
    std::thread::Builder::new()
        .name("telegram-remote".into())
        .spawn(move || run_poller(bot, settings))
        .expect("spawn telegram remote poller");
}

fn run_poller(bot: Arc<Bot>, settings: Arc<RwLock<Settings>>) {
    let mut offset: i64 = 0;
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(25))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to create telegram http client: {e}");
            return;
        }
    };

    let mut registered_token = String::new();
    let mut last_auto_scan = std::time::Instant::now();
    let mut boss_tracker = crate::core::boss_tracker::BossTracker::new();

    // Initialize boss tracker offsets from settings
    {
        let s = settings.read();
        if let Some(ts) = s.boss_tracker.hawkeye_offset {
            boss_tracker.set_offset(crate::core::boss_tracker::BossId::HawkEye, ts);
        }
        if let Some(ts) = s.boss_tracker.roger_offset {
            boss_tracker.set_offset(crate::core::boss_tracker::BossId::Roger, ts);
        }
        if let Some(ts) = s.boss_tracker.soulking_offset {
            boss_tracker.set_offset(crate::core::boss_tracker::BossId::SoulKing, ts);
        }
        if let Some(ts) = s.boss_tracker.radiant_admiral_offset {
            boss_tracker.set_offset(crate::core::boss_tracker::BossId::RadiantAdmiral, ts);
        }
        if let Some(ts) = s.boss_tracker.merchant_offset {
            boss_tracker.set_offset(crate::core::boss_tracker::BossId::TravellingMerchant, ts);
        }
    }

    loop {
        let (token, expected_chat, remote_enabled) = {
            let s = settings.read();
            (
                s.webhook.telegram_bot_token.trim().to_string(),
                s.webhook.telegram_chat_id.trim().to_string(),
                s.features.telegram_remote,
            )
        };

        if !remote_enabled || token.is_empty() || expected_chat.is_empty() {
            std::thread::sleep(Duration::from_secs(4));
            continue;
        }

        if registered_token != token {
            register_bot_commands(&client, &token);
            registered_token = token.clone();
        }

        // Boss Tracker Tick: Runs 24/7 in background even when fishing is stopped
        let (boss_enabled, notify_5m, notify_spawn) = {
            let s = settings.read();
            (
                s.boss_tracker.enabled,
                s.boss_tracker.notify_5m,
                s.boss_tracker.notify_spawn,
            )
        };

        if boss_enabled {
            let now = crate::core::boss_tracker::now_sec();
            let alerts = boss_tracker.tick(now, notify_5m, notify_spawn);
            for alert in alerts {
                let boss_specific_enabled = {
                    let s = settings.read();
                    s.boss_tracker.is_boss_enabled(alert.boss)
                };
                if !boss_specific_enabled {
                    continue;
                }

                let alert_text = match alert.alert_type {
                    crate::core::boss_tracker::AlertType::Warning5m => {
                        format!(
                            "⏰ {} <b>{}</b> will spawn in <b>5 minutes</b>!\n📍 Location: <b>{}</b>\nℹ️ {}",
                            alert.boss.emoji(),
                            alert.boss.name(),
                            alert.boss.location(),
                            alert.boss.despawn_info(),
                        )
                    }
                    crate::core::boss_tracker::AlertType::Spawned => {
                        format!(
                            "🚨 {} <b>{}</b> has <b>SPAWNED</b>!\n📍 Location: <b>{}</b>\nℹ️ {}",
                            alert.boss.emoji(),
                            alert.boss.name(),
                            alert.boss.location(),
                            alert.boss.despawn_info(),
                        )
                    }
                };
                let _ = post_telegram(&token, &expected_chat, &alert_text);
            }

            // Periodic auto-sync of Travelling Merchant via in-game Server Age OCR (every 5 minutes)
            if last_auto_scan.elapsed() >= Duration::from_secs(300) {
                last_auto_scan = std::time::Instant::now();
                let ctx = bot.ctx();
                if let Ok((uptime_sec, time_str, rem, is_spawned)) = crate::bot::actions::scan_server_age(&ctx) {
                    let target_offset = if is_spawned {
                        now + rem + 1800
                    } else {
                        now + rem
                    };
                    boss_tracker.set_offset(crate::core::boss_tracker::BossId::TravellingMerchant, target_offset);
                    let mut s = settings.write();
                    s.boss_tracker.merchant_offset = Some(target_offset);
                    let _ = bot.ctx().store.save(&s);
                    tracing::info!("Auto-synced Travelling Merchant from in-game Server Age: {time_str} (uptime: {uptime_sec}s)");
                }
            }
        }

        let url = format!(
            "https://api.telegram.org/bot{token}/getUpdates?offset={offset}&timeout=15&limit=10"
        );

        let resp = match client.get(&url).send() {
            Ok(r) => r,
            Err(_) => {
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
        };

        let json: Value = match resp.json() {
            Ok(v) => v,
            Err(_) => {
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };

        if let Some(results) = json.get("result").and_then(|r| r.as_array()) {
            for update in results {
                if let Some(up_id) = update.get("update_id").and_then(|id| id.as_i64()) {
                    offset = up_id + 1;
                }

                let Some(msg) = update.get("message") else {
                    continue;
                };

                let chat_id = msg
                    .get("chat")
                    .and_then(|c| c.get("id"))
                    .map(|id| id.to_string())
                    .unwrap_or_default();

                // Security check: only accept messages from configured chat
                let chat_id_clean = chat_id.trim_matches('"').to_string();
                if chat_id_clean != expected_chat && !expected_chat.is_empty() {
                    tracing::warn!("Telegram remote: ignored message from unauthorized chat {chat_id_clean}");
                    continue;
                }

                let text = msg
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default()
                    .trim();

                handle_command(&bot, &settings, &mut boss_tracker, &token, &chat_id_clean, text);
            }
        }

        std::thread::sleep(Duration::from_millis(500));
    }
}

fn handle_command(
    bot: &Arc<Bot>,
    settings: &Arc<RwLock<Settings>>,
    boss_tracker: &mut crate::core::boss_tracker::BossTracker,
    token: &str,
    chat_id: &str,
    text: &str,
) {
    let lower = text.to_lowercase();
    let cmd = lower.split_whitespace().next().unwrap_or("");
    let cmd_clean = cmd.split('@').next().unwrap_or(""); // Handle e.g. /status@bot_name

    match cmd_clean {
        "/status" | "/stats" | "status" => {
            let state = bot.state();
            let paused = bot.is_paused();
            let stats = bot.ctx().session.lock().stats();
            let runtime_s = stats.runtime_s;
            let h = runtime_s / 3600;
            let m = (runtime_s % 3600) / 60;
            let s = runtime_s % 60;
            let state_str = if paused {
                "⏸️ Paused"
            } else {
                match state {
                    crate::events::BotState::Stopped => "⏹️ Stopped",
                    crate::events::BotState::WaitingForRoblox => "⏳ Waiting for Roblox",
                    crate::events::BotState::Tracking => "🎣 Tracking Bar",
                    crate::events::BotState::WaitingForBite => "🐟 Waiting for Bite",
                    crate::events::BotState::Casting => "🎯 Casting",
                    crate::events::BotState::Purchasing => "🛒 Purchasing Bait",
                    crate::events::BotState::StoringFruit => "🍇 Storing Fruit",
                    crate::events::BotState::Recovering => "🔄 Recovering",
                    _ => "🟢 Running",
                }
            };

            let last_fruit_str = stats.last_fruit.as_deref().unwrap_or("None yet");
            let last_spawn_str = stats.last_spawn.as_deref().unwrap_or("None yet");

            let caption = format!(
                "🤖 <b>GPO Autofish Status</b>\n\n\
                 • <b>State</b>: {state_str}\n\
                 • <b>Runtime</b>: {h:02}:{m:02}:{s:02}\n\
                 • <b>Fish Caught</b>: {} ({:.0}% success)\n\
                 • <b>Devil Fruits</b>: {}\n\
                 • <b>Fruit Pity</b>: ⚡ {} fish\n\
                 • <b>Legendary Pity</b>: 🌟 {} fish\n\
                 • <b>Bait Purchased</b>: {}\n\
                 • <b>Last Fruit</b>: {last_fruit_str}\n\
                 • <b>Last Spawn</b>: {last_spawn_str}",
                stats.fish,
                stats.success_rate * 100.0,
                stats.fruits,
                stats.pity_fruit,
                stats.pity_legendary,
                stats.bait_purchased,
            );

            // Capture screenshot of Roblox window
            let screenshot = bot
                .ctx()
                .roblox_rect()
                .and_then(|r| bot.ctx().platform.capture.grab(r).ok())
                .map(|f| f.downscale(1280))
                .and_then(|f| f.to_png_bytes().ok());

            if let Some(bytes) = screenshot {
                let _ = post_telegram_photo(token, chat_id, &bytes, &caption);
            } else {
                let _ = post_telegram(token, chat_id, &caption);
            }
        }
        "/screenshot" | "screenshot" => {
            let screenshot = bot
                .ctx()
                .roblox_rect()
                .and_then(|r| bot.ctx().platform.capture.grab(r).ok())
                .map(|f| f.downscale(1280))
                .and_then(|f| f.to_png_bytes().ok());

            if let Some(bytes) = screenshot {
                let _ = post_telegram_photo(token, chat_id, &bytes, "📸 <b>Current Roblox Screen</b>");
            } else {
                let _ = post_telegram(
                    token,
                    chat_id,
                    "⚠️ <b>Roblox window not detected or capture failed.</b>",
                );
            }
        }
        "/pity" | "pity" => {
            let stats = bot.ctx().session.lock().stats();
            let text = format!(
                "⚡ <b>GPO Pity Status</b>\n\n\
                 • <b>Fruit Pity</b>: <b>{}</b> fish (since last fruit)\n\
                 • <b>Legendary Pity</b>: <b>{}</b> fish\n\
                 • <b>Fruits Caught</b>: <b>{}</b>\n\
                 • <b>Total Fish</b>: <b>{}</b>\n\
                 • <b>Last Fruit</b>: {}",
                stats.pity_fruit,
                stats.pity_legendary,
                stats.fruits,
                stats.fish,
                stats.last_fruit.as_deref().unwrap_or("None yet")
            );
            let _ = post_telegram(token, chat_id, &text);
        }
        "/recast" | "recast" => {
            bot.recast();
            let _ = post_telegram(token, chat_id, "🔄 <b>Rod recast triggered remotely!</b>");
        }
        "/update" | "update" => {
            let _ = post_telegram(token, chat_id, "🔍 <b>Checking for GPO Autofish updates...</b>");
            let cur_ver = env!("CARGO_PKG_VERSION");
            match check_and_apply_update(Some(bot), Some(token), Some(chat_id), cur_ver) {
                Ok(msg) => {
                    let _ = post_telegram(token, chat_id, &msg);
                }
                Err(e) => {
                    let _ = post_telegram(token, chat_id, &format!("⚠️ <b>Update check failed:</b> {e}"));
                }
            }
        }
        "/stop" | "/pause" | "stop" | "pause" => {
            if bot.is_running() {
                bot.pause();
                let _ = post_telegram(
                    token,
                    chat_id,
                    "🛑 <b>GPO Autofish has been paused remotely.</b>\nSend /start to resume.",
                );
            } else {
                let _ = post_telegram(
                    token,
                    chat_id,
                    "ℹ️ <b>Macro is not currently running.</b>\nSend /start to start it.",
                );
            }
        }
        "/start" | "/resume" | "start" | "resume" => {
            bot.start();
            let _ = post_telegram(
                token,
                chat_id,
                "▶️ <b>GPO Autofish started remotely!</b>\nSend /status to check live progress.",
            );
        }
        "/buybait" | "/buy" | "buybait" | "buy" => {
            let _ = post_telegram(token, chat_id, "🛒 <b>Triggering merchant bait purchase...</b>");
            if crate::bot::actions::purchase(&bot.ctx()) {
                let _ = post_telegram(token, chat_id, "✅ <b>Bait purchased successfully!</b> Resuming fishing.");
            } else {
                let _ = post_telegram(
                    token,
                    chat_id,
                    "⚠️ <b>Bait purchase failed.</b> Make sure Auto Purchase is enabled and merchant points are set in Setup.",
                );
            }
        }
        cmd if cmd.starts_with("/setbuy") || cmd.starts_with("setbuy") => {
            let parts: Vec<&str> = text.split_whitespace().collect();
            if let Some(num_str) = parts.get(1) {
                if let Ok(n) = num_str.parse::<u32>() {
                    if (1..=5000).contains(&n) {
                        {
                            let mut s = settings.write();
                            s.purchase.every_n_catches = n;
                            let _ = bot.ctx().store.save(&s);
                        }
                        let _ = post_telegram(
                            token,
                            chat_id,
                            &format!("⚙️ <b>Bait Purchase Interval updated!</b>\nMacro will now buy bait every <b>{n}</b> fish."),
                        );
                    } else {
                        let _ = post_telegram(token, chat_id, "⚠️ Number must be between 1 and 5000. Usage: <code>/setbuy 50</code>");
                    }
                } else {
                    let _ = post_telegram(token, chat_id, "⚠️ Invalid number. Usage: <code>/setbuy 50</code>");
                }
            } else {
                let current = settings.read().purchase.every_n_catches;
                let _ = post_telegram(token, chat_id, &format!("ℹ️ Current bait purchase interval: every <b>{current}</b> fish.\nTo change it, send e.g.: <code>/setbuy 50</code>"));
            }
        }
        cmd if cmd.starts_with("/setprogress") || cmd.starts_with("setprogress") => {
            let parts: Vec<&str> = text.split_whitespace().collect();
            if let Some(num_str) = parts.get(1) {
                if let Ok(n) = num_str.parse::<u32>() {
                    if (1..=5000).contains(&n) {
                        {
                            let mut s = settings.write();
                            s.webhook.progress_every_n = n;
                            let _ = bot.ctx().store.save(&s);
                        }
                        let _ = post_telegram(
                            token,
                            chat_id,
                            &format!("📱 <b>Telegram Progress Interval updated!</b>\nBot will now send updates every <b>{n}</b> fish."),
                        );
                    } else {
                        let _ = post_telegram(token, chat_id, "⚠️ Number must be between 1 and 5000. Usage: <code>/setprogress 100</code>");
                    }
                } else {
                    let _ = post_telegram(token, chat_id, "⚠️ Invalid number. Usage: <code>/setprogress 100</code>");
                }
            } else {
                let current = settings.read().webhook.progress_every_n;
                let _ = post_telegram(token, chat_id, &format!("ℹ️ Current progress update interval: every <b>{current}</b> fish.\nTo change it, send e.g.: <code>/setprogress 100</code>"));
            }
        }
        "/bosses" | "/timers" | "/boss" | "bosses" | "timers" | "boss" => {
            let now = crate::core::boss_tracker::now_sec();
            let msg = {
                let s = settings.read();
                boss_tracker.format_status_message(now, |b| s.boss_tracker.is_boss_enabled(b))
            };
            let _ = post_telegram(token, chat_id, &msg);
        }
        cmd if cmd.starts_with("/volume") || cmd.starts_with("volume") || cmd.starts_with("/sound") || cmd.starts_with("sound") => {
            let parts: Vec<&str> = text.split_whitespace().collect();
            if let Some(arg) = parts.get(1) {
                let arg_low = arg.to_lowercase();
                if arg_low == "max" || arg_low == "100" {
                    match crate::core::audio::set_volume(1.0) {
                        Ok(_) => {
                            let _ = post_telegram(token, chat_id, "🔊 <b>Windows Volume set to MAX (100%)</b>");
                        }
                        Err(e) => {
                            let _ = post_telegram(token, chat_id, &format!("⚠️ Failed to set volume: {e}"));
                        }
                    }
                } else if arg_low == "0" || arg_low == "zero" || arg_low == "min" || arg_low == "mute" {
                    match crate::core::audio::set_volume(0.0) {
                        Ok(_) => {
                            let _ = post_telegram(token, chat_id, "🔇 <b>Windows Volume set to ZERO (0% - Muted)</b>");
                        }
                        Err(e) => {
                            let _ = post_telegram(token, chat_id, &format!("⚠️ Failed to set volume: {e}"));
                        }
                    }
                } else if let Ok(val) = arg_low.trim_end_matches('%').parse::<f32>() {
                    let clamped = val.clamp(0.0, 100.0);
                    let scalar = clamped / 100.0;
                    match crate::core::audio::set_volume(scalar) {
                        Ok(new_vol) => {
                            let pct = (new_vol * 100.0).round() as u32;
                            let emoji = if pct == 0 { "🔇" } else if pct < 50 { "🔉" } else { "🔊" };
                            let _ = post_telegram(token, chat_id, &format!("{emoji} <b>Windows Volume set to {pct}%</b>"));
                        }
                        Err(e) => {
                            let _ = post_telegram(token, chat_id, &format!("⚠️ Failed to set volume: {e}"));
                        }
                    }
                } else {
                    let _ = post_telegram(token, chat_id, "⚠️ Invalid volume value. Usage: <code>/volume 0-100</code>, <code>/volume max</code>, or <code>/volume zero</code>");
                }
            } else {
                let vol_res = crate::core::audio::get_volume();
                let mute_res = crate::core::audio::is_muted();
                match (vol_res, mute_res) {
                    (Ok(vol), Ok(muted)) => {
                        let pct = (vol * 100.0).round() as u32;
                        let status_str = if muted || pct == 0 { "🔇 Muted" } else { "🔊 Unmuted" };
                        let _ = post_telegram(token, chat_id, &format!("🔊 <b>Windows Master Audio:</b>\n\n• Current Volume: <b>{pct}%</b>\n• State: {status_str}\n\n<i>To change volume, send:</i>\n<code>/volume 100</code> (max)\n<code>/volume 0</code> (zero/mute)\n<code>/volume 50</code> (50%)"));
                    }
                    _ => {
                        let _ = post_telegram(token, chat_id, "⚠️ Could not retrieve audio device status.");
                    }
                }
            }
        }
        "/unmute" | "unmute" => {
            match crate::core::audio::set_mute(false) {
                Ok(_) => {
                    let vol = crate::core::audio::get_volume().unwrap_or(0.5);
                    if vol <= 0.01 {
                        let _ = crate::core::audio::set_volume(0.5);
                    }
                    let current = (crate::core::audio::get_volume().unwrap_or(0.5) * 100.0).round() as u32;
                    let _ = post_telegram(token, chat_id, &format!("🔊 <b>Windows Audio UNMUTED!</b> (Volume: <b>{current}%</b>)"));
                }
                Err(e) => {
                    let _ = post_telegram(token, chat_id, &format!("⚠️ Failed to unmute audio: {e}"));
                }
            }
        }
        cmd if cmd.starts_with("/toggle") || cmd.starts_with("toggle") || cmd.starts_with("/mute") || cmd.starts_with("mute") => {
            let parts: Vec<&str> = text.split_whitespace().collect();
            if let Some(target) = parts.get(1) {
                let target_low = target.to_lowercase();
                if target_low == "sound" || target_low == "audio" || target_low == "pc" || target_low == "windows" {
                    let _ = crate::core::audio::set_mute(true);
                    let _ = post_telegram(token, chat_id, "🔇 <b>Windows Audio MUTED!</b>\nSend <code>/unmute</code> or <code>/volume max</code> to restore.");
                    return;
                }
                let reply = {
                    let mut s = settings.write();
                    let res = if target_low.contains("hawk") || target_low.contains("mihawk") {
                        s.boss_tracker.notify_hawkeye = !s.boss_tracker.notify_hawkeye;
                        let state = if s.boss_tracker.notify_hawkeye { "ENABLED 🔔" } else { "MUTED 🔕" };
                        format!("🦅 <b>Hawk Eye (Mihawk) alerts:</b> {state}")
                    } else if target_low.contains("roger") {
                        s.boss_tracker.notify_roger = !s.boss_tracker.notify_roger;
                        let state = if s.boss_tracker.notify_roger { "ENABLED 🔔" } else { "MUTED 🔕" };
                        format!("👑 <b>Roger alerts:</b> {state}")
                    } else if target_low.contains("soul") || target_low.contains("brook") {
                        s.boss_tracker.notify_soulking = !s.boss_tracker.notify_soulking;
                        let state = if s.boss_tracker.notify_soulking { "ENABLED 🔔" } else { "MUTED 🔕" };
                        format!("🎺 <b>Soul King (Brook) alerts:</b> {state}")
                    } else if target_low.contains("radiant") || target_low.contains("admiral") || target_low.contains("kizaru") {
                        s.boss_tracker.notify_radiant_admiral = !s.boss_tracker.notify_radiant_admiral;
                        let state = if s.boss_tracker.notify_radiant_admiral { "ENABLED 🔔" } else { "MUTED 🔕" };
                        format!("⚡ <b>Radiant Admiral (Kizaru) alerts:</b> {state}")
                    } else if target_low.contains("merchant") || target_low.contains("trader") {
                        s.boss_tracker.notify_merchant = !s.boss_tracker.notify_merchant;
                        let state = if s.boss_tracker.notify_merchant { "ENABLED 🔔" } else { "MUTED 🔕" };
                        format!("🛒 <b>Travelling Merchant alerts:</b> {state}")
                    } else if target_low.contains("fruit") || target_low.contains("spawn") || target_low.contains("ase") {
                        s.webhook.spawn = !s.webhook.spawn;
                        let state = if s.webhook.spawn { "ENABLED 🔔 (Active on Roblox)" } else { "STOPPED / MUTED 🔕" };
                        format!("🍇 <b>Devil Fruit & ASE Spawn Alerts:</b> {state}")
                    } else if target_low == "all" {
                        let any_on = s.boss_tracker.notify_hawkeye || s.boss_tracker.notify_roger || s.boss_tracker.notify_soulking || s.boss_tracker.notify_radiant_admiral || s.boss_tracker.notify_merchant;
                        let new_state = !any_on;
                        s.boss_tracker.notify_hawkeye = new_state;
                        s.boss_tracker.notify_roger = new_state;
                        s.boss_tracker.notify_soulking = new_state;
                        s.boss_tracker.notify_radiant_admiral = new_state;
                        s.boss_tracker.notify_merchant = new_state;
                        let text_state = if new_state { "ALL ENABLED 🔔" } else { "ALL MUTED 🔕" };
                        format!("🔔 <b>Boss Alerts:</b> {text_state}")
                    } else {
                        format!("⚠️ Unknown boss <b>{target}</b>.\nValid options: <code>hawkeye</code>, <code>roger</code>, <code>soulking</code>, <code>kizaru</code>, <code>merchant</code>, <code>all</code>, <code>sound</code>")
                    };
                    let _ = bot.ctx().store.save(&s);
                    res
                };
                let _ = post_telegram(token, chat_id, &reply);
            } else if cmd_clean == "/mute" || cmd_clean == "mute" {
                let _ = crate::core::audio::set_mute(true);
                let _ = post_telegram(token, chat_id, "🔇 <b>Windows Audio MUTED!</b>\nSend <code>/unmute</code> or <code>/volume max</code> to restore.");
            } else {
                let s = settings.read();
                let fmt_badge = |en: bool| if en { "ON 🔔" } else { "OFF 🔕" };
                let msg = format!(
                    "⚙️ <b>Boss Alert Notification Settings:</b>\n\n\
                    🦅 Hawk Eye (Mihawk): <b>{}</b>\n\
                    👑 Roger: <b>{}</b>\n\
                    🎺 Soul King (Brook): <b>{}</b>\n\
                    ⚡ Radiant Admiral (Kizaru): <b>{}</b>\n\
                    🛒 Travelling Merchant: <b>{}</b>\n\n\
                    <i>To toggle any alert on/off, send e.g.:</i>\n\
                    <code>/toggle roger</code>\n\
                    <code>/toggle hawkeye</code>\n\
                    <code>/toggle brook</code>\n\
                    <code>/toggle kizaru</code>\n\
                    <code>/toggle merchant</code>\n\
                    <code>/toggle all</code>",
                    fmt_badge(s.boss_tracker.notify_hawkeye),
                    fmt_badge(s.boss_tracker.notify_roger),
                    fmt_badge(s.boss_tracker.notify_soulking),
                    fmt_badge(s.boss_tracker.notify_radiant_admiral),
                    fmt_badge(s.boss_tracker.notify_merchant),
                );
                let _ = post_telegram(token, chat_id, &msg);
            }
        }
        cmd if cmd.starts_with("/sync") || cmd.starts_with("sync") || text.to_lowercase().contains("live spawn times") || text.to_lowercase().contains("event bosses") => {
            let now = crate::core::boss_tracker::now_sec();
            let text_low = text.to_lowercase();
            if text_low.contains("read") || text_low.contains("screen") || text_low.contains("ocr") {
                let ctx = bot.ctx();
                match crate::bot::actions::scan_server_age(&ctx) {
                    Ok((uptime_sec, time_str, rem, is_spawned)) => {
                        let target_offset = if is_spawned {
                            now + rem + 1800
                        } else {
                            now + rem
                        };
                        boss_tracker.set_offset(crate::core::boss_tracker::BossId::TravellingMerchant, target_offset);
                        {
                            let mut s = settings.write();
                            s.boss_tracker.merchant_offset = Some(target_offset);
                            let _ = bot.ctx().store.save(&s);
                        }
                        let status_str = if is_spawned {
                            format!("🛒 <b>Travelling Merchant is SPAWNED RIGHT NOW!</b>\n⏳ Despawns in: <b>{}</b>", crate::core::boss_tracker::format_duration(rem))
                        } else {
                            format!("🛒 <b>Travelling Merchant synced!</b>\n⏳ Next spawn in: <b>{}</b>", crate::core::boss_tracker::format_duration(rem))
                        };
                        let reply = format!(
                            "📷 <b>Auto-Read Server Age from Screen:</b>\n\n\
                             • Detected In-Game Timer: <code>{time_str}</code> (uptime: {uptime_sec}s)\n\
                             • {status_str}\n\n\
                             <i>Travelling Merchant countdown is now synchronized!</i>"
                        );
                        let _ = post_telegram(token, chat_id, &reply);
                    }
                    Err(e) => {
                        let _ = post_telegram(token, chat_id, &format!("⚠️ <b>Screen Scan Failed:</b> {e}\n\nMake sure Roblox is running on your screen!"));
                    }
                }
            } else {
                match boss_tracker.parse_sync_text(text, now) {
                    Ok(reply) => {
                        {
                            let mut s = settings.write();
                            s.boss_tracker.hawkeye_offset = boss_tracker.offsets.get(&crate::core::boss_tracker::BossId::HawkEye).copied();
                            s.boss_tracker.roger_offset = boss_tracker.offsets.get(&crate::core::boss_tracker::BossId::Roger).copied();
                            s.boss_tracker.soulking_offset = boss_tracker.offsets.get(&crate::core::boss_tracker::BossId::SoulKing).copied();
                            s.boss_tracker.radiant_admiral_offset = boss_tracker.offsets.get(&crate::core::boss_tracker::BossId::RadiantAdmiral).copied();
                            s.boss_tracker.merchant_offset = boss_tracker.offsets.get(&crate::core::boss_tracker::BossId::TravellingMerchant).copied();
                            let _ = bot.ctx().store.save(&s);
                        }
                        let _ = post_telegram(token, chat_id, &reply);
                    }
                    Err(err_msg) => {
                        let _ = post_telegram(token, chat_id, &err_msg);
                    }
                }
            }
        }
        cmd if cmd.starts_with("/brightness") || cmd.starts_with("brightness") || cmd.starts_with("/light") || cmd.starts_with("light") => {
            let parts: Vec<&str> = text.split_whitespace().collect();
            if let Some(arg) = parts.get(1) {
                let arg_low = arg.to_lowercase();
                if arg_low == "max" || arg_low == "100" {
                    match crate::core::brightness::set_brightness(100) {
                        Ok(_) => {
                            let _ = post_telegram(token, chat_id, "💡 <b>Screen brightness set to MAX (100%)</b>");
                        }
                        Err(e) => {
                            let _ = post_telegram(token, chat_id, &format!("⚠️ Failed to set brightness: {e}"));
                        }
                    }
                } else if arg_low == "0" || arg_low == "zero" || arg_low == "min" {
                    match crate::core::brightness::set_brightness(0) {
                        Ok(_) => {
                            let _ = post_telegram(token, chat_id, "💡 <b>Screen brightness set to MIN (0%)</b>");
                        }
                        Err(e) => {
                            let _ = post_telegram(token, chat_id, &format!("⚠️ Failed to set brightness: {e}"));
                        }
                    }
                } else if let Ok(val) = arg_low.trim_end_matches('%').parse::<u32>() {
                    let clamped = val.clamp(0, 100);
                    match crate::core::brightness::set_brightness(clamped) {
                        Ok(new_b) => {
                            let emoji = if new_b < 30 { "🌑" } else if new_b < 70 { "🌓" } else { "🌕" };
                            let _ = post_telegram(token, chat_id, &format!("{emoji} <b>Screen brightness set to {new_b}%</b>"));
                        }
                        Err(e) => {
                            let _ = post_telegram(token, chat_id, &format!("⚠️ Failed to set brightness: {e}"));
                        }
                    }
                } else {
                    let _ = post_telegram(token, chat_id, "⚠️ Invalid brightness value. Usage: <code>/brightness 0-100</code>, <code>/brightness max</code>, or <code>/brightness min</code>");
                }
            } else {
                let b = crate::core::brightness::get_brightness().unwrap_or(80);
                let _ = post_telegram(token, chat_id, &format!("💡 <b>Screen Brightness:</b>\n\n• Current Level: <b>{b}%</b>\n\n<i>To change brightness, send:</i>\n<code>/brightness 100</code> (max)\n<code>/brightness 30</code>\n<code>/brightness min</code>"));
            }
        }
        cmd if cmd.starts_with("/craft") || cmd.starts_with("craft") => {
            let parts: Vec<&str> = text.split_whitespace().collect();
            let sub = parts.get(1).map(|s| s.to_lowercase()).unwrap_or_else(|| "status".into());

            if sub == "stop" || sub == "cancel" {
                crate::bot::crafting::stop_auto_craft();
                let _ = post_telegram(token, chat_id, "🛑 <b>Auto-craft stopped.</b>");
            } else if sub == "rare" {
                let ctx_clone = bot.ctx().clone();
                match crate::bot::crafting::start_auto_craft(ctx_clone, crate::bot::crafting::CraftTier::Rare) {
                    Ok(_) => {
                        let _ = post_telegram(token, chat_id, "🔨 <b>Started Auto-Craft for Rare Fish Bait!</b>\nMake sure you are standing at Blacksmith Sen.");
                    }
                    Err(e) => {
                        let _ = post_telegram(token, chat_id, &format!("⚠️ {e}"));
                    }
                }
            } else if sub == "legendary" || sub == "leg" {
                let ctx_clone = bot.ctx().clone();
                match crate::bot::crafting::start_auto_craft(ctx_clone, crate::bot::crafting::CraftTier::Legendary) {
                    Ok(_) => {
                        let _ = post_telegram(token, chat_id, "🔨 <b>Started Auto-Craft for Legendary Fish Bait!</b>\nMake sure you are standing at Blacksmith Sen.");
                    }
                    Err(e) => {
                        let _ = post_telegram(token, chat_id, &format!("⚠️ {e}"));
                    }
                }
            } else if sub == "all" {
                let ctx_clone = bot.ctx().clone();
                match crate::bot::crafting::start_auto_craft(ctx_clone, crate::bot::crafting::CraftTier::All) {
                    Ok(_) => {
                        let _ = post_telegram(token, chat_id, "🔨 <b>Started Auto-Craft for ALL Baits (Legendary & Rare)!</b>\nMake sure you are standing at Blacksmith Sen.");
                    }
                    Err(e) => {
                        let _ = post_telegram(token, chat_id, &format!("⚠️ {e}"));
                    }
                }
            } else {
                let st = crate::bot::crafting::get_craft_status();
                let run_status = if st.is_crafting {
                    format!("🟢 <b>Running</b> ({})\nBatches crafted: {}\nStatus: {}", st.tier, st.crafted_count, st.message)
                } else {
                    format!("⏹️ <b>Idle</b>\nLast status: {}", if st.message.is_empty() { "Ready" } else { &st.message })
                };
                let _ = post_telegram(
                    token,
                    chat_id,
                    &format!(
                        "🔨 <b>Auto-Craft Bait Status</b>\n\n\
                         {run_status}\n\n\
                         <b>Commands:</b>\n\
                         • <code>/craft rare</code> - Craft Rare Fish Bait\n\
                         • <code>/craft legendary</code> - Craft Legendary Fish Bait\n\
                         • <code>/craft all</code> - Craft Legendary then Rare Bait\n\
                         • <code>/craft stop</code> - Stop Auto-Craft"
                    ),
                );
            }
        }
        "/web" | "/dashboard" | "web" | "dashboard" => {
            let local_ip = crate::bot::web_server::get_local_ip().unwrap_or_else(|| "127.0.0.1".into());
            let lan_url = format!("http://{local_ip}:3888");
            let local_url = "http://localhost:3888";
            let reply = format!(
                "🌐 <b>GPO Autofish Web Dashboard</b>\n\n\
                 • <b>Local PC</b>: <code>{local_url}</code>\n\
                 • <b>Mobile (Same Wi-Fi)</b>: <code>{lan_url}</code>\n\n\
                 <i>Access real-time stats, Roblox live screen, sound & brightness sliders, and 1-tap macro controls from any phone or browser!</i>"
            );
            let _ = crate::webhook::post_telegram_with_button(
                token,
                chat_id,
                &reply,
                "🌐 Open Web Dashboard",
                &lan_url,
            );
        }
        "/help" | "help" => {
            let help_text = "🎮 <b>GPO Autofish Remote Controls</b>\n\n\
                👑 /bosses - Live Boss & Merchant countdowns\n\
                🔨 /craft &lt;rare|legendary|all|stop&gt; - Auto craft fish bait at Blacksmith Sen\n\
                🔔 /toggle &lt;boss|spawn&gt; - Mute/unmute alerts (e.g. /toggle roger, /toggle spawn)\n\
                🔄 /sync - Calibrate timers (/sync read, /sync server, or paste Discord)\n\
                🌐 /web - Open live Web Dashboard (Mobile & PC)\n\
                🔊 /volume &lt;0-100|max|zero&gt; - Set sound volume (/volume 0..100)\n\
                💡 /brightness &lt;0-100|max|min&gt; - Set screen brightness\n\
                🔇 /mute / /unmute - Mute or unmute Windows PC audio\n\
                📊 /status - View live stats & screenshot\n\
                📸 /screenshot - Instant Roblox screenshot on demand\n\
                ⚡ /pity - Quick Devil Fruit pity counter\n\
                🔄 /recast - Reset rod & recast immediately\n\
                🛒 /buybait - Force merchant bait purchase now\n\
                ⚙️ /setbuy &lt;N&gt; - Set catches between bait buys (e.g. /setbuy 50)\n\
                📱 /setprogress &lt;N&gt; - Set catches between progress pings (e.g. /setprogress 100)\n\
                ▶️ /start - Start or resume macro\n\
                🛑 /stop - Pause macro\n\
                🚀 /update - Check & apply new app update\n\
                ❓ /help - Show this commands list\n\n\
                <i>💡 Tip: Tap the <b>[/] Menu</b> button next to the input box for one-tap commands! You can also talk to the bot naturally.</i>";
            let _ = post_telegram(token, chat_id, help_text);
        }
        _ => {
            handle_smart_command(bot, settings, boss_tracker, token, chat_id, text);
        }
    }
}

fn handle_smart_command(
    bot: &Arc<Bot>,
    settings: &Arc<RwLock<Settings>>,
    boss_tracker: &mut crate::core::boss_tracker::BossTracker,
    token: &str,
    chat_id: &str,
    text: &str,
) {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();

    // 1. Brightness / Light intent
    if lower.contains("brightness") || lower.contains("screen light") || lower.contains("light level") || lower.contains("dim screen") || (lower.contains("light") && (lower.contains("screen") || lower.contains("monitor") || lower.contains("display") || words.iter().any(|w| w.parse::<u32>().is_ok()))) {
        if lower.contains("max") || lower.contains("100") {
            let _ = crate::core::brightness::set_brightness(100);
            let _ = post_telegram(token, chat_id, "💡 <b>Screen brightness set to MAX (100%)</b>");
            return;
        } else if lower.contains("min") || lower.contains("zero") || words.contains(&"0") {
            let _ = crate::core::brightness::set_brightness(0);
            let _ = post_telegram(token, chat_id, "💡 <b>Screen brightness set to MIN (0%)</b>");
            return;
        } else if let Some(num) = extract_number(&lower) {
            let clamped = num.clamp(0, 100);
            let _ = crate::core::brightness::set_brightness(clamped);
            let emoji = if clamped < 30 { "🌑" } else if clamped < 70 { "🌓" } else { "🌕" };
            let _ = post_telegram(token, chat_id, &format!("{emoji} <b>Screen brightness set to {clamped}%</b>"));
            return;
        } else {
            let b = crate::core::brightness::get_brightness().unwrap_or(80);
            let _ = post_telegram(token, chat_id, &format!("💡 <b>Current Screen Brightness:</b> <b>{b}%</b>\n\n<i>To change:</i> <code>brightness 50</code>, <code>brightness max</code>, <code>dim screen</code>"));
            return;
        }
    }

    // 2. Volume / Sound intent
    if lower.contains("volume") || lower.contains("sound") || lower.contains("audio") || lower.contains("mute") || lower.contains("unmute") || lower.contains("louder") || lower.contains("quieter") {
        if lower.contains("unmute") {
            let _ = crate::core::audio::set_mute(false);
            let vol = (crate::core::audio::get_volume().unwrap_or(0.5) * 100.0).round() as u32;
            let _ = post_telegram(token, chat_id, &format!("🔊 <b>Windows Audio UNMUTED!</b> (Volume: <b>{vol}%</b>)"));
            return;
        } else if lower.contains("mute") || lower.contains("zero") || words.contains(&"0") {
            let _ = crate::core::audio::set_mute(true);
            let _ = post_telegram(token, chat_id, "🔇 <b>Windows Audio MUTED!</b>\nSend <code>unmute</code> or <code>volume max</code> to restore.");
            return;
        } else if lower.contains("max") || lower.contains("100") {
            let _ = crate::core::audio::set_volume(1.0);
            let _ = post_telegram(token, chat_id, "🔊 <b>Windows Volume set to MAX (100%)</b>");
            return;
        } else if let Some(num) = extract_number(&lower) {
            let clamped = num.clamp(0, 100);
            let _ = crate::core::audio::set_volume(clamped as f32 / 100.0);
            let emoji = if clamped == 0 { "🔇" } else if clamped < 50 { "🔉" } else { "🔊" };
            let _ = post_telegram(token, chat_id, &format!("{emoji} <b>Windows Volume set to {clamped}%</b>"));
            return;
        } else {
            let vol = (crate::core::audio::get_volume().unwrap_or(0.5) * 100.0).round() as u32;
            let muted = crate::core::audio::is_muted().unwrap_or(false);
            let state_str = if muted || vol == 0 { "🔇 Muted" } else { "🔊 Unmuted" };
            let _ = post_telegram(token, chat_id, &format!("🔊 <b>Windows Master Audio:</b> <b>{vol}%</b> ({state_str})\n\n<i>To change:</i> <code>volume 60</code>, <code>volume max</code>, <code>mute</code>"));
            return;
        }
    }

    // 3. Screenshot intent
    if lower.contains("screenshot") || lower.contains("photo") || lower.contains("pic") || lower.contains("picture") || lower.contains("screen") || lower.contains("show me") || lower.contains("view game") {
        handle_command(bot, settings, boss_tracker, token, chat_id, "/screenshot");
        return;
    }

    // 4. Status / Progress intent
    if lower.contains("status") || lower.contains("how many fish") || lower.contains("progress") || lower.contains("stats") || lower.contains("how is it going") || lower.contains("what is happening") || lower.contains("runtime") {
        handle_command(bot, settings, boss_tracker, token, chat_id, "/status");
        return;
    }

    // 5. Pity intent
    if lower.contains("pity") {
        handle_command(bot, settings, boss_tracker, token, chat_id, "/pity");
        return;
    }

    // 6. Bosses / Timers intent
    if lower.contains("boss") || lower.contains("timer") || lower.contains("roger") || lower.contains("mihawk") || lower.contains("kizaru") || lower.contains("merchant") || lower.contains("brook") || lower.contains("when does") {
        handle_command(bot, settings, boss_tracker, token, chat_id, "/bosses");
        return;
    }

    // 7. Recast intent
    if lower.contains("recast") || lower.contains("cast again") || lower.contains("reset rod") || lower.contains("throw rod") {
        handle_command(bot, settings, boss_tracker, token, chat_id, "/recast");
        return;
    }

    // 8. Start / Resume intent
    if lower.contains("start") || lower.contains("resume") || lower.contains("continue") || lower.contains("unpause") {
        handle_command(bot, settings, boss_tracker, token, chat_id, "/start");
        return;
    }

    // 9. Pause / Stop intent
    if lower.contains("pause") || lower.contains("stop") || lower.contains("halt") || lower.contains("hold on") {
        handle_command(bot, settings, boss_tracker, token, chat_id, "/stop");
        return;
    }

    // 10. Buy bait intent
    if lower.contains("buy bait") || lower.contains("purchase bait") || lower.contains("get bait") || lower.contains("buy some bait") {
        handle_command(bot, settings, boss_tracker, token, chat_id, "/buybait");
        return;
    }

    // 11. Web App / Dashboard intent
    if lower.contains("web") || lower.contains("dashboard") || lower.contains("panel") || lower.contains("site") || lower.contains("link") {
        handle_command(bot, settings, boss_tracker, token, chat_id, "/web");
        return;
    }

    // 12. Help intent
    if lower.contains("help") || lower.contains("commands") || lower.contains("what can you do") {
        handle_command(bot, settings, boss_tracker, token, chat_id, "/help");
        return;
    }

    // 13. Conversational Gemini AI fallback (if enabled)
    let (gemini_enabled, api_key, model) = {
        let s = settings.read();
        (s.gemini.enabled && !s.gemini.api_key.trim().is_empty(), s.gemini.api_key.clone(), s.gemini.model.clone())
    };

    if gemini_enabled {
        let stats = bot.ctx().session.lock().stats();
        let vol = (crate::core::audio::get_volume().unwrap_or(0.5) * 100.0).round() as u32;
        let brightness = crate::core::brightness::get_brightness().unwrap_or(80);
        let state = if bot.is_paused() { "Paused" } else if bot.is_running() { "Fishing" } else { "Stopped" };
        let context_summary = format!(
            "Macro State: {state}, Runtime: {}s, Fish Caught: {}, Fruits: {}, Fruit Pity: {}, Legendary Pity: {}/100, Master Volume: {}%, Screen Brightness: {}%",
            stats.runtime_s, stats.fish, stats.fruits, stats.pity_fruit, stats.pity_legendary, vol, brightness
        );

        if let Ok(res) = crate::core::gemini::interpret_smart_bot_command(text, &context_summary, &api_key, &model) {
            if let Some(ref act) = res.action {
                match act.as_str() {
                    "start" => bot.start(),
                    "pause" => bot.pause(),
                    "recast" => bot.recast(),
                    "buy_bait" => { let _ = crate::bot::actions::purchase(&bot.ctx()); }
                    "set_volume" => {
                        if let Some(v) = res.param {
                            let _ = crate::core::audio::set_volume(v.clamp(0, 100) as f32 / 100.0);
                        }
                    }
                    "set_brightness" => {
                        if let Some(b) = res.param {
                            let _ = crate::core::brightness::set_brightness(b.clamp(0, 100));
                        }
                    }
                    "screenshot" => {
                        handle_command(bot, settings, boss_tracker, token, chat_id, "/screenshot");
                        return;
                    }
                    "status" => {
                        handle_command(bot, settings, boss_tracker, token, chat_id, "/status");
                        return;
                    }
                    "pity" => {
                        handle_command(bot, settings, boss_tracker, token, chat_id, "/pity");
                        return;
                    }
                    "bosses" => {
                        handle_command(bot, settings, boss_tracker, token, chat_id, "/bosses");
                        return;
                    }
                    "web" => {
                        handle_command(bot, settings, boss_tracker, token, chat_id, "/web");
                        return;
                    }
                    _ => {}
                }
            }
            let _ = post_telegram(token, chat_id, &res.reply);
            return;
        }
    }

    // Final friendly fallback
    let _ = post_telegram(
        token,
        chat_id,
        "🤖 <b>I didn't quite catch that.</b>\n\n\
         <i>Try commands like:</i>\n\
         • <code>status</code> or <code>how many fish</code>\n\
         • <code>screenshot</code> or <code>show screen</code>\n\
         • <code>volume 50</code> or <code>mute</code>\n\
         • <code>brightness 70</code> or <code>dim screen</code>\n\
         • <code>bosses</code> or <code>when roger</code>\n\
         • <code>web</code> (open live web dashboard)\n\
         • <code>help</code> (show all commands)",
    );
}

fn extract_number(text: &str) -> Option<u32> {
    for token in text.split_whitespace() {
        let clean: String = token.chars().filter(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = clean.parse::<u32>() {
            return Some(n);
        }
    }
    None
}

fn register_bot_commands(client: &reqwest::blocking::Client, token: &str) {
    let url = format!("https://api.telegram.org/bot{token}/setMyCommands");
    let payload = serde_json::json!({
        "commands": [
            { "command": "bosses", "description": "👑 Live Boss & Merchant timers" },
            { "command": "toggle", "description": "🔔 Mute/unmute specific boss alerts" },
            { "command": "sync", "description": "🔄 Calibrate boss timers (/sync)" },
            { "command": "web", "description": "🌐 Open live Web Dashboard (Mobile & PC)" },
            { "command": "volume", "description": "🔊 Set sound volume (/volume 0..100, max, zero)" },
            { "command": "brightness", "description": "💡 Set screen brightness (/brightness 0..100, min, max)" },
            { "command": "status", "description": "📊 Live stats & Roblox screenshot" },
            { "command": "screenshot", "description": "📸 Instant Roblox screen capture" },
            { "command": "pity", "description": "⚡ Devil fruit pity status" },
            { "command": "recast", "description": "🔄 Reset rod & recast immediately" },
            { "command": "buybait", "description": "🛒 Buy bait at merchant right now" },
            { "command": "setbuy", "description": "⚙️ Set catches between bait buys (/setbuy N)" },
            { "command": "setprogress", "description": "📱 Set catches between progress pings (/setprogress N)" },
            { "command": "start", "description": "▶️ Start or resume fishing macro" },
            { "command": "stop", "description": "🛑 Pause fishing macro" },
            { "command": "update", "description": "🚀 Check and apply latest app update" },
            { "command": "help", "description": "❓ Show all commands and controls" }
        ]
    });
    if let Err(e) = client.post(&url).json(&payload).send() {
        tracing::warn!("Failed to register Telegram bot commands: {e}");
    } else {
        tracing::info!("Telegram bot commands menu registered successfully");
    }
}

pub fn check_and_apply_update(
    bot: Option<&Arc<Bot>>,
    token: Option<&str>,
    chat_id: Option<&str>,
    cur_ver: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let url = "https://github.com/Maxofmax20/gpo-fishing/releases/latest/download/latest.json";
    let resp: Value = client
        .get(url)
        .header("User-Agent", "gpo-autofish")
        .send()?
        .json()?;

    let remote_ver = resp
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or("Invalid latest.json manifest")?
        .trim_start_matches('v');

    if remote_ver == cur_ver {
        return Ok(format!(
            "✅ <b>You're already running the latest version!</b> (v{cur_ver})"
        ));
    }

    let download_url = resp
        .get("platforms")
        .and_then(|p| p.get("windows-x86_64"))
        .and_then(|w| w.get("url"))
        .and_then(|u| u.as_str())
        .ok_or("No download URL for windows-x86_64")?;

    let notes = resp.get("notes").and_then(|n| n.as_str()).unwrap_or("");
    if let (Some(tok), Some(cid)) = (token, chat_id) {
        let _ = post_telegram(
            tok,
            cid,
            &format!(
                "🚀 <b>New Version Found: v{remote_ver}!</b>\n\n<i>{notes}</i>\n\n⬇️ Downloading installer in the background..."
            ),
        );
    }

    let temp_dir = std::env::temp_dir();
    let installer_path = temp_dir.join(format!("GPO.Autofish_{remote_ver}_setup.exe"));

    let mut exe_resp = client
        .get(download_url)
        .header("User-Agent", "gpo-autofish")
        .send()?;
    let mut file = std::fs::File::create(&installer_path)?;
    std::io::copy(&mut exe_resp, &mut file)?;
    drop(file);

    if let (Some(tok), Some(cid)) = (token, chat_id) {
        let _ = post_telegram(
            tok,
            cid,
            "📦 <b>Update downloaded successfully!</b>\nInstalling update and restarting GPO Autofish...",
        );
    }

    // If bot was running, persist state so it auto-resumes after update
    if let Some(b) = bot {
        let was_running = b.is_running();
        if was_running {
            let _ = b.ctx().store.save_resume_state(true);
        }
    }

    let current_exe = std::env::current_exe()?;
    let current_exe_str = current_exe.to_string_lossy();
    let installer_str = installer_path.to_string_lossy();

    // Hidden PowerShell supervisor: waits for silent installer to finish, then restarts the app!
    let ps_cmd = format!(
        "Start-Sleep -Milliseconds 800; Start-Process -FilePath '{}' -ArgumentList '/S' -Wait; Start-Process -FilePath '{}'",
        installer_str, current_exe_str
    );

    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    std::process::Command::new("powershell")
        .args(["-WindowStyle", "Hidden", "-Command", &ps_cmd])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;

    std::thread::sleep(Duration::from_millis(600));
    std::process::exit(0);
}
