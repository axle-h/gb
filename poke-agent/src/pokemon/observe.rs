use crate::pokemon::GameState;
use crate::pokemon::PokemonApi;
use crate::pokemon::PokemonApiTrait;
use crate::pokemon::badge::Badge;
use crate::pokemon::map::Map;
use crate::pokemon::status::PokemonStatus;
use crate::pokemon::symbols::pokered_symbols;
use crate::pokemon::symbols::DmgPointerRead;
use crate::pokemon::tile::MetaTile;
use crate::pokemon::world_graph::WorldGraph;
use poke_core::geometry::Point8;

/// What a read needs beyond the [`GameState`]: the screen, the clock and the cartridge's tables.
/// The emulator reads them out of memory; the recreation holds them as fields.
pub trait Readout {
    /// Whatever text is on screen; `None` in the overworld, where there is none to read.
    fn screen_text(&self) -> Option<String>;
    /// Hours, minutes and seconds of play.
    fn play_time(&self) -> (u8, u8, u8);
    /// A mart's price for `item`; `None` for what cannot be sold.
    fn price(&self, item: crate::pokemon::item::ItemId) -> Option<u32>;
    fn pc_items(&self) -> crate::pokemon::bag::Bag;
    /// The open mart's stock, in its own order; empty when none is open.
    fn mart_stock(&self) -> Vec<crate::pokemon::item::ItemId>;
    /// The emulator behind it, for a test that reads what only the emulator has.
    fn emulated(&self) -> Option<&PokemonApi<'_>> {
        None
    }
}

impl Readout for PokemonApi<'_> {
    fn screen_text(&self) -> Option<String> {
        self.on_screen_text(false)
    }

    fn play_time(&self) -> (u8, u8, u8) {
        let mmu = self.mmu();
        (
            mmu.read_pointer(&pokered_symbols::wPlayTimeHours),
            mmu.read_pointer(&pokered_symbols::wPlayTimeMinutes),
            mmu.read_pointer(&pokered_symbols::wPlayTimeSeconds),
        )
    }

    fn price(&self, item: crate::pokemon::item::ItemId) -> Option<u32> {
        self.item_price(item)
    }

    fn pc_items(&self) -> crate::pokemon::bag::Bag {
        self.pc_stored_items()
    }

    fn mart_stock(&self) -> Vec<crate::pokemon::item::ItemId> {
        self.mart_item_list()
    }

    fn emulated(&self) -> Option<&PokemonApi<'_>> {
        Some(self)
    }
}

