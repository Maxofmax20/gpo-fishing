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
            let listener = match TcpListener::bind(format!("0.0.0.0:{port}")) {
                Ok(l) => {
                    tracing::info!("Web Dashboard running at http://0.0.0.0:{port}");
                    l
                }
                Err(e) => {
                    tracing::warn!("Failed to bind web server on port {port}: {e}");
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

fn handle_client(mut stream: TcpStream, bot: &Arc<Bot>, settings: &Arc<RwLock<Settings>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf) {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let req_str = String::from_utf8_lossy(&buf[..n]);
    let first_line = req_str.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }

    let method = parts[0];
    let path = parts[1];

    if method == "OPTIONS" {
        let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes());
        return;
    }

    let raw_path = path.split('?').next().unwrap_or(path);
    let query = path.split('?').nth(1).unwrap_or("");

    if raw_path == "/" || raw_path == "/index.html" {
        send_html(&mut stream);
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
    } else {
        let not_found = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(not_found.as_bytes());
    }
}

fn stream_mjpeg(mut stream: TcpStream, bot: &Arc<Bot>, query: &str) {
    let mut fps: u32 = 20;
    let mut scale: usize = 720;
    let mut quality: u8 = 70;

    for param in query.split('&') {
        let mut kv = param.split('=');
        if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
            match k {
                "fps" => {
                    if let Ok(n) = v.parse::<u32>() {
                        fps = n.clamp(5, 60);
                    }
                }
                "scale" => {
                    if let Ok(n) = v.parse::<usize>() {
                        scale = n;
                    }
                }
                "q" | "quality" => {
                    if let Ok(n) = v.parse::<u8>() {
                        quality = n.clamp(20, 95);
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
                  Access-Control-Allow-Origin: *\r\n\
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
        "auto_reconnect": s.features.auto_reconnect,
        "vip_server_url": s.features.vip_server_url.clone(),
    });

    let body = payload.to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
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
                        scale = n;
                    }
                }
                "q" | "quality" => {
                    if let Ok(n) = v.parse::<u8>() {
                        quality = n.clamp(20, 95);
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
            "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-cache, no-store, must-revalidate\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&bytes);
        return;
    }

    if let Some(cached) = LAST_FRAME.read().as_ref() {
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-cache, no-store, must-revalidate\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            cached.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(cached);
        return;
    }

    let msg = "Waiting for Roblox window...";
    let resp = format!(
        "HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        msg.len(),
        msg
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_click(stream: &mut TcpStream, bot: &Arc<Bot>, body: &str) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let rx = parsed.get("rel_x").and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
    let ry = parsed.get("rel_y").and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
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

    let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"ok\":true}";
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
        let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"ok\":true}";
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

    let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"ok\":true}";
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_mouse(stream: &mut TcpStream, bot: &Arc<Bot>, body: &str) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let act = parsed.get("action").and_then(|v| v.as_str()).unwrap_or("click");
    let btn_str = parsed.get("button").and_then(|v| v.as_str()).unwrap_or("left");
    let rx = parsed.get("rel_x").and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
    let ry = parsed.get("rel_y").and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;

    let btn = if btn_str == "right" {
        crate::core::types::MouseButton::Right
    } else {
        crate::core::types::MouseButton::Left
    };

    if act == "release_all" || act == "reset" {
        bot.ctx().platform.input.button(crate::core::types::MouseButton::Left, false);
        bot.ctx().platform.input.button(crate::core::types::MouseButton::Right, false);
        let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"ok\":true}";
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

    let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"ok\":true}";
    let _ = stream.write_all(resp.as_bytes());
}

fn handle_key(stream: &mut TcpStream, bot: &Arc<Bot>, body: &str) {
    ensure_key_watchdog(bot);

    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let key_str = parsed.get("key").and_then(|v| v.as_str()).unwrap_or("");
    let is_down = parsed.get("down").and_then(|v| v.as_bool()).unwrap_or(true);
    let tap = parsed.get("tap").and_then(|v| v.as_bool()).unwrap_or(false);

    *LAST_KEY_ACTIVITY.write() = Some(std::time::Instant::now());

    if key_str == "heartbeat" {
        // Just refresh the activity timestamp, keys remain held!
        let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"ok\":true}";
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

    let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"ok\":true}";
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
            if let Some(val) = parsed.get("value").and_then(|v| v.as_f64()) {
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
            if let Some(val) = parsed.get("value").and_then(|v| v.as_u64()) {
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
        "toggle_auto_reconnect" => {
            let mut s = settings.write();
            s.features.auto_reconnect = !s.features.auto_reconnect;
            let on = s.features.auto_reconnect;
            let _ = bot.ctx().store.save(&s);
            if on { "Auto-reconnect enabled" } else { "Auto-reconnect disabled" }
        }
        _ => "Unknown action",
    };

    let reply = json!({ "ok": true, "message": res_msg }).to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        reply.len(),
        reply
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn send_keyboard_light_status(stream: &mut TcpStream) {
    let st = crate::laptop_light::get_keyboard_light_status();
    let body = serde_json::to_string(&st).unwrap_or_else(|_| "{}".to_string());
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        out.len(),
        out
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn send_html(stream: &mut TcpStream) {
    let html = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no, viewport-fit=cover">
<meta http-equiv="Cache-Control" content="no-cache, no-store, must-revalidate">
<meta http-equiv="Pragma" content="no-cache">
<meta http-equiv="Expires" content="0">
<!-- Mobile / PWA App Meta Tags -->
<meta name="apple-mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">
<meta name="apple-mobile-web-app-title" content="GPO CyberDeck">
<meta name="mobile-web-app-capable" content="yes">
<meta name="theme-color" content="#07090e">
<meta name="format-detection" content="telephone=no">
<title>GPO Autofish Mobile</title>
<!-- Embedded Favicon & Apple Touch Icon -->
<link rel="icon" type="image/svg+xml" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'%3E%3Cdefs%3E%3ClinearGradient id='g' x1='0%25' y1='0%25' x2='100%25' y2='100%25'%3E%3Cstop offset='0%25' stop-color='%2300f0ff'/%3E%3Cstop offset='100%25' stop-color='%23b026ff'/%3E%3C/linearGradient%3E%3C/defs%3E%3Crect width='100' height='100' rx='24' fill='%2307090e'/%3E%3Ccircle cx='50' cy='50' r='38' stroke='url(%23g)' stroke-width='4' fill='none' opacity='0.4'/%3E%3Cpath d='M50 20 L58 42 L80 50 L58 58 L50 80 L42 58 L20 50 L42 42 Z' fill='url(%23g)'/%3E%3C/svg%3E">
<link rel="apple-touch-icon" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'%3E%3Cdefs%3E%3ClinearGradient id='g' x1='0%25' y1='0%25' x2='100%25' y2='100%25'%3E%3Cstop offset='0%25' stop-color='%2300f0ff'/%3E%3Cstop offset='100%25' stop-color='%23b026ff'/%3E%3C/linearGradient%3E%3C/defs%3E%3Crect width='100' height='100' rx='24' fill='%2307090e'/%3E%3Ccircle cx='50' cy='50' r='38' stroke='url(%23g)' stroke-width='4' fill='none' opacity='0.4'/%3E%3Cpath d='M50 20 L58 42 L80 50 L58 58 L50 80 L42 58 L20 50 L42 42 Z' fill='url(%23g)'/%3E%3C/svg%3E">
<!-- Embedded Web App Manifest -->
<link rel="manifest" href="data:application/manifest+json,%7B%22name%22%3A%22GPO%20Autofish%20CyberDeck%22%2C%22short_name%22%3A%22GPO%20Macro%22%2C%22start_url%22%3A%22%2F%22%2C%22display%22%3A%22standalone%22%2C%22background_color%22%3A%22%2307090e%22%2C%22theme_color%22%3A%22%2307090e%22%7D">
<script src="https://telegram.org/js/telegram-web-app.js"></script>
<style>
:root {
  --bg: #07090e;
  --bg-surface: #0c101a;
  --card: rgba(14, 20, 32, 0.88);
  --card-border: rgba(30, 41, 59, 0.8);
  --card-highlight: rgba(255, 255, 255, 0.05);
  --cyan: #00f0ff;
  --purple: #b026ff;
  --emerald: #10b981;
  --amber: #f59e0b;
  --rose: #ef4444;
  --text: #f8fafc;
  --text-dim: #94a3b8;
  --text-mute: #64748b;
  --safe-bottom: env(safe-area-inset-bottom, 16px);
  --safe-top: env(safe-area-inset-top, 0px);
}

* {
  box-sizing: border-box; margin: 0; padding: 0;
  font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Display', 'Segoe UI', Roboto, Helvetica, sans-serif;
  -webkit-tap-highlight-color: transparent !important;
  -webkit-touch-callout: none !important;
  user-select: none !important;
  -webkit-user-select: none !important;
}

html, body {
  background: var(--bg); color: var(--text); min-height: 100vh;
  background-image: radial-gradient(circle at 50% 0%, rgba(0, 240, 255, 0.06), transparent 50%), radial-gradient(circle at 100% 100%, rgba(176, 38, 255, 0.05), transparent 50%);
  overscroll-behavior-y: none !important;
  touch-action: manipulation;
  overflow-x: hidden;
}

input, textarea {
  user-select: text !important;
  -webkit-user-select: text !important;
  -webkit-touch-callout: default !important;
}

/* APP SHELL & CONTAINER */
.app-shell {
  display: flex; flex-direction: column; min-height: 100vh; position: relative;
}

/* STICKY TOP APP HEADER */
.app-header {
  position: sticky; top: 0; z-index: 900;
  padding: max(10px, var(--safe-top)) 14px 10px 14px;
  background: rgba(7, 9, 14, 0.88);
  backdrop-filter: blur(20px); -webkit-backdrop-filter: blur(20px);
  border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  display: flex; justify-content: space-between; align-items: center; gap: 10px;
}

.brand { display: flex; align-items: center; gap: 10px; min-width: 0; }
.brand-avatar {
  width: 36px; height: 36px; border-radius: 10px;
  background: linear-gradient(135deg, rgba(0, 240, 255, 0.25), rgba(176, 38, 255, 0.25));
  border: 1px solid var(--cyan);
  display: flex; align-items: center; justify-content: center; font-size: 1.15rem;
  box-shadow: 0 0 14px rgba(0, 240, 255, 0.35); flex-shrink: 0;
}
.brand-text { display: flex; flex-direction: column; min-width: 0; }
.brand-title {
  font-size: 1.05rem; font-weight: 800; letter-spacing: -0.3px;
  background: linear-gradient(135deg, var(--cyan), #c084fc);
  -webkit-background-clip: text; -webkit-text-fill-color: transparent;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.brand-sub { font-size: 0.68rem; color: var(--text-mute); font-family: monospace; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

.header-actions { display: flex; align-items: center; gap: 6px; flex-shrink: 0; }

.header-btn-toggle {
  height: 32px; padding: 0 10px; border-radius: 999px;
  background: linear-gradient(135deg, #00d2ff, #0084ff); color: #fff;
  border: 1px solid rgba(255,255,255,0.25); font-size: 0.72rem; font-weight: 800;
  display: flex; align-items: center; gap: 5px; cursor: pointer;
  box-shadow: 0 2px 10px rgba(0, 132, 255, 0.4); transition: all 0.15s ease;
}
.header-btn-toggle.active {
  background: linear-gradient(135deg, #f59e0b, #d97706);
  box-shadow: 0 2px 10px rgba(245, 158, 11, 0.4);
}
.header-btn-toggle:active { transform: scale(0.95); }

.status-badge {
  padding: 5px 10px; border-radius: 999px; font-size: 0.68rem; font-weight: 800;
  letter-spacing: 0.4px; text-transform: uppercase; display: flex; align-items: center; gap: 5px;
  border: 1px solid transparent; transition: all 0.25s ease;
}
.badge-running { background: rgba(16, 185, 129, 0.16); color: var(--emerald); border-color: rgba(16, 185, 129, 0.4); box-shadow: 0 0 12px rgba(16, 185, 129, 0.25); }
.badge-paused { background: rgba(245, 158, 11, 0.16); color: var(--amber); border-color: rgba(245, 158, 11, 0.4); box-shadow: 0 0 12px rgba(245, 158, 11, 0.25); }
.badge-stopped { background: rgba(100, 116, 139, 0.16); color: var(--text-dim); border-color: rgba(100, 116, 139, 0.3); }

.toggle-spawn-badge {
  cursor: pointer; padding: 5px 8px; border-radius: 999px; font-size: 0.68rem; font-weight: 700;
  background: rgba(176, 38, 255, 0.15); color: #d8b4fe; border: 1px solid rgba(176, 38, 255, 0.35);
  transition: all 0.2s ease;
}
.toggle-spawn-badge.off {
  background: rgba(100, 116, 139, 0.12); color: var(--text-mute); border-color: rgba(100, 116, 139, 0.25);
}

/* MAIN CONTENT AREA */
.app-content {
  flex: 1; max-width: 680px; width: 100%; margin: 0 auto;
  padding: 12px 14px calc(76px + var(--safe-bottom)) 14px;
  display: flex; flex-direction: column; gap: 12px;
}

/* MOBILE SECTIONS */
.mobile-section {
  display: none; flex-direction: column; gap: 12px;
  animation: mobileFade 0.22s cubic-bezier(0.16, 1, 0.3, 1);
}
.mobile-section.active { display: flex; }
@keyframes mobileFade {
  from { opacity: 0; transform: translateY(6px); }
  to { opacity: 1; transform: translateY(0); }
}

/* CARDS */
.card {
  background: var(--card); border: 1px solid var(--card-border); border-radius: 16px; padding: 14px;
  display: flex; flex-direction: column; gap: 8px; box-shadow: 0 8px 24px rgba(0,0,0,0.35);
  backdrop-filter: blur(14px); -webkit-backdrop-filter: blur(14px);
}
.card-label { font-size: 0.72rem; color: var(--text-dim); text-transform: uppercase; font-weight: 800; letter-spacing: 0.5px; }
.card-val { font-size: 1.45rem; font-weight: 800; color: var(--text); }
.card-val.fruit { color: #d8b4fe; text-shadow: 0 0 16px rgba(176, 38, 255, 0.35); }
.card-meta { font-size: 0.72rem; color: var(--text-mute); }

/* STATS GAUGES & GRIDS */
.stat-grid { display: grid; grid-template-columns: repeat(2, 1fr); gap: 10px; }
.gauge-bar-bg {
  width: 100%; height: 6px; background: rgba(255,255,255,0.08); border-radius: 999px;
  overflow: hidden; margin-top: 4px;
}
.gauge-bar-fill {
  height: 100%; width: 0%; border-radius: 999px;
  background: linear-gradient(90deg, var(--cyan), var(--purple));
  transition: width 0.4s ease;
}

/* STREAM WRAPPER & TOOLBAR */
.stream-wrapper {
  background: var(--card); border: 1px solid var(--card-border); border-radius: 16px; overflow: hidden;
  position: relative; box-shadow: 0 12px 36px rgba(0,0,0,0.5); transition: all 0.25s ease;
}
.stream-bar {
  display: flex; justify-content: space-between; align-items: center; padding: 10px 12px;
  background: rgba(0,0,0,0.45); border-bottom: 1px solid var(--card-border); font-size: 0.74rem; font-weight: 600;
  gap: 6px; flex-wrap: wrap;
}
.stream-indicator { display: flex; align-items: center; gap: 6px; }
.live-dot { width: 8px; height: 8px; border-radius: 50%; background: #64748b; transition: all 0.3s; }
.live-dot.on { background: var(--emerald); box-shadow: 0 0 10px var(--emerald); animation: pulseDot 1.5s infinite; }
@keyframes pulseDot { 0%, 100% { opacity: 1; } 50% { opacity: 0.3; } }

.stream-tools-group { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
.stream-tool-btn {
  height: 32px; padding: 0 9px; font-size: 0.72rem; font-weight: 700; border-radius: 8px;
  background: rgba(30, 41, 59, 0.85); border: 1px solid var(--card-border); color: var(--text);
  display: flex; align-items: center; justify-content: center; gap: 5px; cursor: pointer;
  user-select: none; transition: all 0.15s ease; white-space: nowrap;
}
.stream-tool-btn:hover, .stream-tool-btn:active { background: rgba(51, 65, 85, 0.95); }
.stream-tool-btn.active {
  background: rgba(0, 240, 255, 0.2); border-color: var(--cyan); color: var(--cyan);
}

.screen-box {
  width: 100%; min-height: 220px; background: #030508; display: flex; align-items: center; justify-content: center;
  position: relative; overflow: hidden; cursor: crosshair; touch-action: none;
}
.screen-img {
  width: 100%; max-height: 480px; object-fit: contain; display: block; user-select: none;
  pointer-events: none !important; -webkit-user-drag: none !important;
}

.click-ripple {
  position: absolute; width: 24px; height: 24px; border-radius: 50%;
  border: 2px solid var(--cyan); background: rgba(0, 240, 255, 0.3);
  transform: translate(-50%, -50%) scale(0.2); pointer-events: none;
  animation: ripple 0.4s ease-out forwards; z-index: 9999;
}
@keyframes ripple {
  0% { transform: translate(-50%, -50%) scale(0.2); opacity: 1; }
  100% { transform: translate(-50%, -50%) scale(2.2); opacity: 0; }
}

.touch-drag-indicator {
  position: absolute; width: 50px; height: 50px; border-radius: 50%;
  border: 2px solid var(--cyan); background: rgba(0, 240, 255, 0.2);
  transform: translate(-50%, -50%) scale(0.9); pointer-events: none; z-index: 10000;
  box-shadow: 0 0 18px rgba(0, 240, 255, 0.6), inset 0 0 14px rgba(0, 240, 255, 0.35);
  display: flex; align-items: center; justify-content: center;
  transition: border-color 0.15s, background 0.15s, transform 0.08s ease-out;
}
.touch-drag-indicator.right-mode {
  border-color: var(--purple); background: rgba(176, 38, 255, 0.22);
  box-shadow: 0 0 18px rgba(176, 38, 255, 0.6), inset 0 0 14px rgba(176, 38, 255, 0.35);
}
.touch-drag-indicator.dragging {
  transform: translate(-50%, -50%) scale(1.22); border-width: 2.5px;
}
.touch-drag-indicator-core {
  width: 10px; height: 10px; border-radius: 50%; background: #fff;
  box-shadow: 0 0 8px #fff; pointer-events: none;
}

/* FULLSCREEN IMMERSIVE MODE */
.stream-wrapper.fullscreen-active {
  position: fixed !important; top: 0 !important; left: 0 !important;
  width: 100vw !important; height: 100vh !important; max-width: 100vw !important; max-height: 100vh !important;
  border-radius: 0 !important; border: none !important; z-index: 999999 !important;
  background: #000 !important; display: flex !important; flex-direction: column !important;
  margin: 0 !important; padding: 0 !important;
}
.stream-wrapper.fullscreen-active.rotated-90 {
  width: 100vh !important; height: 100vw !important;
  position: fixed !important; top: 50% !important; left: 50% !important;
  transform: translate(-50%, -50%) rotate(90deg) !important;
}
.stream-wrapper.fullscreen-active .stream-bar { display: none !important; }
.stream-wrapper.fullscreen-active .screen-box {
  flex: 1 !important; width: 100%; height: 100%;
  max-height: 100% !important; min-height: 100% !important; border-radius: 0 !important;
}
.stream-wrapper.fullscreen-active .screen-img {
  width: 100%; height: 100%; max-height: 100% !important; object-fit: contain !important;
}

.fs-floating-bar {
  display: none; position: absolute; top: 12px; left: 12px; right: 12px;
  z-index: 100000; pointer-events: none; justify-content: space-between; align-items: center;
}
.stream-wrapper.fullscreen-active .fs-floating-bar { display: flex; }
.fs-badge-group { display: flex; align-items: center; gap: 8px; pointer-events: auto; }
.fs-stream-pill {
  background: rgba(16, 22, 34, 0.85); border: 1px solid var(--card-border); padding: 4px 10px;
  border-radius: 999px; font-size: 0.7rem; font-family: monospace; font-weight: 800;
  color: var(--cyan); backdrop-filter: blur(10px); box-shadow: 0 4px 12px rgba(0,0,0,0.5);
}
.fs-btn {
  pointer-events: auto; background: rgba(16, 22, 34, 0.85); border: 1px solid var(--card-border);
  color: var(--text); padding: 6px 12px; border-radius: 999px; font-size: 0.72rem; font-weight: 800;
  cursor: pointer; backdrop-filter: blur(10px); transition: all 0.15s; display: flex; align-items: center;
  gap: 5px; box-shadow: 0 4px 16px rgba(0,0,0,0.5); user-select: none;
}
.fs-btn:active { transform: scale(0.95); }
.fs-btn.active { background: rgba(0, 240, 255, 0.2); border-color: var(--cyan); color: var(--cyan); }
.fs-btn-close {
  background: rgba(239, 68, 68, 0.3); border-color: rgba(239, 68, 68, 0.5); color: #fca5a5;
}

.fs-controls-overlay {
  display: none; position: absolute; inset: 0; z-index: 99999;
  pointer-events: none; flex-direction: column; justify-content: space-between;
  padding: 58px 16px 20px 16px;
}
.stream-wrapper.fullscreen-active .fs-controls-overlay.visible { display: flex; }
.fs-quick-bar {
  display: flex; justify-content: center; gap: 8px; pointer-events: none; flex-wrap: wrap;
}
.fs-mini-btn {
  pointer-events: auto; background: rgba(16, 22, 34, 0.85); border: 1px solid rgba(255, 255, 255, 0.2);
  color: var(--text); padding: 6px 12px; border-radius: 8px; font-size: 0.72rem; font-weight: 700;
  cursor: pointer; backdrop-filter: blur(10px); box-shadow: 0 4px 14px rgba(0,0,0,0.5);
  transition: all 0.15s; user-select: none;
}
.fs-mini-btn:active { transform: scale(0.95); }

.fs-bottom-controls {
  display: flex; justify-content: space-between; align-items: flex-end;
  pointer-events: none; gap: 12px; width: 100%;
}
.fs-pad-cluster { display: flex; flex-direction: column; gap: 6px; pointer-events: none; }
.fs-cluster-label {
  font-size: 0.65rem; font-weight: 800; color: var(--text-dim); letter-spacing: 0.5px;
  text-shadow: 0 2px 4px rgba(0,0,0,0.8);
}
.fs-pad-btn {
  pointer-events: auto; backdrop-filter: blur(12px);
  background: rgba(16, 22, 34, 0.75) !important;
  border: 1px solid rgba(255, 255, 255, 0.2) !important;
  box-shadow: 0 6px 18px rgba(0,0,0,0.6);
}

/* ERGONOMIC TOUCH GAMEPAD */
.controller-card {
  background: linear-gradient(180deg, rgba(16, 24, 39, 0.82) 0%, rgba(10, 15, 26, 0.94) 100%);
  border: 1px solid rgba(0, 240, 255, 0.25);
  border-radius: 18px; padding: 14px;
  display: flex; flex-direction: column; gap: 12px;
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.5);
  backdrop-filter: blur(14px); -webkit-backdrop-filter: blur(14px);
}
.controller-layout {
  display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 12px;
}

/* CIRCULAR DPAD & ARROW PADS */
.fs-dpad-circle, .mobile-dpad-circle {
  width: 140px; height: 140px; border-radius: 50%;
  background: radial-gradient(circle, rgba(16, 24, 39, 0.9) 0%, rgba(3, 7, 18, 0.98) 100%);
  border: 2px solid rgba(0, 240, 255, 0.35);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.7), inset 0 0 16px rgba(0, 240, 255, 0.12);
  backdrop-filter: blur(16px); position: relative; display: flex; align-items: center; justify-content: center;
  pointer-events: auto; user-select: none; -webkit-user-select: none; touch-action: none;
}
.fs-dpad-center {
  width: 40px; height: 40px; border-radius: 50%;
  background: radial-gradient(circle, rgba(0, 240, 255, 0.25) 0%, rgba(15, 23, 42, 0.9) 100%);
  border: 1.5px solid rgba(0, 240, 255, 0.45);
  box-shadow: 0 0 12px rgba(0, 240, 255, 0.35);
  display: flex; align-items: center; justify-content: center; font-size: 1.1rem;
  pointer-events: none;
}
.fs-dpad-btn-w {
  position: absolute; top: 5px; left: 50%; transform: translateX(-50%);
  width: 44px; height: 40px; border-radius: 12px 12px 6px 6px;
}
.fs-dpad-btn-s {
  position: absolute; bottom: 5px; left: 50%; transform: translateX(-50%);
  width: 44px; height: 40px; border-radius: 6px 6px 12px 12px;
}
.fs-dpad-btn-a {
  position: absolute; left: 5px; top: 50%; transform: translateY(-50%);
  width: 40px; height: 44px; border-radius: 12px 6px 6px 12px;
}
.fs-dpad-btn-d {
  position: absolute; right: 5px; top: 50%; transform: translateY(-50%);
  width: 40px; height: 44px; border-radius: 6px 12px 12px 6px;
}

.fs-arrow-circle, .mobile-arrow-circle {
  width: 140px; height: 140px; border-radius: 50%;
  background: radial-gradient(circle, rgba(16, 24, 39, 0.9) 0%, rgba(3, 7, 18, 0.98) 100%);
  border: 2px solid rgba(59, 130, 246, 0.4);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.7), inset 0 0 16px rgba(59, 130, 246, 0.15);
  backdrop-filter: blur(16px); position: relative; display: flex; align-items: center; justify-content: center;
  pointer-events: auto; user-select: none; -webkit-user-select: none; touch-action: none;
}
.fs-arrow-center {
  width: 40px; height: 40px; border-radius: 50%;
  background: radial-gradient(circle, rgba(59, 130, 246, 0.25) 0%, rgba(15, 23, 42, 0.9) 100%);
  border: 1.5px solid rgba(59, 130, 246, 0.45);
  box-shadow: 0 0 12px rgba(59, 130, 246, 0.35);
  display: flex; align-items: center; justify-content: center; font-size: 1.1rem;
  pointer-events: none;
}
.fs-arrow-btn-up {
  position: absolute; top: 5px; left: 50%; transform: translateX(-50%);
  width: 44px; height: 40px; border-radius: 12px 12px 6px 6px;
  color: #60a5fa !important; border-color: rgba(96, 165, 250, 0.35) !important; font-size: 1.1rem !important;
}
.fs-arrow-btn-down {
  position: absolute; bottom: 5px; left: 50%; transform: translateX(-50%);
  width: 44px; height: 40px; border-radius: 6px 6px 12px 12px;
  color: #60a5fa !important; border-color: rgba(96, 165, 250, 0.35) !important; font-size: 1.1rem !important;
}
.fs-arrow-btn-left {
  position: absolute; left: 5px; top: 50%; transform: translateY(-50%);
  width: 40px; height: 44px; border-radius: 12px 6px 6px 12px;
  color: #60a5fa !important; border-color: rgba(96, 165, 250, 0.35) !important; font-size: 1.1rem !important;
}
.fs-arrow-btn-right {
  position: absolute; right: 5px; top: 50%; transform: translateY(-50%);
  width: 40px; height: 44px; border-radius: 6px 12px 12px 6px;
  color: #60a5fa !important; border-color: rgba(96, 165, 250, 0.35) !important; font-size: 1.1rem !important;
}

.fs-right-group {
  display: flex; gap: 10px; align-items: center; pointer-events: none; flex-wrap: wrap; justify-content: flex-end;
}
.fs-actions-column {
  display: flex; flex-direction: column; gap: 8px; align-items: flex-end; pointer-events: none;
}
.fs-round-action-btn {
  width: 44px; height: 44px; border-radius: 50% !important;
  display: flex; align-items: center; justify-content: center;
  font-size: 0.85rem; font-weight: 800;
  box-shadow: 0 4px 14px rgba(0, 0, 0, 0.5);
}
.fs-round-jump-btn {
  width: 52px; height: 52px; border-radius: 50% !important;
  background: linear-gradient(135deg, rgba(16, 185, 129, 0.45), rgba(5, 150, 105, 0.7)) !important;
  border: 2px solid var(--emerald) !important;
  box-shadow: 0 0 20px rgba(16, 185, 129, 0.45) !important;
  font-size: 0.95rem; font-weight: 800; color: #fff;
}
.fs-round-jump-btn:active, .fs-round-jump-btn.pressed {
  background: var(--emerald) !important; color: #000 !important;
  box-shadow: 0 0 30px var(--emerald) !important; transform: scale(0.94);
}

.dpad-btn {
  background: rgba(30, 41, 59, 0.75); border: 1.5px solid rgba(255, 255, 255, 0.15); border-radius: 12px; color: var(--text);
  font-weight: 800; font-size: 0.95rem; display: flex; align-items: center; justify-content: center;
  cursor: pointer; user-select: none; -webkit-user-select: none; touch-action: none;
  transition: all 0.12s cubic-bezier(0.2, 0.8, 0.4, 1); box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
}
.dpad-btn:active, .dpad-btn.pressed {
  background: var(--cyan); color: #000; box-shadow: 0 0 20px var(--cyan); transform: scale(0.94);
}
.pad-arrow-btn {
  font-size: 1.15rem; color: #60a5fa; border-color: rgba(96, 165, 250, 0.35); background: rgba(23, 37, 84, 0.5);
}
.pad-arrow-btn:active, .pad-arrow-btn.pressed {
  background: #3b82f6 !important; color: #fff !important; box-shadow: 0 0 20px #3b82f6 !important; transform: scale(0.94);
}

.pad-action-btn {
  padding: 10px 14px; background: rgba(30, 41, 59, 0.75); border: 1.5px solid rgba(255, 255, 255, 0.15); border-radius: 12px;
  color: var(--text); font-weight: 800; font-size: 0.8rem; cursor: pointer; user-select: none;
  touch-action: none; transition: all 0.12s cubic-bezier(0.2, 0.8, 0.4, 1);
  display: flex; align-items: center; justify-content: center; gap: 6px; box-shadow: 0 4px 14px rgba(0, 0, 0, 0.4);
}
.pad-action-btn:active, .pad-action-btn.pressed {
  background: var(--purple); color: #fff; box-shadow: 0 0 20px var(--purple); transform: scale(0.94);
}
.pad-action-btn.btn-jump {
  border-color: rgba(16, 185, 129, 0.4); color: #a7f3d0; background: rgba(6, 78, 59, 0.4);
}
.pad-action-btn.btn-jump:active, .pad-action-btn.btn-jump.pressed {
  background: var(--emerald); color: #000; box-shadow: 0 0 20px var(--emerald);
}
.btn-shift { border-color: rgba(176, 38, 255, 0.4); color: #d8b4fe; }

/* ACTION BUTTONS & GRIDS */
.action-grid { display: grid; grid-template-columns: repeat(2, 1fr); gap: 10px; }
.btn {
  padding: 12px 14px; border-radius: 12px; border: 1px solid transparent; font-size: 0.85rem; font-weight: 700;
  cursor: pointer; display: flex; align-items: center; justify-content: center; gap: 6px; transition: all 0.15s ease;
  user-select: none;
}
.btn:active { transform: scale(0.96); }
.btn-toggle { background: linear-gradient(135deg, #00d2ff, #0084ff); color: #fff; box-shadow: 0 4px 18px rgba(0, 140, 255, 0.35); }
.btn-toggle.paused { background: linear-gradient(135deg, #f59e0b, #d97706); box-shadow: 0 4px 18px rgba(245, 158, 11, 0.35); }
.btn-sub { background: rgba(30, 41, 59, 0.6); color: var(--text); border-color: var(--card-border); }
.btn-update { background: rgba(176, 38, 255, 0.15); color: #d8b4fe; border-color: rgba(176, 38, 255, 0.4); }

/* KEYBOARD LIGHT BUTTONS */
.kbd-btn {
  padding: 10px 4px; border-radius: 10px; border: 1.5px solid rgba(255,255,255,0.12);
  background: rgba(30, 41, 59, 0.7); color: var(--text); font-weight: 700; font-size: 0.8rem;
  cursor: pointer; display: flex; align-items: center; justify-content: center; gap: 4px;
  transition: all 0.15s ease; user-select: none;
}
.kbd-btn:active { transform: scale(0.95); }
.kbd-btn.active {
  border-color: #a855f7 !important;
  background: linear-gradient(135deg, rgba(168, 85, 247, 0.3), rgba(147, 51, 234, 0.45)) !important;
  color: #fff !important; box-shadow: 0 0 14px rgba(168, 85, 247, 0.45);
}

/* SLIDERS */
.sliders-grid { display: flex; flex-direction: column; gap: 12px; }
.slider-group { display: flex; flex-direction: column; gap: 8px; }
.slider-head { display: flex; justify-content: space-between; font-size: 0.8rem; font-weight: 700; }
.slider-val { font-family: monospace; color: var(--cyan); }
input[type=range] {
  -webkit-appearance: none; width: 100%; height: 8px; border-radius: 4px; background: #1e293b; outline: none; cursor: pointer;
}
input[type=range]::-webkit-slider-thumb {
  -webkit-appearance: none; width: 22px; height: 22px; border-radius: 50%; background: var(--cyan); box-shadow: 0 0 12px var(--cyan); cursor: pointer; transition: transform 0.1s;
}
input[type=range]::-webkit-slider-thumb:active { transform: scale(1.25); }

/* BOSS TIMERS */
.boss-grid { display: grid; grid-template-columns: 1fr; gap: 8px; }
.boss-card {
  background: rgba(16, 22, 34, 0.7); border: 1px solid var(--card-border); border-radius: 12px; padding: 10px 12px;
  display: flex; justify-content: space-between; align-items: center;
}
.boss-title { font-size: 0.82rem; font-weight: 700; display: flex; align-items: center; gap: 8px; }
.boss-countdown { font-size: 0.84rem; font-weight: 800; font-family: monospace; color: var(--cyan); }
.boss-spawned { color: var(--emerald); text-shadow: 0 0 10px rgba(16, 185, 129, 0.6); animation: pulseSpawn 1.5s infinite; }
.boss-soon { color: var(--amber); }
@keyframes pulseSpawn { 0%, 100% { opacity: 1; } 50% { opacity: 0.4; } }

/* CUSTOM DROPDOWNS */
.custom-dropdown { position: relative; display: inline-block; flex: 1; min-width: 140px; }
.custom-dropdown-btn {
  width: 100%; height: 42px; padding: 0 12px; background: #14151a; border: 1px solid rgba(255, 255, 255, 0.15);
  border-radius: 10px; color: var(--text); font-size: 0.82rem; font-weight: 700;
  display: flex; align-items: center; justify-content: space-between; gap: 6px; cursor: pointer;
}
.custom-dropdown.open .custom-dropdown-btn { border-color: var(--cyan); box-shadow: 0 0 12px rgba(0, 240, 255, 0.3); }
.custom-dropdown.open .dropdown-chevron { transform: rotate(180deg); }
.dropdown-chevron { font-size: 0.7rem; color: var(--text-dim); transition: transform 0.2s ease; }
.custom-dropdown-menu {
  display: none; position: absolute; top: calc(100% + 4px); left: 0; min-width: 100%; max-width: 320px;
  max-height: 240px; overflow-y: auto; background: #14151a; border: 1px solid rgba(255, 255, 255, 0.18);
  border-radius: 12px; box-shadow: 0 16px 40px rgba(0, 0, 0, 0.8); z-index: 1000; padding: 6px;
}
.custom-dropdown.open .custom-dropdown-menu { display: flex; flex-direction: column; gap: 3px; }
.custom-dropdown-item {
  padding: 8px 10px; border-radius: 8px; font-size: 0.8rem; color: var(--text-dim); cursor: pointer;
  display: flex; align-items: center; justify-content: space-between; gap: 6px;
}
.custom-dropdown-item.active { background: rgba(0, 240, 255, 0.15); color: var(--cyan); font-weight: 700; }
.dropdown-item-sub { font-size: 0.68rem; color: var(--text-mute); font-family: monospace; }

/* MACRO PILLS & INSPECTOR */
.macro-pill-group { display: flex; gap: 6px; flex-wrap: wrap; align-items: center; }
.macro-pill-btn {
  background: rgba(30, 41, 59, 0.7); border: 1px solid var(--card-border); color: var(--text-dim);
  padding: 6px 10px; border-radius: 8px; font-size: 0.74rem; font-weight: 700; cursor: pointer;
}
.macro-pill-btn.active {
  background: rgba(0, 240, 255, 0.16); border-color: var(--cyan); color: var(--cyan); box-shadow: 0 0 12px rgba(0, 240, 255, 0.3);
}
.macro-pill-btn.active-purple {
  background: rgba(176, 38, 255, 0.18); border-color: #c084fc; color: #e9d5ff; box-shadow: 0 0 12px rgba(176, 38, 255, 0.3);
}
.macro-hotkey-box {
  background: rgba(0, 240, 255, 0.05); border: 1px dashed rgba(0, 240, 255, 0.3);
  border-radius: 8px; padding: 8px 10px; font-size: 0.72rem; color: #94a3b8; display: flex; align-items: center; gap: 8px;
}
.macro-steps-box {
  background: rgba(3, 5, 8, 0.7); border: 1px solid rgba(255,255,255,0.08);
  border-radius: 10px; padding: 8px; max-height: 180px; overflow-y: auto;
  font-family: monospace; font-size: 0.7rem; display: flex; flex-direction: column; gap: 4px;
}
.macro-step-row {
  display: flex; justify-content: space-between; align-items: center;
  padding: 4px 6px; border-radius: 6px; background: rgba(255,255,255,0.02);
}

/* FIXED BOTTOM APP TAB BAR */
.bottom-nav-bar {
  position: fixed; bottom: 0; left: 0; right: 0; z-index: 1000;
  height: calc(58px + var(--safe-bottom));
  padding-bottom: var(--safe-bottom);
  background: rgba(10, 14, 23, 0.94);
  border-top: 1px solid rgba(255, 255, 255, 0.09);
  backdrop-filter: blur(24px); -webkit-backdrop-filter: blur(24px);
  display: flex; justify-content: space-around; align-items: center;
}
.bottom-nav-item {
  flex: 1; height: 100%; border: none; background: transparent;
  display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 3px;
  color: var(--text-mute); cursor: pointer; transition: all 0.15s ease; position: relative;
}
.bottom-nav-item .nav-icon { font-size: 1.25rem; transition: transform 0.15s ease; }
.bottom-nav-item .nav-label { font-size: 0.65rem; font-weight: 700; letter-spacing: 0.2px; }
.bottom-nav-item.active { color: var(--cyan); }
.bottom-nav-item.active .nav-icon { transform: scale(1.12); filter: drop-shadow(0 0 8px rgba(0, 240, 255, 0.4)); }
.bottom-nav-item.active::after {
  content: ''; position: absolute; top: 0; width: 36px; height: 2px;
  background: var(--cyan); border-radius: 999px; box-shadow: 0 0 10px var(--cyan);
}
.bottom-nav-item:active { transform: scale(0.92); }

/* TOAST */
#toast {
  position: fixed; bottom: calc(68px + var(--safe-bottom)); left: 50%; transform: translateX(-50%) translateY(100px);
  background: rgba(16, 22, 34, 0.96); border: 1px solid var(--cyan); color: var(--text);
  padding: 8px 18px; border-radius: 999px; font-size: 0.8rem; font-weight: 700;
  box-shadow: 0 10px 30px rgba(0,0,0,0.8), 0 0 18px rgba(0,240,255,0.25);
  transition: transform 0.26s cubic-bezier(0.18, 0.89, 0.32, 1.28);
  pointer-events: none; z-index: 10000; white-space: nowrap;
}
#toast.show { transform: translateX(-50%) translateY(0); }
</style>
</head>
<body>
<div class="app-shell">
  <!-- TOP APP HEADER -->
  <header class="app-header" id="app-header">
    <div class="brand">
      <div class="brand-avatar">⚡</div>
      <div class="brand-text">
        <div class="brand-title">GPO CYBERDECK</div>
        <div class="brand-sub" id="host-sub">CONNECTING...</div>
      </div>
    </div>
    <div class="header-actions">
      <button id="btn-header-toggle" class="header-btn-toggle" onclick="togglePlay()">
        <span>▶</span><span>START</span>
      </button>
      <div id="status-pill" class="status-badge badge-stopped">STOPPED</div>
      <div id="badge-fruit" class="toggle-spawn-badge" onclick="toggleSpawnAlerts()" title="Toggle fruit spawn alerts">🍇 ALERTS</div>
    </div>
  </header>

  <!-- MAIN SCROLLABLE APP CONTENT -->
  <main class="app-content">

    <!-- TAB 1: REMOTE CONTROLS & LIVE STREAM -->
    <section id="sec-remote" class="mobile-section active">
      <!-- LIVE VIDEO STREAM -->
      <div class="stream-wrapper" id="stream-wrapper">
        <div class="stream-bar" id="stream-bar">
          <div class="stream-indicator">
            <div id="stream-dot" class="live-dot on"></div>
            <span style="font-weight: 800;">LIVE</span>
            <span id="stream-fps-badge" style="font-family: monospace; color: var(--cyan); font-size: 0.7rem; font-weight: 800;">20 FPS</span>
            <span id="stream-res-badge" style="font-family: monospace; color: var(--amber); font-size: 0.7rem; font-weight: 800;">720p</span>
          </div>

          <div class="stream-tools-group">
            <!-- FPS Selector Dropdown -->
            <div class="custom-dropdown" id="dropdown-fps" style="min-width: 86px; flex: initial;">
              <button type="button" class="stream-tool-btn" onclick="toggleDropdown('dropdown-fps')" title="Change Frame Rate">
                <span id="dropdown-fps-label">⚡ 20 FPS</span>
                <span class="dropdown-chevron">▼</span>
              </button>
              <div class="custom-dropdown-menu" id="dropdown-fps-menu" style="min-width: 130px;">
                <div class="custom-dropdown-item" data-val="10" onclick="setStreamFps(10)">
                  <span>10 FPS (Eco)</span>
                </div>
                <div class="custom-dropdown-item" data-val="15" onclick="setStreamFps(15)">
                  <span>15 FPS</span>
                </div>
                <div class="custom-dropdown-item active" data-val="20" onclick="setStreamFps(20)">
                  <span>20 FPS (Default)</span>
                  <span class="item-check" style="color:var(--cyan);font-weight:800;">✓</span>
                </div>
                <div class="custom-dropdown-item" data-val="30" onclick="setStreamFps(30)">
                  <span>30 FPS (Smooth)</span>
                </div>
                <div class="custom-dropdown-item" data-val="60" onclick="setStreamFps(60)">
                  <span>60 FPS (Ultra)</span>
                </div>
              </div>
            </div>

            <!-- Quality Selector Dropdown -->
            <div class="custom-dropdown" id="dropdown-quality" style="min-width: 86px; flex: initial;">
              <button type="button" class="stream-tool-btn" onclick="toggleDropdown('dropdown-quality')" title="Change Resolution">
                <span id="dropdown-quality-label">📺 720p</span>
                <span class="dropdown-chevron">▼</span>
              </button>
              <div class="custom-dropdown-menu" id="dropdown-quality-menu" style="min-width: 150px;">
                <div class="custom-dropdown-item" data-val="480" onclick="setStreamQuality(480, 50, '480p (Low)')">
                  <div style="flex:1;text-align:left;">
                    <div style="font-weight:700;">480p (Low)</div>
                    <div class="dropdown-item-sub">Data Saver</div>
                  </div>
                </div>
                <div class="custom-dropdown-item active" data-val="720" onclick="setStreamQuality(720, 70, '720p (Balanced)')">
                  <div style="flex:1;text-align:left;">
                    <div style="font-weight:700;">720p (Balanced)</div>
                    <div class="dropdown-item-sub">Default HD</div>
                  </div>
                  <span class="item-check" style="color:var(--cyan);font-weight:800;">✓</span>
                </div>
                <div class="custom-dropdown-item" data-val="1080" onclick="setStreamQuality(1080, 85, '1080p (Crisp)')">
                  <div style="flex:1;text-align:left;">
                    <div style="font-weight:700;">1080p (Crisp)</div>
                    <div class="dropdown-item-sub">High Detail</div>
                  </div>
                </div>
                <div class="custom-dropdown-item" data-val="0" onclick="setStreamQuality(0, 92, 'Original (Max)')">
                  <div style="flex:1;text-align:left;">
                    <div style="font-weight:700;">Original (Max)</div>
                    <div class="dropdown-item-sub">Native Display</div>
                  </div>
                </div>
              </div>
            </div>

            <!-- Hold & Drag Toggle Button -->
            <button type="button" class="stream-tool-btn active" id="stream-tool-drag" onclick="toggleHoldDragMode()" title="Toggle Touch Hold & Drag vs Tap">
              <span id="stream-drag-label">🖐️ DRAG</span>
            </button>

            <!-- Rotate Button -->
            <button type="button" class="stream-tool-btn" id="stream-tool-rotate" onclick="toggleRotate()" title="Rotate 90° Landscape">
              <span>🔄</span>
            </button>

            <!-- Fullscreen Button -->
            <button type="button" class="stream-tool-btn" onclick="toggleFullscreen()" title="Full Screen View" style="background: linear-gradient(135deg, rgba(0, 240, 255, 0.25), rgba(176, 38, 255, 0.25)); border-color: var(--cyan); color: #fff; font-weight: 800;">
              <span>⛶</span>
            </button>
          </div>
        </div>

        <div id="screen-container" class="screen-box" oncontextmenu="return false;">
          <img id="screen-img" class="screen-img" src="/api/stream" alt="" draggable="false" oncontextmenu="return false;" onerror="fallbackSnapshot()" />

          <!-- Fullscreen Floating Bar -->
          <div id="fs-floating-bar" class="fs-floating-bar">
            <div class="fs-badge-group">
              <div id="fs-status-pill" class="status-badge badge-stopped" style="font-size: 0.68rem; padding: 4px 10px;">STOPPED</div>
              <div id="fs-stream-info" class="fs-stream-pill">20 FPS • 720p</div>
            </div>
            <div style="display: flex; gap: 6px; align-items: center; pointer-events: auto;">
              <button class="fs-btn active" id="btn-fs-drag" onclick="toggleHoldDragMode()">
                <span id="fs-drag-icon">🖐️</span>
                <span id="fs-drag-label">DRAG</span>
              </button>
              <button class="fs-btn" id="btn-click-mode" onclick="toggleClickMode()">
                <span id="click-mode-icon">🎯</span>
                <span id="click-mode-label">CLICK</span>
              </button>
              <button class="fs-btn" id="btn-fs-rotate" onclick="toggleRotate()">
                <span>🔄</span>
              </button>
              <button class="fs-btn" id="btn-fs-overlay-toggle" onclick="toggleFullscreenControls()">
                <span id="fs-ctrl-icon">🎮</span>
                <span id="fs-ctrl-label">PAD</span>
              </button>
              <button class="fs-btn fs-btn-close" onclick="exitFullscreen()">
                <span>✖</span>
              </button>
            </div>
          </div>

          <!-- Fullscreen Controls Overlay -->
          <div id="fs-controls-overlay" class="fs-controls-overlay visible">
            <div class="fs-quick-bar">
              <button class="fs-mini-btn" id="btn-fs-toggle" onclick="togglePlay()">▶ START</button>
              <button class="fs-mini-btn" onclick="doAction('recast')">🔄 RECAST</button>
              <button class="fs-mini-btn" onclick="doAction('buy_bait')">🛒 BUY BAIT</button>
              <button class="fs-mini-btn" id="btn-fs-mute" onclick="toggleMute()">🔇 MUTE</button>
            </div>

            <div class="fs-bottom-controls">
              <!-- Left: Circular D-Pad Joystick -->
              <div class="fs-pad-cluster">
                <div class="fs-cluster-label">🏃 MOVEMENT (WASD)</div>
                <div class="fs-dpad-circle">
                  <div class="fs-dpad-center">🏃</div>
                  <button class="dpad-btn fs-pad-btn fs-dpad-btn-w" data-key="w">▲</button>
                  <button class="dpad-btn fs-pad-btn fs-dpad-btn-s" data-key="s">▼</button>
                  <button class="dpad-btn fs-pad-btn fs-dpad-btn-a" data-key="a">◀</button>
                  <button class="dpad-btn fs-pad-btn fs-dpad-btn-d" data-key="d">▶</button>
                </div>
              </div>

              <!-- Right: Camera & Actions -->
              <div class="fs-right-group">
                <div class="fs-pad-cluster" style="align-items: center;">
                  <div class="fs-cluster-label" style="color: #60a5fa;">👀 CAMERA</div>
                  <div class="fs-arrow-circle">
                    <div class="fs-arrow-center">📷</div>
                    <button class="dpad-btn fs-pad-btn pad-arrow-btn fs-arrow-btn-up" data-key="up">▲</button>
                    <button class="dpad-btn fs-pad-btn pad-arrow-btn fs-arrow-btn-down" data-key="down">▼</button>
                    <button class="dpad-btn fs-pad-btn pad-arrow-btn fs-arrow-btn-left" data-key="left">◀</button>
                    <button class="dpad-btn fs-pad-btn pad-arrow-btn fs-arrow-btn-right" data-key="right">▶</button>
                  </div>
                </div>

                <div class="fs-pad-cluster" style="align-items: flex-end;">
                  <div class="fs-cluster-label">⚡ ACTIONS</div>
                  <div class="fs-actions-column">
                    <div style="display: flex; gap: 6px; align-items: center;">
                      <button class="pad-action-btn fs-pad-btn btn-shift" data-key="shift" style="border-radius: 999px !important; padding: 6px 12px; font-size: 0.72rem;">⚡ SHIFT</button>
                      <button class="pad-action-btn fs-pad-btn fs-round-action-btn" data-key="t" title="Chat (T)">💬</button>
                    </div>
                    <div style="display: flex; gap: 6px; align-items: center;">
                      <button class="pad-action-btn fs-pad-btn fs-round-action-btn" data-key="1" title="Rod (1)">🎣</button>
                      <button class="pad-action-btn fs-pad-btn fs-round-action-btn" data-key="e" title="Interact (E)">🖐️</button>
                      <button class="pad-action-btn fs-pad-btn fs-round-jump-btn" data-key="space" title="Jump (Space)">🦘</button>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- MOBILE DUAL-THUMB GAMEPAD -->
      <div class="controller-card">
        <div class="controller-layout">
          <!-- Left: Circular WASD Joystick -->
          <div class="pad-cluster">
            <div class="cluster-label">🏃 WALK (WASD)</div>
            <div class="mobile-dpad-circle">
              <div class="fs-dpad-center">🏃</div>
              <button class="dpad-btn fs-pad-btn fs-dpad-btn-w" data-key="w">▲</button>
              <button class="dpad-btn fs-pad-btn fs-dpad-btn-s" data-key="s">▼</button>
              <button class="dpad-btn fs-pad-btn fs-dpad-btn-a" data-key="a">◀</button>
              <button class="dpad-btn fs-pad-btn fs-dpad-btn-d" data-key="d">▶</button>
            </div>
          </div>

          <!-- Right: Circular Camera Pad + Quick Action Cluster -->
          <div class="fs-right-group">
            <div class="pad-cluster" style="align-items: center;">
              <div class="cluster-label" style="color: #60a5fa;">👀 CAMERA</div>
              <div class="mobile-arrow-circle">
                <div class="fs-arrow-center">📷</div>
                <button class="dpad-btn fs-pad-btn pad-arrow-btn fs-arrow-btn-up" data-key="up">▲</button>
                <button class="dpad-btn fs-pad-btn pad-arrow-btn fs-arrow-btn-down" data-key="down">▼</button>
                <button class="dpad-btn fs-pad-btn pad-arrow-btn fs-arrow-btn-left" data-key="left">◀</button>
                <button class="dpad-btn fs-pad-btn pad-arrow-btn fs-arrow-btn-right" data-key="right">▶</button>
              </div>
            </div>

            <div class="pad-cluster" style="align-items: flex-end;">
              <div class="cluster-label">⚡ ACTIONS</div>
              <div class="fs-actions-column">
                <div style="display: flex; gap: 6px; align-items: center;">
                  <button class="pad-action-btn fs-pad-btn btn-shift" data-key="shift" style="border-radius: 999px !important; padding: 6px 12px; font-size: 0.72rem;">⚡ SHIFT</button>
                  <button class="pad-action-btn fs-pad-btn fs-round-action-btn" data-key="t" title="Chat (T)">💬</button>
                </div>
                <div style="display: flex; gap: 6px; align-items: center;">
                  <button class="pad-action-btn fs-pad-btn fs-round-action-btn" data-key="1" title="Rod (1)">🎣</button>
                  <button class="pad-action-btn fs-pad-btn fs-round-action-btn" data-key="e" title="Interact (E)">🖐️</button>
                  <button class="pad-action-btn fs-pad-btn fs-round-jump-btn" data-key="space" title="Jump (Space)">🦘</button>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- QUICK MACRO ACTION BUTTONS -->
      <div class="action-grid">
        <button id="btn-toggle" class="btn btn-toggle" onclick="togglePlay()" style="grid-column: span 2; padding: 14px;">
          <span id="toggle-icon">▶️</span>
          <span id="toggle-label">START MACRO</span>
        </button>
        <button class="btn btn-sub" onclick="doAction('recast')">🔄 RECAST</button>
        <button class="btn btn-sub" onclick="doAction('buy_bait')">🛒 BUY BAIT</button>
        <button id="btn-mute" class="btn btn-sub" onclick="toggleMute()">🔇 MUTE</button>
        <button class="btn btn-update" onclick="doUpdate()">🚀 UPDATE</button>
      </div>
    </section>

    <!-- TAB 2: STATS & DASHBOARD -->
    <section id="sec-stats" class="mobile-section">
      <!-- PRIMARY CATCH METRICS -->
      <div class="stat-grid">
        <div class="card">
          <div class="card-label">🐟 Fish Caught</div>
          <div class="card-val" id="val-fish">0</div>
          <div class="card-meta" id="val-rate">0% rate</div>
        </div>
        <div class="card">
          <div class="card-label">🍇 Devil Fruits</div>
          <div class="card-val fruit" id="val-fruits">0</div>
          <div class="card-meta" id="val-pity">Pity: 0</div>
        </div>
        <div class="card">
          <div class="card-label">🌟 Legendary Pity</div>
          <div class="card-val" id="val-leg-pity" style="color: var(--amber);">0</div>
          <div class="gauge-bar-bg"><div class="gauge-bar-fill" id="bar-leg-pity"></div></div>
          <div class="card-meta">Toward 100 guaranteed</div>
        </div>
        <div class="card">
          <div class="card-label">⏱️ Runtime</div>
          <div class="card-val" id="val-runtime" style="font-size: 1.25rem;">00:00:00</div>
          <div class="card-meta">Current session</div>
        </div>
      </div>

      <!-- BAIT & RESTOCK METRICS -->
      <div class="stat-grid">
        <div class="card">
          <div class="card-label">🛒 Orders Placed</div>
          <div class="card-val" id="val-orders">0</div>
          <div class="card-meta" id="val-auto-buy">Auto-buy ON</div>
        </div>
        <div class="card">
          <div class="card-label">⏳ Next Restock</div>
          <div class="card-val" id="val-restock" style="color: var(--cyan); font-size: 1.25rem;">0 / 10</div>
          <div class="gauge-bar-bg"><div class="gauge-bar-fill" id="bar-restock" style="background:var(--cyan)"></div></div>
          <div class="card-meta">Catches until restock</div>
        </div>
        <div class="card">
          <div class="card-label">🎯 Active Tier</div>
          <div class="card-val" id="val-tier" style="color: #60a5fa; font-size: 1.15rem;">Common</div>
          <div class="card-meta">Purchase priority</div>
        </div>
        <div class="card">
          <div class="card-label">🛡️ Leg. Reserve</div>
          <div class="card-val" id="val-reserve" style="color: var(--amber); font-size: 1.15rem;">0</div>
          <div class="card-meta">Protected stock</div>
        </div>
      </div>
    </section>

    <!-- TAB 3: STEP RECORDER & MACRO STUDIO -->
    <section id="sec-macro" class="mobile-section">
      <div class="card" style="border-color: rgba(0, 240, 255, 0.4); background: rgba(0, 240, 255, 0.03);">
        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 6px;">
          <div class="card-label" style="color: var(--cyan); display: flex; align-items: center; gap: 6px;">
            <span>📼 MACRO RECORDER &amp; STUDIO</span>
          </div>
          <div id="macro-status-badge" class="status-badge badge-stopped" style="font-size: 0.7rem; padding: 3px 8px;">IDLE</div>
        </div>

        <!-- 1. RECORD CONTROLS -->
        <div style="background: rgba(15, 23, 42, 0.6); border: 1px solid var(--card-border); border-radius: 12px; padding: 10px; display: flex; flex-direction: column; gap: 8px;">
          <div style="display: flex; justify-content: space-between; align-items: center;">
            <span style="font-size: 0.75rem; font-weight: 800; color: var(--text-dim);">1. RECORD WORKFLOW</span>
            <span id="record-count-badge" style="font-size: 0.75rem; font-family: monospace; color: var(--amber); font-weight: 800;">READY</span>
          </div>
          <div style="display: flex; gap: 8px; flex-wrap: wrap;">
            <input id="txt-macro-name" type="text" placeholder="Macro Name" value="Craft Rare Bait"
                   style="background: #14151a; color: #fff; border: 1px solid var(--card-border); border-radius: 10px; padding: 8px 12px; font-size: 0.85rem; font-weight: 700; flex: 1; min-width: 140px; outline: none;" />
            <button id="btn-record-toggle" class="btn" style="background: linear-gradient(135deg, #00f0ff, #0284c7); color: #000; flex: 1; min-width: 130px; padding: 10px 12px;" onclick="toggleRecord()">
              <span id="record-btn-icon">⏺️</span>
              <span id="record-btn-label">RECORD</span>
            </button>
            <button id="btn-record-cancel" class="btn btn-sub" style="display: none; padding: 10px 12px;" onclick="cancelRecord()">❌ CANCEL</button>
          </div>
          <div id="record-hint" style="font-size: 0.72rem; color: var(--text-mute);">
            Tap Record, then tap live screen and press controls (T, E, WASD). Timing is captured!
          </div>
        </div>

        <!-- 2. PLAYBACK CONTROLS -->
        <div style="background: rgba(15, 23, 42, 0.6); border: 1px solid var(--card-border); border-radius: 12px; padding: 10px; display: flex; flex-direction: column; gap: 8px; margin-top: 4px;">
          <div style="display: flex; justify-content: space-between; align-items: center;">
            <span style="font-size: 0.75rem; font-weight: 800; color: var(--text-dim);">2. PLAY / LOOP MACRO</span>
            <span id="play-loop-badge" style="font-size: 0.75rem; font-family: monospace; color: var(--cyan); font-weight: 800;">READY</span>
          </div>

          <div style="display: flex; gap: 8px; align-items: center;">
            <div class="custom-dropdown" id="dropdown-macro" style="flex: 1;">
              <button type="button" class="custom-dropdown-btn" onclick="toggleDropdown('dropdown-macro')">
                <span class="dropdown-label" id="dropdown-macro-label">(No macros saved)</span>
                <span class="dropdown-chevron">▼</span>
              </button>
              <div class="custom-dropdown-menu" id="dropdown-macro-menu">
                <div class="custom-dropdown-item" style="color:var(--text-mute);cursor:default;">(No macros saved yet)</div>
              </div>
            </div>
            <button class="btn btn-sub" style="padding: 10px 12px; font-size: 0.8rem;" onclick="renameSelectedMacro()" title="Rename">✏️</button>
            <button class="btn btn-sub" style="padding: 10px 12px; font-size: 0.8rem;" onclick="deleteSelectedMacro()" title="Delete">🗑️</button>
          </div>

          <!-- Loop Repetition Pills -->
          <div style="display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 6px; padding-top: 2px;">
            <span style="font-size: 0.72rem; font-weight: 700; color: var(--text-dim);">🔁 Loop Count:</span>
            <div class="macro-pill-group" id="loop-pills">
              <button class="macro-pill-btn active-purple" onclick="setWebLoopCount(1, this)">1x</button>
              <button class="macro-pill-btn" onclick="setWebLoopCount(5, this)">5x</button>
              <button class="macro-pill-btn" onclick="setWebLoopCount(10, this)">10x</button>
              <button class="macro-pill-btn" onclick="setWebLoopCount(25, this)">25x</button>
              <button class="macro-pill-btn" onclick="setWebLoopCount(0, this)">∞ Endless</button>
            </div>
          </div>

          <!-- Playback Speed Pills -->
          <div style="display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 6px;">
            <span style="font-size: 0.72rem; font-weight: 700; color: var(--text-dim);">⚡ Speed:</span>
            <div class="macro-pill-group" id="speed-pills">
              <button class="macro-pill-btn active" onclick="setWebSpeed(1.0, this)">1x</button>
              <button class="macro-pill-btn" onclick="setWebSpeed(2.0, this)">2x</button>
              <button class="macro-pill-btn" onclick="setWebSpeed(5.0, this)">5x</button>
              <button class="macro-pill-btn" onclick="setWebSpeed(10.0, this)">10x</button>
              <button class="macro-pill-btn" onclick="setWebSpeed(25.0, this)">25x</button>
              <button class="macro-pill-btn" onclick="setWebSpeed(50.0, this)">50x</button>
              <button class="macro-pill-btn" onclick="setWebSpeed(100.0, this)">100x ⚡</button>
            </div>
          </div>

          <!-- Play / Loop / Stop Action Buttons -->
          <div style="display: flex; gap: 8px; flex-wrap: wrap; align-items: center; margin-top: 4px;">
            <button id="btn-macro-play" class="btn" style="background: linear-gradient(135deg, #10b981, #059669); color: #fff; flex: 1; padding: 10px 12px;" onclick="playMacro(false)">
              ▶️ PLAY ONCE
            </button>
            <button id="btn-macro-loop" class="btn" style="background: linear-gradient(135deg, #b026ff, #7c3aed); color: #fff; flex: 1; padding: 10px 12px;" onclick="playMacro(true)">
              🔁 LOOP PLAY
            </button>
            <button id="btn-macro-stop" class="btn" style="background: linear-gradient(135deg, #ef4444, #dc2626); color: #fff; flex: 1; padding: 10px 14px; display: none;" onclick="stopMacro()">
              🛑 STOP PLAYBACK
            </button>
          </div>

          <!-- Step Inspector -->
          <details id="macro-steps-details" style="margin-top: 4px;" open>
            <summary style="font-size: 0.74rem; font-weight: 700; color: var(--cyan); cursor: pointer; user-select: none;">
              🎞️ Step Inspector (<span id="macro-steps-count">0</span> steps)
            </summary>
            <div id="macro-steps-list" class="macro-steps-box" style="margin-top: 6px;">
              <div style="color: var(--text-mute);">Select a macro to inspect steps...</div>
            </div>
          </details>
        </div>
        <div id="macro-msg" style="font-size: 0.72rem; color: var(--text-mute); margin-top: 4px;">Record any workflow once and replay or loop it smoothly!</div>
      </div>
    </section>

    <!-- TAB 4: AUTO CRAFT BAIT -->
    <section id="sec-craft" class="mobile-section">
      <div class="card" style="border-color: rgba(245, 158, 11, 0.35); background: rgba(245, 158, 11, 0.04);">
        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px;">
          <div class="card-label" style="color: var(--amber);">🔨 AUTO CRAFT (BLACKSMITH SEN)</div>
          <div id="craft-status-badge" class="status-badge badge-stopped" style="font-size: 0.7rem; padding: 3px 8px;">IDLE</div>
        </div>
        <div style="display: flex; flex-direction: column; gap: 10px;">
          <!-- Custom Craft Dropdown -->
          <div class="custom-dropdown" id="dropdown-craft" style="width: 100%;">
            <button type="button" class="custom-dropdown-btn" onclick="toggleDropdown('dropdown-craft')">
              <span class="dropdown-label" id="dropdown-craft-label">🍇 Rare Fish Bait</span>
              <span class="dropdown-chevron">▼</span>
            </button>
            <div class="custom-dropdown-menu" id="dropdown-craft-menu">
              <div class="custom-dropdown-item active" data-val="rare" onclick="selectCraftTier('rare', '🍇 Rare Fish Bait')">
                <span>🍇 Rare Fish Bait</span>
                <span class="item-check" style="color:var(--cyan);font-weight:800;">✓</span>
              </div>
              <div class="custom-dropdown-item" data-val="legendary" onclick="selectCraftTier('legendary', '👑 Legendary Fish Bait')">
                <span>👑 Legendary Fish Bait</span>
              </div>
              <div class="custom-dropdown-item" data-val="all" onclick="selectCraftTier('all', '🌟 All (Legendary &amp; Rare)')">
                <span>🌟 All (Legendary &amp; Rare)</span>
              </div>
            </div>
          </div>

          <button id="btn-craft-toggle" class="btn" style="background: linear-gradient(135deg, #f59e0b, #d97706); color: #000; padding: 14px;" onclick="toggleAutoCraft()">
            <span id="craft-btn-icon">🔨</span>
            <span id="craft-btn-label">START AUTO CRAFT</span>
          </button>
        </div>
        <div id="craft-msg" style="font-size: 0.75rem; color: var(--text-mute); margin-top: 6px;">Stand at Blacksmith Sen with caught fish in inventory, then tap Start.</div>
      </div>
    </section>

    <!-- TAB 5: SYSTEM & SETTINGS -->
    <section id="sec-system" class="mobile-section">
      <!-- LAPTOP KEYBOARD LIGHT CONTROL -->
      <div class="card" style="border-color: rgba(168, 85, 247, 0.35); background: rgba(168, 85, 247, 0.04);">
        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 6px;">
          <div class="card-label" style="color: #c084fc; display: flex; align-items: center; gap: 6px;">
            <span>⌨️ KEYBOARD LIGHT</span>
            <span style="font-size: 0.65rem; padding: 2px 6px; border-radius: 4px; background: rgba(168,85,247,0.15); border: 1px solid rgba(168,85,247,0.3); color: #e9d5ff;">LENOVO LOQ</span>
          </div>
          <div id="kbd-light-badge" class="status-badge" style="font-size: 0.7rem; padding: 3px 8px; background: rgba(255,255,255,0.08); color: #cbd5e1; border: 1px solid rgba(255,255,255,0.15);">UNKNOWN</div>
        </div>
        <div style="display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px;">
          <button id="btn-kbd-off" class="kbd-btn" onclick="setKbdLight('off')">🌑 OFF</button>
          <button id="btn-kbd-low" class="kbd-btn" onclick="setKbdLight('low')">🔅 LOW</button>
          <button id="btn-kbd-high" class="kbd-btn" onclick="setKbdLight('high')">🔆 HIGH</button>
          <button id="btn-kbd-cycle" class="kbd-btn" style="border-color: rgba(168,85,247,0.4);" onclick="setKbdLight('cycle')">🔄 CYCLE</button>
        </div>
      </div>

      <!-- AUDIO & SCREEN CONTROLS -->
      <div class="card">
        <div class="sliders-grid">
          <div class="slider-group">
            <div class="slider-head">
              <span>🔊 AUDIO VOLUME</span>
              <span id="lbl-volume" class="slider-val">50%</span>
            </div>
            <input id="rng-volume" type="range" min="0" max="100" value="50"
                   oninput="onVolInput(this.value)"
                   onchange="onVolRelease(this.value)"
                   onpointerup="onVolRelease(this.value)"
                   ontouchend="onVolRelease(this.value)" />
          </div>

          <div class="slider-group">
            <div class="slider-head">
              <span>💡 SCREEN BRIGHTNESS</span>
              <span id="lbl-brightness" class="slider-val">80%</span>
            </div>
            <input id="rng-brightness" type="range" min="0" max="100" value="80"
                   oninput="onBrightInput(this.value)"
                   onchange="onBrightRelease(this.value)"
                   onpointerup="onBrightRelease(this.value)"
                   ontouchend="onBrightRelease(this.value)" />
          </div>
        </div>
      </div>

      <!-- WORLD BOSS & TIMERS -->
      <div class="card">
        <div class="card-label" style="margin-bottom: 6px;">👑 WORLD BOSS &amp; MERCHANT COUNTDOWNS</div>
        <div id="boss-list" class="boss-grid">
          <div style="font-size: 0.8rem; color: var(--text-mute);">Loading timers...</div>
        </div>
      </div>

      <!-- MOBILE AUDIO ALERTS & PUSH NOTIFICATIONS -->
      <div class="card">
        <div class="card-label">🔔 MOBILE AUDIO ALERTS &amp; PUSH</div>
        <div style="display: flex; gap: 8px; flex-wrap: wrap;">
          <button id="btn-sound-toggle" class="btn" style="flex: 1; padding: 12px; background: linear-gradient(135deg, #10b981, #059669); color: #fff;" onclick="toggleSoundAlerts()">
            <span>🔊</span><span>SOUND: ON</span>
          </button>
          <button class="btn btn-sub" style="flex: 1; padding: 12px;" onclick="requestMobileNotifications()">
            <span>📲</span><span>ENABLE PUSH</span>
          </button>
          <button class="btn btn-sub" style="padding: 12px;" onclick="playFanfare('fruit')" title="Test sound">
            <span>🎵</span>
          </button>
        </div>
      </div>

      <!-- AUTO-RECONNECT & VIP SERVER LINK -->
      <div class="card">
        <div style="display: flex; justify-content: space-between; align-items: center;">
          <div class="card-label">🛡️ AUTO-RECONNECT &amp; VIP LINK</div>
          <button id="btn-reconnect-toggle" class="status-badge badge-running" style="cursor: pointer;" onclick="toggleAutoReconnect()">ACTIVE</button>
        </div>
        <div style="display: flex; gap: 8px;">
          <input id="txt-vip-url" type="text" placeholder="Roblox VIP / Private Server Link"
                 style="background: #14151a; color: #fff; border: 1px solid var(--card-border); border-radius: 10px; padding: 8px 12px; font-size: 0.8rem; font-weight: 600; flex: 1; outline: none;" />
          <button class="btn btn-sub" style="padding: 8px 14px;" onclick="saveVipUrl()">
            💾 SAVE
          </button>
        </div>
        <div style="font-size: 0.72rem; color: var(--text-mute);">When Roblox disconnects (Error 277/268), the macro clicks Reconnect or launches this VIP link!</div>
      </div>
    </section>

  </main>

  <!-- FIXED BOTTOM DOCK (MOBILE APP TAB BAR) -->
  <nav class="bottom-nav-bar" id="bottom-nav-bar">
    <button class="bottom-nav-item active" data-tab="remote" onclick="switchTab('remote', this)">
      <span class="nav-icon">🎮</span>
      <span class="nav-label">Remote</span>
    </button>
    <button class="bottom-nav-item" data-tab="stats" onclick="switchTab('stats', this)">
      <span class="nav-icon">📊</span>
      <span class="nav-label">Stats</span>
    </button>
    <button class="bottom-nav-item" data-tab="macro" onclick="switchTab('macro', this)">
      <span class="nav-icon">📼</span>
      <span class="nav-label">Studio</span>
    </button>
    <button class="bottom-nav-item" data-tab="craft" onclick="switchTab('craft', this)">
      <span class="nav-icon">🔨</span>
      <span class="nav-label">Craft</span>
    </button>
    <button class="bottom-nav-item" data-tab="system" onclick="switchTab('system', this)">
      <span class="nav-icon">⚙️</span>
      <span class="nav-label">System</span>
    </button>
  </nav>
</div>

<div id="toast"></div>

<script>
let isRunning = false;
let isPaused = false;
let isMuted = false;
let spawnAlerts = true;
let userSlidingVol = false;
let userSlidingBright = false;
let localRuntimeSec = 0;
let bossesState = [];
let serverTimeDelta = 0;

// Web Audio Synthesizer for Mobile Alarms & Fanfares
let audioCtx = null;
let soundEnabled = localStorage.getItem('gpo_sound_enabled') !== 'false';
let lastFruitsCount = null;
let lastBotState = null;

function initAudio() {
  if (!audioCtx) {
    const AudioContext = window.AudioContext || window.webkitAudioContext;
    if (AudioContext) audioCtx = new AudioContext();
  }
  if (audioCtx && audioCtx.state === 'suspended') {
    audioCtx.resume();
  }
}
document.addEventListener('pointerdown', initAudio, { once: true });
document.addEventListener('click', initAudio, { once: true });

function playFanfare(type) {
  if (!soundEnabled) return;
  initAudio();
  if (!audioCtx) return;
  const now = audioCtx.currentTime;

  if (type === 'fruit') {
    [523.25, 659.25, 783.99, 1046.50].forEach((freq, i) => {
      const osc = audioCtx.createOscillator();
      const gain = audioCtx.createGain();
      osc.type = 'triangle';
      osc.frequency.setValueAtTime(freq, now + i * 0.12);
      gain.gain.setValueAtTime(0.25, now + i * 0.12);
      gain.gain.exponentialRampToValueAtTime(0.001, now + i * 0.12 + 0.45);
      osc.connect(gain);
      gain.connect(audioCtx.destination);
      osc.start(now + i * 0.12);
      osc.stop(now + i * 0.12 + 0.5);
    });
    if (navigator.vibrate) navigator.vibrate([200, 100, 200, 100, 400]);
  } else if (type === 'boss') {
    [392.00, 523.25].forEach((freq, i) => {
      const osc = audioCtx.createOscillator();
      const gain = audioCtx.createGain();
      osc.type = 'sawtooth';
      osc.frequency.setValueAtTime(freq, now + i * 0.2);
      gain.gain.setValueAtTime(0.2, now + i * 0.2);
      gain.gain.exponentialRampToValueAtTime(0.001, now + i * 0.2 + 0.6);
      osc.connect(gain);
      gain.connect(audioCtx.destination);
      osc.start(now + i * 0.2);
      osc.stop(now + i * 0.2 + 0.65);
    });
    if (navigator.vibrate) navigator.vibrate([150, 80, 150]);
  } else if (type === 'warning') {
    [880, 440].forEach((freq, i) => {
      const osc = audioCtx.createOscillator();
      const gain = audioCtx.createGain();
      osc.type = 'square';
      osc.frequency.setValueAtTime(freq, now + i * 0.15);
      gain.gain.setValueAtTime(0.15, now + i * 0.15);
      gain.gain.exponentialRampToValueAtTime(0.001, now + i * 0.15 + 0.25);
      osc.connect(gain);
      gain.connect(audioCtx.destination);
      osc.start(now + i * 0.15);
      osc.stop(now + i * 0.15 + 0.3);
    });
    if (navigator.vibrate) navigator.vibrate([300, 100, 300]);
  } else if (type === 'reconnect') {
    [440, 660, 880].forEach((freq, i) => {
      const osc = audioCtx.createOscillator();
      const gain = audioCtx.createGain();
      osc.type = 'sine';
      osc.frequency.setValueAtTime(freq, now + i * 0.1);
      gain.gain.setValueAtTime(0.2, now + i * 0.1);
      gain.gain.exponentialRampToValueAtTime(0.001, now + i * 0.1 + 0.3);
      osc.connect(gain);
      gain.connect(audioCtx.destination);
      osc.start(now + i * 0.1);
      osc.stop(now + i * 0.1 + 0.35);
    });
    if (navigator.vibrate) navigator.vibrate([100, 50, 100]);
  }
}

function toggleSoundAlerts() {
  soundEnabled = !soundEnabled;
  localStorage.setItem('gpo_sound_enabled', soundEnabled);
  updateSoundButtonUi();
  if (soundEnabled) {
    playFanfare('fruit');
    showToast('Sound alarms ENABLED');
  } else {
    showToast('Sound alarms MUTED');
  }
}

function updateSoundButtonUi() {
  const btn = document.getElementById('btn-sound-toggle');
  if (btn) {
    if (soundEnabled) {
      btn.style.background = 'linear-gradient(135deg, #10b981, #059669)';
      btn.innerHTML = '<span>🔊</span><span>SOUND: ON</span>';
    } else {
      btn.style.background = 'rgba(100, 116, 139, 0.25)';
      btn.innerHTML = '<span>🔇</span><span>SOUND: OFF</span>';
    }
  }
}

function requestMobileNotifications() {
  if (!('Notification' in window)) {
    showToast('Notifications not supported in browser');
    return;
  }
  Notification.requestPermission().then(perm => {
    if (perm === 'granted') {
      showToast('Notifications ENABLED!');
      try {
        new Notification('GPO Autofish Alert', { body: 'Mobile notifications active for fruit drops & bosses!' });
      } catch (_) {}
    } else {
      showToast('Notifications permission: ' + perm);
    }
  });
}

async function saveVipUrl() {
  const input = document.getElementById('txt-vip-url');
  if (!input) return;
  const val = input.value.trim();
  try {
    const res = await fetch('/api/action', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'set_vip_url', value: val })
    });
    const d = await res.json();
    showToast(d.message || 'VIP link saved!');
  } catch (e) {
    showToast('Failed to save VIP link: ' + e);
  }
}

async function toggleAutoReconnect() {
  try {
    const res = await fetch('/api/action', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'toggle_auto_reconnect' })
    });
    const d = await res.json();
    showToast(d.message || 'Auto-reconnect toggled');
    fetchStatus();
  } catch (e) {
    showToast('Failed to toggle auto-reconnect');
  }
}

if (window.Telegram && window.Telegram.WebApp) {
  const twa = window.Telegram.WebApp;
  twa.ready();
  twa.expand();
  if (twa.setHeaderColor) twa.setHeaderColor('#07090e');
  if (twa.setBackgroundColor) twa.setBackgroundColor('#07090e');
}

function showToast(msg) {
  const t = document.getElementById('toast');
  t.innerText = msg;
  t.classList.add('show');
  setTimeout(() => t.classList.remove('show'), 2200);
}

function fmtSec(s) {
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = Math.floor(s % 60);
  return `${String(h).padStart(2,'0')}:${String(m).padStart(2,'0')}:${String(sec).padStart(2,'0')}`;
}

function formatDuration(sec) {
  if (sec <= 0) return '00:00';
  let m = Math.floor(sec / 60);
  let s = Math.floor(sec % 60);
  if (m >= 60) {
    let h = Math.floor(m / 60);
    m = m % 60;
    return `${h}h ${String(m).padStart(2,'0')}m`;
  }
  return `${String(m).padStart(2,'0')}:${String(s).padStart(2,'0')}`;
}

async function fetchStatus() {
  try {
    const res = await fetch('/api/status');
    const d = await res.json();

    isRunning = d.is_running;
    isPaused = d.paused;
    isMuted = d.muted;
    spawnAlerts = d.spawn_alerts;
    localRuntimeSec = d.runtime_s;
    serverTimeDelta = d.server_time - Math.floor(Date.now() / 1000);
    bossesState = d.bosses || [];

    document.getElementById('host-sub').innerText = `${d.local_ip}:3888 • v${d.version}`;

    const fruitBadge = document.getElementById('badge-fruit');
    if (fruitBadge) {
      if (spawnAlerts) {
        fruitBadge.className = 'toggle-spawn-badge';
        fruitBadge.innerText = '🍇 ALERTS: ON';
      } else {
        fruitBadge.className = 'toggle-spawn-badge off';
        fruitBadge.innerText = '🍇 ALERTS: OFF';
      }
    }

    const pill = document.getElementById('status-pill');
    if (pill) {
      pill.innerText = d.state.toUpperCase();
      if (!isRunning) {
        pill.className = 'status-badge badge-stopped';
      } else if (isPaused) {
        pill.className = 'status-badge badge-paused';
      } else {
        pill.className = 'status-badge badge-running';
      }
    }

    const fsPill = document.getElementById('fs-status-pill');
    if (fsPill && pill) {
      fsPill.innerText = d.state.toUpperCase();
      fsPill.className = pill.className;
    }

    const tBtn = document.getElementById('btn-toggle');
    const tLabel = document.getElementById('toggle-label');
    const tIcon = document.getElementById('toggle-icon');
    const fsToggleBtn = document.getElementById('btn-fs-toggle');
    const headerToggle = document.getElementById('btn-header-toggle');

    if (isRunning && !isPaused) {
      if (tBtn) {
        tBtn.className = 'btn btn-toggle paused';
        tLabel.innerText = 'PAUSE MACRO';
        tIcon.innerText = '⏸️';
      }
      if (fsToggleBtn) fsToggleBtn.innerText = '⏸ PAUSE';
      if (headerToggle) {
        headerToggle.className = 'header-btn-toggle active';
        headerToggle.innerHTML = '<span>⏸</span><span>PAUSE</span>';
      }
    } else {
      if (tBtn) {
        tBtn.className = 'btn btn-toggle';
        tLabel.innerText = isPaused ? 'RESUME MACRO' : 'START MACRO';
        tIcon.innerText = '▶️';
      }
      if (fsToggleBtn) fsToggleBtn.innerText = isPaused ? '▶ RESUME' : '▶ START';
      if (headerToggle) {
        headerToggle.className = 'header-btn-toggle';
        headerToggle.innerHTML = `<span>▶</span><span>${isPaused ? 'RESUME' : 'START'}</span>`;
      }
    }

    const muteBtn = document.getElementById('btn-mute');
    if (muteBtn) muteBtn.innerText = isMuted ? '🔊 UNMUTE' : '🔇 MUTE';
    const fsMuteBtn = document.getElementById('btn-fs-mute');
    if (fsMuteBtn) fsMuteBtn.innerText = isMuted ? '🔊 UNMUTE' : '🔇 MUTE';

    const elFish = document.getElementById('val-fish');
    if (elFish) elFish.innerText = d.fish;
    const elRate = document.getElementById('val-rate');
    if (elRate) elRate.innerText = `${d.success_rate}% rate`;
    const elFruits = document.getElementById('val-fruits');
    if (elFruits) elFruits.innerText = d.fruits;
    const elPity = document.getElementById('val-pity');
    if (elPity) elPity.innerText = `Pity: ⚡ ${d.pity_fruit}`;
    const elLegPity = document.getElementById('val-leg-pity');
    if (elLegPity) elLegPity.innerText = d.pity_legendary;

    // Progress Gauges
    const barLegPity = document.getElementById('bar-leg-pity');
    if (barLegPity) {
      const legP = Math.min(100, Math.max(0, parseInt(d.pity_legendary || 0, 10)));
      barLegPity.style.width = `${legP}%`;
    }

    const elOrders = document.getElementById('val-orders');
    if (elOrders) elOrders.innerText = d.bait_purchased;
    const elAutoBuy = document.getElementById('val-auto-buy');
    if (elAutoBuy) elAutoBuy.innerText = d.auto_purchase ? 'Auto-buy ON' : 'Auto-buy OFF';
    const elRestock = document.getElementById('val-restock');
    if (elRestock) elRestock.innerText = `${d.since_purchase} / ${d.every_n_catches}`;

    const barRestock = document.getElementById('bar-restock');
    if (barRestock && d.every_n_catches > 0) {
      const pct = Math.min(100, Math.max(0, Math.round((d.since_purchase / d.every_n_catches) * 100)));
      barRestock.style.width = `${pct}%`;
    }

    const elTier = document.getElementById('val-tier');
    if (elTier) elTier.innerText = d.bait_tier;
    const elReserve = document.getElementById('val-reserve');
    if (elReserve) elReserve.innerText = d.legendary_reserve;

    if (!userSlidingVol) {
      const rVol = document.getElementById('rng-volume');
      if (rVol) rVol.value = d.volume;
      const lVol = document.getElementById('lbl-volume');
      if (lVol) lVol.innerText = `${d.volume}%`;
    }
    if (!userSlidingBright) {
      const rBri = document.getElementById('rng-brightness');
      if (rBri) rBri.value = d.brightness;
      const lBri = document.getElementById('lbl-brightness');
      if (lBri) lBri.innerText = `${d.brightness}%`;
    }

    // Audio Fanfare & Push Notification for Fruit Drops
    if (lastFruitsCount !== null && d.fruits > lastFruitsCount) {
      playFanfare('fruit');
      if ('Notification' in window && Notification.permission === 'granted') {
        try {
          new Notification('🍇 DEVIL FRUIT CAUGHT!', {
            body: `You caught a new Devil Fruit! Total catches: ${d.fruits}`,
            icon: 'data:image/svg+xml,%3Csvg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"%3E%3Ccircle cx="50" cy="50" r="40" fill="%23b026ff"/%3E%3C/svg%3E'
          });
        } catch (_) {}
      }
    }
    lastFruitsCount = d.fruits;

    if (d.state === 'recovering' && lastBotState !== 'recovering') {
      playFanfare('warning');
    } else if (lastBotState === 'recovering' && d.state !== 'recovering') {
      playFanfare('reconnect');
    }
    lastBotState = d.state;

    // Auto-reconnect & VIP Link sync
    const btnRec = document.getElementById('btn-reconnect-toggle');
    if (btnRec) {
      if (d.auto_reconnect) {
        btnRec.style.background = 'linear-gradient(135deg, #10b981, #059669)';
        btnRec.innerHTML = '<span>⚡</span><span>AUTO-RECONNECT: ACTIVE</span>';
      } else {
        btnRec.style.background = 'rgba(100, 116, 139, 0.25)';
        btnRec.innerHTML = '<span>🔌</span><span>AUTO-RECONNECT: OFF</span>';
      }
    }
    const txtVip = document.getElementById('txt-vip-url');
    if (txtVip && document.activeElement !== txtVip && d.vip_server_url !== undefined && !txtVip.value) {
      txtVip.value = d.vip_server_url;
    }

    updateCraftUi(d.crafting);
    updateMacroUi(d.recorder, d.macros);
    if (d.keyboard_light) updateKbdLightUi(d.keyboard_light);

    renderTimers();
  } catch (e) {
    console.warn('Status poll failed:', e);
  }
}

// 1s TIMER TICK
function tickTimersLocally() {
  if (isRunning && !isPaused) {
    localRuntimeSec += 1;
    const rt = document.getElementById('val-runtime');
    if (rt) rt.innerText = fmtSec(localRuntimeSec);
  }
  renderTimers();
}

function renderTimers() {
  if (!bossesState || bossesState.length === 0) return;
  const currentUnix = Math.floor(Date.now() / 1000) + serverTimeDelta;
  let bHtml = '';
  for (const b of bossesState) {
    let rem = Math.max(0, b.target_spawn - currentUnix);
    let tClass = 'boss-countdown';
    let tText = formatDuration(rem);
    if (rem <= 0) {
      tClass += ' boss-spawned';
      tText = 'SPAWNED NOW!';
    } else if (rem <= 300) {
      tClass += ' boss-soon';
      tText = `SOON (${formatDuration(rem)})`;
    }
    bHtml += `
      <div class="boss-card">
        <div class="boss-title"><span>${b.emoji}</span><span>${b.name}</span></div>
        <div class="${tClass}">${tText}</div>
      </div>`;
  }
  const bList = document.getElementById('boss-list');
  if (bList) bList.innerHTML = bHtml;
}

// STREAM QUALITY & FPS MANAGEMENT
let currentFps = parseInt(localStorage.getItem('gpo_stream_fps') || '20', 10);
let currentScale = parseInt(localStorage.getItem('gpo_stream_scale') || '720', 10);
let currentQuality = parseInt(localStorage.getItem('gpo_stream_quality') || '70', 10);
let currentQualityLabel = localStorage.getItem('gpo_stream_quality_label') || '720p (Balanced)';
let isFullscreen = false;
let isControlsOverlayVisible = true;

function setStreamFps(fps) {
  currentFps = fps;
  localStorage.setItem('gpo_stream_fps', fps);
  updateStreamLabels();
  reloadStream();
  document.querySelectorAll('#dropdown-fps-menu .custom-dropdown-item').forEach(el => {
    el.classList.toggle('active', el.getAttribute('data-val') == fps);
    const check = el.querySelector('.item-check');
    if (check) check.remove();
    if (el.getAttribute('data-val') == fps) {
      el.insertAdjacentHTML('beforeend', '<span class="item-check" style="color:var(--cyan);font-weight:800;">✓</span>');
    }
  });
  const menu = document.getElementById('dropdown-fps');
  if (menu) menu.classList.remove('open');
}

function setStreamQuality(scale, q, label) {
  currentScale = scale;
  currentQuality = q;
  currentQualityLabel = label;
  localStorage.setItem('gpo_stream_scale', scale);
  localStorage.setItem('gpo_stream_quality', q);
  localStorage.setItem('gpo_stream_quality_label', label);
  updateStreamLabels();
  reloadStream();
  document.querySelectorAll('#dropdown-quality-menu .custom-dropdown-item').forEach(el => {
    el.classList.toggle('active', el.getAttribute('data-val') == scale);
    const check = el.querySelector('.item-check');
    if (check) check.remove();
    if (el.getAttribute('data-val') == scale) {
      el.insertAdjacentHTML('beforeend', '<span class="item-check" style="color:var(--cyan);font-weight:800;">✓</span>');
    }
  });
  const menu = document.getElementById('dropdown-quality');
  if (menu) menu.classList.remove('open');
}

function updateStreamLabels() {
  const fpsBadge = document.getElementById('stream-fps-badge');
  const fpsLabel = document.getElementById('dropdown-fps-label');
  const resBadge = document.getElementById('stream-res-badge');
  const resLabel = document.getElementById('dropdown-quality-label');
  const fsInfo = document.getElementById('fs-stream-info');

  const shortRes = currentScale === 0 ? 'MAX' : `${currentScale}p`;
  if (fpsBadge) fpsBadge.innerText = `${currentFps} FPS`;
  if (fpsLabel) fpsLabel.innerText = `⚡ ${currentFps} FPS`;
  if (resBadge) resBadge.innerText = shortRes;
  if (resLabel) resLabel.innerText = `📺 ${shortRes}`;
  if (fsInfo) fsInfo.innerText = `${currentFps} FPS • ${shortRes}`;
}

function reloadStream() {
  const img = document.getElementById('screen-img');
  if (!img) return;
  const url = `/api/stream?fps=${currentFps}&scale=${currentScale}&q=${currentQuality}&t=` + Date.now();
  img.src = url;
}

function toggleFullscreen() {
  if (!isFullscreen) enterFullscreen();
  else exitFullscreen();
}

function enterFullscreen() {
  isFullscreen = true;
  const wrap = document.getElementById('stream-wrapper');
  if (wrap) wrap.classList.add('fullscreen-active');
  const bNav = document.getElementById('bottom-nav-bar');
  if (bNav) bNav.style.display = 'none';
  const aHead = document.getElementById('app-header');
  if (aHead) aHead.style.display = 'none';

  const docEl = document.documentElement;
  try {
    if (docEl.requestFullscreen) docEl.requestFullscreen();
    else if (docEl.webkitRequestFullscreen) docEl.webkitRequestFullscreen();
  } catch (_) {}
}

function exitFullscreen() {
  isFullscreen = false;
  const wrap = document.getElementById('stream-wrapper');
  if (wrap) {
    wrap.classList.remove('fullscreen-active');
    wrap.classList.remove('rotated-90');
  }
  const bNav = document.getElementById('bottom-nav-bar');
  if (bNav) bNav.style.display = 'flex';
  const aHead = document.getElementById('app-header');
  if (aHead) aHead.style.display = 'flex';

  updateRotateButtons(false);
  try {
    if (document.exitFullscreen) document.exitFullscreen();
    else if (document.webkitExitFullscreen) document.webkitExitFullscreen();
  } catch (_) {}
}

let isRotated = false;
function toggleRotate() {
  isRotated = !isRotated;
  const wrap = document.getElementById('stream-wrapper');
  if (!isFullscreen) enterFullscreen();
  if (wrap) {
    if (isRotated) wrap.classList.add('rotated-90');
    else wrap.classList.remove('rotated-90');
  }
  updateRotateButtons(isRotated);
}

function updateRotateButtons(rot) {
  const b1 = document.getElementById('stream-tool-rotate');
  const b2 = document.getElementById('btn-fs-rotate');
  [b1, b2].forEach(b => {
    if (b) {
      if (rot) b.classList.add('active');
      else b.classList.remove('active');
    }
  });
}

let isRightClickMode = false;
function toggleClickMode() {
  isRightClickMode = !isRightClickMode;
  const icon = document.getElementById('click-mode-icon');
  const label = document.getElementById('click-mode-label');
  const btn = document.getElementById('btn-click-mode');
  if (isRightClickMode) {
    if (icon) icon.innerText = '👀';
    if (label) label.innerText = 'LOOK';
    if (btn) btn.classList.add('active');
    showToast('Touch set to Camera Look (Right-Click)');
  } else {
    if (icon) icon.innerText = '🎯';
    if (label) label.innerText = 'CLICK';
    if (btn) btn.classList.remove('active');
    showToast('Touch set to Primary Click (Left-Click)');
  }
}

function toggleFullscreenControls() {
  isControlsOverlayVisible = !isControlsOverlayVisible;
  const overlay = document.getElementById('fs-controls-overlay');
  const btn = document.getElementById('btn-fs-overlay-toggle');
  if (overlay) overlay.classList.toggle('visible', isControlsOverlayVisible);
  if (btn) btn.classList.toggle('active', isControlsOverlayVisible);
}

let isHoldDragMode = true;
function toggleHoldDragMode() {
  isHoldDragMode = !isHoldDragMode;
  const btnTool = document.getElementById('stream-tool-drag');
  const lblTool = document.getElementById('stream-drag-label');
  const btnFs = document.getElementById('btn-fs-drag');
  const lblFs = document.getElementById('fs-drag-label');
  const iconFs = document.getElementById('fs-drag-icon');

  if (isHoldDragMode) {
    if (btnTool) btnTool.classList.add('active');
    if (lblTool) lblTool.innerText = '🖐️ DRAG';
    if (btnFs) btnFs.classList.add('active');
    if (lblFs) lblFs.innerText = 'DRAG';
    if (iconFs) iconFs.innerText = '🖐️';
    showToast('Touch mode: Camera Hold & Drag');
  } else {
    if (btnTool) btnTool.classList.remove('active');
    if (lblTool) lblTool.innerText = '👆 TAP';
    if (btnFs) btnFs.classList.remove('active');
    if (lblFs) lblFs.innerText = 'TAP';
    if (iconFs) iconFs.innerText = '👆';
    showToast('Touch mode: Direct Tap to Click');
  }
}

// SCREEN TOUCH CONTROLLER
let touchPointerId = null;
let touchStartX = 0;
let touchStartY = 0;
let touchStartRelX = 0;
let touchStartRelY = 0;
let touchStartTime = 0;
let touchIndicatorEl = null;
let touchMoveThrottle = 0;

function getScreenRelCoords(clientX, clientY) {
  const img = document.getElementById('screen-img');
  if (!img) return null;
  const rect = img.getBoundingClientRect();
  if (clientX < rect.left || clientX > rect.right || clientY < rect.top || clientY > rect.bottom) {
    return null;
  }
  const rx = (clientX - rect.left) / rect.width;
  const ry = (clientY - rect.top) / rect.height;
  return {
    rx: Math.max(0, Math.min(1, rx)),
    ry: Math.max(0, Math.min(1, ry)),
    rect
  };
}

function updateIndicatorPos(clientX, clientY) {
  if (!touchIndicatorEl) return;
  const container = document.getElementById('screen-container');
  if (!container) return;
  const cRect = container.getBoundingClientRect();
  const ox = clientX - cRect.left;
  const oy = clientY - cRect.top;
  touchIndicatorEl.style.left = `${ox}px`;
  touchIndicatorEl.style.top = `${oy}px`;
}

function onScreenPointerDown(e) {
  if (touchPointerId !== null) return;
  const coords = getScreenRelCoords(e.clientX, e.clientY);
  if (!coords) return;

  touchPointerId = e.pointerId;
  touchStartX = e.clientX;
  touchStartY = e.clientY;
  touchStartRelX = coords.rx;
  touchStartRelY = coords.ry;
  touchStartTime = Date.now();

  const container = document.getElementById('screen-container');
  try { container.setPointerCapture(e.pointerId); } catch (_) {}

  touchIndicatorEl = document.createElement('div');
  touchIndicatorEl.className = 'touch-drag-indicator' + (isRightClickMode ? ' right-mode' : '');
  touchIndicatorEl.innerHTML = '<div class="touch-drag-indicator-core"></div>';
  container.appendChild(touchIndicatorEl);
  updateIndicatorPos(e.clientX, e.clientY);

  if (isHoldDragMode) {
    const btn = isRightClickMode ? 'right' : 'right';
    fetch('/api/mouse', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'down', button: btn, rx: coords.rx, ry: coords.ry })
    }).catch(() => {});
  }
}

function onScreenPointerMove(e) {
  if (e.pointerId !== touchPointerId) return;
  updateIndicatorPos(e.clientX, e.clientY);

  const now = Date.now();
  if (now - touchMoveThrottle < 20) return;
  touchMoveThrottle = now;

  const coords = getScreenRelCoords(e.clientX, e.clientY);
  if (!coords) return;

  if (isHoldDragMode) {
    if (touchIndicatorEl) touchIndicatorEl.classList.add('dragging');
    fetch('/api/mouse', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'move', rx: coords.rx, ry: coords.ry })
    }).catch(() => {});
  }
}

function onScreenPointerUp(e) {
  if (e.pointerId !== touchPointerId) return;
  const container = document.getElementById('screen-container');
  try { container.releasePointerCapture(e.pointerId); } catch (_) {}

  const duration = Date.now() - touchStartTime;
  const dist = Math.hypot(e.clientX - touchStartX, e.clientY - touchStartY);
  const coords = getScreenRelCoords(e.clientX, e.clientY) || { rx: touchStartRelX, ry: touchStartRelY };

  if (isHoldDragMode) {
    const btn = isRightClickMode ? 'right' : 'right';
    fetch('/api/mouse', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'up', button: btn, rx: coords.rx, ry: coords.ry })
    }).catch(() => {});

    if (dist < 8 && duration < 250) {
      fetch('/api/click', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ rx: coords.rx, ry: coords.ry, button: isRightClickMode ? 'right' : 'left' })
      }).catch(() => {});
    }
  } else {
    fetch('/api/click', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ rx: coords.rx, ry: coords.ry, button: isRightClickMode ? 'right' : 'left' })
    }).catch(() => {});
  }

  if (touchIndicatorEl) {
    touchIndicatorEl.remove();
    touchIndicatorEl = null;
  }
  touchPointerId = null;
}

