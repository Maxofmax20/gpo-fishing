//! Every input surface uses a fresh foreground check, including playback and HTTP.
use super::{GameWindow, Input};
use crate::core::types::{Key, MouseButton, PxPoint, PxRect};
use parking_lot::Mutex;
use std::sync::Arc;

pub struct GuardedInput {
    window: Arc<dyn GameWindow>,
    input: Arc<dyn Input>,
    held: Mutex<(Vec<Key>, Vec<MouseButton>)>,
}
impl GuardedInput {
    pub fn new(window: Arc<dyn GameWindow>, input: Arc<dyn Input>) -> Self {
        Self {
            window,
            input,
            held: Mutex::new((Vec::new(), Vec::new())),
        }
    }
    fn focused(&self) -> bool {
        let focused = self
            .window
            .find()
            .is_some_and(|w| w.visible && w.is_foreground);
        if !focused {
            self.release_all();
        }
        focused
    }
    pub fn watch(this: &Arc<Self>) {
        let weak = Arc::downgrade(this);
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(25));
            let Some(input) = weak.upgrade() else { break };
            input.focused();
        });
    }
}
impl Input for GuardedInput {
    fn move_to(&self, p: PxPoint) {
        if self.focused() {
            self.input.move_to(p);
        }
    }
    fn button(&self, button: MouseButton, down: bool) {
        if down && !self.focused() {
            return;
        }
        let mut held = self.held.lock();
        if down {
            if !held.1.contains(&button) {
                held.1.push(button);
            }
        } else {
            held.1.retain(|b| *b != button);
        }
        self.input.button(button, down);
    }
    fn key(&self, key: Key, down: bool) {
        if down && !self.focused() {
            return;
        }
        let mut held = self.held.lock();
        if down {
            if !held.0.contains(&key) {
                held.0.push(key);
            }
        } else {
            held.0.retain(|k| *k != key);
        }
        self.input.key(key, down);
    }
    fn wheel(&self, delta: i32) {
        if self.focused() {
            self.input.wheel(delta);
        }
    }
    fn type_text(&self, text: &str) {
        let mut buffer = [0; 4];
        for ch in text.chars() {
            if !self.focused() {
                break;
            }
            self.input.type_text(ch.encode_utf8(&mut buffer));
        }
    }
    fn cursor(&self) -> PxPoint {
        self.input.cursor()
    }
    fn screen(&self) -> PxRect {
        self.input.screen()
    }
    fn release_all(&self) {
        let mut held = self.held.lock();
        for key in held.0.drain(..) {
            self.input.key(key, false);
        }
        for button in held.1.drain(..) {
            self.input.button(button, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::platform::mock::{InputEvent, MockWindow, RecordingInput};
    use crate::core::types::WindowInfo;
    #[test]
    fn focus_loss_releases_all_held_input_and_refuses_further_actions() {
        let window = Arc::new(MockWindow::default());
        *window.info.lock() = Some(WindowInfo {
            visible: true,
            is_foreground: true,
            client: PxRect::default(),
            dpi: 96,
        });
        let raw = Arc::new(RecordingInput::default());
        let guarded = GuardedInput::new(window.clone(), raw.clone());
        guarded.button(MouseButton::Left, true);
        guarded.key(Key::Char('e'), true);
        window.info.lock().as_mut().unwrap().is_foreground = false;
        guarded.move_to(PxPoint { x: 5, y: 5 });
        guarded.key(Key::Char('x'), true);
        assert_eq!(
            *raw.events.lock(),
            vec![
                InputEvent::Button(MouseButton::Left, true),
                InputEvent::Key(Key::Char('e'), true),
                InputEvent::Key(Key::Char('e'), false),
                InputEvent::Button(MouseButton::Left, false)
            ]
        );
    }
}
