//! Workstream **C — Fishing**. See `docs/postgame-coverage-plan.md` §6-C.
//!
//! Opens the whole water encounter table — Magikarp, Goldeen, Poliwag, Tentacool, Krabby, Horsea,
//! Staryu.
//!
//! Sub-steps: C1 Old Rod · C2 the fishing driver (face water, use the rod from the bag, handle the
//! "not even a nibble" / "hooked" text, drop into the normal battle handler on a bite) · C3 catch
//! from a bite · C4 Good Rod · C5 Super Rod, then `postgame-fishing.bin`.
//!
//! # What a cast is
//!
//! `ItemUseOldRod` / `ItemUseGoodRod` / `ItemUseSuperRod` (`engine/items/item_effects.asm:1826-1885`)
//! all share `FishingInit`, which refuses unless the tile **in front of the player** is water (raw
//! `$14`) or one of the two eastern-shore tiles, the map's tileset is in `WaterTilesets`, and the
//! player is not surfing. Each then writes `wRodResponse` — `0` no bite, `1` a bite (with
//! `wCurOpponent`/`wCurEnemyLevel` already filled in), `2` "no fish on this map" (Super Rod only) —
//! and runs `FishingAnim`, which sets `BIT_LEDGE_OR_FISHING` for its duration.
//!
//! So one *cast* is: face water → START → ITEM → the rod → USE → wait out the animation. On a bite the
//! overworld loop starts a wild battle, and [`crate::pokemon::agent::PokemonAgent`]'s
//! `assert_battle_state` takes the driver's state away before it can see the outcome — which is why
//! this driver only ever performs **one** cast and returns to `Idle`, and the *policy* owns the
//! repetition (see [`FishGoal`]).

use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::mmu::MMU;
use crate::pokemon::agent::{start_menu_row, AgentEvent, AgentState, PokemonAgent, StartMenuRow};
use crate::pokemon::menu::START_MENU_ORIGIN;
use crate::pokemon::battle::{BattleAction, BattleType};
use crate::pokemon::encoding::GameMode;
use crate::pokemon::item::ItemId;
use crate::pokemon::map::Map;
use crate::pokemon::policy::{FieldMove, PolicyStep};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::tile::MetaTile;
use crate::pokemon::tile_map::MetaTileMap;
use crate::pokemon::{GameState, PokemonApi, PokemonApiTrait};

/// The three rods, in the order they are obtained — which is also **worst to best**, so `Ord` is the
/// ranking [`Rod::best_in_bag`] takes the maximum of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rod { Old, Good, Super }

impl Rod {
    /// The bag item to use from the ITEM menu.
    pub fn item(self) -> ItemId {
        match self { Rod::Old => ItemId::OldRod, Rod::Good => ItemId::GoodRod, Rod::Super => ItemId::SuperRod }
    }

    /// What to call it in prose the model and the page read.
    pub fn name(self) -> &'static str {
        match self { Rod::Old => "Old Rod", Rod::Good => "Good Rod", Rod::Super => "Super Rod" }
    }

    /// The best rod the bag holds, or `None` for a bag with no rod in it.
    ///
    /// ⚠️ **Best rather than "a rod", and it is not a preference — the worse ones are strictly worse.**
    /// `ItemUseOldRod` hard-codes a lv5 Magikarp (`lb bc, 5, MAGIKARP`) and the Good Rod's table is two
    /// lv10 mons, while the Super Rod reads the map's own group. There is no map and no goal on which
    /// an earlier rod catches something a later one cannot.
    pub fn best_in_bag(bag: &crate::pokemon::bag::Bag) -> Option<Self> {
        [Rod::Old, Rod::Good, Rod::Super].into_iter()
            .filter(|r| bag.iter().any(|i| i.id == r.item() && i.quantity > 0))
            .max()
    }
}

