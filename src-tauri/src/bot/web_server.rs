use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use parking_lot::RwLock;
use serde_json::json;
use crate::bot::Bot;
use crate::config::Settings;

static LAST_FRAME: parking_lot::RwLock<Option<Vec<u8>>> = parking_lot::RwLock::new(None);

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

    if raw_path == "/" || raw_path == "/index.html" {
        send_html(&mut stream);
    } else if raw_path == "/api/status" {
        send_status(&mut stream, bot, settings);
    } else if raw_path == "/api/stream" {
        let _ = stream.set_read_timeout(None);
        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
        stream_mjpeg(stream, bot);
    } else if raw_path == "/api/screenshot" {
        send_screenshot(&mut stream, bot);
    } else if raw_path == "/api/click" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
        handle_click(&mut stream, bot, body);
    } else if raw_path == "/api/key" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
        handle_key(&mut stream, bot, body);
    } else if raw_path == "/api/action" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
        handle_action(&mut stream, bot, settings, body);
    } else if raw_path == "/api/craft" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") { &req_str[idx + 4..] } else { "" };
        handle_craft(&mut stream, bot, body);
    } else {
        let not_found = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(not_found.as_bytes());
    }
}

fn stream_mjpeg(mut stream: TcpStream, bot: &Arc<Bot>) {
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
            .map(|f| f.downscale(720))
            .and_then(|f| f.to_jpeg_bytes(70).ok());

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

        std::thread::sleep(Duration::from_millis(50));
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
    });

    let body = payload.to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn send_screenshot(stream: &mut TcpStream, bot: &Arc<Bot>) {
    let fresh_bytes = bot
        .ctx()
        .roblox_rect()
        .and_then(|r| bot.ctx().platform.capture.grab(r).ok())
        .map(|f| f.downscale(760))
        .and_then(|f| f.to_jpeg_bytes(75).ok());

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

fn handle_key(stream: &mut TcpStream, bot: &Arc<Bot>, body: &str) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(json!({}));
    let key_str = parsed.get("key").and_then(|v| v.as_str()).unwrap_or("");
    let is_down = parsed.get("down").and_then(|v| v.as_bool()).unwrap_or(true);
    let tap = parsed.get("tap").and_then(|v| v.as_bool()).unwrap_or(false);

    // Ensure Roblox has window focus before sending keystroke
    let _ = bot.ctx().platform.window.focus();

    if key_str == "release_all" {
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
            } else {
                bot.ctx().platform.input.key(k, is_down);
                // Safety watchdog: auto-release held key after 5s to avoid permanent stuck walking
                if is_down {
                    let ctx_clone = bot.ctx().clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs(5));
                        ctx_clone.platform.input.key(k, false);
                    });
                }
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

