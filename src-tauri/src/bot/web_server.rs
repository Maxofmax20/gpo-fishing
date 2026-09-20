use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use parking_lot::RwLock;
use serde_json::json;
use crate::bot::Bot;
use crate::config::Settings;

pub fn get_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
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

    if path == "/" || path == "/index.html" {
        send_html(&mut stream);
    } else if path == "/api/status" {
        send_status(&mut stream, bot, settings);
    } else if path == "/api/screenshot" {
        send_screenshot(&mut stream, bot);
    } else if path == "/api/action" && method == "POST" {
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
        "runtime_s": stats.runtime_s,
        "fish": stats.fish,
        "fruits": stats.fruits,
        "success_rate": (stats.success_rate * 100.0).round() as u32,
        "pity_fruit": stats.pity_fruit,
        "pity_legendary": stats.pity_legendary,
        "last_fruit": stats.last_fruit.as_deref().unwrap_or("None yet"),
        "last_spawn": stats.last_spawn.as_deref().unwrap_or("None yet"),
        "bait_purchased": stats.bait_purchased,
        "volume": vol,
        "muted": muted,
        "brightness": brightness,
        "local_ip": local_ip,
        "port": 3888,
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
    let screenshot_bytes = bot
        .ctx()
        .roblox_rect()
        .and_then(|r| bot.ctx().platform.capture.grab(r).ok())
        .map(|f| f.downscale(960))
        .and_then(|f| f.to_png_bytes().ok());

    if let Some(bytes) = screenshot_bytes {
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-cache\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&bytes);
    } else {
        let msg = "No Roblox window active";
        let resp = format!(
            "HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            msg.len(),
            msg
        );
        let _ = stream.write_all(resp.as_bytes());
    }
}

fn handle_action(stream: &mut TcpStream, bot: &Arc<Bot>, _settings: &Arc<RwLock<Settings>>, body: &str) {
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
<meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0">
<title>GPO Autofish Web Dashboard</title>
<style>
:root {
  --bg: #0b0f19;
  --card: #131b2e;
  --card-border: #1e293b;
  --primary: #38bdf8;
  --accent: #818cf8;
  --success: #10b981;
  --warning: #f59e0b;
  --danger: #ef4444;
  --text: #f1f5f9;
  --text-dim: #94a3b8;
}
* { box-sizing: border-box; margin: 0; padding: 0; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif; }
body { background: var(--bg); color: var(--text); padding: 16px; min-height: 100vh; }
.container { max-width: 900px; margin: 0 auto; display: flex; flex-direction: column; gap: 16px; }

header { display: flex; justify-content: space-between; align-items: center; padding: 12px 16px; background: var(--card); border: 1px solid var(--card-border); border-radius: 12px; }
.logo-title { display: flex; align-items: center; gap: 10px; }
.logo-title h1 { font-size: 1.25rem; font-weight: 700; background: linear-gradient(135deg, var(--primary), var(--accent)); -webkit-background-clip: text; -webkit-text-fill-color: transparent; }
.badge { padding: 4px 10px; border-radius: 9999px; font-size: 0.75rem; font-weight: 600; text-transform: uppercase; }
.badge-active { background: rgba(16, 185, 129, 0.2); color: var(--success); border: 1px solid rgba(16, 185, 129, 0.4); }
.badge-paused { background: rgba(245, 158, 11, 0.2); color: var(--warning); border: 1px solid rgba(245, 158, 11, 0.4); }

.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 12px; }
.card { background: var(--card); border: 1px solid var(--card-border); border-radius: 12px; padding: 14px; display: flex; flex-direction: column; gap: 6px; }
.card-title { font-size: 0.75rem; color: var(--text-dim); text-transform: uppercase; font-weight: 600; letter-spacing: 0.5px; }
.card-value { font-size: 1.5rem; font-weight: 700; color: var(--text); }

