//! Rebuilt delivery queue. Game input is never executed by this module.
use crate::{
    config::Settings,
    core::{
        fruit::{DropInfo, SpawnInfo},
        fruit_storage::{FruitCatchContext, StorageOutcome},
        platform::Platform,
    },
    events::{BotEvent, Stats},
};
use crossbeam_channel::{bounded, Sender};
use parking_lot::RwLock;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, OnceLock,
};
use std::time::{Duration, Instant};
#[derive(Clone)]
struct Message {
    title: String,
    body: String,
    photo: Option<Vec<u8>>,
    created: Instant,
}
#[derive(Clone, Default)]
pub struct DeliveryHealth {
    pub delivered: u64,
    pub failed: u64,
    pub last_error: Option<String>,
}
pub struct WebhookQueue {
    sender: Option<Sender<Message>>,
    settings: Option<Arc<RwLock<Settings>>>,
    platform: Arc<RwLock<Option<Platform>>>,
    health: Arc<RwLock<DeliveryHealth>>,
    pending: Arc<AtomicUsize>,
}
pub fn should_notify_fruit_outcome(
    s: &Settings,
    catch: &FruitCatchContext,
    outcome: &StorageOutcome,
) -> bool {
    s.webhook.enabled
        && s.webhook.fruit_drop
        && !matches!(outcome, StorageOutcome::Cancelled)
        && if s.webhook.pity_zero_only {
            catch.pity_zero
        } else {
            !s.webhook.legendary_only || catch.high_tier()
        }
}
impl WebhookQueue {
    pub fn disabled() -> Self {
        Self {
            sender: None,
            settings: None,
            platform: Arc::new(RwLock::new(None)),
            health: Arc::new(RwLock::new(DeliveryHealth::default())),
            pending: Arc::new(AtomicUsize::new(0)),
        }
    }
    pub fn start(settings: Arc<RwLock<Settings>>, _events: Sender<BotEvent>) -> Arc<Self> {
        let (sender, receiver) = bounded::<Message>(4);
        let queue = Arc::new(Self {
            sender: Some(sender),
            settings: Some(settings.clone()),
            platform: Arc::new(RwLock::new(None)),
            health: Arc::new(RwLock::new(DeliveryHealth::default())),
            pending: Arc::new(AtomicUsize::new(0)),
        });
        let platform = queue.platform.clone();
        let health = queue.health.clone();
        let pending = queue.pending.clone();
        std::thread::spawn(move || {
            while let Ok(mut message) = receiver.recv() {
                let s = settings.read().clone();
                if !s.webhook.enabled {
                    pending.fetch_sub(1, Ordering::SeqCst);
                    continue;
                }
                if message.photo.is_none()
                    && s.webhook.send_screenshot
                    && message.title.contains("Pity 0")
                    && message.created.elapsed() < Duration::from_secs(3)
                {
                    if let Some(p) = platform.read().as_ref() {
                        if let Some(w) = p.window.find().filter(|w| w.visible && w.is_foreground) {
                            if let Ok(frame) = p.capture.grab(w.client) {
                                if p.window
                                    .find()
                                    .is_some_and(|n| n.is_foreground && n.client == w.client)
                                {
                                    message.photo = frame.downscale(1280).to_jpeg_bytes(80).ok();
                                }
                            }
                        }
                    }
                }
                let mut result = Err("No notification provider is configured".into());
                for attempt in 0..3 {
                    result = deliver(&s, &message);
                    if result.is_ok() {
                        break;
                    }
                    if attempt < 2 {
                        std::thread::sleep(Duration::from_millis(500 * (1u64 << attempt)));
                    }
                }
                let mut status = health.write();
                match result {
                    Ok(()) => {
                        status.delivered += 1;
                        status.last_error = None;
                    }
                    Err(error) => {
                        status.failed += 1;
                        status.last_error = Some(error);
                    }
                }
                pending.fetch_sub(1, Ordering::SeqCst);
            }
        });
        queue
    }
    pub fn attach_platform(&self, platform: Platform) {
        *self.platform.write() = Some(platform);
    }
    pub fn health(&self) -> (u64, u64, Option<String>, usize) {
        let h = self.health.read();
        (
            h.delivered,
            h.failed,
            h.last_error.clone(),
            self.pending.load(Ordering::SeqCst),
        )
    }
    fn send(&self, title: &str, body: String, photo: Option<Vec<u8>>, is_outcome: bool) {
        let Some(settings) = self.settings.as_ref() else {
            return;
        };
        let s = settings.read();
        if !s.webhook.enabled || (!is_outcome && s.webhook.pity_zero_only) {
            return;
        }
        drop(s);
        let Some(sender) = &self.sender else {
            return;
        };
        let photo = photo.filter(|bytes| bytes.len() <= 4 * 1024 * 1024);
        self.pending.fetch_add(1, Ordering::SeqCst);
        if sender
            .try_send(Message {
                title: title.into(),
                body,
                photo,
                created: Instant::now(),
            })
            .is_err()
        {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            self.health.write().last_error = Some("Delivery queue is full".into());
        }
    }
    pub fn read_local_pity(&self) -> Option<String> {
        let p = self.platform.read().clone()?;
        let w = p.window.find().filter(|w| w.visible && w.is_foreground)?;
        let frame = p.capture.grab(w.client).ok()?.downscale(1920);
        let text = p
            .ocr
            .read_with_timeout(&frame, Duration::from_millis(450))
            .ok()?;
        if !p
            .window
            .find()
            .is_some_and(|n| n.is_foreground && n.client == w.client)
        {
            return None;
        }
        let cap = self.settings.as_ref()?.read().fruit_storage.pity_cap;
        parse_pity(&text, cap)
    }
    pub fn test(&self) -> Result<(), String> {
        let s = self
            .settings
            .as_ref()
            .ok_or("Notifications are not configured")?
            .read()
            .clone();
        deliver(
            &s,
            &Message {
                title: "GPO Autofish test".into(),
                body: "This is an explicitly requested delivery test.".into(),
                photo: None,
                created: Instant::now(),
            },
        )
    }
    pub fn progress(&self, s: Stats) {
        self.send(
            "Fishing progress",
            format!("Confirmed catches: {}", s.fish),
            None,
            false,
        );
    }
    pub fn fruit_drop(&self, d: &DropInfo, photo: Option<Vec<u8>>) {
        self.send(
            "Fruit caught",
            format!(
                "{}\n{}",
                d.name.as_deref().unwrap_or("Unidentified fruit"),
                d.text
            ),
            photo,
            false,
        );
    }
    pub fn fruit_storage_outcome(
        &self,
        catch: &FruitCatchContext,
        outcome: &StorageOutcome,
        photo: Option<Vec<u8>>,
    ) {
        let Some(settings) = &self.settings else {
            return;
        };
        if !should_notify_fruit_outcome(&settings.read(), catch, outcome) {
            return;
        }
        let title = if catch.pity_zero {
            "Pity 0 — fruit workflow"
        } else {
            "Fruit workflow"
        };
        self.send(
            title,
            format!(
                "Fruit: {}\nReported result: {}\nPity: {}",
                catch.name,
                outcome.detail(),
                catch.pity.as_deref().unwrap_or("unreadable")
            ),
            photo,
            true,
        );
    }
    pub fn disconnect(&self, reason: &str, photo: Option<Vec<u8>>) {
        self.send("Connection interrupted", reason.into(), photo, false);
    }
    pub fn reconnected(&self, photo: Option<Vec<u8>>) {
        self.send(
            "Connection recovered",
            "Roblox was reconnected.".into(),
            photo,
            false,
        );
    }
    pub fn bait_depleted(&self) {
        self.send(
            "Bait unavailable",
            "Check your bait stock.".into(),
            None,
            false,
        );
    }
    pub fn spawn(&self, info: &SpawnInfo, photo: Option<Vec<u8>>) {
        self.send("World fruit spawn", info.text.clone(), photo, false);
    }
    pub fn purchase(&self, amount: u32) {
        self.send(
            "Bait purchase",
            format!("Requested amount: {amount}"),
            None,
            false,
        );
    }
    pub fn recovery(&self, attempt: u32, reason: &str) {
        self.send(
            "Fishing recovery",
            format!("Attempt {attempt}: {reason}"),
            None,
            false,
        );
    }
    pub fn notify_message(&self, title: &str, msg: &str) {
        self.send(title, msg.into(), None, false);
    }
}
fn parse_pity(text: &str, cap: u32) -> Option<String> {
    let normalized = text.to_ascii_lowercase().replace('/', " / ");
    let words: Vec<_> = normalized
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '/')
        .filter(|s| !s.is_empty())
        .collect();
    for (index, word) in words.iter().enumerate() {
        if *word != "pity" && *word != "legendarypity" {
            continue;
        }
        let current = words
            .get(index + 1)?
            .replace('o', "0")
            .parse::<u32>()
            .ok()?;
        if words.get(index + 2) != Some(&"/") {
            continue;
        }
        let total = words
            .get(index + 3)?
            .replace('o', "0")
            .parse::<u32>()
            .ok()?;
        if total == cap && current <= total {
            return Some(format!("{current}/{total}"));
        }
    }
    None
}
fn client() -> Result<&'static reqwest::blocking::Client, String> {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|_| "Notification HTTP client could not start")?;
    let _ = CLIENT.set(client);
    Ok(CLIENT.get().expect("client initialized"))
}
fn plain(text: &str, limit: usize) -> String {
    let mut result = String::new();
    let mut tag = false;
    let mut units = 0;
    for c in text.chars() {
        if c == '<' {
            tag = true;
            continue;
        }
        if c == '>' {
            tag = false;
            continue;
        }
        if tag {
            continue;
        }
        if units + c.len_utf16() > limit {
            break;
        }
        units += c.len_utf16();
        result.push(c);
    }
    result
}
fn telegram_result(response: reqwest::blocking::Response) -> Result<u16, String> {
    let status = response.status();
    let value = response.json::<Value>().unwrap_or(Value::Null);
    if status.is_success() && value["ok"].as_bool() == Some(true) {
        Ok(status.as_u16())
    } else {
        Err(format!(
            "Telegram rejected delivery (HTTP {}; check bot and chat access)",
            status.as_u16()
        ))
    }
}
pub fn post_telegram(token: &str, chat: &str, text: &str) -> Result<u16, String> {
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let response = client()?
        .post(url)
        .json(&json!({"chat_id":chat,"text":plain(text,3500),"disable_web_page_preview":true}))
        .send()
        .map_err(|_| "Telegram network request failed")?;
    telegram_result(response)
}
pub fn post_telegram_with_button(
    token: &str,
    chat: &str,
    text: &str,
    label: &str,
    url: &str,
) -> Result<u16, String> {
    let response=client()?.post(format!("https://api.telegram.org/bot{token}/sendMessage")).json(&json!({"chat_id":chat,"text":plain(text,3500),"reply_markup":{"inline_keyboard":[[{"text":label,"url":url}]]}})).send().map_err(|_|"Telegram network request failed")?;
    telegram_result(response)
}
pub fn post_telegram_photo(
    token: &str,
    chat: &str,
    photo: &[u8],
    caption: &str,
) -> Result<u16, String> {
    let image = image::load_from_memory(photo)
        .map_err(|_| "Screenshot could not be decoded")?
        .thumbnail(1280, 1280)
        .to_rgb8();
    let mut bytes = vec![];
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 82)
        .encode(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|_| "Screenshot could not be encoded")?;
    let part = reqwest::blocking::multipart::Part::bytes(bytes)
        .file_name("fruit-result.jpg")
        .mime_str("image/jpeg")
        .map_err(|_| "Screenshot format failed")?;
    let form = reqwest::blocking::multipart::Form::new()
        .text("chat_id", chat.to_string())
        .text("caption", plain(caption, 900))
        .part("photo", part);
    let response = client()?
        .post(format!("https://api.telegram.org/bot{token}/sendPhoto"))
        .multipart(form)
        .send()
        .map_err(|_| "Telegram photo request failed")?;
    telegram_result(response)
}
fn deliver(s: &Settings, m: &Message) -> Result<(), String> {
    let text = format!("{}\n\n{}", m.title, m.body);
    let mut sent = false;
    if matches!(s.webhook.provider.as_str(), "telegram" | "both") {
        if s.webhook.telegram_bot_token.trim().is_empty()
            || s.webhook.telegram_chat_id.trim().is_empty()
        {
            return Err("Telegram bot token or chat is missing".into());
        }
        if let Some(photo) = &m.photo {
            if post_telegram_photo(
                &s.webhook.telegram_bot_token,
                &s.webhook.telegram_chat_id,
                photo,
                &text,
            )
            .is_err()
            {
                post_telegram(
                    &s.webhook.telegram_bot_token,
                    &s.webhook.telegram_chat_id,
                    &text,
                )?;
            }
        } else {
            post_telegram(
                &s.webhook.telegram_bot_token,
                &s.webhook.telegram_chat_id,
                &text,
            )?;
        }
        sent = true;
    }
    if matches!(s.webhook.provider.as_str(), "discord" | "both") {
        if s.webhook.url.trim().is_empty() {
            return Err("Discord webhook URL is missing".into());
        }
        let response = client()?
            .post(&s.webhook.url)
            .json(&json!({"content":plain(&text,1900)}))
            .send()
            .map_err(|_| "Discord network request failed")?;
        if !response.status().is_success() {
            return Err(format!(
                "Discord rejected delivery (HTTP {})",
                response.status().as_u16()
            ));
        }
        sent = true;
    }
    if sent {
        Ok(())
    } else {
        Err("Choose Telegram, Discord or both in notification settings".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unrelated_zero_is_not_pity() {
        assert_eq!(parse_pity("HP 0/100 Backpack 0/40", 40), None);
        assert_eq!(parse_pity("Legendary Pity 0/40", 40), Some("0/40".into()));
        assert_eq!(parse_pity("Pity 28/40", 40), Some("28/40".into()));
    }
    #[test]
    fn message_limits_count_unicode_units() {
        assert!(plain(&"🎣".repeat(2000), 3500).encode_utf16().count() <= 3500);
    }
}
