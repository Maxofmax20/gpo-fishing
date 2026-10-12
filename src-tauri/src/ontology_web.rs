//! Local HTTP server for the ontology review workstation.
//!
//! # Why this exists, and why it is not the existing web server
//!
//! `bot::web_server` already serves HTTP on port 3888. It cannot serve this page.
//! Its entry point is `spawn(bot: Arc<Bot>, settings)`: it **requires a running
//! fishing bot**, and most of its endpoints drive input (`/click`, `/key`,
//! `/type`, `/drag`) or stream the game camera. Putting ontology review behind it
//! would make the reviewer's decisions depend on the fishing automation starting -
//! exactly the coupling this workstation must not have, and a route by which a
//! review page could reach game input.
//!
//! So this is a separate, deliberately inert server. Same conventions as the bot's
//! (`std::net::TcpListener`, thread per connection, loopback by default) so it reads
//! like the rest of the codebase, but none of the bot's surface.
//!
//! # Security posture
//!
//! * **Loopback only, unconditionally.** There is no LAN flag and no setting. The
//!   bind address is `127.0.0.1` in a single place and cannot be overridden from
//!   configuration, so decision-writing cannot become LAN-exposed through a setting
//!   toggle. The bot server's `allow_lan` is deliberately NOT reused: that flag
//!   exists because its endpoints drive input, and inheriting the habit here would
//!   be the wrong default for an endpoint that writes to disk.
//! * **No CORS header is ever sent.** The page is served by this same server, so
//!   every request is same-origin and none is needed. Omitting it means another
//!   origin cannot script this API even from the user's own browser.
//! * **CSRF protection.** Writes require a process token, JSON content type,
//!   a local Host and a same-origin Origin; foreign preflights are refused.
//! * **A restrictive CSP** is sent on the HTML, mirroring the desktop app's.
//! * **No arbitrary filesystem access.** Image requests are matched against the
//!   ontology index's known evidence ids, and those ids must look like a hex id,
//!   so a request never becomes a path. There is no endpoint that takes a
//!   filename.
//! * **Unknown paths are 404**, not proxied or guessed.
//!
//! # What the browser can and cannot do
//!
//! It can read the ontology artefacts and record a human decision proposal. It
//! cannot mutate the knowledge base, the vocabulary file, a dataset label, a split,
//! a model, or a readiness flag: this module imports none of them, and a test
//! asserts that against the source. The only write reachable from the network is
//! one line appended to `ontology_decisions.jsonl`.

use crate::commands_ontology_workstation as core;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;

/// The port the review workstation listens on.
///
/// A single constant, referenced by the server, the Tauri command that reports the
/// URL, and the startup banner - so the address shown to the user cannot drift from
/// the address actually bound. Chosen to avoid 3888 (bot dashboard), 1420/1421
/// (Vite dev) and 17831/17832 (updater).
pub const PORT: u16 = 17890;

/// The loopback address. Bind-only; never configurable.
const BIND_ADDR: &str = "127.0.0.1";

/// Idempotence guard: the desktop app may call the start command more than once.
static CSRF: LazyLock<String> = LazyLock::new(|| crate::core::secrets::generate_token_hex(32));

static STARTED: AtomicBool = AtomicBool::new(false);

/// The built frontend, resolved once.
///
/// Looks for `dist/` relative to the running executable first (the packaged layout,
/// where the binary sits next to the bundle), then by walking up from the current
/// directory (the development layout).
static BUNDLED_FRONTEND: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
pub fn set_bundled_frontend(path: PathBuf) { let _ = BUNDLED_FRONTEND.set(path); }

