use std::fmt::Display;
use crate::geometry::Point8;
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
    /// The player cannot walk on it, but can interact with a sprite one tile behind it
    /// by facing the counter and pressing A — pokered's "talking over" mechanic.
    Counter,
    /// A shrub that blocks passage until the player uses HM Cut.
    /// Treated as impassable until `can_use_cut` is true.
    CutTree,
    /// A PC (a hidden-object tile the player faces and presses A to use — Someone's PC / Bill's PC).
    /// Impassable like `Obstacle`, but `actions()` emits a route that faces it and presses A. The
    /// tile is not classified from the tileset; PC coordinates are looked up per map (`pc_locations`).
    Pc,
    /// Tall-grass tile (tile ID matches `wGrassTile` for the current tileset).
    /// Walkable; stepping on it can trigger a wild Pokémon encounter.
    Grass,
}

impl MetaTile {
    /// The variant's own name — `"Warp"`, `"Connection"`, `"Sprite"` — and the **stable** half of
    /// the pair this type formats itself as.
    ///
    /// ⚠️ **This is what an action id is minted from**, not [`Display`]: `llm::tools::overworld_id`
    /// builds `"PalletTown:5,6:Warp"`, the model quotes it back, and it is re-resolved against a
    /// freshly recomputed action list by string equality. `Display` is prose written for a person
    /// and is free to change wording; an id is a key and is not, so the two must not be the same
    /// function. (`Conversation.tsx` shows those ids verbatim, which is the other reason.)
    ///
    /// ⚠️ **[`Self::id_kind`] is what an id actually ends in**, and it differs for exactly one
    /// variant — see there.
    pub fn kind(&self) -> &'static str {
        self.into()
    }

    /// The last field of an action id: [`Self::kind`] for everything except a person, who is named.
    ///
    /// ⚠️ **`Sprite` is the one variant whose *kind* is not worth saying.** Every other id ends in a
    /// word that tells the model what it is choosing — `:Warp`, `:Grass`, `:CutTree` — but "sprite"
    /// is the emulator's vocabulary for a moving object on a screen, and the model does not have a
    /// screen. It reads as jargon, it is the same word for Professor Oak and for a boulder, and the
    /// row beside it then had to spend the name a second time to say who was actually there. So the
    /// id carries the name — `OaksLab:2,2:ProfessorOak` — and the row carries only the distance.
    ///
    /// ⚠️ **Spaces are stripped, and that is not cosmetic.** The names come from
    /// [`MapSprite`](crate::pokemon::map::MapSprite) and several have them ("Middle Aged Woman"), so
    /// an id built straight off one would be whitespace-sensitive under a string-equality resolve —
    /// a model that re-spaced or collapsed it would silently miss. `Display` keeps the spaces,
    /// because that half is prose.
    pub fn id_kind(&self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::Sprite(name) if name.contains(' ') => name.replace(' ', "").into(),
            Self::Sprite(name) => (*name).into(),
            other => other.kind().into(),
        }
    }
}

impl Display for MetaTile {
    /// **Prose, and a UI contract** — this is what the status log says the agent is walking to, via
    /// `AgentEvent::{StartedOverworldAction, OverworldActionCompleted, OverworldActionAborted}`, and
    /// what `observe::map_view` calls each action for the model.
    ///
    /// ⚠️ **A destination that does not name its target is not worth reporting.** This used to be
    /// `strum`'s derive, so a random run's whole log was `→ heading for Warp` / `✓ reached Warp` /
    /// `→ heading for Sprite` — the three most common lines on the page, and none of them said which
    /// warp, which map or which person. Every variant that has a target names it here. A warp and a
    /// connection are named by the map they come out on rather than by the landing coordinates that
    /// go with it: the tile a viewer cares about is the one the run ends up standing on, and the
    /// exact square is the agent's business (`OverworldAction`'s own `Display`, which is the menu
    /// the model chooses from, still carries it).
    ///
    /// Each renders as a noun phrase, because the three sentences above and
    /// `OverworldActionAbortedReason::NoRoute`'s "there is no route to {tile}" all have to read as
    /// English with it substituted in.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "an open tile"),
            Self::Obstacle => write!(f, "an obstacle"),
            Self::Water => write!(f, "water"),
            Self::Jump(direction) => write!(f, "a ledge going {}", direction.compass()),
            // Already a person's name ("Mom", "Gym Guide"), so it stands alone: "→ heading for Mom".
            Self::Sprite(name) => write!(f, "{name}"),
            Self::Warp { to_map, .. } => write!(f, "the warp to {to_map}"),
            Self::Connection { to_map, .. } => write!(f, "the way into {to_map}"),
            Self::ConnectionWater(to_map) => write!(f, "the water crossing into {to_map}"),
            Self::Counter => write!(f, "a counter"),
            Self::CutTree => write!(f, "a cuttable tree"),
            Self::Pc => write!(f, "the PC"),
            Self::Grass => write!(f, "tall grass"),
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