fn send_html(stream: &mut TcpStream) {
    let html = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no">
<title>GPO Autofish CyberDeck</title>
<script src="https://telegram.org/js/telegram-web-app.js"></script>
<style>
:root {
  --bg: #07090e;
  --card: rgba(16, 22, 34, 0.85);
  --border: rgba(30, 41, 59, 0.8);
  --border-focus: #00f0ff;
  --cyan: #00f0ff;
  --purple: #b026ff;
  --emerald: #10b981;
  --amber: #f59e0b;
  --rose: #f43f5e;
  --text: #f8fafc;
  --text-dim: #94a3b8;
  --text-mute: #64748b;
}

* { box-sizing: border-box; margin: 0; padding: 0; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, sans-serif; -webkit-tap-highlight-color: transparent; }
body { background: var(--bg); color: var(--text); padding: 14px; min-height: 100vh; background-image: radial-gradient(circle at 50% 0%, rgba(0, 240, 255, 0.05), transparent 40%), radial-gradient(circle at 100% 100%, rgba(176, 38, 255, 0.04), transparent 40%); }
.container { max-width: 960px; margin: 0 auto; display: flex; flex-direction: column; gap: 14px; }

header {
  display: flex; justify-content: space-between; align-items: center; padding: 14px 18px;
  background: var(--card); border: 1px solid var(--border); border-radius: 16px;
  backdrop-filter: blur(16px); box-shadow: 0 8px 32px rgba(0,0,0,0.4);
}
.brand { display: flex; align-items: center; gap: 12px; }
.brand-icon { font-size: 1.6rem; filter: drop-shadow(0 0 10px rgba(0,240,255,0.4)); }
.brand h1 { font-size: 1.2rem; font-weight: 800; letter-spacing: -0.5px; background: linear-gradient(135deg, var(--cyan), var(--purple)); -webkit-background-clip: text; -webkit-text-fill-color: transparent; }
.brand-sub { font-size: 0.72rem; color: var(--text-mute); font-family: monospace; }

.header-badges { display: flex; align-items: center; gap: 8px; }
.status-badge {
  padding: 6px 14px; border-radius: 999px; font-size: 0.75rem; font-weight: 700;
  letter-spacing: 0.5px; text-transform: uppercase; display: flex; align-items: center; gap: 6px;
  border: 1px solid transparent; transition: all 0.3s ease;
}
.badge-running { background: rgba(16, 185, 129, 0.15); color: var(--emerald); border-color: rgba(16, 185, 129, 0.4); box-shadow: 0 0 16px rgba(16, 185, 129, 0.2); }
.badge-paused { background: rgba(245, 158, 11, 0.15); color: var(--amber); border-color: rgba(245, 158, 11, 0.4); box-shadow: 0 0 16px rgba(245, 158, 11, 0.2); }
.badge-stopped { background: rgba(100, 116, 139, 0.15); color: var(--text-dim); border-color: rgba(100, 116, 139, 0.3); }

.toggle-spawn-badge {
  cursor: pointer; padding: 6px 12px; border-radius: 999px; font-size: 0.75rem; font-weight: 700;
  background: rgba(176, 38, 255, 0.15); color: #d8b4fe; border: 1px solid rgba(176, 38, 255, 0.4);
  transition: all 0.2s ease; user-select: none;
}
.toggle-spawn-badge.off {
  background: rgba(100, 116, 139, 0.15); color: var(--text-mute); border-color: rgba(100, 116, 139, 0.3);
}

/* SCREEN STREAM & TAP-TO-CONTROL */
.stream-wrapper {
  background: var(--card); border: 1px solid var(--border); border-radius: 16px; overflow: hidden;
  position: relative; box-shadow: 0 12px 36px rgba(0,0,0,0.5);
}
.stream-bar {
  display: flex; justify-content: space-between; align-items: center; padding: 10px 16px;
  background: rgba(0,0,0,0.4); border-bottom: 1px solid var(--border); font-size: 0.75rem; font-weight: 600;
}
.stream-indicator { display: flex; align-items: center; gap: 8px; }
.live-dot { width: 8px; height: 8px; border-radius: 50%; background: #64748b; transition: all 0.3s; }
.live-dot.on { background: var(--emerald); box-shadow: 0 0 10px var(--emerald); animation: pulseDot 1.5s infinite; }
@keyframes pulseDot { 0%, 100% { opacity: 1; } 50% { opacity: 0.3; } }

.screen-box {
  width: 100%; min-height: 240px; background: #030508; display: flex; align-items: center; justify-content: center;
  position: relative; overflow: hidden; cursor: crosshair;
}
.screen-img {
  width: 100%; max-height: 480px; object-fit: contain; display: block; user-select: none;
}
.click-ripple {
  position: absolute; width: 24px; height: 24px; border-radius: 50%;
  border: 2px solid var(--cyan); background: rgba(0, 240, 255, 0.3);
  transform: translate(-50%, -50%) scale(0.2); pointer-events: none;
  animation: ripple 0.4s ease-out forwards;
}
@keyframes ripple {
  0% { transform: translate(-50%, -50%) scale(0.2); opacity: 1; }
  100% { transform: translate(-50%, -50%) scale(2.2); opacity: 0; }
}

/* ACTIONS */
.action-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(130px, 1fr)); gap: 10px; }
.btn {
  padding: 12px 16px; border-radius: 12px; border: 1px solid transparent; font-size: 0.88rem; font-weight: 700;
  cursor: pointer; display: flex; align-items: center; justify-content: center; gap: 8px; transition: all 0.18s ease;
  user-select: none;
}
.btn:active { transform: scale(0.96); }
.btn-toggle { background: linear-gradient(135deg, #00d2ff, #0084ff); color: #fff; box-shadow: 0 4px 18px rgba(0, 140, 255, 0.35); }
.btn-toggle.paused { background: linear-gradient(135deg, #f59e0b, #d97706); box-shadow: 0 4px 18px rgba(245, 158, 11, 0.35); }
.btn-sub { background: rgba(30, 41, 59, 0.6); color: var(--text); border-color: var(--border); }
.btn-sub:hover { background: rgba(51, 65, 85, 0.8); border-color: rgba(255,255,255,0.1); }
.btn-update { background: rgba(176, 38, 255, 0.15); color: #d8b4fe; border-color: rgba(176, 38, 255, 0.4); }

/* VIRTUAL CONTROLLER */
.controller-card {
  background: var(--card); border: 1px solid var(--border); border-radius: 14px; padding: 14px;
  display: flex; flex-direction: column; gap: 10px;
}
.controller-layout {
  display: flex; flex-wrap: wrap; justify-content: space-around; align-items: flex-start; gap: 14px;
}
.pad-cluster {
  display: flex; flex-direction: column; align-items: center; gap: 8px;
}
.cluster-label {
  font-size: 0.68rem; font-weight: 800; color: var(--text-dim); letter-spacing: 0.5px;
}
.dpad-grid {
  display: grid; grid-template-columns: repeat(3, 44px); grid-template-rows: repeat(2, 44px); gap: 6px;
}
.dpad-btn {
  background: rgba(30, 41, 59, 0.7); border: 1px solid var(--border); border-radius: 10px; color: var(--text);
  font-weight: 800; font-size: 1rem; display: flex; align-items: center; justify-content: center;
  cursor: pointer; user-select: none; -webkit-user-select: none; touch-action: none; -webkit-touch-callout: none;
  transition: all 0.1s;
}
.dpad-btn:active, .dpad-btn.pressed { background: var(--cyan); color: #000; box-shadow: 0 0 16px var(--cyan); }
.pad-arrow-btn {
  font-size: 1.15rem; color: #60a5fa; border-color: rgba(96, 165, 250, 0.3);
}
.pad-arrow-btn:active, .pad-arrow-btn.pressed {
  background: #3b82f6; color: #fff; box-shadow: 0 0 16px #3b82f6;
}

.action-buttons-pad {
  display: grid; grid-template-columns: repeat(2, 1fr); gap: 8px; width: 100%;
}
.pad-action-btn {
  padding: 10px 12px; background: rgba(30, 41, 59, 0.7); border: 1px solid var(--border); border-radius: 10px;
  color: var(--text); font-weight: 700; font-size: 0.78rem; cursor: pointer; user-select: none;
  -webkit-user-select: none; touch-action: none; -webkit-touch-callout: none;
  transition: all 0.1s; display: flex; align-items: center; justify-content: center; gap: 6px;
}
.pad-action-btn:active, .pad-action-btn.pressed { background: var(--purple); color: #fff; box-shadow: 0 0 16px var(--purple); }
.btn-shift { border-color: rgba(176, 38, 255, 0.4); color: #d8b4fe; }

/* STATS */
.stat-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(135px, 1fr)); gap: 10px; }
.card {
  background: var(--card); border: 1px solid var(--border); border-radius: 14px; padding: 14px;
  display: flex; flex-direction: column; gap: 6px; box-shadow: 0 4px 16px rgba(0,0,0,0.25);
  backdrop-filter: blur(12px);
}
.card-label { font-size: 0.72rem; color: var(--text-dim); text-transform: uppercase; font-weight: 700; letter-spacing: 0.5px; }
.card-val { font-size: 1.45rem; font-weight: 800; color: var(--text); }
.card-val.fruit { color: #d8b4fe; text-shadow: 0 0 16px rgba(176, 38, 255, 0.3); }
.card-meta { font-size: 0.72rem; color: var(--text-mute); }

/* SLIDERS */
.sliders-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 12px; }
.slider-group { display: flex; flex-direction: column; gap: 8px; }
.slider-head { display: flex; justify-content: space-between; font-size: 0.82rem; font-weight: 700; }
.slider-val { font-family: monospace; color: var(--cyan); }
input[type=range] {
  -webkit-appearance: none; width: 100%; height: 7px; border-radius: 4px; background: #1e293b; outline: none; cursor: pointer;
}
input[type=range]::-webkit-slider-thumb {
  -webkit-appearance: none; width: 18px; height: 18px; border-radius: 50%; background: var(--cyan); box-shadow: 0 0 12px var(--cyan); cursor: pointer; transition: transform 0.1s;
}
input[type=range]::-webkit-slider-thumb:active { transform: scale(1.2); }

/* BOSS TIMERS */
.boss-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(210px, 1fr)); gap: 10px; }
.boss-card {
  background: rgba(16, 22, 34, 0.7); border: 1px solid var(--border); border-radius: 12px; padding: 12px 14px;
  display: flex; justify-content: space-between; align-items: center;
}
.boss-title { font-size: 0.82rem; font-weight: 700; display: flex; align-items: center; gap: 8px; }
.boss-countdown { font-size: 0.84rem; font-weight: 800; font-family: monospace; color: var(--cyan); }
.boss-spawned { color: var(--emerald); text-shadow: 0 0 10px rgba(16, 185, 129, 0.6); animation: pulseSpawn 1.5s infinite; }
.boss-soon { color: var(--amber); }
@keyframes pulseSpawn { 0%, 100% { opacity: 1; } 50% { opacity: 0.4; } }

/* TOAST */
#toast {
  position: fixed; bottom: 20px; left: 50%; transform: translateX(-50%) translateY(100px);
  background: rgba(16, 22, 34, 0.95); border: 1px solid var(--border-focus); color: var(--text);
  padding: 10px 20px; border-radius: 999px; font-size: 0.82rem; font-weight: 700;
  box-shadow: 0 10px 30px rgba(0,0,0,0.8), 0 0 20px rgba(0,240,255,0.25);
  transition: transform 0.28s cubic-bezier(0.18, 0.89, 0.32, 1.28);
  pointer-events: none; z-index: 999;
}
#toast.show { transform: translateX(-50%) translateY(0); }
</style>
</head>
<body>
<div class="container">
  <header>
    <div class="brand">
      <span class="brand-icon">⚡</span>
      <div>
        <h1>GPO AUTOFISH</h1>
        <div class="brand-sub" id="host-sub">CONNECTING...</div>
      </div>
    </div>
    <div class="header-badges">
      <div id="badge-fruit" class="toggle-spawn-badge" onclick="toggleSpawnAlerts()" title="Toggle fruit spawn alerts on/off">🍇 FRUIT ALERTS: ON</div>
      <div id="status-pill" class="status-badge badge-stopped">STOPPED</div>
    </div>
  </header>

  <!-- LIVE VIDEO STREAM & SCREEN TOUCH -->
  <div class="stream-wrapper">
    <div class="stream-bar">
      <div class="stream-indicator">
        <div id="stream-dot" class="live-dot on"></div>
        <span>MJPEG LIVE STREAM (TAP TO CLICK)</span>
      </div>
      <div id="stream-fps" style="font-family: monospace; color: var(--cyan);">LIVE 20 FPS</div>
    </div>
    <div id="screen-container" class="screen-box">
      <img id="screen-img" class="screen-img" src="/api/stream" alt="" onerror="fallbackSnapshot()" />
    </div>
  </div>

  <!-- REMOTE GAMEPAD / MOVEMENT CONTROLLER -->
  <div class="controller-card">
    <div class="controller-layout">
      <!-- 1. Movement WASD -->
      <div class="pad-cluster">
        <div class="cluster-label">🏃 WALK (WASD)</div>
        <div class="dpad-grid">
          <div></div>
          <button class="dpad-btn" data-key="w" title="Walk Forward (W)">W</button>
          <div></div>
          <button class="dpad-btn" data-key="a" title="Walk Left (A)">A</button>
          <button class="dpad-btn" data-key="s" title="Walk Backward (S)">S</button>
          <button class="dpad-btn" data-key="d" title="Walk Right (D)">D</button>
        </div>
      </div>

      <!-- 2. Face / Look (Arrow Controls) -->
      <div class="pad-cluster">
        <div class="cluster-label">👀 FACE / TURN (ARROWS)</div>
        <div class="dpad-grid">
          <div></div>
          <button class="dpad-btn pad-arrow-btn" data-key="up" title="Face / Tilt Up (↑)">▲</button>
          <div></div>
          <button class="dpad-btn pad-arrow-btn" data-key="left" title="Turn Left (←)">◀</button>
          <button class="dpad-btn pad-arrow-btn" data-key="down" title="Face / Tilt Down (↓)">▼</button>
          <button class="dpad-btn pad-arrow-btn" data-key="right" title="Turn Right (→)">▶</button>
        </div>
      </div>

      <!-- 3. Actions -->
      <div class="pad-cluster" style="flex: 1; min-width: 140px;">
        <div class="cluster-label">⚡ ACTIONS & VIEW</div>
        <div class="action-buttons-pad">
          <button class="pad-action-btn btn-shift" data-key="shift">⚡ SHIFT-LOCK</button>
          <button class="pad-action-btn" data-key="space">🦘 JUMP (SPACE)</button>
          <button class="pad-action-btn" data-key="1">🎣 EQUIP ROD (1)</button>
          <button class="pad-action-btn" data-key="e">🖐️ INTERACT (E)</button>
          <button class="pad-action-btn" data-key="t" style="grid-column: span 2;">💬 TALK / ACTION (T)</button>
        </div>
      </div>
    </div>
  </div>

  <!-- MACRO ACTIONS -->
  <div class="action-grid">
    <button id="btn-toggle" class="btn btn-toggle" onclick="togglePlay()">
      <span id="toggle-icon">▶️</span>
      <span id="toggle-label">START MACRO</span>
    </button>
    <button class="btn btn-sub" onclick="doAction('recast')">🔄 RECAST</button>
    <button class="btn btn-sub" onclick="doAction('buy_bait')">🛒 BUY BAIT</button>
    <button id="btn-mute" class="btn btn-sub" onclick="toggleMute()">🔇 MUTE</button>
    <button class="btn btn-update" onclick="doUpdate()">🚀 UPDATE</button>
  </div>

  <!-- AUTO CRAFT BAIT (BLACKSMITH SEN) -->
  <div class="card" style="border-color: rgba(245, 158, 11, 0.35); background: rgba(245, 158, 11, 0.04);">
    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px;">
      <div class="card-label" style="color: var(--amber);">🔨 AUTO CRAFT BAIT (BLACKSMITH SEN)</div>
      <div id="craft-status-badge" class="status-badge badge-stopped" style="font-size: 0.7rem; padding: 3px 8px;">IDLE</div>
    </div>
    <div style="display: flex; flex-wrap: wrap; gap: 10px; align-items: center;">
      <select id="sel-craft-tier" style="background: #1e293b; color: #fff; border: 1px solid var(--border); border-radius: 10px; padding: 10px 14px; font-size: 0.85rem; font-weight: 700; outline: none; cursor: pointer; flex: 1; min-width: 160px;">
        <option value="rare">🍇 Rare Fish Bait</option>
        <option value="legendary">👑 Legendary Fish Bait</option>
        <option value="all">🌟 All (Legendary &amp; Rare)</option>
      </select>
      <button id="btn-craft-toggle" class="btn" style="background: linear-gradient(135deg, #f59e0b, #d97706); color: #000; flex: 1; min-width: 160px;" onclick="toggleAutoCraft()">
        <span id="craft-btn-icon">🔨</span>
        <span id="craft-btn-label">START AUTO CRAFT</span>
      </button>
    </div>
    <div id="craft-msg" style="font-size: 0.75rem; color: var(--text-mute); margin-top: 6px;">Stand at Blacksmith Sen with caught fish, then tap Start.</div>
  </div>

  <!-- PRIMARY STATS -->
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
      <div class="card-meta">Pity fish</div>
    </div>
    <div class="card">
      <div class="card-label">⏱️ Runtime</div>
      <div class="card-val" id="val-runtime" style="font-size: 1.25rem;">00:00:00</div>
      <div class="card-meta">Current session</div>
    </div>
  </div>

  <!-- BAIT AUTOMATION & RESTOCK -->
  <div class="stat-grid">
    <div class="card">
      <div class="card-label">🛒 Orders Placed</div>
      <div class="card-val" id="val-orders">0</div>
      <div class="card-meta" id="val-auto-buy">Auto-buy ON</div>
    </div>
    <div class="card">
      <div class="card-label">⏳ Next Restock</div>
      <div class="card-val" id="val-restock" style="color: var(--cyan); font-size: 1.25rem;">0 / 10</div>
      <div class="card-meta">Fish until purchase</div>
    </div>
    <div class="card">
      <div class="card-label">🎯 Active Tier</div>
      <div class="card-val" id="val-tier" style="color: #60a5fa; font-size: 1.2rem;">Common</div>
      <div class="card-meta">Purchase priority</div>
    </div>
    <div class="card">
      <div class="card-label">🛡️ Leg. Reserve</div>
      <div class="card-val" id="val-reserve" style="color: var(--amber);">0</div>
      <div class="card-meta">Protected stock</div>
    </div>
  </div>

  <!-- TOUCH-RELEASE SLIDERS: VOLUME & BRIGHTNESS -->
  <div class="card">
    <div class="sliders-grid">
      <div class="slider-group">
        <div class="slider-head">
          <span>🔊 WINDOWS AUDIO VOLUME</span>
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

  <!-- WORLD BOSSES -->
  <div class="card">
    <div class="card-label" style="margin-bottom: 8px;">👑 WORLD BOSS & MERCHANT COUNTDOWNS</div>
    <div id="boss-list" class="boss-grid">
      <div style="font-size: 0.8rem; color: var(--text-mute);">Loading timers...</div>
    </div>
  </div>
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
    if (spawnAlerts) {
      fruitBadge.className = 'toggle-spawn-badge';
      fruitBadge.innerText = '🍇 FRUIT ALERTS: ON';
    } else {
      fruitBadge.className = 'toggle-spawn-badge off';
      fruitBadge.innerText = '🍇 FRUIT ALERTS: OFF';
    }

    const pill = document.getElementById('status-pill');
    pill.innerText = d.state.toUpperCase();
    if (!isRunning) {
      pill.className = 'status-badge badge-stopped';
    } else if (isPaused) {
      pill.className = 'status-badge badge-paused';
    } else {
      pill.className = 'status-badge badge-running';
    }

    const tBtn = document.getElementById('btn-toggle');
    const tLabel = document.getElementById('toggle-label');
    const tIcon = document.getElementById('toggle-icon');
    if (isRunning && !isPaused) {
      tBtn.className = 'btn btn-toggle paused';
      tLabel.innerText = 'PAUSE MACRO';
      tIcon.innerText = '⏸️';
    } else {
      tBtn.className = 'btn btn-toggle';
      tLabel.innerText = isPaused ? 'RESUME MACRO' : 'START MACRO';
      tIcon.innerText = '▶️';
    }

    document.getElementById('btn-mute').innerText = isMuted ? '🔊 UNMUTE' : '🔇 MUTE';

    document.getElementById('val-fish').innerText = d.fish;
    document.getElementById('val-rate').innerText = `${d.success_rate}% catch rate`;
    document.getElementById('val-fruits').innerText = d.fruits;
    document.getElementById('val-pity').innerText = `Pity: ⚡ ${d.pity_fruit}`;
    document.getElementById('val-leg-pity').innerText = d.pity_legendary;

    document.getElementById('val-orders').innerText = d.bait_purchased;
    document.getElementById('val-auto-buy').innerText = d.auto_purchase ? 'Auto-buy ON' : 'Auto-buy OFF';
    document.getElementById('val-restock').innerText = `${d.since_purchase} / ${d.every_n_catches}`;
    document.getElementById('val-tier').innerText = d.bait_tier;
    document.getElementById('val-reserve').innerText = d.legendary_reserve;

    if (!userSlidingVol) {
      document.getElementById('rng-volume').value = d.volume;
      document.getElementById('lbl-volume').innerText = `${d.volume}%`;
    }
    if (!userSlidingBright) {
      document.getElementById('rng-brightness').value = d.brightness;
      document.getElementById('lbl-brightness').innerText = `${d.brightness}%`;
    }

    updateCraftUi(d.crafting);

    renderTimers();
  } catch (e) {
    console.warn('Status poll failed:', e);
  }
}

// SMOOTH HIGH-FREQUENCY CLIENT TIMER TICK (1s)
function tickTimersLocally() {
  if (isRunning && !isPaused) {
    localRuntimeSec += 1;
    document.getElementById('val-runtime').innerText = fmtSec(localRuntimeSec);
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
  document.getElementById('boss-list').innerHTML = bHtml;
}

// TAP TO CLICK DIRECTLY ON GAME SCREEN (ACCOUNTS FOR LETTERBOXING/PILLARBOXING)
const screenContainer = document.getElementById('screen-container');
screenContainer.addEventListener('pointerdown', handleScreenTap);

function handleScreenTap(e) {
  e.preventDefault();
  const img = document.getElementById('screen-img');
  if (!img) return;

  const rect = img.getBoundingClientRect();
  const clickX = e.clientX - rect.left;
  const clickY = e.clientY - rect.top;

  const naturalW = img.naturalWidth || 1280;
  const naturalH = img.naturalHeight || 720;
  const imageAspect = naturalW / naturalH;
  const elementAspect = rect.width / rect.height;

  let renderW = rect.width;
  let renderH = rect.height;
  let offsetX = 0;
  let offsetY = 0;

  if (elementAspect > imageAspect) {
    renderW = rect.height * imageAspect;
    offsetX = (rect.width - renderW) / 2;
  } else {
    renderH = rect.width / imageAspect;
    offsetY = (rect.height - renderH) / 2;
  }

  if (clickX < offsetX || clickX > (offsetX + renderW) ||
      clickY < offsetY || clickY > (offsetY + renderH)) {
    return;
  }

  const relX = Math.max(0, Math.min(1, (clickX - offsetX) / renderW));
  const relY = Math.max(0, Math.min(1, (clickY - offsetY) / renderH));

  // Visual Ripple
  const ripple = document.createElement('div');
  ripple.className = 'click-ripple';
  ripple.style.left = `${e.clientX - screenContainer.getBoundingClientRect().left}px`;
  ripple.style.top = `${e.clientY - screenContainer.getBoundingClientRect().top}px`;
  screenContainer.appendChild(ripple);
  setTimeout(() => ripple.remove(), 450);

  fetch('/api/click', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ rel_x: relX, rel_y: relY, button: 'left' })
  }).catch(() => {});
}

