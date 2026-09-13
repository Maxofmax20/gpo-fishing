use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};
use parking_lot::RwLock;
use serde_json::{json, Value};

use crate::config::Settings;
use crate::core::fruit::{DropInfo, SpawnInfo};
use crate::events::Stats;

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
}

pub struct WebhookQueue {
    tx: Option<Sender<Notification>>,
    settings: Option<Arc<RwLock<Settings>>>,
}

impl WebhookQueue {
    pub fn disabled() -> Self {
        Self { tx: None, settings: None }
    }

    pub fn start(settings: Arc<RwLock<Settings>>) -> Arc<Self> {
        let (tx, rx) = unbounded::<Notification>();
        let s2 = Arc::clone(&settings);
        std::thread::Builder::new()
            .name("webhook".into())
            .spawn(move || worker(rx, s2))
            .expect("spawn webhook worker");
        Arc::new(Self { tx: Some(tx), settings: Some(settings) })
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
        if !self.enabled() {
            return;
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(notif);
        }
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
        });
    }

    pub fn fruit_drop(&self, d: &DropInfo, photo: Option<Vec<u8>>) {
        let fruit_name = d.name.as_deref().unwrap_or("Devil Fruit");
        let rarity = crate::core::fruit::fruit_rarity(fruit_name);
        let pity_info = if let Some(p) = &d.pity {
            if p.starts_with("0/") || d.is_legendary {
                format!("🌟 Legendary Pity: {p} (HIT! Guaranteed Legendary/Mythical!)")
            } else {
                format!("⚡ Legendary Pity: {p}")
            }
        } else if d.is_legendary {
            "🌟 Legendary Pity: 0/100 (HIT! Guaranteed Legendary/Mythical!)".into()
        } else {
            "⚡ Check backpack".into()
        };

        let (title, desc, color) = if rarity == crate::core::fruit::FruitRarity::Mythical {
            ("🔥 MYTHICAL DEVIL FRUIT DROPPED!", format!("🎉 Extraordinary luck! You got a Mythical Devil Fruit: {fruit_name}!\n\n{pity_info}"), COLOR_GOLD)
        } else if d.is_legendary || rarity == crate::core::fruit::FruitRarity::Legendary {
            ("🌟 Legendary Devil Fruit Dropped!", format!("Pity reset to 0! You got a legendary devil fruit: {fruit_name}!\n\n{pity_info}"), COLOR_GOLD)
        } else if rarity != crate::core::fruit::FruitRarity::Unknown {
            ("🍇 Devil Fruit Dropped!", format!("You got a {} devil fruit: {fruit_name}.\n\n{pity_info}", rarity.as_str()), COLOR_PURPLE)
        } else {
            ("🍇 Devil Fruit Dropped!", format!("You got a devil fruit drop!\n\n{pity_info}"), COLOR_PURPLE)
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
        });
    }

    pub fn fruit_stored(&self, fruit_name: &str, photo: Option<Vec<u8>>) {
        let rarity = crate::core::fruit::fruit_rarity(fruit_name);
        let title = if rarity == crate::core::fruit::FruitRarity::Mythical {
            "🔥 Mythical Devil Fruit Stored!"
        } else if rarity == crate::core::fruit::FruitRarity::Legendary {
            "🌟 Legendary Devil Fruit Stored!"
        } else {
            "📦 Devil Fruit Stored!"
        };
        let desc = if rarity != crate::core::fruit::FruitRarity::Unknown {
            format!("Successfully stored <b>{fruit_name}</b> ({}) into your inventory/bag.", rarity.as_str())
        } else {
            format!("Successfully stored <b>{fruit_name}</b> into your inventory/bag.")
        };
        self.send(Notification {
            title: title.into(),
            desc,
            color: COLOR_GREEN,
            fields: vec![
                ("Fruit Name".into(), fruit_name.to_string()),
                ("Rarity".into(), rarity.as_str().to_string()),
                ("Status".into(), "Stored safely in inventory".into()),
            ],
            photo,
        });
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
        });
    }

    pub fn spawn(&self, info: &SpawnInfo) {
        let at = info.location.as_deref().map(|l| format!(" at {l}")).unwrap_or_default();
        let (title, desc, color) = match &info.name {
            Some(n) => ("🌀 Devil fruit spawned", format!("{n} has spawned{at}."), COLOR_PURPLE),
            None => ("🌀 Devil fruit spawned", format!("A devil fruit has spawned{at}!"), COLOR_BLUE),
        };
        self.send(Notification {
            title: title.into(),
            desc,
            color,
            fields: vec![],
            photo: None,
        });
    }

    pub fn purchase(&self, amount: u32) {
        self.send(Notification {
            title: "🛒 Bait purchased".into(),
            desc: format!("Bought {amount} bait."),
            color: COLOR_GREEN,
            fields: vec![],
            photo: None,
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
        });
    }
}

