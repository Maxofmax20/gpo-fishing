use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use parking_lot::RwLock;
use serde_json::json;
use crate::bot::Bot;
use crate::config::Settings;

use std::sync::atomic::{AtomicBool, Ordering};

static LAST_FRAME: parking_lot::RwLock<Option<Vec<u8>>> = parking_lot::RwLock::new(None);
static HELD_KEYS: parking_lot::RwLock<Vec<crate::core::types::Key>> = parking_lot::RwLock::new(Vec::new());
static LAST_KEY_ACTIVITY: parking_lot::RwLock<Option<std::time::Instant>> = parking_lot::RwLock::new(None);
static KEY_WATCHDOG_STARTED: AtomicBool = AtomicBool::new(false);

fn ensure_key_watchdog(bot: &Arc<Bot>) {
    if KEY_WATCHDOG_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let bot_clone = Arc::clone(bot);
    thread::Builder::new()
        .name("key-watchdog".into())
        .spawn(move || loop {
            thread::sleep(Duration::from_millis(500));
            let should_release = {
                let keys = HELD_KEYS.read();
                if keys.is_empty() {
                    false
                } else if let Some(last) = *LAST_KEY_ACTIVITY.read() {
                    last.elapsed() >= Duration::from_secs(30)
                } else {
                    false
                }
            };

            if should_release {
                let mut keys = HELD_KEYS.write();
                for &k in keys.iter() {
                    bot_clone.ctx().platform.input.key(k, false);
                }
                keys.clear();
                tracing::info!("Auto-released held keys after 30s inactivity");
            }
        })
        .expect("spawn key-watchdog thread");
}

pub fn get_local_ip() -> Option<String> {
    #[cfg(windows)]
    {
        use std::process::Command;
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "(Get-NetIPAddress -AddressFamily IPv4 | Where-Object { ($_.InterfaceAlias -match 'Wi-Fi|Ethernet') -and ($_.InterfaceAlias -notmatch 'vEthernet|WARP|Loopback') -and ($_.IPAddress -notmatch '^169\\.254\\.') } | Select-Object -ExpandProperty IPAddress -First 1)"
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();

        if let Ok(out) = output {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() && s.contains('.') {
                return Some(s);
            }
        }
    }

    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let ip = socket.local_addr().ok()?.ip().to_string();
    if ip.starts_with("172.16.") || ip.starts_with("172.17.") || ip.starts_with("169.254.") {
        return Some("192.168.1.3".into());
    }
    Some(ip)
}

pub fn spawn(bot: Arc<Bot>, settings: Arc<RwLock<Settings>>) {
    thread::Builder::new()
        .name("web-server".into())
        .spawn(move || {
            let port = 3888;
            // Loopback-only by default. LAN exposure is an explicit opt-in
            // (`web.allow_lan`) because every POST endpoint drives input.
            // NOTE: toggling `allow_lan` takes effect on next app start.
            let allow_lan = settings.read().web.allow_lan;
            let bind_addr = if allow_lan { format!("0.0.0.0:{port}") } else { format!("127.0.0.1:{port}") };
            let listener = match TcpListener::bind(&bind_addr) {
                Ok(l) => {
                    if allow_lan {
                        tracing::info!("Web Dashboard running at http://0.0.0.0:{port} (LAN enabled, token required)");
                    } else {
                        tracing::info!("Web Dashboard running at http://127.0.0.1:{port} (loopback only)");
                    }
                    l
                }
                Err(e) => {
                    tracing::warn!("Failed to bind web server on {bind_addr}: {e}");
                    return;
                }
            };

            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let bot = Arc::clone(&bot);
                let settings = Arc::clone(&settings);
                thread::spawn(move || {
                    handle_client(stream, &bot, &settings);
                });
            }
        })
        .expect("spawn web-server thread");
}

/// Extract a header value (case-insensitive name) from a raw HTTP request.
fn header_value(req: &str, name: &str) -> Option<String> {
    let prefix = format!("{}:", name.to_ascii_lowercase());
    req.lines().skip(1).find_map(|l| {
        let t = l.trim();
        if t.len() > prefix.len() && t[..prefix.len()].to_ascii_lowercase() == prefix {
            Some(t[prefix.len()..].trim().to_string())
        } else {
            None
        }
    })
}

/// Extract `token=<hex>` from a URL query string (no decoding needed: hex).
fn query_token(query: &str) -> Option<String> {
    query.split('&').find_map(|p| p.strip_prefix("token=").map(|v| v.to_string()))
}

/// True when either credential matches the per-install dashboard token.
/// Fail-closed: an empty expected token never authorizes. Comparison is
/// constant-time so response latency cannot oracle the token byte-by-byte.
fn is_authorized(
    settings: &Arc<RwLock<Settings>>,
    bearer: Option<String>,
    qtoken: Option<String>,
) -> bool {
    use crate::core::secrets::tokens_equal;
    let expected = settings.read().web.token.clone();
    if expected.is_empty() {
        return false;
    }
    bearer.as_deref().is_some_and(|b| tokens_equal(b, &expected))
        || qtoken.as_deref().is_some_and(|q| tokens_equal(q, &expected))
}

/// Restricted CORS: reflect the Origin only for loopback origins, plus
/// private-LAN origins when LAN mode is on. Everything else gets no
/// `Access-Control-Allow-Origin` (same-origin dashboard needs none).
fn cors_origin_for(req: &str, allow_lan: bool) -> Option<String> {
    let origin = header_value(req, "origin")?;
    let low = origin.to_ascii_lowercase();
    if low.starts_with("http://localhost") || low.starts_with("http://127.0.0.1") {
        return Some(origin);
    }
    if allow_lan
        && (low.starts_with("http://192.168.")
            || low.starts_with("http://10.")
            || low.starts_with("http://172.16.")
            || low.starts_with("http://172.17.")
            || low.starts_with("http://172.18"))
    {
        return Some(origin);
    }
    None
}

fn send_unauthorized(stream: &mut TcpStream) {
    let msg = "Unauthorized: missing or invalid dashboard token. Open the dashboard from the app Panel so the token is attached.";
    let resp = format!(
        "HTTP/1.1 401 Unauthorized\r\nContent-Type: text/plain\r\nWWW-Authenticate: Bearer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        msg.len(),
        msg
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn send_forbidden(stream: &mut TcpStream) {
    let msg = "Forbidden: LAN access is disabled. Enable it in Settings (Web dashboard) if you need remote access.";
    let resp = format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        msg.len(),
        msg
    );
    let _ = stream.write_all(resp.as_bytes());
}

/// Read one full HTTP request: loop until headers complete, then read
/// exactly Content-Length body bytes. A single `read()` races TCP
/// segmentation (headers and body often arrive separately), which used to
/// truncate POST bodies into "unknown action" failures.
fn read_full_request(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.len() > 65536 {
                    return None;
                }
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return None,
        }
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    let split = text.find("\r\n\r\n")?;
    let (head, mut body) = (text[..split].to_string(), text[split + 4..].as_bytes().to_vec());
    let want = content_length(&head).unwrap_or(0).min(1 << 20);
    while body.len() < want {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
        if body.len() > 1 << 20 {
            break;
        }
    }
    body.truncate(want);
    Some(format!("{head}\r\n\r\n{}", String::from_utf8_lossy(&body)))
}

