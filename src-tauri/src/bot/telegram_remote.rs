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
            match check_and_apply_update(token, chat_id, cur_ver) {
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
        cmd if cmd.starts_with("/toggle") || cmd.starts_with("toggle") || cmd.starts_with("/mute") || cmd.starts_with("mute") => {
            let parts: Vec<&str> = text.split_whitespace().collect();
            if let Some(target) = parts.get(1) {
                let target_low = target.to_lowercase();
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
                        format!("⚠️ Unknown boss <b>{target}</b>.\nValid options: <code>hawkeye</code>, <code>roger</code>, <code>soulking</code>, <code>kizaru</code>, <code>merchant</code>, <code>all</code>")
                    };
                    let _ = bot.ctx().store.save(&s);
                    res
                };
                let _ = post_telegram(token, chat_id, &reply);
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
        "/help" | "help" => {
            let help_text = "🎮 <b>GPO Autofish Remote Controls</b>\n\n\
                👑 /bosses - Live Boss & Merchant countdowns\n\
                🔔 /toggle &lt;boss&gt; - Mute/unmute alerts (e.g. /toggle roger)\n\
                🔄 /sync - Calibrate timers (or paste Discord bot text)\n\
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
                <i>💡 Tip: Tap the <b>[/] Menu</b> button next to the input box for one-tap commands!</i>";
            let _ = post_telegram(token, chat_id, help_text);
        }
        _ => {}
    }
}

fn register_bot_commands(client: &reqwest::blocking::Client, token: &str) {
    let url = format!("https://api.telegram.org/bot{token}/setMyCommands");
    let payload = serde_json::json!({
        "commands": [
            { "command": "bosses", "description": "👑 Live Boss & Merchant timers" },
            { "command": "toggle", "description": "🔔 Mute/unmute specific boss alerts" },
            { "command": "sync", "description": "🔄 Calibrate boss timers (/sync)" },
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

fn check_and_apply_update(
    token: &str,
    chat_id: &str,
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
    let _ = post_telegram(
        token,
        chat_id,
        &format!(
            "🚀 <b>New Version Found: v{remote_ver}!</b>\n\n<i>{notes}</i>\n\n⬇️ Downloading installer in the background..."
        ),
    );

    let temp_dir = std::env::temp_dir();
    let installer_path = temp_dir.join(format!("GPO.Autofish_{remote_ver}_setup.exe"));

    let mut exe_resp = client
        .get(download_url)
        .header("User-Agent", "gpo-autofish")
        .send()?;
    let mut file = std::fs::File::create(&installer_path)?;
    std::io::copy(&mut exe_resp, &mut file)?;
    drop(file);

    let _ = post_telegram(
        token,
        chat_id,
        "📦 <b>Update downloaded successfully!</b>\nLaunching installer and restarting GPO Autofish...",
    );

    // Launch installer and exit current process so file is replaced cleanly
    std::process::Command::new(&installer_path)
        .args(["/S"])
        .spawn()?;

    std::thread::sleep(Duration::from_millis(600));
    std::process::exit(0);
}