// BULLETPROOF REMOTE CONTROLLER WITH POINTER CAPTURE & AUTO-RELEASE
const activeKeys = new Set();

function sendKey(k, down, tap = false) {
  fetch('/api/key', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key: k, down, tap })
  }).catch(() => {});
}

function releaseAllKeys() {
  if (activeKeys.size === 0) return;
  activeKeys.clear();
  document.querySelectorAll('.dpad-btn.pressed, .pad-action-btn.pressed').forEach(b => b.classList.remove('pressed'));
  sendKey('release_all', false);
}

document.querySelectorAll('.dpad-btn').forEach(btn => {
  const key = btn.getAttribute('data-key');
  const isTap = btn.getAttribute('data-tap') === 'true';
  if (!key) return;

  if (isTap) {
    btn.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      btn.classList.add('pressed');
      sendKey(key, true, true);
      setTimeout(() => btn.classList.remove('pressed'), 140);
    });
    return;
  }

  btn.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    try { btn.setPointerCapture(e.pointerId); } catch (_) {}
    btn.classList.add('pressed');
    activeKeys.add(key);
    sendKey(key, true);
  });

  const onRelease = (e) => {
    e.preventDefault();
    try { btn.releasePointerCapture(e.pointerId); } catch (_) {}
    btn.classList.remove('pressed');
    if (activeKeys.has(key)) {
      activeKeys.delete(key);
      sendKey(key, false);
    }
  };

  btn.addEventListener('pointerup', onRelease);
  btn.addEventListener('pointercancel', onRelease);
  btn.addEventListener('pointerleave', onRelease);
});