/// When a `PolicyStep::Fish` step is finished.
///
/// A cast's *outcome* is invisible to the policy: the bite is a wild battle, and by the time
/// `pick_field_move` is polled again the battle is long over and nothing in `GameState` records that
/// it happened. (`pokedex_seen` does not help — this save has seen 112 of 151 species, so every fish
/// in every rod's table is already on it.) So the two goals below are the two things the policy *can*
/// see: a cast counter it keeps itself, and the Pokédex's **owned** set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FishGoal {
    /// Cast exactly `n` times, fleeing whatever bites. Proves the rod and the driver; the encounters
    /// themselves show up on the agent's event stream and in `pokedex_seen`.
    Casts(u32),
    /// Cast until `species` is **owned**, throwing balls at it and fleeing everything else. Bounded by
    /// `max_casts` so a species that is not in this map's table gives up rather than fishing forever.
    Catch { species: PokemonSpecies, max_casts: u32 },
}

impl PolicyStep {
    /// **C2–C5** — fish on `map` with `rod` until `goal` is met.
    ///
    /// Routing to `map` happens in `pick_overworld_action`; from there `pick_field_move` picks a water
    /// tile and hands each individual cast to [`tick`]. The step stays queued across casts (and across
    /// the battles they start), so it is one step per fishing session rather than one per cast.
    pub const fn fish(rod: Rod, map: Map, goal: FishGoal) -> Self {
        PolicyStep::Fish { rod, map, goal }
    }
}

/// `wMovementFlags` bit 6 — set for the whole of `FishingAnim` and cleared just before it returns
/// (`engine/overworld/player_animations.asm:382, 449`). The rod's text prints and the overworld is
/// reloaded *inside* that window, so without this the driver sees `GameMode::Overworld` mid-animation
/// and declares the cast resolved while the rod is still in the water.
///
/// The bit is shared with the ledge hop (`engine/overworld/ledges.asm:45`), which cannot overlap a
/// cast: fishing needs water in front, and a ledge hop needs a ledge.
const BIT_LEDGE_OR_FISHING: u8 = 1 << 6;

/// `wWalkBikeSurfState == 2` is surfing, which `FishingInit` refuses outright.
const WALK_BIKE_SURF_SURFING: u8 = 2;

/// Tilesets in `WaterTilesets` (`data/tilesets/water_tilesets.asm`). Outside these, `IsNextTileShoreOrWater`
/// fails whatever is in front of the player and every cast answers "Not the time to use that!".
const WATER_TILESETS: [u8; 9] = [0, 3, 5, 7, 13, 14, 17, 22, 23];

/// Whether a cast on a map with this tileset can do anything at all — `IsNextTileShoreOrWater` is
/// gated on `WaterTilesets` before it even looks at the tile in front, so outside these every cast
/// answers "Not the time to use that!" no matter what the map looks like. Read by
/// `MetaTileMap::actions`, which must not offer a row the game refuses (the `CutTree` rule).
pub fn tileset_holds_water(tileset: crate::pokemon::map_header::TileSetId) -> bool {
    WATER_TILESETS.contains(&(tileset as u8))
}

/// Live state of one cast. Carried in [`AgentState::Fishing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FishState {
    /// The rod to use, which is also the bag row to navigate to.
    pub rod: Rod,
    /// The water tile to face. Chosen by the policy ([`pick`]) from the live tile map.
    pub at: Point8,
    /// Press/release alternation, so every input is a fresh rising edge (§10 of the plan).
    press: bool,
    /// Set once the bag menus have opened, i.e. the cast is under way.
    entered_menu: bool,
    /// Ticks spent on this cast, so a wedge reports itself instead of pulsing A for the whole budget.
    ticks: u16,
}

/// Ceiling on driver ticks for a single cast (20 ms each).
///
/// The driver owns the walk to the shore as well as the cast, and the cast itself is slower than it
/// looks: `HandleMenuInput` inserts a delay after every cursor move, so a rod sitting at bag row 16
/// costs a second or two on its own, and `ItemUseText00`'s 80-frame pause plus `FishingAnim`'s 110
/// frames are four seconds before anything can happen. Measured at ~400–500 ticks for a cast from the
/// Pallet Fly landing, so this is triple the observed cost — it exists to catch a genuine wedge, not
/// to police a slow cast.
const TICK_BUDGET: u16 = 1500;

