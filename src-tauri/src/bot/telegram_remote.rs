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

                handle_command(&bot, &token, &chat_id_clean, text);
            }
        }

        std::thread::sleep(Duration::from_millis(500));
    }
}

fn handle_command(bot: &Arc<Bot>, token: &str, chat_id: &str, text: &str) {
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
        "/help" | "help" => {
            let help_text = "🎮 <b>GPO Autofish Remote Controls</b>\n\n\
                /status - View live stats & Roblox screenshot\n\
                /stop - Pause the macro\n\
                /start - Start or resume the macro\n\
                /help - Show this commands list";
            let _ = post_telegram(token, chat_id, help_text);
        }
        _ => {}
    }
}
