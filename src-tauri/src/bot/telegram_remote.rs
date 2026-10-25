//! Rebuilt owner-only bot commands. Notifications have their own queue/client.
use crate::{
    bot::Bot,
    config::Settings,
    webhook::{post_telegram, post_telegram_with_button},
};
use parking_lot::RwLock;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

pub fn dashboard_authed_url(base: &str, token: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(base) else {
        return base.into();
    };
    if !token.trim().is_empty() {
        url.query_pairs_mut().append_pair("token", token);
    }
    url.to_string()
}
fn call(
    client: &reqwest::blocking::Client,
    token: &str,
    method: &str,
    body: Value,
) -> Result<Value, String> {
    let response = client
        .post(format!(
            "https://api.telegram.org/bot{}/{method}",
            token.trim()
        ))
        .json(&body)
        .send()
        .map_err(|_| "Telegram connection failed")?;
    let status = response.status();
    let value = response
        .json::<Value>()
        .map_err(|_| "Telegram returned invalid data")?;
    if !status.is_success() || value["ok"].as_bool() != Some(true) {
        return Err(format!(
            "Telegram request rejected (HTTP {})",
            status.as_u16()
        ));
    }
    Ok(value["result"].clone())
}
pub fn spawn(bot: Arc<Bot>, settings: Arc<RwLock<Settings>>) {
    std::thread::spawn(move || {
        let Ok(client) = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(25))
            .build()
        else {
            return;
        };
        let mut previous = String::new();
        let mut offset = 0i64;
        let mut owner = None;
        let mut failures = 0u32;
        loop {
            let s = settings.read().clone();
            let token = s.webhook.telegram_bot_token.trim();
            let configured = s.webhook.telegram_chat_id.trim();
            if token.is_empty() || configured.is_empty() {
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
            let identity = format!("{token}:{configured}");
            if identity != previous {
                previous = identity;
                offset = call(
                    &client,
                    token,
                    "getUpdates",
                    json!({"offset":-1,"timeout":0,"allowed_updates":["message","callback_query"]}),
                )
                .ok()
                .and_then(|r| {
                    r.as_array()
                        .and_then(|a| a.last())
                        .and_then(|u| u["update_id"].as_i64())
                })
                .map(|n| n.saturating_add(1))
                .unwrap_or(0);
                owner = configured.parse::<i64>().ok().or_else(|| {
                    call(&client, token, "getChat", json!({"chat_id":configured}))
                        .ok()
                        .and_then(|r| r["id"].as_i64())
                });
                let _ = call(
                    &client,
                    token,
                    "setMyCommands",
                    json!({"commands":[
                        {"command":"status","description":"Fishing and recorder status"},
                        {"command":"fish","description":"Start fishing"},
                        {"command":"pause","description":"Pause fishing"},
                        {"command":"stop","description":"Stop fishing and playback"},
                        {"command":"jump","description":"Jump in Roblox"},
                        {"command":"web","description":"Open the private remote"},
                        {"command":"record","description":"Record PC game inputs"},
                        {"command":"save","description":"Save the active recording"}
                    ]}),
                );
            }
            let updates = match call(
                &client,
                token,
                "getUpdates",
                json!({"offset":offset,"timeout":20,"allowed_updates":["message","callback_query"]}),
            ) {
                Ok(Value::Array(updates)) => {
                    failures = 0;
                    updates
                }
                _ => {
                    failures = failures.saturating_add(1);
                    std::thread::sleep(Duration::from_secs((1u64 << failures.min(4)).min(16)));
                    continue;
                }
            };
            for update in updates {
                if let Some(id) = update["update_id"].as_i64() {
                    offset = offset.max(id.saturating_add(1));
                }
                let callback = update.get("callback_query");
                let message = callback
                    .and_then(|q| q.get("message"))
                    .or_else(|| update.get("message"));
                let Some(message) = message else {
                    continue;
                };
                if message["chat"]["id"].as_i64() != owner {
                    continue;
                }
                let text = callback
                    .and_then(|q| q["data"].as_str())
                    .or_else(|| message["text"].as_str())
                    .unwrap_or("");
                let mut parts = text.splitn(2, ' ');
                let command = parts.next().unwrap_or("").split('@').next().unwrap_or("");
                let argument = parts.next().unwrap_or("").trim();
                let result = (|| -> Result<String, String> {
                    match command{
                        "/start"=>Ok("GPO Remote is connected. Use /status, /fish, /pause, /stop, /jump or /web.".into()),
                        "/status"=>Ok(format!("Fishing: {:?}\nRecorder: {}",bot.state(),super::recorder::get_status().message)),
                        "/fish"=>{bot.start_checked()?;Ok("Fishing start requested.".into())},
                        "/pause"=>{bot.pause();super::recorder::stop_playback();Ok("Paused.".into())},
                        "/stop"=>{bot.stop();super::recorder::stop_playback();Ok("Stopped; held inputs released.".into())},
                        "/jump"=>{let control=super::control::service()?;control.focus()?;control.tap("space")?;Ok("Jump input sent.".into())},
                        "/record"=>{if bot.is_running(){return Err("Pause fishing before recording.".into());}super::recorder::start_recording(super::recorder::RecordMode::PcWindow)?;Ok("Recording game inputs. Use /save when finished.".into())},
                        "/save"=>{let saved=super::recorder::stop_recording(argument,&bot.ctx().store)?;Ok(format!("Saved recording: {}",saved.name))},
                        "/play"=>{super::recorder::play_macro(bot.ctx().clone(),bot.ctx().store.clone(),argument,false,Some(1.0),Some(1))?;Ok("Playback requested.".into())},
                        "/web"=>{
                            let ip=super::web_server::get_local_ip().unwrap_or_else(||"127.0.0.1".into());
                            let url=dashboard_authed_url(&format!("http://{ip}:3888"),&s.web.token);
                            post_telegram_with_button(token,configured,"Open your authenticated GPO Remote.","Open remote",&url)?;
                            Ok(String::new())
                        },
                        "/update"=>Ok("Use Settings → Check for updates in the app. Only the signed in-app updater installs releases.".into()),
                        _=>Ok("Unknown command. Use /status, /fish, /pause, /stop, /jump or /web.".into()),
                    }
                })();
                if let Some(query) = callback {
                    if let Some(id) = query["id"].as_str() {
                        let _ = call(
                            &client,
                            token,
                            "answerCallbackQuery",
                            json!({"callback_query_id":id}),
                        );
                    }
                }
                let reply =
                    result.unwrap_or_else(|error| format!("Could not complete request: {error}"));
                if !reply.is_empty() {
                    let _ = post_telegram(token, configured, &reply);
                }
            }
        }
    });
}
pub fn check_and_apply_update(
    _bot: Option<&Arc<Bot>>,
    token: Option<&str>,
    chat: Option<&str>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let message =
        "Use Settings → Check for updates. Installation verifies the existing updater signature.";
    if let (Some(token), Some(chat)) = (token, chat) {
        post_telegram(token, chat, message)?;
    }
    Ok(message.into())
}
