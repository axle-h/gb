use poke_core::map::Map;
use poke_core::map_objects::initial_toggleable_object_flags;
use poke_core::sprite::SpriteFacing;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;

/// Where the player is, and what of the map a save carries and a battle on top leaves alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    /// `wCurMap`.
    pub map: Map,
    /// `wXCoord` and `wYCoord`, in squares.
    pub x: u8,
    pub y: u8,
    /// `wSpritePlayerStateData1FacingDirection`.
    pub facing: SpriteFacing,
    /// `wLastMap`: where a `LAST_MAP` warp leads.
    pub last_map: Map,
    /// `wWalkBikeSurfState`: [`WALKING`], [`BIKING`] or [`SURFING`].
    pub walk_bike_surf: u8,
    /// `BIT_ALWAYS_ON_BIKE`: Cycling Road, where the bike cannot be put away.
    #[serde(default)]
    pub always_on_bike: bool,
    /// What `.displayDialogue` last found in front of the player.
    #[serde(default)]
    pub ahead: Ahead,
    /// `wToggleableObjectFlags`: a set bit hides that toggleable object.
    pub hidden_objects: Vec<u8>,
    /// `wTownVisitedFlag`, a bit per town.
    pub towns_visited: u16,
    /// `wLastBlackoutMap`: the town a blackout returns to, which a Pokémon Center sets.
    #[serde(default)]
    pub last_blackout_map: Map,
    /// `wRepelRemainingSteps`.
    #[serde(default)]
    pub repel_steps: u8,
    /// `wStatusFlags1`'s `BIT_STRENGTH_ACTIVE`: STRENGTH has been used, so a boulder in front of the
    /// player moves. `EnterMap` clears it on any map not re-entered after a battle.
    #[serde(default)]
    pub strength_active: bool,
    /// What the party menu's field move leaves for the overworld to finish once the start menu has
    /// closed. STRENGTH is not here: its whole effect is `strength_active`, set where it is chosen.
    #[serde(default)]
    pub used_field_move: Option<UsedFieldMove>,
    /// `BIT_FLY_WARP` with `wDestinationMap`: the town the town map was answered with, which the
    /// overworld's next pass warps to.
    #[serde(default)]
    pub fly_warp: Option<Map>,
    /// `BIT_ESCAPE_WARP`: the escape rope, DIG and TELEPORT, which spin the player out to
    /// `last_blackout_map` rather than flying them to the town map's answer.
    #[serde(default)]
    pub escape_warp: bool,
}

/// A field move used from the party menu whose work is the overworld's, since the map view and the
/// palette it touches are the overworld's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UsedFieldMove {
    /// `UsedCut` with `wCutTile`: the text, the block swap and the animation over the map.
    Cut(u8),
    /// `.flash`: `wMapPalOffset` back to zero, which lights a dark map.
    Flash,
}

/// `NewGameWarp`: Red's room, by the stairs.
impl Default for Location {
    fn default() -> Self {
        Self {
            map: Map::RedsHouse2F,
            x: 3,
            y: 6,
            facing: SpriteFacing::Down,
            last_map: Map::PalletTown,
            walk_bike_surf: WALKING,
            always_on_bike: false,
            ahead: Ahead::default(),
            hidden_objects: initial_toggleable_object_flags(),
            towns_visited: 0,
            last_blackout_map: Map::PalletTown,
            repel_steps: 0,
            strength_active: false,
            used_field_move: None,
            fly_warp: None,
            escape_warp: false,
        }
    }
}

pub const WALKING: u8 = 0;
pub const BIKING: u8 = 1;
pub const SURFING: u8 = 2;

/// `.displayDialogue` reads the tiles before the start menu goes up, and nothing moves while it is
/// up, so what the bag and the field moves test is what was there then.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ahead {
    /// `wTileInFrontOfPlayer`.
    pub tile: u8,
    /// `wTilePlayerStandingOn`.
    pub standing_on: u8,
    /// `IsSpriteInFrontOfPlayer2` at the talking range, with the start menu's box up.
    pub sprite: bool,
}

impl Location {
    pub fn is_hidden(&self, global_index: u8) -> bool {
        self.hidden_objects.get(global_index as usize / 8).is_some_and(|byte| byte & 1 << (global_index % 8) != 0)
    }
}

/// `PLAYER_DIR_*`: the bits `wPlayerDirection` and `wPlayerMovingDirection` hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Direction {
    Right = 1,
    Left = 2,
    Down = 4,
    Up = 8,
}

impl Direction {
    pub const ALL: [Direction; 4] = [Direction::Down, Direction::Up, Direction::Left, Direction::Right];

    pub fn from_bits(bits: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|&direction| direction as u8 == bits)
    }

    pub fn button(self) -> Joypad {
        match self {
            Direction::Right => Joypad::RIGHT,
            Direction::Left => Joypad::LEFT,
            Direction::Down => Joypad::DOWN,
            Direction::Up => Joypad::UP,
        }
    }

    pub fn facing(self) -> SpriteFacing {
        match self {
            Direction::Right => SpriteFacing::Right,
            Direction::Left => SpriteFacing::Left,
            Direction::Down => SpriteFacing::Down,
            Direction::Up => SpriteFacing::Up,
        }
    }

    pub fn of_facing(facing: SpriteFacing) -> Self {
        match facing {
            SpriteFacing::Right => Direction::Right,
            SpriteFacing::Left => Direction::Left,
            SpriteFacing::Down => Direction::Down,
            SpriteFacing::Up => Direction::Up,
        }
    }

    /// `(x, y)` a step moves by.
    pub fn delta(self) -> (i8, i8) {
        match self {
            Direction::Right => (1, 0),
            Direction::Left => (-1, 0),
            Direction::Down => (0, 1),
            Direction::Up => (0, -1),
        }
    }
}
