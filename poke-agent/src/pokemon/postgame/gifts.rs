//! Fossil revival, gift Pokémon, one-off rooms, and the party-menu script driver they share.

use gb::geometry::Point8;
use gb::joypad::JoypadButton;
use crate::pokemon::agent::{AgentEvent, AgentState, PokemonAgent};
use crate::pokemon::encoding::GameMode;
use crate::pokemon::item::ItemId;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::map_metadata::PlayerFacingDirection;
use crate::pokemon::policy::{FieldMove, PartyRef, PolicyStep};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::tile::MetaTile;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use gb::ram::ROM;
use crate::pokemon::{GameState, PokemonApi, PokemonApiTrait};

/// Silph's lift panel, a bg-event in `SilphCoElevator` shared by every floor.
const SILPH_ELEVATOR_PANEL: Point8 = Point8 { x: 3, y: 0 };

/// The Silph floors `silph_floors_steps` visits.
const SILPH_FLOORS: &[SilphFloor] = &[
    SilphFloor { lift: 1, items: &[], walk_to: Some(MapSprite::SILPHCO2F_SILPH_WORKER_F) },
    SilphFloor { lift: 3, walk_to: None, items: &[
        MapSprite::SILPHCO4F_FULL_HEAL, MapSprite::SILPHCO4F_MAX_REVIVE, MapSprite::SILPHCO4F_ESCAPE_ROPE] },
    SilphFloor { lift: 5, walk_to: None, items: &[
        MapSprite::SILPHCO6F_HP_UP, MapSprite::SILPHCO6F_X_ACCURACY] },
    // 7F's lift side, which the route into the rival pocket cannot reach.
    SilphFloor { lift: 6, walk_to: None, items: &[
        MapSprite::SILPHCO7F_CALCIUM, MapSprite::SILPHCO7F_TM_SWORDS_DANCE] },
    SilphFloor { lift: 7, items: &[], walk_to: Some(MapSprite::SILPHCO8F_SILPH_WORKER_M) },
    SilphFloor { lift: 9, walk_to: None, items: &[
        MapSprite::SILPHCO10F_TM_EARTHQUAKE, MapSprite::SILPHCO10F_RARE_CANDY, MapSprite::SILPHCO10F_CARBOS] },
];

struct SilphFloor {
    /// Lift menu index (floor − 1).
    lift: u8,
    /// Item balls to collect, in the order the walk is cheapest.
    items: &'static [MapSprite],
    /// An NPC to talk to when `items` is empty.
    walk_to: Option<MapSprite>,
}