function onScreenPointerCancel(e) {
  if (e.pointerId !== touchPointerId) return;
  const container = document.getElementById('screen-container');
  try { container.releasePointerCapture(e.pointerId); } catch (_) {}
  if (isHoldDragMode) {
    fetch('/api/mouse', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'up', button: 'right', rx: touchStartRelX, ry: touchStartRelY })
    }).catch(() => {});
  }
  if (touchIndicatorEl) {
    touchIndicatorEl.remove();
    touchIndicatorEl = null;
  }
  touchPointerId = null;
}

const sBox = document.getElementById('screen-container');
if (sBox) {
  sBox.addEventListener('pointerdown', onScreenPointerDown);
  sBox.addEventListener('pointermove', onScreenPointerMove);
  sBox.addEventListener('pointerup', onScreenPointerUp);
  sBox.addEventListener('pointercancel', onScreenPointerCancel);
}

// KEYBOARD EMULATION WITH MULTI-TOUCH
const activeKeys = new Set();
const activePointers = new Map();
let heartbeatInterval = null;

function sendKey(k, down, tap = false) {
  fetch('/api/key', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key: k, down, tap })
  }).catch(() => {});
}

function startKeyHeartbeat() {
  if (heartbeatInterval) return;
  heartbeatInterval = setInterval(() => {
    if (activeKeys.size === 0) {
      clearInterval(heartbeatInterval);
      heartbeatInterval = null;
      return;
    }
    const keysArray = Array.from(activeKeys);
    fetch('/api/key', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ keys: keysArray, heartbeat: true })
    }).catch(() => {});
  }, 450);
}

