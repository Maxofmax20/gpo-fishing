use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::bot::ctx::Ctx;
use crate::config::Store;
use crate::core::types::{Key, MouseButton, PxPoint};

static IS_RECORDING: AtomicBool = AtomicBool::new(false);
static IS_PLAYING: AtomicBool = AtomicBool::new(false);
static STOP_PLAYBACK_REQUESTED: AtomicBool = AtomicBool::new(false);

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
    KeyTap {
        key: String,
        delay_ms: u64,
    },
    Sleep {
        ms: u64,
    },
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
    st.message = format!("Recording started ({mode:?})");
    Ok(())
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

pub fn stop_playback() {
    if IS_PLAYING.load(Ordering::SeqCst) {
        STOP_PLAYBACK_REQUESTED.store(true, Ordering::SeqCst);
        let mut st = STATUS.write();
        st.message = "Stopping macro playback...".into();
    }
}

pub fn play_macro(ctx: Arc<Ctx>, store: Arc<Store>, name_or_id: &str, loop_mode: bool) -> Result<(), String> {
    if IS_PLAYING.swap(true, Ordering::SeqCst) {
        return Err("A macro is already playing".into());
    }

    let list = load_macros(&store);
    let target = list
        .into_iter()
        .find(|m| m.id == name_or_id || m.name.eq_ignore_ascii_case(name_or_id))
        .ok_or_else(|| format!("Macro '{name_or_id}' not found"))?;

    STOP_PLAYBACK_REQUESTED.store(false, Ordering::SeqCst);

    {
        let mut st = STATUS.write();
        st.is_playing = true;
        st.playing_macro_name = Some(target.name.clone());
        st.current_loop = 1;
        st.is_looping = loop_mode;
        st.message = format!("Playing '{}' (Loop 1)", target.name);
    }

    std::thread::spawn(move || {
        let mut loop_count = 1u32;
        ctx.log_info(&format!("📼 Starting playback of macro '{}' (loop={loop_mode})", target.name));

        loop {
            if STOP_PLAYBACK_REQUESTED.load(Ordering::SeqCst) {
                break;
            }

            {
                let mut st = STATUS.write();
                st.current_loop = loop_count;
                st.message = format!("Playing '{}' (Iteration #{loop_count})", target.name);
            }

            let mut finished_steps = true;
            for step in &target.steps {
                if STOP_PLAYBACK_REQUESTED.load(Ordering::SeqCst) {
                    finished_steps = false;
                    break;
                }

                match step {
                    MacroStep::Click { rx, ry, button, delay_ms } => {
                        if !sleep_responsive(*delay_ms) {
                            finished_steps = false;
                            break;
                        }
                        ctx.ensure_roblox_focus();
                        if let Some(rect) = ctx.roblox_rect() {
                            let px = rect.x + (rx.clamp(0.0, 1.0) * rect.w as f32).round() as i32;
                            let py = rect.y + (ry.clamp(0.0, 1.0) * rect.h as f32).round() as i32;
                            ctx.platform.input.move_to(PxPoint { x: px, y: py });
                            std::thread::sleep(Duration::from_millis(25));
                            let btn = if button == "right" {
                                MouseButton::Right
                            } else {
                                MouseButton::Left
                            };
                            ctx.platform.input.button(btn, true);
                            std::thread::sleep(Duration::from_millis(50));
                            ctx.platform.input.button(btn, false);
                        }
                    }
                    MacroStep::KeyTap { key, delay_ms } => {
                        if !sleep_responsive(*delay_ms) {
                            finished_steps = false;
                            break;
                        }
                        ctx.ensure_roblox_focus();
                        if let Some(k) = parse_key(key) {
                            ctx.platform.input.key(k, true);
                            std::thread::sleep(Duration::from_millis(60));
                            ctx.platform.input.key(k, false);
                        }
                    }
                    MacroStep::Sleep { ms } => {
                        if !sleep_responsive(*ms) {
                            finished_steps = false;
                            break;
                        }
                    }
                }
            }

            if !finished_steps || !loop_mode || STOP_PLAYBACK_REQUESTED.load(Ordering::SeqCst) {
                break;
            }

            loop_count += 1;
            // Short breathing room between loops
            if !sleep_responsive(300) {
                break;
            }
        }

        IS_PLAYING.store(false, Ordering::SeqCst);
        let mut st = STATUS.write();
        st.is_playing = false;
        st.playing_macro_name = None;
        st.is_looping = false;
        st.message = format!("Macro '{}' finished (completed {loop_count} loops)", target.name);
        ctx.log_info(&format!("📼 Macro '{}' finished after {loop_count} loop(s).", target.name));
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
    let step = 30;
    let mut elapsed = 0;
    while elapsed < ms {
        if STOP_PLAYBACK_REQUESTED.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(step.min(ms - elapsed)));
        elapsed += step;
    }
    !STOP_PLAYBACK_REQUESTED.load(Ordering::SeqCst)
}