impl PolicyStep {
    /// Hand the Helix Fossil to the Cinnabar Lab and come back for the Omanyte.
    pub fn fossil_revival_steps() -> Vec<Self> {
        let mut s = vec![Self::Fly { to: Map::CinnabarIsland }];
        s.extend(Self::into_fossil_room());
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::CINNABARLABFOSSILROOM_SCIENTIST1), 3));
        // Out to the island, which clears `EVENT_LAB_STILL_REVIVING_FOSSIL`, and back in.
        s.extend(Self::out_of_fossil_room());
        s.extend(Self::into_fossil_room());
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::CINNABARLABFOSSILROOM_SCIENTIST1), 3));
        s.extend(Self::out_of_fossil_room());
        s
    }

    /// The Old Amber from the Pewter Museum, revived into an Aerodactyl.
    pub fn old_amber_steps() -> Vec<Self> {
        let mut s = vec![
            Self::Fly { to: Map::PewterCity },
            // Venusaur leads, because `CuttingTree` only ever asks party slot 0.
            Self::MovePokemonToFront { target: PartyRef::Slot(0) },
            Self::CutTree { map: Map::PewterCity },
            Self::enter_at(Map::Museum1F, 16, 7),
        ];
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::MUSEUM1F_SCIENTIST2), 3));
        s.push(Self::enter(Map::PewterCity));
        s.push(Self::Fly { to: Map::CinnabarIsland });
        s.extend(Self::into_fossil_room());
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::CINNABARLABFOSSILROOM_SCIENTIST1), 3));
        s.extend(Self::out_of_fossil_room());
        s.extend(Self::into_fossil_room());
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::CINNABARLABFOSSILROOM_SCIENTIST1), 3));
        s.extend(Self::out_of_fossil_room());
        s
    }

    /// The Lapras the rescued Silph employee gives.
    pub fn lapras_steps() -> Vec<Self> {
        let mut s = vec![
            Self::Fly { to: Map::SaffronCity },
            Self::enter(Map::SilphCo1F),
            Self::enter(Map::SilphCoElevator),
            Self::UseElevator { panel: SILPH_ELEVATOR_PANEL, floor: 2 }, // 3F = menu index 2
            Self::EnterMap { to_map: Map::SilphCo7F, to_position: Some(Point8 { x: 5, y: 3 }) },
        ];
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::SILPHCO7F_SILPH_WORKER_M1), 3));
        s.push(Self::EnterMap { to_map: Map::SilphCo3F, to_position: Some(Point8 { x: 11, y: 11 }) });
        s.extend(Self::out_of_silph());
        s
    }

    /// Beat the Saffron Karate Master and take a Hitmonlee.
    pub fn hitmonlee_steps(bank_slot: u8) -> Vec<Self> {
        let mut s = vec![
            Self::Fly { to: Map::SaffronCity },
            Self::enter(Map::SaffronPokecenter),
            Self::Interact(MapSprite::SAFFRONPOKECENTER_NURSE),
            Self::deposit_pokemon(bank_slot, Map::SaffronPokecenter),
            Self::enter(Map::SaffronCity),
            Self::enter(Map::FightingDojo),
        ];
        s.extend(std::iter::repeat_n(Self::InteractIfReachable(MapSprite::FIGHTINGDOJO_KARATE_MASTER), 8));
        s.push(Self::CollectItem(MapSprite::FIGHTINGDOJO_HITMONLEE_POKE_BALL));
        s.push(Self::enter(Map::SaffronCity));
        s
    }

    /// The five Silph floors `complete_game_steps` never opens, and the two items on 7F it passes.
    pub fn silph_floors_steps(bank: &[(ItemId, u8)]) -> Vec<Self> {
        let mut s = vec![
            Self::Fly { to: Map::SaffronCity },
            Self::enter(Map::SaffronPokecenter),
            Self::Interact(MapSprite::SAFFRONPOKECENTER_NURSE),
        ];
        s.extend(bank.iter().map(|&(item, qty)| Self::deposit_item(item, qty, Map::SaffronPokecenter)));
        s.extend([Self::enter(Map::SaffronCity), Self::enter(Map::SilphCo1F)]);
        // Floor order is the lift's own order, so each ride is one stop where it can be.
        for floor in SILPH_FLOORS {
            s.push(Self::enter(Map::SilphCoElevator));
            s.push(Self::UseElevator { panel: SILPH_ELEVATOR_PANEL, floor: floor.lift });
            s.extend(floor.items.iter().map(|&ball| Self::CollectItem(ball)));
            s.extend(floor.walk_to.map(Self::Interact));
        }
        s.extend(Self::out_of_silph());
        s
    }

    /// The two TM gifts in the one-off Saffron houses, and the purchase one of them needs.
    pub fn saffron_tm_gifts_steps(bank: &[(ItemId, u8)]) -> Vec<Self> {
        let mut s = vec![
            Self::Fly { to: Map::SaffronCity },
            Self::enter(Map::SaffronPokecenter),
        ];
        s.extend(bank.iter().map(|&(item, qty)| Self::deposit_item(item, qty, Map::SaffronPokecenter)));
        s.extend([
            Self::enter(Map::SaffronCity),
            Self::Fly { to: Map::CeladonCity },
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonMart2F),
            Self::enter(Map::CeladonMart3F),
            Self::enter(Map::CeladonMart4F),
            Self::BuyFromMart { item: crate::pokemon::BagItem::new(ItemId::PokeDoll, 1), map: Map::CeladonMart4F },
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonCity),
            Self::Fly { to: Map::SaffronCity },
            Self::enter(Map::MrPsychicsHouse),
        ]);
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::MRPSYCHICSHOUSE_MR_PSYCHIC), 3));
        s.extend([
            Self::enter(Map::SaffronCity),
            Self::enter(Map::CopycatsHouse1F),
            Self::enter(Map::CopycatsHouse2F),
        ]);
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::COPYCATSHOUSE2F_COPYCAT), 3));
        s.extend([Self::enter(Map::CopycatsHouse1F), Self::enter(Map::SaffronCity)]);
        s
    }

    /// The Day Care on Route 5: leave a Pokémon, collect it, pay the bill.
    pub fn daycare_steps(hm_free_slot: u8) -> Vec<Self> {
        vec![
            Self::Fly { to: Map::CeruleanCity },
            // Fly lands on the wrong half of Cerulean; the trashed house bridges to the Route 5 side.
            Self::enter(Map::CeruleanTrashedHouse),
            Self::enter_at(Map::CeruleanCity, 27, 9),
            Self::enter_at(Map::Route5, 10, 0),
            Self::enter(Map::Daycare),
            Self::MovePokemonToFront { target: PartyRef::Slot(hm_free_slot) },
            Self::PartyScript { script: PartyScript::Daycare, slot: 0 }, // deposit the lead
            Self::enter(Map::Route5),
            Self::enter(Map::Daycare),
            Self::PartyScript { script: PartyScript::Daycare, slot: 0 }, // collect, and pay
            Self::enter(Map::Route5),
        ]
    }

    /// Rename a party mon at the Lavender Name Rater, then the four rooms that are nothing
    /// but text.
    pub fn name_rater_and_rooms_steps(rename_slot: u8) -> Vec<Self> {
        vec![
            Self::Fly { to: Map::LavenderTown },
            Self::enter(Map::NameRatersHouse),
            Self::PartyScript { script: PartyScript::NameRater, slot: rename_slot },
            Self::enter(Map::LavenderTown),
            Self::Fly { to: Map::ViridianCity },
            Self::enter(Map::ViridianSchoolHouse),
            Self::Interact(MapSprite::VIRIDIANSCHOOLHOUSE_BRUNETTE_GIRL),
            Self::enter(Map::ViridianCity),
            Self::Fly { to: Map::CeladonCity },
            Self::enter(Map::CeladonHotel),
            Self::Interact(MapSprite::CELADONHOTEL_GRANNY),
            Self::enter(Map::CeladonCity),
            Self::enter(Map::CeladonChiefHouse),
            Self::Interact(MapSprite::CELADONCHIEFHOUSE_CHIEF),
            Self::enter(Map::CeladonCity),
        ]
    }

    /// Ride back down to 1F (menu index 0) and step outside, so the next leg's `Fly` is allowed.
    fn out_of_silph() -> Vec<Self> {
        vec![
            Self::enter(Map::SilphCoElevator),
            Self::UseElevator { panel: SILPH_ELEVATOR_PANEL, floor: 0 },
            Self::enter(Map::SaffronCity),
        ]
    }

    fn into_fossil_room() -> Vec<Self> {
        vec![Self::enter(Map::CinnabarLab), Self::enter(Map::CinnabarLabFossilRoom)]
    }

    /// Ends on the island, both the walk the scientist asks for and the outdoor tile `Fly` needs.
    fn out_of_fossil_room() -> Vec<Self> {
        vec![Self::enter(Map::CinnabarLab), Self::enter(Map::CinnabarIsland)]
    }
}

