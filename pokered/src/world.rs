use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct World {
    pub options: Options,
    /// Charmap bytes, unterminated.
    pub player_name: Vec<u8>,
    pub rival_name: Vec<u8>,
    /// `BIT_NO_TEXT_DELAY`.
    pub no_text_delay: bool,
}

/// `wOptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    pub text_speed: TextSpeed,
    pub battle_animation: bool,
    pub battle_style: BattleStyle,
}

/// `InitOptions`: medium text, animations on, shift.
impl Default for Options {
    fn default() -> Self {
        Self { text_speed: TextSpeed::Medium, battle_animation: true, battle_style: BattleStyle::Shift }
    }
}

/// Frames per printed character: `TEXT_DELAY_FAST`, `_MEDIUM` and `_SLOW`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextSpeed {
    Fast = 1,
    #[default]
    Medium = 3,
    Slow = 5,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BattleStyle {
    #[default]
    Shift,
    Set,
}