function releaseAllKeys() {
  fetch('/api/key', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ action: 'release_all' })
  }).catch(() => {});

  if (activeKeys.size === 0 && activePointers.size === 0) return;
  activePointers.clear();
  activeKeys.clear();
  if (heartbeatInterval) {
    clearInterval(heartbeatInterval);
    heartbeatInterval = null;
  }
  document.querySelectorAll('.dpad-btn.pressed, .pad-action-btn.pressed').forEach(b => b.classList.remove('pressed'));
  sendKey('release_all', false);
}

// ATTACH TOUCH HANDLERS TO GAMEPAD BUTTONS
document.querySelectorAll('.dpad-btn').forEach(btn => {
  const key = btn.getAttribute('data-key');
  if (!key) return;

  btn.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    e.stopPropagation();
    try { btn.setPointerCapture(e.pointerId); } catch (_) {}
    btn.classList.add('pressed');
    activePointers.set(e.pointerId, { key, btn });
    activeKeys.add(key);
    if (navigator.vibrate) navigator.vibrate(10);
    sendKey(key, true);
    startKeyHeartbeat();
  });

  const onPointerRelease = (e) => {
    if (!activePointers.has(e.pointerId)) return;
    e.preventDefault();
    e.stopPropagation();
    try { btn.releasePointerCapture(e.pointerId); } catch (_) {}
    const entry = activePointers.get(e.pointerId);
    activePointers.delete(e.pointerId);

    let stillHeld = false;
    for (const p of activePointers.values()) {
      if (p.key === entry.key) { stillHeld = true; break; }
    }
    if (!stillHeld) {
      entry.btn.classList.remove('pressed');
      activeKeys.delete(entry.key);
      sendKey(entry.key, false);
    }
    if (activeKeys.size === 0 && heartbeatInterval) {
      clearInterval(heartbeatInterval);
      heartbeatInterval = null;
    }
  };

  btn.addEventListener('pointerup', onPointerRelease);
  btn.addEventListener('pointercancel', onPointerRelease);
});