fn content_length(head: &str) -> Option<usize> {
    head.lines().skip(1).find_map(|l| {
        let t = l.trim();
        if t.len() > 15 && t[..15].eq_ignore_ascii_case("content-length:") {
            t[15..].trim().parse::<usize>().ok()
        } else {
            None
        }
    })
}

fn handle_client(mut stream: TcpStream, bot: &Arc<Bot>, settings: &Arc<RwLock<Settings>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    let req_str = match read_full_request(&mut stream) {
        Some(s) if !s.is_empty() => s,
        _ => return,
    };
    let first_line = req_str.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }

    let method = parts[0];
    let path = parts[1];

    // Request origin for restricted CORS (same-origin dashboard needs none).
    let allow_lan = settings.read().web.allow_lan;
    let origin_allow = cors_origin_for(&req_str, allow_lan);

    if method == "OPTIONS" {
        let allow = origin_allow
            .map(|o| format!("Access-Control-Allow-Origin: {o}\r\n"))
            .unwrap_or_default();
        let resp = format!("HTTP/1.1 204 No Content\r\n{allow}Access-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nVary: Origin\r\n\r\n");
        let _ = stream.write_all(resp.as_bytes());
        return;
    }

    let raw_path = path.split('?').next().unwrap_or(path);
    let query = path.split('?').nth(1).unwrap_or("");

    let from_loopback = stream.peer_addr().map(|a| a.ip().is_loopback()).unwrap_or(false);
    if !allow_lan && !from_loopback {
        send_forbidden(&mut stream);
        return;
    }

    // Authentication: per-install token via `Authorization: Bearer` (preferred)
    // or `?token=` (required for <img> stream/screenshot URLs).
    let authed = {
        let bearer = header_value(&req_str, "authorization").and_then(|h| {
            h.strip_prefix("Bearer ")
                .map(|v| v.trim().to_string())
                .or_else(|| h.strip_prefix("bearer ").map(|v| v.trim().to_string()))
        });
        let qtoken = query_token(query);
        is_authorized(settings, bearer, qtoken)
    };
    // Never log the raw path: it may contain `?token=`.
    let safe_path = crate::core::secrets::strip_token_from_uri(path);

    // Public shell + gated API: the HTML shell carries no secrets and the
    // embedded JS attaches the token (stored from the Panel-provided
    // `?token=` on first visit). Everything that reads or drives the PC
    // requires the token — including on loopback, so a leaked token is
    // still needed for abuse.
    let needs_auth = raw_path != "/"
        && raw_path != "/index.html"
        && raw_path != "/stats"
        && raw_path != "/studio"
        && raw_path != "/craft"
        && raw_path != "/system"
        && !raw_path.starts_with("/assets/");
    if needs_auth && !authed {
        tracing::warn!("web dashboard: unauthorized {} {}", method, safe_path);
        send_unauthorized(&mut stream);
        return;
    }

    if raw_path == "/" || raw_path == "/index.html" {
        send_html_page(&mut stream, WEB_REMOTE_HTML);
    } else if raw_path == "/stats" {
        send_html_page(&mut stream, WEB_STATS_HTML);
    } else if raw_path == "/studio" {
        send_html_page(&mut stream, WEB_STUDIO_HTML);
    } else if raw_path == "/craft" {
        send_html_page(&mut stream, WEB_CRAFT_HTML);
    } else if raw_path == "/system" {
        send_html_page(&mut stream, WEB_SYSTEM_HTML);
    } else if serve_asset(&mut stream, raw_path) {
        // handled
    } else if raw_path == "/api/status" {
        send_status(&mut stream, bot, settings);
    } else if raw_path == "/api/stream" {
        let _ = stream.set_read_timeout(None);
        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
        stream_mjpeg(stream, bot, query);
    } else if raw_path == "/api/screenshot" {
        send_screenshot(&mut stream, bot, query);
    } else if raw_path == "/api/click" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
        handle_click(&mut stream, bot, body);
    } else if raw_path == "/api/drag" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
        handle_drag(&mut stream, bot, body);
    } else if raw_path == "/api/mouse" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
        handle_mouse(&mut stream, bot, body);
    } else if raw_path == "/api/key" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
        handle_key(&mut stream, bot, body);
    } else if raw_path == "/api/action" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
        handle_action(&mut stream, bot, settings, body);
    } else if raw_path == "/api/craft" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
        handle_craft(&mut stream, bot, body);
    } else if raw_path == "/api/macro/record" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
        handle_macro_record(&mut stream, bot, body);
    } else if raw_path == "/api/macro/play" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
        handle_macro_play(&mut stream, bot, body);
    } else if raw_path == "/api/macro/list" {
        handle_macro_list(&mut stream, bot);
    } else if raw_path == "/api/macro/rename" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
        handle_macro_rename(&mut stream, bot, body);
    } else if raw_path == "/api/macro/delete" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
        handle_macro_delete(&mut stream, bot, body);
    } else if raw_path == "/api/keyboard/light" {
        if method == "POST" {
            let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
            handle_keyboard_light(&mut stream, body);
        } else {
            send_keyboard_light_status(&mut stream);
        }
    } else if raw_path == "/api/fan/status" || raw_path == "/api/fan" {
        if method == "POST" {
            let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
            handle_fan_set(&mut stream, body);
        } else {
            send_fan_status(&mut stream);
        }
    } else if raw_path == "/api/fan/set" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
        handle_fan_set(&mut stream, body);
    } else {
        let not_found = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(not_found.as_bytes());
    }
}

fn stream_mjpeg(mut stream: TcpStream, bot: &Arc<Bot>, query: &str) {
    let mut fps: u32 = 8;
    let mut scale: usize = 720;
    let mut quality: u8 = 70;

    for param in query.split('&') {
        let mut kv = param.split('=');
        if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
            match k {
                // Bounded to prevent abusive CPU/LAN usage (DoD: fps<=10).
                "fps" => {
                    if let Ok(n) = v.parse::<u32>() {
                        fps = n.clamp(1, 10);
                    }
                }
                "scale" => {
                    if let Ok(n) = v.parse::<usize>() {
                        scale = n.clamp(240, 1080);
                    }
                }
                "q" | "quality" => {
                    if let Ok(n) = v.parse::<u8>() {
                        quality = n.clamp(30, 85);
                    }
                }
                _ => {}
            }
        }
    }

    let sleep_dur = Duration::from_millis((1000 / fps).max(15) as u64);

    let header = "HTTP/1.1 200 OK\r\n\
                  Content-Type: multipart/x-mixed-replace; boundary=frame\r\n\
                  Cache-Control: no-cache, no-store, must-revalidate\r\n\
                  Connection: close\r\n\r\n";
    if stream.write_all(header.as_bytes()).is_err() {
        return;
    }

    loop {
        let frame_opt = bot.ctx().roblox_rect()
            .and_then(|r| bot.ctx().platform.capture.grab(r).ok())
            .map(|f| if scale > 0 { f.downscale(scale) } else { f })
            .and_then(|f| f.to_jpeg_bytes(quality).ok());

        let bytes = match frame_opt {
            Some(b) => {
                *LAST_FRAME.write() = Some(b.clone());
                b
            }
            None => {
                match LAST_FRAME.read().as_ref() {
                    Some(b) => b.clone(),
                    None => {
                        std::thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                }
            }
        };

        let part_header = format!(
            "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
            bytes.len()
        );
        if stream.write_all(part_header.as_bytes()).is_err() {
            break;
        }
        if stream.write_all(&bytes).is_err() {
            break;
        }
        if stream.write_all(b"\r\n").is_err() {
            break;
        }

        std::thread::sleep(sleep_dur);
    }
}

