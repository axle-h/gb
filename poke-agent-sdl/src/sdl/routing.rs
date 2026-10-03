//! Which game the keyboard plays: both at once, or either alone.

use gb::joypad::JoypadButton;
use pokered::input::Joypad;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Routing {
    #[default]
    Both,
    Native,
    Emulated,
}

impl Routing {
    /// In the order the toggle shows them and `Tab` cycles them.
    pub const ALL: [Routing; 3] = [Routing::Both, Routing::Native, Routing::Emulated];

    pub fn next(self) -> Self {
        Self::ALL[(Self::ALL.iter().position(|&r| r == self).unwrap() + 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            Routing::Both => "BOTH",
            Routing::Native => "NATIVE",
            Routing::Emulated => "EMULATED",
        }
    }

    /// The buttons held, split into what the emulated game and the native game each see held.
    pub fn route(self, held: Joypad) -> (Joypad, Joypad) {
        match self {
            Routing::Both => (held, held),
            Routing::Native => (Joypad::empty(), held),
            Routing::Emulated => (held, Joypad::empty()),
        }
    }
}

/// The emulator's button for each of the pad's.
pub const BUTTONS: [(Joypad, JoypadButton); 8] = [
    (Joypad::A, JoypadButton::A),
    (Joypad::B, JoypadButton::B),
    (Joypad::SELECT, JoypadButton::Select),
    (Joypad::START, JoypadButton::Start),
    (Joypad::RIGHT, JoypadButton::Right),
    (Joypad::LEFT, JoypadButton::Left),
    (Joypad::UP, JoypadButton::Up),
    (Joypad::DOWN, JoypadButton::Down),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_mode_sends_the_pad_where_it_says() {
        let held = Joypad::A | Joypad::UP;
        assert_eq!(Routing::Both.route(held), (held, held));
        assert_eq!(Routing::Native.route(held), (Joypad::empty(), held));
        assert_eq!(Routing::Emulated.route(held), (held, Joypad::empty()));
        assert_eq!(Routing::Both.route(Joypad::empty()), (Joypad::empty(), Joypad::empty()));
    }

    #[test]
    fn tab_cycles_through_every_mode_back_to_the_first() {
        let cycle: Vec<Routing> = std::iter::successors(Some(Routing::Both), |r| Some(r.next())).take(4).collect();
        assert_eq!(cycle, [Routing::Both, Routing::Native, Routing::Emulated, Routing::Both]);
    }
}
