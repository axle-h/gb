use std::fmt::Display;
use gb::geometry::Point8;
use crate::pokemon::map::Map;

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, strum_macros::IntoStaticStr, Default)]
pub enum MetaTile {
    #[default]
    Empty,
    Obstacle,
    /// Water tile (tile ID 0x14) — can only be crossed while surfing.
    Water,
    /// Ledge tile — can only be crossed by jumping in the specified direction.
    Jump(JumpDirection),
    Sprite(&'static str),
    Warp { to_map: Map, to_position: Point8 },
    /// Walkable entry point into an adjacent map.
    Connection { to_map: Map, to_position: Point8 },
    /// Water entry point into an adjacent map — only reachable while surfing.
    ConnectionWater(Map),
    /// Counter / desk tile listed in `wTilesetTalkingOverTiles`.
    Counter,
    /// A shrub that blocks passage until the player uses HM Cut.
    CutTree,
    /// The whole cut of the tree at `at`: walk to a square beside it, face it, use Cut.
    Cut { at: Point8 },
    /// One shove of the boulder at `at`, one tile in `push`.
    BoulderGoal { boulder: Point8, at: Point8, hole: bool },
    /// A PC (a hidden-object tile the player faces and presses A to use — Someone's PC / Bill's
    /// PC).
    Pc,
    /// A hidden object the player faces and presses A on: a gym trash can, a vending machine, a
    /// poster switch, a Pokémon Mansion statue.
    Switch { object: HiddenObject, ordinal: u8 },
    /// Tall-grass tile (tile ID matches `wGrassTile` for the current tileset).
    Grass,
    /// A shore tile to fish from: the player stands here, faces the water in front, and casts.
    Fish { rod: crate::pokemon::postgame::fishing::Rod },
}

impl MetaTile {
    /// The variant's own name — `"Warp"`, `"Connection"`, `"Sprite"` — and the stable half of the
    /// pair this type formats itself as.
    pub fn kind(&self) -> &'static str {
        self.into()
    }

    /// Whether `other` is the same row of the menu as this one, for a walk that re-derives its
    /// target every tick.
    pub fn is_same_row_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::BoulderGoal { at, hole, .. }, Self::BoulderGoal { at: b, hole: h, .. }) =>
                at == b && hole == h,
            _ => self == other,
        }
    }

    /// The last field of an action id: [`Self::kind`] for everything except a person, who is
    /// named.
    pub fn id_kind(&self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::Sprite(name) if name.contains(' ') => name.replace(' ', "").into(),
            Self::Sprite(name) => (*name).into(),
            // Same argument as `Sprite`, one step down: "Switch" is the mechanism's name and says
            // nothing about what is being pressed.
            Self::Switch { object, ordinal } => format!("{}{ordinal}", <&'static str>::from(object)).into(),
            // The direction is part of the key rather than of the prose: one boulder is up to
            // four different decisions and they share the square the player stands on for none of
            // them, but `stand + push` is what names the boulder, so without the word two rows
            // for two boulders either side of one tile would mint the same id.
            Self::BoulderGoal { hole, .. } =>
                if *hole { "PushBoulderIntoHole".into() } else { "PushBoulderOntoSwitch".into() },
            // Not `"Cut"`.
            Self::Cut { .. } => "CutTree".into(),
            other => other.kind().into(),
        }
    }
}

impl Display for MetaTile {
    /// Prose, and a UI contract — this is what the status log says the agent is walking to, via
    /// `AgentEvent::{StartedOverworldAction, OverworldActionCompleted, OverworldActionAborted}`,
    /// and what `observe::map_view` calls each action for the model.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "an open tile"),
            Self::Obstacle => write!(f, "an obstacle"),
            Self::Water => write!(f, "water"),
            Self::Jump(direction) => write!(f, "a ledge going {}", direction.compass()),
            // Already a person's name ("Mom", "Gym Guide"), so it stands alone: "→ heading for
            // Mom".
            Self::Sprite(name) => write!(f, "{name}"),
            Self::Warp { to_map, .. } => write!(f, "the warp to {to_map}"),
            Self::Connection { to_map, .. } => write!(f, "the way into {to_map}"),
            Self::ConnectionWater(to_map) => write!(f, "the water crossing into {to_map}"),
            Self::Counter => write!(f, "a counter"),
            Self::CutTree => write!(f, "a cuttable tree"),
            Self::Cut { at } => write!(f, "the tree at ({}, {}), to cut it down", at.x, at.y),
            // Names the *goal*, because that is the decision being taken; how many shoves it
            // costs and from which side is the solver's business and changes nothing the model
            // can act on.
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
        }
    }
}

/// What a [`MetaTile::Switch`] actually is.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, strum_macros::IntoStaticStr)]
pub enum HiddenObject {
    /// One of Vermilion Gym's fifteen bins.
    TrashCan,
    /// A Celadon Mart roof drink machine.
    VendingMachine,
    /// The Game Corner poster that opens the Rocket Hideout.
    Poster,
    /// A Pokémon Mansion statue.
    Statue,
    /// Bill's cell separator: the PC in his house, pressed once to turn him back into a person.
    CellSeparator,
}

impl Display for HiddenObject {
    /// Prose, and a noun phrase, for the same four frames [`MetaTile`]'s own `Display` serves.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TrashCan => write!(f, "a trash can"),
            Self::VendingMachine => write!(f, "a vending machine"),
            Self::Poster => write!(f, "the poster"),
            Self::Statue => write!(f, "a statue"),
            Self::CellSeparator => write!(f, "the cell separator"),
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

/// The direction a ledge can be jumped over.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, strum_macros::Display)]
pub enum JumpDirection {
    South,
    West,
    East,
}

impl JumpDirection {
    /// Lower-cased, for the middle of a sentence — `Display` is the capitalised variant name and
    /// reads as a shout inside "a ledge going south".
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
    use gb::geometry::Point8;

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

        // Every other tile keeps plain equality, which is what the walk relies on everywhere
        // else.
        assert!(MetaTile::Cut { at: Point8 { x: 5, y: 8 } }
            .is_same_row_as(&MetaTile::Cut { at: Point8 { x: 5, y: 8 } }));
        assert!(!MetaTile::Cut { at: Point8 { x: 5, y: 8 } }
            .is_same_row_as(&MetaTile::Cut { at: Point8 { x: 5, y: 9 } }));
    }
}
