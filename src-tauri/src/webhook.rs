use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{bounded, Receiver, Sender};
#[cfg(test)]
use crossbeam_channel::unbounded;
use std::sync::OnceLock;
use parking_lot::RwLock;
use serde_json::{json, Value};

use crate::config::Settings;
use crate::core::fruit::{DropInfo, SpawnInfo};
use crate::events::{Stats, BotEvent, LogLine};
use crate::core::fruit_storage::{FruitCatchContext, StorageOutcome};

const COLOR_BLUE: u32 = 0x3B82F6;
const COLOR_PURPLE: u32 = 0x8B5CF6;
const COLOR_GOLD: u32 = 0xF59E0B;
const COLOR_GREEN: u32 = 0x22C55E;
const COLOR_RED: u32 = 0xEF4444;

#[derive(Debug, Clone)]
pub struct Notification {
    pub title: String,
    pub desc: String,
    pub color: u32,
    pub fields: Vec<(String, String)>,
    pub photo: Option<Vec<u8>>,
    pub custom_telegram_html: Option<String>,
}

enum Delivery {
    Message(Notification),
    Spawn(Notification, SpawnInfo),
    FruitOutcome(Notification, bool),
}

pub struct WebhookQueue {
    tx: Option<Sender<Delivery>>,
    priority_tx: Option<Sender<Delivery>>,
    settings: Option<Arc<RwLock<Settings>>>,
}

impl WebhookQueue {
    pub fn disabled() -> Self {
        Self { tx: None, priority_tx: None, settings: None }
    }

    pub fn start(settings: Arc<RwLock<Settings>>, events: Sender<BotEvent>) -> Arc<Self> {
        let (tx, rx) = bounded::<Delivery>(8);
        let (priority_tx, priority_rx) = bounded::<Delivery>(8);
        let s2 = Arc::clone(&settings);
        std::thread::Builder::new()
            .name("webhook".into())
            .spawn(move || worker(rx, priority_rx, s2, events))
            .expect("spawn webhook worker");
        Arc::new(Self { tx: Some(tx), priority_tx: Some(priority_tx), settings: Some(settings) })
    }

    fn enqueue(&self, mut delivery: Delivery, priority: bool) {
        // A slow provider must never grow RAM or block the controller.
        // At most 16 messages * 1 MiB photo bytes are queued.
        let notification = match &mut delivery {
            Delivery::Message(n) | Delivery::Spawn(n, _) | Delivery::FruitOutcome(n, _) => n,
        };
        if notification.photo.as_ref().is_some_and(|p| p.len() > 1024 * 1024) {
            notification.photo = None;
            tracing::warn!("Notification screenshot exceeded 1 MiB budget; keeping text details");
        }
        let sender = if priority { self.priority_tx.as_ref().or(self.tx.as_ref()) } else { self.tx.as_ref() };
        if sender.is_some_and(|tx| tx.try_send(delivery).is_err()) {
            tracing::warn!("Notification queue full; delivery not accepted (fishing continues)");
        }
    }

    fn enabled(&self) -> bool {
        self.settings
            .as_ref()
            .map(|s| {
                let s = s.read();
                if !s.webhook.enabled {
                    return false;
                }
                let provider = s.webhook.provider.as_str();
                let has_discord = !s.webhook.url.trim().is_empty();
                let has_telegram = !s.webhook.telegram_bot_token.trim().is_empty()
                    && !s.webhook.telegram_chat_id.trim().is_empty();
                match provider {
                    "discord" => has_discord,
                    "telegram" => has_telegram,
                    _ => has_discord || has_telegram,
                }
            })
            .unwrap_or(false)
    }

    fn send(&self, notif: Notification) {
        if self.settings.as_ref().is_some_and(|s| s.read().webhook.pity_zero_only) {
            return;
        }
        if !self.enabled() {
            return;
        }
        self.enqueue(Delivery::Message(notif), false);
    }