/// `#[derive(Serialize)]` only when something is going to serialise it.
macro_rules! view {
    ($(#[$meta:meta])* pub struct $name:ident { $($body:tt)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq)]
        #[derive(serde::Serialize)]
        pub struct $name { $($body)* }
    };
}

// ── Play time

/// `HH:MM:SS` of in-game play time. Saturates at 255:59:59, as the game itself does.
pub fn playtime(readout: &(impl Readout + ?Sized)) -> String {
    let (hours, minutes, seconds) = readout.play_time();
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

/// The same clock as a plain count of seconds.
pub fn playtime_seconds(readout: &(impl Readout + ?Sized)) -> u32 {
    let (hours, minutes, seconds) = readout.play_time();
    u32::from(hours) * 3600 + u32::from(minutes) * 60 + u32::from(seconds)
}


// ── Party

view! {
    /// One move, with the PP that decides whether it can be used again.
    pub struct MoveView {
        pub name: String,
        pub pp: u8,
        pub max_pp: u8,
        /// `None` for a status move, which the ROM prices at zero power.
        pub power: Option<u8>,
        pub accuracy: u8,
        pub move_type: String,
    }
}

view! {
    /// A party member, without the IVs, EVs and raw experience the game never shows.
    pub struct PartyMemberView {
        /// 0-based party slot, which every action targeting a Pokémon takes.
        pub slot: usize,
        pub species: String,
        /// `None` when the nickname is the species name.
        pub nickname: Option<String>,
        pub level: u8,
        pub hp: u16,
        pub max_hp: u16,
        /// `"OK"`, or the status the game shows: `"SLP"`, `"PSN"`, `"BRN"`, `"FRZ"`, `"PAR"`.
        pub status: String,
        pub fainted: bool,
        pub types: Vec<String>,
        pub attack: u16,
        pub defense: u16,
        pub speed: u16,
        pub special: u16,
        pub moves: Vec<MoveView>,
    }
}

pub fn party(state: &GameState) -> Vec<PartyMemberView> {
    state.pokemon.iter().enumerate().map(|(slot, mon)| {
        let species = format!("{:?}", mon.species);
        let nickname = mon.nickname.to_default_string();
        PartyMemberView {
            slot,
            // The game stores a nickname for every mon, defaulting to the species name in caps.
            nickname: (!nickname.eq_ignore_ascii_case(&species)).then_some(nickname),
            species,
            level: mon.level,
            hp: mon.current_hp,
            max_hp: mon.stats.hp,
            status: format!("{}", mon.status),
            fainted: mon.current_hp == 0,
            types: type_names(&mon.types),
            attack: mon.stats.attack,
            defense: mon.stats.defense,
            speed: mon.stats.speed,
            special: mon.stats.special,
            moves: mon.moves.iter().flatten().map(move_view).collect(),
        }
    }).collect()
}

fn move_view(m: &crate::pokemon::move_name::PokemonMove) -> MoveView {
    let metadata = m.name.metadata();
    MoveView {
        name: format!("{:?}", m.name),
        pp: crate::pokemon::postgame::items::move_pp(m),
        max_pp: crate::pokemon::postgame::items::max_pp(m),
        power: metadata.power,
        accuracy: metadata.accuracy,
        move_type: format!("{:?}", metadata.move_type),
    }
}

// ── Bag

view! {
    pub struct BagItemView {
        pub item: String,
        pub quantity: u8,
        /// A mart's price, from the ROM's table; `None` for anything no mart sells.
        pub price: Option<u32>,
    }
}

view! {
    pub struct BagView {
        pub money: u32,
        pub items: Vec<BagItemView>,
        /// The bag holds 20 distinct entries, and a full one silently refuses pickups.
        pub slots_used: usize,
        pub slots_total: usize,
    }
}

pub fn bag(state: &GameState, readout: &(impl Readout + ?Sized)) -> BagView {
    BagView {
        money: state.money,
        items: state.bag.iter().map(|item| BagItemView {
            item: format!("{:?}", item.id),
            quantity: item.quantity,
            price: readout.price(item.id),
        }).collect(),
        slots_used: state.bag.len(),
        slots_total: crate::pokemon::bag::Bag::MAX_ITEMS,
    }
}

// ── PC

view! {
    pub struct BoxedPokemonView {
        /// The slot the `pc:withdraw@` and `pc:release@` rows want.
        pub box_slot: u8,
        pub species: String,
        pub nickname: String,
        pub level: u8,
        pub hp: u16,
        pub moves: Vec<String>,
    }
}

view! {
    pub struct PcView {
        /// 1-based, the way the game's own CHANGE BOX menu numbers them.
        pub open_box: u8,
        pub boxes_total: u8,
        pub slots_used: usize,
        pub slots_total: usize,
        pub pokemon: Vec<BoxedPokemonView>,
        pub stored_items: Vec<String>,
        pub party_size: usize,
        /// Only the open box can be read: the other eleven live in SRAM banks the emulator does not
        /// window.
        pub note: String,
    }
}

pub fn pc(state: &GameState, readout: &(impl Readout + ?Sized)) -> PcView {
    use crate::pokemon::postgame::pc_box::{BOX_CAPACITY, BOX_COUNT};
    PcView {
        open_box: state.current_box + 1,
        boxes_total: BOX_COUNT,
        slots_used: state.boxed_pokemon.len(),
        slots_total: BOX_CAPACITY,
        pokemon: state.boxed_pokemon.iter().enumerate().map(|(slot, mon)| BoxedPokemonView {
            box_slot: slot as u8,
            species: format!("{:?}", mon.species),
            nickname: mon.nickname.to_default_string(),
            level: mon.level,
            hp: mon.current_hp,
            moves: mon.moves.iter().flatten().map(|mv| mv.name.to_string()).collect(),
        }).collect(),
        stored_items: readout.pc_items().iter().map(|item| item.to_string()).collect(),
        party_size: state.pokemon.len(),
        note: format!("Only box {} can be read.", state.current_box + 1),
    }
}

// ── Map

/// What each character of `impl Display for MetaTileMap` means.
pub const MAP_LEGEND: &[(char, &str)] = &[
    ('P', "the player"),
    ('_', "walkable"),
    ('O', "obstacle"),
    ('X', "water — needs Surf"),
    ('S', "someone or something you can interact with (person, item ball, boulder); see `people`"),
    ('W', "a warp — door, stairs, cave mouth; see `warps`"),
    ('C', "a connection to the adjacent map; see `connections`"),
    ('~', "water leading to the adjacent map"),
    ('v', "a ledge that can be jumped south only"),
    ('<', "a ledge that can be jumped west only"),
    ('>', "a ledge that can be jumped east only"),
    ('=', "a shop/gym counter — talk across it"),
    ('t', "a tree that Cut clears"),
    ('p', "a PC"),
    ('g', "tall grass — wild encounters"),
];

view! {
    /// One person (or item ball, or boulder) standing on the map.
    pub struct PersonView {
        pub index: u8,
        pub name: String,
        pub position: Point,
        pub on_screen: bool,
    }
}

view! {
    pub struct WarpView {
        pub at: Point,
        pub to_map: String,
        pub to_position: Point,
        pub reachable_from_here: bool,
    }
}

view! {
    /// Carries no actions: the turn's menu has them, with the ids this view cannot mint.
    pub struct MapView {
        pub map: String,
        pub position: Point,
        pub facing: String,
        pub width: usize,
        pub height: usize,
        pub people: Vec<PersonView>,
        pub warps: Vec<WarpView>,
        pub connections: Vec<String>,
        pub is_dark: bool,
        pub can_use_cut: bool,
        pub can_use_surf: bool,
    }
}

view! {
    /// `Point8` as `{"x": 4, "y": 7}` rather than an array a model has to guess the order of.
    pub struct Point { pub x: u8, pub y: u8 }
}

impl From<Point8> for Point {
    fn from(p: Point8) -> Self { Self { x: p.x, y: p.y } }
}

pub fn map_view(state: &GameState) -> MapView {
    let map = &state.map;
    let reachable: std::collections::HashSet<_> = map.actions().into_iter()
        .filter_map(|action| match action.tile { MetaTile::Sprite(name) => Some(name), _ => None })
        .collect();
    MapView {
        map: format!("{}", map.map),
        position: map.player_position.into(),
        facing: format!("{:?}", map.player_direction),
        width: map.width,
        height: map.height,
        // Hidden people are not on the map the player sees.
        people: map.sprites.iter().filter(|s| !s.hidden && reachable.contains(&s.name)).map(|s| PersonView {
            index: s.index,
            // Through `MetaTile::id_kind`, so the spelling is the action id's.
            name: MetaTile::Sprite(s.name).id_kind().into_owned(),
            position: s.position.into(),
            on_screen: s.on_screen,
        }).collect(),
        warps: warps(state),
        connections: {
            let mut connections: Vec<String> =
                map.connection_targets.iter().map(|m| format!("{m}")).collect();
            connections.sort();
            connections
        },
        is_dark: state.map_is_dark,
        can_use_cut: state.can_use_cut,
        can_use_surf: state.can_use_surf,
    }
}

/// The map's warps, sorted: `warp_targets` is a `HashSet`, and a reordered list reads to a model
/// as a changed world.
fn warps(state: &GameState) -> Vec<WarpView> {
    let map = &state.map;
    let routable = map.reachable_tiles();
    let mut warps: Vec<WarpView> = map.meta_tiles.iter().enumerate().filter_map(|(i, tile)| {
        let crate::pokemon::tile::MetaTile::Warp { to_map, to_position } = tile else { return None };
        let at = Point8 { x: (i % map.width) as u8, y: (i / map.width) as u8 };
        Some(WarpView {
            at: at.into(),
            to_map: format!("{to_map}"),
            to_position: (*to_position).into(),
            reachable_from_here: routable.contains(&at),
        })
    }).collect();
    warps.sort_by_key(|w| (w.at.y, w.at.x));
    warps
}

// ── Screen text

/// Whatever text is on screen.
pub fn screen_text(readout: &(impl Readout + ?Sized)) -> Option<String> {
    readout.screen_text()
}

// ── World graph

view! {
    /// One map on the way. `via` is the tile of the previous map to leave by, e.g. `"Warp at (25,
    /// 9)"`, and is absent on the first hop, the map already stood on.
    pub struct RouteHopView {
        pub map: String,
        pub via: Option<String>,
        /// Whether `via`'s tile is reachable from here; set on the second hop only.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub reachable_from_here: Option<bool>,
    }
}

/// The maps between here and `to`, in order, or `None` if the walked graph does not join them.
pub fn route(graph: &WorldGraph, from: Map, to: Map) -> Option<Vec<RouteHopView>> {
    Some(
        graph
            .shortest_path(from, to)?
            .into_iter()
            .map(|step| RouteHopView {
                map: format!("{}", step.map),
                via: step.via.zip(step.via_at).map(|(kind, at)| format!("{kind:?} at ({}, {})", at.x, at.y)),
                reachable_from_here: None,
            })
            .collect(),
    )
}

/// [`route`], plus whether the player can get to the tile it says to leave by.
pub fn route_from(graph: &WorldGraph, map: &crate::pokemon::tile_map::MetaTileMap, to: Map)
    -> Option<Vec<RouteHopView>> {
    let steps = graph.shortest_path(map.map, to)?;
    let reachable = (steps.len() > 1).then(|| map.reachable_tiles());
    Some(steps.into_iter().enumerate().map(|(index, step)| RouteHopView {
        map: format!("{}", step.map),
        via: step.via.zip(step.via_at).map(|(kind, at)| format!("{kind:?} at ({}, {})", at.x, at.y)),
        // Hop 1's `via` is the only coordinate in the route on the map being stood on.
        reachable_from_here: match (index, step.via_at, reachable.as_ref()) {
            (1, Some(at), Some(reachable)) => Some(reachable.contains(&at)),
            _ => None,
        },
    }).collect())
}

// ── Status

view! {
    /// One badge, in [`Badge::ORDER`](crate::pokemon::badge::Badge::ORDER), which is also the
    /// sprite order of `/api/badges.png`.
    pub struct BadgeView {
        pub name: String,
        pub earned: bool,
    }
}

view! {
    /// One party slot on the status panel, and the sheet it opens onto.
    pub struct PartyMonView {
        /// The species name in upper case unless renamed.
        pub nickname: String,
        /// The cartridge's own spelling, `NIDORAN♂` and `MR.MIME` included.
        pub species: String,
        pub dex: u16,
        pub level: u8,
        pub hp: u16,
        pub max_hp: u16,
        /// `""` when healthy, so the client can test it without knowing the spelling of "None".
        pub status: String,
        /// One entry for a single-typed mon.
        pub types: Vec<String>,
        pub attack: u16,
        pub defense: u16,
        pub speed: u16,
        pub special: u16,
        pub moves: Vec<MoveView>,
        pub exp: u32,
        /// The experience this level began at, and the next one begins at; `None` at 100.
        pub exp_level: u32,
        pub exp_next: Option<u32>,
    }
}

view! {
    /// What the player is fighting; a ghost the Silph Scope has not unmasked is `GHOST`, as on screen.
    pub struct OpponentView {
        pub species: String,
        /// `None` for an unmasked ghost, which has no sprite to draw.
        pub dex: Option<u16>,
        pub level: u8,
        pub hp: u16,
        pub max_hp: u16,
        pub status: String,
        pub types: Vec<String>,
        /// `Wild`, `Trainer` or `Safari`.
        pub battle: String,
        /// The party slot fighting it.
        pub active_slot: u8,
    }
}

view! {
    /// The cheap subset the web UI polls; all of it is in `GameState` but the clock.
    pub struct StatusView {
        /// The name on the save: `AI` for a model, `HUMAN`, or a random draw.
        pub trainer: String,
        /// `wPlayerID`; the client prints it five digits wide, zero-padded, as the game does.
        pub trainer_id: u16,
        pub map: String,
        pub position: Point,
        pub mode: String,
        pub badges: Vec<BadgeView>,
        pub money: u32,
        /// `HH:MM:SS` of in-game play time.
        pub playtime: String,
        pub party: Vec<PartyMonView>,
        pub in_battle: bool,
        pub opponent: Option<OpponentView>,
    }
}

pub fn status(state: &GameState, readout: &(impl Readout + ?Sized)) -> StatusView {
    StatusView {
        trainer: state.name.to_default_string(),
        trainer_id: state.player_id,
        map: format!("{}", state.map.map),
        position: state.map.player_position.into(),
        mode: format!("{:?}", state.mode),
        badges: Badge::ORDER
            .iter()
            .map(|badge| BadgeView { name: format!("{badge}"), earned: state.badges.contains(*badge) })
            .collect(),
        money: state.money,
        playtime: playtime(readout),
        party: state.pokemon.iter()
            .map(|mon| {
                let group = mon.species.metadata().experience_group;
                PartyMonView {
                    nickname: mon.nickname.to_default_string(),
                    species: species_name(mon.species),
                    dex: mon.species.metadata().pokedex_number as u16,
                    level: mon.level,
                    hp: mon.current_hp,
                    max_hp: mon.stats.hp,
                    status: status_word(mon.status),
                    types: type_names(&mon.types),
                    attack: mon.stats.attack,
                    defense: mon.stats.defense,
                    speed: mon.stats.speed,
                    special: mon.stats.special,
                    moves: mon.moves.iter().flatten()
                        .map(|m| MoveView { name: m.name.to_string(), ..move_view(m) })
                        .collect(),
                    exp: mon.experience,
                    exp_level: group.experience_for_level(mon.level.max(1)),
                    exp_next: (mon.level < 100).then(|| group.experience_for_level(mon.level + 1)),
                }
            })
            .collect(),
        in_battle: state.battle.is_some(),
        opponent: state.battle.as_ref().map(|battle| {
            let enemy = &battle.enemy;
            let ghost = crate::pokemon::battle::is_ghost_battle(state.map.map, &state.bag, battle.battle_type);
            OpponentView {
                species: if ghost { "GHOST".into() } else { species_name(enemy.species) },
                dex: (!ghost).then(|| enemy.species.metadata().pokedex_number as u16),
                level: enemy.level,
                hp: enemy.current_hp,
                max_hp: enemy.stats.hp,
                status: status_word(enemy.status),
                types: if ghost { Vec::new() } else { type_names(&enemy.types) },
                battle: battle.battle_type.to_string(),
                active_slot: battle.active_party_slot,
            }
        }),
    }
}

fn species_name(species: crate::pokemon::species::PokemonSpecies) -> String {
    poke_core::tables::MONSTER_NAMES[species as usize - 1].to_string()
}

fn status_word(status: PokemonStatus) -> String {
    match status {
        PokemonStatus::None => String::new(),
        other => format!("{other}"),
    }
}

/// A single-typed mon stores its one type in both slots.
fn type_names(types: &[crate::pokemon::pokemon::PokemonType; 2]) -> Vec<String> {
    let mut names: Vec<String> = types.iter().map(|t| format!("{t:?}")).collect();
    names.dedup();
    names
}

/// The maps the world graph knows, for a caller that wants the list without the edges.
pub fn known_maps(graph: &WorldGraph) -> Vec<Map> {
    let mut maps: Vec<Map> = graph.nodes().into_iter().map(|((map, _), _)| map).collect();
    maps.sort_by_key(|m| format!("{m}"));
    maps.dedup();
    maps
}
