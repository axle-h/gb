//! Game controllers, on SDL's standard mapping, held like the keyboard and merged with it.

use sdl2::GameControllerSubsystem;
use sdl2::controller::{Axis, Button, GameController};
use pokered::input::Joypad;

/// How far the left stick leans, of 32767, before it holds a direction.
const DEAD_ZONE: i16 = 16_384;

/// By label rather than position: SDL names a Nintendo pad's buttons as printed, so its A is the
/// Game Boy's A, and so is the A on any other pad.
const BUTTONS: [(Button, Joypad); 8] = [
    (Button::A, Joypad::A),
    (Button::B, Joypad::B),
    (Button::Start, Joypad::START),
    (Button::Back, Joypad::SELECT),
    (Button::DPadUp, Joypad::UP),
    (Button::DPadDown, Joypad::DOWN),
    (Button::DPadLeft, Joypad::LEFT),
    (Button::DPadRight, Joypad::RIGHT),
];

/// The pad one controller holds, from its buttons and its left stick's (x, y), y growing downwards.
pub fn pad(pressed: impl Fn(Button) -> bool, (x, y): (i16, i16)) -> Joypad {
    let buttons = BUTTONS.into_iter().filter(|&(button, _)| pressed(button)).fold(Joypad::empty(), |held, (_, pad)| held | pad);
    let stick = [
        (x <= -DEAD_ZONE, Joypad::LEFT),
        (x >= DEAD_ZONE, Joypad::RIGHT),
        (y <= -DEAD_ZONE, Joypad::UP),
        (y >= DEAD_ZONE, Joypad::DOWN),
    ].into_iter().filter(|&(leaning, _)| leaning).fold(Joypad::empty(), |held, (_, pad)| held | pad);
    buttons | stick
}

/// Every controller plugged in. SDL announces those present at start as added too.
pub struct Controllers {
    subsystem: GameControllerSubsystem,
    open: Vec<GameController>,
}

impl Controllers {
    pub fn new(subsystem: GameControllerSubsystem) -> Self {
        Self { subsystem, open: Vec::new() }
    }

    /// Opens the controller at joystick `index`, returning its name, or `None` if it is open already.
    pub fn added(&mut self, index: u32) -> Result<Option<String>, String> {
        let controller = self.subsystem.open(index).map_err(|e| e.to_string())?;
        if self.open.iter().any(|open| open.instance_id() == controller.instance_id()) {
            return Ok(None);
        }
        let name = controller.name();
        self.open.push(controller);
        Ok(Some(name))
    }

    /// Closes the controller with joystick `instance_id`, returning its name.
    pub fn removed(&mut self, instance_id: u32) -> Option<String> {
        let at = self.open.iter().position(|open| open.instance_id() == instance_id)?;
        Some(self.open.remove(at).name())
    }

    /// What every controller holds, together.
    pub fn held(&self) -> Joypad {
        self.open.iter().fold(Joypad::empty(), |held, controller| {
            held | pad(|button| controller.button(button), (controller.axis(Axis::LeftX), controller.axis(Axis::LeftY)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_buttons_press_the_pad_by_their_labels() {
        for (button, joypad) in BUTTONS {
            assert_eq!(pad(|b| b == button, (0, 0)), joypad);
        }
        assert_eq!(pad(|b| matches!(b, Button::X | Button::Y | Button::Guide | Button::LeftShoulder), (0, 0)), Joypad::empty());
        assert_eq!(pad(|b| matches!(b, Button::A | Button::DPadUp), (0, 0)), Joypad::A | Joypad::UP);
    }

    #[test]
    fn the_left_stick_holds_a_direction_past_its_dead_zone() {
        let none = |_| false;
        assert_eq!(pad(none, (DEAD_ZONE - 1, -(DEAD_ZONE - 1))), Joypad::empty());
        assert_eq!(pad(none, (-DEAD_ZONE, 0)), Joypad::LEFT);
        assert_eq!(pad(none, (i16::MAX, 0)), Joypad::RIGHT);
        assert_eq!(pad(none, (0, i16::MIN)), Joypad::UP);
        assert_eq!(pad(none, (i16::MAX, i16::MAX)), Joypad::RIGHT | Joypad::DOWN);
        assert_eq!(pad(|b| b == Button::DPadLeft, (i16::MAX, 0)), Joypad::LEFT | Joypad::RIGHT, "either source presses");
    }
}
