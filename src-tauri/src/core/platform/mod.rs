use std::sync::Arc;

use crate::core::types::{Frame, Key, MouseButton, PxPoint, PxRect, WindowInfo};

pub mod capture_bounds;
pub mod ocr_gate;
pub mod playback_guard;
pub mod mock;
pub mod capture_broker;
pub mod guarded_input;
#[cfg(windows)]
pub mod windows;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("game window not found")]
    WindowNotFound,
    #[error("capture failed: {0}")]
    Capture(String),
    #[error("ocr unavailable")]
    OcrUnavailable,
    #[error("ocr failed: {0}")]
    Ocr(String),
    #[error("input failed: {0}")]
    Input(String),
}

pub type Result<T> = std::result::Result<T, PlatformError>;

pub trait GameWindow: Send + Sync {
    fn find(&self) -> Option<WindowInfo>;
    fn focus(&self) -> bool;
}

pub trait Capture: Send + Sync {
    fn grab(&self, rect: PxRect) -> Result<Frame>;
    fn grab_preview(&self, rect: PxRect) -> Result<Frame> { self.grab(rect) }
    fn preview_age(&self, _rect: PxRect) -> Option<std::time::Duration> { None }
}

pub trait Input: Send + Sync {
    fn release_all(&self) {}
    fn move_to(&self, p: PxPoint);
    fn button(&self, b: MouseButton, down: bool);
    fn key(&self, k: Key, down: bool);
    fn wheel(&self, delta: i32);
    fn type_text(&self, s: &str);
    fn cursor(&self) -> PxPoint;
    fn screen(&self) -> PxRect;
}

pub trait Ocr: Send + Sync {
    fn available(&self) -> bool;
    fn read(&self, frame: &Frame) -> Result<String>;
    /// Optional per-read budget. Existing deterministic providers retain their
    /// synchronous implementation; asynchronous providers must override this to
    /// cancel on expiry and reject late results.
    fn read_with_timeout(&self, frame: &Frame, _timeout: std::time::Duration) -> Result<String> {
        self.read(frame)
    }
}

#[derive(Clone)]
pub struct Platform {
    pub window: Arc<dyn GameWindow>,
    pub capture: Arc<dyn Capture>,
    pub input: Arc<dyn Input>,
    pub ocr: Arc<dyn Ocr>,
}

#[cfg(windows)]
pub fn build() -> Platform {
    let window: Arc<dyn GameWindow> = Arc::new(windows::window::Win32Window::new("Roblox"));
    let input = Arc::new(guarded_input::GuardedInput::new(Arc::clone(&window), Arc::new(windows::input::SendInputBackend::new())));
    guarded_input::GuardedInput::watch(&input);
    Platform {
        window,
        capture: Arc::new(capture_broker::PriorityCapture::new(Arc::new(windows::capture_dxgi::DxgiCapture::new()))),
        input,
        ocr: build_ocr(),
    }
}

#[cfg(all(windows, feature = "ocr-windows", not(feature = "ocr-none")))]
fn build_ocr() -> Arc<dyn Ocr> {
    Arc::new(windows::ocr::WindowsOcr::new())
}

#[cfg(all(windows, any(feature = "ocr-none", not(feature = "ocr-windows"))))]
fn build_ocr() -> Arc<dyn Ocr> {
    Arc::new(mock::NoOcr)
}

#[cfg(not(windows))]
pub fn build() -> Platform {
    mock::platform()
}

#[cfg(test)]
mod ocr_timeout_api_tests {
    use super::*;
    struct LegacyOcr;
    impl Ocr for LegacyOcr {
        fn available(&self) -> bool { true }
        fn read(&self, _: &Frame) -> Result<String> { Ok("legacy provider".into()) }
    }
    #[test]
    fn timeout_api_is_backward_compatible_for_existing_implementations() {
        let provider: &dyn Ocr = &LegacyOcr;
        assert_eq!(provider.read_with_timeout(&Frame::default(), std::time::Duration::from_millis(200)).unwrap(), "legacy provider");
    }
}
