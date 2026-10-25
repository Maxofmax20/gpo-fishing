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
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    Progress,
    FruitCatch { high_tier: bool },
    FruitOutcome { pity_zero: bool, high_tier: bool },
    Spawn,
    Purchase,
    Recovery,
    Disconnect,
    Reconnected,
    Bait,
    General,
    Test,
}
impl Event {
    fn allowed(self, s: &Settings) -> bool {
        let w = &s.webhook;
        if !w.enabled {
            return false;
        }
        // Test is an explicit user action, not an automatic catch notification.
        if self == Self::Test {
            return true;
        }
        if w.pity_zero_only {
            return matches!(
                self,
                Self::FruitOutcome {
                    pity_zero: true,
                    ..
                }
            ) && w.fruit_drop;
        }
        match self {
            Self::Progress => w.progress,
            Self::FruitCatch { high_tier } | Self::FruitOutcome { high_tier, .. } => {
                w.fruit_drop && (!w.legendary_only || high_tier)
            }
            Self::Spawn => w.spawn,
            Self::Purchase => w.purchase,
            Self::Recovery => w.recovery,
            Self::Disconnect | Self::Reconnected => w.disconnect_alert,
            Self::Bait => w.bait_alert,
            Self::General | Self::Test => true,
        }
    }
    fn photo_allowed(self, s: &Settings) -> bool {
        s.webhook.send_screenshot
            && match self {
                Self::FruitCatch { .. } => s.webhook.send_catch_screenshot,
                Self::FruitOutcome { .. } => s.webhook.send_drop_screenshot,
                Self::Spawn | Self::Disconnect | Self::Reconnected => true,
                _ => false,
            }
    }
}
#[derive(Clone)]
struct Message {
    title: String,
    body: String,
    photo: Option<Vec<u8>>,
    event: Event,
    fruit_crop: bool,
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
    !matches!(outcome, StorageOutcome::Cancelled)
        && Event::FruitOutcome {
            pity_zero: catch.pity_zero,
            high_tier: catch.high_tier(),
        }
        .allowed(s)
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
        let health = queue.health.clone();
        let pending = queue.pending.clone();
        std::thread::spawn(move || {
            while let Ok(message) = receiver.recv() {
                let mut result = Ok(false);
                let mut ack = ProviderAcks::default();
                for attempt in 0..3 {
                    // Settings can change while queued or while a provider is offline.
                    // Never retain a consent snapshot over retries or provider requests.
                    result = deliver_pending_using(
                        || settings.read().clone(),
                        &message,
                        &mut ack,
                        &HttpTransport,
                    );
                    if result.is_ok() {
                        break;
                    }
                    if attempt < 2 {
                        std::thread::sleep(Duration::from_millis(500 * (1u64 << attempt)));
                    }
                }
                let mut status = health.write();
                match result {
                    Ok(true) => {
                        status.delivered += 1;
                        status.last_error = None;
                    }
                    Ok(false) => {} // Revoked consent is suppression, not successful delivery.
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
    fn send(&self, event: Event, title: &str, body: String, photo: Option<Vec<u8>>) {
        let Some(settings) = self.settings.as_ref() else {
            return;
        };
        let s = settings.read();
        if !event.allowed(&s) {
            return;
        }
        // The producer supplied the actual event-time crop. Do not recapture a
        // later unrelated game screen or resurrect a photo disabled at enqueue.
        let photo = photo.filter(|bytes| event.photo_allowed(&s) && bytes.len() <= 4 * 1024 * 1024);
        let fruit_crop = s.webhook.crop_fruit_screenshot;
        drop(s);
        let Some(sender) = &self.sender else {
            return;
        };
        self.pending.fetch_add(1, Ordering::SeqCst);
        if sender
            .try_send(Message {
                title: title.into(),
                body,
                photo,
                event,
                fruit_crop,
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
        let settings = self
            .settings
            .as_ref()
            .ok_or("Notifications are not configured")?;
        let message = Message {
            title: "GPO Autofish test".into(),
            body: "This is an explicitly requested delivery test.".into(),
            photo: None,
            event: Event::Test,
            fruit_crop: false,
        };
        deliver_pending_using(
            || settings.read().clone(),
            &message,
            &mut ProviderAcks::default(),
            &HttpTransport,
        )
        .map(|_| ())
    }
    pub fn progress(&self, s: Stats) {
        self.send(
            Event::Progress,
            "Fishing progress",
            format!("Confirmed catches: {}", s.fish),
            None,
        );
    }
    pub fn fruit_drop(&self, d: &DropInfo, photo: Option<Vec<u8>>) {
        let high_tier = d.is_legendary
            || d.name
                .as_deref()
                .is_some_and(|name| crate::core::fruit::fruit_rarity(name).is_high_tier())
            || crate::core::fruit::is_pity_zero(d.pity.as_deref(), &d.text);
        self.send(
            Event::FruitCatch { high_tier },
            "Fruit caught",
            format!(
                "{}\n{}",
                d.name.as_deref().unwrap_or("Unidentified fruit"),
                d.text
            ),
            photo,
        );
    }
    pub fn fruit_storage_outcome(
        &self,
        catch: &FruitCatchContext,
        outcome: &StorageOutcome,
        photo: Option<Vec<u8>>,
    ) {
        if matches!(outcome, StorageOutcome::Cancelled) {
            return;
        }
        let title = if catch.pity_zero {
            "Pity 0 — fruit workflow"
        } else {
            "Fruit workflow"
        };
        self.send(
            Event::FruitOutcome {
                pity_zero: catch.pity_zero,
                high_tier: catch.high_tier(),
            },
            title,
            format!(
                "Fruit: {}\nReported result: {}\nPity: {}",
                catch.name,
                outcome.detail(),
                catch.pity.as_deref().unwrap_or("unreadable")
            ),
            photo,
        );
    }
    pub fn disconnect(&self, reason: &str, photo: Option<Vec<u8>>) {
        self.send(
            Event::Disconnect,
            "Connection interrupted",
            reason.into(),
            photo,
        );
    }
    pub fn reconnected(&self, photo: Option<Vec<u8>>) {
        self.send(
            Event::Reconnected,
            "Connection recovered",
            "Roblox was reconnected.".into(),
            photo,
        );
    }
    pub fn bait_depleted(&self) {
        self.send(
            Event::Bait,
            "Bait unavailable",
            "Check your bait stock.".into(),
            None,
        );
    }
    pub fn spawn(&self, info: &SpawnInfo, photo: Option<Vec<u8>>) {
        self.send(Event::Spawn, "World fruit spawn", info.text.clone(), photo);
    }
    pub fn purchase(&self, amount: u32) {
        self.send(
            Event::Purchase,
            "Bait purchase",
            format!("Requested amount: {amount}"),
            None,
        );
    }
    pub fn recovery(&self, attempt: u32, reason: &str) {
        self.send(
            Event::Recovery,
            "Fishing recovery",
            format!("Attempt {attempt}: {reason}"),
            None,
        );
    }
    pub fn notify_message(&self, title: &str, msg: &str) {
        self.send(Event::General, title, msg.into(), None);
    }
}
fn parse_pity(text: &str, cap: u32) -> Option<String> {
    let normalized = text.to_ascii_lowercase().replace('/', " / ");
    let words: Vec<_> = normalized
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '/')
        .filter(|s| !s.is_empty())
        .collect();
    let mut values = std::collections::BTreeSet::new();
    for (index, word) in words.iter().enumerate() {
        let legendary = *word == "legendarypity"
            || (*word == "pity" && index > 0 && words[index - 1] == "legendary");
        if !legendary {
            continue;
        }
        let Some(current) = words
            .get(index + 1)
            .and_then(|w| w.replace('o', "0").parse::<u32>().ok())
        else {
            continue;
        };
        if words.get(index + 2) != Some(&"/") {
            continue;
        }
        let Some(total) = words
            .get(index + 3)
            .and_then(|w| w.replace('o', "0").parse::<u32>().ok())
        else {
            continue;
        };
        if total == cap && current <= total {
            values.insert(current);
        }
    }
    if values.len() == 1 {
        values.first().map(|current| format!("{current}/{cap}"))
    } else {
        None
    }
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
#[derive(Default)]
struct ProviderAcks {
    telegram: bool,
    discord: bool,
}
trait Transport {
    fn telegram(&self, settings: &Settings, text: &str, photo: Option<&[u8]>)
        -> Result<(), String>;
    fn discord(&self, settings: &Settings, text: &str, photo: Option<&[u8]>) -> Result<(), String>;
}
struct HttpTransport;
impl Transport for HttpTransport {
    fn telegram(&self, s: &Settings, text: &str, photo: Option<&[u8]>) -> Result<(), String> {
        if s.webhook.telegram_bot_token.trim().is_empty()
            || s.webhook.telegram_chat_id.trim().is_empty()
        {
            return Err("Telegram bot token or chat is missing".into());
        }
        if let Some(photo) = photo {
            if post_telegram_photo(
                &s.webhook.telegram_bot_token,
                &s.webhook.telegram_chat_id,
                photo,
                text,
            )
            .is_ok()
            {
                return Ok(());
            }
        }
        post_telegram(
            &s.webhook.telegram_bot_token,
            &s.webhook.telegram_chat_id,
            text,
        )
        .map(|_| ())
    }
    fn discord(&self, s: &Settings, text: &str, photo: Option<&[u8]>) -> Result<(), String> {
        if s.webhook.url.trim().is_empty() {
            return Err("Discord webhook URL is missing".into());
        }
        let body = json!({"content":plain(text,1900)});
        let request = client()?.post(&s.webhook.url);
        let request = if let Some(photo) = photo {
            // Preserve the event-time screenshot for both supported providers.
            let (name, mime) = match image::guess_format(photo) {
                Ok(image::ImageFormat::Png) => ("fruit-result.png", "image/png"),
                Ok(image::ImageFormat::Jpeg) => ("fruit-result.jpg", "image/jpeg"),
                _ => return Err("Screenshot is not a supported PNG or JPEG".into()),
            };
            let part = reqwest::blocking::multipart::Part::bytes(photo.to_vec())
                .file_name(name)
                .mime_str(mime)
                .map_err(|_| "Screenshot format failed")?;
            request.multipart(
                reqwest::blocking::multipart::Form::new()
                    .text("payload_json", body.to_string())
                    .part("files[0]", part),
            )
        } else {
            request.json(&body)
        };
        let response = request
            .send()
            .map_err(|_| "Discord network request failed")?;
        if !response.status().is_success() {
            return Err(format!(
                "Discord rejected delivery (HTTP {})",
                response.status().as_u16()
            ));
        }
        Ok(())
    }
}

/// Read consent again before each provider; a setting revoked during a request
/// prevents the next provider or retry. Already acknowledged providers are not resent.
fn deliver_pending_using(
    settings: impl Fn() -> Settings,
    message: &Message,
    ack: &mut ProviderAcks,
    transport: &impl Transport,
) -> Result<bool, String> {
    let initial = settings();
    if message.event == Event::Test && !initial.webhook.enabled {
        return Err("Enable notifications before testing delivery".into());
    }
    if !message.event.allowed(&initial) {
        return Ok(false);
    }
    let text = format!("{}\n\n{}", message.title, message.body);
    for telegram in [true, false] {
        let current = settings();
        if !message.event.allowed(&current) {
            return Ok(ack.telegram || ack.discord);
        }
        let selected = if telegram {
            matches!(current.webhook.provider.as_str(), "telegram" | "both") && !ack.telegram
        } else {
            matches!(current.webhook.provider.as_str(), "discord" | "both") && !ack.discord
        };
        if !selected {
            continue;
        }
        let photo = message
            .photo
            .as_deref()
            .filter(|_| message.event.photo_allowed(&current))
            .filter(|_| {
                !matches!(
                    message.event,
                    Event::FruitCatch { .. } | Event::FruitOutcome { .. }
                ) || message.fruit_crop == current.webhook.crop_fruit_screenshot
            });
        if telegram {
            transport.telegram(&current, &text, photo)?;
            ack.telegram = true;
        } else {
            transport.discord(&current, &text, photo)?;
            ack.discord = true;
        }
    }
    if ack.telegram || ack.discord {
        Ok(true)
    } else {
        Err("Choose Telegram, Discord or both in notification settings".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct FakeTransport {
        calls: parking_lot::Mutex<Vec<(&'static str, bool)>>,
        discord_fail_once: std::sync::atomic::AtomicBool,
        revoke_after_telegram: Option<Arc<RwLock<Settings>>>,
    }
    impl Transport for FakeTransport {
        fn telegram(&self, _: &Settings, _: &str, photo: Option<&[u8]>) -> Result<(), String> {
            self.calls.lock().push(("telegram", photo.is_some()));
            if let Some(settings) = &self.revoke_after_telegram {
                settings.write().webhook.enabled = false;
            }
            Ok(())
        }
        fn discord(&self, _: &Settings, _: &str, photo: Option<&[u8]>) -> Result<(), String> {
            self.calls.lock().push(("discord", photo.is_some()));
            if self.discord_fail_once.swap(false, Ordering::SeqCst) {
                return Err("synthetic offline provider".into());
            }
            Ok(())
        }
    }
    fn configured() -> Settings {
        let mut s = Settings::default();
        s.webhook.enabled = true;
        s.webhook.provider = "both".into();
        s
    }
    fn queue(
        settings: Arc<RwLock<Settings>>,
    ) -> (WebhookQueue, crossbeam_channel::Receiver<Message>) {
        let (sender, receiver) = bounded(16);
        (
            WebhookQueue {
                sender: Some(sender),
                settings: Some(settings),
                platform: Arc::new(RwLock::new(None)),
                health: Arc::new(RwLock::new(DeliveryHealth::default())),
                pending: Arc::new(AtomicUsize::new(0)),
            },
            receiver,
        )
    }
    fn revoke(s: &mut Settings, event: Event) {
        match event {
            Event::Progress => s.webhook.progress = false,
            Event::FruitCatch { .. } | Event::FruitOutcome { .. } => s.webhook.fruit_drop = false,
            Event::Spawn => s.webhook.spawn = false,
            Event::Purchase => s.webhook.purchase = false,
            Event::Recovery => s.webhook.recovery = false,
            Event::Disconnect | Event::Reconnected => s.webhook.disconnect_alert = false,
            Event::Bait => s.webhook.bait_alert = false,
            _ => s.webhook.enabled = false,
        }
    }
    #[test]
    fn every_event_toggle_is_checked_before_enqueue_and_delivery() {
        for event in [
            Event::Progress,
            Event::FruitCatch { high_tier: true },
            Event::FruitOutcome {
                pity_zero: true,
                high_tier: true,
            },
            Event::Spawn,
            Event::Purchase,
            Event::Recovery,
            Event::Disconnect,
            Event::Reconnected,
            Event::Bait,
            Event::General,
        ] {
            let settings = Arc::new(RwLock::new(configured()));
            let (queue, receiver) = queue(settings.clone());
            queue.send(event, "event", "test".into(), None);
            let queued = receiver.try_recv().unwrap();
            revoke(&mut settings.write(), event);
            let transport = FakeTransport::default();
            assert!(
                !deliver_pending_using(
                    || settings.read().clone(),
                    &queued,
                    &mut ProviderAcks::default(),
                    &transport
                )
                .unwrap(),
                "{event:?}"
            );
            assert!(transport.calls.lock().is_empty());
            queue.send(event, "event", "test".into(), None);
            assert!(receiver.try_recv().is_err(), "revoked producer {event:?}");
        }
    }
    #[test]
    fn strict_mode_uses_typed_pity_evidence_never_titles_or_fruit_rarity() {
        let settings = Arc::new(RwLock::new(configured()));
        settings.write().webhook.pity_zero_only = true;
        let (queue, receiver) = queue(settings.clone());
        queue.notify_message("Pity 0 — fake title", "not a fruit outcome");
        let mut drop = DropInfo {
            name: Some("Pika".into()),
            pity: Some("0/40".into()),
            text: "New Item Pika Legendary Pity 0/40".into(),
            is_legendary: true,
            custom_telegram_message: None,
        };
        queue.fruit_drop(&drop, Some(vec![1]));
        assert!(
            receiver.try_recv().is_err(),
            "strict mode must not send an early duplicate"
        );
        let zero = FruitCatchContext::new(&drop, "", true, true);
        queue.fruit_storage_outcome(&zero, &StorageOutcome::Unknown, Some(vec![1]));
        let message = receiver.try_recv().unwrap();
        assert!(message.body.contains("unconfirmed"));
        assert!(matches!(
            message.event,
            Event::FruitOutcome {
                pity_zero: true,
                ..
            }
        ));
        drop.pity = None;
        drop.text = "New Item Pika".into();
        let unknown = FruitCatchContext::new(&drop, "", true, true);
        queue.fruit_storage_outcome(&unknown, &StorageOutcome::Stored, None);
        queue.fruit_storage_outcome(&zero, &StorageOutcome::Cancelled, None);
        assert!(
            receiver.try_recv().is_err(),
            "legendary name and cancelled actions are not zero alerts"
        );
        settings.write().webhook.pity_zero_only = false;
        queue.send(Event::Progress, "progress", String::new(), None);
        let queued = receiver.try_recv().unwrap();
        settings.write().webhook.pity_zero_only = true;
        assert!(!deliver_pending_using(
            || settings.read().clone(),
            &queued,
            &mut ProviderAcks::default(),
            &FakeTransport::default()
        )
        .unwrap());
    }
    #[test]
    fn screenshots_respect_master_and_per_event_flags_when_queued_and_delivered() {
        for event in [
            Event::FruitCatch { high_tier: true },
            Event::FruitOutcome {
                pity_zero: true,
                high_tier: true,
            },
            Event::Spawn,
        ] {
            let settings = Arc::new(RwLock::new(configured()));
            settings.write().webhook.send_catch_screenshot = true;
            let (queue, receiver) = queue(settings.clone());
            queue.send(event, "photo", String::new(), Some(vec![1]));
            let queued = receiver.try_recv().unwrap();
            assert!(queued.photo.is_some());
            match event {
                Event::FruitCatch { .. } => settings.write().webhook.send_catch_screenshot = false,
                Event::FruitOutcome { .. } => settings.write().webhook.send_drop_screenshot = false,
                _ => settings.write().webhook.send_screenshot = false,
            }
            let transport = FakeTransport::default();
            assert!(deliver_pending_using(
                || settings.read().clone(),
                &queued,
                &mut ProviderAcks::default(),
                &transport
            )
            .unwrap());
            assert_eq!(
                *transport.calls.lock(),
                vec![("telegram", false), ("discord", false)]
            );
            queue.send(event, "photo", String::new(), Some(vec![1]));
            assert!(receiver.try_recv().unwrap().photo.is_none());
        }
    }
    #[test]
    fn changing_fruit_crop_does_not_deliver_an_old_full_screen_photo() {
        let settings = Arc::new(RwLock::new(configured()));
        settings.write().webhook.crop_fruit_screenshot = false;
        let (queue, receiver) = queue(settings.clone());
        queue.send(
            Event::FruitOutcome {
                pity_zero: true,
                high_tier: true,
            },
            "fruit",
            String::new(),
            Some(vec![1]),
        );
        let queued = receiver.try_recv().unwrap();
        settings.write().webhook.crop_fruit_screenshot = true;
        let transport = FakeTransport::default();
        assert!(deliver_pending_using(
            || settings.read().clone(),
            &queued,
            &mut ProviderAcks::default(),
            &transport
        )
        .unwrap());
        assert_eq!(
            *transport.calls.lock(),
            vec![("telegram", false), ("discord", false)]
        );
    }

    #[test]
    fn master_screenshot_off_blocks_every_photo_even_if_event_flag_is_on() {
        let mut s = configured();
        s.webhook.send_screenshot = false;
        s.webhook.send_catch_screenshot = true;
        for event in [
            Event::FruitCatch { high_tier: true },
            Event::FruitOutcome {
                pity_zero: true,
                high_tier: true,
            },
            Event::Spawn,
            Event::Disconnect,
            Event::Reconnected,
        ] {
            assert!(!event.photo_allowed(&s));
        }
    }
    #[test]
    fn connection_toggle_and_fruit_toggle_apply_to_public_producers() {
        let settings = Arc::new(RwLock::new(configured()));
        settings.write().webhook.disconnect_alert = false;
        settings.write().webhook.fruit_drop = false;
        let (queue, receiver) = queue(settings);
        queue.reconnected(Some(vec![1]));
        queue.disconnect("offline", Some(vec![1]));
        queue.fruit_drop(
            &DropInfo {
                name: Some("Pika".into()),
                pity: Some("0/40".into()),
                text: String::new(),
                is_legendary: true,
                custom_telegram_message: None,
            },
            Some(vec![1]),
        );
        assert!(receiver.try_recv().is_err());
    }
    #[test]
    fn rarity_toggle_is_rechecked_for_already_queued_catches() {
        let settings = Arc::new(RwLock::new(configured()));
        settings.write().webhook.legendary_only = false;
        let (queue, receiver) = queue(settings.clone());
        queue.send(
            Event::FruitOutcome {
                pity_zero: false,
                high_tier: false,
            },
            "fruit",
            String::new(),
            None,
        );
        let queued = receiver.try_recv().unwrap();
        settings.write().webhook.legendary_only = true;
        assert!(!deliver_pending_using(
            || settings.read().clone(),
            &queued,
            &mut ProviderAcks::default(),
            &FakeTransport::default()
        )
        .unwrap());
    }
    #[test]
    fn retry_preserves_provider_ack_and_rechecks_revoked_consent() {
        let settings = Arc::new(RwLock::new(configured()));
        let message = Message {
            event: Event::Progress,
            title: "progress".into(),
            body: String::new(),
            photo: None,
            fruit_crop: false,
        };
        let transport = FakeTransport {
            discord_fail_once: std::sync::atomic::AtomicBool::new(true),
            ..FakeTransport::default()
        };
        let mut ack = ProviderAcks::default();
        assert!(
            deliver_pending_using(|| settings.read().clone(), &message, &mut ack, &transport)
                .is_err()
        );
        assert!(
            deliver_pending_using(|| settings.read().clone(), &message, &mut ack, &transport)
                .unwrap()
        );
        assert_eq!(
            *transport.calls.lock(),
            vec![("telegram", false), ("discord", false), ("discord", false)]
        );
        let transport = FakeTransport {
            revoke_after_telegram: Some(settings.clone()),
            ..FakeTransport::default()
        };
        assert!(deliver_pending_using(
            || settings.read().clone(),
            &message,
            &mut ProviderAcks::default(),
            &transport
        )
        .unwrap());
        assert_eq!(*transport.calls.lock(), vec![("telegram", false)]);
        assert!(!deliver_pending_using(
            || settings.read().clone(),
            &message,
            &mut ProviderAcks::default(),
            &transport
        )
        .unwrap());
    }
    #[test]
    fn explicit_test_is_allowed_in_strict_mode_but_requires_master_enable() {
        let mut settings = configured();
        settings.webhook.pity_zero_only = true;
        let message = Message {
            event: Event::Test,
            title: "test".into(),
            body: String::new(),
            photo: None,
            fruit_crop: false,
        };
        assert!(deliver_pending_using(
            || settings.clone(),
            &message,
            &mut ProviderAcks::default(),
            &FakeTransport::default()
        )
        .unwrap());
        settings.webhook.enabled = false;
        assert!(deliver_pending_using(
            || settings.clone(),
            &message,
            &mut ProviderAcks::default(),
            &FakeTransport::default()
        )
        .is_err());
    }
    #[test]
    fn unrelated_zero_is_not_pity() {
        assert_eq!(parse_pity("HP 0/100 Backpack 0/40", 40), None);
        assert_eq!(parse_pity("Legendary Pity 0/40", 40), Some("0/40".into()));
        assert_eq!(parse_pity("Pity 28/40", 40), None);
        assert_eq!(
            parse_pity("Fruit Pity 0/40 Legendary Pity 17/40", 40),
            Some("17/40".into())
        );
        assert_eq!(
            parse_pity("Legendary Pity 17/40 Fruit Pity 0/40", 40),
            Some("17/40".into())
        );
        assert_eq!(
            parse_pity("Legendary Pity 0/40 Legendary Pity 17/40", 40),
            None
        );
        assert_eq!(parse_pity("Fruit Pity 0/40", 40), None);
        assert_eq!(parse_pity("Legendary Pity O/4O", 40), Some("0/40".into()));
    }
    #[test]
    fn message_limits_count_unicode_units() {
        assert!(plain(&"🎣".repeat(2000), 3500).encode_utf16().count() <= 3500);
    }
}