    pub fn test(&self) -> Result<(), String> {
        let (provider, url, tg_token, tg_chat) = self
            .settings
            .as_ref()
            .map(|s| {
                let s = s.read();
                (
                    s.webhook.provider.clone(),
                    s.webhook.url.clone(),
                    s.webhook.telegram_bot_token.clone(),
                    s.webhook.telegram_chat_id.clone(),
                )
            })
            .unwrap_or_default();

        let mut sent_any = false;
        let mut errors = Vec::new();

        if (provider == "discord" || provider == "both") && !url.trim().is_empty() {
            let body = json!({
                "embeds": [embed("Webhook connected", "GPO Autofish can reach this Discord channel.", COLOR_GREEN, vec![])]
            });
            match post_discord(&url, &body) {
                Ok(_) => sent_any = true,
                Err(e) => errors.push(format!("Discord: {e}")),
            }
        }

        if (provider == "telegram" || provider == "both") && !tg_token.trim().is_empty() && !tg_chat.trim().is_empty() {
            let text = "✅ <b>GPO Autofish</b>\nTelegram notifications connected successfully! You will receive alerts here.";
            match post_telegram(&tg_token, &tg_chat, text) {
                Ok(_) => sent_any = true,
                Err(e) => errors.push(format!("Telegram: {e}")),
            }
        }

        if !sent_any && errors.is_empty() {
            return Err("Please configure your Telegram Bot Token & Chat ID (or Discord Webhook URL)".into());
        }

        if !errors.is_empty() {
            return Err(errors.join("; "));
        }

        Ok(())
    }

    pub fn progress(&self, st: Stats) {
        self.send(Notification {
            title: "Fishing Progress".into(),
            desc: String::new(),
            color: COLOR_BLUE,
            fields: vec![
                ("Fish".into(), st.fish.to_string()),
                ("Fruits".into(), st.fruits.to_string()),
                ("Runtime".into(), fmt_runtime(st.runtime_s)),
                ("Success".into(), format!("{:.0}%", st.success_rate * 100.0)),
            ],
            photo: None,
            custom_telegram_html: None,
        });
    }

    pub fn fruit_drop(&self, d: &DropInfo, photo: Option<Vec<u8>>) {
        if self.settings.as_ref().is_some_and(|s| s.read().webhook.pity_zero_only) {
            return; // strict mode has one post-workflow notification, no early duplicate
        }
        let fruit_name = d.name.as_deref().unwrap_or("Devil Fruit");
        let rarity = crate::core::fruit::fruit_rarity(fruit_name);
        let is_ase = self.settings.as_ref().map(|s| s.read().fruit_storage.all_seeing_eye).unwrap_or(true);
        let cap_str = if is_ase { "40" } else { "100" };
        let is_pity_zero = crate::core::fruit::is_pity_zero(d.pity.as_deref(), &d.text);
        let pity_info = if let Some(p) = &d.pity {
            if is_pity_zero {
                format!("🌟 Legendary Pity: {p} (HIT! Guaranteed Legendary/Mythical!)")
            } else {
                format!("⚡ Legendary Pity: {p}")
            }
        } else if is_pity_zero {
            format!("🌟 Legendary Pity: 0/{cap_str} (HIT! Guaranteed Legendary/Mythical!)")
        } else {
            "⚡ Check backpack".into()
        };

        let (title, desc, color) = if rarity == crate::core::fruit::FruitRarity::Mythical {
            ("🔥 MYTHICAL DEVIL FRUIT CAUGHT!", format!("🎉 Extraordinary luck! You got a Mythical Devil Fruit: {fruit_name}!\n\n{pity_info}"), COLOR_GOLD)
        } else if rarity == crate::core::fruit::FruitRarity::Legendary {
            ("🌟 Legendary Devil Fruit Caught!", format!("You got a legendary devil fruit: {fruit_name}!\n\n{pity_info}"), COLOR_GOLD)
        } else if is_pity_zero {
            ("🌟 Pity 0 Devil Fruit Caught!", format!("Pity reset to 0! Pity-zero catch: {fruit_name}!\n\n{pity_info}"), COLOR_GOLD)
        } else if rarity != crate::core::fruit::FruitRarity::Unknown {
            ("🍇 Devil Fruit Caught!", format!("You got a {} devil fruit: {fruit_name}.\n\n{pity_info}", rarity.as_str()), COLOR_PURPLE)
        } else {
            ("🍇 Devil Fruit Caught!", format!("You caught a devil fruit!\n\n{pity_info}"), COLOR_PURPLE)
        };
        self.send(Notification {
            title: title.into(),
            desc,
            color,
            fields: vec![
                ("Fruit Name".into(), fruit_name.to_string()),
                ("Pity Status".into(), pity_info),
                ("Raw OCR".into(), d.text.clone()),
            ],
            photo,
            custom_telegram_html: d.custom_telegram_message.clone(),
        });
    }

    pub fn fruit_storage_outcome(&self, catch: &FruitCatchContext, outcome: &StorageOutcome, photo: Option<Vec<u8>>) {
        let Some(settings) = &self.settings else { return; };
        let settings = settings.read();
        if let Some(notification) = storage_notification(&settings, catch, outcome, photo) {
            drop(settings);
            if self.enabled() {
                self.enqueue(Delivery::FruitOutcome(notification, catch.pity_zero), true);
            }
        }
    }

