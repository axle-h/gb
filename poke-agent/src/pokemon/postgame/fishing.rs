//! Workstream C — Fishing.

use gb::geometry::Point8;
use gb::joypad::JoypadButton;
use gb::mmu::MMU;
use crate::pokemon::agent::{start_menu_row, AgentEvent, AgentState, OverworldActionAbortedReason, PokemonAgent, StartMenuRow};
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

/// The three rods, in the order they are obtained — which is also worst to best, so `Ord` is the
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
    pub fn best_in_bag(bag: &crate::pokemon::bag::Bag) -> Option<Self> {
        [Rod::Old, Rod::Good, Rod::Super].into_iter()
            .filter(|r| bag.iter().any(|i| i.id == r.item() && i.quantity > 0))
            .max()
    }
}

/// When a `PolicyStep::Fish` step is finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FishGoal {
    /// Cast exactly `n` times, fleeing whatever bites.
    Casts(u32),
    /// Cast until `species` is owned, throwing balls at it and fleeing everything else.
    Catch { species: PokemonSpecies, max_casts: u32 },
}

impl PolicyStep {
    pub const fn fish(rod: Rod, map: Map, goal: FishGoal) -> Self {
        PolicyStep::Fish { rod, map, goal }
    }
}

/// `wMovementFlags` bit 6 — set for the whole of `FishingAnim` and cleared just before it returns
/// (`engine/overworld/player_animations.asm:382, 449`).
const BIT_LEDGE_OR_FISHING: u8 = 1 << 6;

/// `wWalkBikeSurfState == 2` is surfing, which `FishingInit` refuses outright.
const WALK_BIKE_SURF_SURFING: u8 = 2;

/// Tilesets in `WaterTilesets` (`data/tilesets/water_tilesets.asm`).
const WATER_TILESETS: [u8; 9] = [0, 3, 5, 7, 13, 14, 17, 22, 23];

pub fn tileset_holds_water(tileset: crate::pokemon::map_header::TileSetId) -> bool {
    WATER_TILESETS.contains(&(tileset as u8))
}

/// Live state of one cast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FishState {
    /// The rod to use, which is also the bag row to navigate to.
    pub rod: Rod,
    /// The water tile to face.
    pub at: Point8,
    press: bool,
    /// Set once the bag menus have opened, i.e. the cast is under way.
    entered_menu: bool,
    /// Ticks spent on this cast, so a wedge reports itself instead of pulsing A for the whole
    /// budget.
    ticks: u16,
}

/// Ceiling on driver ticks for a single cast (20 ms each).
const TICK_BUDGET: u16 = 1500;

/// [`TICK_BUDGET`] in whole seconds of game time, which is the number
/// [`OverworldActionAbortedReason::CastNeverFinished`] puts in front of the model.
pub(crate) const CAST_BUDGET_SECS: u64 = {
    let nanos = TICK_BUDGET as u64
        * crate::pokemon::agent::AGENT_RESOLUTION.to_duration().as_nanos() as u64;
    (nanos + 500_000_000) / 1_000_000_000
};

impl FishState {
    pub fn new(rod: Rod, at: Point8) -> Self {
        Self { rod, at, press: true, entered_menu: false, ticks: 0 }
    }

    /// Why this cast cannot happen, if it cannot — checked before the bag is opened, because each
    /// of these fails silently from the driver's point of view.
    fn blocked_by(&self, api: &PokemonApi<'_>) -> Option<CastRefusal> {
        if api.bag_item_position(self.rod.item()).is_none() {
            return Some(CastRefusal::NoRod(self.rod));
        }
        let mmu = api.mmu();
        if mmu.read_pointer(&pokered_symbols::wWalkBikeSurfState) == WALK_BIKE_SURF_SURFING {
            return Some(CastRefusal::Surfing);
        }
        let tileset = mmu.read_pointer(&pokered_symbols::wCurMapTileset);
        if !WATER_TILESETS.contains(&tileset) {
            return Some(CastRefusal::DryTileset(tileset));
        }
        None
    }
}

/// Why `FishState::blocked_by` will not let a cast start.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CastRefusal {
    /// The rod the row was minted with has left the bag.
    NoRod(Rod),
    /// `wWalkBikeSurfState == 2`.
    Surfing,
    DryTileset(u8),
}

impl std::fmt::Display for CastRefusal {
    /// A clause, not a sentence: it is read inside "the cast was refused because ...".
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRod(rod) => write!(f, "there is no {} in the bag", rod.name()),
            Self::Surfing => write!(f, "you are surfing, and a cast is made from land"),
            Self::DryTileset(tileset) => write!(f, "this map's tileset ({tileset}) has no water in it"),
        }
    }
}