/// A wedged conversation reports itself instead of pulsing A for the whole test budget.
const TICK_BUDGET: u32 = 1200;

/// A trade takes much longer than a Day Care visit.
const TRADE_TICK_BUDGET: u32 = 2400;

/// A one-NPC script that opens the party menu and needs the cursor driven to a chosen slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartyScript {
    Daycare,
    NameRater,
    Trade { at: Map, npc: MapSprite, give: PokemonSpecies },
}

impl PartyScript {
    /// The caller routes to this map; the driver owns only the last few tiles.
    pub const fn map(self) -> Map {
        match self {
            Self::Daycare => Map::Daycare,
            Self::NameRater => Map::NameRatersHouse,
            Self::Trade { at, .. } => at,
        }
    }

    const fn sprite(self) -> MapSprite {
        match self {
            Self::Daycare => MapSprite::DAYCARE_GENTLEMAN,
            Self::NameRater => MapSprite::NAMERATERSHOUSE_NAME_RATER,
            Self::Trade { npc, .. } => npc,
        }
    }
}

/// Length of a `wPartyMonNicks` entry.
const NAME_LENGTH: usize = 11;

/// What "done" looks like, captured before the conversation starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Baseline {
    /// Day Care: any change is completion, so one test serves deposit and collection.
    PartyCount(u8),
    Nickname([u8; NAME_LENGTH]),
    SpeciesGone(PokemonSpecies),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartyScriptState {
    pub script: PartyScript,
    /// The mon to hand over or rename; ignored when collecting from the Day Care.
    pub slot: u8,
    /// The tile to face and the facing, resolved from `actions()`.
    pub stand: Point8,
    pub facing: PlayerFacingDirection,
    baseline: Baseline,
    /// Press/release alternation, so every input is a fresh rising edge.
    pub press: bool,
    pub entered_menu: bool,
    pub ticks: u32,
}

