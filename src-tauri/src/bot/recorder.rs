use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::bot::ctx::Ctx;
use crate::config::Store;
use crate::core::types::{Key, MouseButton, PxPoint, WindowInfo};

static IS_RECORDING: AtomicBool = AtomicBool::new(false);
static IS_PLAYING: AtomicBool = AtomicBool::new(false);
static STOP_PLAYBACK_REQUESTED: AtomicBool = AtomicBool::new(false);

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
        MacroStep::VpnConnect { engine, timeout_s, required } => {
            match vpn.vpn_connect_verified(engine, clamp_vpn_timeout(*timeout_s)) {
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
            }
        }
        MacroStep::VpnDisconnect { timeout_s } => vpn
            .vpn_disconnect_verified(clamp_vpn_timeout(*timeout_s))
            .map(|_| ())
            .map_err(|e| format!("VPN disconnect failed: {e}")),
        MacroStep::VpnWaitConnected { timeout_s, required } => {
            match vpn.vpn_wait_for(true, clamp_vpn_timeout(*timeout_s)) {
                Ok(_) => Ok(()),
                Err(e) => {
                    if *required {
                        Err(format!("VPN wait failed. Expected: connected. Observed: {}. Reason: {e}", vpn.vpn_state().as_str()))
                    } else {
                        Ok(())
                    }
                }
            }
        }
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
            VpnMacroAction::Connect => MacroStep::VpnConnect { engine, timeout_s, required },
            VpnMacroAction::Disconnect => MacroStep::VpnDisconnect { timeout_s },
            VpnMacroAction::WaitConnected => MacroStep::VpnWaitConnected { timeout_s, required },
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

struct ActiveRecording {
    _mode: RecordMode,
    _start_time: Instant,
    last_action_time: Instant,
    steps: Vec<MacroStep>,
}

static ACTIVE_RECORDING: RwLock<Option<ActiveRecording>> = RwLock::new(None);
static STATUS: RwLock<RecorderStatus> = RwLock::new(RecorderStatus {
    is_recording: false,
    record_mode: None,
    recorded_steps_count: 0,
    is_playing: false,
    playing_macro_name: None,
    current_loop: 0,
    is_looping: false,
    message: String::new(),
});

fn macros_file_path(store: &Store) -> PathBuf {
    store.dir().join("custom_macros.json")
}