impl FishState {
    pub fn new(rod: Rod, at: Point8) -> Self {
        Self { rod, at, press: true, entered_menu: false, ticks: 0 }
    }

    /// Why this cast cannot happen, if it cannot — checked **before** the bag is opened, because each
    /// of these fails silently from the driver's point of view. A missing rod leaves the bag cursor
    /// hunting a row that is not there; surfing and a dry tileset both answer with "Not the time to
    /// use that!", which looks exactly like a resolved cast, so the policy would re-issue for ever.
    fn blocked_by(&self, api: &PokemonApi<'_>) -> Option<String> {
        if api.bag_item_position(self.rod.item()).is_none() {
            return Some(format!("{:?} is not in the bag", self.rod.item()));
        }
        let mmu = api.mmu();
        if mmu.read_pointer(&pokered_symbols::wWalkBikeSurfState) == WALK_BIKE_SURF_SURFING {
            return Some("cannot fish while surfing".into());
        }
        let tileset = mmu.read_pointer(&pokered_symbols::wCurMapTileset);
        if !WATER_TILESETS.contains(&tileset) {
            return Some(format!("tileset {tileset} has no water (WaterTilesets)"));
        }
        None
    }
}

/// True while `FishingAnim` is running — see [`BIT_LEDGE_OR_FISHING`].
fn fishing_animation_running(mmu: &MMU) -> bool {
    mmu.read_pointer(&pokered_symbols::wMovementFlags) & BIT_LEDGE_OR_FISHING != 0
}

// ── Policy side ──────────────────────────────────────────────────────────────────────────────────

/// Balls worth spending on a fish, cheapest first.
///
/// Deliberately **not** `Bag::best_pokeball`, which sorts by item id and so throws the Master Ball
/// first. Every fishable species is catch rate 155–255 (`data/pokemon/base_stats/*.asm`), i.e. an
/// ordinary Poké Ball is already a coin flip or better, so spending anything scarcer is waste — and
/// the Master Ball, if one is ever in the bag again, is not for a Magikarp.
const FISHING_BALLS: [ItemId; 3] = [ItemId::PokeBall, ItemId::GreatBall, ItemId::UltraBall];

/// Whether `goal` has been reached. Read by `policy.rs`, which pops the step when this is true.
///
/// `casts` is the number of casts the current step has already issued.
pub fn goal_met(state: &GameState, goal: FishGoal, casts: u32) -> bool {
    match goal {
        FishGoal::Casts(n) => casts >= n,
        FishGoal::Catch { species, max_casts } => {
            state.pokedex_owned.contains(&species) || casts >= max_casts
        }
    }
}

/// Pick the water tile to cast at, as a [`FieldMove::Fish`] — or `None` if this map has no water the
/// player can stand next to, in which case the policy pops the step rather than waiting for ever.
///
/// # Choosing the tile
///
/// Every `MetaTile::Water` on the map is a candidate; [`MetaTileMap::route_to_face`] turns each into a
/// walk-then-turn route, and the shortest wins. Two subtleties:
///
/// - **The route must stay on land.** With `can_surf` set the BFS treats water as pass-through
///   (`bfs_from_player`), so a route to a far shore may cross the sea — which the plain overworld
///   walker cannot execute, since mounting Surf is a menu chain. Preferring the shortest route almost
///   always avoids that on its own, but [`route_stays_on_land`] rejects it outright.
/// - **Water the player is already facing wins.** `route_to_face` returns an empty route in that case,
///   which sorts first, so a step re-issued between casts does not wander off to a different shore.
pub fn pick(state: &GameState, rod: Rod) -> Option<FieldMove> {
    Some(FieldMove::Fish { rod, at: nearest_castable_water(&state.map)? })
}

