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
    } else if raw_path == "/api/screenshot" {
        send_screenshot(&mut stream, bot);
    } else if raw_path == "/api/action" && method == "POST" {
        let body = if let Some(idx) = req_str.find("\r\n\r\n") {
            &req_str[idx + 4..]
        } else {
            ""
        };
        handle_action(&mut stream, bot, settings, body);
    } else {
        let not_found = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(not_found.as_bytes());
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
        "last_fruit": stats.last_fruit,
        "last_spawn": stats.last_spawn,
        "volume": vol,
        "muted": muted,
        "brightness": brightness,
        "local_ip": local_ip,
        "version": env!("CARGO_PKG_VERSION"),
        "bosses": bosses_data,
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
        .map(|f| f.downscale(960))
        .and_then(|f| f.to_png_bytes().ok());

    if let Some(bytes) = fresh_bytes {
        *LAST_FRAME.write() = Some(bytes.clone());
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-cache, no-store, must-revalidate\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&bytes);
        return;
    }

    if let Some(cached) = LAST_FRAME.read().as_ref() {
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-cache, no-store, must-revalidate\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
  --card-glow: rgba(0, 240, 255, 0.08);
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

.status-badge {
  padding: 6px 14px; border-radius: 999px; font-size: 0.75rem; font-weight: 700;
  letter-spacing: 0.5px; text-transform: uppercase; display: flex; align-items: center; gap: 6px;
  border: 1px solid transparent; transition: all 0.3s ease;
}
.badge-running { background: rgba(16, 185, 129, 0.15); color: var(--emerald); border-color: rgba(16, 185, 129, 0.4); box-shadow: 0 0 16px rgba(16, 185, 129, 0.2); }
.badge-paused { background: rgba(245, 158, 11, 0.15); color: var(--amber); border-color: rgba(245, 158, 11, 0.4); box-shadow: 0 0 16px rgba(245, 158, 11, 0.2); }
.badge-stopped { background: rgba(100, 116, 139, 0.15); color: var(--text-dim); border-color: rgba(100, 116, 139, 0.3); }

/* SCREEN STREAM */
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
.live-dot.reconnecting { background: var(--amber); box-shadow: 0 0 10px var(--amber); }
@keyframes pulseDot { 0%, 100% { opacity: 1; } 50% { opacity: 0.3; } }

.screen-box {
  width: 100%; min-height: 240px; background: #030508; display: flex; align-items: center; justify-content: center;
  position: relative; overflow: hidden;
}
.screen-img {
  width: 100%; max-height: 480px; object-fit: contain; display: block;
  transition: opacity 0.2s ease;
}
.screen-loader {
  position: absolute; color: var(--text-mute); font-size: 0.85rem; font-weight: 600;
  display: flex; flex-direction: column; align-items: center; gap: 8px;
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
.btn-update:hover { background: rgba(176, 38, 255, 0.3); }

/* STATS */
.stat-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(135px, 1fr)); gap: 10px; }
.card {
  background: var(--card); border: 1px solid var(--border); border-radius: 14px; padding: 14px;
  display: flex; flex-direction: column; gap: 6px; box-shadow: 0 4px 16px rgba(0,0,0,0.25);
  backdrop-filter: blur(12px);
}
.card-label { font-size: 0.72rem; color: var(--text-dim); text-transform: uppercase; font-weight: 700; letter-spacing: 0.5px; }
.card-val { font-size: 1.45rem; font-weight: 800; color: var(--text); font-family: -apple-system, sans-serif; }
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
    <div id="status-pill" class="status-badge badge-stopped">STOPPED</div>
  </header>

  <!-- LIVE STREAM -->
  <div class="stream-wrapper">
    <div class="stream-bar">
      <div class="stream-indicator">
        <div id="stream-dot" class="live-dot on"></div>
        <span id="stream-status-text">LIVE FEED</span>
      </div>
      <div id="stream-fps" style="font-family: monospace; color: var(--text-mute);">ROBLOX MIRROR</div>
    </div>
    <div class="screen-box">
      <div id="screen-placeholder" class="screen-loader">
        <span style="font-size: 1.5rem;">🎮</span>
        <span>Awaiting Game Frame...</span>
      </div>
      <img id="screen-img" class="screen-img" alt="" style="opacity: 0;" />
    </div>
  </div>

  <!-- ACTION BUTTONS -->
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

  <!-- SLIDERS: VOLUME & BRIGHTNESS -->
  <div class="card">
    <div class="sliders-grid">
      <div class="slider-group">
        <div class="slider-head">
          <span>🔊 WINDOWS AUDIO VOLUME</span>
          <span id="lbl-volume" class="slider-val">50%</span>
        </div>
        <input id="rng-volume" type="range" min="0" max="100" value="50" oninput="onVolInput(this.value)" onchange="onVolChange(this.value)" />
      </div>

      <div class="slider-group">
        <div class="slider-head">
          <span>💡 SCREEN BRIGHTNESS</span>
          <span id="lbl-brightness" class="slider-val">80%</span>
        </div>
        <input id="rng-brightness" type="range" min="0" max="100" value="80" oninput="onBrightInput(this.value)" onchange="onBrightChange(this.value)" />
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
let screenLoading = false;
let userAdjustingVol = false;
let userAdjustingBright = false;