pub fn load_macros(store: &Store) -> Vec<CustomMacro> {
    let p = macros_file_path(store);
    if !p.exists() {
        return vec![];
    }
    fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_macros(store: &Store, list: &[CustomMacro]) -> Result<(), String> {
    let p = macros_file_path(store);
    let s = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    fs::write(&p, s).map_err(|e| e.to_string())
}

pub fn get_status() -> RecorderStatus {
    STATUS.read().clone()
}

pub fn is_recording() -> bool {
    IS_RECORDING.load(Ordering::SeqCst)
}

pub fn toggle_recording() -> Result<bool, String> {
    static LAST_TOGGLE: parking_lot::Mutex<Option<Instant>> = parking_lot::Mutex::new(None);
    {
        let mut last = LAST_TOGGLE.lock();
        if let Some(t) = *last {
            if t.elapsed() < Duration::from_millis(600) {
                return Ok(IS_RECORDING.load(Ordering::SeqCst));
            }
        }
        *last = Some(Instant::now());
    }

    if IS_RECORDING.load(Ordering::SeqCst) {
        if let Some(store) = STORE_REF.read().as_ref() {
            let m = stop_recording("", store)?;
            tracing::info!("📼 Macro recording stopped via shortcut: '{}' ({} steps)", m.name, m.steps.len());
            Ok(false)
        } else {
            Err("No store reference available".into())
        }
    } else {
        start_recording(RecordMode::PcWindow)?;
        tracing::info!("📼 Macro recording started via shortcut");
        Ok(true)
    }
}

pub fn start_recording(mode: RecordMode) -> Result<(), String> {
    if IS_RECORDING.swap(true, Ordering::SeqCst) {
        return Err("Already recording a macro".into());
    }
    let now = Instant::now();
    *ACTIVE_RECORDING.write() = Some(ActiveRecording {
        _mode: mode,
        _start_time: now,
        last_action_time: now,
        steps: Vec::new(),
    });

    let mut st = STATUS.write();
    st.is_recording = true;
    st.record_mode = Some(mode);
    st.recorded_steps_count = 0;
    st.message = match mode {
        RecordMode::PcWindow => "Recording PC window... Click & move in Roblox. Press F7 or F8 to save!".into(),
        RecordMode::WebScreen => "Recording via live screen... Tap buttons and controls.".into(),
    };

    if mode == RecordMode::PcWindow {
        spawn_pc_recorder_thread();
    }

    Ok(())
}

fn spawn_pc_recorder_thread() {
    std::thread::Builder::new()
        .name("macro-pc-recorder".into())
        .spawn(move || {
            // Short grace period so clicking "Record" on UI isn't captured
            std::thread::sleep(Duration::from_millis(400));

            #[cfg(windows)]
            {
                use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
                use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
                use windows::Win32::Foundation::POINT;

                let mut prev_f8_down = false;
                let mut prev_f7_down = false;

                struct KeyState {
                    vk: i32,
                    name: &'static str,
                    down: bool,
                    pressed_at: Instant,
                }

                let mut monitored = vec![
                    KeyState { vk: 0x57, name: "w", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x41, name: "a", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x53, name: "s", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x44, name: "d", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x45, name: "e", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x54, name: "t", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x20, name: "space", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x10, name: "shift", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x26, name: "up", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x28, name: "down", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x25, name: "left", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x27, name: "right", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x31, name: "1", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x32, name: "2", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x33, name: "3", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x34, name: "4", down: false, pressed_at: Instant::now() },
                    KeyState { vk: 0x35, name: "5", down: false, pressed_at: Instant::now() },
                ];

                struct MouseBtnTracker {
                    down: bool,
                    start_time: Instant,
                    start_pt: POINT,
                    start_rx: f32,
                    start_ry: f32,
                    max_dist_px: f32,
                }

                let mut l_tracker = MouseBtnTracker {
                    down: false,
                    start_time: Instant::now(),
                    start_pt: POINT::default(),
                    start_rx: 0.0,
                    start_ry: 0.0,
                    max_dist_px: 0.0,
                };

                let mut r_tracker = MouseBtnTracker {
                    down: false,
                    start_time: Instant::now(),
                    start_pt: POINT::default(),
                    start_rx: 0.0,
                    start_ry: 0.0,
                    max_dist_px: 0.0,
                };

                while IS_RECORDING.load(Ordering::SeqCst) {
                    // Hotkey F8 (0x77) or F7 (0x76) finishes recording from within Roblox
                    let f8_down = unsafe { (GetAsyncKeyState(0x77) as u16 & 0x8000) != 0 };
                    let f7_down = unsafe { (GetAsyncKeyState(0x76) as u16 & 0x8000) != 0 };
                    if (f8_down && !prev_f8_down) || (f7_down && !prev_f7_down) {
                        if let Some(store) = STORE_REF.read().as_ref() {
                            let _ = stop_recording("", store);
                            break;
                        }
                    }
                    prev_f8_down = f8_down;
                    prev_f7_down = f7_down;

                    let info_opt = get_roblox_window_info();
                    if let Some(info) = info_opt {
                        if info.is_foreground && info.visible {
                            let mut pt = POINT::default();
                            let _ = unsafe { GetCursorPos(&mut pt) };

                            let inside_roblox = pt.x >= info.client.x
                                && pt.x < info.client.x + info.client.w
                                && pt.y >= info.client.y
                                && pt.y < info.client.y + info.client.h;

                            let l_down = unsafe { (GetAsyncKeyState(0x01) as u16 & 0x8000) != 0 };
                            let r_down = unsafe { (GetAsyncKeyState(0x02) as u16 & 0x8000) != 0 };

                            if inside_roblox {
                                let rx = ((pt.x - info.client.x) as f32 / info.client.w.max(1) as f32).clamp(0.0, 1.0);
                                let ry = ((pt.y - info.client.y) as f32 / info.client.h.max(1) as f32).clamp(0.0, 1.0);

                                // Left Button (Clicks and Drag & Drop)
                                if l_down && !l_tracker.down {
                                    l_tracker.down = true;
                                    l_tracker.start_time = Instant::now();
                                    l_tracker.start_pt = pt;
                                    l_tracker.start_rx = rx;
                                    l_tracker.start_ry = ry;
                                    l_tracker.max_dist_px = 0.0;
                                } else if l_down && l_tracker.down {
                                    let dx = (pt.x - l_tracker.start_pt.x) as f32;
                                    let dy = (pt.y - l_tracker.start_pt.y) as f32;
                                    let dist = (dx * dx + dy * dy).sqrt();
                                    l_tracker.max_dist_px = l_tracker.max_dist_px.max(dist);
                                } else if !l_down && l_tracker.down {
                                    l_tracker.down = false;
                                    let dur = l_tracker.start_time.elapsed().as_millis() as u64;
                                    let dx = (pt.x - l_tracker.start_pt.x) as f32;
                                    let dy = (pt.y - l_tracker.start_pt.y) as f32;
                                    let final_dist = (dx * dx + dy * dy).sqrt();
                                    if l_tracker.max_dist_px >= 16.0 || final_dist >= 16.0 {
                                        record_drag(l_tracker.start_rx, l_tracker.start_ry, rx, ry, dur);
                                    } else {
                                        record_click(l_tracker.start_rx, l_tracker.start_ry, "left");
                                    }
                                }

                                // Right Button
                                if r_down && !r_tracker.down {
                                    r_tracker.down = true;
                                    r_tracker.start_time = Instant::now();
                                    r_tracker.start_pt = pt;
                                    r_tracker.start_rx = rx;
                                    r_tracker.start_ry = ry;
                                    r_tracker.max_dist_px = 0.0;
                                } else if r_down && r_tracker.down {
                                    let dx = (pt.x - r_tracker.start_pt.x) as f32;
                                    let dy = (pt.y - r_tracker.start_pt.y) as f32;
                                    let dist = (dx * dx + dy * dy).sqrt();
                                    r_tracker.max_dist_px = r_tracker.max_dist_px.max(dist);
                                } else if !r_down && r_tracker.down {
                                    r_tracker.down = false;
                                    let dur = r_tracker.start_time.elapsed().as_millis() as u64;
                                    let dx = (pt.x - r_tracker.start_pt.x) as f32;
                                    let dy = (pt.y - r_tracker.start_pt.y) as f32;
                                    let final_dist = (dx * dx + dy * dy).sqrt();
                                    if r_tracker.max_dist_px >= 16.0 || final_dist >= 16.0 {
                                        record_drag(r_tracker.start_rx, r_tracker.start_ry, rx, ry, dur);
                                    } else {
                                        record_click(r_tracker.start_rx, r_tracker.start_ry, "right");
                                    }
                                }
                            } else {
                                if !l_down && l_tracker.down {
                                    l_tracker.down = false;
                                }
                                if !r_down && r_tracker.down {
                                    r_tracker.down = false;
                                }
                            }

                            for k in &mut monitored {
                                let is_down = unsafe { (GetAsyncKeyState(k.vk) as u16 & 0x8000) != 0 };
                                if is_down && !k.down {
                                    k.down = true;
                                    k.pressed_at = Instant::now();
                                } else if !is_down && k.down {
                                    k.down = false;
                                    let dur = k.pressed_at.elapsed().as_millis() as u64;
                                    if dur >= 150 {
                                        record_key_hold(k.name, dur);
                                    } else {
                                        record_key(k.name);
                                    }
                                }
                            }
                        }
                    }

                    std::thread::sleep(Duration::from_millis(15));
                }
            }
        })
        .expect("spawn pc recorder thread");
}

pub fn record_drag(start_rx: f32, start_ry: f32, end_rx: f32, end_ry: f32, duration_ms: u64) {
    if !IS_RECORDING.load(Ordering::SeqCst) {
        return;
    }
    let mut rec_lock = ACTIVE_RECORDING.write();
    if let Some(rec) = rec_lock.as_mut() {
        let now = Instant::now();
        let delay_ms = now.duration_since(rec.last_action_time).as_millis().clamp(20, 10000) as u64;
        rec.last_action_time = now;
        rec.steps.push(MacroStep::Drag {
            start_rx,
            start_ry,
            end_rx,
            end_ry,
            duration_ms: duration_ms.max(50),
            delay_ms,
        });
        let mut st = STATUS.write();
        st.recorded_steps_count = rec.steps.len();
        st.message = format!("Recorded Drag & Drop ({}ms) (#{})", duration_ms, rec.steps.len());
    }
}

pub fn record_click(rx: f32, ry: f32, button: &str) {
    if !IS_RECORDING.load(Ordering::SeqCst) {
        return;
    }
    let mut rec_lock = ACTIVE_RECORDING.write();
    if let Some(rec) = rec_lock.as_mut() {
        let now = Instant::now();
        let delay_ms = now.duration_since(rec.last_action_time).as_millis().clamp(20, 10000) as u64;
        rec.last_action_time = now;
        rec.steps.push(MacroStep::Click {
            rx,
            ry,
            button: button.to_string(),
            delay_ms,
        });
        let mut st = STATUS.write();
        st.recorded_steps_count = rec.steps.len();
        st.message = format!("Recorded click #{}", rec.steps.len());
    }
}

pub fn record_key(key: &str) {
    if !IS_RECORDING.load(Ordering::SeqCst) {
        return;
    }
    let mut rec_lock = ACTIVE_RECORDING.write();
    if let Some(rec) = rec_lock.as_mut() {
        let now = Instant::now();
        let delay_ms = now.duration_since(rec.last_action_time).as_millis().clamp(20, 10000) as u64;
        rec.last_action_time = now;
        rec.steps.push(MacroStep::KeyTap {
            key: key.to_string(),
            delay_ms,
        });
        let mut st = STATUS.write();
        st.recorded_steps_count = rec.steps.len();
        st.message = format!("Recorded key [{key}] (#{})", rec.steps.len());
    }
}

pub fn record_key_hold(key: &str, duration_ms: u64) {
    if !IS_RECORDING.load(Ordering::SeqCst) {
        return;
    }
    let mut rec_lock = ACTIVE_RECORDING.write();
    if let Some(rec) = rec_lock.as_mut() {
        let now = Instant::now();
        let delay_ms = now.duration_since(rec.last_action_time).as_millis().clamp(20, 10000) as u64;
        rec.last_action_time = now;
        rec.steps.push(MacroStep::KeyHold {
            key: key.to_string(),
            duration_ms,
            delay_ms,
        });
        let mut st = STATUS.write();
        st.recorded_steps_count = rec.steps.len();
        st.message = format!("Recorded move/hold [{key}] ({}ms) (#{})", duration_ms, rec.steps.len());
    }
}

pub fn stop_recording(name: &str, store: &Store) -> Result<CustomMacro, String> {
    if !IS_RECORDING.swap(false, Ordering::SeqCst) {
        return Err("Not currently recording".into());
    }

    let rec_opt = ACTIVE_RECORDING.write().take();
    let rec = rec_opt.ok_or("No active recording found")?;

    if rec.steps.is_empty() {
        let mut st = STATUS.write();
        st.is_recording = false;
        st.record_mode = None;
        st.recorded_steps_count = 0;
        st.message = "Recording discarded (0 steps captured)".into();
        return Err("Recording has 0 steps".into());
    }

    let mut list = load_macros(store);
    let id = format!("macro_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
    let custom_name = if name.trim().is_empty() {
        format!("Macro #{}", list.len() + 1)
    } else {
        name.trim().to_string()
    };

    let new_macro = CustomMacro {
        id,
        name: custom_name,
        created_at: crate::core::boss_tracker::now_sec().to_string(),
        steps: rec.steps,
    };

    list.push(new_macro.clone());
    save_macros(store, &list)?;

    let mut st = STATUS.write();
    st.is_recording = false;
    st.record_mode = None;
    st.recorded_steps_count = 0;
    st.message = format!("Saved macro '{}' ({} steps)", new_macro.name, new_macro.steps.len());

    Ok(new_macro)
}

pub fn cancel_recording() {
    IS_RECORDING.store(false, Ordering::SeqCst);
    *ACTIVE_RECORDING.write() = None;
    let mut st = STATUS.write();
    st.is_recording = false;
    st.record_mode = None;
    st.recorded_steps_count = 0;
    st.message = "Recording cancelled".into();
}

pub fn delete_macro(store: &Store, id_or_name: &str) -> Result<(), String> {
    let mut list = load_macros(store);
    let init_len = list.len();
    list.retain(|m| m.id != id_or_name && m.name != id_or_name);
    if list.len() == init_len {
        return Err(format!("Macro '{id_or_name}' not found"));
    }
    save_macros(store, &list)
}

pub fn rename_macro(store: &Store, id_or_name: &str, new_name: &str) -> Result<(), String> {
    let trimmed = new_name.trim();
    if trimmed.is_empty() {
        return Err("New macro name cannot be empty".into());
    }
    let mut list = load_macros(store);
    let mut found = false;
    for m in &mut list {
        if m.id == id_or_name || m.name.eq_ignore_ascii_case(id_or_name) {
            m.name = trimmed.to_string();
            found = true;
            break;
        }
    }
    if !found {
        return Err(format!("Macro '{id_or_name}' not found"));
    }
    save_macros(store, &list)
}

pub fn stop_playback() {
    if IS_PLAYING.load(Ordering::SeqCst) {
        STOP_PLAYBACK_REQUESTED.store(true, Ordering::SeqCst);
        let mut st = STATUS.write();
        st.message = "Stopping macro playback...".into();
    }
}

pub fn play_macro(
    ctx: Arc<Ctx>,
    store: Arc<Store>,
    name_or_id: &str,
    loop_mode: bool,
    speed: Option<f32>,
    max_loops: Option<u32>,
) -> Result<(), String> {
    if IS_PLAYING.swap(true, Ordering::SeqCst) {
        return Err("A macro is already playing".into());
    }

    let list = load_macros(&store);
    let target = list
        .into_iter()
        .find(|m| m.id == name_or_id || m.name.eq_ignore_ascii_case(name_or_id))
        .ok_or_else(|| format!("Macro '{name_or_id}' not found"))?;

    let sp = speed.unwrap_or(1.0).clamp(0.1, 100.0);
    let target_loops = if loop_mode {
        max_loops.unwrap_or(0)
    } else {
        1
    };
    STOP_PLAYBACK_REQUESTED.store(false, Ordering::SeqCst);

    {
        let mut st = STATUS.write();
        st.is_playing = true;
        st.playing_macro_name = Some(target.name.clone());
        st.current_loop = 1;
        st.is_looping = loop_mode;
        st.message = if target_loops > 1 {
            format!("Playing '{}' (Loop 1 of {target_loops}, {:.1}x)", target.name, sp)
        } else if loop_mode && target_loops == 0 {
            format!("Playing '{}' (Infinite Loop, {:.1}x)", target.name, sp)
        } else if (sp - 1.0).abs() > 0.05 {
            format!("Playing '{}' ({:.1}x speed)", target.name, sp)
        } else {
            format!("Playing '{}'", target.name)
        };
    }

    std::thread::spawn(move || {
        let mut loop_count = 1u32;
        let mut completed_all = false;
        ctx.log_info(&format!(
            "📼 Starting playback of macro '{}' (speed={:.1}x, loop={loop_mode}, max_loops={target_loops})",
            target.name, sp
        ));

        loop {
            if STOP_PLAYBACK_REQUESTED.load(Ordering::SeqCst) {
                break;
            }

            {
                let mut st = STATUS.write();
                st.current_loop = loop_count;
                st.message = if target_loops > 1 {
                    format!("Playing '{}' (Loop {loop_count} of {target_loops})", target.name)
                } else if loop_mode && target_loops == 0 {
                    format!("Playing '{}' (Loop #{loop_count})", target.name)
                } else {
                    format!("Playing '{}'", target.name)
                };
            }

            let mut finished_steps = true;
            for step in &target.steps {
                if STOP_PLAYBACK_REQUESTED.load(Ordering::SeqCst) {
                    finished_steps = false;
                    break;
                }

                match step {
                    MacroStep::Click { rx, ry, button, delay_ms } => {
                        let scaled_delay = ((*delay_ms as f32) / sp).round() as u64;
                        if scaled_delay > 0 && !sleep_responsive(scaled_delay) {
                            finished_steps = false;
                            break;
                        }
                        ctx.ensure_roblox_focus();
                        if let Some(rect) = ctx.roblox_rect() {
                            let px = rect.x + (rx.clamp(0.0, 1.0) * rect.w as f32).round() as i32;
                            let py = rect.y + (ry.clamp(0.0, 1.0) * rect.h as f32).round() as i32;
                            ctx.platform.input.move_to(PxPoint { x: px, y: py });
                            let pre_sleep = if sp >= 20.0 { 0 } else if sp >= 5.0 { 2 } else { (20.0 / sp).round().max(4.0) as u64 };
                            if pre_sleep > 0 {
                                std::thread::sleep(Duration::from_millis(pre_sleep));
                            }
                            let btn = if button == "right" {
                                MouseButton::Right
                            } else {
                                MouseButton::Left
                            };
                            ctx.platform.input.button(btn, true);
                            let hold_sleep = if sp >= 50.0 { 1 } else if sp >= 10.0 { 3 } else { (40.0 / sp).round().max(6.0) as u64 };
                            std::thread::sleep(Duration::from_millis(hold_sleep));
                            ctx.platform.input.button(btn, false);
                            let post_sleep = if sp >= 20.0 { 0 } else if sp >= 5.0 { 1 } else { (15.0 / sp).round().max(3.0) as u64 };
                            if post_sleep > 0 {
                                std::thread::sleep(Duration::from_millis(post_sleep));
                            }
                        }
                    }
                    MacroStep::Drag { start_rx, start_ry, end_rx, end_ry, duration_ms, delay_ms } => {
                        let scaled_delay = ((*delay_ms as f32) / sp).round() as u64;
                        let scaled_duration = if sp >= 50.0 { 4 } else if sp >= 10.0 { 12 } else { ((*duration_ms as f32) / sp).round().max(20.0) as u64 };
                        if scaled_delay > 0 && !sleep_responsive(scaled_delay) {
                            finished_steps = false;
                            break;
                        }
                        ctx.ensure_roblox_focus();
                        if let Some(rect) = ctx.roblox_rect() {
                            let start_px = rect.x + (start_rx.clamp(0.0, 1.0) * rect.w as f32).round() as i32;
                            let start_py = rect.y + (start_ry.clamp(0.0, 1.0) * rect.h as f32).round() as i32;
                            let end_px = rect.x + (end_rx.clamp(0.0, 1.0) * rect.w as f32).round() as i32;
                            let end_py = rect.y + (end_ry.clamp(0.0, 1.0) * rect.h as f32).round() as i32;

                            // 1. Move to start position
                            ctx.platform.input.move_to(PxPoint { x: start_px, y: start_py });
                            let pre_drag = if sp >= 20.0 { 1 } else { (30.0 / sp).round().max(4.0) as u64 };
                            std::thread::sleep(Duration::from_millis(pre_drag));

                            // 2. Mouse button DOWN
                            ctx.platform.input.button(MouseButton::Left, true);
                            let down_wait = if sp >= 20.0 { 2 } else { (40.0 / sp).round().max(6.0) as u64 };
                            std::thread::sleep(Duration::from_millis(down_wait));

                            // 3. Step interpolation
                            let num_steps = if sp >= 50.0 { 2 } else if sp >= 10.0 { 4 } else { ((scaled_duration as f32 / 12.0).round() as i32).clamp(4, 30) };
                            let step_delay = if sp >= 20.0 { 1 } else { (scaled_duration / num_steps as u64).max(2) };
                            for i in 1..=num_steps {
                                if STOP_PLAYBACK_REQUESTED.load(Ordering::SeqCst) {
                                    break;
                                }
                                let t = i as f32 / num_steps as f32;
                                let ease = t * t * (3.0 - 2.0 * t);
                                let cur_x = (start_px as f32 + (end_px - start_px) as f32 * ease).round() as i32;
                                let cur_y = (start_py as f32 + (end_py - start_py) as f32 * ease).round() as i32;
                                ctx.platform.input.move_to(PxPoint { x: cur_x, y: cur_y });
                                std::thread::sleep(Duration::from_millis(step_delay));
                            }

                            // 4. Ensure at target position
                            ctx.platform.input.move_to(PxPoint { x: end_px, y: end_py });
                            let post_drag = if sp >= 20.0 { 1 } else { (40.0 / sp).round().max(6.0) as u64 };
                            std::thread::sleep(Duration::from_millis(post_drag));

                            // 5. Mouse button UP
                            ctx.platform.input.button(MouseButton::Left, false);
                            let up_wait = if sp >= 20.0 { 1 } else { (25.0 / sp).round().max(3.0) as u64 };
                            std::thread::sleep(Duration::from_millis(up_wait));
                        }
                    }
                    MacroStep::KeyTap { key, delay_ms } => {
                        let scaled_delay = ((*delay_ms as f32) / sp).round() as u64;
                        if scaled_delay > 0 && !sleep_responsive(scaled_delay) {
                            finished_steps = false;
                            break;
                        }
                        ctx.ensure_roblox_focus();
                        if let Some(k) = parse_key(key) {
                            ctx.platform.input.key(k, true);
                            let tap_dur = if sp >= 50.0 { 1 } else if sp >= 10.0 { 3 } else { (50.0 / sp).round().max(5.0) as u64 };
                            std::thread::sleep(Duration::from_millis(tap_dur));
                            ctx.platform.input.key(k, false);
                        }
                    }
                    MacroStep::KeyHold { key, duration_ms, delay_ms } => {
                        let scaled_delay = ((*delay_ms as f32) / sp).round() as u64;
                        let scaled_duration = if sp >= 50.0 { ((*duration_ms as f32) / sp).round().max(1.0) as u64 } else { ((*duration_ms as f32) / sp).round().max(4.0) as u64 };
                        if scaled_delay > 0 && !sleep_responsive(scaled_delay) {
                            finished_steps = false;
                            break;
                        }
                        ctx.ensure_roblox_focus();
                        if let Some(k) = parse_key(key) {
                            ctx.platform.input.key(k, true);
                            if scaled_duration > 0 && !sleep_responsive(scaled_duration) {
                                ctx.platform.input.key(k, false);
                                finished_steps = false;
                                break;
                            }
                            ctx.platform.input.key(k, false);
                        }
                    }
                    MacroStep::MouseMove { rx, ry, delay_ms } => {
                        let scaled_delay = ((*delay_ms as f32) / sp).round() as u64;
                        if scaled_delay > 0 && !sleep_responsive(scaled_delay) {
                            finished_steps = false;
                            break;
                        }
                        ctx.ensure_roblox_focus();
                        if let Some(rect) = ctx.roblox_rect() {
                            let px = rect.x + (rx.clamp(0.0, 1.0) * rect.w as f32).round() as i32;
                            let py = rect.y + (ry.clamp(0.0, 1.0) * rect.h as f32).round() as i32;
                            ctx.platform.input.move_to(PxPoint { x: px, y: py });
                        }
                    }
                    MacroStep::Sleep { ms } => {
                        let scaled_ms = ((*ms as f32) / sp).round() as u64;
                        if scaled_ms > 0 && !sleep_responsive(scaled_ms) {
                            finished_steps = false;
                            break;
                        }
                    }
                    MacroStep::VpnConnect { .. }
                    | MacroStep::VpnDisconnect { .. }
                    | MacroStep::VpnWaitConnected { .. }
                    | MacroStep::VpnWaitDisconnected { .. } => {
                        // VPN waits use real seconds (never speed-scaled).
                        let provider = VPN_PROVIDER.read().clone();
                        if let Err(e) = execute_vpn_step(provider.as_ref(), step) {
                            ctx.log_error(&format!("📼 Macro '{}' stopped: {e}", target.name));
                            let mut st = STATUS.write();
                            st.message = format!("Macro stopped: {e}");
                            finished_steps = false;
                            break;
                        }
                    }
                }
            }

            if !finished_steps || !loop_mode || STOP_PLAYBACK_REQUESTED.load(Ordering::SeqCst) {
                // Clean exit only for a successful single play: loop mode
                // continues below, failures/stops stay incomplete.
                completed_all = finished_steps
                    && !loop_mode
                    && !STOP_PLAYBACK_REQUESTED.load(Ordering::SeqCst);
                break;
            }

            if target_loops > 0 && loop_count >= target_loops {
                completed_all = finished_steps;
                break;
            }

            loop_count += 1;
            let loop_delay = if sp >= 50.0 { 1 } else if sp >= 10.0 { 4 } else { (250.0 / sp).round().max(10.0) as u64 };
            if loop_delay > 0 && !sleep_responsive(loop_delay) {
                break;
            }
        }

        IS_PLAYING.store(false, Ordering::SeqCst);
        if completed_all {
            let mut st = STATUS.write();
            st.is_playing = false;
            st.playing_macro_name = None;
            st.is_looping = false;
            st.message = format!("Macro '{}' finished (completed {loop_count} loops)", target.name);
            ctx.log_info(&format!("📼 Macro '{}' finished after {loop_count} loop(s).", target.name));
        } else {
            // A step already recorded the specific failure in STATUS.message;
            // only clear the playing flags here, never overwrite the reason.
            let mut st = STATUS.write();
            st.is_playing = false;
            st.playing_macro_name = None;
            st.is_looping = false;
            if !st.message.starts_with("Macro stopped:") {
                st.message = format!("Macro '{}' stopped before completion", target.name);
            }
            ctx.log_info(&format!("📼 Macro '{}' stopped before completion.", target.name));
        }
    });

    Ok(())
}

fn parse_key(key_str: &str) -> Option<Key> {
    match key_str.to_lowercase().as_str() {
        "w" => Some(Key::Char('w')),
        "a" => Some(Key::Char('a')),
        "s" => Some(Key::Char('s')),
        "d" => Some(Key::Char('d')),
        "t" => Some(Key::Char('t')),
        "e" => Some(Key::Char('e')),
        "1" => Some(Key::Char('1')),
        "2" => Some(Key::Char('2')),
        "3" => Some(Key::Char('3')),
        "4" => Some(Key::Char('4')),
        "5" => Some(Key::Char('5')),
        "space" | "jump" => Some(Key::Char(' ')),
        "shift" => Some(Key::Shift),
        "left" => Some(Key::Left),
        "right" => Some(Key::Right),
        "up" => Some(Key::Up),
        "down" => Some(Key::Down),
        s if s.len() == 1 => s.chars().next().map(Key::Char),
        _ => None,
    }
}

fn sleep_responsive(ms: u64) -> bool {
    let step = 25;
    let mut elapsed = 0;
    while elapsed < ms {
        if STOP_PLAYBACK_REQUESTED.load(Ordering::SeqCst) {
            return false;
        }
        #[cfg(windows)]
        {
            use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
            // Hotkey F8 (0x77) or F9 (0x78) halts macro playback directly from laptop keyboard
            let f8 = (unsafe { GetAsyncKeyState(0x77) } as u16 & 0x8000) != 0;
            let f9 = (unsafe { GetAsyncKeyState(0x78) } as u16 & 0x8000) != 0;
            if f8 || f9 {
                STOP_PLAYBACK_REQUESTED.store(true, Ordering::SeqCst);
                return false;
            }
        }
        std::thread::sleep(Duration::from_millis(step.min(ms - elapsed)));
        elapsed += step;
    }
    !STOP_PLAYBACK_REQUESTED.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpn::{VpnState, VpnStatus};

    /// Scripted fake provider: scripted verification outcomes, real state
    /// tracking. Never touches the OS.
    struct FakeVpn {
        state: parking_lot::Mutex<VpnState>,
        connect_ok: bool,
        wait_ok: bool,
        calls: parking_lot::Mutex<Vec<String>>,
    }

    impl FakeVpn {
        fn succeeding() -> Self {
            Self {
                state: parking_lot::Mutex::new(VpnState::Disconnected),
                connect_ok: true,
                wait_ok: true,
                calls: parking_lot::Mutex::new(Vec::new()),
            }
        }

        fn failing() -> Self {
            Self {
                state: parking_lot::Mutex::new(VpnState::Disconnected),
                connect_ok: false,
                wait_ok: false,
                calls: parking_lot::Mutex::new(Vec::new()),
            }
        }

        fn status(&self) -> VpnStatus {
            let st = *self.state.lock();
            VpnStatus {
                connected: st == VpnState::Connected,
                state: st,
                managed: true,
                engine: "fake".to_string(),
                engine_name: "Fake".to_string(),
                ip: String::new(),
                country: String::new(),
                city: String::new(),
                latency_ms: None,
                uptime_secs: 0,
                auto_reconnect: false,
                last_error: None,
                auto_detected: false,
                process_running: false,
                tunnel_detected: false,
                verification_detail: "fake".to_string(),
            }
        }
    }

    impl crate::vpn::VpnControl for FakeVpn {
        fn vpn_state(&self) -> VpnState {
            *self.state.lock()
        }

        fn vpn_connect_verified(&self, engine: &str, _timeout: Duration) -> Result<VpnStatus, String> {
            self.calls.lock().push(format!("connect:{engine}"));
            if self.connect_ok {
                *self.state.lock() = VpnState::Connected;
                Ok(self.status())
            } else {
                *self.state.lock() = VpnState::Error;
                Err("fake verification timeout".to_string())
            }
        }

        fn vpn_disconnect_verified(&self, _timeout: Duration) -> Result<VpnStatus, String> {
            self.calls.lock().push("disconnect".to_string());
            *self.state.lock() = VpnState::Disconnected;
            Ok(self.status())
        }

        fn vpn_wait_for(&self, want_connected: bool, _timeout: Duration) -> Result<VpnStatus, String> {
            self.calls
                .lock()
                .push(format!("wait:{}", if want_connected { "up" } else { "down" }));
            let st = self.status();
            if st.connected == want_connected && self.wait_ok {
                Ok(st)
            } else {
                Err("fake wait timeout".to_string())
            }
        }
    }

    fn provider(fake: FakeVpn) -> Arc<dyn crate::vpn::VpnControl> {
        Arc::new(fake)
    }

    #[test]
    fn vpn_connect_success_continues_macro() {
        let p = provider(FakeVpn::succeeding());
        let step = MacroStep::VpnConnect { engine: "auto".into(), timeout_s: 20, required: true };
        assert!(execute_vpn_step(Some(&p), &step).is_ok());
    }

    #[test]
    fn vpn_connect_timeout_fails_macro_when_required() {
        let p = provider(FakeVpn::failing());
        let step = MacroStep::VpnConnect { engine: "dedicated".into(), timeout_s: 20, required: true };
        let err = execute_vpn_step(Some(&p), &step).unwrap_err();
        assert!(err.contains("VPN connection failed"), "got: {err}");
        assert!(err.contains("dedicated"), "got: {err}");
        assert!(err.contains("Expected: connected"), "got: {err}");
    }

    #[test]
    fn vpn_connect_timeout_continues_when_optional() {
        let p = provider(FakeVpn::failing());
        let step = MacroStep::VpnConnect { engine: "warp".into(), timeout_s: 5, required: false };
        assert!(execute_vpn_step(Some(&p), &step).is_ok());
    }

    #[test]
    fn vpn_wait_connected_timeout_fails_when_required() {
        let p = provider(FakeVpn::failing());
        let step = MacroStep::VpnWaitConnected { timeout_s: 10, required: true };
        let err = execute_vpn_step(Some(&p), &step).unwrap_err();
        assert!(err.contains("VPN wait failed"), "got: {err}");
    }

    #[test]
    fn vpn_wait_disconnected_success() {
        let p = provider(FakeVpn::succeeding());
        let step = MacroStep::VpnWaitDisconnected { timeout_s: 10 };
        assert!(execute_vpn_step(Some(&p), &step).is_ok());
    }

    #[test]
    fn vpn_disconnect_reports_provider_errors() {
        struct FailingDisconnect(FakeVpn);
        impl crate::vpn::VpnControl for FailingDisconnect {
            fn vpn_state(&self) -> VpnState {
                self.0.vpn_state()
            }
            fn vpn_connect_verified(&self, e: &str, t: Duration) -> Result<VpnStatus, String> {
                self.0.vpn_connect_verified(e, t)
            }
            fn vpn_disconnect_verified(&self, _t: Duration) -> Result<VpnStatus, String> {
                Err("fake stuck tunnel".to_string())
            }
            fn vpn_wait_for(&self, w: bool, t: Duration) -> Result<VpnStatus, String> {
                self.0.vpn_wait_for(w, t)
            }
        }
        let p: Arc<dyn crate::vpn::VpnControl> = Arc::new(FailingDisconnect(FakeVpn::succeeding()));
        let step = MacroStep::VpnDisconnect { timeout_s: 8 };
        let err = execute_vpn_step(Some(&p), &step).unwrap_err();
        assert!(err.contains("disconnect"), "got: {err}");
    }

    #[test]
    fn vpn_step_without_provider_fails_loudly() {
        let step = MacroStep::VpnConnect { engine: "auto".into(), timeout_s: 20, required: true };
        let err = execute_vpn_step(None, &step).unwrap_err();
        assert!(err.contains("unavailable"), "got: {err}");
    }

    #[test]
    fn vpn_steps_reject_non_vpn_steps() {
        let p = provider(FakeVpn::succeeding());
        let step = MacroStep::Sleep { ms: 100 };
        assert!(execute_vpn_step(Some(&p), &step).is_err());
    }

    #[test]
    fn vpn_step_json_round_trips() {
        let step = MacroStep::VpnConnect { engine: "warp".into(), timeout_s: 30, required: false };
        let j = serde_json::to_string(&step).unwrap();
        assert!(j.contains("VpnConnect"));
        let back: MacroStep = serde_json::from_str(&j).unwrap();
        match back {
            MacroStep::VpnConnect { engine, timeout_s, required } => {
                assert_eq!(engine, "warp");
                assert_eq!(timeout_s, 30);
                assert!(!required);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn append_vpn_step_never_autoconnects_other_steps() {
        // A macro without VPN steps is untouched: appending only adds.
        let dir = std::env::temp_dir().join(format!("gpo-macro-vpn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(dir.clone());
        save_macros(
            &store,
            &[CustomMacro {
                id: "m1".into(),
                name: "Plain".into(),
                created_at: "t".into(),
                steps: vec![MacroStep::Sleep { ms: 100 }],
            }],
        )
        .unwrap();
        let before = load_macros(&store);
        assert_eq!(before[0].steps.len(), 1);

        let after = append_vpn_step(&store, "Plain", VpnMacroAction::Connect, Some("WARP".into()), Some(25), None)
            .unwrap();
        assert_eq!(after.steps.len(), 2);
        match &after.steps[1] {
            MacroStep::VpnConnect { engine, timeout_s, required } => {
                assert_eq!(engine, "warp"); // normalized
                assert_eq!(*timeout_s, 25);
                assert!(*required); // default true
            }
            _ => panic!("wrong variant"),
        }
        // Timeout clamping: 1000s → 120s.
        append_vpn_step(&store, "Plain", VpnMacroAction::WaitConnected, None, Some(1000), Some(false)).unwrap();
        let list = load_macros(&store);
        match &list[0].steps[2] {
            MacroStep::VpnWaitConnected { timeout_s, required } => {
                assert_eq!(*timeout_s, 120);
                assert!(!required);
            }
            _ => panic!("wrong variant"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
