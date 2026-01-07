use bincode::{Decode, Encode};
use crate::activation::Activation;
/// https://gbdev.io/pandocs/Joypad_Input.html#ff00--p1joyp-joypad
#[derive(Debug, Clone, Copy, PartialEq, Eq, Decode, Encode, Default)]
pub struct JoypadRegister {
    state: JoypadButtonState,
    select_buttons: bool,
    select_directions: bool,
    interrupt_pending: bool,
}

impl JoypadRegister {
    pub fn set(&mut self, value: u8) {
        self.select_buttons = (value & 0x20) == 0;
        self.select_directions = (value & 0x10) == 0;
    }

    pub fn state(&self) -> JoypadButtonState {
        self.state
    }

    pub fn get(&self) -> u8 {
        let button_bits = if self.select_buttons {
            (self.state.a as u8) | ((self.state.b as u8) << 1) | ((self.state.select as u8) << 2) | ((self.state.start as u8) << 3)
        } else { 0 };

        let direction_bits = if self.select_directions {
            (self.state.right as u8) | ((self.state.left as u8) << 1) | ((self.state.up as u8) << 2) | ((self.state.down as u8) << 3)
        } else { 0 };

        let value = button_bits | direction_bits;

        // Button pressed = bit is 0, so invert the lower 4 bits
        (!value & 0xF) | (!self.select_buttons as u8) << 5 | (!self.select_directions as u8) << 4
    }

    pub fn is_button_pressed(&self, button: JoypadButton) -> bool {
        self.state.is_button_pressed(button)
    }

    pub fn update_button(&mut self, button: JoypadButton, pressed: bool) {
        self.interrupt_pending = self.interrupt_pending || (pressed && !self.state.is_button_pressed(button));
        self.state.update_button(button, pressed);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Decode, Encode)]
pub struct JoypadButtonState {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub a: bool,
    pub b: bool,
    pub select: bool,
    pub start: bool,
}

impl JoypadButtonState {
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