document.querySelectorAll('.pad-action-btn').forEach(btn => {
  const key = btn.getAttribute('data-key');
  if (!key) return;

  btn.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    e.stopPropagation();
    try { btn.setPointerCapture(e.pointerId); } catch (_) {}
    btn.classList.add('pressed');
    activePointers.set(e.pointerId, { key, btn });
    activeKeys.add(key);
    if (navigator.vibrate) navigator.vibrate(10);
    sendKey(key, true);
    startKeyHeartbeat();
  });

  const onPointerRelease = (e) => {
    if (!activePointers.has(e.pointerId)) return;
    e.preventDefault();
    e.stopPropagation();
    try { btn.releasePointerCapture(e.pointerId); } catch (_) {}
    const entry = activePointers.get(e.pointerId);
    activePointers.delete(e.pointerId);

    let stillHeld = false;
    for (const p of activePointers.values()) {
      if (p.key === entry.key) { stillHeld = true; break; }
    }
    if (!stillHeld) {
      entry.btn.classList.remove('pressed');
      activeKeys.delete(entry.key);
      sendKey(entry.key, false);
    }
    if (activeKeys.size === 0 && heartbeatInterval) {
      clearInterval(heartbeatInterval);
      heartbeatInterval = null;
    }
  };

  btn.addEventListener('pointerup', onPointerRelease);
  btn.addEventListener('pointercancel', onPointerRelease);
});