fn send_status(stream: &mut TcpStream, bot: &Arc<Bot>, settings: &Arc<RwLock<Settings>>) {
    let state = bot.state();
    let paused = bot.is_paused();
    let stats = bot.ctx().session.lock().stats();
    let vol = (crate::core::audio::get_volume().unwrap_or(0.5) * 100.0).round() as u32;
    let muted = crate::core::audio::is_muted().unwrap_or(false);
    let brightness = crate::core::brightness::get_brightness().unwrap_or(80);
    let local_ip = get_local_ip().unwrap_or_else(|| "127.0.0.1".into());

    let state_str = if paused {
        "Paused"
    } else {
        match state {
            crate::events::BotState::Stopped => "Stopped",
            crate::events::BotState::WaitingForRoblox => "Waiting For Roblox",
            crate::events::BotState::Tracking => "Fishing (Tracking)",
            crate::events::BotState::WaitingForBite => "Waiting For Bite",
            crate::events::BotState::Casting => "Casting Rod",
            crate::events::BotState::Purchasing => "Buying Bait",
            crate::events::BotState::StoringFruit => "Storing Fruit",
            crate::events::BotState::Recovering => "Recovering",
            _ => "Active",
        }
    };

    let now = crate::core::boss_tracker::now_sec();
    let mut tracker = crate::core::boss_tracker::BossTracker::default();
    let s = settings.read();
    if let Some(off) = s.boss_tracker.hawkeye_offset {
        tracker.set_offset(crate::core::boss_tracker::BossId::HawkEye, off);
    }
    if let Some(off) = s.boss_tracker.roger_offset {
        tracker.set_offset(crate::core::boss_tracker::BossId::Roger, off);
    }
    if let Some(off) = s.boss_tracker.soulking_offset {
        tracker.set_offset(crate::core::boss_tracker::BossId::SoulKing, off);
    }
    if let Some(off) = s.boss_tracker.radiant_admiral_offset {
        tracker.set_offset(crate::core::boss_tracker::BossId::RadiantAdmiral, off);
    }
    if let Some(off) = s.boss_tracker.merchant_offset {
        tracker.set_offset(crate::core::boss_tracker::BossId::TravellingMerchant, off);
    }

    let mut bosses_data = Vec::new();
    for &boss in crate::core::boss_tracker::BossId::all() {
        let rem = tracker.remaining_seconds(boss, now);
        bosses_data.push(json!({
            "name": boss.name(),
            "emoji": boss.emoji(),
            "location": boss.location(),
            "target_spawn": now + rem,
            "next_spawn_s": rem,
            "next_spawn_fmt": crate::core::boss_tracker::format_duration(rem),
            "is_spawned": rem <= 0,
            "is_soon": rem > 0 && rem <= 300,
        }));
    }

    let payload = json!({
        "state": state_str,
        "paused": paused,
        "is_running": bot.is_running(),
        "fish": stats.fish,
        "fruits": stats.fruits,
        "pity_fruit": stats.pity_fruit,
        "pity_legendary": stats.pity_legendary,
        "success_rate": (stats.success_rate * 100.0).round(),
        "runtime_s": stats.runtime_s,
        "bait_purchased": stats.bait_purchased,
        "since_purchase": stats.since_purchase,
        "every_n_catches": s.purchase.every_n_catches,
        "bait_tier": format!("{:?}", s.purchase.bait_tier),
        "legendary_reserve": s.purchase.legendary_reserve,
        "auto_purchase": s.features.auto_purchase,
        "spawn_alerts": s.webhook.spawn,
        "server_time": now,
        "last_fruit": stats.last_fruit,
        "last_spawn": stats.last_spawn,
        "volume": vol,
        "muted": muted,
        "brightness": brightness,
        "local_ip": local_ip,
        "version": env!("CARGO_PKG_VERSION"),
        "bosses": bosses_data,
        "crafting": crate::bot::crafting::get_craft_status(),
        "recorder": crate::bot::recorder::get_status(),
        "macros": crate::bot::recorder::load_macros(&bot.ctx().store),
        "keyboard_light": crate::laptop_light::get_keyboard_light_status(),
        "fan": crate::laptop_fan::get_fan_status(),
        "auto_reconnect": s.features.auto_reconnect,
        "vip_server_url": s.features.vip_server_url.clone(),
        "private_server_code": s.features.private_server_code.clone(),
        "rejoin_macro_name": s.features.rejoin_macro_name.clone(),
        "legendary_only": s.webhook.legendary_only,
        "send_drop_screenshot": s.webhook.send_drop_screenshot,
        "recent_catches": recent_catches(&bot.ctx().store),
    });

    let body = payload.to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
}

/// Last catch log rows for the telemetry loot feed (newest first).
/// Tolerant mini-CSV reader: `Timestamp,Type,Name,RawText` with optional
/// quoting. Never fails the status call — any read/parse problem yields [].
fn recent_catches(store: &crate::config::Store) -> serde_json::Value {
    let raw = std::fs::read_to_string(store.catches_path()).unwrap_or_default();
    let mut rows: Vec<(String, String, String, String)> = Vec::new();
    for line in raw.lines().rev() {
        if rows.len() >= 6 {
            break;
        }
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with("Timestamp,") {
            continue;
        }
        let fields = split_csv_line(line);
        if fields.len() < 3 {
            continue;
        }
        rows.push((
            fields[0].clone(),
            fields[1].clone(),
            fields[2].clone(),
            fields.get(3).cloned().unwrap_or_default(),
        ));
    }
    let arr: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(timestamp, kind, name, raw_text)| {
            serde_json::json!({
                "timestamp": timestamp,
                "type": kind,
                "name": if name.trim().is_empty() { "Unknown" } else { name.as_str() },
                "raw": raw_text,
            })
        })
        .collect();
    serde_json::Value::Array(arr)
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            fields.push(cur.trim().to_string());
            cur = String::new();
        } else {
            cur.push(c);
        }
    }
    fields.push(cur.trim().to_string());
    fields
}