/// True while `FishingAnim` is running — see [`BIT_LEDGE_OR_FISHING`].
fn fishing_animation_running(mmu: &MMU) -> bool {
    mmu.read_pointer(&pokered_symbols::wMovementFlags) & BIT_LEDGE_OR_FISHING != 0
}

// ── Policy side
// ──────────────────────────────────────────────────────────────────────────────────

/// Balls worth spending on a fish, cheapest first.
const FISHING_BALLS: [ItemId; 3] = [ItemId::PokeBall, ItemId::GreatBall, ItemId::UltraBall];

/// Whether `goal` has been reached.
pub fn goal_met(state: &GameState, goal: FishGoal, casts: u32) -> bool {
    match goal {
        FishGoal::Casts(n) => casts >= n,
        FishGoal::Catch { species, max_casts } => {
            state.pokedex_owned.contains(&species) || casts >= max_casts
        }
    }
}

/// Pick the water tile to cast at, as a [`FieldMove::Fish`] — or `None` if this map has no water
/// the player can stand next to, in which case the policy pops the step rather than waiting for
/// ever.
pub fn pick(state: &GameState, rod: Rod) -> Option<FieldMove> {
    Some(FieldMove::Fish { rod, at: nearest_castable_water(&state.map)? })
}

/// The water tile a cast from here should be aimed at, by the rules in [`pick`]'s doc — or `None`
/// when this map has no water the player can stand next to and face.
pub fn nearest_castable_water(map: &MetaTileMap) -> Option<Point8> {
    // One search for the whole sweep, not one per water tile.
    let (dist, came_from) = map.search_for_faces();
    (0..map.height as u8)
        .flat_map(|y| (0..map.width as u8).map(move |x| Point8 { x, y }))
        .filter(|&p| map.tile_at(p) == MetaTile::Water)
        .filter_map(|p| map.route_to_face_within(&dist, &came_from, p, None).map(|route| (p, route)))
        .filter(|(_, route)| route_stays_on_land(map, route))
        .min_by_key(|(_, route)| route.len())
        .map(|(p, _)| p)
}

/// Whether walking `route` from the player's position never steps onto water.
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
            // The last button of a `route_to_face` route is the turn toward the target, and the
            // target is the water tile — that one is the point of the exercise.
            Some(MetaTile::Water) | Some(MetaTile::ConnectionWater(_)) => return i + 1 == route.len(),
            Some(MetaTile::Empty) | Some(MetaTile::Grass) => pos = next,
            // Anything else (a ledge hop, a spinner, a sprite that has since moved) means the
            // replay has lost track of where the player is; stop guessing and accept the route.
            _ => return true,
        }
    }
    true
}

/// The battle half of a `Fish` step: throw balls at the target species, flee everything else.
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

// ── Agent side
// ───────────────────────────────────────────────────────────────────────────────────

