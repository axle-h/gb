use crate::activation::Activation;
/// https://gbdev.io/pandocs/Joypad_Input.html#ff00--p1joyp-joypad
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JoypadRegister {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    a: bool,
    b: bool,
    select: bool,
    start: bool,
    select_buttons: bool,
    select_directions: bool,
    interrupt_pending: bool,
}

impl Default for JoypadRegister {
    fn default() -> Self {
        Self {
            up: false,
            down: false,
            left: false,
            right: false,
            a: false,
            b: false,
            select: false,
            start: false,
            select_buttons: false,
            select_directions: false,
            interrupt_pending: false,
        }
    }
}

impl JoypadRegister {
    pub fn set(&mut self, value: u8) {
        self.select_buttons = (value & 0x20) == 0;
        self.select_directions = (value & 0x10) == 0;
    }

    pub fn get(&self) -> u8 {
        let button_bits = if self.select_buttons {
            (self.a as u8) | ((self.b as u8) << 1) | ((self.select as u8) << 2) | ((self.start as u8) << 3)
        } else { 0 };

        let direction_bits = if self.select_directions {
            (self.right as u8) | ((self.left as u8) << 1) | ((self.up as u8) << 2) | ((self.down as u8) << 3)
        } else { 0 };

        let value = button_bits | direction_bits;

        // Button pressed = bit is 0, so invert the lower 4 bits
        (!value & 0xF) | (!self.select_buttons as u8) << 5 | (!self.select_directions as u8) << 4
    }

    pub fn is_button_pressed(&self, button: JoypadButton) -> bool {
        match button {
            JoypadButton::Up => self.up,
            JoypadButton::Down => self.down,
            JoypadButton::Left => self.left,
            JoypadButton::Right => self.right,
            JoypadButton::A => self.a,
            JoypadButton::B => self.b,
            JoypadButton::Select => self.select,
            JoypadButton::Start => self.start,
        }
    }

    pub fn update_button(&mut self, button: JoypadButton, pressed: bool) {
        self.interrupt_pending = self.interrupt_pending || (pressed && !self.is_button_pressed(button));
        match button {
            JoypadButton::Up => self.up = pressed,
            JoypadButton::Down => self.down = pressed,
            JoypadButton::Left => self.left = pressed,
            JoypadButton::Right => self.right = pressed,
            JoypadButton::A => self.a = pressed,
            JoypadButton::B => self.b = pressed,
            JoypadButton::Select => self.select = pressed,
            JoypadButton::Start => self.start = pressed,
        }
    }

    pub fn press_button(&mut self, button: JoypadButton) {
        self.update_button(button, true);
    }

    pub fn release_button(&mut self, button: JoypadButton) {
        self.update_button(button, false);
    }
}

impl Activation for JoypadRegister {
    fn is_activation_pending(&self) -> bool {
        self.interrupt_pending
    }

    fn clear_activation(&mut self) {
        self.interrupt_pending = false;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::EnumIter, strum_macros::Display)]
pub enum JoypadButton {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    Select,
    Start,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::joypad::JoypadButton::*;

    #[test]
    fn to_byte() {
        let mut joypad = JoypadRegister::default();
        assert_eq!(joypad.get(), 0x3F); // All buttons released
        joypad.set(0x10); // Select buttons
        assert_eq!(joypad.get(), 0x1F); // none pressed
        joypad.press_button(A);
        joypad.press_button(B);
        joypad.press_button(Select);
        joypad.press_button(Start);
        assert_eq!(joypad.get(), 0x10);

        joypad.set(0x20); // Select directions
        assert_eq!(joypad.get(), 0x2F); // none pressed
        joypad.press_button(Up);
        joypad.press_button(Down);
        joypad.press_button(Left);
        joypad.press_button(Right);
        assert_eq!(joypad.get(), 0x20); // All directions pressed
    }

    #[test]
    fn interrupts() {
        let mut joypad = JoypadRegister::default();
        assert!(!joypad.is_activation_pending()); // disabled by default
        joypad.release_button(A);
        assert!(!joypad.is_activation_pending()); // no interrupt on release
        joypad.press_button(A);
        assert!(joypad.is_activation_pending()); // interrupt on press
        joypad.release_button(A);
        assert!(joypad.is_activation_pending()); // still interrupt required until read
    }
}