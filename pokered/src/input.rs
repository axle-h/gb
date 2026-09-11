use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// `hJoyHeld`'s bit order, so a harvested tape maps across.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
    pub struct Joypad: u8 {
        const A      = 1 << 0;
        const B      = 1 << 1;
        const SELECT = 1 << 2;
        const START  = 1 << 3;
        const RIGHT  = 1 << 4;
        const LEFT   = 1 << 5;
        const UP     = 1 << 6;
        const DOWN   = 1 << 7;
    }
}

/// The joypad as `_Joypad` leaves it. Edges are against the last `poll`, not the last frame, so a
/// routine that stops polling for a few frames sees a press made meanwhile when it resumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Pad {
    /// `hJoyInput`: what the hardware read at the start of this frame.
    pub input: Joypad,
    last: Joypad,
    pub held: Joypad,
    pub pressed: Joypad,
    pub released: Joypad,
    /// `wJoyIgnore`.
    pub ignore: Joypad,
    /// `hJoy7`, which a list menu sets and clears.
    pub repeat_held: bool,
    /// `hJoy6`.
    pub repeat_a_b: bool,
}

impl Pad {
    /// `_Joypad`.
    pub fn poll(&mut self) {
        let changed = self.last ^ self.input;
        self.released = changed & self.last;
        self.pressed = changed & self.input;
        self.last = self.input;
        self.held = self.input & !self.ignore;
        self.pressed &= !self.ignore;
    }

    /// `JoypadLowSensitivity`, returning `hJoy5`: new presses only, or with `repeat_held` what is
    /// held, once and then every 5 frames after 30.
    pub fn low_sensitivity(&mut self, frame_counter: &mut u8) -> Joypad {
        self.poll();
        let reported = if self.repeat_held { self.held } else { self.pressed };
        if !self.pressed.is_empty() {
            *frame_counter = 30;
            return reported;
        }
        if *frame_counter != 0 {
            return Joypad::empty();
        }
        *frame_counter = 5;
        if self.held.intersects(Joypad::A | Joypad::B) && !self.repeat_a_b { Joypad::empty() } else { reported }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll(pad: &mut Pad, input: Joypad) {
        pad.input = input;
        pad.poll();
    }

    #[test]
    fn a_press_is_an_edge_and_holding_is_not() {
        let mut pad = Pad::default();
        poll(&mut pad, Joypad::A);
        assert_eq!(pad.pressed, Joypad::A);
        poll(&mut pad, Joypad::A | Joypad::UP);
        assert_eq!((pad.pressed, pad.held), (Joypad::UP, Joypad::A | Joypad::UP));
        poll(&mut pad, Joypad::UP);
        assert_eq!((pad.pressed, pad.released), (Joypad::empty(), Joypad::A));
    }

    #[test]
    fn a_press_made_while_nobody_polled_is_still_an_edge() {
        let mut pad = Pad::default();
        pad.input = Joypad::A; // a frame nobody polled
        poll(&mut pad, Joypad::A);
        assert_eq!(pad.pressed, Joypad::A);
    }

    #[test]
    fn a_held_button_repeats_after_thirty_frames_and_then_every_five() {
        let mut pad = Pad { repeat_held: true, input: Joypad::DOWN, ..Pad::default() };
        let mut counter = 0;
        let reports: Vec<bool> = (0..45).map(|_| {
            counter = u8::saturating_sub(counter, 1);
            !pad.low_sensitivity(&mut counter).is_empty()
        }).collect();
        let frames: Vec<usize> = reports.iter().enumerate().filter(|(_, r)| **r).map(|(i, _)| i).collect();
        assert_eq!(frames, [0, 30, 35, 40]);
    }

    #[test]
    fn ignored_buttons_are_neither_held_nor_pressed() {
        let mut pad = Pad { ignore: Joypad::START, ..Pad::default() };
        poll(&mut pad, Joypad::START | Joypad::A);
        assert_eq!((pad.held, pad.pressed), (Joypad::A, Joypad::A));
    }
}