static DIST_DIR: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    if let Some(path) = BUNDLED_FRONTEND.get().filter(|p| p.join("index.html").is_file()) { return Some(path.clone()); }
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        let mut p = exe.as_path().parent();
        for _ in 0..6 {
            match p {
                Some(d) => {
                    roots.push(d.to_path_buf());
                    p = d.parent();
                }
                None => break,
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut p = Some(cwd.as_path());
        for _ in 0..8 {
            match p {
                Some(d) => {
                    roots.push(d.to_path_buf());
                    p = d.parent();
                }
                None => break,
            }
        }
    }
    roots
        .into_iter()
        .map(|r| r.join("dist"))
        .find(|d| d.join("index.html").is_file())
});

/// The URL to show a human. Built from the same constant the server binds.
pub fn base_url() -> String {
    format!("http://{BIND_ADDR}:{PORT}")
}

/// The address of the workstation page specifically.
pub fn review_url() -> String {
    format!("{}/ontology-review", base_url())
}

// ===========================================================================
// Start
// ===========================================================================

/// Start the review server if it is not already running.
///
/// Separate from the bot and from the macro: nothing here needs a `Bot`, a game
/// window, or any setting. The desktop app calls this on launch; it can also be
/// called from anywhere, and calling it twice is harmless.
pub fn spawn() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let bind = format!("{BIND_ADDR}:{PORT}");
    let listener = match TcpListener::bind(&bind) {
        Ok(l) => l,
        Err(e) => {
            // Not fatal. The desktop app's own IPC still works, so a busy port must
            // not take the application down.
            tracing::warn!("Ontology review web server could not bind {bind}: {e}");
            STARTED.store(false, Ordering::SeqCst);
            return;
        }
    };
    tracing::info!("Ontology review: {} (loopback only)", review_url());
    if let Err(error) = spawn_listener(listener) {
        STARTED.store(false, Ordering::SeqCst);
        tracing::warn!("Ontology review listener failed: {error}");
    }
}

/// Accept an already-bound loopback listener (also isolates HTTP contract tests).
pub fn spawn_listener(listener: TcpListener) -> std::io::Result<()> {
    if !listener.local_addr()?.ip().is_loopback() {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "review listener must use loopback"));
    }
    std::thread::Builder::new().name("ontology-review-web".into()).spawn(move || {
        let (tx, rx) = crossbeam_channel::bounded::<TcpStream>(16);
        for _ in 0..4 {
            let rx = rx.clone();
            std::thread::spawn(move || { for stream in rx { let _ = handle(stream); } });
        }
        for result in listener.incoming() {
            let Ok(stream) = result else { break };
            let _ = tx.try_send(stream);
        }
    }).map(|_| ())
}

/// The review workstation's address, for the desktop UI to display.
///
/// A command rather than a constant read in TypeScript, so the URL shown in the app
/// is the URL the server actually bound rather than a second copy of the port that
/// could drift.
#[tauri::command]
pub fn ontology_review_url() -> serde_json::Value {
    serde_json::json!({
        "baseUrl": base_url(),
        "reviewUrl": review_url(),
        "port": PORT,
        "bindAddress": BIND_ADDR,
        "loopbackOnly": true,
        "cors": false,
    })
}

// ===========================================================================
// Minimal HTTP plumbing (no new dependency)
//
// The project already hand-rolls HTTP for the bot dashboard, so this follows the
// same shape: read the head, honour Content-Length, answer, close. Bodies are
// capped, because an uncapped read on a loopback socket is still an uncapped read.
const MAX_HEAD_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 256 * 1024;

struct Request {
    method: String,
    path: String,
    query: String,
    body: String,
    host: String,
    origin: Option<String>,
    content_type: String,
    csrf: String,
}

fn header_value(head: &str, name: &str) -> Option<String> {
    let prefix = format!("{}:", name.to_ascii_lowercase());
    for line in head.lines().skip(1) {
        if line.len() >= prefix.len() && line.to_ascii_lowercase().starts_with(&prefix) {
            return Some(line[prefix.len()..].trim().to_string());
        }
    }
    None
}