fn send_screenshot(stream: &mut TcpStream, bot: &Arc<Bot>, query: &str) {
    let mut scale: usize = 760;
    let mut quality: u8 = 75;

    for param in query.split('&') {
        let mut kv = param.split('=');
        if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
            match k {
                "scale" => {
                    if let Ok(n) = v.parse::<usize>() {
                        scale = n.clamp(240, 1080);
                    }
                }
                "q" | "quality" => {
                    if let Ok(n) = v.parse::<u8>() {
                        quality = n.clamp(30, 85);
                    }
                }
                _ => {}
            }
        }
    }

    let fresh_bytes = bot
        .ctx()
        .roblox_rect()
        .and_then(|r| bot.ctx().platform.capture.grab(r).ok())
        .map(|f| if scale > 0 { f.downscale(scale) } else { f })
        .and_then(|f| f.to_jpeg_bytes(quality).ok());

    if let Some(bytes) = fresh_bytes {
        *LAST_FRAME.write() = Some(bytes.clone());
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nCache-Control: no-cache, no-store, must-revalidate\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&bytes);
        return;
    }

    if let Some(cached) = LAST_FRAME.read().as_ref() {
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nCache-Control: no-cache, no-store, must-revalidate\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            cached.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(cached);
        return;
    }

    let msg = "Waiting for Roblox window...";
    let resp = format!(
        "HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        msg.len(),
        msg
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_click(stream: &mut TcpStream, bot: &Arc<Bot>, body: &str) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let rx = parsed.get("rx")
        .or_else(|| parsed.get("rel_x"))
        .or_else(|| parsed.get("x"))
        .and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
    let ry = parsed.get("ry")
        .or_else(|| parsed.get("rel_y"))
        .or_else(|| parsed.get("y"))
        .and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
    let btn_str = parsed.get("button").and_then(|v| v.as_str()).unwrap_or("left");

    // Record step if macro recorder is active
    crate::bot::recorder::record_click(rx, ry, btn_str);

    // Ensure Roblox has window focus before sending click
    let _ = bot.ctx().platform.window.focus();
    std::thread::sleep(Duration::from_millis(20));

    if let Some(rect) = bot.ctx().roblox_rect() {
        let px = rect.x + (rx.clamp(0.0, 1.0) * rect.w as f32).round() as i32;
        let py = rect.y + (ry.clamp(0.0, 1.0) * rect.h as f32).round() as i32;
        let pt = crate::core::types::PxPoint { x: px, y: py };

        bot.ctx().platform.input.move_to(pt);
        std::thread::sleep(Duration::from_millis(25));
        let btn = if btn_str == "right" {
            crate::core::types::MouseButton::Right
        } else {
            crate::core::types::MouseButton::Left
        };
        bot.ctx().platform.input.button(btn, true);
        std::thread::sleep(Duration::from_millis(50));
        bot.ctx().platform.input.button(btn, false);
    }

    let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"ok\":true}";
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_drag(stream: &mut TcpStream, bot: &Arc<Bot>, body: &str) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let start_rx = parsed.get("start_x").or_else(|| parsed.get("start_rx")).and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
    let start_ry = parsed.get("start_y").or_else(|| parsed.get("start_ry")).and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
    let end_rx = parsed.get("end_x").or_else(|| parsed.get("end_rx")).and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
    let end_ry = parsed.get("end_y").or_else(|| parsed.get("end_ry")).and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
    let duration_ms = parsed.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(200);
    let btn_str = parsed.get("button").and_then(|v| v.as_str()).unwrap_or("left");
    let record_only = parsed.get("record_only").and_then(|v| v.as_bool()).unwrap_or(false);

    let btn = if btn_str == "right" {
        crate::core::types::MouseButton::Right
    } else {
        crate::core::types::MouseButton::Left
    };

    // Record step if macro recorder is active
    crate::bot::recorder::record_drag(start_rx, start_ry, end_rx, end_ry, duration_ms);

    if record_only {
        let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"ok\":true}";
        let _ = stream.write_all(resp.as_bytes());
        return;
    }

    // Execute drag in game
    let _ = bot.ctx().platform.window.focus();
    std::thread::sleep(Duration::from_millis(15));

    if let Some(rect) = bot.ctx().roblox_rect() {
        let start_px = rect.x + (start_rx.clamp(0.0, 1.0) * rect.w as f32).round() as i32;
        let start_py = rect.y + (start_ry.clamp(0.0, 1.0) * rect.h as f32).round() as i32;
        let end_px = rect.x + (end_rx.clamp(0.0, 1.0) * rect.w as f32).round() as i32;
        let end_py = rect.y + (end_ry.clamp(0.0, 1.0) * rect.h as f32).round() as i32;

        bot.ctx().platform.input.move_to(crate::core::types::PxPoint { x: start_px, y: start_py });
        std::thread::sleep(Duration::from_millis(20));
        bot.ctx().platform.input.button(btn, true);
        std::thread::sleep(Duration::from_millis(25));

        let num_steps = ((duration_ms as f32 / 12.0).round() as i32).clamp(6, 25);
        let step_delay = (duration_ms / num_steps as u64).max(8);
        for i in 1..=num_steps {
            let t = i as f32 / num_steps as f32;
            let ease = t * t * (3.0 - 2.0 * t);
            let cur_x = (start_px as f32 + (end_px - start_px) as f32 * ease).round() as i32;
            let cur_y = (start_py as f32 + (end_py - start_py) as f32 * ease).round() as i32;
            bot.ctx().platform.input.move_to(crate::core::types::PxPoint { x: cur_x, y: cur_y });
            std::thread::sleep(Duration::from_millis(step_delay));
        }

        bot.ctx().platform.input.move_to(crate::core::types::PxPoint { x: end_px, y: end_py });
        std::thread::sleep(Duration::from_millis(25));
        bot.ctx().platform.input.button(btn, false);
    }

    let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"ok\":true}";
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_mouse(stream: &mut TcpStream, bot: &Arc<Bot>, body: &str) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let act = parsed.get("action").and_then(|v| v.as_str()).unwrap_or("click");
    let btn_str = parsed.get("button").and_then(|v| v.as_str()).unwrap_or("left");
    let rx = parsed.get("rx")
        .or_else(|| parsed.get("rel_x"))
        .or_else(|| parsed.get("x"))
        .and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
    let ry = parsed.get("ry")
        .or_else(|| parsed.get("rel_y"))
        .or_else(|| parsed.get("y"))
        .and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;

    let btn = if btn_str == "right" {
        crate::core::types::MouseButton::Right
    } else {
        crate::core::types::MouseButton::Left
    };

    if act == "release_all" || act == "reset" {
        bot.ctx().platform.input.button(crate::core::types::MouseButton::Left, false);
        bot.ctx().platform.input.button(crate::core::types::MouseButton::Right, false);
        let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"ok\":true}";
        let _ = stream.write_all(resp.as_bytes());
        return;
    }

    let _ = bot.ctx().platform.window.focus();

    if let Some(rect) = bot.ctx().roblox_rect() {
        let px = rect.x + (rx.clamp(0.0, 1.0) * rect.w as f32).round() as i32;
        let py = rect.y + (ry.clamp(0.0, 1.0) * rect.h as f32).round() as i32;
        let pt = crate::core::types::PxPoint { x: px, y: py };

        match act {
            "down" => {
                bot.ctx().platform.input.move_to(pt);
                std::thread::sleep(Duration::from_millis(10));
                bot.ctx().platform.input.button(btn, true);
            }
            "move" => {
                bot.ctx().platform.input.move_to(pt);
            }
            "up" => {
                bot.ctx().platform.input.move_to(pt);
                std::thread::sleep(Duration::from_millis(10));
                bot.ctx().platform.input.button(btn, false);
            }
            _ => {}
        }
    }

    let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"ok\":true}";
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_key(stream: &mut TcpStream, bot: &Arc<Bot>, body: &str) {
    ensure_key_watchdog(bot);

    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let key_str = parsed.get("key").and_then(|v| v.as_str()).unwrap_or("");
    let is_down = parsed.get("down").and_then(|v| v.as_bool()).unwrap_or(true);
    let tap = parsed.get("tap").and_then(|v| v.as_bool()).unwrap_or(false);
    let is_heartbeat = parsed.get("heartbeat").and_then(|v| v.as_bool()).unwrap_or(false);

    *LAST_KEY_ACTIVITY.write() = Some(std::time::Instant::now());

    if is_heartbeat || key_str == "heartbeat" {
        // Just refresh the activity timestamp, keys remain held!
        let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"ok\":true}";
        let _ = stream.write_all(resp.as_bytes());
        return;
    }

    // Record step if macro recorder is active and key is tapped/pressed
    if is_down && key_str != "release_all" && !key_str.is_empty() {
        crate::bot::recorder::record_key(key_str);
    }

    // Ensure Roblox has window focus before sending keystroke
    let _ = bot.ctx().platform.window.focus();

    if key_str == "release_all" {
        let mut held = HELD_KEYS.write();
        for &k in held.iter() {
            bot.ctx().platform.input.key(k, false);
        }
        held.clear();
        for k in [
            crate::core::types::Key::Char('w'),
            crate::core::types::Key::Char('a'),
            crate::core::types::Key::Char('s'),
            crate::core::types::Key::Char('d'),
            crate::core::types::Key::Char('t'),
            crate::core::types::Key::Char(' '),
            crate::core::types::Key::Shift,
            crate::core::types::Key::Left,
            crate::core::types::Key::Right,
            crate::core::types::Key::Up,
            crate::core::types::Key::Down,
        ] {
            bot.ctx().platform.input.key(k, false);
        }
    } else {
        let k_opt = match key_str.to_lowercase().as_str() {
            "w" => Some(crate::core::types::Key::Char('w')),
            "a" => Some(crate::core::types::Key::Char('a')),
            "s" => Some(crate::core::types::Key::Char('s')),
            "d" => Some(crate::core::types::Key::Char('d')),
            "t" => Some(crate::core::types::Key::Char('t')),
            "space" | "jump" => Some(crate::core::types::Key::Char(' ')),
            "shift" => Some(crate::core::types::Key::Shift),
            "e" | "interact" => Some(crate::core::types::Key::Char('e')),
            "1" => Some(crate::core::types::Key::Char('1')),
            "2" => Some(crate::core::types::Key::Char('2')),
            "3" => Some(crate::core::types::Key::Char('3')),
            "4" => Some(crate::core::types::Key::Char('4')),
            "5" => Some(crate::core::types::Key::Char('5')),
            "left" | "arrowleft" => Some(crate::core::types::Key::Left),
            "right" | "arrowright" => Some(crate::core::types::Key::Right),
            "up" | "arrowup" => Some(crate::core::types::Key::Up),
            "down" | "arrowdown" => Some(crate::core::types::Key::Down),
            _ => None,
        };

        if let Some(k) = k_opt {
            if tap {
                bot.ctx().platform.input.key(k, true);
                std::thread::sleep(Duration::from_millis(80));
                bot.ctx().platform.input.key(k, false);
                HELD_KEYS.write().retain(|&x| x != k);
            } else if is_down {
                bot.ctx().platform.input.key(k, true);
                let mut held = HELD_KEYS.write();
                if !held.contains(&k) {
                    held.push(k);
                }
            } else {
                bot.ctx().platform.input.key(k, false);
                HELD_KEYS.write().retain(|&x| x != k);
            }
        }
    }

    let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"ok\":true}";
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_action(stream: &mut TcpStream, bot: &Arc<Bot>, settings: &Arc<RwLock<Settings>>, body: &str) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let action = parsed.get("action").and_then(|a| a.as_str()).unwrap_or("");

    let res_msg = match action {
        "start" => {
            bot.start();
            "Macro started"
        }
        "pause" => {
            bot.pause();
            "Macro paused"
        }
        "recast" => {
            bot.recast();
            "Rod recast triggered"
        }
        "buy_bait" => {
            let ok = crate::bot::actions::purchase(&bot.ctx());
            if ok { "Bait purchased successfully" } else { "Bait purchase failed (check setup)" }
        }
        "set_volume" => {
            let val_opt = parsed.get("value").and_then(|v| {
                v.as_f64()
                    .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
                    .or_else(|| v.as_u64().map(|u| u as f64))
            });
            if let Some(val) = val_opt {
                let clamped = (val as f32).clamp(0.0, 100.0) / 100.0;
                let _ = crate::core::audio::set_volume(clamped);
            }
            "Volume updated"
        }
        "mute" => {
            let _ = crate::core::audio::set_mute(true);
            "Audio muted"
        }
        "unmute" => {
            let _ = crate::core::audio::set_mute(false);
            "Audio unmuted"
        }
        "set_brightness" => {
            let val_opt = parsed.get("value").and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
                    .or_else(|| v.as_f64().map(|f| f as u64))
            });
            if let Some(val) = val_opt {
                let clamped = (val as u32).clamp(0, 100);
                let _ = crate::core::brightness::set_brightness(clamped);
            }
            "Brightness updated"
        }
        "toggle_spawn" => {
            let mut s = settings.write();
            s.webhook.spawn = !s.webhook.spawn;
            let enabled = s.webhook.spawn;
            let _ = bot.ctx().store.save(&s);
            if enabled { "Fruit Spawn alerts enabled (Active on Roblox)" } else { "Fruit Spawn alerts stopped" }
        }
        "update" => {
            let bot_clone = Arc::clone(bot);
            let s = settings.read();
            let token = s.webhook.telegram_bot_token.clone();
            let chat_id = s.webhook.telegram_chat_id.clone();
            drop(s);

            thread::spawn(move || {
                let cur_ver = env!("CARGO_PKG_VERSION");
                let tok_opt = if !token.trim().is_empty() { Some(token.as_str()) } else { None };
                let cid_opt = if !chat_id.trim().is_empty() { Some(chat_id.as_str()) } else { None };
                let _ = crate::bot::telegram_remote::check_and_apply_update(
                    Some(&bot_clone),
                    tok_opt,
                    cid_opt,
                    cur_ver,
                );
            });
            "Update initiated! The app will install and auto-resume fishing."
        }
        "set_vip_url" => {
            if let Some(val) = parsed.get("value").and_then(|v| v.as_str()) {
                let mut s = settings.write();
                s.features.vip_server_url = val.trim().to_string();
                let _ = bot.ctx().store.save(&s);
            }
            "VIP server link saved"
        }
        "set_private_server_code" => {
            if let Some(val) = parsed.get("value").and_then(|v| v.as_str()) {
                let mut s = settings.write();
                s.features.private_server_code = val.trim().to_string();
                let _ = bot.ctx().store.save(&s);
            }
            "Private server code saved"
        }
        "set_rejoin_macro" => {
            if let Some(val) = parsed.get("value").and_then(|v| v.as_str()) {
                let mut s = settings.write();
                s.features.rejoin_macro_name = val.trim().to_string();
                let _ = bot.ctx().store.save(&s);
            }
            "Rejoin macro assigned"
        }
        "toggle_auto_reconnect" => {
            let mut s = settings.write();
            s.features.auto_reconnect = !s.features.auto_reconnect;
            let on = s.features.auto_reconnect;
            let _ = bot.ctx().store.save(&s);
            if on { "Auto-reconnect enabled" } else { "Auto-reconnect disabled" }
        }
        "toggle_legendary_only" => {
            let mut s = settings.write();
            s.webhook.legendary_only = !s.webhook.legendary_only;
            let on = s.webhook.legendary_only;
            let _ = bot.ctx().store.save(&s);
            if on { "Notifications restricted to Legendary/Mythical & Pity 0 only" } else { "Notifications enabled for ALL fruit drops" }
        }
        "toggle_drop_screenshot" => {
            let mut s = settings.write();
            s.webhook.send_drop_screenshot = !s.webhook.send_drop_screenshot;
            let on = s.webhook.send_drop_screenshot;
            let _ = bot.ctx().store.save(&s);
            if on { "Drop screenshots enabled" } else { "Drop screenshots disabled" }
        }
        _ => "Unknown action",
    };

    let reply = json!({ "ok": true, "message": res_msg }).to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        reply.len(),
        reply
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_craft(stream: &mut TcpStream, bot: &Arc<Bot>, body: &str) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let act = parsed.get("action").and_then(|a| a.as_str()).unwrap_or("");
    let tier_str = parsed.get("tier").and_then(|t| t.as_str()).unwrap_or("rare");

    let (ok, msg) = if act == "stop" {
        crate::bot::crafting::stop_auto_craft();
        (true, "Auto-craft stop requested".to_string())
    } else {
        let tier = match tier_str.to_lowercase().as_str() {
            "legendary" | "leg" => crate::bot::crafting::CraftTier::Legendary,
            "all" => crate::bot::crafting::CraftTier::All,
            "common" => crate::bot::crafting::CraftTier::Common,
            _ => crate::bot::crafting::CraftTier::Rare,
        };
        let ctx = bot.ctx().clone();
        match crate::bot::crafting::start_auto_craft(ctx, tier) {
            Ok(_) => (true, format!("Started auto-crafting {tier:?} Fish Bait!")),
            Err(e) => (false, e),
        }
    };

    let reply = json!({ "ok": ok, "message": msg }).to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        reply.len(),
        reply
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_macro_record(stream: &mut TcpStream, bot: &Arc<Bot>, body: &str) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let act = parsed.get("action").and_then(|a| a.as_str()).unwrap_or("");
    let name = parsed.get("name").and_then(|n| n.as_str()).unwrap_or("");

    let (ok, msg) = match act {
        "start" => {
            match crate::bot::recorder::start_recording(crate::bot::recorder::RecordMode::WebScreen) {
                Ok(_) => (true, "Recording started! Tap on the game screen & controls to record steps.".into()),
                Err(e) => (false, e),
            }
        }
        "stop" => {
            match crate::bot::recorder::stop_recording(name, &bot.ctx().store) {
                Ok(m) => (true, format!("Successfully saved macro '{}' ({} steps)!", m.name, m.steps.len())),
                Err(e) => (false, e),
            }
        }
        "cancel" => {
            crate::bot::recorder::cancel_recording();
            (true, "Recording cancelled.".into())
        }
        _ => (false, "Unknown recording action".into()),
    };

    let reply = json!({ "ok": ok, "message": msg }).to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        reply.len(),
        reply
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_macro_play(stream: &mut TcpStream, bot: &Arc<Bot>, body: &str) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let act = parsed.get("action").and_then(|a| a.as_str()).unwrap_or("play");
    let name = parsed.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let loop_mode = act == "loop" || parsed.get("loop").and_then(|l| l.as_bool()).unwrap_or(false);
    let speed = parsed.get("speed").and_then(|s| s.as_f64()).map(|s| s as f32);
    let max_loops = parsed.get("max_loops").and_then(|m| m.as_u64()).map(|m| m as u32);

    let (ok, msg) = if act == "stop" {
        crate::bot::recorder::stop_playback();
        (true, "Macro playback stop requested.".into())
    } else {
        match crate::bot::recorder::play_macro(bot.ctx().clone(), bot.ctx().store.clone(), name, loop_mode, speed, max_loops) {
            Ok(_) => (true, format!("Playing macro '{name}'{}!", if loop_mode { " in loop" } else { "" })),
            Err(e) => (false, e),
        }
    };

    let reply = json!({ "ok": ok, "message": msg }).to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        reply.len(),
        reply
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_macro_list(stream: &mut TcpStream, bot: &Arc<Bot>) {
    let macros = crate::bot::recorder::load_macros(&bot.ctx().store);
    let status = crate::bot::recorder::get_status();
    let reply = json!({ "ok": true, "macros": macros, "status": status }).to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        reply.len(),
        reply
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_macro_rename(stream: &mut TcpStream, bot: &Arc<Bot>, body: &str) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let id_or_name = parsed.get("name").or_else(|| parsed.get("id")).and_then(|v| v.as_str()).unwrap_or("");
    let new_name = parsed.get("new_name").and_then(|v| v.as_str()).unwrap_or("");
    let (ok, msg) = match crate::bot::recorder::rename_macro(&bot.ctx().store, id_or_name, new_name) {
        Ok(_) => (true, format!("Renamed macro to '{new_name}'")),
        Err(e) => (false, e),
    };
    let reply = json!({ "ok": ok, "message": msg }).to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        reply.len(),
        reply
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_macro_delete(stream: &mut TcpStream, bot: &Arc<Bot>, body: &str) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let id_or_name = parsed.get("name").or_else(|| parsed.get("id")).and_then(|v| v.as_str()).unwrap_or("");
    let (ok, msg) = match crate::bot::recorder::delete_macro(&bot.ctx().store, id_or_name) {
        Ok(_) => (true, format!("Deleted macro '{id_or_name}'")),
        Err(e) => (false, e),
    };
    let reply = json!({ "ok": ok, "message": msg }).to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        reply.len(),
        reply
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn send_keyboard_light_status(stream: &mut TcpStream) {
    let st = crate::laptop_light::get_keyboard_light_status();
    let body = serde_json::to_string(&st).unwrap_or_else(|_| "{}".to_string());
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_keyboard_light(stream: &mut TcpStream, body: &str) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let action = parsed.get("action").and_then(|a| a.as_str()).unwrap_or("toggle");

    let st = crate::laptop_light::set_keyboard_light(action);
    let out = serde_json::to_string(&st).unwrap_or_else(|_| "{}".to_string());

    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        out.len(),
        out
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn send_fan_status(stream: &mut TcpStream) {
    let st = crate::laptop_fan::get_fan_status();
    let body = serde_json::to_string(&st).unwrap_or_else(|_| "{}".to_string());
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_fan_set(stream: &mut TcpStream, body: &str) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let st = if let Some(auto_turbo) = parsed.get("auto_turbo").and_then(|v| v.as_bool()) {
        crate::laptop_fan::set_auto_turbo(auto_turbo)
    } else if let Some(pct) = parsed.get("pct").and_then(|v| v.as_u64()) {
        crate::laptop_fan::set_fan_percentage(pct as u32)
    } else if let Some(mode) = parsed.get("mode").and_then(|v| v.as_str()) {
        crate::laptop_fan::set_fan_mode(mode)
    } else {
        crate::laptop_fan::get_fan_status()
    };
    let out = serde_json::to_string(&st).unwrap_or_else(|_| "{}".to_string());
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        out.len(),
        out
    );
    let _ = stream.write_all(resp.as_bytes());
}

/// GPO Cyberdeck webapp: five tab pages + vendored static assets.
///
/// Pages are plain files under `src/bot/web/` compiled in with
/// `include_str!` (zero runtime disk dependency). All Tailwind/fonts/JS
/// assets are vendored (`/assets/*`) so the webapp works fully offline on
/// the LAN — no CDN. HTML shells carry no secrets; the embedded JS
/// attaches `?token=` exactly like before.
const WEB_REMOTE_HTML: &str = include_str!("web/remote.html");
const WEB_STATS_HTML: &str = include_str!("web/stats.html");
const WEB_STUDIO_HTML: &str = include_str!("web/studio.html");
const WEB_CRAFT_HTML: &str = include_str!("web/craft.html");
const WEB_SYSTEM_HTML: &str = include_str!("web/system.html");

const ASSET_TAILWIND_JS: &[u8] = include_bytes!("web/assets/tailwind.js");
const ASSET_FONTS_CSS: &str = include_str!("web/assets/fonts.css");
const ASSET_GPO_CORE_JS: &str = include_str!("web/assets/gpo-core.js");
const ASSET_GPO_REMOTE_JS: &str = include_str!("web/assets/gpo-remote.js");
const ASSET_GPO_STATS_JS: &str = include_str!("web/assets/gpo-stats.js");
const ASSET_GPO_STUDIO_JS: &str = include_str!("web/assets/gpo-studio.js");
const ASSET_GPO_CRAFT_JS: &str = include_str!("web/assets/gpo-craft.js");
const ASSET_GPO_SYSTEM_JS: &str = include_str!("web/assets/gpo-system.js");

macro_rules! include_font {
    ($name:literal) => {
        include_bytes!(concat!("web/assets/fonts/", $name))
    };
}

/// Map every vendored woff2 to its bytes. The list is generated from
/// `web/assets/fonts/` — a build break here means fonts.css references a
/// file that was not vendored (fix by re-running the vendor step).
fn font_bytes(name: &str) -> Option<&'static [u8]> {
    match name {
        "sym1.woff2" => Some(include_font!("sym1.woff2")),
        "w1.woff2" => Some(include_font!("w1.woff2")),
        "w10.woff2" => Some(include_font!("w10.woff2")),
        "w11.woff2" => Some(include_font!("w11.woff2")),
        "w12.woff2" => Some(include_font!("w12.woff2")),
        "w13.woff2" => Some(include_font!("w13.woff2")),
        "w14.woff2" => Some(include_font!("w14.woff2")),
        "w15.woff2" => Some(include_font!("w15.woff2")),
        "w16.woff2" => Some(include_font!("w16.woff2")),
        "w2.woff2" => Some(include_font!("w2.woff2")),
        "w3.woff2" => Some(include_font!("w3.woff2")),
        "w4.woff2" => Some(include_font!("w4.woff2")),
        "w5.woff2" => Some(include_font!("w5.woff2")),
        "w6.woff2" => Some(include_font!("w6.woff2")),
        "w7.woff2" => Some(include_font!("w7.woff2")),
        "w8.woff2" => Some(include_font!("w8.woff2")),
        "w9.woff2" => Some(include_font!("w9.woff2")),
        _ => None,
    }
}

