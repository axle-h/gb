//! **W0.5** — the observation facade: every read the LLM tool layer needs, in one place.
//!
//! Each function here is pure over the triple a policy already holds at a poll — `&GameState`, a
//! `&mut PokemonApi`, and the agent's `&WorldGraph` — which is exactly what
//! [`Policy::service_tools`](crate::pokemon::policy::Policy::service_tools) is handed. That is not a
//! coincidence but the point of the module: a tool call is answered by a direct function call
//! against state already in hand, with no round trip through the agent and no second read of RAM
//! that could disagree with the first.
//!
//! Nothing here formats prose or decides anything. The types are plain data, `Serialize` under the
//! `web` feature, and the tool layer in W5 is a thin dispatch onto them.
//!
//! **Why a struct per view rather than serialising `GameState` directly.** `GameState` is the
//! agent's working set: `raw_tile_ids`, `tile_pair_collisions`, spinner tables, the BFS's inputs. It
//! is large, it is shaped for pathfinding, and most of it would be noise in a context window — and
//! every rename inside it would silently change the tool schema the model was prompted against.
//! These views are the contract; `GameState` stays free to move.

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
use crate::geometry::Point8;

/// `#[derive(Serialize)]` only when something is going to serialise it. `serde` is optional (see
/// `Cargo.toml`) because nothing in the emulator or the agent needs it — only what leaves the
/// process — and a default build should not pay for a proc macro it never uses.
macro_rules! view {
    ($(#[$meta:meta])* pub struct $name:ident { $($body:tt)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq)]
        #[cfg_attr(feature = "web", derive(serde::Serialize))]
        pub struct $name { $($body)* }
    };
}

// ── Trainer ──────────────────────────────────────────────────────────────────────────────────────

// ⚠️ **There was a `TrainerView` and a `read_trainer` here.** Everything it returned but the
// Pokédex counts was already in the header of every turn request — badges, money, play time — and
// the counts are one line, so they moved into the header too and the tool went. A read whose answer
// the model was already holding is a round trip bought for nothing.

/// `HH:MM:SS` of in-game play time. Saturates at 255:59:59, as the game itself does.
///
/// Public because a turn request wants it in its header and the status heartbeat wants it too —
/// see [`crate::llm::prompt::ApiSnapshot`].
pub fn playtime(api: &PokemonApi<'_>) -> String {
    let (hours, minutes, seconds) = playtime_parts(api);
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

/// The same clock as a plain count of seconds.
///
/// ⚠️ **Anything that sorts or compares play times must use this, not [`playtime`].** The hours field
/// runs to 255, so the string is two digits below 100 hours and three above it, and a lexical
/// comparison puts `255:59:59` *before* `06:12:44` — a ranking that inverts itself only for the runs
/// that took longest, which is exactly the kind nobody checks. `run::hall_of_fame` ranks on this.
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

// ── Party ────────────────────────────────────────────────────────────────────────────────────────

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
    /// A party member. Deliberately *not* the whole [`crate::pokemon::pokemon::Pokemon`]: IVs, EVs
    /// and raw experience are invisible in-game and would be several hundred tokens of noise per
    /// mon. `experience_to_next_level` is the part of that a player can actually see.
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
            // The game stores a nickname for every mon, defaulting to the species name in caps. Only
            // report one when the player actually chose it, or every party listing carries six
            // redundant strings.
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

// ── Bag ──────────────────────────────────────────────────────────────────────────────────────────

view! {
    pub struct BagItemView {
        pub item: String,
        pub quantity: u8,
        /// What a mart charges, from the ROM's own price table. `None` for the key items and TMs no
        /// mart sells — which is also the answer to "can I buy more of this?".
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

// ── Map ──────────────────────────────────────────────────────────────────────────────────────────

/// What each character of `impl Display for MetaTileMap` means.
///
/// ⚠️ **`read_map` no longer ships this.** The model is sent a *picture* now — see
/// [`crate::llm::map_image`] — and a legend describing an ASCII grid it is not being given would be
/// pure confusion. `Display` itself stays, because every dump and probe in the repo prints through
/// it and the renderer falls back to it for a map with no metadata, so this stays as the
/// documentation of that alphabet.
#[allow(dead_code)]
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
    ///
    /// ⚠️ **`name` is the id form, not the pretty one.** It is spelled exactly as the last field of
    /// the action id the menu offers — `Pokedex1`, not `Pokedex 1` — because the only thing the
    /// model does with a name is find the row that talks to them. Two spellings of the same person
    /// across two blocks of the same request is a way to be wrong that has no upside.
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
    }
}

view! {
    /// ⚠️ **The reachable actions are deliberately not here.** They were, and they were a second
    /// copy of the menu the turn request already renders — but *without the ids*, since an id is
    /// minted from `MetaTile::kind` in the tool layer and this view never had one. A duplicate the
    /// model cannot quote back is worse than no duplicate: it reads as a list of choices and every
    /// one of them is rejected. The menu in the turn is the only list of actions there is.
    ///
    /// ⚠️ **Neither is the terrain.** This carried a `grid` of ASCII and the `legend` that explained
    /// it; `read_map` now answers with a rendered picture ([`crate::llm::map_image`]) and this is
    /// what the picture cannot say — names, and exact coordinates for a model to quote back. Sending
    /// both would be the same map twice, in two coordinate systems, for twice the tokens.
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
        // model to try to talk to someone who is not there. ⚠️ So would someone the agent cannot
        // route to: the menu lists only people it can reach, so a person here and not there reads
        // as an action the menu forgot, and the deployed run spent `press_buttons` trying to walk
        // to Rockets on the far side of a Mt Moon wall. The predicate is `actions()` itself, so
        // this list and the menu agree by construction.
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

/// The map's warps, sorted so two consecutive reads of an unchanged map produce identical output —
/// `warp_targets` is a `HashSet` and would otherwise reorder on every call, which reads to a model
/// as the world having changed.
fn warps(state: &GameState) -> Vec<WarpView> {
    let map = &state.map;
    let mut warps: Vec<WarpView> = map.meta_tiles.iter().enumerate().filter_map(|(i, tile)| {
        let crate::pokemon::tile::MetaTile::Warp { to_map, to_position } = tile else { return None };
        Some(WarpView {
            at: Point { x: (i % map.width) as u8, y: (i / map.width) as u8 },
            to_map: format!("{to_map}"),
            to_position: (*to_position).into(),
        })
    }).collect();
    warps.sort_by_key(|w| (w.at.y, w.at.x));
    warps
}

// ── Screen text ──────────────────────────────────────────────────────────────────────────────────

/// Whatever text is on screen, decoded from VRAM.
///
/// `None` in the overworld — and that is not a failure to report as one. The overworld has no
/// dialogue font loaded, so there is nothing to decode; a menu or a text box is the only time this
/// answers.
pub fn screen_text(api: &PokemonApi<'_>) -> Option<String> {
    api.on_screen_text(false)
}

// ── World graph ──────────────────────────────────────────────────────────────────────────────────

view! {
    /// One map on the way to somewhere. `via` is how it is entered and **which tile of the previous
    /// map to leave by** — `"Warp at (25, 9)"`, `"Connection at (9, 0)"` — and is absent on the
    /// first hop, which is the map already stood on. ⚠️ The coordinate is on the hop *before* this
    /// one, in that map's action-id space, because the choice is made there: a bare `"Warp"` on a
    /// floor with four warps to the same map answered nothing, and the deployed run read it as the
    /// nearest one.
    pub struct RouteHopView {
        pub map: String,
        pub via: Option<String>,
    }
}

/// The maps between here and `to`, in order, or `None` if the walked graph does not join them.
///
/// ⚠️ **This replaced a view that serialised the whole graph**, and the reason is size rather than
/// taste: every visited `(map, entry)` node with all of its edges is unbounded by construction and,
/// by the late game, a meaningful fraction of a context window in one tool result. Nothing ever
/// wanted the adjacency list — the question is always "which way is Celadon" — so the search runs
/// here, where the graph already is, and what crosses into the context is the answer to it.
///
/// ⚠️ **Only maps the player has physically stood on.** The graph is built incrementally by
/// [`WorldGraph::observe`], so `None` means "no route through ground you have walked", never "does
/// not exist" or "unreachable".
pub fn route(graph: &WorldGraph, from: Map, to: Map) -> Option<Vec<RouteHopView>> {
    Some(
        graph
            .shortest_path(from, to)?
            .into_iter()
            .map(|step| RouteHopView {
                map: format!("{}", step.map),
                via: step.via.zip(step.via_at).map(|(kind, at)| format!("{kind:?} at ({}, {})", at.x, at.y)),
            })
            .collect(),
    )
}

// ── Battle ───────────────────────────────────────────────────────────────────────────────────────

view! {
    pub struct BattleSideView {
        pub species: String,
        pub level: u8,
        pub hp: u16,
        pub max_hp: u16,
        pub status: String,
        /// ⚠️ **The field that made this read answerable.** Every `MoveView` carries its own
        /// `move_type` and `power`, so a model reading this had the *attacking* half of every
        /// matchup and never the defending half — no types on either side, so the multiplier could
        /// not be worked out from the result at all, and `read_party` (which does carry types) only
        /// ever covers your own. Paying for a read and still needing the type chart from memory is
        /// the read not doing its job.
        pub types: Vec<String>,
        /// Slot of a move Disable has locked out this battle. The game bounces straight back to the
        /// move menu if it is chosen, so a decider that ignores this can loop forever.
        pub disabled_move_slot: Option<u8>,
        pub moves: Vec<MoveView>,
    }
}

view! {
    /// ⚠️ **The legal actions are deliberately not here**, for the same reason [`MapView`] does not
    /// carry them: they were a second copy of the turn's own battle menu without the ids that menu
    /// mints, so every one of them was a choice the model could not make.
    pub struct BattleView {
        /// `"Wild"`, `"Trainer"` or `"Safari"`.
        pub battle_type: String,
        pub player: BattleSideView,
        pub enemy: BattleSideView,
        /// Which party slot is out.
        pub active_party_slot: u8,
        /// ⚠️ Set while the enemy has the player in Wrap/Fire Spin/Clamp/Bind. The battle menu still
        /// opens and items, switching and running all still work, but **any move chosen is replaced
        /// with "cannot move"** — so a decider that keeps picking moves here achieves nothing.
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

// ── Status ───────────────────────────────────────────────────────────────────────────────────────

view! {
    /// One badge and whether it has been earned, in the order of
    /// [`Badge::ORDER`](crate::pokemon::badge::Badge::ORDER) — which is also the order of the sprites
    /// in `/api/badges.png`, so index `i` is the badge and the sprite.
    ///
    /// All eight are always reported, rather than only the earned ones: the UI draws all eight
    /// either way, gyms can legitimately be beaten out of order, and a name beside each one saves
    /// the client from carrying its own copy of the list.
    pub struct BadgeView {
        pub name: String,
        pub earned: bool,
    }
}

view! {
    /// One party slot on the status panel: enough for a sprite, a name and a health bar.
    ///
    /// ⚠️ **`dex` is a number, not an image.** The sprite is 3 KB of PNG and the heartbeat is sent
    /// several times a second; what rides the wire is the Pokédex number, and the client fetches
    /// `/api/pokemon/{dex}/front.png` once and lets the browser cache it for ever (the endpoint is
    /// `immutable` — it is a function of the cartridge).
    pub struct PartyMonView {
        /// What the player calls it, which is the species name in upper case unless they renamed it.
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
        /// The name on the save, which is whoever the run was started for — `GB_MODEL` shortened to
        /// the seven characters Gen 1 allows, or `HUMAN`, or a random draw. Not the same thing as
        /// the header's `model`: a resume keeps the name it was given, so a process restarted under
        /// a different `GB_MODEL` shows one of each, and that difference is worth being able to see.
        pub trainer: String,
        /// `wPlayerID`. Sent as the number and formatted by the client, because the game itself
        /// prints it five digits wide with leading zeroes (`PrintNumber`, `LEADING_ZEROES | 2, 5`)
        /// and that is what a player recognises as their ID.
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
