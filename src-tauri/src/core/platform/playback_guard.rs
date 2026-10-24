//! Recorder playback has its own lifecycle: fishing may be stopped throughout.
use super::GameWindow;
use crate::core::types::WindowInfo;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The explicit Play action permits one focus handoff, never repeated focus stealing.
pub fn acquire_playback_focus(
    window: &dyn GameWindow,
    stop: &AtomicBool,
    timeout: Duration,
) -> Result<WindowInfo, String> {
    if stop.load(Ordering::SeqCst) {
        return Err("Playback was cancelled".into());
    }
    let initial = window.find().ok_or("Roblox window was not found")?;
    if initial.visible && initial.is_foreground {
        return Ok(initial);
    }
    if !window.focus() {
        return Err("Windows could not bring Roblox to the foreground".into());
    }
    let started = Instant::now();
    loop {
        if stop.load(Ordering::SeqCst) {
            return Err("Playback was cancelled".into());
        }
        if let Some(info) = window
            .find()
            .filter(|info| info.visible && info.is_foreground)
        {
            return Ok(info);
        }
        let Some(left) = timeout
            .checked_sub(started.elapsed())
            .filter(|left| !left.is_zero())
        else {
            return Err(
                "Roblox did not become the active window; select Roblox and retry Play".into(),
            );
        };
        std::thread::sleep(left.min(Duration::from_millis(10)));
    }
}

/// A launch key held at Play must be released and pressed again to become Stop.
/// Other stop keys remain active while the launch key is still held.
pub struct StopKeyEdges {
    previous: [bool; 2],
}
impl StopKeyEdges {
    pub fn new(current: [bool; 2]) -> Self {
        Self { previous: current }
    }
    pub fn pressed(&mut self, current: [bool; 2]) -> bool {
        let pressed = current
            .iter()
            .zip(self.previous)
            .any(|(now, previous)| *now && !previous);
        self.previous = current;
        pressed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    struct Window {
        info: Mutex<Option<WindowInfo>>,
        focus_ok: bool,
        focus_calls: std::sync::atomic::AtomicUsize,
    }
    impl GameWindow for Window {
        fn find(&self) -> Option<WindowInfo> {
            *self.info.lock().unwrap()
        }
        fn focus(&self) -> bool {
            self.focus_calls.fetch_add(1, Ordering::SeqCst);
            if self.focus_ok {
                if let Some(info) = self.info.lock().unwrap().as_mut() {
                    info.visible = true;
                    info.is_foreground = true;
                }
            }
            self.focus_ok
        }
    }
    fn window(foreground: bool, focus_ok: bool) -> Window {
        Window {
            info: Mutex::new(Some(WindowInfo {
                client: crate::core::types::PxRect {
                    x: 0,
                    y: 0,
                    w: 800,
                    h: 600,
                },
                is_foreground: foreground,
                visible: true,
                dpi: 96,
            })),
            focus_ok,
            focus_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
    #[test]
    fn app_foreground_play_performs_one_explicit_handoff() {
        let window = window(false, true);
        let info =
            acquire_playback_focus(&window, &AtomicBool::new(false), Duration::from_millis(30))
                .unwrap();
        assert!(info.is_foreground);
        assert_eq!(window.focus_calls.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn already_focused_and_cancelled_plays_do_not_steal_focus() {
        let window = window(true, true);
        assert!(acquire_playback_focus(&window, &AtomicBool::new(false), Duration::ZERO).is_ok());
        assert!(acquire_playback_focus(&window, &AtomicBool::new(true), Duration::ZERO).is_err());
        assert_eq!(window.focus_calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn denied_focus_or_missing_game_fails_without_input() {
        let window = window(false, false);
        assert!(acquire_playback_focus(&window, &AtomicBool::new(false), Duration::ZERO).is_err());
        *window.info.lock().unwrap() = None;
        assert!(acquire_playback_focus(&window, &AtomicBool::new(false), Duration::ZERO).is_err());
        assert_eq!(window.focus_calls.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn held_f8_launch_does_not_stop_until_next_press() {
        let mut keys = StopKeyEdges::new([true, false]);
        assert!(!keys.pressed([true, false]));
        assert!(!keys.pressed([false, false]));
        assert!(keys.pressed([true, false]));
    }
    #[test]
    fn f9_remains_an_immediate_stop_while_f8_launch_is_held() {
        let mut keys = StopKeyEdges::new([true, false]);
        assert!(keys.pressed([true, true]));
    }
}
