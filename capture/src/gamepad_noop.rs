use crate::KeyEvent;

pub struct GamepadEmulator {}

impl GamepadEmulator {
    pub fn new() -> Self {
        Self {}
    }

    pub fn send_gamepad_event(&mut self, ev: KeyEvent) {}
}