fn send_bytes(stream: &mut TcpStream, content_type: &str, bytes: &[u8]) {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nCache-Control: public, max-age=86400\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        content_type,
        bytes.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(bytes);
}

fn send_html_page(stream: &mut TcpStream, html: &str) {
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-cache, no-store, must-revalidate, max-age=0\r\nPragma: no-cache\r\nExpires: 0\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = stream.write_all(resp.as_bytes());
}

/// Public (unauthenticated) static asset. Returns true when the path was an
/// asset route (served or 404); false for non-asset paths.
fn serve_asset(stream: &mut TcpStream, raw_path: &str) -> bool {
    const PREFIX: &str = "/assets/";
    if !raw_path.starts_with(PREFIX) {
        return false;
    }
    let name = &raw_path[PREFIX.len()..];
    // Only flat asset names plus exactly `fonts/<file>.woff2` exist.
    // Anything else (nesting, `..`, backslashes) is a 404.
    let is_font = name.strip_prefix("fonts/").is_some_and(|rest| {
        rest.ends_with(".woff2") && !rest.contains('/') && !rest.contains('\\') && !rest.contains("..")
    });
    if (!is_font && (name.contains('/') || name.contains('\\') || name.contains(".."))) || name.is_empty() {
        let nf = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(nf.as_bytes());
        return true;
    }
    if is_font {
        let rest = &name["fonts/".len()..];
        if let Some(bytes) = font_bytes(rest) {
            send_bytes(stream, "font/woff2", bytes);
        } else {
            let nf = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(nf.as_bytes());
        }
        return true;
    }
    match name {
        "tailwind.js" => send_bytes(stream, "text/javascript; charset=utf-8", ASSET_TAILWIND_JS),
        "fonts.css" => send_bytes(stream, "text/css; charset=utf-8", ASSET_FONTS_CSS.as_bytes()),
        "gpo-core.js" => send_bytes(stream, "text/javascript; charset=utf-8", ASSET_GPO_CORE_JS.as_bytes()),
        "gpo-remote.js" => send_bytes(stream, "text/javascript; charset=utf-8", ASSET_GPO_REMOTE_JS.as_bytes()),
        "gpo-stats.js" => send_bytes(stream, "text/javascript; charset=utf-8", ASSET_GPO_STATS_JS.as_bytes()),
        "gpo-studio.js" => send_bytes(stream, "text/javascript; charset=utf-8", ASSET_GPO_STUDIO_JS.as_bytes()),
        "gpo-craft.js" => send_bytes(stream, "text/javascript; charset=utf-8", ASSET_GPO_CRAFT_JS.as_bytes()),
        "gpo-system.js" => send_bytes(stream, "text/javascript; charset=utf-8", ASSET_GPO_SYSTEM_JS.as_bytes()),
        _ => {
            let nf = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(nf.as_bytes());
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;

    fn settings_with_token(token: &str) -> Arc<RwLock<Settings>> {
        let mut s = Settings::default();
        s.web.token = token.into();
        Arc::new(RwLock::new(s))
    }

    #[test]
    fn bearer_or_query_token_authorizes() {
        let st = settings_with_token("tok123");
        assert!(is_authorized(&st, Some("tok123".into()), None));
        assert!(is_authorized(&st, None, Some("tok123".into())));
        assert!(!is_authorized(&st, Some("wrong".into()), None));
        assert!(!is_authorized(&st, None, None));
        assert!(!is_authorized(&st, Some("tok123 ".into()), None));
    }

    #[test]
    fn empty_expected_token_never_authorizes() {
        let st = settings_with_token("");
        assert!(!is_authorized(&st, Some(String::new()), None));
        assert!(!is_authorized(&st, None, None));
    }

    #[test]
    fn query_token_extraction() {
        assert_eq!(query_token("fps=5&token=abc123"), Some("abc123".into()));
        assert_eq!(query_token("token=abc123&fps=5"), Some("abc123".into()));
        assert_eq!(query_token("fps=5"), None);
        assert_eq!(query_token(""), None);
    }

    #[test]
    fn header_extraction_is_case_insensitive() {
        let req = "POST /api/click HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer abc\r\n\r\n";
        assert_eq!(header_value(req, "authorization"), Some("Bearer abc".into()));
        assert_eq!(header_value(req, "ORIGIN"), None);
        let req2 = "GET / HTTP/1.1\r\norigin: http://localhost:3888\r\n\r\n";
        assert_eq!(header_value(req2, "Origin"), Some("http://localhost:3888".into()));
    }

    #[test]
    fn split_post_body_reassembles() {
        use std::io::Write;
        use std::net::TcpListener;
        // Simulate TCP segmentation: headers first, body 100ms later.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let writer = std::thread::spawn(move || {
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(b"POST /api/action HTTP/1.1\r\nHost: x\r\nContent-Length: 18\r\n\r\n").unwrap();
            s.flush().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
            s.write_all(br#"{"action":"start"}"#).unwrap();
            s.flush().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(300));
        });
        let (mut server, _) = listener.accept().unwrap();
        server.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let req = read_full_request(&mut server).expect("request assembles");
        assert!(req.contains("POST /api/action"), "got: {req}");
        assert!(req.contains(r#"{"action":"start"}"#), "body must survive segmentation, got: {req}");
        writer.join().unwrap();
    }

    #[test]
    fn cors_only_allows_loopback_by_default() {
        let loopback = "GET / HTTP/1.1\r\nOrigin: http://127.0.0.1:3888\r\n\r\n";
        let lan = "GET / HTTP/1.1\r\nOrigin: http://192.168.1.5:3000\r\n\r\n";
        let evil = "GET / HTTP/1.1\r\nOrigin: https://evil.example\r\n\r\n";
        assert!(cors_origin_for(loopback, false).is_some());
        assert!(cors_origin_for(lan, false).is_none());
        assert!(cors_origin_for(lan, true).is_some());
        assert!(cors_origin_for(evil, true).is_none());
        assert!(cors_origin_for("GET / HTTP/1.1\r\n\r\n", true).is_none());
    }

    #[test]
    fn split_csv_line_handles_quotes() {
        assert_eq!(split_csv_line("a,b,c"), vec!["a", "b", "c"]);
        assert_eq!(
            split_csv_line("2026-09-13 11:04:17,fish,\"Fish\",\"u-ev, Item\""),
            vec!["2026-09-13 11:04:17", "fish", "Fish", "u-ev, Item"]
        );
        assert_eq!(
            split_csv_line("t,fish,\"A \"\"quoted\"\" name\",x"),
            vec!["t", "fish", "A \"quoted\" name", "x"]
        );
        assert_eq!(split_csv_line(""), vec![""]);
    }

    #[test]
    fn recent_catches_reads_newest_first_capped() {
        let dir = std::env::temp_dir().join(format!(
            "gpo-web-catches-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let store = crate::config::Store::new(dir.clone());
        let mut csv = String::from("Timestamp,Type,Name,RawText\n");
        for i in 0..8 {
            csv.push_str(&format!("2026-10-0{i} 10:00:0{i},fish,\"Fish {i}\",raw {i}\n"));
        }
        std::fs::write(store.catches_path(), csv).unwrap();
        let val = recent_catches(&store);
        let arr = val.as_array().expect("array");
        assert_eq!(arr.len(), 6, "capped at 6, got {arr:?}");
        assert_eq!(arr[0].get("timestamp").and_then(|v| v.as_str()), Some("2026-10-07 10:00:07"));
        assert_eq!(arr[0].get("type").and_then(|v| v.as_str()), Some("fish"));
        assert_eq!(arr[0].get("name").and_then(|v| v.as_str()), Some("Fish 7"));
        assert_eq!(arr[5].get("name").and_then(|v| v.as_str()), Some("Fish 2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recent_catches_missing_file_is_empty() {
        let dir = std::env::temp_dir().join(format!(
            "gpo-web-nocatch-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let store = crate::config::Store::new(dir.clone());
        assert_eq!(recent_catches(&store), serde_json::json!([]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vendored_fonts_cover_css_without_remote_refs() {
        for css in [ASSET_FONTS_CSS] {
            assert!(!css.contains("https://"), "fonts.css must not fetch remote URLs");
        }
        let mut missing = Vec::new();
        let mut count = 0;
        for cap in ASSET_FONTS_CSS.split("url(").skip(1) {
            let end = cap.find(')').unwrap_or(cap.len());
            let url = cap[..end].trim_matches(|c| c == '\'' || c == '"' || c == ' ');
            if let Some(file) = url.strip_prefix("fonts/") {
                count += 1;
                if font_bytes(file).is_none() {
                    missing.push(file.to_string());
                }
            }
        }
        assert!(count >= 10, "expected many vendored font faces, got {count}");
        assert!(missing.is_empty(), "unvendored font files: {missing:?}");
    }

    #[test]
    fn tab_pages_are_self_contained() {
        for (name, page) in [
            ("remote", WEB_REMOTE_HTML),
            ("stats", WEB_STATS_HTML),
            ("studio", WEB_STUDIO_HTML),
            ("craft", WEB_CRAFT_HTML),
            ("system", WEB_SYSTEM_HTML),
        ] {
            for needle in [
                "cdn.tailwindcss.com",
                "fonts.googleapis.com",
                "fonts.gstatic.com",
                "googleusercontent.com",
            ] {
                assert!(!page.contains(needle), "{name} must not reference {needle} (offline LAN app)");
            }
            assert!(page.contains("/assets/tailwind.js"), "{name} must load vendored tailwind");
            assert!(page.contains("/assets/fonts.css"), "{name} must load vendored fonts");
            assert!(page.contains("GPO CYBERDECK"), "{name} must carry the shell");
        }
    }

    fn read_response(stream: &mut std::net::TcpStream) -> String {
        use std::io::Read;
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    #[test]
    fn asset_and_page_routes_serve() {
        use std::io::Write;
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        // Handlers only write (never read the request), so: connect, accept,
        // serve, drop the server side (EOF), then slurp the response.
        let fetch = |path: &str| {
            let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            let (mut server, _) = listener.accept().unwrap();
            if path.starts_with("/assets/") {
                assert!(serve_asset(&mut server, path));
            } else {
                send_html_page(&mut server, WEB_STATS_HTML);
            }
            drop(server);
            read_response(&mut client)
        };
        let serve = fetch;

        let js = serve("/assets/tailwind.js");
        assert!(js.starts_with("HTTP/1.1 200 OK"), "tailwind route, got: {}", &js[..js.len().min(60)]);
        assert!(js.contains("text/javascript"), "js content type, got: {}", &js[..js.len().min(200)]);
        assert!(js.len() > 100_000, "vendored tailwind must be substantial");

        let css = serve("/assets/fonts.css");
        assert!(css.starts_with("HTTP/1.1 200 OK"));
        assert!(css.contains("text/css"));

        let font = serve("/assets/fonts/w1.woff2");
        assert!(font.starts_with("HTTP/1.1 200 OK"));
        assert!(font.contains("font/woff2"));

        let missing = serve("/assets/fonts/nope.woff2");
        assert!(missing.starts_with("HTTP/1.1 404"));

        let traversal = serve("/assets/../secret");
        assert!(traversal.starts_with("HTTP/1.1 404"));

        let page = serve("/stats");
        assert!(page.starts_with("HTTP/1.1 200 OK"));
        assert!(page.contains("text/html"));
        assert!(page.contains("loot-telemetry-list"), "stats page must carry the loot feed");
    }
}
