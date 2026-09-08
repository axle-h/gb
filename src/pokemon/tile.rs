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
    /// ⭐ **The whole Strength puzzle as one decision: put a boulder on `at`.**
    ///
    /// `at` is a pressure switch (`strength_switches`) or a floor hole (`holes`); `hole` says which,
    /// because the sentence differs and so does the point of doing it.
    ///
    /// ⚠️ **This exists because a Sokoban puzzle solved a shove at a time is the wrong unit of
    /// decision, and the evidence is unusually direct.** Every individual shove is a paid request
    /// and a chance to seal the floor, and the prompt layer twice tried to explain the puzzle in
    /// prose instead and had to withdraw both attempts — one of them told a deployed run the floor
    /// could not be solved the instant it arrived on Victory Road 3F, and the run walked up from 2F
    /// and straight back down **twenty times**. Two more deployed runs filed issue reports asking
    /// whether the switch coordinates were wrong. They were not; the puzzle was simply not a thing
    /// to ask a language model to do one shove at a time.
    ///
    /// `MetaTileMap::solve_boulder_push` is a capped BFS over boulder layouts that the scripted
    /// route has relied on for the whole game, so the planning was already solved and only the
    /// *offer* was missing. It is the same move `Cut` and `Boulder` each made a level lower: a
    /// sequence whose every step has one legal continuation is one decision, not N.
    ///
    /// ⚠️ **Withheld unless it is solvable right now** — `solve_boulder_push` answering `None` is
    /// the gate, exactly as `can_cut` gates a tree. Offering it otherwise would recreate "a row the
    /// agent cannot then execute" on the hardest floor in the game.
    BoulderGoal { boulder: Point8, at: Point8, hole: bool },
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

    /// Whether `other` is **the same row of the menu as this one**, for a walk that re-derives its
    /// target every tick.
    ///
    /// ⚠️ **`==` is wrong here for exactly one variant, and it cost a coverage sweep.**
    /// `OverworldMovement` re-asks `actions()` from wherever the player now stands and looks for the
    /// row it set out for. Every other tile is the thing itself and holds still, but a
    /// `BoulderGoal` also carries the boulder `actions()` picked as nearest-capable — and after a
    /// push, or after the player has walked, that can be a different boulder. Full equality then
    /// finds nothing, the walk has no route, and it is abandoned on `MAX_MOVEMENT_SILENCE` sixty
    /// seconds later: the walk of 2026-09-08 gave up "without getting there" **standing one square
    /// from the push tile**. What the row is about is the target, which is also why the id is keyed
    /// on it (see [`Self::id_kind`]).
    pub fn is_same_row_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::BoulderGoal { at, hole, .. }, Self::BoulderGoal { at: b, hole: h, .. }) =>
                at == b && hole == h,
            _ => self == other,
        }
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
    ///
    /// ⚠️ **For `Sprite` this is the *whole* key past the map prefix** — see
    /// [`OverworldAction::id`](crate::pokemon::actions::OverworldAction::id), which gives a sprite
    /// row no coordinate, because the only coordinate it had was the square beside the object and
    /// that square moves with the player.
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
            // ⚠️ **The target is in the key, not just in the prose.** A floor has several switches
            // and the square the player stands on to start is the solver's choice and moves between
            // turns, so without the target two goals could mint the same id — and the *same* goal
            // could mint two.
            //
            // ⚠️ **The boulder is named in the row's prose and deliberately *not* in its id, and
            // the coordinate is the target rather than the square the player stands on.** Both
            // halves move: every shove changes the layout, so `actions()` re-picks the nearest
            // capable boulder and the solver re-picks the square to start from — and an id built
            // from either is a *new* id after every push. The coverage walk of 2026-09-07 spent two
            // and a half hours at one action a minute on VictoryRoad3F because of it: each push
            // minted an id the frontier had never seen, so one puzzle was an unbounded family of
            // rows that could never be finished, and each new id started the long walk again.
            // The target is the one thing about a goal that does not move, so the target is the key.
            Self::BoulderGoal { hole, .. } =>
                if *hole { "PushBoulderIntoHole".into() } else { "PushBoulderOntoSwitch".into() },
            // ⚠️ **Not `"Cut"`.** An id is a key, and a run resumed across this change reads
            // `Route9:5,9:CutTree` back out of its own conversation and quotes it at
            // `resolve_overworld`. The variant split is an implementation detail; the key is not.
            Self::Cut { .. } => "CutTree".into(),
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
            Self::Cut { at } => write!(f, "the tree at ({}, {}), to cut it down", at.x, at.y),
            // Names the *goal*, because that is the decision being taken; how many shoves it costs
            // and from which side is the solver's business and changes nothing the model can act on.
            // ⚠️ **The boulder is named as well as the target.** A floor with two of each — Seafoam
            // B3F — makes "push a boulder into that hole" ambiguous, and choosing the wrong one
            // leaves the other hole unreachable: a complete search then answers "unsolvable" on a
            // floor that was fine ten pushes earlier.
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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Point8;

    /// **A goal row is identified by its target, and the boulder is free to move under it.**
    ///
    /// ⚠️ Both halves of this cost a coverage sweep. `MetaTile::id_kind` keys the id on the target
    /// because an id that changes per push is a row the frontier has never seen — a 24-hour walk
    /// spent two and a half hours at one action a minute on VictoryRoad3F. `is_same_row_as` is the
    /// same fact one layer down: `OverworldMovement` re-derives its target every tick, and matching
    /// the row with `==` stopped finding it the moment `actions()` re-picked the nearest capable
    /// boulder, so the walk was abandoned sixty seconds later a single square from the push tile.
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

        // Every other tile keeps plain equality, which is what the walk relies on everywhere else.
        assert!(MetaTile::Cut { at: Point8 { x: 5, y: 8 } }
            .is_same_row_as(&MetaTile::Cut { at: Point8 { x: 5, y: 8 } }));
        assert!(!MetaTile::Cut { at: Point8 { x: 5, y: 8 } }
            .is_same_row_as(&MetaTile::Cut { at: Point8 { x: 5, y: 9 } }));
    }
}
