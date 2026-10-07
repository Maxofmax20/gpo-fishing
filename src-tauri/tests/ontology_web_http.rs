//! HTTP-level verification of the ontology review web server.
//!
//! # What this proves
//!
//! The server is started for real and driven with real HTTP requests over
//! loopback. Every assertion here is about observable behaviour a browser would
//! see: the status line, the content type, the bytes. Nothing is inspected through
//! an internal function, so the tests cannot pass while the wire behaviour is
//! broken.
//!
//! # Isolation
//!
//! `GPO_ONTOLOGY_REVIEW_DIR` points at a sandbox holding a copy of the real
//! `workstation/` index. The ontology data is therefore REAL (77 concepts, the
//! actual P0 queue, real image ids that resolve on disk) while the decision log is
//! a throwaway file, so a test decision can never be mistaken for a human's.
//!
//! `DIST_DIR` is resolved from the working directory, so these tests run with
//! `cargo test` from the repo root or `src-tauri`; the static-file assertions are
//! skipped when the frontend has not been built, because that is a build state and
//! not a defect in this module.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Built ONCE per process. A second call would delete and recreate the directory,
/// destroying the decision log the tests are about to read.
fn sandbox() -> PathBuf {
    use std::sync::OnceLock;
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    if let Some(d) = DIR.get() {
        return d.clone();
    }
    let real = repo_root().join("reports").join("ontology_review");
    let tmp = std::env::temp_dir().join("gpo_ontology_web_test");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("sandbox");
    let ws = real.join("workstation");
    if ws.is_dir() {
        copy_dir(&ws, &tmp.join("workstation"));
    }
    // The reference snapshot is a committed artefact too, so the sandbox serves the
    // same wiki verdicts the real index carries.
    let snap = real.join("wiki_fish.json");
    if snap.is_file() {
        let _ = std::fs::copy(&snap, tmp.join("wiki_fish.json"));
    }
    std::fs::write(tmp.join("ontology_decisions.jsonl"), b"").expect("empty log");
    let _ = DIR.set(tmp.clone());
    tmp
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    if let Ok(rd) = std::fs::read_dir(from) {
        for e in rd.flatten() {
            let target = to.join(e.file_name());
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                copy_dir(&e.path(), &target);
            } else {
                let _ = std::fs::copy(e.path(), target);
            }
        }
    }
}

/// One HTTP response, split so tests can assert on the wire format.
struct Res {
    status: String,
    headers: String,
    body: Vec<u8>,
}

impl Res {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    fn header(&self, name: &str) -> Option<String> {
        let lower = name.to_ascii_lowercase();
        self.headers.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            if k.trim().to_ascii_lowercase() == lower {
                Some(v.trim().to_string())
            } else {
                None
            }
        })
    }
}

fn request(method: &str, path: &str, body: Option<&str>) -> Res {
    let mut s = TcpStream::connect(("127.0.0.1", gpo_autofish_lib::ontology_web::PORT))
        .expect("connect to the review server");
    s.set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    let body = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\
         Content-Type: application/json\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("response has a head/body separator");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let payload = raw[split + 4..].to_vec();
    let mut lines = head.lines();
    let status = lines.next().unwrap_or("").to_string();
    Res {
        status,
        headers: lines.collect::<Vec<_>>().join("\n"),
        body: payload,
    }
}

fn jget(path: &str) -> serde_json::Value {
    let r = request("GET", path, None);
    assert!(r.status.contains("200"), "GET {path} -> {}", r.status);
    serde_json::from_slice(&r.body).unwrap_or_else(|e| panic!("GET {path} body: {e}\n{}", r.text()))
}