fn embed(title: &str, desc: &str, color: u32, fields: Vec<Value>) -> Value {
    json!({
        "title": title,
        "description": desc,
        "color": color,
        "fields": fields,
        "footer": { "text": "GPO Autofish v4" },
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

fn post_discord(url: &str, body: &Value) -> Result<u16, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.post(url).json(body).send().map_err(|e| e.to_string())?;
    let status = resp.status();
    if status.is_success() {
        Ok(status.as_u16())
    } else {
        Err(format!("Discord returned {}", status.as_u16()))
    }
}

fn post_discord_photo(url: &str, body: &Value, photo: &[u8]) -> Result<u16, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
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
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
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
        let err_body = resp.text().unwrap_or_default();
        Err(format!("Telegram error {}: {}", status.as_u16(), err_body))
    }
}

pub fn post_telegram_photo(token: &str, chat_id: &str, photo: &[u8], caption: &str) -> Result<u16, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("https://api.telegram.org/bot{token}/sendPhoto");
    let part = reqwest::blocking::multipart::Part::bytes(photo.to_vec())
        .file_name("catch.png")
        .mime_str("image/png")
        .map_err(|e| e.to_string())?;
    let form = reqwest::blocking::multipart::Form::new()
        .text("chat_id", chat_id.to_string())
        .text("caption", caption.to_string())
        .text("parse_mode", "HTML".to_string())
        .part("photo", part);
    let resp = client.post(&url).multipart(form).send().map_err(|e| e.to_string())?;
    let status = resp.status();
    if status.is_success() {
        Ok(status.as_u16())
    } else {
        let err_body = resp.text().unwrap_or_default();
        Err(format!("Telegram photo error {}: {}", status.as_u16(), err_body))
    }
}

fn worker(rx: Receiver<Notification>, settings: Arc<RwLock<Settings>>) {
    for notif in rx.iter() {
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
                        tracing::warn!("discord webhook attempt {} failed: {e}", attempt + 1);
                        std::thread::sleep(delay);
                        delay *= 2;
                    }
                }
            }
        }

        // 2. Telegram delivery
        if (provider == "telegram" || provider == "both") && !tg_token.trim().is_empty() && !tg_chat.trim().is_empty() {
            let mut tg_text = if notif.desc.is_empty() {
                format!("<b>{}</b>", escape_html(&notif.title))
            } else {
                format!("<b>{}</b>\n{}", escape_html(&notif.title), escape_html(&notif.desc))
            };
            if !notif.fields.is_empty() {
                tg_text.push_str("\n\n");
                for (k, v) in &notif.fields {
                    tg_text.push_str(&format!("• <b>{}</b>: {}\n", escape_html(k), escape_html(v)));
                }
            }
            let mut delay = Duration::from_secs(1);
            for attempt in 0..3 {
                let res = if let Some(ref bytes) = notif.photo {
                    post_telegram_photo(&tg_token, &tg_chat, bytes, &tg_text)
                } else {
                    post_telegram(&tg_token, &tg_chat, &tg_text)
                };
                match res {
                    Ok(_) => break,
                    Err(e) => {
                        tracing::warn!("telegram attempt {} failed: {e}", attempt + 1);
                        std::thread::sleep(delay);
                        delay *= 2;
                    }
                }
            }
        }
    }
}