/// The water tile a cast from here should be aimed at, by the rules in [`pick`]'s doc — or `None`
/// when this map has no water the player can stand next to and face.
///
/// Split out of [`pick`] so `MetaTileMap::actions` can ask the same question: the fishing row it
/// offers and the cast the driver then performs have to agree about which shore, or the row routes
/// the player to one puddle and the driver casts at another.
pub fn nearest_castable_water(map: &MetaTileMap) -> Option<Point8> {
    (0..map.height as u8)
        .flat_map(|y| (0..map.width as u8).map(move |x| Point8 { x, y }))
        .filter(|&p| map.tile_at(p) == MetaTile::Water)
        .filter_map(|p| map.route_to_face(p).map(|route| (p, route)))
        .filter(|(_, route)| route_stays_on_land(map, route))
        .min_by_key(|(_, route)| route.len())
        .map(|(p, _)| p)
}

/// Whether walking `route` from the player's position never steps onto water.
///
/// The route is a button sequence, so this replays it over the tile grid. A step that lands somewhere
/// unexpected (a ledge hop, a spinner) ends the replay and the route is accepted: this is a guard
/// against the one failure mode above, not a second pathfinder.
fn route_stays_on_land(map: &MetaTileMap, route: &[JoypadButton]) -> bool {
    let mut pos = map.player_position;
    for (i, &button) in route.iter().enumerate() {
        let next = match button {
            JoypadButton::Up => Point8 { x: pos.x, y: pos.y.wrapping_sub(1) },
            JoypadButton::Down => Point8 { x: pos.x, y: pos.y.wrapping_add(1) },
            JoypadButton::Left => Point8 { x: pos.x.wrapping_sub(1), y: pos.y },
            JoypadButton::Right => Point8 { x: pos.x + 1, y: pos.y },
            _ => return true,
        };
        match map.tile_at_checked(next) {
            // The **last** button of a `route_to_face` route is the turn toward the target, and the
            // target is the water tile — that one is the point of the exercise. Water under any earlier
            // button means the route expects to Surf across, which the overworld walker cannot do.
            Some(MetaTile::Water) | Some(MetaTile::ConnectionWater(_)) => return i + 1 == route.len(),
            Some(MetaTile::Empty) | Some(MetaTile::Grass) => pos = next,
            // Anything else (a ledge hop, a spinner, a sprite that has since moved) means the replay has
            // lost track of where the player is; stop guessing and accept the route.
            _ => return true,
        }
    }
    true
}

/// The battle half of a `Fish` step: throw balls at the target species, flee everything else.
///
/// Called from `pick_battle_action` while a `Fish` step is at the queue front. Fleeing the rest is not
/// only cheaper than fighting — the party arrives from the overworld with whatever PP the trek left it
/// and a `Fish { .. Casts(n) }` session is a dozen encounters — it is also what keeps the party from
/// filling up with incidental catches.
///
/// No weakening pass: the fishable species are catch rate 155–255, where the HP term is worth a few
/// per cent at most and a stray critical hit costs the encounter outright. (This is the same
/// reasoning as `legendaries::pre_catch_action`, from the opposite end of the catch-rate range.)
pub fn pick_battle_action(state: &GameState, goal: FishGoal, actions: &[BattleAction]) -> Option<BattleAction> {
    let battle = state.battle.as_ref()?;
    if battle.battle_type != BattleType::Wild {
        return None;
    }
    if let FishGoal::Catch { species, .. } = goal {
        if battle.enemy.species == species {
            let ball = FISHING_BALLS.iter()
                .find(|&&id| state.bag.iter().any(|i| i.id == id && i.quantity > 0));
            if let Some(&ball) = ball {
                if let Some(throw) = actions.iter()
                    .find(|a| matches!(a, BattleAction::UseItem { item, .. } if item.id == ball))
                {
                    println!("[fishing] hooked a {species} — throwing a {ball:?}");
                    return Some(throw.clone());
                }
            } else {
                println!("[fishing] hooked a {species} but there is no ball to throw");
            }
        }
    }
    actions.iter().find(|a| matches!(a, BattleAction::Run)).cloned()
}

// ── Agent side ───────────────────────────────────────────────────────────────────────────────────