    pub fn disconnect(&self, reason: &str, photo: Option<Vec<u8>>) {
        let flag = self.settings.as_ref().map(|s| s.read().webhook.disconnect_alert).unwrap_or(true);
        if !flag {
            return;
        }
        self.send(Notification {
            title: "⚠️ Roblox Disconnected".into(),
            desc: format!("{reason}. Macro is safely paused."),
            color: COLOR_RED,
            fields: vec![("Status".into(), "Paused".into())],
            photo,
            custom_telegram_html: None,
        });
    }

    pub fn reconnected(&self, photo: Option<Vec<u8>>) {
        self.send(Notification {
            title: "✅ Roblox Reconnected".into(),
            desc: "Auto-reconnect engine successfully recovered the session. Autofishing resumed!".into(),
            color: COLOR_GREEN,
            fields: vec![("Status".into(), "Running".into())],
            photo,
            custom_telegram_html: Some("✅ <b>Roblox Reconnected</b>\nAuto-reconnect engine successfully recovered the session! Autofishing resumed.".into()),
        });
    }

    pub fn bait_depleted(&self) {
        let flag = self.settings.as_ref().map(|s| s.read().webhook.bait_alert).unwrap_or(true);
        if !flag {
            return;
        }
        self.send(Notification {
            title: "🎣 Bait Depleted".into(),
            desc: "Your fishing bait has run out. Macro has stopped safely to avoid wasting casts.".into(),
            color: COLOR_GOLD,
            fields: vec![("Action".into(), "Restock bait to continue".into())],
            photo: None,
            custom_telegram_html: None,
        });
    }

    pub fn spawn(&self, info: &SpawnInfo, photo: Option<Vec<u8>>) {
        let at = info.location.as_deref().map(|l| format!(" at {l}")).unwrap_or_default();
        let rarity = info.name.as_deref().map(crate::core::fruit::fruit_rarity).unwrap_or(crate::core::fruit::FruitRarity::Unknown);

        let (title, desc, color) = match (&info.name, info.is_ase) {
            (Some(n), true) => {
                let badge = match rarity {
                    crate::core::fruit::FruitRarity::Mythical => "🔥 MYTHICAL",
                    crate::core::fruit::FruitRarity::Legendary => "🌟 LEGENDARY",
                    crate::core::fruit::FruitRarity::Epic => "💎 EPIC",
                    crate::core::fruit::FruitRarity::Rare => "🍇 RARE",
                    _ => "👁️ ALL-SEEING EYE",
                };
                (
                    format!("{badge} FRUIT SPAWNED!"),
                    format!("👁️ All-Seeing Eye detected: <b>{n}</b> ({}) has spawned{at}!", rarity.as_str()),
                    if rarity.is_high_tier() { COLOR_GOLD } else { COLOR_PURPLE },
                )
            }
            (Some(n), false) => {
                (
                    "🌀 Devil Fruit Spawned!".into(),
                    format!("<b>{n}</b> ({}) has spawned{at}!", rarity.as_str()),
                    if rarity.is_high_tier() { COLOR_GOLD } else { COLOR_PURPLE },
                )
            }
            (None, true) => {
                (
                    "👁️ All-Seeing Eye: Fruit Spawned!".into(),
                    format!("A devil fruit has spawned{at}!"),
                    COLOR_GOLD,
                )
            }
            (None, false) => {
                (
                    "🌀 Devil Fruit Spawned!".into(),
                    format!("A devil fruit has spawned{at}!"),
                    COLOR_BLUE,
                )
            }
        };

        // If custom_telegram_message is provided (e.g. from Gemini), use it; otherwise build rich HTML
        let tg_html = info.custom_telegram_message.clone().unwrap_or_else(|| {
            let mut h = String::new();
            if info.is_ase {
                h.push_str("👁️ <b>ALL-SEEING EYE SPAWN ALERT!</b>\n\n");
            } else {
                h.push_str("🌀 <b>DEVIL FRUIT SPAWN ALERT!</b>\n\n");
            }
            if let Some(ref n) = info.name {
                let icon = match rarity {
                    crate::core::fruit::FruitRarity::Mythical => "🔥",
                    crate::core::fruit::FruitRarity::Legendary => "🌟",
                    crate::core::fruit::FruitRarity::Epic => "💎",
                    crate::core::fruit::FruitRarity::Rare => "🍇",
                    _ => "🍎",
                };
                h.push_str(&format!("• <b>Fruit</b>: {icon} <b>{n}</b> (<i>{}</i>)\n", rarity.as_str()));
            } else {
                h.push_str("• <b>Fruit</b>: ❓ Unknown Fruit\n");
            }
            if let Some(ref l) = info.location {
                h.push_str(&format!("• <b>Location</b>: 📍 <b>{l}</b>\n"));
            }
            h.push_str(&format!("• <b>Banner Text</b>: <code>{}</code>", escape_html(&info.text)));
            h
        });

        let notification = Notification {
            title,
            desc,
            color,
            fields: vec![
                ("Fruit Name".into(), info.name.clone().unwrap_or_else(|| "Unknown".into())),
                ("Rarity".into(), rarity.as_str().to_string()),
                ("Location".into(), info.location.clone().unwrap_or_else(|| "Unknown".into())),
                ("All-Seeing Eye".into(), if info.is_ase { "Yes 👁️".into() } else { "No".into() }),
            ],
            photo,
            custom_telegram_html: Some(tg_html),
        };
        // Enrichment and transport belong to the notification worker. The
        // spawn observer never waits for a network request or changes control.
        if self.enabled() {
            self.enqueue(Delivery::Spawn(notification, info.clone()), false);
        }
    }