window.addEventListener('blur', releaseAllKeys);
document.addEventListener('visibilitychange', () => {
  if (document.hidden) releaseAllKeys();
});

async function doAction(act, val = null) {
  try {
    const res = await fetch('/api/action', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: act, value: val })
    });
    const d = await res.json();
    if (d.error) showToast('Error: ' + d.error);
    else showToast('Action sent: ' + act);
    fetchStatus();
  } catch (e) {
    showToast('Failed to trigger ' + act);
  }
}

function togglePlay() {
  if (isRunning && !isPaused) doAction('pause');
  else if (isRunning && isPaused) doAction('resume');
  else doAction('start');
}

function toggleMute() {
  doAction(isMuted ? 'unmute' : 'mute');
}

function toggleSpawnAlerts() {
  doAction('toggle_spawn_alerts');
}

function doUpdate() {
  showToast('Triggering update check...');
  doAction('check_update');
}

// KEYBOARD BACKLIGHT CONTROLS
function updateKbdLightUi(kbd) {
  if (!kbd) return;
  const badge = document.getElementById('kbd-light-badge');
  const bOff = document.getElementById('btn-kbd-off');
  const bLow = document.getElementById('btn-kbd-low');
  const bHigh = document.getElementById('btn-kbd-high');
  if (!badge) return;

  const lvl = kbd.level;
  const st = (kbd.status || '').toLowerCase();

  [bOff, bLow, bHigh].forEach(b => { if (b) b.classList.remove('active'); });

  if (lvl === 0 || st === 'off') {
    badge.innerText = 'OFF';
    badge.style.background = 'rgba(255,255,255,0.08)';
    badge.style.color = '#94a3b8';
    badge.style.borderColor = 'rgba(255,255,255,0.15)';
    badge.style.boxShadow = 'none';
    if (bOff) bOff.classList.add('active');
  } else if (lvl === 1 || st === 'low') {
    badge.innerText = 'LOW';
    badge.style.background = 'rgba(245, 158, 11, 0.2)';
    badge.style.color = '#fbbf24';
    badge.style.borderColor = 'rgba(245, 158, 11, 0.4)';
    badge.style.boxShadow = '0 0 10px rgba(245, 158, 11, 0.3)';
    if (bLow) bLow.classList.add('active');
  } else if (lvl === 2 || st === 'high') {
    badge.innerText = 'HIGH';
    badge.style.background = 'rgba(168, 85, 247, 0.25)';
    badge.style.color = '#d8b4fe';
    badge.style.borderColor = 'rgba(168, 85, 247, 0.5)';
    badge.style.boxShadow = '0 0 12px rgba(168, 85, 247, 0.45)';
    if (bHigh) bHigh.classList.add('active');
  } else {
    badge.innerText = kbd.status ? kbd.status.toUpperCase() : 'UNKNOWN';
  }
}

