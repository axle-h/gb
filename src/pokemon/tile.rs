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
    ///
    /// ⚠️ **As an *action* it is the whole cut, not the walk up to it.** The row's route ends
    /// facing the tree and `AgentState::OverworldMovement`'s empty-route arm hands off to
    /// `AgentState::CuttingTree`, the same seam [`Self::Fish`] uses. It used to end there and
    /// leave the model to call `use_field_move` with `cut` as a second turn, which is a paid
    /// request for a step with exactly one legal continuation.
    CutTree,
    /// **The whole cut of the tree at `at`**: walk to a square beside it, face it, use Cut.
    ///
    /// ⚠️ **A separate variant from [`Self::CutTree`], which is the *terrain*.** They used to be
    /// one, and the row was therefore about "a tree" rather than about a tree: `actions()` emits one
    /// per reachable tree, `AgentState::OverworldMovement` re-derives its target every tick by
    /// `a.tile == destination`, and with every row carrying the same tile that match found whichever
    /// sorted first — so a row that named its tree could be walked to a different one. It also meant
    /// the row could not say which tree it was about at all, which is the thing the model is
    /// choosing between. Carrying `at` fixes both, and makes the walk stable when the nearest square
    /// beside a tree changes as the player approaches it.
    ///
    /// ⚠️ **[`Self::id_kind`] still says `CutTree`.** The id is a key the model quotes back out of
    /// its own history, and a resumed run's history is full of `Route9:5,9:CutTree`.
    Cut { at: Point8 },
    /// **One shove of the boulder at `at`, one tile in `push`.** Synthesised by `actions()` like
    /// [`Self::Fish`] rather than classified from the tileset: the tile the row's coordinate names
    /// is ordinary floor, and what makes it a row is the conjunction of a boulder beside it, a push
    /// the cartridge would not refuse ([`MetaTileMap::boulder_push_refusal`]) and a party that can
    /// use Strength ([`MetaTileMap::can_strength`]).
    ///
    /// ⚠️ **Strength is armed by the row rather than asked for.** `AgentState::PushingBoulder`
    /// opens the party menu itself when `BIT_STRENGTH_ACTIVE` is clear, so there is no separate
    /// "arm Strength" decision to get wrong — the flag is reset on every map change, and a run that
    /// has to remember that spends a request per floor on it and stalls silently when it forgets.
    ///
    /// ⚠️ **`at` is carried even though it is derivable** (`at` = the row's tile stepped one square
    /// in `push`). The prose has to name the boulder's own square, and an action that computes the
    /// thing it is about from the square the player stands on is one more place for the two
    /// coordinate conventions to be confused.
    Boulder { at: Point8, push: crate::joypad::JoypadButton },
    /// A PC (a hidden-object tile the player faces and presses A to use — Someone's PC / Bill's PC).
    /// Impassable like `Obstacle`, but `actions()` emits a route that faces it and presses A. The
    /// tile is not classified from the tileset; PC coordinates are looked up per map (`pc_locations`).
    Pc,
    /// A hidden object the player faces and presses A on: a gym trash can, a vending machine, a
    /// poster switch, a Pokémon Mansion statue.
    ///
    /// ⚠️ **The same mechanism as [`Self::Pc`] and kept a separate variant on purpose.** Both are
    /// `hidden_event`s dispatched by `CheckForHiddenEvent`, and both are looked up per map rather
    /// than classified from the tileset — but a PC opens a menu with drivers of its own behind it
    /// (`postgame::{pc_box, item_storage}`) and one of these is a single press and nothing more.
    /// Folding them together would put every box operation behind a row whose whole contract is
    /// "press A once".
    /// ⚠️ **`ordinal` is 1-based and is in the id, because the approach tile is not unique.** An id
    /// is `{map}:{x},{y}:{kind}` where the coordinate is the tile the player *stands on*, and two
    /// hidden objects a tile apart share one: Vermilion Gym's bins at (9, 7) and (9, 9) are both
    /// approached from (9, 8), so both rows minted `VermilionGym:9,8:TrashCan` and
    /// `resolve_overworld`, which matches by string equality, could only ever reach the first. One
    /// of the fifteen was silently unreachable. This is the same fix `MapSprite` already carries —
    /// `Rocket1`, `Rocket2` — and it numbers within the map's own table, so a bin's ordinal is the
    /// `wGymTrashCanIndex` the puzzle uses plus one.
    Switch { object: HiddenObject, ordinal: u8 },
    /// Tall-grass tile (tile ID matches `wGrassTile` for the current tileset).
    /// Walkable; stepping on it can trigger a wild Pokémon encounter.
    Grass,
    /// A **shore tile to fish from**: the player stands here, faces the water in front, and casts.
    ///
    /// ⚠️ **Synthesised like [`Self::Pc`] and [`Self::Switch`], not classified from the tileset.**
    /// The tile itself is ordinary `Empty` ground; what makes it a row in the action menu is the
    /// conjunction of three things `actions()` checks — water it can face, a tileset the ROM lists in
    /// `WaterTilesets`, and a rod in the bag. So this variant is only ever an *action*, never
    /// something `meta_tiles` holds, and `player_tile()` never equals it.
    ///
    /// ⚠️ **It carries the rod so the menu row can name it**, and the rod is always the best one in
    /// the bag: the earlier two are strictly worse rather than differently useful (see
    /// `fishing::Rod::best_in_bag`), so there is nothing here for a policy to choose between.
    Fish { rod: crate::pokemon::postgame::fishing::Rod },
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
            // Same argument as `Sprite`, one step down: "Switch" is the mechanism's name and says
            // nothing about what is being pressed. Four statues on one Mansion floor are already
            // told apart by their coordinates, so this is about the row reading as English rather
            // than about uniqueness.
            Self::Switch { object, ordinal } => format!("{}{ordinal}", <&'static str>::from(object)).into(),
            // The direction is part of the key rather than of the prose: one boulder is up to four
            // different decisions and they share the square the player stands on for none of them,
            // but `stand + push` is what names the boulder, so without the word two rows for two
            // boulders either side of one tile would mint the same id.
            Self::Boulder { push, .. } => format!("PushBoulder{push}").into(),
            // ⚠️ **Not `"Cut"`.** An id is a key, and a run resumed across this change reads
            // `Route9:5,9:CutTree` back out of its own conversation and quotes it at
            // `resolve_overworld`. The variant split is an implementation detail; the key is not.
            Self::Cut { .. } => "CutTree".into(),
            other => other.kind().into(),
        }
    }
}

