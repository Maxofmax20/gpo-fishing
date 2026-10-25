//! Rebuilt authenticated remote service. Transport and game inputs are separate.
use crate::{bot::Bot, config::Settings};
use gpo_control_runtime::InputAction;
use parking_lot::{Mutex, RwLock};
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
    control: Arc<super::control::Service>,
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
    let deadline = Instant::now() + Duration::from_secs(2);
    let set_budget = |stream: &TcpStream| -> Result<(), String> {
        let left = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or("Request deadline exceeded")?;
        stream
            .set_read_timeout(Some(left))
            .map_err(|_| "Socket timeout".into())
    };
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
        set_budget(stream)?;
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
        set_budget(stream)?;
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
    let Ok(control) = super::control::service() else {
        return;
    };
    let server = Arc::new(Server {
        control,
        bot,
        settings,
        streams: AtomicUsize::new(0),
        held: Mutex::new(vec![]),
        rates: Mutex::new(vec![]),
    });
    let expires = server.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(50));
        {
            let control = &expires.control;
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
                    "control_epoch":server.control.runtime.epoch(),"stream_fps":fps,"fps_options":gpo_control_runtime::STREAM_RATES}),
                    ),
                );
            }
            "/api/notifications/status" => {
                let (delivered, failed, error, pending) = server.bot.ctx().webhook.health();
                result(
                    &mut stream,
                    Ok(
                        json!({"ok":true,"delivered":delivered,"failed":failed,"error":error,"pending":pending}),
                    ),
                );
            }
            "/api/macros" => result(
                &mut stream,
                Ok(
                    json!({"ok":true,"macros":super::recorder::load_macros(&server.bot.ctx().store)}),
                ),
            ),
            "/api/screenshot" => match server
                .control
                .preview()
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
                "pause" => {
                    server.control.runtime.stop();
                    server.bot.pause();
                    super::recorder::stop_playback();
                }
                "stop" => {
                    server.control.runtime.stop();
                    server.bot.stop();
                    super::recorder::stop_playback();
                }
                "focus" => server.control.focus()?,
                _ => return Err("Unknown control command".into()),
            },
            "/api/tap" => {
                let key = body["key"].as_str().ok_or("Key is required")?;
                InputAction::KeyDown { key: key.into() }.validate()?;
                if super::recorder::is_playing() {
                    return Err("Stop playback before manual controls".into());
                }
                let control = server.control.clone();
                if body["epoch"].as_u64() != Some(control.runtime.epoch()) {
                    return Err("Input session changed; refresh the remote".into());
                }
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
                let mut action: InputAction = serde_json::from_value(body["input"].clone())
                    .map_err(|_| "Input action is invalid")?;
                match &mut action {
                    InputAction::KeyDown { key } | InputAction::KeyUp { key } => {
                        key.make_ascii_lowercase()
                    }
                    _ => {}
                }
                action.validate()?;
                if super::recorder::is_playing() {
                    return Err("Stop playback before manual controls".into());
                }
                let control = server.control.clone();
                if body["epoch"].as_u64() != Some(control.runtime.epoch()) {
                    return Err("Input session changed; refresh the remote".into());
                }
                if body["focus"].as_bool().unwrap_or(false) {
                    control.focus()?;
                }
                let epoch = body["epoch"]
                    .as_u64()
                    .ok_or("Refresh the remote page before sending inputs")?;
                let release = match &action {
                    InputAction::KeyDown { key } => Some(InputAction::KeyUp { key: key.clone() }),
                    InputAction::MouseDown { button } => Some(InputAction::MouseUp {
                        button: button.clone(),
                    }),
                    _ => None,
                };
                let mut held = server.held.lock();
                held.retain(|h| h.1 == epoch);
                if let Some(release) = &release {
                    if control
                        .platform
                        .window
                        .find()
                        .is_some_and(|w| w.visible && w.is_foreground)
                    {
                        if let Some(entry) = held.iter_mut().find(|h| h.0 == *release) {
                            entry.2 = Instant::now();
                            return Ok(json!({"ok":true,"renewed":true}));
                        }
                    }
                    if held.len() >= 32 {
                        return Err(
                            "Held-input limit reached; release another control first".into()
                        );
                    }
                }
                control.runtime.apply(epoch, &action)?;
                if let Some(release) = release {
                    held.push((release, epoch, Instant::now()));
                }
                if matches!(
                    action,
                    InputAction::KeyUp { .. } | InputAction::MouseUp { .. }
                ) {
                    held.retain(|h| h.0 != action);
                }
                drop(held);
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
                        if server.bot.is_running() {
                            return Err("Pause fishing before recording".into());
                        }
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
    let control = server.control.clone();
    let mut count = 0u32;
    let mut measured = Instant::now();
    let mut last_frame = Instant::now();
    loop {
        if !authorized(&request, &server.settings.read()) {
            break;
        }
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

#[cfg(test)]
mod remote_http_tests {
    use super::*;
    use crate::{
        config::Store,
        core::{
            platform::{
                mock::{InputEvent, MockWindow, NoOcr, RecordingInput, ScriptedCapture},
                Platform,
            },
            types::{Key, PxRect, WindowInfo},
        },
        webhook::WebhookQueue,
    };
    fn exchange(server: Arc<Server>, request: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let r = read_request(&mut socket).unwrap();
            handle(socket, r, server);
        });
        let mut client = TcpStream::connect(address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        client.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        worker.join().unwrap();
        response
    }
    fn post(token: &str, path: &str, body: Value) -> String {
        let text = body.to_string();
        format!("POST {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{text}",text.len())
    }
    #[test]
    fn incomplete_http_request_has_total_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let at = Instant::now();
            assert!(read_request(&mut socket).is_err());
            assert!(at.elapsed() < Duration::from_secs(3));
        });
        let mut client = TcpStream::connect(address).unwrap();
        client
            .write_all(
                b"POST /api/tap HTTP/1.1\r\nHost: localhost\r\nContent-Length: 20\r\n\r\n{",
            )
            .unwrap();
        for _ in 0..4{
            std::thread::sleep(Duration::from_millis(600));
            if client.write_all(b"x").is_err(){break;}
        }
        worker.join().unwrap();
    }
    #[test]
    fn authenticated_jump_and_cancel_use_real_http_with_mock_game() {
        let input = Arc::new(RecordingInput::default());
        let window = Arc::new(MockWindow::default());
        let info = WindowInfo {
            client: PxRect {
                x: 10,
                y: 20,
                w: 960,
                h: 540,
            },
            visible: true,
            is_foreground: true,
            dpi: 96,
        };
        *window.info.lock() = Some(info);
        let control = super::super::control::Service::new(Platform {
            window,
            capture: Arc::new(ScriptedCapture::default()),
            input: input.clone(),
            ocr: Arc::new(NoOcr),
        });
        let platform = control.platform.clone();
        let mut settings = Settings::default();
        settings.web.token = "synthetic-test-token".into();
        let settings = Arc::new(RwLock::new(settings));
        let (tx, _) = crossbeam_channel::unbounded();
        let dir = std::env::temp_dir().join(format!(
            "gpo-remote-http-{}-{}",
            std::process::id(),
            crate::events::now_ms()
        ));
        let bot = Bot::new(
            platform,
            settings.clone(),
            Arc::new(RwLock::new(Some(info))),
            tx,
            Arc::new(WebhookQueue::disabled()),
            Arc::new(Store::new(dir)),
        );
        let server = Arc::new(Server {
            control: control.clone(),
            bot,
            settings,
            streams: AtomicUsize::new(0),
            held: Mutex::new(vec![]),
            rates: Mutex::new(vec![]),
        });
        let epoch = control.runtime.epoch();
        let denied = exchange(
            server.clone(),
            post("wrong", "/api/tap", json!({"key":"space","epoch":epoch})),
        );
        assert!(denied.starts_with("HTTP/1.1 401"));
        assert!(input.events.lock().is_empty());
        let invalid = exchange(
            server.clone(),
            post(
                "synthetic-test-token",
                "/api/input",
                json!({"input":{"action":"move","x":2,"y":0},"epoch":epoch}),
            ),
        );
        assert!(invalid.starts_with("HTTP/1.1 400"));
        assert!(input.events.lock().is_empty());
        let jump = exchange(
            server.clone(),
            post(
                "synthetic-test-token",
                "/api/tap",
                json!({"key":"space","epoch":epoch}),
            ),
        );
        assert!(jump.starts_with("HTTP/1.1 200"), "{jump}");
        assert_eq!(
            *input.events.lock(),
            vec![
                InputEvent::Key(Key::Char(' '), true),
                InputEvent::Key(Key::Char(' '), false)
            ]
        );
        input.events.lock().clear();
        for key in (b'a'..=b'z').chain(b'0'..=b'5') {
            let response = exchange(
                server.clone(),
                post(
                    "synthetic-test-token",
                    "/api/input",
                    json!({"input":{"action":"key_down","key":(key as char).to_string()},"epoch":epoch}),
                ),
            );
            assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        }
        assert_eq!(input.events.lock().len(), 32);
        let renewed = exchange(
            server.clone(),
            post(
                "synthetic-test-token",
                "/api/input",
                json!({"input":{"action":"key_down","key":"A"},"epoch":epoch}),
            ),
        );
        assert!(renewed.starts_with("HTTP/1.1 200"));
        assert_eq!(input.events.lock().len(), 32);
        let full = exchange(
            server.clone(),
            post(
                "synthetic-test-token",
                "/api/input",
                json!({"input":{"action":"key_down","key":"6"},"epoch":epoch}),
            ),
        );
        assert!(full.starts_with("HTTP/1.1 400"));
        assert_eq!(input.events.lock().len(), 32);
        let stopped = exchange(
            server.clone(),
            post(
                "synthetic-test-token",
                "/api/control",
                json!({"command":"stop"}),
            ),
        );
        assert!(stopped.starts_with("HTTP/1.1 200"));
        input.events.lock().clear();
        let stale = exchange(
            server,
            post(
                "synthetic-test-token",
                "/api/tap",
                json!({"key":"space","epoch":epoch}),
            ),
        );
        assert!(stale.starts_with("HTTP/1.1 400"));
        assert!(input.events.lock().is_empty());
    }
}