document.querySelectorAll('.pad-action-btn').forEach(btn => {
  const key = btn.getAttribute('data-key');
  if (!key) return;

  btn.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    btn.classList.add('pressed');
    sendKey(key, true, true);
    setTimeout(() => btn.classList.remove('pressed'), 140);
  });
});

window.addEventListener('pointerup', releaseAllKeys);
window.addEventListener('pointercancel', releaseAllKeys);
window.addEventListener('blur', releaseAllKeys);
document.addEventListener('visibilitychange', () => {
  if (document.hidden) releaseAllKeys();
});

async function doAction(act, val = null) {
  try {
    const res = await fetch('/api/action', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: act, value: val !== null ? Number(val) : undefined })
    });
    const data = await res.json();
    showToast(data.message || 'Action executed');
    fetchStatus();
  } catch (e) {
    showToast('Failed: ' + e);
  }
}

function togglePlay() {
  if (isRunning && !isPaused) {
    doAction('pause');
  } else {
    doAction('start');
  }
}

function toggleMute() {
  if (isMuted) {
    doAction('unmute');
  } else {
    doAction('mute');
  }
}

function toggleSpawnAlerts() {
  doAction('toggle_spawn');
}

function doUpdate() {
  if (confirm('Check for update and restart macro? (If running, it will automatically resume fishing!)')) {
    doAction('update');
  }
}

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

