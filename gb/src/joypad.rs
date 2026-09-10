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
        let before = self.low_nibble();
        self.select_buttons = (value & 0x20) == 0;
        self.select_directions = (value & 0x10) == 0;
        // D9: writing the select bits can reveal an already-held button, and that edge is an
        // interrupt just as much as a fresh press is.
        self.raise_on_falling_edge(before);
    }

    /// The four button lines as the guest reads them: **`1` is released**, and a line only goes
    /// low when its group is selected.
    fn low_nibble(&self) -> u8 {
        self.get() & 0x0F
    }

    /// **D9.** Hardware raises the joypad interrupt on a **high-to-low edge of a register line**,
    /// not on a button press.
    ///
    /// ⚠️ The difference is real: `gb` used to fire on any press, including one in a group the
    /// guest has not selected — where the lines do not move at all — and to *miss* the edge
    /// produced by selecting a group that already has a button held down.
    fn raise_on_falling_edge(&mut self, before: u8) {
        let after = self.low_nibble();
        if before & !after != 0 {
            self.interrupt_pending = true;
        }
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
        let before = self.low_nibble();
        self.state.update_button(button, pressed);
        self.raise_on_falling_edge(before);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum_macros::EnumIter, strum_macros::Display)]
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
        // A13: bits 6-7 are unused and read as 1 on hardware; the MMU ORs in 0xC0.
        assert_eq!(joypad.get(), 0x3F); // All buttons released (before the 0xC0 mask)
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
        joypad.set(0x10); // select buttons, so a press moves a line
        assert!(!joypad.is_activation_pending()); // disabled by default
        joypad.release_button(A);
        assert!(!joypad.is_activation_pending()); // no interrupt on release
        joypad.press_button(A);
        assert!(joypad.is_activation_pending()); // interrupt on press
        joypad.release_button(A);
        assert!(joypad.is_activation_pending()); // still interrupt required until read
    }

    /// **D9.** The interrupt follows the *register lines*, not the buttons. A press in a group the
    /// guest has not selected moves no line, so it raises nothing.
    #[test]
    fn a_press_in_an_unselected_group_raises_nothing() {
        let mut joypad = JoypadRegister::default();
        joypad.set(0x10); // buttons selected, directions not
        joypad.press_button(Up);
        assert!(!joypad.is_activation_pending(), "directions are not selected");
        joypad.press_button(A);
        assert!(joypad.is_activation_pending(), "...but buttons are");
    }

    /// ...and the converse, which the old press-triggered code could not produce at all:
    /// selecting a group that already has a button held pulls a line low, and that is an edge.
    #[test]
    fn selecting_a_group_with_a_button_held_raises_the_interrupt() {
        let mut joypad = JoypadRegister::default();
        joypad.set(0x10); // buttons selected
        joypad.press_button(Down);
        assert!(!joypad.is_activation_pending());

        joypad.set(0x20); // now select directions — Down is already held
        assert!(joypad.is_activation_pending());
    }

    /// With neither group selected every line is high, so nothing can produce an edge.
    #[test]
    fn nothing_is_raised_while_no_group_is_selected() {
        let mut joypad = JoypadRegister::default();
        joypad.set(0x30); // neither group
        for button in [A, B, Up, Down, Left, Right, Start, Select] {
            joypad.press_button(button);
        }
        assert!(!joypad.is_activation_pending());
    }
}