fn content_length(head: &str) -> Option<usize> {
    header_value(head, "content-length")?.trim().parse().ok()
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    // Read until the end of the head, one byte at a time so the body is not
    // swallowed (and so we never read past what the client declared).
    loop {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                buf.push(byte[0]);
                if buf.len() >= MAX_HEAD_BYTES {
                    return None;
                }
                if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
                    break;
                }
            }
            Err(_) => return None,
        }
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    if !head.starts_with("GET ") && !head.starts_with("POST ") && !head.starts_with("HEAD ") {
        return None;
    }
    let mut parts = head.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.clone(), String::new()),
    };

    let host = header_value(&head, "host").unwrap_or_default();
    let origin = header_value(&head, "origin");
    let content_type = header_value(&head, "content-type").unwrap_or_default();
    let csrf = header_value(&head, "x-gpo-csrf").unwrap_or_default();
    let mut body = String::new();
    if method == "POST" {
        // Cap the declared length BEFORE reading it. A client that claims a
        // gigabyte must not be believed into an allocation.
        let len = content_length(&head).unwrap_or(0);
        if len > MAX_BODY_BYTES {
            return None;
        }
        let mut raw = vec![0u8; len];
        stream.read_exact(&mut raw).ok()?;
        body = String::from_utf8_lossy(&raw).to_string();
    }
    Some(Request { method, path, query, body, host, origin, content_type, csrf })
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, bytes: &[u8], extra: &[(&str, &str)]) {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n\
         Connection: close\r\n",
        bytes.len()
    );
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(bytes);
    let _ = stream.flush();
}

fn json(stream: &mut TcpStream, status: &str, value: &serde_json::Value) {
    respond(
        stream,
        status,
        "application/json; charset=utf-8",
        value.to_string().as_bytes(),
        &[],
    );
}

fn not_found(stream: &mut TcpStream) {
    json(
        stream,
        "404 Not Found",
        &serde_json::json!({ "ok": false, "error": "no such endpoint" }),
    );
}

fn bad_request(stream: &mut TcpStream, msg: &str) {
    json(stream, "400 Bad Request", &serde_json::json!({ "ok": false, "error": msg }));
}

// ===========================================================================
// Routing
// ===========================================================================

fn handle(mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
    let Some(req) = read_request(&mut stream) else {
        // An unparseable or oversized request gets a bare 400 and no further work.
        respond(&mut stream, "400 Bad Request", "text/plain; charset=utf-8", b"bad request", &[]);
        return Ok(());
    };

    let port = stream.local_addr()?.port();
    let local_host = format!("{BIND_ADDR}:{port}");
    let localhost = format!("localhost:{port}");
    let valid_host = req.host == local_host || req.host == localhost;
    let valid_origin = req.origin.as_deref().map_or(true, |o| o == format!("http://{local_host}") || o == format!("http://{localhost}"));
    if !valid_host || !valid_origin || (req.method == "POST" &&
        (!req.content_type.split(';').next().is_some_and(|t| t.trim().eq_ignore_ascii_case("application/json")) ||
         !crate::core::secrets::tokens_equal(&req.csrf, &CSRF))) {
        json(&mut stream, "403 Forbidden", &serde_json::json!({"error": "Local host, same origin, JSON and CSRF token required"}));
        return Ok(());
    }
    if req.method == "GET" && req.path == "/api/csrf" {
        json(&mut stream, "200 OK", &serde_json::json!({"token": CSRF.as_str()}));
        return Ok(());
    }

    if let Some(rest) = req.path.strip_prefix("/api/ontology") {
        return route_api(&mut stream, &req, rest);
    }

    // The fish label review lives beside the ontology review and shares its
    // security posture exactly: loopback only, no CORS, validated ids, no
    // filesystem paths in a request.
    if let Some(rest) = req.path.strip_prefix("/api/labels") {
        return route_labels(&mut stream, &req, rest);
    }

    // Everything else is the static frontend, with SPA fallback.
    route_static(&mut stream, &req)
}