impl PartyScriptState {
    pub fn new(script: PartyScript, slot: u8, npc: (Point8, PlayerFacingDirection), api: &PokemonApi<'_>) -> Self {
        Self {
            script,
            slot,
            stand: npc.0,
            facing: npc.1,
            baseline: match script {
                PartyScript::Daycare => Baseline::PartyCount(party_count(api)),
                PartyScript::NameRater => Baseline::Nickname(nickname(api, slot)),
                PartyScript::Trade { give, .. } => Baseline::SpeciesGone(give),
            },
            press: true,
            entered_menu: false,
            ticks: 0,
        }
    }

    fn done(&self, api: &PokemonApi<'_>) -> bool {
        match self.baseline {
            Baseline::PartyCount(before) => party_count(api) != before,
            Baseline::Nickname(before) => nickname(api, self.slot) != before,
            Baseline::SpeciesGone(give) => !party_holds(api, give),
        }
    }

    fn describe(&self, api: &PokemonApi<'_>) -> String {
        match self.baseline {
            Baseline::PartyCount(before) => format!("party {before} → {}", party_count(api)),
            Baseline::Nickname(_) => format!("slot {} renamed", self.slot),
            Baseline::SpeciesGone(give) => format!("traded away {give:?}"),
        }
    }
}

fn party_holds(api: &PokemonApi<'_>, species: PokemonSpecies) -> bool {
    let base = pokered_symbols::wPartySpecies.address;
    (0..party_count(api)).any(|i| api.mmu().read(base + i as u16) == species as u8)
}

fn party_count(api: &PokemonApi<'_>) -> u8 {
    api.mmu().read_pointer(&pokered_symbols::wPartyCount)
}

/// The raw nickname bytes of party `slot` — compared, never decoded, so no charmap is involved.
fn nickname(api: &PokemonApi<'_>, slot: u8) -> [u8; NAME_LENGTH] {
    let base = pokered_symbols::wPartyMonNicks.address + slot as u16 * NAME_LENGTH as u16;
    std::array::from_fn(|i| api.mmu().read(base + i as u16))
}