/// A push direction as the word a person would use, lower case, for the prose above.
///
/// ⚠️ **Not `JoypadButton`'s own `Display`**, which is the variant name and therefore capitalised:
/// this reads inside a sentence ("push it left"), and the capitalised form is what the *id* uses,
/// where it is a key rather than English. `tile_map::push_word` is the same three lines for the
/// refusal sentences; they are separate because one is prose about a button and the other is prose
/// about a tile, and folding them would put a `pub` on a formatting detail.
fn push_word(push: crate::joypad::JoypadButton) -> &'static str {
    use crate::joypad::JoypadButton;
    match push {
        JoypadButton::Up => "up", JoypadButton::Down => "down",
        JoypadButton::Left => "left", JoypadButton::Right => "right",
        _ => "that way",
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
            Self::Cut { at } => write!(f, "the tree at ({}, {}), to cut it down", at.x, at.y),
            Self::Boulder { at, push } => write!(f, "the boulder at ({}, {}), to push it {}", at.x, at.y, push_word(*push)),
            Self::Pc => write!(f, "the PC"),
            Self::Fish { rod } => write!(f, "the water's edge, to fish with the {}", rod.name()),
            Self::Switch { object, .. } => write!(f, "{object}"),
            Self::Grass => write!(f, "tall grass"),
        }
    }
}

/// What a [`MetaTile::Switch`] actually is. One press of A on each, and what it does is the
/// cartridge's business.
///
/// ⚠️ **These are the ones a playthrough cannot avoid**, which is the whole reason the table exists;
/// see [`hidden_objects_for`](crate::pokemon::tile_map::hidden_objects_for). Slot machines, signs
/// and hidden items are `hidden_event`s too and are deliberately not here: a sign is text the model
/// is already shown, and a hidden item has nothing to point the player at.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, strum_macros::IntoStaticStr)]
pub enum HiddenObject {
    /// One of Vermilion Gym's fifteen bins. Two of them hold the switches that open Lt. Surge's door.
    TrashCan,
    /// A Celadon Mart roof drink machine. The menu opens with the cheapest drink under the cursor,
    /// so one press buys a Fresh Water, which is what the Saffron gate guards want.
    VendingMachine,
    /// The Game Corner poster that opens the Rocket Hideout.
    Poster,
    /// A Pokémon Mansion statue. They toggle one shared switch, so pressing a second undoes the
    /// first.
    Statue,
    /// Bill's cell separator: the PC in his house, pressed once to turn him back into a person.
    ///
    /// ⚠️ **The same tile `pc_locations_for(BillsHouse)` names, deliberately counted twice.** As a
    /// PC it is storage the scripted policies drive through `FieldMove::UsePcBox`; as this it is a
    /// one-press story beat, and it is the *only* way to the S.S. Ticket, so to HM01 Cut, so to the
    /// rest of the game. It is offered only while pressing it would do that — see
    /// [`MetaTileMap::bill_cell_separator`](crate::pokemon::tile_map::MetaTileMap::bill_cell_separator).
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