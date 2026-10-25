use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::bot::ctx::Ctx;
use crate::config::Store;
use crate::core::types::{Key, MouseButton, PxPoint, WindowInfo};

static IS_RECORDING: AtomicBool = AtomicBool::new(false);
static IS_PLAYING: AtomicBool = AtomicBool::new(false);

pub fn is_playing() -> bool {
    IS_PLAYING.load(Ordering::SeqCst)
}

static ROBLOX_REF: RwLock<Option<Arc<RwLock<Option<WindowInfo>>>>> = RwLock::new(None);
static STORE_REF: RwLock<Option<Arc<Store>>> = RwLock::new(None);

pub fn init_roblox_ref(roblox: Arc<RwLock<Option<WindowInfo>>>, store: Arc<Store>) {
    *ROBLOX_REF.write() = Some(roblox);
    *STORE_REF.write() = Some(store);
}

fn get_roblox_window_info() -> Option<WindowInfo> {
    if let Some(r) = ROBLOX_REF.read().as_ref() {
        return *r.read();
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordMode {
    WebScreen,
    PcWindow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MacroStep {
    Click {
        rx: f32,
        ry: f32,
        button: String,
        delay_ms: u64,
    },
    Drag {
        start_rx: f32,
        start_ry: f32,
        end_rx: f32,
        end_ry: f32,
        duration_ms: u64,
        delay_ms: u64,
    },
    KeyTap {
        key: String,
        delay_ms: u64,
    },
    KeyHold {
        key: String,
        duration_ms: u64,
        delay_ms: u64,
    },
    MouseMove {
        rx: f32,
        ry: f32,
        delay_ms: u64,
    },
    Sleep {
        ms: u64,
    },
    /// Explicit VPN connect with verification wait. Timeouts are real
    /// seconds (never scaled by playback speed). Fails the macro when
    /// `required` (default) and verification times out.
    VpnConnect {
        engine: String,
        timeout_s: u64,
        required: bool,
    },
    /// Explicit VPN disconnect with verification. Always fails the macro on
    /// timeout (a half-torn-down tunnel is never silently accepted).
    VpnDisconnect {
        timeout_s: u64,
    },
    VpnWaitConnected {
        timeout_s: u64,
        required: bool,
    },
    VpnWaitDisconnected {
        timeout_s: u64,
    },
}

/// Typed VPN macro action for `append_vpn_step` (no stringly-typed actions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VpnMacroAction {
    Connect,
    Disconnect,
    WaitConnected,
    WaitDisconnected,
}

static VPN_PROVIDER: RwLock<Option<Arc<dyn crate::vpn::VpnControl>>> = RwLock::new(None);

/// Wired once at startup (`app::build_state`). Macros without VPN steps never
/// touch the provider, so VPN stays DISCONNECTED unless explicitly requested.
pub fn set_vpn_provider(provider: Arc<dyn crate::vpn::VpnControl>) {
    *VPN_PROVIDER.write() = Some(provider);
}

fn clamp_vpn_timeout(timeout_s: u64) -> Duration {
    Duration::from_secs(timeout_s.clamp(5, 120))
}

/// Execute one VPN macro step against the provider. Pure dispatch (no
/// threads): fully unit-testable with a scripted fake provider.
pub fn execute_vpn_step(
    provider: Option<&Arc<dyn crate::vpn::VpnControl>>,
    step: &MacroStep,
) -> Result<(), String> {
    let vpn = provider.ok_or_else(|| "VPN step failed: VPN provider unavailable".to_string())?;
    match step {
        MacroStep::VpnConnect {
            engine,
            timeout_s,
            required,
        } => match vpn.vpn_connect_verified(engine, clamp_vpn_timeout(*timeout_s)) {
            Ok(_) => Ok(()),
            Err(e) => {
                let msg = format!(
                        "VPN connection failed. Expected: connected. Observed: {}. Engine: {engine}. Reason: {e}",
                        vpn.vpn_state().as_str()
                    );
                if *required {
                    Err(msg)
                } else {
                    Ok(())
                }
            }
        },
        MacroStep::VpnDisconnect { timeout_s } => vpn
            .vpn_disconnect_verified(clamp_vpn_timeout(*timeout_s))
            .map(|_| ())
            .map_err(|e| format!("VPN disconnect failed: {e}")),
        MacroStep::VpnWaitConnected {
            timeout_s,
            required,
        } => match vpn.vpn_wait_for(true, clamp_vpn_timeout(*timeout_s)) {
            Ok(_) => Ok(()),
            Err(e) => {
                if *required {
                    Err(format!(
                        "VPN wait failed. Expected: connected. Observed: {}. Reason: {e}",
                        vpn.vpn_state().as_str()
                    ))
                } else {
                    Ok(())
                }
            }
        },
        MacroStep::VpnWaitDisconnected { timeout_s } => vpn
            .vpn_wait_for(false, clamp_vpn_timeout(*timeout_s))
            .map(|_| ())
            .map_err(|e| format!("VPN wait failed. Expected: disconnected. Reason: {e}")),
        _ => Err("not a VPN macro step".to_string()),
    }
}

/// Append a typed VPN step to a saved macro. Macros never auto-connect:
/// without one of these steps the VPN provider is untouched by playback.
pub fn append_vpn_step(
    store: &Store,
    name_or_id: &str,
    action: VpnMacroAction,
    engine: Option<String>,
    timeout_s: Option<u64>,
    required: Option<bool>,
) -> Result<CustomMacro, String> {
    let mut list = load_macros(store);
    let timeout_s = timeout_s.unwrap_or(20).clamp(5, 120);
    let required = required.unwrap_or(true);
    let engine = engine
        .map(|e| e.trim().to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "auto".to_string());
    let out = {
        let m = list
            .iter_mut()
            .find(|m| m.id == name_or_id || m.name.eq_ignore_ascii_case(name_or_id))
            .ok_or_else(|| format!("Macro '{name_or_id}' not found"))?;
        m.steps.push(match action {
            VpnMacroAction::Connect => MacroStep::VpnConnect {
                engine,
                timeout_s,
                required,
            },
            VpnMacroAction::Disconnect => MacroStep::VpnDisconnect { timeout_s },
            VpnMacroAction::WaitConnected => MacroStep::VpnWaitConnected {
                timeout_s,
                required,
            },
            VpnMacroAction::WaitDisconnected => MacroStep::VpnWaitDisconnected { timeout_s },
        });
        m.clone()
    };
    save_macros(store, &list)?;
    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomMacro {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub steps: Vec<MacroStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecorderStatus {
    pub is_recording: bool,
    pub record_mode: Option<RecordMode>,
    pub recorded_steps_count: usize,
    pub is_playing: bool,
    pub playing_macro_name: Option<String>,
    pub current_loop: u32,
    pub is_looping: bool,
    pub message: String,
}

struct Recording {
    mode: RecordMode,
    start: Instant,
    events: Vec<gpo_control_runtime::TimedInput>,
    steps: Vec<MacroStep>,
    last: u64,
}
static RECORDING: parking_lot::Mutex<Option<Recording>> = parking_lot::Mutex::new(None);
static CANCEL: parking_lot::Mutex<Option<Arc<AtomicBool>>> = parking_lot::Mutex::new(None);
static MESSAGE: RwLock<String> = RwLock::new(String::new());
static CURRENT: RwLock<Option<String>> = RwLock::new(None);
static LOOP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static LOOPING: AtomicBool = AtomicBool::new(false);
use gpo_control_runtime::{validate_recording, InputAction, TimedInput};

fn macros_file_path(store: &Store) -> PathBuf {
    store.dir().join("custom_macros.json")
}
fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("writing");
    let mut file = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(tmp, path).map_err(|e| e.to_string())
}
pub fn load_macros(store: &Store) -> Vec<CustomMacro> {
    let p = macros_file_path(store);
    if fs::metadata(&p).is_ok_and(|m| m.len() > 16 * 1024 * 1024) {
        return vec![];
    }
    fs::read(p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
pub fn save_macros(store: &Store, list: &[CustomMacro]) -> Result<(), String> {
    let path = macros_file_path(store);
    if path.exists() {
        fs::copy(&path, path.with_extension("backup.json")).map_err(|e| e.to_string())?;
    }
    atomic_write(
        &path,
        &serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?,
    )
}
pub fn get_status() -> RecorderStatus {
    let recording = RECORDING.lock();
    RecorderStatus {
        is_recording: IS_RECORDING.load(Ordering::SeqCst),
        record_mode: recording.as_ref().map(|r| r.mode),
        recorded_steps_count: recording.as_ref().map_or(0, |r| r.events.len()),
        is_playing: is_playing(),
        playing_macro_name: CURRENT.read().clone(),
        current_loop: LOOP.load(Ordering::SeqCst),
        is_looping: LOOPING.load(Ordering::SeqCst),
        message: MESSAGE.read().clone(),
    }
}
pub fn is_recording() -> bool {
    IS_RECORDING.load(Ordering::SeqCst)
}
pub fn toggle_recording() -> Result<bool, String> {
    if is_recording() {
        let store = STORE_REF
            .read()
            .clone()
            .ok_or("Recording storage is not ready")?;
        stop_recording("", &store)?;
        Ok(false)
    } else {
        start_recording(RecordMode::PcWindow)?;
        Ok(true)
    }
}
pub fn start_recording(mode: RecordMode) -> Result<(), String> {
    let _mode = super::control::MODE_LOCK.lock();
    if is_playing() {
        return Err("Stop playback before recording".into());
    }
    if RECORDING.lock().is_some() {
        return Err("Save or discard the existing recording first".into());
    }
    *RECORDING.lock() = Some(Recording {
        mode,
        start: Instant::now(),
        events: vec![],
        steps: vec![],
        last: 0,
    });
    IS_RECORDING.store(true, Ordering::SeqCst);
    *MESSAGE.write() = "Recording game inputs; focus Roblox to begin".into();
    if mode == RecordMode::PcWindow {
        if let Err(error) = start_pc_capture() {
            cancel_recording();
            return Err(error);
        }
    }
    Ok(())
}
pub fn record_input(input: InputAction) {
    if !is_recording() || input.validate().is_err() {
        return;
    }
    let mut active = RECORDING.lock();
    let Some(r) = active.as_mut() else {
        return;
    };
    if r.events.len() >= 100_000 {
        IS_RECORDING.store(false, Ordering::SeqCst);
        *MESSAGE.write() = "Recording limit reached; save the captured inputs".into();
        return;
    }
    let at_ms = r.start.elapsed().as_millis() as u64;
    let delay_ms = at_ms.saturating_sub(r.last);
    match &input {
        InputAction::KeyDown { key } => r.steps.push(MacroStep::KeyTap {
            key: key.clone(),
            delay_ms,
        }),
        InputAction::Move { x, y } => r.steps.push(MacroStep::MouseMove {
            rx: *x,
            ry: *y,
            delay_ms,
        }),
        InputAction::MouseDown { button } => {
            let point = r
                .events
                .iter()
                .rev()
                .find_map(|e| {
                    if let InputAction::Move { x, y } = e.input {
                        Some((x, y))
                    } else {
                        None
                    }
                })
                .unwrap_or((0.5, 0.5));
            r.steps.push(MacroStep::Click {
                rx: point.0,
                ry: point.1,
                button: button.clone(),
                delay_ms,
            });
        }
        _ => {}
    }
    r.last = at_ms;
    r.events.push(TimedInput { at_ms, input });
}
pub fn record_click(rx: f32, ry: f32, button: &str) {
    record_input(InputAction::Move { x: rx, y: ry });
    record_input(InputAction::MouseDown {
        button: button.into(),
    });
    record_input(InputAction::MouseUp {
        button: button.into(),
    });
}
pub fn record_drag(sx: f32, sy: f32, ex: f32, ey: f32, duration_ms: u64) {
    if !is_recording() {
        return;
    }
    let mut rec = RECORDING.lock();
    let Some(r) = rec.as_mut() else {
        return;
    };
    let end = r.start.elapsed().as_millis() as u64;
    let start = end.saturating_sub(duration_ms);
    r.events.extend([
        TimedInput {
            at_ms: start,
            input: InputAction::Move { x: sx, y: sy },
        },
        TimedInput {
            at_ms: start,
            input: InputAction::MouseDown {
                button: "left".into(),
            },
        },
        TimedInput {
            at_ms: end,
            input: InputAction::Move { x: ex, y: ey },
        },
        TimedInput {
            at_ms: end,
            input: InputAction::MouseUp {
                button: "left".into(),
            },
        },
    ]);
    r.steps.push(MacroStep::Drag {
        start_rx: sx,
        start_ry: sy,
        end_rx: ex,
        end_ry: ey,
        duration_ms,
        delay_ms: start.saturating_sub(r.last),
    });
    r.last = end;
}
pub fn record_key(key: &str) {
    record_key_hold(key, 50);
}
pub fn record_key_hold(key: &str, duration_ms: u64) {
    if !is_recording() {
        return;
    }
    let mut rec = RECORDING.lock();
    let Some(r) = rec.as_mut() else {
        return;
    };
    let end = r.start.elapsed().as_millis() as u64;
    let start = end.saturating_sub(duration_ms);
    r.events.push(TimedInput {
        at_ms: start,
        input: InputAction::KeyDown { key: key.into() },
    });
    r.events.push(TimedInput {
        at_ms: end,
        input: InputAction::KeyUp { key: key.into() },
    });
    r.steps.push(MacroStep::KeyHold {
        key: key.into(),
        duration_ms,
        delay_ms: start.saturating_sub(r.last),
    });
    r.last = end;
}
pub fn stop_recording(name: &str, store: &Store) -> Result<CustomMacro, String> {
    IS_RECORDING.store(false, Ordering::SeqCst);
    let mut active = RECORDING.lock();
    let r = active.as_mut().ok_or("No recording to save")?;
    r.events.sort_by_key(|event| event.at_ms);
    validate_recording(&r.events)?;
    let mut list = load_macros(store);
    let id = format!("r2_{}", crate::core::secrets::generate_token_hex(12));
    let title = if name.trim().is_empty() {
        format!("Macro #{}", list.len() + 1)
    } else {
        name.trim().chars().take(120).collect()
    };
    let saved = CustomMacro {
        id: id.clone(),
        name: title,
        created_at: crate::core::boss_tracker::now_sec().to_string(),
        steps: r.steps.clone(),
    };
    atomic_write(
        &store.dir().join("recordings-v2").join(format!("{id}.json")),
        &serde_json::to_vec(&r.events).map_err(|e| e.to_string())?,
    )?;
    list.push(saved.clone());
    save_macros(store, &list)?;
    *MESSAGE.write() = format!("Saved {} input events", r.events.len());
    *active = None;
    Ok(saved)
}
pub fn cancel_recording() {
    IS_RECORDING.store(false, Ordering::SeqCst);
    *RECORDING.lock() = None;
    *MESSAGE.write() = "Recording discarded".into();
}
pub fn delete_macro(store: &Store, id: &str) -> Result<(), String> {
    let mut list = load_macros(store);
    let before = list.len();
    list.retain(|m| m.id != id && !m.name.eq_ignore_ascii_case(id));
    if before == list.len() {
        return Err("Recording was not found".into());
    }
    save_macros(store, &list)
}
pub fn rename_macro(store: &Store, id: &str, name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.chars().count() > 120 {
        return Err("Name must contain 1 to 120 characters".into());
    }
    let mut list = load_macros(store);
    let item = list
        .iter_mut()
        .find(|m| m.id == id || m.name.eq_ignore_ascii_case(id))
        .ok_or("Recording was not found")?;
    item.name = name.trim().into();
    save_macros(store, &list)
}
fn legacy_events(steps: &[MacroStep]) -> Result<Vec<TimedInput>, String> {
    let mut events = vec![];
    let mut at = 0u64;
    for step in steps {
        match step {
            MacroStep::Sleep{ms}=>at=at.saturating_add(*ms),
            MacroStep::Click{rx,ry,button,delay_ms}=>{
                at=at.saturating_add(*delay_ms);
                events.push(TimedInput{at_ms:at,input:InputAction::Move{x:*rx,y:*ry}});
                events.push(TimedInput{at_ms:at,input:InputAction::MouseDown{button:button.clone()}});
                at=at.saturating_add(50);events.push(TimedInput{at_ms:at,input:InputAction::MouseUp{button:button.clone()}});
            },
            MacroStep::MouseMove{rx,ry,delay_ms}=>{at=at.saturating_add(*delay_ms);events.push(TimedInput{at_ms:at,input:InputAction::Move{x:*rx,y:*ry}});},
            MacroStep::KeyTap{key,delay_ms}|MacroStep::KeyHold{key,delay_ms,..}=>{
                at=at.saturating_add(*delay_ms);let key=if key.eq_ignore_ascii_case("jump"){"space".into()}else{key.to_ascii_lowercase()};
                events.push(TimedInput{at_ms:at,input:InputAction::KeyDown{key:key.clone()}});
                at=at.saturating_add(if let MacroStep::KeyHold{duration_ms,..}=step{*duration_ms}else{50});
                events.push(TimedInput{at_ms:at,input:InputAction::KeyUp{key}});
            },
            MacroStep::Drag{start_rx,start_ry,end_rx,end_ry,duration_ms,delay_ms}=>{
                at=at.saturating_add(*delay_ms);
                events.push(TimedInput{at_ms:at,input:InputAction::Move{x:*start_rx,y:*start_ry}});
                events.push(TimedInput{at_ms:at,input:InputAction::MouseDown{button:"left".into()}});
                for index in 1..=20{let f=index as f32/20.0;events.push(TimedInput{at_ms:at+duration_ms*index/20,input:InputAction::Move{x:start_rx+(end_rx-start_rx)*f,y:start_ry+(end_ry-start_ry)*f}});}
                at=at.saturating_add(*duration_ms);events.push(TimedInput{at_ms:at,input:InputAction::MouseUp{button:"left".into()}});
            },
            _=>return Err("This legacy recording contains VPN steps. Use the VPN controls separately while the new recorder is in game-input mode.".into()),
        }
    }
    validate_recording(&events)?;
    Ok(events)
}
pub fn stop_playback() {
    if let Some(cancel) = CANCEL.lock().as_ref() {
        cancel.store(true, Ordering::SeqCst);
    }
    if let Ok(service) = super::control::service() {
        service.runtime.stop();
    }
    if is_playing() {
        *MESSAGE.write() = "Stopping playback; input session cancelled".into();
    }
}
pub fn play_macro(
    ctx: Arc<Ctx>,
    store: Arc<Store>,
    name: &str,
    loop_mode: bool,
    speed: Option<f32>,
    max_loops: Option<u32>,
) -> Result<(), String> {
    let target = load_macros(&store)
        .into_iter()
        .find(|m| m.id == name || m.name.eq_ignore_ascii_case(name))
        .ok_or("Recording was not found")?;
    let speed = speed.unwrap_or(1.0);
    if !speed.is_finite() || !(0.1..=100.0).contains(&speed) {
        return Err("Speed must be between 0.1 and 100".into());
    }
    let timeline = if target.id.starts_with("r2_")
        && target
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        let path = store
            .dir()
            .join("recordings-v2")
            .join(format!("{}.json", target.id));
        if path.exists() {
            if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
                return Err("Recording exceeds the size limit".into());
            }
            serde_json::from_slice::<Vec<TimedInput>>(&fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Recording data is invalid: {e}"))?
        } else {
            legacy_events(&target.steps)?
        }
    } else {
        legacy_events(&target.steps)?
    };
    validate_recording(&timeline)?;
    let service = super::control::service()?;
    let _mode = super::control::MODE_LOCK.lock();
    if ctx.alive() {
        return Err("Pause fishing before playing a recorded macro".into());
    }
    if is_recording() {
        return Err("Save or discard the active recording first".into());
    }
    if IS_PLAYING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("A recording is already playing".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *CANCEL.lock() = Some(cancel.clone());
    *CURRENT.write() = Some(target.name.clone());
    LOOPING.store(loop_mode, Ordering::SeqCst);
    *MESSAGE.write() = "Focusing Roblox…".into();
    let loops = if loop_mode { max_loops.unwrap_or(0) } else { 1 };
    std::thread::spawn(move || {
        let result = (|| {
            if cancel.load(Ordering::SeqCst) {
                return Err("Playback cancelled".into());
            }
            service.focus()?;
            if cancel.load(Ordering::SeqCst) {
                return Err("Playback cancelled".into());
            }
            let epoch = service.runtime.begin();
            if cancel.load(Ordering::SeqCst) {
                service.runtime.finish(epoch);
                return Err("Playback cancelled".into());
            }
            service
                .runtime
                .play(epoch, &timeline, speed, loops, |count| {
                    LOOP.store(count, Ordering::SeqCst);
                    *MESSAGE.write() = format!("Playing {} — loop {}", target.name, count);
                })
        })();
        *MESSAGE.write() = match result {
            Ok(()) => "Playback completed".into(),
            Err(error) => error,
        };
        *CURRENT.write() = None;
        LOOPING.store(false, Ordering::SeqCst);
        *CANCEL.lock() = None;
        IS_PLAYING.store(false, Ordering::SeqCst);
    });
    Ok(())
}
#[cfg(not(windows))]
fn start_pc_capture() -> Result<(), String> {
    IS_RECORDING.store(false, Ordering::SeqCst);
    *RECORDING.lock() = None;
    Err("PC recording is available on Windows; web recording remains available".into())
}
#[cfg(windows)]
fn start_pc_capture() -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    let service = super::control::service()?;
    std::thread::spawn(move || {
        let mut keys: Vec<(i32, String, bool)> = (b'A'..=b'Z')
            .map(|c| {
                (
                    i32::from(c),
                    (c as char).to_ascii_lowercase().to_string(),
                    false,
                )
            })
            .chain((b'0'..=b'9').map(|c| (i32::from(c), (c as char).to_string(), false)))
            .chain(
                [
                    (32, "space"),
                    (16, "shift"),
                    (17, "ctrl"),
                    (13, "enter"),
                    (27, "escape"),
                    (8, "backspace"),
                    (46, "delete"),
                    (37, "left"),
                    (38, "up"),
                    (39, "right"),
                    (40, "down"),
                ]
                .into_iter()
                .map(|(c, k)| (c, k.into(), false)),
            )
            .collect();
        let mut buttons = [(1, "left", false), (2, "right", false)];
        let mut focused = false;
        let mut last = None;
        while is_recording() {
            let window = service
                .platform
                .window
                .find()
                .filter(|w| w.visible && w.is_foreground);
            if let Some(window) = window {
                let cursor = service.platform.input.cursor();
                let inside = cursor.x >= window.client.x
                    && cursor.x < window.client.right()
                    && cursor.y >= window.client.y
                    && cursor.y < window.client.bottom();
                if !focused {
                    for (vk, _, pressed) in &mut keys {
                        *pressed = (unsafe { GetAsyncKeyState(*vk) } as u16 & 0x8000) != 0;
                    }
                    for (vk, _, pressed) in &mut buttons {
                        *pressed = (unsafe { GetAsyncKeyState(*vk) } as u16 & 0x8000) != 0;
                    }
                    focused = true;
                    last = None;
                } else {
                    if inside {
                        let point = cursor.to_rel(&window.client);
                        if last != Some(cursor) {
                            record_input(InputAction::Move {
                                x: point.x,
                                y: point.y,
                            });
                            last = Some(cursor);
                        }
                    }
                    for (vk, key, pressed) in &mut keys {
                        let down = (unsafe { GetAsyncKeyState(*vk) } as u16 & 0x8000) != 0;
                        if down != *pressed {
                            record_input(if down {
                                InputAction::KeyDown { key: key.clone() }
                            } else {
                                InputAction::KeyUp { key: key.clone() }
                            });
                            *pressed = down;
                        }
                    }
                    for (vk, button, pressed) in &mut buttons {
                        let down = (unsafe { GetAsyncKeyState(*vk) } as u16 & 0x8000) != 0;
                        if down != *pressed && (!down || inside) {
                            record_input(if down {
                                InputAction::MouseDown {
                                    button: (*button).into(),
                                }
                            } else {
                                InputAction::MouseUp {
                                    button: (*button).into(),
                                }
                            });
                        }
                        *pressed = down;
                    }
                }
            } else {
                if focused {
                    for (_, key, pressed) in &mut keys {
                        if *pressed {
                            record_input(InputAction::KeyUp { key: key.clone() });
                            *pressed = false;
                        }
                    }
                    for (_, button, pressed) in &mut buttons {
                        if *pressed {
                            record_input(InputAction::MouseUp {
                                button: (*button).into(),
                            });
                            *pressed = false;
                        }
                    }
                }
                focused = false;
            }
            std::thread::sleep(Duration::from_millis(8));
        }
    });
    Ok(())
}