    pub fn purchase(&self, amount: u32) {
        self.send(Notification {
            title: "🛒 Bait purchased".into(),
            desc: format!("Bought {amount} bait."),
            color: COLOR_GREEN,
            fields: vec![],
            photo: None,
            custom_telegram_html: None,
        });
    }

    pub fn recovery(&self, attempt: u32, reason: &str) {
        let flag = self.settings.as_ref().map(|s| s.read().webhook.recovery).unwrap_or(false);
        if !flag {
            return;
        }
        self.send(Notification {
            title: "⚠️ Macro Recovery".into(),
            desc: reason.into(),
            color: COLOR_RED,
            fields: vec![("Attempt".into(), attempt.to_string())],
            photo: None,
            custom_telegram_html: None,
        });
    }

    pub fn notify_message(&self, title: &str, msg: &str) {
        self.send(Notification {
            title: title.into(),
            desc: msg.into(),
            color: COLOR_GREEN,
            fields: vec![],
            photo: None,
            custom_telegram_html: Some(msg.into()),
        });
    }
}

fn embed(title: &str, desc: &str, color: u32, fields: Vec<Value>) -> Value {
    json!({
        "title": title,
        "description": desc,
        "color": color,
        "fields": fields,
        "footer": { "text": concat!("GPO Autofish ", env!("CARGO_PKG_VERSION")) },
        "timestamp": chrono_now(),
    })
}

fn field(name: &str, value: &str) -> Value {
    json!({ "name": name, "value": value, "inline": true })
}

fn fmt_runtime(s: u64) -> String {
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

fn chrono_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    let rem = secs % 86400;
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn http_client() -> Result<&'static reqwest::blocking::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::blocking::Client, String>> = OnceLock::new();
    CLIENT.get_or_init(|| reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(10))
        .build().map_err(|_| "Could not initialise notification transport".to_string()))
        .as_ref().map_err(Clone::clone)
}

fn post_discord(url: &str, body: &Value) -> Result<u16, String> {
    let client = http_client()?;
    let resp = client.post(url).json(body).send().map_err(|e| e.to_string())?;
    let status = resp.status();
    if status.is_success() {
        Ok(status.as_u16())
    } else {
        Err(format!("Discord returned {}", status.as_u16()))
    }
}

fn post_discord_photo(url: &str, body: &Value, photo: &[u8]) -> Result<u16, String> {
    let client = http_client()?;
    let part = reqwest::blocking::multipart::Part::bytes(photo.to_vec())
        .file_name("catch.png")
        .mime_str("image/png")
        .map_err(|e| e.to_string())?;
    let form = reqwest::blocking::multipart::Form::new()
        .text("payload_json", body.to_string())
        .part("files[0]", part);
    let resp = client.post(url).multipart(form).send().map_err(|e| e.to_string())?;
    let status = resp.status();
    if status.is_success() {
        Ok(status.as_u16())
    } else {
        Err(format!("Discord returned {}", status.as_u16()))
    }
}

pub fn post_telegram(token: &str, chat_id: &str, html: &str) -> Result<u16, String> {
    let client = http_client()?;
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let body = json!({
        "chat_id": chat_id,
        "text": html,
        "parse_mode": "HTML",
        "disable_web_page_preview": true,
    });
    let resp = client.post(&url).json(&body).send().map_err(|e| e.to_string())?;
    let status = resp.status();
    if status.is_success() {
        Ok(status.as_u16())
    } else {
        Err(format!("Telegram returned HTTP {}", status.as_u16()))
    }
}