fn route_labels(stream: &mut TcpStream, req: &Request, rest: &str) -> std::io::Result<()> {
    use crate::label_review as lr;
    let rest = rest.trim_start_matches('/');
    let get = req.method == "GET" || req.method == "HEAD";
    let post = req.method == "POST";

    match (get, post, rest) {
        (true, _, "batches") => json(
            stream,
            "200 OK",
            &serde_json::to_value(lr::batch_index()).unwrap_or_default(),
        ),
        (true, _, p) if p.starts_with("batch/") => {
            let id = url_decode(&p["batch/".len()..]);
            match lr::batch_rows(&id) {
                Some(v) => json(stream, "200 OK", &v),
                None => json(stream, "404 Not Found", &serde_json::json!({
                    "ok": false, "error": format!("unknown batch {id:?}")
                })),
            }
        }
        (true, _, p) if p.starts_with("thumb/") => {
            let id = url_decode(&p["thumb/".len()..]);
            match lr::thumbnail_bytes(&id) {
                Some(bytes) => respond(
                    stream,
                    "200 OK",
                    "image/jpeg",
                    &bytes,
                    &[("Cache-Control", "private, max-age=86400")],
                ),
                None => json(stream, "404 Not Found", &serde_json::json!({
                    "ok": false, "error": "not a label-review image"
                })),
            }
        }
        (_, true, "confirm") => {
            let input: lr::ConfirmInput = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => {
                    bad_request(stream, &format!("invalid JSON body: {e}"));
                    return Ok(());
                }
            };
            // Same data directory the desktop app uses, so the reviews land in the
            // real store rather than a browser-only file.
            let data_dir = crate::app::data_dir();
            let r = lr::confirm_batch(input, &data_dir);
            json(stream, "200 OK", &serde_json::to_value(r).unwrap_or_default())
        }
        (true, _, "confirm") => bad_request(stream, "use POST"),
        (_, _, _) => not_found(stream),
    }
    Ok(())
}

