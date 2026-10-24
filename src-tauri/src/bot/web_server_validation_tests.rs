use super::*;
use super::tests::{drive, input_test_bot};
use crate::core::platform::mock::InputEvent;
use std::io::{Read, Write};

fn assert_framed_error(response: &str) {
    let (headers, body) = response.split_once("\r\n\r\n").expect("HTTP response");
    assert!(headers.starts_with("HTTP/1.1 400"), "{response}");
    let length: usize = header_value(response, "content-length").unwrap().parse().unwrap();
    assert_eq!(length, body.len(), "byte accurate response length");
    assert_eq!(serde_json::from_str::<serde_json::Value>(body).unwrap()["ok"], false);
}

#[test]
fn malformed_mutating_requests_have_no_input_or_job_side_effects() {
    let (bot, input, dir) = input_test_bot();
    let cases = [
        ("click", "not-json"), ("click", "{}"), ("click", r#"{"rx":0.5,"ry":null}"#),
        ("click", r#"{"rx":-0.1,"ry":0.5}"#), ("click", r#"{"rx":0.5,"ry":1.1}"#),
        ("click", r#"{"rx":0.5,"ry":0.5,"button":"middle"}"#),
        ("click", r#"{"rx":0.5,"x":0.6,"ry":0.5}"#),
        ("drag", "{}"), ("drag", r#"{"x1":0,"y1":0,"x2":1,"y2":1,"ms":18446744073709551615}"#),
        ("drag", r#"{"x1":0,"y1":0,"x2":1,"y2":1,"ms":-1}"#),
        ("drag", r#"{"x1":0,"y1":0,"x2":1,"y2":1,"ms":50,"record_only":"yes"}"#),
        ("mouse", "{}"), ("mouse", r#"{"action":"invalid"}"#),
        ("mouse", r#"{"action":"down","rx":0.5,"ry":0.5,"button":"unknown"}"#),
        ("key", "{}"), ("key", r#"{"key":"unrecognized"}"#), ("key", r#"{"key":"w","down":"false"}"#),
        ("type", "{}"), ("type", r#"{"text":15}"#),
        ("clipboard_set", "{}"), ("clipboard_set", r#"{"text":false}"#),
        ("craft", "{}"), ("craft", r#"{"action":"anything","tier":"rare"}"#),
        ("craft", r#"{"action":"start","tier":"unknown"}"#),
        ("craft", r#"{"action":"start","tier":"rare","target_batches":0}"#),
        ("craft", r#"{"action":"start","tier":"rare","target_batches":100}"#),
        ("play", "{}"), ("play", r#"{"action":"explode","name":"test"}"#),
        ("play", r#"{"action":"play","name":"test","speed":0}"#),
        ("play", r#"{"action":"loop","name":"test","max_loops":4294967296}"#),
    ];
    for (route, body) in cases {
        let response = drive(&bot, route, body);
        assert_framed_error(&response);
    }
    assert!(input.events.lock().is_empty());
    assert!(!crate::bot::crafting::is_crafting_running());
    assert!(!crate::bot::recorder::get_status().is_playing);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn oversize_text_and_clipboard_are_rejected_instead_of_silently_truncated() {
    let (bot, input, dir) = input_test_bot();
    for (route, size) in [("type", 501), ("clipboard_set", 4001)] {
        assert_framed_error(&drive(&bot, route, &json!({"text":"س".repeat(size)}).to_string()));
    }
    assert!(input.events.lock().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn drag_aliases_reach_requested_points_not_implicit_center_defaults() {
    let (bot, input, dir) = input_test_bot();
    let response = drive(&bot, "drag", r#"{"x1":0.25,"y1":0.5,"x2":0.75,"y2":0.25,"ms":60,"button":"right"}"#);
    assert!(response.starts_with("HTTP/1.1 200"));
    let events = input.events.lock();
    assert!(events.contains(&InputEvent::Move(crate::core::types::PxPoint { x: 480, y: 540 })));
    assert!(events.contains(&InputEvent::Move(crate::core::types::PxPoint { x: 1439, y: 270 })));
    assert!(events.contains(&InputEvent::Button(crate::core::types::MouseButton::Right, false)));
    drop(events);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn click_endpoints_stay_within_last_client_pixel_and_have_correct_length() {
    let (bot, input, dir) = input_test_bot();
    let response = drive(&bot, "click", r#"{"rx":1,"ry":1}"#);
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    assert_eq!(header_value(&response, "content-length").unwrap().parse::<usize>().unwrap(), body.len());
    assert!(input.events.lock().contains(&InputEvent::Move(crate::core::types::PxPoint { x: 1919, y: 1079 })));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_game_rejects_input_without_success_or_recording() {
    let (bot, input, dir) = input_test_bot();
    *bot.ctx().roblox.write() = None;
    assert_framed_error(&drive(&bot, "click", r#"{"rx":0.5,"ry":0.5}"#));
    assert!(input.events.lock().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

fn raw_request(parts: Vec<Vec<u8>>) -> Option<String> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let writer = thread::spawn(move || {
        let mut socket = TcpStream::connect(address).unwrap();
        for part in parts { if socket.write_all(&part).is_err() { break; } thread::sleep(Duration::from_millis(10)); }
    });
    let (mut stream, _) = listener.accept().unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let result = read_full_request(&mut stream);
    drop(stream);
    writer.join().unwrap();
    result
}

#[test]
fn http_parser_preserves_segmented_unicode_bytes() {
    let body = json!({"text":"العربية"}).to_string();
    let split = body.find('ا').unwrap() + 1;
    let mut first = format!("POST /api/type HTTP/1.1\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    first.extend_from_slice(&body.as_bytes()[..split]);
    let result = raw_request(vec![first, body.as_bytes()[split..].to_vec()]).unwrap();
    assert_eq!(result.split_once("\r\n\r\n").unwrap().1, body);
}

#[test]
fn http_parser_rejects_ambiguous_lengths_and_incomplete_or_invalid_bodies() {
    for request in [
        b"POST /api/click HTTP/1.1\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}".as_slice(),
        b"POST /api/click HTTP/1.1\r\nContent-Length: +2\r\n\r\n{}",
        b"POST /api/click HTTP/1.1\r\nContent-Length: 1048577\r\n\r\n",
        b"POST /api/click HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
        b"POST /api/click HTTP/1.1\r\nContent-Length: 3\r\n\r\n{}",
        b"POST /api/click HTTP/1.1\r\nContent-Length: 2\r\n\r\n\xff\xff",
    ] { assert!(raw_request(vec![request.to_vec()]).is_none()); }
}

#[test]
fn priority_lane_admits_stop_and_status_but_not_slow_or_unknown_actions() {
    assert!(priority_request("GET /api/status HTTP/1.1\r\n\r\n"));
    assert!(priority_request("POST /api/action HTTP/1.1\r\n\r\n{\"action\":\"pause\"}"));
    assert!(priority_request("POST /api/craft HTTP/1.1\r\n\r\n{\"action\":\"stop\"}"));
    assert!(!priority_request("POST /api/action HTTP/1.1\r\n\r\n{\"action\":\"buy_bait\"}"));
    assert!(!priority_request("POST /api/craft HTTP/1.1\r\n\r\n{\"action\":\"invalid\"}"));
    assert!(!priority_request("GET /api/stream HTTP/1.1\r\n\r\n"));
}

#[test]
fn status_reports_configured_keys() {
    let (bot, _, dir) = input_test_bot();
    bot.ctx().settings.write().keys.rod = '4';
    let response = drive(&bot, "status", "");
    let value: serde_json::Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(value["remote_keys"]["rod"], "4");
    let _ = std::fs::remove_dir_all(dir);
}


#[test]
fn explicit_stop_is_supported_and_routes_through_reserved_control_lane() {
    let request=format!("POST /api/action HTTP/1.1{}{}","\r\n\r\n",r#"{"action":"stop"}"#);
    assert!(priority_request(&request));
    let (bot,input,dir)=input_test_bot();
    let response=drive(&bot,"action",r#"{"action":"stop"}"#);
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(!bot.is_running());
    assert_eq!(bot.state(),crate::events::BotState::Stopped);
    // Mock input has no native device; Stop must not synthesize a new press.
    assert!(!input.events.lock().iter().any(|e|matches!(e,InputEvent::Button(_,true)|InputEvent::Key(_,true))));
    let _=std::fs::remove_dir_all(dir);
}


#[test]
fn stop_releases_input_with_all_stream_and_action_workers_occupied() {
    use crate::core::platform::{guarded_input::GuardedInput,mock::ScriptedCapture,Input};
    use crate::core::types::{Frame,MouseButton};
    let (mut bot,input,dir)=input_test_bot();
    {
        let ctx=Arc::get_mut(&mut Arc::get_mut(&mut bot).unwrap().ctx).unwrap();
        let capture=Arc::new(ScriptedCapture::default());
        capture.push(Frame::new(2,2,vec![0;16]));
        ctx.platform.capture=capture;
        ctx.platform.input=Arc::new(GuardedInput::new(ctx.platform.window.clone(),input.clone()));
    }
    let settings=bot.ctx().settings.clone();
    let token=settings.read().web.token.clone();
    let lanes=RequestLanes::new(&bot,&settings);
    thread::sleep(Duration::from_millis(50));
    fn submit(lanes:&RequestLanes,token:&str,path:&str,body:&str)->TcpStream {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();
        let client=TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server,_)=listener.accept().unwrap();
        let method=if body.is_empty() {"GET"} else {"POST"};
        let request=format!("{method} {path} HTTP/1.1\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}",body.len());
        lanes.dispatch(server,request);
        client
    }
    let mut occupied=Vec::new();
    for _ in 0..4 {
        occupied.push(submit(&lanes,&token,"/api/drag",r#"{"x1":0.1,"y1":0.1,"x2":0.9,"y2":0.9,"ms":5000}"#));
        occupied.push(submit(&lanes,&token,"/api/stream?fps=10&scale=240",""));
    }
    let start=std::time::Instant::now();
    while !input.events.lock().iter().any(|e|matches!(e,InputEvent::Button(_,true))) {
        assert!(start.elapsed()<Duration::from_secs(2),"synthetic drag must begin");
        thread::sleep(Duration::from_millis(5));
    }
    let at=std::time::Instant::now();
    let mut stop=submit(&lanes,&token,"/api/action",r#"{"action":"stop"}"#);
    stop.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut response=String::new();stop.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"),"{response}");
    assert!(at.elapsed()<Duration::from_secs(2),"reserved control lane must not wait for 5s drags");
    assert!(input.events.lock().contains(&InputEvent::Button(MouseButton::Left,false)));
    assert!(!bot.ctx().mouse_held.load(Ordering::SeqCst));
    bot.ctx().platform.input.release_all();
    drop(occupied);drop(lanes);
    let _=std::fs::remove_dir_all(dir);
}
