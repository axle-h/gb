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
use crate::pokemon::policy::battle_options;
use crate::pokemon::status::PokemonStatus;
use crate::pokemon::symbols::pokered_symbols;
use crate::pokemon::symbols::DmgPointerRead;
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

view! {
    /// Who the player is and what they have to show for it.
    pub struct TrainerView {
        pub name: String,
        pub rival_name: String,
        /// Badge names in the order the game awards them, e.g. `["BoulderBadge", "CascadeBadge"]`.
        pub badges: Vec<String>,
        pub badge_count: u32,
        pub money: u32,
        pub coins: u16,
        pub has_pokedex: bool,
        pub pokedex_owned: usize,
        pub pokedex_seen: usize,
        /// `HH:MM:SS` of in-game play time. Saturates at 255:59:59, as the game itself does — pokered
        /// sets `wPlayTimeMaxed` and stops counting.
        pub playtime: String,
        pub playtime_maxed: bool,
    }
}

/// `HH:MM:SS` of in-game play time. Saturates at 255:59:59, as the game itself does.
///
/// Public because it is the one line of [`TrainerView`] a turn request wants without the other
/// eleven — see [`crate::llm::prompt::ApiSnapshot`].
pub fn playtime(api: &PokemonApi<'_>) -> String {
    let mmu = api.mmu();
    let hours = mmu.read_pointer(&pokered_symbols::wPlayTimeHours);
    let minutes = mmu.read_pointer(&pokered_symbols::wPlayTimeMinutes);
    let seconds = mmu.read_pointer(&pokered_symbols::wPlayTimeSeconds);
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

pub fn trainer(state: &GameState, api: &PokemonApi<'_>) -> TrainerView {
    let mmu = api.mmu();
    TrainerView {
        name: state.name.to_default_string(),
        rival_name: state.rival_name.to_default_string(),
        badges: state.badges.iter_names().map(|(name, _)| name.to_string()).collect(),
        badge_count: state.badges.bits().count_ones(),
        money: state.money,
        coins: state.coins,
        has_pokedex: state.has_pokedex,
        pokedex_owned: state.pokedex_owned.species().len(),
        pokedex_seen: state.pokedex_seen.species().len(),
        playtime: playtime(api),
        playtime_maxed: mmu.read_pointer(&pokered_symbols::wPlayTimeMaxed) != 0,
    }
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

/// What each character in [`MapView::grid`] means. Kept beside the grid in every response rather
/// than in a system prompt: the model reads the two together, and a legend that can drift out of
/// sync with `impl Display for MetaTileMap` is worse than none.
pub const MAP_LEGEND: &[(char, &str)] = &[
    ('P', "the player"),
    ('_', "walkable"),
    ('O', "obstacle"),
    ('X', "water — needs Surf"),
    ('S', "a sprite (person, item ball, boulder); see `sprites`"),
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
    pub struct SpriteView {
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
    /// An action the agent can execute right now, as the policy would receive it. `steps` is the
    /// route length rather than the route itself — the button sequence is the agent's business and
    /// forty `Up`/`Down` tokens tell a decider nothing that "23 steps" does not.
    pub struct ActionView {
        pub destination: Point,
        pub tile: String,
        pub steps: usize,
    }
}

view! {
    pub struct MapView {
        pub map: String,
        pub position: Point,
        pub facing: String,
        pub width: usize,
        pub height: usize,
        /// One line per row, `width` characters each. See [`MAP_LEGEND`].
        pub grid: Vec<String>,
        pub legend: Vec<(char, String)>,
        pub sprites: Vec<SpriteView>,
        pub warps: Vec<WarpView>,
        pub connections: Vec<String>,
        /// Everything reachable from where the player is standing, which is the menu a decider
        /// actually chooses from. An empty list means the player is boxed in.
        pub actions: Vec<ActionView>,
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
    // `impl Display for MetaTileMap` renders exactly this grid and is what every existing
    // dump/probe prints, so the model and the debugging output see the same picture. It ends with a
    // blank line (a `writeln!` after the row loop) — trimmed rather than `take(height)`d, so that a
    // future change to the renderer shows up as a row-count mismatch instead of being cut off.
    let grid = format!("{map}").trim_end_matches('\n').lines().map(str::to_string).collect();
    MapView {
        map: format!("{}", map.map),
        position: map.player_position.into(),
        facing: format!("{:?}", map.player_direction),
        width: map.width,
        height: map.height,
        grid,
        legend: MAP_LEGEND.iter().map(|(c, meaning)| (*c, meaning.to_string())).collect(),
        // Hidden sprites are absent from the map the player sees; reporting them would invite the
        // model to try to talk to someone who is not there.
        sprites: map.sprites.iter().filter(|s| !s.hidden).map(|s| SpriteView {
            index: s.index,
            name: s.name.to_string(),
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
        actions: {
            // Sorted for the same reason as `warps` below: `actions()` walks `warp_targets`, a
            // `HashSet`, so the natural order changes between two calls on an unchanged map — which
            // reads to a model as the world having moved under it.
            let mut actions: Vec<ActionView> = map.actions().into_iter().map(|action| ActionView {
                destination: action.destination.into(),
                tile: format!("{}", action.tile),
                steps: action.route.len(),
            }).collect();
            actions.sort_by(|a, b| (a.destination.y, a.destination.x, &a.tile)
                .cmp(&(b.destination.y, b.destination.x, &b.tile)));
            actions
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
    pub struct WorldGraphEdgeView {
        pub from: Point,
        pub to_map: String,
        pub to_position: Point,
        /// `"Warp"` or `"Connection"`.
        pub kind: String,
    }
}

view! {
    pub struct WorldGraphNodeView {
        pub map: String,
        /// Where the player entered this map. One map can hold several nodes, one per entrance.
        pub entry: Point,
        pub edges: Vec<WorldGraphEdgeView>,
    }
}

view! {
    /// ⚠️ **Only maps the player has physically stood on.** The graph is built incrementally by
    /// [`WorldGraph::observe`], so an absent map means "not visited yet", never "does not exist" or
    /// "unreachable". It answers "how do I get back to somewhere I have been", not "where is X".
    pub struct WorldGraphView {
        pub map_count: usize,
        pub edge_count: usize,
        pub nodes: Vec<WorldGraphNodeView>,
    }
}

pub fn world_graph(graph: &WorldGraph) -> WorldGraphView {
    let mut nodes: Vec<WorldGraphNodeView> = graph.nodes().into_iter()
        .map(|((map, entry), edges)| WorldGraphNodeView {
            map: format!("{map}"),
            entry: entry.into(),
            edges: edges.into_iter().map(|edge| WorldGraphEdgeView {
                from: edge.from.location.into(),
                to_map: format!("{}", edge.to.map),
                to_position: edge.to.location.into(),
                kind: format!("{:?}", edge.kind),
            }).collect(),
        }).collect();
    // Same reason as `warps`: a stable order, so an unchanged world reads as unchanged.
    nodes.sort_by(|a, b| (&a.map, a.entry.y, a.entry.x).cmp(&(&b.map, b.entry.y, b.entry.x)));
    WorldGraphView { map_count: graph.map_count(), edge_count: graph.edge_count(), nodes }
}

// ── Battle ───────────────────────────────────────────────────────────────────────────────────────

view! {
    pub struct BattleSideView {
        pub species: String,
        pub level: u8,
        pub hp: u16,
        pub max_hp: u16,
        pub status: String,
        /// Slot of a move Disable has locked out this battle. The game bounces straight back to the
        /// move menu if it is chosen, so a decider that ignores this can loop forever.
        pub disabled_move_slot: Option<u8>,
        pub moves: Vec<MoveView>,
    }
}

view! {
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
        /// Every legal action, as the policy would be offered it.
        pub options: Vec<String>,
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
        options: battle_options(state).unwrap_or_default()
            .into_iter().map(|action| format!("{action}")).collect(),
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