async function setKbdLight(act) {
  try {
    const res = await fetch('/api/keyboard/light', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: act })
    });
    const data = await res.json();
    if (data.status) {
      updateKbdLightUi(data);
      showToast('Keyboard light: ' + data.status.toUpperCase());
    } else if (data.error) {
      showToast('Light error: ' + data.error);
    }
    fetchStatus();
  } catch (e) {
    showToast('Failed to set light: ' + e);
  }
}

// AUTO CRAFT CONTROLS
let isCrafting = false;
function updateCraftUi(c) {
  if (!c) return;
  isCrafting = c.is_crafting;
  const badge = document.getElementById('craft-status-badge');
  const btn = document.getElementById('btn-craft-toggle');
  const label = document.getElementById('craft-btn-label');
  const icon = document.getElementById('craft-btn-icon');
  const msg = document.getElementById('craft-msg');

  if (!badge || !btn) return;

  if (isCrafting) {
    badge.className = 'status-badge badge-running';
    badge.innerText = `CRAFTING (${c.crafted_count})`;
    btn.style.background = 'linear-gradient(135deg, #ef4444, #dc2626)';
    btn.style.color = '#fff';
    label.innerText = 'STOP AUTO CRAFT';
    icon.innerText = '🛑';
    if (c.message) msg.innerText = c.message;
  } else {
    badge.className = 'status-badge badge-stopped';
    badge.innerText = 'IDLE';
    btn.style.background = 'linear-gradient(135deg, #f59e0b, #d97706)';
    btn.style.color = '#000';
    label.innerText = 'START AUTO CRAFT';
    icon.innerText = '🔨';
    if (c.message) msg.innerText = c.message;
  }
}