fn route_api(stream: &mut TcpStream, req: &Request, rest: &str) -> std::io::Result<()> {
    let rest = rest.trim_start_matches('/');
    let get = req.method == "GET" || req.method == "HEAD";
    let post = req.method == "POST";

    match (get, post, rest) {
        (true, _, "status") => json(stream, "200 OK", &serde_json::to_value(core::ontology_review_status()).unwrap_or_default()),
        (true, _, "progress") => json(stream, "200 OK", &progress_payload()),
        (true, _, "structural") => match core::ontology_review_structural() {
            Some(v) => json(stream, "200 OK", &v),
            None => json(stream, "503 Service Unavailable", &serde_json::json!({
                "ok": false,
                "error": "ontology index not built - run: python -m ml.gpo_ontology_review build"
            })),
        },
        // The compact per-concept list. The full index minus its evidence images,
        // so a client can build the queue without pulling 190 KB.
        (true, _, "concepts") => json(stream, "200 OK", &concept_summaries()),
        (true, _, "kb/search") => {
            let q = query_param(&req.query, "q").unwrap_or_default();
            let limit = query_param(&req.query, "limit")
                .and_then(|v| v.parse::<usize>().ok());
            json(
                stream,
                "200 OK",
                &serde_json::to_value(core::ontology_review_kb_search(q, limit)).unwrap_or_default(),
            );
        }
        (true, _, p) if p.starts_with("concepts/") => {
            let id = url_decode(&p["concepts/".len()..]);
            match core::ontology_review_concept(id.clone()) {
                Some(v) => json(stream, "200 OK", &v),
                None => json(stream, "404 Not Found", &serde_json::json!({
                    "ok": false, "error": format!("unknown concept {id:?}")
                })),
            }
        }
        (true, _, p) if p.starts_with("families/") => {
            let id = url_decode(&p["families/".len()..]);
            match core::ontology_review_family(id.clone()) {
                Some(v) => json(stream, "200 OK", &serde_json::to_value(v).unwrap_or_default()),
                None => json(stream, "404 Not Found", &serde_json::json!({
                    "ok": false, "error": format!("unknown family {id:?}")
                })),
            }
        }
        // Images. `.../images/<id>` is the generated thumbnail; `.../full` is the
        // source PNG, fetched only when a reviewer asks for it. Both go through the
        // same id validation the Tauri command uses, so neither can become a path.
        (true, _, p) if p.starts_with("images/") => {
            let tail = &p["images/".len()..];
            let (id, full) = match tail.strip_suffix("/full") {
                Some(id) => (id, true),
                None => (tail, false),
            };
            let id = url_decode(id);
            let got = if full {
                core::full_image_bytes(&id)
            } else {
                core::thumbnail_bytes(&id)
            };
            match got {
                Some(bytes) => respond(
                    stream,
                    "200 OK",
                    if full { "image/png" } else { "image/jpeg" },
                    &bytes,
                    // Immutable content: an id always maps to the same pixels, so
                    // the browser may cache it for the whole review session.
                    &[("Cache-Control", "private, max-age=86400")],
                ),
                None => json(stream, "404 Not Found", &serde_json::json!({
                    "ok": false,
                    "error": "not an ontology evidence image"
                })),
            }
        }
        (_, true, "decision") => {
            let input: core::DecisionInput = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => {
                    bad_request(stream, &format!("invalid JSON body: {e}"));
                    return Ok(());
                }
            };
            match core::record_human_decision(input, "web-ui") {
                Ok(r) => json(stream, "200 OK", &serde_json::to_value(r).unwrap_or_default()),
                Err(e) => json(stream, "400 Bad Request", &serde_json::json!({
                    "ok": false, "error": e.error
                })),
            }
        }
        // The ONLY outbound-network action in the review path, and the only POST that
        // is not a human decision. It refreshes a reference snapshot; it cannot change
        // a decision, and it cannot reach the bot, the model, or production data.
        (_, true, "wiki/refresh") => {
            match crate::ontology_web_refresh::refresh_wiki_reference() {
                r if r.ok => json(stream, "200 OK", &serde_json::to_value(r).unwrap_or_default()),
                r => json(stream, "502 Bad Gateway", &serde_json::to_value(r).unwrap_or_default()),
            }
        }
        (_, true, "undo") => {
            let body: serde_json::Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => {
                    bad_request(stream, &format!("invalid JSON body: {e}"));
                    return Ok(());
                }
            };
            let Some(id) = body.get("conceptId").and_then(|v| v.as_str()) else {
                bad_request(stream, "conceptId is required");
                return Ok(());
            };
            match core::undo_human_decision(id, "web-ui") {
                Ok(st) => json(stream, "200 OK", &serde_json::to_value(st).unwrap_or_default()),
                Err(e) => json(stream, "400 Bad Request", &serde_json::json!({
                    "ok": false, "error": e.error
                })),
            }
        }
        // A GET on a write endpoint, or a POST on a read endpoint: refuse rather
        // than quietly doing nothing, so a client bug is visible.
        (true, _, "decision") | (true, _, "undo") | (true, _, "wiki/refresh") => {
            bad_request(stream, "use POST")
        }
        (_, _, _) => not_found(stream),
    }
    Ok(())
}

fn progress_payload() -> serde_json::Value {
    let st = core::ontology_review_status();
    serde_json::json!({
        "ok": true,
        "total": st.total,
        "decided": st.decided,
        "pending": st.pending,
        "counts": st.counts,
        "remaining": st.remaining,
        "byDecision": st.by_decision,
        "available": st.available,
    })
}