pub fn post_telegram_with_button(
    token: &str,
    chat_id: &str,
    html: &str,
    btn_text: &str,
    btn_url: &str,
) -> Result<u16, String> {
    let client = http_client()?;
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let body = json!({
        "chat_id": chat_id,
        "text": html,
        "parse_mode": "HTML",
        "disable_web_page_preview": true,
        "reply_markup": {
            "inline_keyboard": [[
                { "text": btn_text, "url": btn_url }
            ]]
        }
    });
    let resp = client.post(&url).json(&body).send().map_err(|e| e.to_string())?;
    let status = resp.status();
    if status.is_success() {
        Ok(status.as_u16())
    } else {
        Err(format!("Telegram returned HTTP {}", status.as_u16()))
    }
}


pub fn post_telegram_photo(token: &str, chat_id: &str, photo: &[u8], caption: &str) -> Result<u16, String> {
    let client = http_client()?;
    let url = format!("https://api.telegram.org/bot{token}/sendPhoto");
    let part = reqwest::blocking::multipart::Part::bytes(photo.to_vec())
        .file_name("catch.png")
        .mime_str("image/png")
        .map_err(|e| e.to_string())?;
    let inline_keyboard = serde_json::json!({
        "inline_keyboard": [
            [
                { "text": "⏸️ Pause", "callback_data": "/pause" },
                { "text": "🔄 Recast", "callback_data": "/recast" }
            ],
            [
                { "text": "🛒 Buy Bait", "callback_data": "/buybait" },
                { "text": "📊 Status", "callback_data": "/status" }
            ]
        ]
    });
    let form = reqwest::blocking::multipart::Form::new()
        .text("chat_id", chat_id.to_string())
        .text("caption", caption.to_string())
        .text("parse_mode", "HTML".to_string())
        .text("reply_markup", inline_keyboard.to_string())
        .part("photo", part);
    let resp = client.post(&url).multipart(form).send().map_err(|e| e.to_string())?;
    let status = resp.status();
    if status.is_success() {
        Ok(status.as_u16())
    } else {
        Err(format!("Telegram photo returned HTTP {}", status.as_u16()))
    }
}

pub(crate) fn should_notify_fruit_outcome(settings: &Settings, catch: &FruitCatchContext, outcome: &StorageOutcome) -> bool {
    if !settings.webhook.enabled || !settings.webhook.fruit_drop { return false; }
    if settings.webhook.pity_zero_only {
        // An unreadable pity is not zero, even for a known legendary fruit.
        // Failed/unknown results retain honest labels and occur only AFTER the
        // attempted workflow, so a genuine pity-zero catch is never silently lost.
        return catch.pity_zero && !matches!(outcome, StorageOutcome::Cancelled);
    }
    !settings.webhook.legendary_only || catch.high_tier()
}

fn storage_notification(settings: &Settings, catch: &FruitCatchContext, outcome: &StorageOutcome, photo: Option<Vec<u8>>) -> Option<Notification> {
    if !should_notify_fruit_outcome(settings, catch, outcome) { return None; }
    let status = outcome.label();
    let detail = if let StorageOutcome::Failed(reason) = outcome { format!("{status}: {reason}") } else { status.into() };
    Some(Notification {
        title: format!("{} — {status}", catch.name), desc: detail,
        color: if matches!(outcome, StorageOutcome::Stored) { COLOR_GREEN } else { COLOR_GOLD },
        fields: vec![
            ("Fruit Name".into(), catch.name.clone()),
            ("Pity".into(), if catch.pity_zero { "0 — pity-zero catch".into() } else { catch.pity.clone().unwrap_or_else(|| "Unreadable — treated as nonzero".into()) }),
            ("Protection".into(), catch.policy.reason.clone()), ("Status".into(), status.into()),
        ], photo, custom_telegram_html: None,
    })
}

fn delivery_log(events: &Sender<BotEvent>, message: String) {
    tracing::warn!("{message}");
    let _ = events.try_send(BotEvent::Log(LogLine { ts: crate::events::now_ms(), level: crate::events::LogLevel::Warn, msg: message }));
}

fn delivery_allowed(settings: &Settings, delivery: &Delivery) -> bool {
    settings.webhook.enabled && (!settings.webhook.pity_zero_only
        || matches!(delivery, Delivery::FruitOutcome(_, true)))
}