let selectedCraftTier = "rare";
let selectedMacroValue = "";

// MOBILE APP TAB SWITCHING
function switchTab(tab, btn) {
  localStorage.setItem('gpo_mobile_tab', tab);
  document.querySelectorAll('.bottom-nav-item').forEach(b => b.classList.remove('active'));
  const activeBtn = btn || document.querySelector(`.bottom-nav-item[data-tab="${tab}"]`);
  if (activeBtn) activeBtn.classList.add('active');

  const remoteSec = document.getElementById('sec-remote');
  const macroSec = document.getElementById('sec-macro');
  const craftSec = document.getElementById('sec-craft');
  const statsSec = document.getElementById('sec-stats');
  const systemSec = document.getElementById('sec-system');

  if (tab === 'all') {
    [remoteSec, macroSec, craftSec, statsSec, systemSec].forEach(s => { if (s) s.classList.add('active'); });
  } else {
    if (remoteSec) remoteSec.classList.toggle('active', tab === 'remote');
    if (statsSec) statsSec.classList.toggle('active', tab === 'stats');
    if (macroSec) macroSec.classList.toggle('active', tab === 'macro');
    if (craftSec) craftSec.classList.toggle('active', tab === 'craft');
    if (systemSec) systemSec.classList.toggle('active', tab === 'system');
  }

  if (window.Telegram?.WebApp?.HapticFeedback) {
    window.Telegram.WebApp.HapticFeedback.selectionChanged();
  } else if (navigator.vibrate) {
    navigator.vibrate(10);
  }
  window.scrollTo({ top: 0, behavior: 'smooth' });
}