/// One agent tick of the fishing driver — **one cast**, start to finish.
///
/// ```text
/// overworld                       walk to a tile facing `at`, then START
///   → START menu                  cursor → 2 (ITEM), A
///   → bag list                    cursor → the rod's row, A
///   → USE/TOSS                    cursor → 0 (USE), A
///   → "used the OLD ROD!"         A — and keep mashing A right through the animation, which ends in
///                                 a `prompt` that blocks until a button is pressed
///   → no bite: back to overworld  → Idle, and the policy decides whether to cast again
///   → a bite:  a wild battle      → `assert_battle_state` takes over before this is polled again
/// ```
///
/// The driver never sees a bite's outcome, and that is deliberate: the battle is `PokemonAgent`'s job
/// and the repetition is the policy's. What it owns is a *single* well-formed cast.
pub fn tick(agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: FishState) -> Result<(), String> {
    use crate::pokemon::menu::TextBoxId;

    let game_mode = api.game_mode().unwrap_or(GameMode::Overworld);
    let casting = fishing_animation_running(api.mmu());
    let finish = |agent: &mut PokemonAgent, api: &mut PokemonApi<'_>| {
        api.release_all_buttons();
        agent.set_state(AgentState::Idle);
    };

    // ── The cast has resolved ────────────────────────────────────────────────────────────────────
    // Back in the overworld with the rod out of the water. A bite never reaches here — the wild battle
    // has already replaced this state — so this is the "not even a nibble" path (or a refusal).
    if s.entered_menu && game_mode == GameMode::Overworld && !casting {
        let response = api.mmu().read_pointer(&pokered_symbols::wRodResponse);
        if response == 2 {
            // Super Rod only: this map has no fishing group at all, so every cast here will answer the
            // same. Say so on the event stream — the policy would otherwise burn its whole cast budget.
            agent.event(AgentEvent::TextBox { message: format!("{:?}: no fish on this map", s.rod) });
        }
        finish(agent, api);
        return Ok(());
    }

    if s.ticks > TICK_BUDGET {
        agent.event(AgentEvent::TextBox {
            message: format!("Fishing: no cast completed in {TICK_BUDGET} ticks at {}", s.at),
        });
        finish(agent, api);
        return Ok(());
    }
    let s = FishState { ticks: s.ticks + 1, ..s };

    // ── In the overworld: walk to the shore, face the water, then open the bag ────────────────────
    //
    // `!casting` guards this: the fishing animation reloads the overworld under itself, so mid-cast
    // `game_mode` can read `Overworld` while the rod is still in the water. Walking (or pressing START)
    // there would be driving a player the game has frozen.
    if game_mode == GameMode::Overworld && !casting {
        if let Some(why) = s.blocked_by(api) {
            agent.event(AgentEvent::TextBox { message: format!("Fishing: {why}") });
            finish(agent, api);
            return Ok(());
        }
        let gs = agent.observe_state(api)?;
        match gs.map.route_to_face(s.at).as_deref() {
            Some([]) => {
                api.release_all_buttons();
                if s.press { api.press_button(JoypadButton::Start); }
                agent.set_state(AgentState::Fishing(FishState { press: !s.press, ..s }));
            }
            Some(&[button, ..]) => {
                api.release_all_buttons();
                api.press_button(button);
                agent.set_state(AgentState::Fishing(FishState { press: true, ..s }));
            }
            _ => {
                agent.event(AgentEvent::TextBox { message: format!("Fishing: can't reach the water at {}", s.at) });
                finish(agent, api);
            }
        }
        return Ok(());
    }

    // ── In the bag menus, or mid-animation. Plain press/release mashing, a fresh rising edge every
    //    two ticks ─────────────────────────────────────────────────────────────────────────────────
    //
    // ⚠️ **The animation must be mashed through, not waited out.** `FishingAnim` ends by printing
    // `NoNibbleText` / `ItsABiteText`, both of which end in `prompt` — they block until a button is
    // pressed — and only *then* does it clear `BIT_LEDGE_OR_FISHING`
    // (`engine/overworld/player_animations.asm:447-449`). A driver that keeps its hands off while that
    // bit is set therefore deadlocks with the game: the flag it is waiting on cannot clear until it
    // presses something. `DelayFrames` does not read the joypad and there is no menu between the rod
    // and the bite, so mashing A across the whole animation is free.
    if !s.press {
        api.release_all_buttons();
        agent.set_state(AgentState::Fishing(FishState { press: true, entered_menu: true, ..s }));
        return Ok(());
    }
    let (top_x, top_y, current, scroll) = api.menu_geometry();
    let tbid = api.menu_state().map(|m| m.text_box_id);
    let nav = |cur: u8, target: u8| -> JoypadButton {
        if cur < target { JoypadButton::Down } else if cur > target { JoypadButton::Up } else { JoypadButton::A }
    };
    let button = if (top_x, top_y) == START_MENU_ORIGIN {
        // The Pokédex is owned by construction here, but the index is asked for rather than
        // assumed — see `start_menu_row`; a seventh copy of the literal is how this drifts.
        nav(current, start_menu_row(api, StartMenuRow::Item))
    } else if tbid == Some(TextBoxId::ListMenuBox) {
        nav(current + scroll, api.bag_item_position(s.rod.item()).unwrap_or(0))
    } else if tbid == Some(TextBoxId::UseTossMenuTemplate) {
        nav(current, 0) // USE/TOSS → USE
    } else {
        JoypadButton::A // "used the OLD ROD!", "Not even a nibble!" and friends
    };
    api.release_all_buttons();
    api.press_button(button);
    agent.set_state(AgentState::Fishing(FishState { press: false, entered_menu: true, ..s }));
    Ok(())
}

