//! Rebuilt authenticated remote service. Transport and game inputs are separate.
use crate::{bot::Bot, config::Settings};
use gpo_control_runtime::InputAction;
use parking_lot::{Mutex, RwLock};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

pub fn get_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}
struct Server {
    bot: Arc<Bot>,
    settings: Arc<RwLock<Settings>>,
    streams: AtomicUsize,
    held: Mutex<Vec<(InputAction, u64, Instant)>>,
    rates: Mutex<Vec<(String, f32, Instant)>>,
}
struct Request {
    method: String,
    path: String,
    query: String,
    auth: Option<String>,
    body: Vec<u8>,
}
fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| "Socket timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| "Socket timeout")?;
    let mut bytes = vec![];
    let split;
    loop {
        let mut chunk = [0; 2048];
        let n = stream.read(&mut chunk).map_err(|_| "Request read failed")?;
        if n == 0 {
            return Err("Request ended before its headers".into());
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.len() > 65536 {
            return Err("Request is too large".into());
        }
        if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            split = index;
            break;
        }
    }
    let headers = std::str::from_utf8(&bytes[..split]).map_err(|_| "Invalid headers")?;
    let mut lines = headers.split("\r\n");
    let mut start = lines
        .next()
        .ok_or("Missing request line")?
        .split_whitespace();
    let method = start.next().ok_or("Missing method")?.to_string();
    let target = start.next().ok_or("Missing path")?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut length = 0usize;
    let mut auth = None;
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            if key.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().map_err(|_| "Invalid content length")?;
            }
            if key.eq_ignore_ascii_case("authorization") {
                auth = value.trim().strip_prefix("Bearer ").map(str::to_string);
            }
        }
    }
    if length > 32768 {
        return Err("Request body exceeds 32 KiB".into());
    }
    let path = path.to_string();
    let query = query.to_string();
    let offset = split + 4;
    while bytes.len() - offset < length {
        let mut chunk = [0; 2048];
        let n = stream
            .read(&mut chunk)
            .map_err(|_| "Request body timed out")?;
        if n == 0 {
            return Err("Request body is incomplete".into());
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    Ok(Request {
        method,
        path,
        query,
        auth,
        body: bytes[offset..offset + length].to_vec(),
    })
}
fn query(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        part.split_once('=')
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| v.to_string())
    })
}
fn authorized(request: &Request, settings: &Settings) -> bool {
    let supplied = request
        .auth
        .clone()
        .or_else(|| query(&request.query, "token"))
        .unwrap_or_default();
    let expected = &settings.web.token;
    !expected.is_empty()
        && supplied.len() == expected.len()
        && supplied
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}
fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) {
    let header=format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",body.len());
    let _ = stream
        .write_all(header.as_bytes())
        .and_then(|_| stream.write_all(body));
}
fn result(stream: &mut TcpStream, value: Result<Value, String>) {
    match value {
        Ok(value) => respond(
            stream,
            "200 OK",
            "application/json",
            value.to_string().as_bytes(),
        ),
        Err(error) => respond(
            stream,
            "400 Bad Request",
            "application/json",
            json!({"ok":false,"error":error}).to_string().as_bytes(),
        ),
    }
}
pub fn spawn(bot: Arc<Bot>, settings: Arc<RwLock<Settings>>) {
    let server = Arc::new(Server {
        bot,
        settings,
        streams: AtomicUsize::new(0),
        held: Mutex::new(vec![]),
        rates: Mutex::new(vec![]),
    });
    let expires = server.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(50));
        if let Ok(control) = super::control::service() {
            expires.held.lock().retain(|(release, epoch, at)| {
                if at.elapsed() < Duration::from_millis(750)
                    && control
                        .platform
                        .window
                        .find()
                        .is_some_and(|w| w.is_foreground)
                {
                    true
                } else {
                    let _ = control.runtime.apply(*epoch, release);
                    false
                }
            });
        }
    });
    let (sender, receiver) = crossbeam_channel::bounded::<TcpStream>(16);
    for _ in 0..4 {
        let receiver = receiver.clone();
        let worker = server.clone();
        std::thread::spawn(move || {
            while let Ok(mut stream) = receiver.recv() {
                match read_request(&mut stream) {
                    Ok(request) => handle(stream, request, worker.clone()),
                    Err(error) => result(&mut stream, Err(error)),
                }
            }
        });
    }
    std::thread::spawn(move || {
        let address = if server.settings.read().web.allow_lan {
            "0.0.0.0:3888"
        } else {
            "127.0.0.1:3888"
        };
        match TcpListener::bind(address) {
            Ok(listener) => {
                for stream in listener.incoming().flatten() {
                    let _ = sender.try_send(stream);
                }
            }
            Err(_) => tracing::error!("Remote service could not bind port 3888"),
        }
    });
}
fn handle(mut stream: TcpStream, request: Request, server: Arc<Server>) {
    if request.method == "GET" {
        match request.path.as_str() {
            "/assets/remote-v2.css" => {
                respond(
                    &mut stream,
                    "200 OK",
                    "text/css",
                    include_bytes!("web/assets/remote-v2.css"),
                );
                return;
            }
            "/assets/remote-v2.js" => {
                respond(
                    &mut stream,
                    "200 OK",
                    "text/javascript",
                    include_bytes!("web/assets/remote-v2.js"),
                );
                return;
            }
            _ => {}
        }
    }
    if !authorized(&request, &server.settings.read()) {
        respond(
            &mut stream,
            "401 Unauthorized",
            "application/json",
            br#"{"ok":false,"error":"Open the authenticated link from the app"}"#,
        );
        return;
    }
    if request.method == "GET" {
        match request.path.as_str() {
            "/" | "/remote" | "/stats" | "/studio" | "/craft" | "/system" => respond(
                &mut stream,
                "200 OK",
                "text/html; charset=utf-8",
                include_bytes!("web/index.html"),
            ),
            "/assets/remote-v2.css" => respond(
                &mut stream,
                "200 OK",
                "text/css",
                include_bytes!("web/assets/remote-v2.css"),
            ),
            "/assets/remote-v2.js" => respond(
                &mut stream,
                "200 OK",
                "text/javascript",
                include_bytes!("web/assets/remote-v2.js"),
            ),
            "/api/status" => {
                let window = server.bot.ctx().platform.window.find();
                let session = query(&request.query, "session").unwrap_or_default();
                let fps = server
                    .rates
                    .lock()
                    .iter()
                    .find(|r| r.0 == session && r.2.elapsed() < Duration::from_secs(3))
                    .map(|r| r.1);
                result(
                    &mut stream,
                    Ok(
                        json!({"ok":true,"api_version":2,"state":server.bot.state(),"running":server.bot.is_running(),
                    "window":window,"stats":server.bot.ctx().session.lock().stats(),"recorder":super::recorder::get_status(),
                    "control_epoch":super::control::service().map(|c|c.runtime.epoch()).unwrap_or(0),"stream_fps":fps,"fps_options":gpo_control_runtime::STREAM_RATES}),
                    ),
                );
            }
            "/api/macros" => result(
                &mut stream,
                Ok(
                    json!({"ok":true,"macros":super::recorder::load_macros(&server.bot.ctx().store)}),
                ),
            ),
            "/api/screenshot" => match super::control::service()
                .and_then(|c| c.preview())
                .and_then(|f| f.downscale(1080).to_jpeg_bytes(85))
            {
                Ok(bytes) => respond(&mut stream, "200 OK", "image/jpeg", &bytes),
                Err(error) => result(&mut stream, Err(error)),
            },
            "/api/stream" => {
                if server.streams.fetch_add(1, Ordering::SeqCst) >= 2 {
                    server.streams.fetch_sub(1, Ordering::SeqCst);
                    respond(
                        &mut stream,
                        "503 Busy",
                        "text/plain",
                        b"Two stream clients are already connected",
                    );
                    return;
                }
                std::thread::spawn(move || stream_frames(stream, request, server));
            }
            _ => respond(
                &mut stream,
                "404 Not Found",
                "application/json",
                br#"{"ok":false,"error":"Unknown route"}"#,
            ),
        }
        return;
    }
    if request.method != "POST" {
        respond(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"Use GET or POST",
        );
        return;
    }
    let body = serde_json::from_slice::<Value>(&request.body).unwrap_or(Value::Null);
    let response = (|| -> Result<Value, String> {
        match request.path.as_str() {
            "/api/control" => match body["command"].as_str().ok_or("Command is required")? {
                "start" => server.bot.start_checked()?,
                "pause" => server.bot.pause(),
                "stop" => {
                    server.bot.stop();
                    super::recorder::stop_playback();
                }
                "focus" => super::control::service()?.focus()?,
                _ => return Err("Unknown control command".into()),
            },
            "/api/tap" => {
                let key = body["key"].as_str().ok_or("Key is required")?;
                if super::recorder::is_playing() {
                    return Err("Stop playback before manual controls".into());
                }
                let control = super::control::service()?;
                if body["focus"].as_bool().unwrap_or(false) {
                    control.focus()?;
                }
                control.tap_at(
                    body["epoch"]
                        .as_u64()
                        .ok_or("Refresh the remote page before sending inputs")?,
                    key,
                )?;
                if super::recorder::get_status().record_mode
                    == Some(super::recorder::RecordMode::WebScreen)
                {
                    super::recorder::record_key(key);
                }
            }
            "/api/input" => {
                let action: InputAction = serde_json::from_value(body["input"].clone())
                    .map_err(|_| "Input action is invalid")?;
                action.validate()?;
                if super::recorder::is_playing() {
                    return Err("Stop playback before manual controls".into());
                }
                let control = super::control::service()?;
                if body["focus"].as_bool().unwrap_or(false) {
                    control.focus()?;
                }
                let epoch = body["epoch"]
                    .as_u64()
                    .ok_or("Refresh the remote page before sending inputs")?;
                control.runtime.apply(epoch, &action)?;
                let release = match &action {
                    InputAction::KeyDown { key } => Some(InputAction::KeyUp { key: key.clone() }),
                    InputAction::MouseDown { button } => Some(InputAction::MouseUp {
                        button: button.clone(),
                    }),
                    _ => None,
                };
                if let Some(release) = release {
                    let mut held = server.held.lock();
                    held.retain(|h| h.0 != release);
                    if held.len() < 32 {
                        held.push((release, epoch, Instant::now()));
                    }
                }
                if matches!(
                    action,
                    InputAction::KeyUp { .. } | InputAction::MouseUp { .. }
                ) {
                    server.held.lock().retain(|h| h.0 != action);
                }
                if super::recorder::get_status().record_mode
                    == Some(super::recorder::RecordMode::WebScreen)
                {
                    super::recorder::record_input(action);
                }
            }
            "/api/recorder" => {
                match body["command"]
                    .as_str()
                    .ok_or("Recorder command is required")?
                {
                    "record" => {
                        super::recorder::start_recording(super::recorder::RecordMode::WebScreen)?
                    }
                    "save" => {
                        super::recorder::stop_recording(
                            body["name"].as_str().unwrap_or(""),
                            &server.bot.ctx().store,
                        )?;
                    }
                    "discard" => super::recorder::cancel_recording(),
                    "play" => super::recorder::play_macro(
                        server.bot.ctx().clone(),
                        server.bot.ctx().store.clone(),
                        body["id"].as_str().ok_or("Recording id is required")?,
                        body["loop"].as_bool().unwrap_or(false),
                        Some(body["speed"].as_f64().unwrap_or(1.0) as f32),
                        body["loops"].as_u64().and_then(|n| u32::try_from(n).ok()),
                    )?,
                    "stop" => super::recorder::stop_playback(),
                    _ => return Err("Unknown recorder command".into()),
                }
            }
            "/api/notifications/test" => server.bot.ctx().webhook.test()?,
            _ => return Err("Unknown action route".into()),
        }
        Ok(json!({"ok":true}))
    })();
    result(&mut stream, response);
}
fn stream_frames(mut stream: TcpStream, request: Request, server: Arc<Server>) {
    struct Permit(Arc<Server>);
    impl Drop for Permit {
        fn drop(&mut self) {
            self.0.streams.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let _permit = Permit(server.clone());
    let fps = query(&request.query, "fps")
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|n| gpo_control_runtime::STREAM_RATES.contains(n))
        .unwrap_or(20);
    let scale = query(&request.query, "scale")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(720)
        .clamp(240, 1080);
    let session = query(&request.query, "session")
        .filter(|s| s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or_default();
    let period = Duration::from_secs_f64(1.0 / f64::from(fps));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    if stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n").is_err(){return;}
    let Ok(control) = super::control::service() else {
        return;
    };
    let mut count = 0u32;
    let mut measured = Instant::now();
    let mut last_frame = Instant::now();
    loop {
        let started = Instant::now();
        match control
            .preview()
            .and_then(|f| f.downscale(scale).to_jpeg_bytes(75))
        {
            Ok(bytes) => {
                let header = format!(
                    "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                    bytes.len()
                );
                if stream
                    .write_all(header.as_bytes())
                    .and_then(|_| stream.write_all(&bytes))
                    .and_then(|_| stream.write_all(b"\r\n"))
                    .is_err()
                {
                    break;
                }
                count += 1;
                last_frame = Instant::now();
                if measured.elapsed() >= Duration::from_secs(1) {
                    let actual = count as f32 / measured.elapsed().as_secs_f32();
                    let mut rates = server.rates.lock();
                    rates.retain(|r| r.0 != session && r.2.elapsed() < Duration::from_secs(5));
                    if rates.len() >= 4 {
                        rates.remove(0);
                    }
                    rates.push((session.clone(), actual, Instant::now()));
                    count = 0;
                    measured = Instant::now();
                }
            }
            Err(_) => {
                if last_frame.elapsed() > Duration::from_millis(750) {
                    break;
                }
            }
        }
        std::thread::sleep(period.saturating_sub(started.elapsed()));
    }
}