// Initialize Telegram Web App SDK if opened inside Telegram
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
  const sec = s % 60;
  return `${String(h).padStart(2,'0')}:${String(m).padStart(2,'0')}:${String(sec).padStart(2,'0')}`;
}

async function fetchStatus() {
  try {
    const res = await fetch('/api/status');
    const d = await res.json();

    isRunning = d.is_running;
    isPaused = d.paused;
    isMuted = d.muted;

    document.getElementById('host-sub').innerText = `${d.local_ip}:3888 • v${d.version}`;

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
    document.getElementById('val-runtime').innerText = fmtSec(d.runtime_s);

    document.getElementById('val-orders').innerText = d.bait_purchased;
    document.getElementById('val-auto-buy').innerText = d.auto_purchase ? 'Auto-buy ON' : 'Auto-buy OFF';
    document.getElementById('val-restock').innerText = `${d.since_purchase} / ${d.every_n_catches}`;
    document.getElementById('val-tier').innerText = d.bait_tier;
    document.getElementById('val-reserve').innerText = d.legendary_reserve;

    if (!userAdjustingVol) {
      document.getElementById('rng-volume').value = d.volume;
      document.getElementById('lbl-volume').innerText = `${d.volume}%`;
    }
    if (!userAdjustingBright) {
      document.getElementById('rng-brightness').value = d.brightness;
      document.getElementById('lbl-brightness').innerText = `${d.brightness}%`;
    }

    if (d.bosses && d.bosses.length > 0) {
      let bHtml = '';
      for (const b of d.bosses) {
        let tClass = 'boss-countdown';
        let tText = b.next_spawn_fmt;
        if (b.is_spawned) {
          tClass += ' boss-spawned';
          tText = 'SPAWNED NOW!';
        } else if (b.is_soon) {
          tClass += ' boss-soon';
          tText = `SOON (${b.next_spawn_fmt})`;
        }
        bHtml += `
          <div class="boss-card">
            <div class="boss-title"><span>${b.emoji}</span><span>${b.name}</span></div>
            <div class="${tClass}">${tText}</div>
          </div>`;
      }
      document.getElementById('boss-list').innerHTML = bHtml;
    }
  } catch (e) {
    console.warn('Status poll failed:', e);
  }
}

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

function doUpdate() {
  if (confirm('Check for update and restart macro? (If running, it will automatically resume fishing!)')) {
    doAction('update');
  }
}

function onVolInput(val) {
  userAdjustingVol = true;
  document.getElementById('lbl-volume').innerText = `${val}%`;
}
function onVolChange(val) {
  userAdjustingVol = false;
  doAction('set_volume', val);
}

function onBrightInput(val) {
  userAdjustingBright = true;
  document.getElementById('lbl-brightness').innerText = `${val}%`;
}
function onBrightChange(val) {
  userAdjustingBright = false;
  doAction('set_brightness', val);
}

// DOUBLE-BUFFERED IMAGE PRELOADER: NEVER GOES BLACK
function refreshScreen() {
  if (screenLoading) return;
  screenLoading = true;

  const preloader = new Image();
  preloader.onload = () => {
    const target = document.getElementById('screen-img');
    target.src = preloader.src;
    target.style.opacity = '1';
    document.getElementById('screen-placeholder').style.display = 'none';

    const dot = document.getElementById('stream-dot');
    dot.className = 'live-dot on';
    document.getElementById('stream-status-text').innerText = 'LIVE FEED';
    screenLoading = false;
  };

  preloader.onerror = () => {
    // Keep old frame visible! Do NOT clear canvas or show black screen
    const dot = document.getElementById('stream-dot');
    dot.className = 'live-dot reconnecting';
    document.getElementById('stream-status-text').innerText = 'STANDBY';
    screenLoading = false;
  };

  preloader.src = '/api/screenshot?t=' + Date.now();
}

setInterval(fetchStatus, 1000);
setInterval(refreshScreen, 1500);
fetchStatus();
refreshScreen();
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
