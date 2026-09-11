use std::fmt::Display;
use poke_core::geometry::Point8;
use crate::pokemon::map::Map;

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, strum_macros::IntoStaticStr, Default)]
pub enum MetaTile {
    #[default]
    Empty,
    Obstacle,
    Water,
    Jump(JumpDirection),
    Sprite(&'static str),
    Warp { to_map: Map, to_position: Point8 },
    Connection { to_map: Map, to_position: Point8 },
    /// A connection reachable only while surfing.
    ConnectionWater(Map),
    /// Counter / desk tile listed in `wTilesetTalkingOverTiles`.
    Counter,
    CutTree,
    /// The whole cut of the tree at `at`: walk beside it, face it, use Cut.
    Cut { at: Point8 },
    /// Push `boulder` until it lands on the switch or in the hole at `at`.
    BoulderGoal { boulder: Point8, at: Point8, hole: bool },
    Pc,
    /// A hidden object the player faces and presses A on.
    Switch { object: HiddenObject, ordinal: u8 },
    Grass,
    /// A shore tile to fish from, facing the water.
    Fish { rod: crate::pokemon::postgame::fishing::Rod },
    /// Floor or water to walk up and down until something attacks, where there is no grass.
    Pace { water: bool },
}

impl MetaTile {
    /// The variant's name, the stable half of the pair this type formats itself as.
    pub fn kind(&self) -> &'static str {
        self.into()
    }

    /// Whether `other` is the same menu row, for a walk that re-derives its target every tick.
    pub fn is_same_row_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::BoulderGoal { at, hole, .. }, Self::BoulderGoal { at: b, hole: h, .. }) =>
                at == b && hole == h,
            _ => self == other,
        }
    }

    /// The last field of an action id: [`Self::kind`], except for a person or object, named.
    pub fn id_kind(&self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::Sprite(name) if name.contains(' ') => name.replace(' ', "").into(),
            Self::Sprite(name) => (*name).into(),
            Self::Switch { object: HiddenObject::Quiz { yes }, ordinal } =>
                format!("Quiz{}{ordinal}", if *yes { "Yes" } else { "No" }).into(),
            Self::Switch { object, ordinal } => format!("{}{ordinal}", <&'static str>::from(object)).into(),
            // The id names only the target, because the boulder moves on every shove.
            Self::BoulderGoal { hole, .. } =>
                if *hole { "PushBoulderIntoHole".into() } else { "PushBoulderOntoSwitch".into() },
            Self::Cut { .. } => "CutTree".into(),
            Self::Pace { water: true } => "PaceOnWater".into(),
            other => other.kind().into(),
        }
    }
}

impl Display for MetaTile {
    /// Prose and a UI contract: the status log and `observe::map_view` show it verbatim.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "an open tile"),
            Self::Obstacle => write!(f, "an obstacle"),
            Self::Water => write!(f, "water"),
            Self::Jump(direction) => write!(f, "a ledge going {}", direction.compass()),
            Self::Sprite(name) => write!(f, "{name}"),
            Self::Warp { to_map, .. } => write!(f, "the warp to {to_map}"),
            Self::Connection { to_map, .. } => write!(f, "the way into {to_map}"),
            Self::ConnectionWater(to_map) => write!(f, "the water crossing into {to_map}"),
            Self::Counter => write!(f, "a counter"),
            Self::CutTree => write!(f, "a cuttable tree"),
            Self::Cut { at } => write!(f, "the tree at ({}, {}), to cut it down", at.x, at.y),
            // Names the goal; how many shoves it costs changes nothing the model can act on.
            Self::BoulderGoal { boulder, at, hole: false } => write!(
                f, "the boulder at ({}, {}), to push it onto the switch at ({}, {})",
                boulder.x, boulder.y, at.x, at.y),
            Self::BoulderGoal { boulder, at, hole: true } => write!(
                f, "the boulder at ({}, {}), to push it into the hole at ({}, {})",
                boulder.x, boulder.y, at.x, at.y),
            Self::Pc => write!(f, "the PC"),
            Self::Fish { rod } => write!(f, "the water's edge, to fish with the {}", rod.name()),
            Self::Switch { object, .. } => write!(f, "{object}"),
            Self::Grass => write!(f, "tall grass"),
            Self::Pace { water: false } => write!(f, "the floor, to walk it for wild Pokémon"),
            Self::Pace { water: true } => write!(f, "the water, to surf it for wild Pokémon"),
        }
    }
}

/// What a [`MetaTile::Switch`] actually is.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, strum_macros::IntoStaticStr)]
pub enum HiddenObject {
    TrashCan,
    VendingMachine,
    Poster,
    Statue,
    /// Bill's cell separator: the PC in his house, pressed once to turn him back into a person.
    CellSeparator,
    /// A Cinnabar Gym quiz machine, answered YES or NO: two rows, since the answer is the choice.
    Quiz { yes: bool },
}

impl Display for HiddenObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TrashCan => write!(f, "a trash can"),
            Self::VendingMachine => write!(f, "a vending machine"),
            Self::Poster => write!(f, "the poster"),
            Self::Statue => write!(f, "a statue"),
            Self::CellSeparator => write!(f, "the cell separator"),
            Self::Quiz { .. } => write!(f, "a quiz machine"),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct WarpEvent {
    pub position: Point8,
    pub destination_map: Map,
    pub destination_position: Point8,
}

impl WarpEvent {
    pub fn tile(&self) -> MetaTile {
        MetaTile::Warp {
            to_map: self.destination_map,
            to_position: self.destination_position,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, strum_macros::Display)]
pub enum JumpDirection {
    South,
    West,
    East,
}

impl JumpDirection {
    /// Lower-cased, for the middle of a sentence.
    pub fn compass(&self) -> &'static str {
        match self {
            Self::South => "south",
            Self::West => "west",
            Self::East => "east",
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use poke_core::geometry::Point8;

    /// A goal row is identified by its target, and the boulder is free to move under it.
    #[test]
    fn a_boulder_goal_is_the_target_and_not_the_boulder() {
        let target = Point8 { x: 3, y: 5 };
        let goal = |bx, by| MetaTile::BoulderGoal {
            boulder: Point8 { x: bx, y: by }, at: target, hole: false };

        assert!(goal(2, 3).is_same_row_as(&goal(13, 12)),
            "the same switch is the same row whichever boulder is going to reach it");
        assert_eq!(goal(2, 3).id_kind(), goal(13, 12).id_kind(), "and so is its id");

        // A different target is a different row, and a hole is not a switch.
        let elsewhere = MetaTile::BoulderGoal {
            boulder: Point8 { x: 2, y: 3 }, at: Point8 { x: 9, y: 16 }, hole: false };
        assert!(!goal(2, 3).is_same_row_as(&elsewhere));
        let hole = MetaTile::BoulderGoal { boulder: Point8 { x: 2, y: 3 }, at: target, hole: true };
        assert!(!goal(2, 3).is_same_row_as(&hole), "a hole at the same square is not the switch");
        assert_ne!(goal(2, 3).id_kind(), hole.id_kind());

        // Every other tile keeps plain equality.
        assert!(MetaTile::Cut { at: Point8 { x: 5, y: 8 } }
            .is_same_row_as(&MetaTile::Cut { at: Point8 { x: 5, y: 8 } }));
        assert!(!MetaTile::Cut { at: Point8 { x: 5, y: 8 } }
            .is_same_row_as(&MetaTile::Cut { at: Point8 { x: 5, y: 9 } }));
    }
}