impl PolicyStep {
    /// **C1** — the **Old Rod**, from the fishing guru in `VermilionOldRodHouse`.
    ///
    /// One `Interact`: the guru asks "do you like to fish?", `YesNoChoice` opens on YES and the
    /// agent's generic A-mash answers it, then `GiveItem OLD_ROD` runs and
    /// `BIT_GOT_OLD_ROD` is set (`scripts/VermilionOldRodHouse.asm:9-30`). Talking again afterwards is
    /// inert, so the interaction is repeated in case the first lands mid-script.
    pub fn old_rod_steps() -> Vec<Self> {
        rod_pickup(Map::VermilionCity, Map::VermilionOldRodHouse,
            crate::pokemon::map::MapSprite::VERMILIONOLDRODHOUSE_FISHING_GURU)
    }

    /// **C4** — the **Good Rod**, from the guru in `FuchsiaGoodRodHouse`. Same shape as C1
    /// (`scripts/FuchsiaGoodRodHouse.asm:9-30`).
    pub fn good_rod_steps() -> Vec<Self> {
        rod_pickup(Map::FuchsiaCity, Map::FuchsiaGoodRodHouse,
            crate::pokemon::map::MapSprite::FUCHSIAGOODRODHOUSE_FISHING_GURU)
    }

    /// **C5** — the **Super Rod**, from the guru in `Route12SuperRodHouse`.
    ///
    /// Route 12 is not a Fly destination, so this one flies to **Lavender** and walks south. Two things
    /// about that walk:
    ///
    /// - **The Route 12 Gate building blocks the road.** Lavender's south connection lands on the
    ///   route's north tip, and the only way past is in the gate's north warp and out its south one
    ///   (`poke_flute_steps` does the same). The two gate→Route 12 warps have to be told apart by their
    ///   landing, or `EnterMap` takes the north one straight back out and loops.
    /// - **The house is at the far south end**, `warp_event 11, 77`
    ///   (`data/maps/objects/Route12.asm:19`) — 56 tiles below the gate, past where the Snorlax was.
    pub fn super_rod_steps() -> Vec<Self> {
        let mut s = vec![
            Self::Fly { to: Map::LavenderTown },
            Self::enter(Map::Route12),
            Self::enter(Map::Route12Gate1F),
            Self::EnterMap { to_map: Map::Route12, to_position: Some(crate::geometry::Point8 { x: 10, y: 21 }) },
        ];
        s.push(Self::enter(Map::Route12SuperRodHouse));
        s.extend(std::iter::repeat_n(
            Self::Interact(crate::pokemon::map::MapSprite::ROUTE12SUPERRODHOUSE_FISHING_GURU), 3));
        s.push(Self::enter(Map::Route12)); // back outdoors, so the next `Fly` is not refused
        s
    }