.pity-box { display: flex; flex-direction: column; gap: 8px; }
.bar-track { width: 100%; height: 8px; background: #1e293b; border-radius: 4px; overflow: hidden; }
.bar-fill { height: 100%; background: linear-gradient(90deg, #3b82f6, #a855f7); width: 0%; transition: width 0.3s ease; }
.bar-legendary { background: linear-gradient(90deg, #f59e0b, #ef4444); }

.controls-card { display: flex; flex-wrap: wrap; gap: 10px; justify-content: stretch; }
button { flex: 1; min-width: 120px; padding: 12px 16px; border-radius: 8px; border: none; font-size: 0.9rem; font-weight: 600; cursor: pointer; transition: all 0.2s; display: flex; align-items: center; justify-content: center; gap: 6px; }
button:active { transform: scale(0.97); }
.btn-primary { background: #2563eb; color: #fff; }
.btn-primary:hover { background: #1d4ed8; }
.btn-warning { background: #d97706; color: #fff; }
.btn-warning:hover { background: #b45309; }
.btn-secondary { background: #334155; color: #f8fafc; }
.btn-secondary:hover { background: #475569; }

.sliders { display: flex; flex-direction: column; gap: 12px; }
.slider-group { display: flex; flex-direction: column; gap: 6px; }
.slider-header { display: flex; justify-content: space-between; font-size: 0.85rem; font-weight: 600; }
input[type=range] { width: 100%; accent-color: var(--primary); height: 6px; border-radius: 3px; cursor: pointer; }

.screen-card { position: relative; overflow: hidden; min-height: 200px; display: flex; align-items: center; justify-content: center; background: #000; border-radius: 12px; }
.screen-img { width: 100%; height: auto; max-height: 480px; object-fit: contain; border-radius: 12px; }
.screen-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; }

.boss-list { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 10px; }
.boss-card { background: rgba(30, 41, 59, 0.5); padding: 10px 12px; border-radius: 8px; border: 1px solid var(--card-border); display: flex; justify-content: space-between; align-items: center; }
.boss-name { font-size: 0.85rem; font-weight: 600; }
.boss-time { font-size: 0.85rem; font-weight: 700; color: var(--primary); }
.spawned-badge { color: #10b981; font-weight: 800; animation: pulse 1.5s infinite; }

@keyframes pulse { 0% { opacity: 1; } 50% { opacity: 0.4; } 100% { opacity: 1; } }
</style>
</head>
<body>
<div class="container">
  <header>
    <div class="logo-title">
      <span style="font-size: 1.5rem;">🎣</span>
      <div>
        <h1>GPO Autofish Web</h1>
        <div style="font-size: 0.75rem; color: var(--text-dim);" id="host-info">Connecting...</div>
      </div>
    </div>
    <div id="state-badge" class="badge badge-active">Loading...</div>
  </header>

  <div class="grid">
    <div class="card">
      <div class="card-title">🐟 Fish Caught</div>
      <div class="card-value" id="val-fish">0</div>
      <div style="font-size: 0.75rem; color: var(--text-dim);" id="val-rate">0% success</div>
    </div>
    <div class="card">
      <div class="card-title">🍇 Devil Fruits</div>
      <div class="card-value" id="val-fruits" style="color: #c084fc;">0</div>
      <div style="font-size: 0.75rem; color: var(--text-dim);" id="val-last-fruit">None yet</div>
    </div>
    <div class="card">
      <div class="card-title">🛒 Bait Purchases</div>
      <div class="card-value" id="val-bait">0</div>
      <div style="font-size: 0.75rem; color: var(--text-dim);">Merchant orders</div>
    </div>
    <div class="card">
      <div class="card-title">⏱️ Active Runtime</div>
      <div class="card-value" id="val-runtime" style="font-size: 1.25rem;">00:00:00</div>
      <div style="font-size: 0.75rem; color: var(--text-dim);">Live session</div>
    </div>
  </div>

  <div class="card">
    <div class="pity-box">
      <div style="display:flex; justify-content:space-between; font-size:0.8rem; font-weight:600;">
        <span>⚡ Fruit Pity</span>
        <span id="txt-fruit-pity">0 fish</span>
      </div>
      <div class="bar-track"><div class="bar-fill" id="bar-fruit" style="width: 10%;"></div></div>
    </div>
    <div class="pity-box" style="margin-top: 10px;">
      <div style="display:flex; justify-content:space-between; font-size:0.8rem; font-weight:600;">
        <span>🌟 Legendary Pity</span>
        <span id="txt-leg-pity">0/100</span>
      </div>
      <div class="bar-track"><div class="bar-fill bar-legendary" id="bar-leg" style="width: 0%;"></div></div>
    </div>
  </div>

  <div class="card controls-card">
    <button class="btn-primary" id="btn-toggle" onclick="togglePlay()">▶️ Start</button>
    <button class="btn-secondary" onclick="action('recast')">🔄 Recast Rod</button>
    <button class="btn-secondary" onclick="action('buy_bait')">🛒 Buy Bait Now</button>
  </div>

  <div class="card sliders">
    <div class="slider-group">
      <div class="slider-header">
        <span>🔊 Master Volume: <span id="lbl-volume">50%</span></span>
        <div>
          <button style="padding: 2px 8px; font-size: 0.75rem; min-width: auto; display:inline;" onclick="setVol(0)">0%</button>
          <button style="padding: 2px 8px; font-size: 0.75rem; min-width: auto; display:inline;" onclick="setVol(100)">100%</button>
        </div>
      </div>
      <input type="range" id="rng-volume" min="0" max="100" value="50" onchange="action('set_volume', this.value)">
    </div>

    <div class="slider-group">
      <div class="slider-header">
        <span>💡 Screen Brightness: <span id="lbl-brightness">80%</span></span>
        <div>
          <button style="padding: 2px 8px; font-size: 0.75rem; min-width: auto; display:inline;" onclick="setBright(0)">Min</button>
          <button style="padding: 2px 8px; font-size: 0.75rem; min-width: auto; display:inline;" onclick="setBright(100)">Max</button>
        </div>
      </div>
      <input type="range" id="rng-brightness" min="0" max="100" value="80" onchange="action('set_brightness', this.value)">
    </div>
  </div>

  <div class="card">
    <div class="screen-header">
      <span style="font-weight: 700; font-size: 0.95rem;">📸 Live Roblox Screen</span>
      <button style="min-width:auto; padding: 4px 10px; font-size: 0.75rem;" class="btn-secondary" onclick="refreshScreen()">🔄 Refresh</button>
    </div>
    <div class="screen-card">
      <img id="screen-img" class="screen-img" src="/api/screenshot" alt="Roblox Screen" onerror="this.style.display='none'">
    </div>
  </div>

  <div class="card">
    <div style="font-weight: 700; font-size: 0.95rem; margin-bottom: 8px;">👑 Live Boss & Merchant Timers</div>
    <div class="boss-list" id="boss-list">
      <div style="color: var(--text-dim); font-size: 0.8rem;">Loading timers...</div>
    </div>
  </div>
</div>

<script>
let isRunning = false;
let isPaused = false;

function fmtTime(sec) {
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = sec % 60;
  return `${h.toString().padStart(2, '0')}:${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
}

async function fetchStatus() {
  try {
    const res = await fetch('/api/status');
    const d = await res.json();
    
    isRunning = d.is_running;
    isPaused = d.paused;

    const b = document.getElementById('state-badge');
    b.innerText = d.state;
    if (d.paused || !d.is_running) {
      b.className = 'badge badge-paused';
    } else {
      b.className = 'badge badge-active';
    }

    const tBtn = document.getElementById('btn-toggle');
    if (d.is_running && !d.paused) {
      tBtn.innerText = '⏸️ Pause';
      tBtn.className = 'btn-warning';
    } else {
      tBtn.innerText = '▶️ Start';
      tBtn.className = 'btn-primary';
    }

    document.getElementById('host-info').innerText = `${d.local_ip}:${d.port} • Local Access`;
    document.getElementById('val-fish').innerText = d.fish;
    document.getElementById('val-rate').innerText = `${d.success_rate}% success`;
    document.getElementById('val-fruits').innerText = d.fruits;
    document.getElementById('val-last-fruit').innerText = `Last: ${d.last_fruit}`;
    document.getElementById('val-bait').innerText = d.bait_purchased;
    document.getElementById('val-runtime').innerText = fmtTime(d.runtime_s);

    document.getElementById('txt-fruit-pity').innerText = `${d.pity_fruit} fish`;
    document.getElementById('txt-leg-pity').innerText = `${d.pity_legendary}/100`;
    document.getElementById('bar-leg').style.width = `${Math.min(100, d.pity_legendary)}%`;

    if (!document.getElementById('rng-volume').matches(':active')) {
      document.getElementById('rng-volume').value = d.volume;
      document.getElementById('lbl-volume').innerText = d.muted ? '0% (Muted)' : `${d.volume}%`;
    }

    if (!document.getElementById('rng-brightness').matches(':active')) {
      document.getElementById('rng-brightness').value = d.brightness;
      document.getElementById('lbl-brightness').innerText = `${d.brightness}%`;
    }

    if (d.bosses && d.bosses.length > 0) {
      let bHtml = '';
      for (const boss of d.bosses) {
        let tStr = boss.next_spawn_fmt;
        if (boss.is_spawned) {
          tStr = `<span class="spawned-badge">SPAWNED (${boss.despawn_fmt})</span>`;
        } else if (boss.is_soon) {
          tStr = `<span style="color: #f59e0b; font-weight:700;">SOON (${boss.next_spawn_fmt})</span>`;
        }
        bHtml += `<div class="boss-card"><span class="boss-name">${boss.name}</span><span class="boss-time">${tStr}</span></div>`;
      }
      document.getElementById('boss-list').innerHTML = bHtml;
    }
  } catch (e) {
    console.error(e);
  }
}

async function action(act, val = null) {
  try {
    await fetch('/api/action', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: act, value: val !== null ? Number(val) : undefined })
    });
    fetchStatus();
  } catch (e) {
    alert('Action failed: ' + e);
  }
}

function togglePlay() {
  if (isRunning && !isPaused) {
    action('pause');
  } else {
    action('start');
  }
}

function setVol(val) {
  document.getElementById('rng-volume').value = val;
  document.getElementById('lbl-volume').innerText = `${val}%`;
  action('set_volume', val);
}

function setBright(val) {
  document.getElementById('rng-brightness').value = val;
  document.getElementById('lbl-brightness').innerText = `${val}%`;
  action('set_brightness', val);
}

function refreshScreen() {
  const img = document.getElementById('screen-img');
  img.style.display = 'block';
  img.src = '/api/screenshot?t=' + Date.now();
}

setInterval(fetchStatus, 2000);
setInterval(refreshScreen, 3500);
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