fn worker(rx: Receiver<Delivery>, priority_rx: Receiver<Delivery>, settings: Arc<RwLock<Settings>>, events: Sender<BotEvent>) {
    loop {
        let delivery = crossbeam_channel::select_biased! {
            recv(priority_rx) -> item => match item { Ok(item) => item, Err(_) => break },
            recv(rx) -> item => match item { Ok(item) => item, Err(_) => break },
        };
        // Apply current consent/filtering again: changing modes must discard
        // already-queued nonzero catches, routine alerts and world spawns.
        if !delivery_allowed(&settings.read(), &delivery) { continue; }
        let (mut notif, spawn) = match delivery {
            Delivery::Message(notification) | Delivery::FruitOutcome(notification, _) => (notification, None),
            Delivery::Spawn(notification, info) => (notification, Some(info)),
        };
        if let Some(info) = spawn {
            // Clone before calling Gemini: never hold a settings lock over IO.
            let gemini = settings.read().gemini.clone();
            if gemini.enabled && !gemini.api_key.trim().is_empty() {
                if let Ok(message) = crate::core::gemini::rewrite_spawn_message_gemini(
                    info.name.as_deref(), info.location.as_deref(), info.is_ase,
                    &info.text, &gemini.api_key, &gemini.model,
                ) {
                    notif.custom_telegram_html = Some(message);
                }
            }
        }
        let (provider, url, tg_token, tg_chat) = {
            let s = settings.read();
            (
                s.webhook.provider.clone(),
                s.webhook.url.clone(),
                s.webhook.telegram_bot_token.clone(),
                s.webhook.telegram_chat_id.clone(),
            )
        };

        // 1. Discord delivery
        if (provider == "discord" || provider == "both") && !url.trim().is_empty() {
            let discord_fields: Vec<Value> = notif.fields.iter().map(|(k, v)| field(k, v)).collect();
            let mut emb = embed(&notif.title, &notif.desc, notif.color, discord_fields);
            if notif.photo.is_some() {
                emb["image"] = json!({ "url": "attachment://catch.png" });
            }
            let body = json!({ "embeds": [emb] });
            let mut delay = Duration::from_secs(1);
            for attempt in 0..3 {
                let res = if let Some(ref bytes) = notif.photo {
                    post_discord_photo(&url, &body, bytes)
                } else {
                    post_discord(&url, &body)
                };
                match res {
                    Ok(_) => break,
                    Err(e) => {
                        // reqwest errors echo the request URL, which IS the
                        // secret webhook URL — redact before logging.
                        let safe = crate::core::secrets::redact(&e, &[&url]);
                        delivery_log(&events, format!("Discord notification attempt {} failed: {safe}", attempt + 1));
                        if attempt < 2 { std::thread::sleep(delay); delay *= 2; }
                    }
                }
            }
        }

        // 2. Telegram delivery
        if (provider == "telegram" || provider == "both") && !tg_token.trim().is_empty() && !tg_chat.trim().is_empty() {
            let tg_text = if let Some(ref custom) = notif.custom_telegram_html {
                custom.clone()
            } else {
                let mut text = if notif.desc.is_empty() {
                    format!("<b>{}</b>", escape_html(&notif.title))
                } else if notif.desc.contains("<b>") || notif.desc.contains("<i>") || notif.desc.contains("<code>") {
                    format!("<b>{}</b>\n{}", escape_html(&notif.title), notif.desc)
                } else {
                    format!("<b>{}</b>\n{}", escape_html(&notif.title), escape_html(&notif.desc))
                };
                if !notif.fields.is_empty() {
                    text.push_str("\n\n");
                    for (k, v) in &notif.fields {
                        text.push_str(&format!("• <b>{}</b>: {}\n", escape_html(k), escape_html(v)));
                    }
                }
                text
            };
            let mut delay = Duration::from_secs(1);
            for attempt in 0..3 {
                let res = deliver_telegram(&tg_text, notif.photo.as_deref(),
                    |photo, caption| {
                        let result = post_telegram_photo(&tg_token, &tg_chat, photo, caption);
                        if let Err(error) = &result {
                            let safe = crate::core::secrets::redact(error, &[&tg_token, &tg_chat]);
                            delivery_log(&events, format!("Telegram photo could not be delivered; sending text details: {safe}"));
                        }
                        result
                    },
                    |text| post_telegram(&tg_token, &tg_chat, text));
                match res {
                    Ok(_) => break,
                    Err(e) => {
                        // Telegram API URLs embed the bot token — redact it.
                        let safe = crate::core::secrets::redact(&e, &[&tg_token, &tg_chat]);
                        delivery_log(&events, format!("Telegram notification attempt {} failed: {safe}", attempt + 1));
                        if attempt < 2 { std::thread::sleep(delay); delay *= 2; }
                    }
                }
            }
        }
    }
}