    /// **C2/C3** — a fishing session at **Pallet Town**, the cheapest complete fishing spot in the game.
    ///
    /// Pallet is a Fly destination *and* has a Super Rod fishing group (`.Group1` — Tentacool and
    /// Poliwag, `data/wild/super_rod.asm:4`), so all three rods have something to catch on one map that
    /// is one step away from anywhere. Its beach is the south-east shore the Route 21 surf crossing
    /// already uses.
    pub fn fish_at_pallet_steps(rod: Rod, goal: FishGoal) -> Vec<Self> {
        vec![Self::Fly { to: Map::PalletTown }, Self::fish(rod, Map::PalletTown, goal)]
    }
}

/// Fly to `town`, step into `house`, talk to the guru until the rod is handed over, then **step back
/// outside**.
///
/// That last step is not cosmetic. Each of these legs snapshots its end state for the next one, and
/// every following leg starts with a `Fly` — which `FlyState::blocked_by` refuses indoors, popping the
/// step with a reason and leaving the rest of the queue to be walked (or, here, silently discarded for
/// want of a route). D's Moltres leg records the same rule.
fn rod_pickup(town: Map, house: Map, guru: crate::pokemon::map::MapSprite) -> Vec<PolicyStep> {
    let mut s = vec![PolicyStep::Fly { to: town }, PolicyStep::enter(house)];
    s.extend(std::iter::repeat_n(PolicyStep::Interact(guru), 3));
    s.push(PolicyStep::enter(town));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `WATER_TILESETS` is the ROM's `WaterTilesets` list, which is what decides whether a cast can
    /// happen at all. The ids are `constants/tileset_constants.asm`'s `const`s, and getting one wrong
    /// is invisible: fishing simply answers "Not the time to use that!" on the affected map.
    #[test]
    fn water_tilesets_match_the_rom_list() {
        // OVERWORLD 0, FOREST 3, DOJO 5, GYM 7, SHIP 13, SHIP_PORT 14, CAVERN 17, FACILITY 22,
        // PLATEAU 23 — `data/tilesets/water_tilesets.asm`.
        assert_eq!(WATER_TILESETS, [0, 3, 5, 7, 13, 14, 17, 22, 23]);
        // The three maps this workstream fishes from are all OVERWORLD.
        assert!(WATER_TILESETS.contains(&0));
    }

    /// The cast counter drives both goals, so its boundaries are worth pinning: `Casts(n)` must allow
    /// exactly `n`, and a `Catch` must stop on the *owned* flag rather than run to its bound.
    #[test]
    fn goal_met_counts_casts() {
        let mut state = GameState::default();
        assert!(!goal_met(&state, FishGoal::Casts(3), 2));
        assert!(goal_met(&state, FishGoal::Casts(3), 3));

        let goal = FishGoal::Catch { species: PokemonSpecies::Magikarp, max_casts: 40 };
        assert!(!goal_met(&state, goal, 39));
        assert!(goal_met(&state, goal, 40), "the bound must stop a species this map cannot hook");

        state.pokedex_owned = owning(PokemonSpecies::Magikarp);
        assert!(goal_met(&state, goal, 0), "owning the species ends the step before its bound");
    }

    /// A `Pokedex` with exactly `species` set. `Pokedex` has no setter — it is only ever decoded from
    /// `wPokedexOwned` — so the bit is built the way `contains` reads it.
    fn owning(species: PokemonSpecies) -> crate::pokemon::pokedex::Pokedex {
        let bit = species.metadata().pokedex_number - 1;
        let mut bytes = [0u8; crate::pokemon::pokedex::Pokedex::BYTES_LENGTH];
        bytes[bit as usize / 8] |= 1 << (bit % 8);
        crate::pokemon::pokedex::Pokedex::try_from_slice(&bytes).unwrap()
    }
}