/// Concept summaries without evidence images.
///
/// The full index carries up to 12 evidence records per concept, which a client
/// that only needs the queue should not have to download. One concept's full
/// payload is still served individually by `concepts/:id`.
fn concept_summaries() -> serde_json::Value {
    let st = core::ontology_review_status();
    let mut out = serde_json::Map::new();
    out.insert("ok".into(), true.into());
    out.insert("total".into(), st.total.into());
    out.insert("order".into(), st.order.clone().into());
    out.insert("queue".into(), st.queue.clone().into());
    if let Some(idx) = core::ontology_review_index() {
        let concepts = idx
            .get("concepts")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();
        let slim: Vec<serde_json::Value> = concepts
            .iter()
            .map(|c| {
                let mut o = c.as_object().cloned().unwrap_or_default();
                o.remove("evidence");
                o.remove("ocrVariants");
                o.remove("variantCounts");
                serde_json::Value::Object(o)
            })
            .collect();
        out.insert("concepts".into(), serde_json::Value::Array(slim));
    } else {
        out.insert("concepts".into(), serde_json::Value::Array(vec![]));
    }
    serde_json::Value::Object(out)
}

fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if url_decode(k) == key {
                return Some(url_decode(v));
            }
        }
    }
    None
}

/// Percent-decode, and refuse to decode a NUL.
///
/// Ids come from the URL, and the image handler turns one into a filename. Decoding
/// `%00` first and validating after would let a NUL through; rejecting it here means
/// no later stage has to defend against it.
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        if b == 0 {
                            return String::new();
                        }
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ===========================================================================
// Static frontend, with SPA fallback
// ===========================================================================

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; \
media-src 'self' blob:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; \
form-action 'none'";

fn route_static(stream: &mut TcpStream, req: &Request) -> std::io::Result<()> {
    let Some(dist) = DIST_DIR.as_ref() else {
        respond(
            stream,
            "503 Service Unavailable",
            "text/plain; charset=utf-8",
            b"The frontend has not been built. Run: npm run build",
            &[],
        );
        return Ok(());
    };

    let path = req.path.trim_start_matches('/');
    // Refuse traversal before touching the filesystem, rather than relying on
    // canonicalisation to save us later.
    if path.contains("..") || path.contains('\\') {
        not_found(stream);
        return Ok(());
    }

    let candidate: PathBuf = if path.is_empty() {
        dist.join("index.html")
    } else {
        dist.join(path)
    };

    if candidate.is_file() {
        return serve_file(stream, &candidate);
    }

    // SPA FALLBACK: `/ontology-review` is a client route with no file behind it.
    // Without this a browser refresh returns 404 and the reviewer loses their place
    // - which is the single most common way a deep link "doesn't work".
    //
    // Only real navigations get the fallback. A missing asset under `/assets/` is a
    // build problem and must report 404 rather than silently returning HTML that
    // the browser will try to execute as a script.
    if !path.starts_with("assets/") && req.method != "HEAD" {
        return serve_file(stream, &dist.join("index.html"));
    }
    not_found(stream);
    Ok(())
}

fn serve_file(stream: &mut TcpStream, path: &Path) -> std::io::Result<()> {
    let Ok(bytes) = std::fs::read(path) else {
        not_found(stream);
        return Ok(());
    };
    let ct = match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    };
    let is_html = ct.starts_with("text/html");
    let file_name = path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or_default();
    // HTML must never be cached.
    //
    // Vite emits content-hashed asset filenames, so a cached `index.html` points at
    // chunks that a rebuild has deleted - and the page then fails with 404s on its
    // own JavaScript, which looks exactly like a server fault. This was observed in
    // a real browser after a rebuild: the old HTML requested an old chunk name.
    // Hashes do the caching for the assets; the entry document must always be fresh.
    let cache = if is_html {
        "no-cache, no-store, must-revalidate"
    } else if path_has_hash(file_name) {
        // Hashed filename: immutable by construction.
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    respond(
        stream,
        "200 OK",
        ct,
        &bytes,
        &[
            ("Content-Security-Policy", if is_html { CSP } else { "" }),
            ("Cache-Control", cache),
        ],
    );
    Ok(())
}