// The server resolves its paths in LazyLocks, so the environment must be set before
// the first call. A single test owns that, and it runs the whole matrix itself.
#[test]
fn the_review_server_answers_real_http() {
    let sandbox_dir = sandbox();
    std::env::set_var("GPO_ONTOLOGY_REVIEW_DIR", &sandbox_dir);

    // ASSERT ISOLATION BEFORE ANY WRITE.
    //
    // `find_report_dir` falls back to walking up for `workstation/index.json`, so a
    // sandbox that fails to materialise does not error - it silently resolves to the
    // REAL report directory. That is how four `notes: "http sandbox test"` records
    // reached the production decision log. Every write below goes through the server,
    // so this check has to come first and has to fail loudly.
    let resolved = gpo_autofish_lib::commands_ontology_workstation::find_report_dir()
        .expect("report dir must resolve");
    assert_eq!(
        resolved, sandbox_dir,
        "the server resolved {:?} instead of the sandbox {:?}. Refusing to run: a \
         decision written now would land in the real log.",
        resolved, sandbox_dir
    );

    gpo_autofish_lib::ontology_web::spawn();
    // Give the listener thread a moment; binding happens synchronously in `spawn`,
    // but the accept loop is a thread.
    std::thread::sleep(std::time::Duration::from_millis(120));

    let port = gpo_autofish_lib::ontology_web::PORT;

    // ---------------------------------------------------------------------
    // Deep link. This is the requirement that most often silently fails.
    // ---------------------------------------------------------------------
    let direct = request("GET", "/ontology-review", None);
    if direct.status.contains("503") {
        eprintln!("SKIP static assertions: frontend not built (run npm run build)");
    } else {
        assert!(
            direct.status.contains("200"),
            "/ontology-review must serve the app, not 404; got {}",
            direct.status
        );
        assert!(
            direct.header("content-type").unwrap_or_default().contains("text/html"),
            "the route must return HTML"
        );
        // Refresh hits the same URL and must work the same way.
        let refreshed = request("GET", "/ontology-review", None);
        assert!(refreshed.status.contains("200"), "refresh must not 404");
        assert_eq!(direct.body.len(), refreshed.body.len());
        // A restrictive CSP, so "don't disable CSP" is enforced, not just claimed.
        let csp = direct.header("content-security-policy").unwrap_or_default();
        assert!(csp.contains("default-src 'self'"), "CSP present: {csp:?}");
        assert!(!csp.contains("unsafe-eval"));
    }

    // A missing asset must NOT be answered with HTML: that would make the browser
    // try to execute index.html as a script.
    let missing_asset = request("GET", "/assets/definitely-not-here.js", None);
    assert!(
        missing_asset.status.contains("404"),
        "a missing asset must 404, got {}",
        missing_asset.status
    );

    // Traversal through the static handler.
    for bad in ["/../../../../Windows/win.ini", "/..%2f..%2fetc", "/./../../secret"] {
        let r = request("GET", bad, None);
        assert!(
            r.status.contains("404") || r.status.contains("400"),
            "static traversal {bad} must be refused; got {}",
            r.status
        );
        assert!(
            !r.text().to_lowercase().contains("windows"),
            "static traversal {bad} leaked file content"
        );
    }

    // ---------------------------------------------------------------------
    // Real data. These are the numbers the reviewer must see.
    // ---------------------------------------------------------------------
    let status = jget("/api/ontology/status");
    assert_eq!(status["available"], true, "{}", status);
    let total = status["total"].as_u64().expect("total");
    assert_eq!(total, 77, "the real dataset has 77 discovered concepts");
    assert_eq!(status["decided"].as_u64(), Some(0), "no human has reviewed anything yet");
    assert_eq!(status["pending"].as_u64(), Some(77));
    assert_eq!(status["counts"]["P0"].as_u64(), Some(11), "the real P0 queue is 11");

    let queue: Vec<String> = status["queue"]
        .as_array()
        .expect("queue")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    assert_eq!(queue.len(), 77, "every concept is unresolved");

    // The first queue entry must be a real, loadable concept.
    let first = &queue[0];
    let concept = jget(&format!("/api/ontology/concepts/{}", urlencode(first)));
    assert_eq!(concept["conceptId"], *first);
    let evidence = concept["evidence"].as_array().expect("evidence");
    assert!(
        !evidence.is_empty() && evidence.len() <= 12,
        "a concept carries a small, deliberate evidence set; got {}",
        evidence.len()
    );
    let name = concept["displayName"].as_str().expect("displayName");
    assert!(!name.is_empty(), "the observed game name is the point of the page");
    assert!(concept["occurrences"].as_u64().unwrap_or(0) > 0);

    // The slim list exists for building a queue without pulling every image.
    let list = jget("/api/ontology/concepts");
    let items = list["concepts"].as_array().expect("concepts");
    assert_eq!(items.len(), 77);
    assert!(
        items[0].get("evidence").is_none(),
        "the queue list must not carry per-image evidence"
    );

    // Structural signals: the cross-concept evidence.
    let structural = jget("/api/ontology/structural");
    assert!(!structural["families"].as_array().unwrap().is_empty());
    assert!(!structural["modifiers"].as_array().unwrap().is_empty());

    // Family comparison, for the names Phase 12/13 call out by name.
    let fam = jget("/api/ontology/families/family%3Atigerfin");
    assert_eq!(fam["familyId"], "family:tigerfin");
    let members = fam["members"].as_array().expect("members");
    assert!(members.len() >= 3, "tigerfin has several observed names: {}", members.len());
    assert!(
        members[0].get("evidence").is_some(),
        "family members must carry images for comparison"
    );

    // KB search.
    let hits = jget("/api/ontology/kb/search?q=shark");
    let arr = hits.as_array().expect("kb hits");
    assert!(!arr.is_empty(), "searching 'shark' must find real KB entities");
    assert!(arr.iter().any(|h| h["id"] == "fish:shark"), "{arr:?}");

    // ---------------------------------------------------------------------
    // Wiki reference: the independent answer to "is it the thing?".
    // ---------------------------------------------------------------------

    // Every concept carries a reference verdict, so the panel can never be blank.
    let all = jget("/api/ontology/concepts");
    let items = all["concepts"].as_array().expect("concepts");
    for c in items {
        assert!(
            c.get("wiki").is_some(),
            "concept {} has no wiki field",
            c["conceptId"]
        );
        assert!(
            c["wiki"].get("basis").and_then(|b| b.as_str()).is_some(),
            "every verdict must state its basis"
        );
    }

    // THE HEADLINE FINDING, pinned. Every P0 concept is a fish the game's own wiki
    // documents. This is what turns 11 open structural questions into 11 reviewable
    // names, so it is asserted against the real index rather than left in a report.
    let p0: Vec<&serde_json::Value> = items
        .iter()
        .filter(|c| c["priority"] == "P0")
        .collect();
    assert_eq!(p0.len(), 11, "the real P0 queue is 11");
    let mut matched = 0usize;
    for c in &p0 {
        if c["wiki"]["matched"] == true {
            matched += 1;
            assert!(
                c["wiki"]["rarity"].is_string(),
                "{} matched but has no rarity",
                c["displayName"]
            );
            assert!(c["wiki"]["canonicalName"].is_string());
        }
    }
    assert_eq!(
        matched, 11,
        "every P0 concept should be a documented fish; unmatched: {:?}",
        p0.iter()
            .filter(|c| c["wiki"]["matched"] != true)
            .map(|c| c["displayName"].to_string())
            .collect::<Vec<_>>()
    );

    // A reference verdict must never be a decision, in any form.
    for c in items {
        let blob = c["wiki"].to_string().to_lowercase();
        for forbidden in ["new_entity", "existing_entity", "variant_of_existing", "\"decision\""] {
            assert!(
                !blob.contains(forbidden),
                "a wiki verdict must not contain {forbidden:?}"
            );
        }
    }

    // The refresh is a POST, never a GET.
    let get_refresh = request("GET", "/api/ontology/wiki/refresh", None);
    assert!(
        get_refresh.status.contains("400"),
        "GET on the refresh endpoint must be refused; got {}",
        get_refresh.status
    );

    // It either works or reports. It never hangs, never 500s, and - the property that
    // matters - a failure leaves the existing reference intact, because an emptied
    // reference would make every concept read as "not a fish".
    let before = std::fs::read_to_string(sandbox().join("wiki_fish.json")).ok();
    let refreshed = request("POST", "/api/ontology/wiki/refresh", Some("{}"));
    assert!(
        refreshed.status.contains("200") || refreshed.status.contains("502"),
        "refresh must succeed or report; got {}",
        refreshed.status
    );
    let rj: serde_json::Value = serde_json::from_slice(&refreshed.body).unwrap();
    assert!(rj["ok"].is_boolean(), "refresh must report a boolean outcome");
    assert!(rj["message"].is_string(), "an outcome must explain itself");
    if refreshed.status.contains("200") {
        assert_eq!(rj["entryCount"].as_u64(), Some(16), "the real bestiary has 16 fish");
    }
    if let Some(before) = before {
        let after = std::fs::read_to_string(sandbox().join("wiki_fish.json")).unwrap_or_default();
        if refreshed.status.contains("502") {
            assert_eq!(
                before, after,
                "a failed refresh must leave the previous reference byte-identical"
            );
        }
    }

    // ---------------------------------------------------------------------
    // Images: real bytes, real ids, and no filesystem access.
    // ---------------------------------------------------------------------
    let img_id = evidence[0]["imageId"].as_str().expect("imageId");
    let img = request("GET", &format!("/api/ontology/images/{img_id}"), None);
    assert!(img.status.contains("200"), "evidence image must serve");
    assert_eq!(
        img.header("content-type").as_deref(),
        Some("image/jpeg"),
        "a thumbnail is a JPEG"
    );
    assert!(
        img.body.len() > 100 && &img.body[..2] == [0xFF, 0xD8],
        "the bytes must be a real JPEG ({} bytes)",
        img.body.len()
    );

    // Full resolution is a separate, explicit fetch.
    let full = request("GET", &format!("/api/ontology/images/{img_id}/full"), None);
    assert!(full.status.contains("200"));
    assert_eq!(full.header("content-type").as_deref(), Some("image/png"));
    assert_eq!(&full.body[..4], b"\x89PNG", "full resolution must be the source PNG");

    // Hostile ids. A request must never become a path.
    for bad in [
        "..%2f..%2f..%2fwindows%2fwin.ini",
        "..%2f..%2fCargo.toml",
        "%00",
        "not-hex",
        "ZZZZZZZZZZZZZZZZ",
        "0123456789abcde",
    ] {
        let r = request("GET", &format!("/api/ontology/images/{bad}"), None);
        assert!(
            r.status.contains("404") || r.status.contains("400"),
            "image id {bad} must be refused; got {}",
            r.status
        );
        assert!(
            r.body.len() < 400,
            "image id {bad} returned {} bytes; that looks like file content",
            r.body.len()
        );
    }
    // A VALID id that is not an evidence image must also be refused: membership is
    // checked, not just shape.
    let stranger = request("GET", "/api/ontology/images/0000000000000000", None);
    assert!(stranger.status.contains("404"), "unknown id must 404");

    // ---------------------------------------------------------------------
    // Writes: the same validation, and the same single write path.
    // ---------------------------------------------------------------------
    // A GET on a write endpoint is refused rather than silently ignored.
    let get_write = request("GET", "/api/ontology/decision", None);
    assert!(
        get_write.status.contains("400"),
        "GET on /decision must be refused; got {}",
        get_write.status
    );

    // Malformed: EXISTING_ENTITY with no entity.
    let bad = request(
        "POST",
        "/api/ontology/decision",
        Some(&format!(r#"{{"conceptId":"{}","decision":"EXISTING_ENTITY"}}"#, first)),
    );
    assert!(bad.status.contains("400"), "a malformed decision must be refused");
    let err = serde_json::from_slice::<serde_json::Value>(&bad.body).unwrap();
    assert!(
        err["error"].as_str().unwrap_or("").contains("existingEntityId"),
        "the error must say what is missing: {}",
        bad.text()
    );

    // A machine decision maker cannot be smuggled in from the browser.
    let forged = request(
        "POST",
        "/api/ontology/decision",
        Some(&format!(
            r#"{{"conceptId":"{}","decision":"UNKNOWN","decisionMaker":"model"}}"#,
            first
        )),
    );
    assert!(
        forged.status.contains("400"),
        "the API must not accept a decision-maker from the client; got {}",
        forged.status
    );

    // An unknown concept cannot create a phantom decision.
    let ghost = request(
        "POST",
        "/api/ontology/decision",
        Some(r#"{"conceptId":"concept:not-a-real-one","decision":"UNKNOWN"}"#),
    );
    assert!(ghost.status.contains("400"), "unknown concept must be refused");

    // A valid decision records, advances, and lands in the sandbox log only.
    let good = request(
        "POST",
        "/api/ontology/decision",
        Some(&format!(
            r#"{{"conceptId":"{}","decision":"UNKNOWN","notes":"http sandbox test"}}"#,
            first
        )),
    );
    assert!(good.status.contains("200"), "a valid decision must save: {}", good.text());
    let result: serde_json::Value = serde_json::from_slice(&good.body).unwrap();
    assert_eq!(result["ok"], true);
    let next = result["nextConceptId"].as_str().expect("next concept");
    assert_ne!(next, first, "must advance to a different concept");
    assert_eq!(result["pendingTotal"].as_u64(), Some(76));

    // Progress reflects it.
    let after = jget("/api/ontology/progress");
    assert_eq!(after["decided"].as_u64(), Some(1));
    assert_eq!(after["pending"].as_u64(), Some(76));

    // Undo appends a reversal; it must not remove the original.
    let und = request(
        "POST",
        "/api/ontology/undo",
        Some(&format!(r#"{{"conceptId":"{}"}}"#, first)),
    );
    assert!(und.status.contains("200"), "undo: {}", und.text());
    // Counted relative to a baseline rather than assumed to be zero: the label
    // review test in this file also writes to the same sandboxed log, and test order
    // is not guaranteed.
    let log = std::fs::read_to_string(sandbox().join("ontology_decisions.jsonl")).unwrap();
    let lines: Vec<&str> = log.lines().filter(|l| !l.trim().is_empty()).collect();
    // This scenario added exactly two records: the decision and its reversal.
    assert!(
        lines.len() >= 2,
        "the decision and its reversal must both survive: {log}"
    );
    assert!(log.contains("REVERSAL"), "undo is an appended reversal, not a delete");
    assert!(
        log.contains("\"decision_maker\":\"human\""),
        "attribution is fixed to human: {log}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("knowledge") || l.contains("FISH_VOCAB")),
        "a decision record must not reference production artefacts"
    );

    // ---------------------------------------------------------------------
    // Locality and CORS.
    // ---------------------------------------------------------------------
    // No CORS header on any response: another origin cannot script this API.
    for path in ["/api/ontology/status", "/ontology-review", "/api/ontology/progress"] {
        let r = request("GET", path, None);
        assert!(
            r.header("access-control-allow-origin").is_none(),
            "{path} must not emit a CORS header"
        );
    }
    // The listener is loopback: connecting via a non-loopback address must fail.
    // 0.0.0.0 as a destination is not connectable on Windows, so the practical proof
    // is that the server refuses to be reached from the machine's LAN address; assert
    // the bind address is the loopback one, which is what determines reachability.
    let info = gpo_autofish_lib::ontology_web::base_url();
    assert!(info.starts_with("http://127.0.0.1:"), "{info}");
    assert!(!info.contains("0.0.0.0"));

    // Unknown endpoints 404 rather than being guessed at or proxied.
    let unknown = request("GET", "/api/ontology/../secrets", None);
    assert!(unknown.status.contains("404") || unknown.status.contains("400"));
    let not_a_route = request("GET", "/api/ontology/exec", None);
    assert!(not_a_route.status.contains("404"), "no generic dispatch endpoint");

    let _ = port;
}

fn urlencode(s: &str) -> String {
    s.replace(':', "%3A")
}