/// Resolve the walk to the script's NPC, as `game_corner::pick_sale` does a clerk.
pub fn pick(state: &GameState, script: PartyScript, slot: u8) -> Option<FieldMove> {
    // A trade finds its own slot rather than trusting the caller's.
    let slot = match script {
        PartyScript::Trade { give, .. } => match state.pokemon.iter().position(|p| p.species == give) {
            Some(i) => i as u8,
            None => {
                println!("[policy] {script:?}: no {give:?} in the party to trade");
                return None;
            }
        },
        _ => slot,
    };
    let npc = state.map.sprites.iter()
        .find(|s| !s.hidden && s.name == script.sprite().name)?;
    let action = state.map.actions().into_iter()
        .find(|a| a.tile == MetaTile::Sprite(npc.name))?;

    let stand = action.destination;
    let (dx, dy) = (npc.position.x as i16 - stand.x as i16, npc.position.y as i16 - stand.y as i16);
    let facing = match (dx.signum(), dy.signum()) {
        (0, -1) => PlayerFacingDirection::Up,
        (0, 1) => PlayerFacingDirection::Down,
        (-1, 0) => PlayerFacingDirection::Left,
        (1, 0) => PlayerFacingDirection::Right,
        _ => {
            println!("[policy] {script:?}: NPC at {} is not aligned with {stand}", npc.position);
            return None;
        }
    };
    let face_tile = match facing {
        PlayerFacingDirection::Up => Point8 { x: stand.x, y: stand.y.saturating_sub(1) },
        PlayerFacingDirection::Down => Point8 { x: stand.x, y: stand.y + 1 },
        PlayerFacingDirection::Left => Point8 { x: stand.x.saturating_sub(1), y: stand.y },
        PlayerFacingDirection::Right => Point8 { x: stand.x + 1, y: stand.y },
    };
    Some(FieldMove::UsePartyScript { script, slot, npc: (face_tile, facing) })
}

pub fn tick(agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: PartyScriptState) -> Result<(), String> {
    let game_mode = api.game_mode().unwrap_or(GameMode::Overworld);

    let abort = |agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, why: String| {
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox { message: format!("party-script: {why}") });
        agent.set_state(AgentState::Idle);
    };

    if s.entered_menu && s.done(api) {
        if game_mode != GameMode::Overworld {
            api.release_all_buttons();
            if s.press { api.press_button(JoypadButton::A); } // clear the closing text
            agent.set_state(AgentState::UsingPartyScript(PartyScriptState { press: !s.press, ticks: s.ticks + 1, ..s }));
            return Ok(());
        }
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox { message: format!("{:?}: {}", s.script, s.describe(api)) });
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    let budget = match s.script {
        PartyScript::Trade { .. } => TRADE_TICK_BUDGET,
        _ => TICK_BUDGET,
    };
    if s.ticks > budget {
        abort(agent, api, format!("nothing changed in {budget} ticks"));
        return Ok(());
    }

    if game_mode == GameMode::Overworld && !s.entered_menu {
        let gs = agent.observe_state(api)?;
        match gs.map.route_to_face_dir(s.stand, Some(s.facing)).as_deref() {
            Some([]) => {
                api.release_all_buttons();
                if s.press { api.press_button(JoypadButton::A); }
                agent.set_state(AgentState::UsingPartyScript(PartyScriptState { press: !s.press, ticks: s.ticks + 1, ..s }));
            }
            Some(&[btn, ..]) => {
                api.release_all_buttons();
                api.press_button(btn);
                agent.set_state(AgentState::UsingPartyScript(PartyScriptState { press: true, ticks: s.ticks + 1, ..s }));
            }
            _ => abort(agent, api, format!("can't reach the NPC at {}", s.stand)),
        }
        return Ok(());
    }

    let s = PartyScriptState { entered_menu: true, ticks: s.ticks + 1, ..s };
    if !s.press {
        api.release_all_buttons();
        agent.set_state(AgentState::UsingPartyScript(PartyScriptState { press: true, ..s }));
        return Ok(());
    }

    let (top_x, top_y, cursor, _) = api.menu_geometry();
    // The party list's box origin, the same signal `agent::field_move_menu_button` keys on.
    let button = if top_x == 0 && (top_y == 1 || top_y == 3) {
        if cursor < s.slot { JoypadButton::Down }
        else if cursor > s.slot { JoypadButton::Up }
        else { JoypadButton::A }
    } else {
        JoypadButton::A
    };

    api.release_all_buttons();
    api.press_button(button);
    agent.set_state(AgentState::UsingPartyScript(PartyScriptState { press: false, ..s }));
    Ok(())
}