async function toggleAutoCraft() {
  const tier = document.getElementById('sel-craft-tier').value;
  try {
    const res = await fetch('/api/craft', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: isCrafting ? 'stop' : 'start', tier })
    });
    const data = await res.json();
    showToast(data.message || (isCrafting ? 'Auto-craft stop requested' : 'Auto-craft started'));
    fetchStatus();
  } catch (e) {
    showToast('Failed: ' + e);
  }
}

// TOUCH-RELEASE SLIDERS
function onVolInput(val) {
  userSlidingVol = true;
  document.getElementById('lbl-volume').innerText = `${val}%`;
}
function onVolRelease(val) {
  userSlidingVol = false;
  doAction('set_volume', val);
}

function onBrightInput(val) {
  userSlidingBright = true;
  document.getElementById('lbl-brightness').innerText = `${val}%`;
}
function onBrightRelease(val) {
  userSlidingBright = false;
  doAction('set_brightness', val);
}

function fallbackSnapshot() {
  const img = document.getElementById('screen-img');
  img.src = '/api/screenshot?t=' + Date.now();
}

setInterval(fetchStatus, 1500);
setInterval(tickTimersLocally, 1000);
fetchStatus();
</script>
</body>
</html>
"#;

    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = stream.write_all(resp.as_bytes());
}