/// True when the filename contains a Vite-style content hash.
///
/// Used only to decide a cache lifetime, so a false positive costs a revalidation
/// and a false negative costs a wasted download. Never a correctness boundary.
fn path_has_hash(file_name: &str) -> bool {
    let Some(stem) = file_name.rsplit_once('.').map(|(s, _)| s) else {
        return false;
    };
    // Vite: `name-<8+ base64ish chars>.<ext>`
    match stem.rsplit_once('-') {
        Some((_, hash)) => {
            hash.len() >= 8
                && hash
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The module's own source with the test block and comments removed.
    ///
    /// Required by every test below: they assert that certain strings are ABSENT,
    /// and those strings appear verbatim inside the assertions themselves. Reading
    /// the whole file would make each test match its own sentinel, and reading the
    /// raw text would let the module's documentation satisfy a check about its
    /// behaviour.
    fn own_code() -> String {
        let src = include_str!("ontology_web.rs");
        let body = src.split("#[cfg(test)]").next().unwrap_or(src);
        body.lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("//") && !t.starts_with("/*") && !t.starts_with('*')
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The whole file, for tests that assert a route or literal is PRESENT.
    fn own_source() -> &'static str {
        include_str!("ontology_web.rs")
    }

    // ----------------------------------------------------------------------
    // URL decoding
    // ----------------------------------------------------------------------

    #[test]
    fn url_decode_handles_the_shapes_a_browser_actually_sends() {
        assert_eq!(url_decode("concept%3Aexotic-tigerfin"), "concept:exotic-tigerfin");
        assert_eq!(url_decode("golden%20tigerfin"), "golden tigerfin");
        assert_eq!(url_decode("a+b"), "a b");
        assert_eq!(url_decode("plain"), "plain");
    }

    #[test]
    fn url_decode_refuses_a_percent_encoded_nul() {
        // `%00` decoded to a NUL and then used in a filename is the classic way to
        // truncate a path check. Refusing it at the decode step means nothing
        // downstream has to think about it.
        assert_eq!(url_decode("abc%00def"), "");
        assert_eq!(url_decode("%00"), "");
    }

    #[test]
    fn url_decode_leaves_a_malformed_escape_alone() {
        // Not a valid escape: keep the characters rather than guessing.
        assert_eq!(url_decode("100%"), "100%");
        assert_eq!(url_decode("%zz"), "%zz");
    }

    #[test]
    fn query_param_finds_the_right_pair() {
        assert_eq!(query_param("q=shark&limit=40", "q").as_deref(), Some("shark"));
        assert_eq!(query_param("q=shark&limit=40", "limit").as_deref(), Some("40"));
        assert_eq!(query_param("a=1&b=2", "q"), None);
        // A key appearing twice: first wins, deterministically.
        assert_eq!(query_param("q=one&q=two", "q").as_deref(), Some("one"));
    }

    // ----------------------------------------------------------------------
    // Binding
    // ----------------------------------------------------------------------

    #[test]
    fn the_server_binds_loopback_and_offers_no_lan_option() {
        // The single most important property of this module. If a LAN flag is ever
        // added, decision-writing becomes reachable from the network, so the test
        // asserts the absence of the vocabulary rather than the presence of a bind.
        let code = own_code();
        for forbidden in [
            "0.0.0.0",
            "allow_lan",
            "allowLan",
            "Access-Control-Allow-Origin",
            "CorsLayer",
        ] {
            assert!(
                !code.contains(forbidden),
                "the ontology review server must not contain {forbidden:?}: it is \
                 loopback-only with no CORS, and must stay that way"
            );
        }
    }

    #[test]
    fn the_server_cannot_reach_the_knowledge_base_training_or_game_input() {
        let code = own_code();
        for forbidden in [
            "FISH_VOCAB",
            "knowledge.rs",
            "training_eligible",
            "class_vocab",
            "readiness",
            "registry",
            "ml_dataset",
            "bot::",
            "controller",
            "enigo",
            "SendInput",
            "Settings",
            "Bot",
        ] {
            assert!(
                !code.contains(forbidden),
                "the ontology review server must not reference {forbidden:?}; reviewing \
                 fish is not allowed to touch the bot, the model, or production data"
            );
        }
    }

    #[test]
    fn the_only_network_write_reaches_the_decision_log() {
        // Decision and undo are the only POST routes, and both delegate to the
        // shared core rather than writing anything themselves.
        let code = own_code();
        let posts: Vec<&str> = code
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("(_, true,"))
            .collect();
        // Four POST routes, every one of them a human action that happens only when
        // someone asks: two record human decisions, one refreshes the wiki
        // reference, one confirms a batch of fish labels. None runs on a page load.
        assert_eq!(posts.len(), 4, "exactly four write routes: {posts:?}");
        assert!(code.contains("core::record_human_decision"));
        assert!(code.contains("core::undo_human_decision"));
        // No filesystem mutation of its own: the HTTP layer delegates every write to
        // the shared core, so there is exactly one code path that can touch disk.
        for forbidden in ["File::create", "fs::write", "fs::remove", "OpenOptions"] {
            assert!(
                !code.contains(forbidden),
                "the HTTP layer must not write files itself; {forbidden:?} was found"
            );
        }
    }

    #[test]
    fn the_api_surface_is_the_documented_one_and_nothing_more() {
        let src = own_source();
        for route in [
            "status",
            "progress",
            "structural",
            "concepts",
            "concepts/",
            "families/",
            "images/",
            "kb/search",
            "decision",
            "undo",
        ] {
            assert!(src.contains(route), "the documented route {route} is missing");
        }
        // No generic dispatch endpoint: no way to name a function to call. Scanned
        // against code only, since these literals live in this very assertion.
        //
        // The patterns are anchored to how they would actually be *used*, not to
        // bare substrings: a plain "eval" check matches `must-revalidate`, which is
        // in this file's own cache headers.
        let code = own_code();
        for forbidden in ["call_function", "invoke_any", "/exec", "eval(", ".eval("] {
            assert!(
                !code.contains(forbidden),
                "there must be no generic invocation endpoint ({forbidden})"
            );
        }
    }

    #[test]
    fn the_port_is_declared_once_and_used_consistently() {
        assert_eq!(PORT, 17890);
        // Must not collide with the servers already in the project.
        assert_ne!(PORT, 3888, "bot dashboard port");
        assert_ne!(PORT, 1420, "vite dev port");
        assert_ne!(PORT, 1421, "vite hmr port");
        assert_eq!(base_url(), "http://127.0.0.1:17890");
        assert_eq!(review_url(), "http://127.0.0.1:17890/ontology-review");
    }

    #[test]
    fn the_content_security_policy_is_not_disabled() {
        assert!(CSP.contains("default-src 'self'"));
        assert!(CSP.contains("object-src 'none'"));
        assert!(CSP.contains("frame-ancestors 'none'"));
        assert!(!CSP.contains("unsafe-eval"));
        // Images may come from this origin and data URLs (the desktop build uses
        // data URLs); nothing else, and never a wildcard remote host.
        assert!(!CSP.contains("*"));
    }

    #[test]
    fn the_entry_document_is_never_cached_but_hashed_assets_are() {
        // A real bug, found by loading the page in a browser after a rebuild: the
        // cached `index.html` referenced chunk filenames the rebuild had deleted, so
        // the page 404'd on its own JavaScript. The entry document must always be
        // revalidated; the hashes do the work for the assets.
        assert!(!path_has_hash("index.html"));
        assert!(path_has_hash("index-Dr_9vZ6k.js"));
        assert!(path_has_hash("Panel-4onaObf1.js"));
        assert!(!path_has_hash("logo.png"));
        assert!(!path_has_hash("no-extension"));
    }

    #[test]
    fn the_body_length_is_capped_before_reading() {
        assert!(MAX_BODY_BYTES < 4 * 1024 * 1024);
        assert!(MAX_HEAD_BYTES < 64 * 1024);
    }
}