// Keep captions comfortably below Telegram's limit, including UTF-16 emoji units.
fn deliver_telegram(text: &str, photo: Option<&[u8]>, mut send_photo: impl FnMut(&[u8], &str) -> Result<u16, String>, mut send_text: impl FnMut(&str) -> Result<u16, String>) -> Result<u16, String> {
    if let Some(photo) = photo {
        if text.encode_utf16().count() > 900 {
            // Text first: retry cannot duplicate an already-sent photo if text fails.
            let delivered = send_text(text)?;
            let _ = send_photo(photo, "Fruit event — details in the message above.");
            return Ok(delivered);
        }
        if let Ok(status) = send_photo(photo, text) { return Ok(status); }
    }
    send_text(text)
}

#[cfg(test)]
mod fruit_delivery_tests {
    use super::*;
    #[test]
    fn spawn_enrichment_is_queued_and_never_runs_on_the_producer() {
        let mut settings = Settings::default();
        settings.webhook.enabled = true;
        settings.webhook.provider = "telegram".into();
        settings.webhook.telegram_bot_token = "unused-test-token".into();
        settings.webhook.telegram_chat_id = "unused-test-chat".into();
        settings.gemini.enabled = true;
        settings.gemini.api_key = "unused-test-key".into();
        let settings = Arc::new(RwLock::new(settings));
        let (tx, rx) = unbounded(); // no transport worker; no external requests
        let queue = WebhookQueue { tx: Some(tx), priority_tx: None, settings: Some(settings.clone()) };
        let info = SpawnInfo { text: "A Kilo has spawned at Orange Town".into(), name: Some("Kilo".into()), location: Some("Orange Town".into()), is_ase: false, custom_telegram_message: None };
        queue.spawn(&info, None);
        assert!(settings.try_write().is_some(), "producer must release settings locks");
        match rx.try_recv().unwrap() {
            Delivery::Spawn(notification, context) => {
                assert_eq!(context.name, info.name);
                assert!(notification.custom_telegram_html.unwrap().contains("Kilo"));
            }
            _ => panic!("enrichment context must travel to the worker"),
        }
    }
    fn catch(raw: &str) -> FruitCatchContext {
        let d = DropInfo { name: None, pity: None, text: raw.into(), is_legendary: false, custom_telegram_message: None };
        FruitCatchContext::new(&d, raw, true, true)
    }
    #[test] fn pity_zero_unknown_name_bypasses_rarity_filter() {
        let mut s = Settings::default(); s.webhook.enabled = true;
        for outcome in [StorageOutcome::Stored, StorageOutcome::ProtectedKept, StorageOutcome::Dropped, StorageOutcome::Unknown] {
            let n = storage_notification(&s, &catch("0/40"), &outcome, Some(vec![1])).unwrap();
            assert_eq!(n.photo, Some(vec![1])); assert!(n.title.contains(outcome.label()));
        }
        assert!(storage_notification(&s, &catch("4/40"), &StorageOutcome::Stored, None).is_none());
        s.webhook.fruit_drop = false;
        assert!(storage_notification(&s, &catch("0/40"), &StorageOutcome::Stored, None).is_none());
        s.webhook.fruit_drop = true; s.webhook.enabled = false;
        assert!(storage_notification(&s, &catch("0/40"), &StorageOutcome::Stored, None).is_none());
    }
    #[test]
    fn strict_pity_zero_rejects_nonzero_and_unreadable_legendary_catches() {
        let mut s = Settings::default(); s.webhook.enabled = true; s.webhook.pity_zero_only = true;
        for name in ["Pika", "Devil Fruit"] {
            for pity in [None, Some("4/40")] {
                let d = DropInfo { name: Some(name.into()), pity: pity.map(str::to_string), text: String::new(), is_legendary: true, custom_telegram_message: None };
                let c = FruitCatchContext::new(&d, "", true, true);
                assert!(storage_notification(&s, &c, &StorageOutcome::Stored, Some(vec![1])).is_none());
            }
        }
        for raw in ["0/40", "0/100"] {
            for outcome in [StorageOutcome::Stored, StorageOutcome::Dropped] {
                let n = storage_notification(&s, &catch(raw), &outcome, Some(vec![7])).unwrap();
                assert!(n.title.contains(outcome.label()));
                assert_eq!(n.photo, Some(vec![7]));
            }
            let n = storage_notification(&s, &catch(raw), &StorageOutcome::Unknown, None).unwrap();
            assert!(n.title.contains("unconfirmed"), "never mislabel a failed workflow as stored");
        }
        assert!(storage_notification(&s, &catch("0/40"), &StorageOutcome::Cancelled, None).is_none());
    }

    #[test]
    fn strict_mode_suppresses_pre_action_duplicate_queue_messages() {
        let mut s = Settings::default(); s.webhook.enabled = true; s.webhook.pity_zero_only = true;
        s.webhook.telegram_bot_token = "unused-test-token".into();
        s.webhook.telegram_chat_id = "unused-test-chat".into();
        let settings = Arc::new(RwLock::new(s));
        let (tx, rx) = unbounded();
        let queue = WebhookQueue { tx: Some(tx), priority_tx: None, settings: Some(settings) };
        let d = DropInfo { name: None, pity: Some("0/40".into()), text: "0/40".into(), is_legendary: false, custom_telegram_message: None };
        queue.fruit_drop(&d, Some(vec![1]));
        assert!(rx.try_recv().is_err(), "no screenshot or notification before storage");
        queue.fruit_storage_outcome(&catch("0/40"), &StorageOutcome::Stored, Some(vec![2]));
        match rx.try_recv().unwrap() {
            Delivery::FruitOutcome(n, true) => { assert!(n.title.contains("Stored")); assert_eq!(n.photo, Some(vec![2])); }
            _ => panic!("expected one post-storage result"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn enabling_strict_mode_discards_already_queued_unrelated_alerts() {
        let mut s = Settings::default(); s.webhook.enabled = true;
        let notification = storage_notification(&s, &catch("0/40"), &StorageOutcome::Stored, None).unwrap();
        let unrelated = Delivery::Message(notification.clone());
        assert!(delivery_allowed(&s, &unrelated));
        s.webhook.pity_zero_only = true;
        assert!(!delivery_allowed(&s, &unrelated));
        assert!(!delivery_allowed(&s, &Delivery::FruitOutcome(notification.clone(), false)));
        assert!(delivery_allowed(&s, &Delivery::FruitOutcome(notification.clone(), true)));
        s.webhook.enabled = false;
        assert!(!delivery_allowed(&s, &Delivery::FruitOutcome(notification, true)));
    }

    #[test]
    fn queue_pressure_never_blocks_and_priority_has_reserved_capacity() {
        let mut s = Settings::default();
        s.webhook.enabled = true;
        s.webhook.telegram_bot_token = "synthetic-test-token".into();
        s.webhook.telegram_chat_id = "synthetic-test-chat".into();
        let (tx, rx) = bounded(1);
        let (priority_tx, priority_rx) = bounded(1);
        let q = WebhookQueue { tx: Some(tx), priority_tx: Some(priority_tx), settings: Some(Arc::new(RwLock::new(s))) };
        let at = std::time::Instant::now();
        q.notify_message("first", "routine");
        q.notify_message("full", "must not block");
        q.fruit_storage_outcome(&catch("0/40"), &StorageOutcome::Stored, Some(vec![1; 1024 * 1024 + 1]));
        assert!(at.elapsed() < Duration::from_millis(100));
        assert_eq!(rx.len(), 1);
        match priority_rx.try_recv().unwrap() {
            Delivery::FruitOutcome(n, true) => assert!(n.photo.is_none()),
            _ => panic!("priority result must not compete with routine messages"),
        }
    }

    #[test] fn photo_failure_falls_back_to_text() {
        let mut text_calls = 0;
        assert_eq!(deliver_telegram("Stored", Some(&[1]), |_, _| Err("photo failed".into()), |_| { text_calls += 1; Ok(200) }), Ok(200));
        assert_eq!(text_calls, 1);
    }
    #[test] fn long_caption_is_separate_text() {
        let long = "🌟".repeat(500);
        let mut photos = 0; let mut texts = 0;
        assert_eq!(deliver_telegram(&long, Some(&[1]), |_, caption| { photos += 1; assert!(caption.len() < 100); Ok(200) }, |text| { texts += 1; assert_eq!(text, long); Ok(200) }), Ok(200));
        assert_eq!((photos, texts), (1, 1));
    }
    #[test] fn delivery_failure_is_returned() {
        assert!(deliver_telegram("Stored", None, |_, _| Ok(200), |_| Err("offline".into())).is_err());
    }
}