/// One agent tick of the fishing driver — one cast, start to finish.
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
pub fn tick(agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: FishState) -> Result<(), String> {
    use crate::pokemon::menu::TextBoxId;

    let game_mode = api.game_mode().unwrap_or(GameMode::Overworld);
    let casting = fishing_animation_running(api.mmu());
    let finish = |agent: &mut PokemonAgent, api: &mut PokemonApi<'_>| {
        api.release_all_buttons();
        agent.set_state(AgentState::Idle);
    };
    let give_up = |agent: &mut PokemonAgent, api: &mut PokemonApi<'_>,
                   reason: OverworldActionAbortedReason| {
        api.release_all_buttons();
        let at = agent.player_at(api);
        agent.abort_overworld(MetaTile::Fish { rod: s.rod }, reason, at);
    };

    // ── The cast has resolved
    // ──────────────────────────────────────────────────────────────────── Back in the overworld
    // with the rod out of the water.
    if s.entered_menu && game_mode == GameMode::Overworld && !casting {
        let response = api.mmu().read_pointer(&pokered_symbols::wRodResponse);
        agent.event(AgentEvent::TextBox {
            message: match response {
                // Super Rod only: this map has no fishing group at all, so every cast here will
                // answer the same.
                2 => format!("You cast the {} in. There are no fish on this map at all, so \
                              casting here again will do the same.", s.rod.name()),
                _ => format!("You cast the {} in. Not even a nibble.", s.rod.name()),
            },
        });
        // A cast that caught nothing is a *completed* action, and saying so is the whole point.
        agent.event(AgentEvent::OverworldActionCompleted {
            destination: crate::pokemon::tile::MetaTile::Fish { rod: s.rod } });
        finish(agent, api);
        return Ok(());
    }

    if s.ticks > TICK_BUDGET {
        give_up(agent, api, OverworldActionAbortedReason::CastNeverFinished);
        return Ok(());
    }
    let s = FishState { ticks: s.ticks + 1, ..s };

    // ── In the overworld: walk to the shore, face the water, then open the bag
    // ────────────────────
    if game_mode == GameMode::Overworld && !casting {
        if let Some(why) = s.blocked_by(api) {
            give_up(agent, api, OverworldActionAbortedReason::CastRefused(why));
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
            // `NoRoute`, which is the sentence for exactly this: the row named a shore and the
            // walk cannot get to it.
            _ => give_up(agent, api, OverworldActionAbortedReason::NoRoute(MetaTile::Fish { rod: s.rod })),
        }
        return Ok(());
    }

    // ── In the bag menus, or mid-animation.
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
    pub fn old_rod_steps() -> Vec<Self> {
        rod_pickup(Map::VermilionCity, Map::VermilionOldRodHouse,
            crate::pokemon::map::MapSprite::VERMILIONOLDRODHOUSE_FISHING_GURU)
    }

    /// C4 — the Good Rod, from the guru in `FuchsiaGoodRodHouse`.
    pub fn good_rod_steps() -> Vec<Self> {
        rod_pickup(Map::FuchsiaCity, Map::FuchsiaGoodRodHouse,
            crate::pokemon::map::MapSprite::FUCHSIAGOODRODHOUSE_FISHING_GURU)
    }

    /// C5 — the Super Rod, from the guru in `Route12SuperRodHouse`.
    pub fn super_rod_steps() -> Vec<Self> {
        let mut s = vec![
            Self::Fly { to: Map::LavenderTown },
            Self::enter(Map::Route12),
            Self::enter(Map::Route12Gate1F),
            Self::EnterMap { to_map: Map::Route12, to_position: Some(gb::geometry::Point8 { x: 10, y: 21 }) },
        ];
        s.push(Self::enter(Map::Route12SuperRodHouse));
        s.extend(std::iter::repeat_n(
            Self::Interact(crate::pokemon::map::MapSprite::ROUTE12SUPERRODHOUSE_FISHING_GURU), 3));
        s.push(Self::enter(Map::Route12)); // back outdoors, so the next `Fly` is not refused
        s
    }

    pub fn fish_at_pallet_steps(rod: Rod, goal: FishGoal) -> Vec<Self> {
        vec![Self::Fly { to: Map::PalletTown }, Self::fish(rod, Map::PalletTown, goal)]
    }
}

/// Fly to `town`, step into `house`, talk to the guru until the rod is handed over, then step
/// back outside.
fn rod_pickup(town: Map, house: Map, guru: crate::pokemon::map::MapSprite) -> Vec<PolicyStep> {
    let mut s = vec![PolicyStep::Fly { to: town }, PolicyStep::enter(house)];
    s.extend(std::iter::repeat_n(PolicyStep::Interact(guru), 3));
    s.push(PolicyStep::enter(town));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `WATER_TILESETS` is the ROM's `WaterTilesets` list, which is what decides whether a cast
    /// can happen at all.
    #[test]
    fn water_tilesets_match_the_rom_list() {
        // OVERWORLD 0, FOREST 3, DOJO 5, GYM 7, SHIP 13, SHIP_PORT 14, CAVERN 17, FACILITY 22,
        // PLATEAU 23 — `data/tilesets/water_tilesets.asm`.
        assert_eq!(WATER_TILESETS, [0, 3, 5, 7, 13, 14, 17, 22, 23]);
        // The three maps this workstream fishes from are all OVERWORLD.
        assert!(WATER_TILESETS.contains(&0));
    }

    /// The cast counter drives both goals, so its boundaries are worth pinning: `Casts(n)` must
    /// allow exactly `n`, and a `Catch` must stop on the *owned* flag rather than run to its
    /// bound.
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

    /// A `Pokedex` with exactly `species` set.
    fn owning(species: PokemonSpecies) -> crate::pokemon::pokedex::Pokedex {
        let bit = species.metadata().pokedex_number - 1;
        let mut bytes = [0u8; crate::pokemon::pokedex::Pokedex::BYTES_LENGTH];
        bytes[bit as usize / 8] |= 1 << (bit % 8);
        crate::pokemon::pokedex::Pokedex::try_from_slice(&bytes).unwrap()
    }
}
