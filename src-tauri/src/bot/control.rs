//! One input contract for web controls and the rebuilt player.
use crate::core::platform::Platform;
use crate::core::types::{Key, MouseButton, PxPoint};
use gpo_control_runtime::{InputAction, InputPort, Runtime};
use parking_lot::RwLock;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, OnceLock,
};
use std::time::Duration;

struct FrameBroker {
    source: Arc<dyn crate::core::platform::Capture>,
    gate: parking_lot::Mutex<()>,
    waiting: AtomicUsize,
}
impl crate::core::platform::Capture for FrameBroker {
    fn grab(
        &self,
        rect: crate::core::types::PxRect,
    ) -> crate::core::platform::Result<crate::core::types::Frame> {
        self.waiting.fetch_add(1, Ordering::SeqCst);
        let guard = self.gate.lock();
        self.waiting.fetch_sub(1, Ordering::SeqCst);
        let result = self.source.grab(rect);
        drop(guard);
        result
    }
}
pub struct Service {
    pub runtime: Arc<Runtime>,
    pub platform: Platform,
    preview: Arc<FrameBroker>,
}

pub static MODE_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
static SERVICE: OnceLock<RwLock<Option<Arc<Service>>>> = OnceLock::new();
pub fn attach(mut platform: Platform) -> Platform {
    let preview = Arc::new(FrameBroker {
        source: platform.capture.clone(),
        gate: parking_lot::Mutex::new(()),
        waiting: AtomicUsize::new(0),
    });
    platform.capture = preview.clone();
    let runtime = Arc::new(Runtime::new(Arc::new(Port(platform.clone()))));
    *SERVICE.get_or_init(|| RwLock::new(None)).write() = Some(Arc::new(Service {
        runtime,
        platform: platform.clone(),
        preview,
    }));
    platform
}
pub fn service() -> Result<Arc<Service>, String> {
    SERVICE
        .get_or_init(|| RwLock::new(None))
        .read()
        .clone()
        .ok_or_else(|| "Input service is not ready".into())
}
pub fn key(name: &str) -> Option<Key> {
    let name = name.to_ascii_lowercase();
    match name.as_str() {
        "space" => Some(Key::Char(' ')),
        "shift" => Some(Key::Shift),
        "ctrl" => Some(Key::Control),
        "enter" => Some(Key::Enter),
        "escape" => Some(Key::Escape),
        "backspace" => Some(Key::Backspace),
        "delete" => Some(Key::Delete),
        "left" => Some(Key::Left),
        "right" => Some(Key::Right),
        "up" => Some(Key::Up),
        "down" => Some(Key::Down),
        s if s.len() == 1 && s.as_bytes()[0].is_ascii_alphanumeric() => {
            s.chars().next().map(Key::Char)
        }
        _ => None,
    }
}
struct Port(Platform);
impl InputPort for Port {
    fn focused(&self) -> bool {
        self.0
            .window
            .find()
            .is_some_and(|w| w.visible && w.is_foreground)
    }
    fn release_all(&self) {
        self.0.input.release_all();
    }
    fn send(&self, action: &InputAction) -> Result<(), String> {
        match action {
            InputAction::KeyDown { key: k } | InputAction::KeyUp { key: k } => {
                self.0.input.key(
                    key(k).ok_or("Unsupported key")?,
                    matches!(action, InputAction::KeyDown { .. }),
                );
            }
            InputAction::Move { x, y } => {
                let rect = self.0.window.find().ok_or("Roblox not found")?.client;
                if rect.is_empty() {
                    return Err("Roblox client area is empty".into());
                }
                self.0.input.move_to(PxPoint {
                    x: rect.x + (*x * (rect.w - 1) as f32).round() as i32,
                    y: rect.y + (*y * (rect.h - 1) as f32).round() as i32,
                });
            }
            InputAction::MouseDown { button } | InputAction::MouseUp { button } => {
                let b = match button.as_str() {
                    "left" => MouseButton::Left,
                    "right" => MouseButton::Right,
                    _ => return Err("Unsupported mouse button".into()),
                };
                self.0
                    .input
                    .button(b, matches!(action, InputAction::MouseDown { .. }));
            }
            InputAction::Wheel { amount } => self.0.input.wheel(*amount),
        }
        Ok(())
    }
}
impl Service {
    pub fn preview(&self) -> Result<crate::core::types::Frame, String> {
        let window = self
            .platform
            .window
            .find()
            .filter(|w| w.visible && w.is_foreground)
            .ok_or("Focus Roblox to view its screen")?;
        if self.preview.waiting.load(Ordering::SeqCst) > 0 {
            return Err("Fishing capture has priority".into());
        }
        let _gate = self
            .preview
            .gate
            .try_lock()
            .ok_or("Fishing capture is busy")?;
        if self.preview.waiting.load(Ordering::SeqCst) > 0 {
            return Err("Fishing capture has priority".into());
        }
        let frame = self
            .preview
            .source
            .grab(window.client)
            .map_err(|e| e.to_string())?;
        if !self
            .platform
            .window
            .find()
            .is_some_and(|w| w.visible && w.is_foreground && w.client == window.client)
        {
            return Err("Window changed during capture".into());
        }
        Ok(frame)
    }

    pub fn focus(&self) -> Result<(), String> {
        if !self.platform.window.focus() {
            return Err("Windows could not focus Roblox".into());
        }
        for _ in 0..25 {
            if self
                .platform
                .window
                .find()
                .is_some_and(|w| w.visible && w.is_foreground)
            {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err("Roblox did not become the active window".into())
    }
    pub fn tap(&self, key: &str) -> Result<(), String> {
        self.tap_at(self.runtime.epoch(), key)
    }
    pub fn tap_at(&self, epoch: u64, key: &str) -> Result<(), String> {
        self.runtime
            .apply(epoch, &InputAction::KeyDown { key: key.into() })?;
        let held = self.runtime.sleep(epoch, Duration::from_millis(50));
        let released = self
            .runtime
            .apply(epoch, &InputAction::KeyUp { key: key.into() });
        if !held {
            return Err("Input was cancelled or focus changed".into());
        }
        released
    }
}