function toggleDropdown(id) {
  const el = document.getElementById(id);
  if (!el) return;
  const isOpen = el.classList.contains('open');
  document.querySelectorAll('.custom-dropdown').forEach(d => d.classList.remove('open'));
  if (!isOpen) {
    el.classList.add('open');
  }
}

document.addEventListener('click', (e) => {
  if (!e.target.closest('.custom-dropdown')) {
    document.querySelectorAll('.custom-dropdown').forEach(d => d.classList.remove('open'));
  }
});

function selectCraftTier(val, label) {
  selectedCraftTier = val;
  const labelEl = document.getElementById('dropdown-craft-label');
  if (labelEl) labelEl.innerText = label;
  document.querySelectorAll('#dropdown-craft-menu .custom-dropdown-item').forEach(el => {
    el.classList.toggle('active', el.getAttribute('data-val') === val);
    const check = el.querySelector('.item-check');
    if (check) check.remove();
    if (el.getAttribute('data-val') === val) {
      el.insertAdjacentHTML('beforeend', '<span class="item-check" style="color:var(--cyan);font-weight:800;">✓</span>');
    }
  });
  const menu = document.getElementById('dropdown-craft');
  if (menu) menu.classList.remove('open');
}

function selectMacroItem(val) {
  selectedMacroValue = val;
  const m = cachedMacros.find(x => x.name === val || x.id === val);
  const labelEl = document.getElementById('dropdown-macro-label');
  if (labelEl && m) labelEl.innerText = `📋 ${m.name} (${m.steps.length} steps)`;
  document.querySelectorAll('#dropdown-macro-menu .custom-dropdown-item').forEach(el => {
    el.classList.toggle('active', el.getAttribute('data-val') === val);
    const check = el.querySelector('.item-check');
    if (check) check.remove();
    if (el.getAttribute('data-val') === val) {
      el.insertAdjacentHTML('beforeend', '<span class="item-check" style="color:var(--cyan);font-weight:800;">✓</span>');
    }
  });
  const menu = document.getElementById('dropdown-macro');
  if (menu) menu.classList.remove('open');
  renderMacroSteps(m);
}

async function toggleAutoCraft() {
  if (isCrafting) {
    try {
      const res = await fetch('/api/craft', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'stop' })
      });
      const data = await res.json();
      showToast(data.message || 'Auto-craft stopped');
      fetchStatus();
    } catch (e) {
      showToast('Stop craft failed: ' + e);
    }
  } else {
    try {
      const res = await fetch('/api/craft', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'start', tier: selectedCraftTier })
      });
      const data = await res.json();
      showToast(data.message || 'Auto-craft started!');
      fetchStatus();
    } catch (e) {
      showToast('Start craft failed: ' + e);
    }
  }
}

// MACRO STUDIO CONTROLS
let isRecordingMacro = false;
let isPlayingMacro = false;
let webSpeed = 1.0;
let webLoopCount = 1;
let cachedMacros = [];

function setWebSpeed(val, btn) {
  webSpeed = Number(val);
  document.querySelectorAll('#speed-pills .macro-pill-btn').forEach(b => b.classList.remove('active'));
  if (btn) btn.classList.add('active');
}

function setWebLoopCount(val, btn) {
  webLoopCount = Number(val);
  document.querySelectorAll('#loop-pills .macro-pill-btn').forEach(b => b.classList.remove('active-purple'));
  if (btn) btn.classList.add('active-purple');
}

function renderMacroSteps(m) {
  const countEl = document.getElementById('macro-steps-count');
  const listEl = document.getElementById('macro-steps-list');
  if (!m || !m.steps || m.steps.length === 0) {
    if (countEl) countEl.innerText = '0';
    if (listEl) listEl.innerHTML = '<div style="color: var(--text-mute);">No steps recorded for this macro.</div>';
    return;
  }
  if (countEl) countEl.innerText = m.steps.length;
  let html = '';
  m.steps.forEach((s, i) => {
    let desc = '';
    if (s.type === 'Drag') {
      desc = `<span style="color:#6ee7b7">🖱️ DRAG (${(s.start_rx*100).toFixed(0)}%, ${(s.start_ry*100).toFixed(0)}%) ➔ (${(s.end_rx*100).toFixed(0)}%, ${(s.end_ry*100).toFixed(0)}%) [${s.duration_ms}ms]</span>`;
    } else if (s.type === 'Click') {
      desc = `<span style="color:#67e8f9">👆 ${s.button.toUpperCase()} CLICK (${(s.rx*100).toFixed(0)}%, ${(s.ry*100).toFixed(0)}%)</span>`;
    } else if (s.type === 'KeyHold') {
      desc = `<span style="color:#fcd34d">⌨️ HOLD [${s.key.toUpperCase()}] for ${s.duration_ms}ms</span>`;
    } else if (s.type === 'KeyTap') {
      desc = `<span style="color:#d8b4fe">⌨️ TAP [${s.key.toUpperCase()}]</span>`;
    } else if (s.type === 'MouseMove') {
      desc = `<span style="color:#93c5fd">🖱️ MOVE (${(s.rx*100).toFixed(0)}%, ${(s.ry*100).toFixed(0)}%)</span>`;
    } else {
      desc = `<span style="color:#94a3b8">⏳ SLEEP ${s.ms || 0}ms</span>`;
    }
    const delay = s.delay_ms ? `<span style="color:#64748b;font-size:0.68rem;">+${s.delay_ms}ms</span>` : '';
    html += `<div class="macro-step-row"><div><span style="color:#64748b;margin-right:6px;">#${i+1}</span>${desc}</div>${delay}</div>`;
  });
  if (listEl) listEl.innerHTML = html;
}

function updateMacroUi(rec, macros) {
  if (!rec) return;
  isRecordingMacro = rec.is_recording;
  isPlayingMacro = rec.is_playing;

  const stBadge = document.getElementById('macro-status-badge');
  const countBadge = document.getElementById('record-count-badge');
  const recToggle = document.getElementById('btn-record-toggle');
  const recCancel = document.getElementById('btn-record-cancel');
  const recIcon = document.getElementById('record-btn-icon');
  const recLabel = document.getElementById('record-btn-label');
  const recHint = document.getElementById('record-hint');

  const playBadge = document.getElementById('play-loop-badge');
  const btnPlay = document.getElementById('btn-macro-play');
  const btnLoop = document.getElementById('btn-macro-loop');
  const btnStop = document.getElementById('btn-macro-stop');
  const macroMsg = document.getElementById('macro-msg');

  if (isRecordingMacro) {
    if (stBadge) {
      stBadge.className = 'status-badge badge-running';
      stBadge.innerText = 'RECORDING';
    }
    if (countBadge) countBadge.innerText = `${rec.recorded_steps_count} STEPS`;
    if (recToggle) {
      recToggle.style.background = 'linear-gradient(135deg, #ef4444, #dc2626)';
      recToggle.style.color = '#fff';
    }
    if (recIcon) recIcon.innerText = '⏹️';
    if (recLabel) recLabel.innerText = 'FINISH & SAVE';
    if (recCancel) recCancel.style.display = 'inline-flex';
    if (recHint) recHint.innerText = 'Tap live screen or controls (T, E, WASD, 1-5). Each action & delay is saved!';
  } else {
    if (countBadge) countBadge.innerText = rec.recorded_steps_count > 0 ? `${rec.recorded_steps_count} STEPS` : 'READY';
    if (recToggle) {
      recToggle.style.background = 'linear-gradient(135deg, #00f0ff, #0284c7)';
      recToggle.style.color = '#000';
    }
    if (recIcon) recIcon.innerText = '⏺️';
    if (recLabel) recLabel.innerText = 'RECORD';
    if (recCancel) recCancel.style.display = 'none';
    if (recHint) recHint.innerText = 'Tap Record, then tap live screen and press controls. Captures timing automatically!';
  }

  if (isPlayingMacro) {
    if (stBadge) {
      stBadge.className = 'status-badge badge-running';
      stBadge.innerText = rec.is_looping ? `LOOP #${rec.current_loop}` : 'PLAYING';
    }
    if (playBadge) playBadge.innerText = rec.is_looping ? `LOOPING (#${rec.current_loop}) [${webSpeed}x]` : `PLAYING ONCE [${webSpeed}x]`;
    if (btnPlay) btnPlay.style.display = 'none';
    if (btnLoop) btnLoop.style.display = 'none';
    if (btnStop) btnStop.style.display = 'inline-flex';
  } else {
    if (!isRecordingMacro && stBadge) {
      stBadge.className = 'status-badge badge-stopped';
      stBadge.innerText = 'IDLE';
    }
    if (playBadge) playBadge.innerText = 'READY';
    if (btnPlay) btnPlay.style.display = 'inline-flex';
    if (btnLoop) btnLoop.style.display = 'inline-flex';
    if (btnStop) btnStop.style.display = 'none';
  }

  if (rec.message && macroMsg) {
    macroMsg.innerText = rec.message;
  }

  // Update Custom Macro Dropdown
  cachedMacros = macros || [];
  const menu = document.getElementById('dropdown-macro-menu');
  const labelEl = document.getElementById('dropdown-macro-label');
  if (menu) {
    if (!macros || macros.length === 0) {
      menu.innerHTML = '<div class="custom-dropdown-item" style="color:var(--text-mute);cursor:default;">(No macros saved yet)</div>';
      if (labelEl) labelEl.innerText = '(No macros saved yet)';
      selectedMacroValue = '';
      renderMacroSteps(null);
    } else {
      let itemsHtml = '';
      if (!selectedMacroValue || !macros.some(m => m.name === selectedMacroValue || m.id === selectedMacroValue)) {
        selectedMacroValue = macros[0].name;
      }
      for (const m of macros) {
        const isSel = (m.name === selectedMacroValue || m.id === selectedMacroValue);
        itemsHtml += `<div class="custom-dropdown-item ${isSel ? 'active' : ''}" data-val="${m.name}" onclick="selectMacroItem('${m.name.replace(/'/g, "\\'")}')">
          <div style="flex:1;min-width:0;text-align:left;">
            <div style="font-weight:700;color:#fff;">${m.name}</div>
            <div class="dropdown-item-sub">${m.steps.length} steps</div>
          </div>
          ${isSel ? '<span class="item-check" style="color:var(--cyan);font-weight:800;">✓</span>' : ''}
        </div>`;
      }
      menu.innerHTML = itemsHtml;
      const activeM = macros.find(m => m.name === selectedMacroValue || m.id === selectedMacroValue) || macros[0];
      if (labelEl && activeM) {
        labelEl.innerText = `📋 ${activeM.name} (${activeM.steps.length} steps)`;
      }
      renderMacroSteps(activeM);
    }
  }
}

async function toggleRecord() {
  if (isRecordingMacro) {
    const name = document.getElementById('txt-macro-name').value.trim() || 'My Macro';
    try {
      const res = await fetch('/api/macro/record', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'stop', name })
      });
      const data = await res.json();
      showToast(data.message || 'Macro saved!');
      fetchStatus();
    } catch (e) {
      showToast('Save failed: ' + e);
    }
  } else {
    try {
      const res = await fetch('/api/macro/record', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'start', mode: 'web' })
      });
      const data = await res.json();
      showToast(data.message || 'Recording started!');
      fetchStatus();
    } catch (e) {
      showToast('Record start failed: ' + e);
    }
  }
}

async function cancelRecord() {
  try {
    const res = await fetch('/api/macro/record', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'cancel' })
    });
    const data = await res.json();
    showToast(data.message || 'Recording cancelled');
    fetchStatus();
  } catch (e) {
    showToast('Cancel failed: ' + e);
  }
}

async function renameSelectedMacro() {
  if (!selectedMacroValue) {
    showToast('Please select a macro first');
    return;
  }
  const current = selectedMacroValue;
  const newName = prompt(`Enter new name for macro "${current}":`, current);
  if (!newName || newName.trim() === '' || newName.trim() === current) return;
  try {
    const res = await fetch('/api/macro/rename', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id: current, name: newName.trim() })
    });
    const data = await res.json();
    if (data.status === 'ok') {
      showToast('Macro renamed to ' + newName.trim());
      selectedMacroValue = newName.trim();
      fetchStatus();
    } else {
      showToast('Rename failed: ' + (data.error || 'unknown'));
    }
  } catch (e) {
    showToast('Rename error: ' + e);
  }
}

async function playMacro(isLoop) {
  if (!selectedMacroValue) {
    showToast('Please record or select a macro first');
    return;
  }
  try {
    const res = await fetch('/api/macro/play', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        id: selectedMacroValue,
        loop: isLoop,
        loop_count: isLoop ? webLoopCount : 1,
        speed: webSpeed
      })
    });
    const data = await res.json();
    showToast(data.message || (isLoop ? 'Started loop playback' : 'Playing macro once'));
    fetchStatus();
  } catch (e) {
    showToast('Play failed: ' + e);
  }
}

async function stopMacro() {
  try {
    const res = await fetch('/api/macro/play', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'stop' })
    });
    const data = await res.json();
    showToast(data.message || 'Macro playback stopped');
    fetchStatus();
  } catch (e) {
    showToast('Stop failed: ' + e);
  }
}

async function deleteSelectedMacro() {
  if (!selectedMacroValue) {
    showToast('No macro selected');
    return;
  }
  if (!confirm(`Are you sure you want to delete macro "${selectedMacroValue}"?`)) return;
  try {
    const res = await fetch('/api/macro/delete', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id: selectedMacroValue })
    });
    const data = await res.json();
    if (data.status === 'ok') {
      showToast('Macro deleted');
      selectedMacroValue = '';
      fetchStatus();
    } else {
      showToast('Delete error: ' + (data.error || 'unknown'));
    }
  } catch (e) {
    showToast('Delete failed: ' + e);
  }
}

// SLIDERS
function onVolInput(val) {
  userSlidingVol = true;
  const lbl = document.getElementById('lbl-volume');
  if (lbl) lbl.innerText = `${val}%`;
}
function onVolRelease(val) {
  userSlidingVol = false;
  doAction('set_volume', val);
}
function onBrightInput(val) {
  userSlidingBright = true;
  const lbl = document.getElementById('lbl-brightness');
  if (lbl) lbl.innerText = `${val}%`;
}
function onBrightRelease(val) {
  userSlidingBright = false;
  doAction('set_brightness', val);
}

function fallbackSnapshot() {
  const img = document.getElementById('screen-img');
  if (img) {
    img.src = `/api/screenshot?scale=${currentScale}&q=${currentQuality}&t=` + Date.now();
  }
}

// RESTORE SAVED TAB
const savedTab = localStorage.getItem('gpo_mobile_tab') || 'remote';
switchTab(savedTab);

updateStreamLabels();
reloadStream();

setInterval(fetchStatus, 1500);
setInterval(tickTimersLocally, 1000);
fetchStatus();
</script>
</body>
</html>
"##;

    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-cache, no-store, must-revalidate, max-age=0\r\nPragma: no-cache\r\nExpires: 0\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = stream.write_all(resp.as_bytes());
}
