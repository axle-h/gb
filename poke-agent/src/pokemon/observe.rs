
use crate::pokemon::GameState;
use crate::pokemon::PokemonApi;
use crate::pokemon::PokemonApiTrait;
use crate::pokemon::badge::Badge;
use crate::pokemon::battle::BattleType;
use crate::pokemon::map::Map;
use crate::pokemon::status::PokemonStatus;
use crate::pokemon::symbols::pokered_symbols;
use crate::pokemon::symbols::DmgPointerRead;
use crate::pokemon::tile::MetaTile;
use crate::pokemon::world_graph::WorldGraph;
use gb::geometry::Point8;

/// `#[derive(Serialize)]` only when something is going to serialise it.
macro_rules! view {
    ($(#[$meta:meta])* pub struct $name:ident { $($body:tt)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq)]
        #[derive(serde::Serialize)]
        pub struct $name { $($body)* }
    };
}

// ── Trainer
// ──────────────────────────────────────────────────────────────────────────────────────

// There was a `TrainerView` and a `read_trainer` here.

/// `HH:MM:SS` of in-game play time. Saturates at 255:59:59, as the game itself does.
pub fn playtime(api: &PokemonApi<'_>) -> String {
    let (hours, minutes, seconds) = playtime_parts(api);
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

/// The same clock as a plain count of seconds.
pub fn playtime_seconds(api: &PokemonApi<'_>) -> u32 {
    let (hours, minutes, seconds) = playtime_parts(api);
    u32::from(hours) * 3600 + u32::from(minutes) * 60 + u32::from(seconds)
}

fn playtime_parts(api: &PokemonApi<'_>) -> (u8, u8, u8) {
    let mmu = api.mmu();
    (
        mmu.read_pointer(&pokered_symbols::wPlayTimeHours),
        mmu.read_pointer(&pokered_symbols::wPlayTimeMinutes),
        mmu.read_pointer(&pokered_symbols::wPlayTimeSeconds),
    )
}

// ── Party
// ────────────────────────────────────────────────────────────────────────────────────────

view! {
    /// One move, with the PP that decides whether it can be used again.
    pub struct MoveView {
        pub name: String,
        pub pp: u8,
        pub max_pp: u8,
        /// `None` for a status move — the ROM prices those at zero power, which is not the same
        /// thing as "does nothing".
        pub power: Option<u8>,
        pub accuracy: u8,
        pub move_type: String,
    }
}

view! {
    /// A party member. Deliberately *not* the whole [`crate::pokemon::pokemon::Pokemon`]: IVs,
    /// EVs and raw experience are invisible in-game and would be several hundred tokens of noise
    /// per mon.
    pub struct PartyMemberView {
        /// 0-based party slot, which is what every action that targets a Pokémon takes.
        pub slot: usize,
        pub species: String,
        /// `None` when the mon has not been nicknamed, i.e. the nickname is the species name.
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
            types: {
                let mut types: Vec<String> = mon.types.iter().map(|t| format!("{t:?}")).collect();
                types.dedup(); // a single-type mon stores its one type in both slots
                types
            },
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
        pp: m.pp,
        max_pp: metadata.pp,
        power: metadata.power,
        accuracy: metadata.accuracy,
        move_type: format!("{:?}", metadata.move_type),
    }
}

// ── Bag
// ──────────────────────────────────────────────────────────────────────────────────────────

view! {
    pub struct BagItemView {
        pub item: String,
        pub quantity: u8,
        /// What a mart charges, from the ROM's own price table. `None` for the key items and TMs
        /// no mart sells — which is also the answer to "can I buy more of this?".
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

pub fn bag(state: &GameState, api: &PokemonApi<'_>) -> BagView {
    BagView {
        money: state.money,
        items: state.bag.iter().map(|item| BagItemView {
            item: format!("{:?}", item.id),
            quantity: item.quantity,
            price: api.item_price(item.id),
        }).collect(),
        slots_used: state.bag.len(),
        slots_total: crate::pokemon::bag::Bag::MAX_ITEMS,
    }
}

// ── PC
// ───────────────────────────────────────────────────────────────────────────────────────────

view! {
    pub struct BoxedPokemonView {
        /// The slot `use_field_move` wants for `withdraw` and `release`.
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
        /// The honest caveat, and it is a real limit rather than a hedge. Eleven of the twelve
        /// boxes live in SRAM banks the emulator layer does not window, so only the open one can
        /// be read — and `change_box` is what copies WRAM to SRAM, which means looking in another
        /// box is a write that saves the game rather than a read.
        pub note: String,
    }
}

pub fn pc(state: &GameState, api: &PokemonApi<'_>) -> PcView {
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
        stored_items: api.pc_stored_items().iter().map(|item| item.to_string()).collect(),
        party_size: state.pokemon.len(),
        note: format!(
            "Only box {} can be read. Switching to another with `change_box` saves the game.",
            state.current_box + 1,
        ),
    }
}

// ── Map
// ──────────────────────────────────────────────────────────────────────────────────────────

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
        /// Whether `at` can be walked to from where the player is standing right now.
        pub reachable_from_here: bool,
    }
}

view! {
    /// The reachable actions are deliberately not here. They were, and they were a second copy of
    /// the menu the turn request already renders — but *without the ids*, since an id is minted
    /// from `MetaTile::kind` in the tool layer and this view never had one.
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
    /// `Point8` by another name. Its own type so the JSON is `{"x": 4, "y": 7}` rather than a
    /// two-element array a model has to guess the order of.
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
        // Hidden people are absent from the map the player sees; reporting them would invite the
        // model to try to talk to someone who is not there.
        people: map.sprites.iter().filter(|s| !s.hidden && reachable.contains(&s.name)).map(|s| PersonView {
            index: s.index,
            // Through `MetaTile::id_kind` so this is the same spelling as the action id, by
            // construction rather than by two functions agreeing.
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

/// The map's warps, sorted so two consecutive reads of an unchanged map produce identical output
/// — `warp_targets` is a `HashSet` and would otherwise reorder on every call, which reads to a
/// model as the world having changed.
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
// ──────────────────────────────────────────────────────────────────────────────────

/// Whatever text is on screen, decoded from VRAM.
pub fn screen_text(api: &PokemonApi<'_>) -> Option<String> {
    api.on_screen_text(false)
}

// ── World graph
// ──────────────────────────────────────────────────────────────────────────────────

view! {
    /// One map on the way to somewhere. `via` is how it is entered and which tile of the previous
    /// map to leave by — `"Warp at (25, 9)"`, `"Connection at (9, 0)"` — and is absent on the
    /// first hop, which is the map already stood on.
    pub struct RouteHopView {
        pub map: String,
        pub via: Option<String>,
        /// Whether `via`'s tile can actually be walked to from where the player is standing right
        /// now. Only ever set on the second hop, which is the only one that is a fact about the
        /// map under the player's feet; `None` everywhere else, and absent from the JSON.
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

/// [`route`], with the one thing the graph cannot know added: whether the player can get to the
/// tile it is telling them to leave by.
pub fn route_from(graph: &WorldGraph, map: &crate::pokemon::tile_map::MetaTileMap, to: Map)
    -> Option<Vec<RouteHopView>> {
    // One search, not two.
    let steps = graph.shortest_path(map.map, to)?;
    let reachable = (steps.len() > 1).then(|| map.reachable_tiles());
    Some(steps.into_iter().enumerate().map(|(index, step)| RouteHopView {
        map: format!("{}", step.map),
        via: step.via.zip(step.via_at).map(|(kind, at)| format!("{kind:?} at ({}, {})", at.x, at.y)),
        // Hop 0 is the map being stood on and carries no `via`; hop 1 names the tile to leave it
        // by, which is the only coordinate in the whole route on ground the player is standing
        // on.
        reachable_from_here: match (index, step.via_at, reachable.as_ref()) {
            (1, Some(at), Some(reachable)) => Some(reachable.contains(&at)),
            _ => None,
        },
    }).collect())
}

// ── Battle
// ───────────────────────────────────────────────────────────────────────────────────────

view! {
    pub struct BattleSideView {
        pub species: String,
        pub level: u8,
        pub hp: u16,
        pub max_hp: u16,
        pub status: String,
        /// The field that made this read answerable. Every `MoveView` carries its own `move_type`
        /// and `power`, so a model reading this had the *attacking* half of every matchup and
        /// never the defending half — no types on either side, so the multiplier could not be
        /// worked out from the result at all, and `read_party` (which does carry types) only ever
        /// covers your own.
        pub types: Vec<String>,
        /// Slot of a move Disable has locked out this battle. The game bounces straight back to
        /// the move menu if it is chosen, so a decider that ignores this can loop forever.
        pub disabled_move_slot: Option<u8>,
        pub moves: Vec<MoveView>,
    }
}

view! {
    /// The legal actions are deliberately not here, for the same reason [`MapView`] does not
    /// carry them: they were a second copy of the turn's own battle menu without the ids that
    /// menu mints, so every one of them was a choice the model could not make.
    pub struct BattleView {
        /// `"Wild"`, `"Trainer"` or `"Safari"`.
        pub battle_type: String,
        pub player: BattleSideView,
        pub enemy: BattleSideView,
        /// Which party slot is out.
        pub active_party_slot: u8,
        /// Set while the enemy has the player in Wrap/Fire Spin/Clamp/Bind. The battle menu still
        /// opens and items, switching and running all still work, but any move chosen is replaced
        /// with "cannot move" — so a decider that keeps picking moves here achieves nothing.
        pub enemy_trapping: bool,
        /// The live catch rate `ItemUseBall` compares against, after any Safari rock or bait.
        pub enemy_catch_rate: u8,
    }
}

/// `None` when no battle is in progress.
pub fn battle(state: &GameState) -> Option<BattleView> {
    let battle = state.battle.as_ref()?;
    let side = |summary: &crate::pokemon::pokemon::PokemonSummary| BattleSideView {
        species: format!("{:?}", summary.species),
        level: summary.level,
        hp: summary.current_hp,
        max_hp: summary.stats.hp,
        status: format!("{}", summary.status),
        types: {
            let mut types: Vec<String> = summary.types.iter().map(|t| format!("{t:?}")).collect();
            types.dedup(); // a single-type mon stores its one type in both slots
            types
        },
        disabled_move_slot: summary.disabled_move_slot,
        moves: summary.moves.iter().flatten().map(move_view).collect(),
    };
    Some(BattleView {
        battle_type: match battle.battle_type {
            BattleType::Wild => "Wild",
            BattleType::Trainer => "Trainer",
            BattleType::Safari => "Safari",
        }.to_string(),
        player: side(&battle.player),
        enemy: side(&battle.enemy),
        active_party_slot: battle.active_party_slot,
        enemy_trapping: battle.enemy_trapping,
        enemy_catch_rate: battle.enemy_catch_rate,
    })
}

// ── Status
// ───────────────────────────────────────────────────────────────────────────────────────

view! {
    /// One badge and whether it has been earned, in the order of
    /// [`Badge::ORDER`](crate::pokemon::badge::Badge::ORDER) — which is also the order of the
    /// sprites in `/api/badges.png`, so index `i` is the badge and the sprite.
    pub struct BadgeView {
        pub name: String,
        pub earned: bool,
    }
}

view! {
    /// One party slot on the status panel: enough for a sprite, a name and a health bar.
    pub struct PartyMonView {
        /// What the player calls it, which is the species name in upper case unless they renamed
        /// it.
        pub nickname: String,
        pub dex: u16,
        pub level: u8,
        pub hp: u16,
        pub max_hp: u16,
        /// `""` when healthy, so the client can test it without knowing the spelling of "None".
        pub status: String,
    }
}

view! {
    /// The cheap subset the web UI polls at 10 Hz. Everything but the clock is already in
    /// `GameState`, so this costs one clone of a few small fields and three byte reads.
    pub struct StatusView {
        /// The name on the save, which is whoever the run was started for — `GB_MODEL` shortened
        /// to the seven characters Gen 1 allows, or `HUMAN`, or a random draw.
        pub trainer: String,
        /// `wPlayerID`. Sent as the number and formatted by the client, because the game itself
        /// prints it five digits wide with leading zeroes (`PrintNumber`, `LEADING_ZEROES | 2,
        /// 5`) and that is what a player recognises as their ID.
        pub trainer_id: u16,
        pub map: String,
        pub position: Point,
        pub mode: String,
        pub badges: Vec<BadgeView>,
        pub money: u32,
        /// `HH:MM:SS` of in-game play time — the run's own clock, which is what a viewer wants to
        /// see rather than how long the process has been up.
        pub playtime: String,
        pub party: Vec<PartyMonView>,
        pub in_battle: bool,
    }
}

pub fn status(state: &GameState, api: &PokemonApi<'_>) -> StatusView {
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
        playtime: playtime(api),
        party: state.pokemon.iter()
            .map(|mon| PartyMonView {
                nickname: mon.nickname.to_default_string(),
                dex: mon.species.metadata().pokedex_number as u16,
                level: mon.level,
                hp: mon.current_hp,
                max_hp: mon.stats.hp,
                status: match mon.status {
                    PokemonStatus::None => String::new(),
                    other => format!("{other}"),
                },
            })
            .collect(),
        in_battle: state.battle.is_some(),
    }
}

/// The maps the world graph knows, for a caller that wants the list without the edges.
pub fn known_maps(graph: &WorldGraph) -> Vec<Map> {
    let mut maps: Vec<Map> = graph.nodes().into_iter().map(|((map, _), _)| map).collect();
    maps.sort_by_key(|m| format!("{m}"));
    maps.dedup();
    maps
}
