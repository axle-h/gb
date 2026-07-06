use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver};
use rand::prelude::StdRng;
use rand::seq::IteratorRandom;
use rand::SeedableRng;
use crate::pokemon::GameState;
use crate::pokemon::actions::OverworldAction;
use crate::geometry::Point8;
use crate::pokemon::badge::Badge;
use crate::pokemon::bag::BagItem;
use crate::pokemon::battle::{BattleAction, BattleType};
use crate::pokemon::damage::{expected_damage, is_damaging_move, pick_best_move};
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::data::PokemonNamePicker;
use crate::pokemon::tile::MetaTile;
pub use crate::pokemon::item::ItemId;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::world_graph::WorldGraph;

/// Non-blocking policy interface.
///
/// All methods return `Option<_>`. `None` means "not ready yet — ask again next frame".
/// This keeps the game loop running while the policy waits for input.
pub trait Policy {
    /// Choose the next overworld action.
    ///
    /// `world_graph` is the agent's **incrementally-built** map graph — it only contains
    /// sections the player has already physically visited (accurate, sprite-resolved). Use it
    /// for backtracking to known places (e.g. heal-return to a Pokémon Center); forward travel
    /// into not-yet-visited maps must be scripted with explicit [`PolicyStep::EnterMap`] steps.
    fn pick_overworld_action(&mut self, state: &GameState, world_graph: &WorldGraph) -> Option<OverworldAction>;
    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction>;

    /// Called when the nickname-entry screen opens for `species`.
    ///
    /// - `None`          → not ready yet; will be called again next frame.
    /// - `Some(None)`    → decline a nickname; the game keeps the default species name.
    /// - `Some(Some(s))` → give this nickname (up to 10 characters, A-Z / a-z / 0-9 / common punctuation).
    fn pick_nickname(&mut self, _species: PokemonSpecies) -> Option<Option<String>> {
        Some(None) // default: keep the default species name
    }

    /// Called when the mart's Buy/Sell/Quit menu first appears.
    ///
    /// - `None`       → not ready yet; will be called again next frame.
    /// - `Some(None)` → do not buy anything.
    /// - `Some(Some(item))` → buy the item.
    fn pick_mart_purchase(&mut self, _state: &GameState) -> Option<Option<BagItem>> {
        Some(None) // default: open the mart but buy nothing
    }

    /// Called on the level-up "Which move should be forgotten?" prompt, when a Pokémon that already
    /// knows 4 moves would learn `new_move`. `current_moves` are the 4 known moves (slot order).
    ///
    /// - `None`             → not ready yet; asked again next frame.
    /// - `Some(None)`       → decline learning; keep the current four moves.
    /// - `Some(Some(slot))` → forget the move in `slot` (0-3) and learn `new_move`.
    fn pick_move_to_forget(&mut self, _current_moves: &[PokemonMove], _new_move: PokemonMoveName)
        -> Option<Option<usize>>
    {
        Some(None) // default: never drop an existing move
    }

    /// Called each idle overworld tick. Returns a non-walking field action to perform (e.g. teach an
    /// HM), or `None` to fall through to [`pick_overworld_action`].
    fn pick_field_move(&mut self, _state: &GameState) -> Option<FieldMove> {
        None
    }

    fn is_exhausted(&self) -> bool {
        false
    }

    /// Returns the number of steps remaining in the policy queue, if known.
    fn steps_remaining(&self) -> Option<usize> {
        None
    }

    /// Returns true if the current step is expected to run for a long time without
    /// advancing the queue (e.g. grinding levels or catching a Pokémon). Used by the
    /// test fixture to exempt these steps from the short stall-detection threshold.
    fn current_step_is_long_running(&self) -> bool {
        false
    }
}

// ── Random (always-ready) ─────────────────────────────────────────────────────

#[derive(Default)]
pub struct RandomPolicy;

impl Policy for RandomPolicy {
    fn pick_overworld_action(&mut self, state: &GameState, _world_graph: &WorldGraph) -> Option<OverworldAction> {
        state.map.actions().into_iter().choose(&mut rand::rng())
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        battle_options(state)?.into_iter().choose(&mut rand::rng())
    }
}

// ── Console (human-driven, non-blocking) ─────────────────────────────────────

/// Displays a numbered menu, then reads the user's choice from stdin on a
/// background thread so the game loop is never blocked.
pub struct ConsolePolicy {
    overworld_rx:   Option<Receiver<usize>>,
    battle_rx:      Option<Receiver<usize>>,
    nickname_rx:    Option<Receiver<Option<String>>>,
    ow_menu_shown:  bool,
    btl_menu_shown: bool,
    /// Tiles shown when the last overworld menu was displayed; used to match
    /// the user's selection by destination rather than by list index, since
    /// the action list can reorder between display and selection.
    ow_shown_tiles: Vec<MetaTile>,
}

impl Default for ConsolePolicy {
    fn default() -> Self {
        Self {
            overworld_rx:   None,
            battle_rx:      None,
            nickname_rx:    None,
            ow_menu_shown:  false,
            btl_menu_shown: false,
            ow_shown_tiles: vec![],
        }
    }
}

impl Policy for ConsolePolicy {
    fn pick_overworld_action(&mut self, state: &GameState, _world_graph: &WorldGraph) -> Option<OverworldAction> {
        let actions = state.map.actions();
        if actions.is_empty() { return None; }

        if !self.ow_menu_shown || self.overworld_rx.is_none() {
            println!("\nYou are on {} at {}. Available actions:", state.map.map, state.map.player_position);
            for (i, a) in actions.iter().enumerate() {
                println!("  {}. {}", i + 1, a);
            }
            let max = actions.len();
            // Cache the destinations so we can match by tile, not index.
            self.ow_shown_tiles = actions.iter().map(|a| a.tile.clone()).collect();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                loop {
                    print!("Pick (1-{max}): ");
                    io::stdout().flush().ok();
                    let mut line = String::new();
                    if io::stdin().read_line(&mut line).is_err() { break; }
                    if let Ok(n) = line.trim().parse::<usize>() {
                        if n >= 1 && n <= max { tx.send(n).ok(); break; }
                    }
                    println!("Invalid.");
                }
            });
            self.overworld_rx = Some(rx);
            self.ow_menu_shown = true;
        }

        if let Ok(n) = self.overworld_rx.as_ref().unwrap().try_recv() {
            let chosen_tile = self.ow_shown_tiles.get(n - 1).cloned();
            self.overworld_rx  = None;
            self.ow_menu_shown = false;
            self.ow_shown_tiles.clear();
            if let Some(tile) = chosen_tile {
                return actions.into_iter().find(|a| a.tile == tile);
            }
            return None;
        }
        None
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        if !self.btl_menu_shown || self.battle_rx.is_none() {
            let battle_state = state.battle.as_ref()?;

            println!("\n═══ BATTLE ═══");
            println!("Enemy:  {:?} Lv.{}  HP {}/{}  {}",
                battle_state.enemy.species, battle_state.enemy.level,
                battle_state.enemy.current_hp, battle_state.enemy.stats.hp,
                battle_state.enemy.status);
            println!("Player: {:?} Lv.{}  HP {}/{}  {}",
                battle_state.player.species, battle_state.player.level,
                battle_state.player.current_hp, battle_state.player.stats.hp,
                battle_state.player.status);
            println!("\nBattle actions:");

            let opts = battle_options(state)?;
            for (i, battle_action) in opts.iter().enumerate() {
                println!("  {}. {}", i + 1, battle_action);
            }

            let max = opts.len();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                loop {
                    print!("Pick (1-{max}): ");
                    io::stdout().flush().ok();
                    let mut line = String::new();
                    if io::stdin().read_line(&mut line).is_err() { break; }
                    if let Ok(n) = line.trim().parse::<usize>() {
                        if n >= 1 && n <= max { tx.send(n).ok(); break; }
                    }
                    println!("Invalid.");
                }
            });
            self.battle_rx    = Some(rx);
            self.btl_menu_shown = true;
        }

        if let Ok(n) = self.battle_rx.as_ref().unwrap().try_recv() {
            self.battle_rx     = None;
            self.btl_menu_shown = false;
            let mut opts = battle_options(state)?;
            return Some(opts.remove(n - 1));
        }
        None
    }

    fn pick_nickname(&mut self, species: PokemonSpecies) -> Option<Option<String>> {
        if self.nickname_rx.is_none() {
            println!("\nGive a nickname to {}?", species);
            println!("  Enter a nickname (up to 10 chars), or press Enter to keep the default.");
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                print!("> ");
                io::stdout().flush().ok();
                let mut line = String::new();
                if io::stdin().read_line(&mut line).is_err() { return; }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    tx.send(None).ok();
                } else {
                    tx.send(Some(trimmed.to_string())).ok();
                }
            });
            self.nickname_rx = Some(rx);
        }

        if let Ok(decision) = self.nickname_rx.as_ref().unwrap().try_recv() {
            self.nickname_rx = None;
            return Some(decision);
        }
        None
    }
}


pub(crate) fn battle_options(state: &GameState) -> Option<Vec<BattleAction>> {
    let battle_state = state.battle.as_ref()?;

    // Safari Zone battles have their own menu (no FIGHT/PKMN/ITEM). Offer all four Safari options so a
    // future LLM-driven policy can actually hunt; the deterministic policy just RUNs (below).
    if battle_state.battle_type == BattleType::Safari {
        return Some(vec![
            BattleAction::SafariBall,
            BattleAction::SafariBait,
            BattleAction::SafariRock,
            BattleAction::Run,
        ]);
    }

    let mut opts = battle_state.player.available_battle_moves();

    for (i, item) in state.bag.iter().enumerate() {
        opts.push(BattleAction::UseItem { slot: i as u8, item: item.clone() });
    }

    for (i, pokemon) in state.pokemon.iter().enumerate() {
        if i == battle_state.active_party_slot as usize { continue; }
        if pokemon.current_hp == 0 { continue; }
        opts.push(BattleAction::SwitchPokemon { slot: i as u8, pokemon: pokemon.summary() });
    }

    if battle_state.battle_type == BattleType::Wild {
        opts.push(BattleAction::Run);
    }

    Some(opts)
}

/// Returns `true` total PP remaining across all damaging moves dips below ≤20% of its maximum PP remaining.
fn all_damaging_moves_low_pp(actions: &[BattleAction]) -> bool {
    const MIN_PP_PCT: f32 = 0.2;

    let mut total_damaging_pp = 0;
    let mut total_max_pp = 0;

    for action in actions.iter() {
        if let BattleAction::Fight { battle_move, .. } = action {
            if is_damaging_move(battle_move.name) {
                total_damaging_pp += battle_move.pp as usize;
                total_max_pp += battle_move.name.metadata().pp as usize;
            }
        }
    }

    if total_max_pp == 0 {
        // No damaging moves, so we can't say they're all low on PP.
        return false;
    }

    (total_damaging_pp as f32 / total_max_pp as f32) < MIN_PP_PCT
}


#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PolicyStep {
    Goto { map: Map, strict: bool },
    /// Take exactly one explicit map transition: walk to and use the warp/connection on the
    /// current map that leads to `to_map` (matching the raw landing `to_position` when given, to
    /// disambiguate maps with several warps to the same target — e.g. Mt Moon). This is how the
    /// deterministic policy crosses not-yet-explored mazes; it is a **hard requirement** — if the
    /// transition is not reachable on the current map the agent stalls (proving under-specification)
    /// rather than silently rerouting over an inaccurate pre-resolved graph.
    EnterMap { to_map: Map, to_position: Option<Point8> },
    /// Walk to and interact with a visible sprite by name.
    Interact(MapSprite),
    /// Walk to the map's PC tile, face it, and press A (e.g. Bill's cell-separator PC). The PC is a
    /// hidden-object tile, not a sprite; `MetaTileMap::pc_locations` supplies its coordinate. Should
    /// be scripted only when using the PC is valid (e.g. after Bill's Pokémon enters the machine).
    UsePc { map: Map },
    /// Walk to and pick up an item sprite (a Poké Ball on the ground), staying on this step until
    /// the sprite is gone. Unlike [`Interact`], this does **not** pop after issuing a single walk:
    /// picking up an item can be interrupted (e.g. the Mt Moon fossil area triggers the Super Nerd
    /// battle at the only approach tile), so the step persists and re-issues the walk after each
    /// interruption until the item sprite disappears from the map. Also used to clear item-sprite
    /// blockers that plug a corridor (collecting one Mt Moon fossil opens the exit passage).
    CollectItem(MapSprite),
    DefeatGymLeader { leader: MapSprite, badge: Badge },
    /// Walk in grass and throw Pokéballs until a Pokémon is caught.
    CatchPokemon { species: PokemonSpecies, on_map: Map },
    /// Walk in grass until the leading party member reaches at least this level.
    GrindUntilLevel { target_level: u8, on_map: Map },
    /// Buy item from the currently open Pokémart (must follow an Interact with the clerk).
    BuyFromMart { map: Map, item: BagItem },
    /// Teach an HM/TM `item` (e.g. HM01 Cut) to the party member in `target_slot`, from the
    /// overworld. Drives the START → ITEM → use → choose-Pokémon menus; the move-replace menu (if the
    /// mon already knows 4 moves) is handled by the global forget-move handler. Persists until the
    /// target knows the move.
    TeachMove { item: ItemId, target_slot: u8 },
    /// Cut down a tree blocking the way on `map` (requires Cut + the Cascade Badge). Routes to face a
    /// `MetaTile::CutTree`, then uses the Cut field move. Persists until no reachable tree remains.
    CutTree { map: Map },
    /// Solve the Vermilion Gym trash-can switch puzzle: check the first switch can, then the second,
    /// unlocking the door to Lt. Surge. The correct cans are read from RAM (`GameState::trash_cans`)
    /// so the agent goes straight to them and never triggers a reset. Persists until the 2nd lock is
    /// open. Only meaningful on `Map::VermilionGym`.
    SolveTrashCans,
    /// Walk to face the hidden switch/poster BG-event tile at `at` on `map` and press A, until doing so
    /// reveals a passage — a reachable warp/connection to `reveals` appears (e.g. the Celadon Game
    /// Corner poster flips a switch that opens the staircase down to the Rocket Hideout). The tile is a
    /// `bg_event`, not a sprite, so `Interact` can't target it. Idempotent: re-pressing after the reveal
    /// is harmless; the step pops once `reveals` is reachable.
    FlipSwitch { map: Map, at: Point8, reveals: Map },
    /// Inside an elevator room, use the floor panel to travel to the floor at menu index `floor`.
    /// Faces the panel bg-event, opens the floor list-menu, navigates the cursor to `floor`, confirms,
    /// then steps back onto the elevator warp (whose destination the menu redirected at runtime) — the
    /// step completes when the resulting warp changes the map. Used for the Rocket Hideout elevator
    /// (needs the Lift Key) to reach Giovanni's split-off B4F room.
    UseElevator { panel: Point8, floor: u8 },
    /// Face the sprite `target`, then **use** the bag item `item` on it from the field (START → ITEM →
    /// select → USE). Used for the Poké Flute on a road-blocking Snorlax: the item's effect starts a
    /// battle, which the normal battle handler wins; the step completes once `target` is gone.
    UseFieldItem { item: ItemId, target: MapSprite },
    /// Face the vending-machine bg-event at `at` and press A to buy `drink` (the machine's menu opens
    /// with the cheapest drink at the cursor, so A-mashing selects it). Persists until `drink` is in the
    /// bag. Used for the Celadon Mart roof drink needed to pass the Saffron gate guards.
    UseVendingMachine { at: Point8, drink: ItemId },
}

/// A non-walking overworld action the agent performs directly (opening menus / using field moves),
/// requested by the policy when the corresponding queue step is at the front.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FieldMove {
    /// Teach `item` (an HM/TM) to the party member in `target_slot` via the bag.
    TeachMove { item: ItemId, target_slot: u8 },
    /// Use the Cut field move on the tree the player is currently facing.
    CutTree,
    /// Walk to the trash can at `target` and press A to check it for a hidden switch.
    CheckTrashCan { target: crate::geometry::Point8 },
    /// Drive the elevator floor menu (panel at `panel`) to select menu index `floor`, then ride the
    /// redirected warp out. Done when the map changes (we've left the elevator room).
    UseElevator { panel: crate::geometry::Point8, floor: u8 },
    /// Face the sprite at `target`, then use bag `item` on it (START → ITEM → select → USE). The
    /// item's field effect (e.g. the Poké Flute waking a Snorlax) does the rest.
    UseFieldItem { item: ItemId, target: crate::geometry::Point8 },
}

/// The move an HM item teaches (HM01 Cut … HM05 Flash), used to check whether a mon already knows it.
pub fn hm_move(item: ItemId) -> Option<PokemonMoveName> {
    match item {
        ItemId::Hm01Cut => Some(PokemonMoveName::Cut),
        ItemId::Hm02Fly => Some(PokemonMoveName::Fly),
        ItemId::Hm03Surf => Some(PokemonMoveName::Surf),
        ItemId::Hm04Strength => Some(PokemonMoveName::Strength),
        ItemId::Hm05Flash => Some(PokemonMoveName::Flash),
        _ => None,
    }
}

impl PolicyStep {
    pub const fn goto(map: Map) -> Self {
        Self::Goto { map, strict: true }
    }

    pub const fn soft_goto(map: Map) -> Self {
        Self::Goto { map, strict: false }
    }

    /// Explicit single forward map transition (any warp/connection to `map`).
    pub const fn enter(map: Map) -> Self {
        Self::EnterMap { to_map: map, to_position: None }
    }

    /// Explicit forward transition to `map`, disambiguated by the raw landing `to_position`.
    pub const fn enter_at(map: Map, x: u8, y: u8) -> Self {
        Self::EnterMap { to_map: map, to_position: Some(Point8 { x, y }) }
    }

    /// The explicit Mt Moon crossing (1F west entrance → Route 4 east exit), including the fossil
    /// chokepoint. Requires standing in Mt Moon 1F. See `mt_moon_traversal` doc in the tests.
    pub fn mt_moon_traversal() -> Vec<Self> { vec![
        Self::enter_at(Map::MtMoonB1F, 5, 5),
        Self::enter_at(Map::MtMoonB2F, 21, 17),
        Self::CollectItem(MapSprite::MTMOONB2F_HELIX_FOSSIL),
        Self::enter_at(Map::MtMoonB1F, 23, 3),
        Self::enter(Map::Route4),
        Self::enter(Map::CeruleanCity),
    ] }

    /// The Bill's-House SS-Ticket sub-sequence (pokered `scripts/BillsHouse.asm`), assuming the
    /// agent is already inside `BillsHouse`: talk to Bill's Pokémon (A-mash picks the default YES →
    /// it walks into the cell separator) → use the PC (runs the Cell Separation System, Bill exits
    /// the machine) → talk to Bill for the SS Ticket. Bill's exit is a ~1-2s scripted walk, so an
    /// `Interact` issued mid-script aborts (reason `Script`); retry a few times so one lands after he
    /// settles (extra talks after the ticket is received are harmless — same text, no re-give).
    pub fn bill_ss_ticket_steps() -> Vec<Self> {
        let mut steps = vec![
            Self::Interact(MapSprite::BILLSHOUSE_BILL_POKEMON),
            Self::UsePc { map: Map::BillsHouse },
        ];
        steps.extend(std::iter::repeat(Self::Interact(MapSprite::BILLSHOUSE_BILL1)).take(8));
        steps
    }

    /// Heal the party at the Vermilion Pokémon Center and return to Vermilion City.
    fn heal_at_vermilion() -> Vec<Self> {
        vec![
            Self::enter(Map::VermilionPokecenter),
            Self::Interact(MapSprite::VERMILIONPOKECENTER_NURSE),
            Self::enter(Map::VermilionCity),
        ]
    }

    /// Board the S.S. Anne (from Vermilion City, SS Ticket in the bag), defeat every trainer in the
    /// ship's cabins to level the party, beat the rival guarding the captain's door, and receive
    /// **HM01 Cut** from the captain.
    ///
    /// Cabins are disconnected rooms within the `*Rooms` maps, each reached by a distinct warp
    /// landing (`enter_at` disambiguates); we visit them one by one and `Interact` each trainer
    /// (walking up + A starts a trainer battle). There is **no Pokémon Center on the ship**, so each
    /// floor is a self-contained heal → board → sweep → disembark cycle that returns to Vermilion —
    /// the lone starter would otherwise be worn down by attrition. Coordinates are decoded from
    /// pokered `data/maps/objects/SSAnne*Rooms.asm`. Floors are ordered to level the party as high as
    /// possible before the rival (a single 6-Pokémon battle with no mid-battle healing).
    pub fn ss_anne_steps() -> Vec<Self> {
        let mut s = vec![];

        // ── 1F cabins (4 trainers) ──
        s.extend(Self::heal_at_vermilion());
        s.extend([Self::enter(Map::VermilionDock), Self::enter(Map::SSAnne1F)]);
        s.extend([
            Self::enter_at(Map::SSAnne1FRooms, 0, 0),   Self::Interact(MapSprite::SSANNE1FROOMS_GENTLEMAN1), Self::enter(Map::SSAnne1F),
            Self::enter_at(Map::SSAnne1FRooms, 10, 0),  Self::Interact(MapSprite::SSANNE1FROOMS_GENTLEMAN2), Self::enter(Map::SSAnne1F),
            Self::enter_at(Map::SSAnne1FRooms, 10, 10), Self::Interact(MapSprite::SSANNE1FROOMS_YOUNGSTER),
                                                        Self::Interact(MapSprite::SSANNE1FROOMS_COOLTRAINER_F), Self::enter(Map::SSAnne1F),
        ]);
        s.extend([Self::enter(Map::VermilionDock), Self::enter(Map::VermilionCity)]); // disembark

        // ── B1F cabins (6 trainers) ──
        s.extend(Self::heal_at_vermilion());
        s.extend([Self::enter(Map::VermilionDock), Self::enter(Map::SSAnne1F), Self::enter(Map::SSAnneB1F)]);
        s.extend([
            Self::enter_at(Map::SSAnneB1FRooms, 2, 5),  Self::Interact(MapSprite::SSANNEB1FROOMS_SAILOR5),
                                                        Self::Interact(MapSprite::SSANNEB1FROOMS_FISHER), Self::enter(Map::SSAnneB1F),
            Self::enter_at(Map::SSAnneB1FRooms, 12, 5), Self::Interact(MapSprite::SSANNEB1FROOMS_SAILOR3), Self::enter(Map::SSAnneB1F),
            Self::enter_at(Map::SSAnneB1FRooms, 22, 5), Self::Interact(MapSprite::SSANNEB1FROOMS_SAILOR4), Self::enter(Map::SSAnneB1F),
            Self::enter_at(Map::SSAnneB1FRooms, 2, 15), Self::Interact(MapSprite::SSANNEB1FROOMS_SAILOR1),
                                                        Self::Interact(MapSprite::SSANNEB1FROOMS_SAILOR2), Self::enter(Map::SSAnneB1F),
        ]);
        s.extend([Self::enter(Map::SSAnne1F), Self::enter(Map::VermilionDock), Self::enter(Map::VermilionCity)]); // disembark

        // ── 2F cabins (4 trainers) + Bow (2 trainers, via 3F) ──
        s.extend(Self::heal_at_vermilion());
        s.extend([Self::enter(Map::VermilionDock), Self::enter(Map::SSAnne1F), Self::enter(Map::SSAnne2F)]);
        s.extend([
            Self::enter_at(Map::SSAnne2FRooms, 12, 5), Self::Interact(MapSprite::SSANNE2FROOMS_GENTLEMAN1),
                                                       Self::Interact(MapSprite::SSANNE2FROOMS_FISHER), Self::enter(Map::SSAnne2F),
            Self::enter_at(Map::SSAnne2FRooms, 2, 15), Self::Interact(MapSprite::SSANNE2FROOMS_GENTLEMAN2),
                                                       Self::Interact(MapSprite::SSANNE2FROOMS_COOLTRAINER_F), Self::enter(Map::SSAnne2F),
        ]);
        // Bow: SSAnne2F → SSAnne3F → SSAnneBow (one open room, two sailors). Party is strong by now.
        s.extend([
            Self::enter(Map::SSAnne3F), Self::enter(Map::SSAnneBow),
            Self::Interact(MapSprite::SSANNEBOW_SAILOR2), Self::Interact(MapSprite::SSANNEBOW_SAILOR3),
            Self::enter(Map::SSAnne3F), Self::enter(Map::SSAnne2F),
        ]);
        s.extend([Self::enter(Map::SSAnne1F), Self::enter(Map::VermilionDock), Self::enter(Map::VermilionCity)]); // disembark

        // ── Rival + Captain (HM01) ── (heal first — the rival is 6 Pokémon in one battle)
        s.extend(Self::heal_at_vermilion());
        s.extend([Self::enter(Map::VermilionDock), Self::enter(Map::SSAnne1F), Self::enter(Map::SSAnne2F)]);
        s.push(Self::enter(Map::SSAnneCaptainsRoom)); // rival battle triggers on approach to the (36,4) warp
        s.extend(std::iter::repeat(Self::Interact(MapSprite::SSANNECAPTAINSROOM_CAPTAIN)).take(4));
        // ── Disembark back to Vermilion (after HM01 the ship departs on the way out of the dock) ──
        s.extend([
            Self::enter(Map::SSAnne2F), Self::enter(Map::SSAnne1F),
            Self::enter(Map::VermilionDock), Self::enter(Map::VermilionCity),
        ]);
        s
    }

    /// From Cerulean City (post-Cascade): fetch the **SS Ticket** from Bill, then cross to Vermilion
    /// City via the **trashed-house terrace bridge** + Underground Path (Route 5 → 6). The trashed
    /// house is the only way between Cerulean's split terraces: its back door lands in the Route-5
    /// terrace (`enter_at(CeruleanCity, 27, 9)` — front door ~27,11 does not reach it). See
    /// `can_reach_vermilion`. Bill's guard on Route 25 clears once you meet him, opening the bridge.
    pub fn cerulean_to_vermilion_steps() -> Vec<Self> {
        let mut steps = vec![
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::Route24),
            Self::enter(Map::Route25),
            Self::enter(Map::BillsHouse),
        ];
        steps.extend(Self::bill_ss_ticket_steps());
        steps.extend([
            Self::enter(Map::Route25),
            Self::enter(Map::Route24),
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::CeruleanTrashedHouse),   // front door (main terrace, ~27,11)
            Self::enter_at(Map::CeruleanCity, 27, 9), // back door lands in the Route-5 terrace
            Self::enter(Map::Route5),
            Self::enter(Map::UndergroundPathRoute5),
            Self::enter(Map::UndergroundPathNorthSouth),
            Self::enter(Map::UndergroundPathRoute6),
            Self::enter(Map::Route6),
            Self::enter(Map::VermilionCity),
        ]);
        steps
    }

    /// Thunder Badge (from Vermilion City after the S.S. Anne, with **HM01 Cut** in the bag): teach Cut
    /// to the starter, cut the tree sealing the gym enclosure, solve the two-switch **trash-can
    /// puzzle** (which unlocks the door), then beat Lt. Surge. All via the real UI — see
    /// `can_get_thunder_badge` (integrated) and `can_teach_cut` / `can_cut_gym_tree` /
    /// `can_beat_lt_surge` (focused). `SolveTrashCans` must precede `DefeatGymLeader`: the door is
    /// shut (Surge unreachable) until both switches are hit.
    pub fn thunder_badge_steps() -> Vec<Self> {
        let mut s = Self::heal_at_vermilion();
        s.extend([
            Self::TeachMove { item: ItemId::Hm01Cut, target_slot: 0 },
            Self::CutTree { map: Map::VermilionCity },
            Self::enter(Map::VermilionGym),
            Self::SolveTrashCans,
            Self::DefeatGymLeader { leader: MapSprite::VERMILIONGYM_LT_SURGE, badge: Badge::ThunderBadge },
        ]);
        s
    }

    /// Head back from Vermilion (just after the Thunder Badge, standing inside the gym) to Cerulean
    /// City, reusing the Underground Path in reverse. Saffron's south gate (Route 6) is guard-blocked,
    /// so the Underground Path (Route 5 ↔ Route 6) is the only legal way north. Exiting the gym drops
    /// the player into the Cut-tree enclosure (the tree regrows on re-entering the map), so cut it
    /// again before reaching the rest of the city. Heal at both ends of the trek.
    pub fn back_to_cerulean_steps() -> Vec<Self> {
        let mut s = vec![
            Self::enter(Map::VermilionCity), // exit the gym into the Cut-tree enclosure
            Self::CutTree { map: Map::VermilionCity },
        ];
        s.extend(Self::heal_at_vermilion());
        s.extend([
            Self::enter(Map::Route6),
            Self::enter(Map::UndergroundPathRoute6),
            Self::enter(Map::UndergroundPathNorthSouth),
            Self::enter(Map::UndergroundPathRoute5),
            Self::enter(Map::Route5),
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
            Self::enter(Map::CeruleanCity),
        ]);
        s
    }

    /// The Rock Tunnel warp-maze crossing (Route 10 north entrance → Route 10 south exit), discovered
    /// offline by `discover_rock_tunnel_path` (ExplorerPolicy). Assumes the agent stands on Route 10
    /// having just come from Route 9. No Flash needed — the agent routes from RAM tile collision, not
    /// the darkened screen.
    pub fn rock_tunnel_traversal() -> Vec<Self> { vec![
        // North entrance → a 4-hop 1F↔B1F chain → south exit. Warp pairs (from ROM + real-engine
        // probing): each `enter_at` lands in a region whose only forward (unvisited) warp is the next.
        Self::enter_at(Map::RockTunnel1F, 15, 3),   // Route 10 north entrance
        Self::enter_at(Map::RockTunnelB1F, 33, 25),
        Self::enter_at(Map::RockTunnel1F, 5, 3),
        Self::enter_at(Map::RockTunnelB1F, 23, 11),
        Self::enter_at(Map::RockTunnel1F, 37, 17),
        Self::enter_at(Map::Route10, 8, 53),        // south exit (→ Lavender)
    ] }

    /// Cerulean City (main terrace, post-Thunder) → Lavender Town. Route 9 (east) is on a separate
    /// Cerulean terrace reached via the **trashed-house back door** (27,9) — the same bridge used to
    /// reach Route 5/Vermilion. Route 9's west-entry pocket is sealed by a **Cut tree at (5,8)**; cut
    /// it to cross east. Then Route 10 → **Rock Tunnel** (warp maze) → Route 10 south → Lavender.
    pub fn cerulean_to_lavender_steps() -> Vec<Self> {
        let mut s = vec![
            Self::enter(Map::CeruleanTrashedHouse),   // main terrace front door
            Self::enter_at(Map::CeruleanCity, 27, 9), // back door → Route-9 terrace
            Self::enter(Map::Route9),
            Self::CutTree { map: Map::Route9 },        // cut the (5,8) tree boxing the west pocket
            Self::enter(Map::Route10),
            // Heal at the Rock Tunnel Pokémon Center (Route 10, at the tunnel mouth) before diving in:
            // the encounter-dense maze must be crossed in one uninterrupted push (a mid-tunnel
            // flee-to-heal or blackout can't resume the scripted warp chain), so enter at full HP/PP.
            // This also makes it the nearest heal-return target if PP still runs low mid-crossing.
            Self::enter(Map::RockTunnelPokecenter),
            Self::Interact(MapSprite::ROCKTUNNELPOKECENTER_NURSE),
            Self::enter(Map::Route10),
        ];
        s.extend(Self::rock_tunnel_traversal());
        s.extend([
            Self::enter(Map::LavenderTown),
            Self::enter(Map::LavenderPokecenter),
            Self::Interact(MapSprite::LAVENDERPOKECENTER_NURSE),
            Self::enter(Map::LavenderTown),
        ]);
        s
    }

    /// Lavender Town → Celadon City via the **Route 7–8 Underground Path** (all four Saffron gates
    /// demand a drink only sold in Celadon — a chicken/egg — so Saffron is bypassed). Linear tunnel,
    /// same building-tunnel-building shape as the Route 5–6 path already used: Lavender → Route 8 →
    /// `UndergroundPathRoute8` → `UndergroundPathWestEast` → `UndergroundPathRoute7` → Route 7 →
    /// Celadon City, then heal at the Celadon Center.
    pub fn lavender_to_celadon_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::Route8),
            Self::enter(Map::UndergroundPathRoute8),
            Self::enter(Map::UndergroundPathWestEast),
            Self::enter(Map::UndergroundPathRoute7),
            Self::enter(Map::Route7),
            Self::enter(Map::CeladonCity),
            Self::enter(Map::CeladonPokecenter),
            Self::Interact(MapSprite::CELADONPOKECENTER_NURSE),
            Self::enter(Map::CeladonCity),
        ]
    }

    /// Rainbow Badge (from Celadon City): the gym entrance is sealed by a row of trees, so cut them,
    /// enter, and beat Erika. `DefeatGymLeader` persists until the badge is won (self-heals on a
    /// blackout and re-routes through the grass-maze junior trainers). Erika's team is all Grass/Poison
    /// (Victreebel/Tangela/Vileplume ~lv24–29); Grass moves are resisted, so the starter leans on its
    /// Normal move (Cut/Body Slam) + level lead — the party is ~lv35+ Venusaur by now.
    pub fn celadon_rainbow_steps() -> Vec<Self> {
        vec![
            Self::CutTree { map: Map::CeladonCity },   // cut the trees sealing the gym entrance
            Self::enter(Map::CeladonGym),
            // The gym is a garden maze whose paths are blocked by real cuttable trees (GYM tileset
            // tile $50 — pokered `cut.asm`). Cut them to weave up to Erika (junior trainers engage by
            // LOS en route). `CutTree` persists until no reachable tree remains, so it clears each
            // chokepoint as the previous cut opens access to the next.
            Self::CutTree { map: Map::CeladonGym },
            Self::DefeatGymLeader { leader: MapSprite::CELADONGYM_ERIKA, badge: Badge::RainbowBadge },
        ]
    }

    /// From Celadon City (post-Erika, inside the gym) to inside the **Rocket Hideout** (B1F). Exit the
    /// gym — its entrance trees regrew on re-entry, so re-cut them — heal, walk to the **Game Corner**,
    /// beat the Rocket guarding the poster (he vanishes on defeat), flip the poster switch to open the
    /// hidden staircase, and descend. Getting the **Silph Scope** (needed for the Poké Flute) means
    /// crossing the hideout's spinner-tile floors + elevator to Giovanni — handled separately.
    pub fn rocket_hideout_entrance_steps() -> Vec<Self> {
        let mut s = vec![
            Self::enter(Map::CeladonCity),          // exit the gym into the (regrown) tree enclosure
            Self::CutTree { map: Map::CeladonCity }, // re-cut to reach the rest of the city
            Self::enter(Map::CeladonPokecenter),
            Self::Interact(MapSprite::CELADONPOKECENTER_NURSE),
            Self::enter(Map::CeladonCity),
            Self::enter(Map::GameCorner),
        ];
        // The Rocket stands on (9,5) blocking the poster at (9,4) — beat him (he vanishes on defeat,
        // freeing (9,5)), then flip the poster switch to open the hidden staircase and descend. A
        // single `Interact` (not retried): it pops the instant it issues the walk, so it never hangs
        // after the Rocket vanishes; the ensuing `FlipSwitch` waits out the battle and then flips.
        s.extend([
            Self::Interact(MapSprite::GAMECORNER_ROCKET),
            Self::FlipSwitch { map: Map::GameCorner, at: Point8 { x: 9, y: 4 }, reveals: Map::RocketHideoutB1F },
            Self::enter(Map::RocketHideoutB1F),
        ]);
        s
    }

    /// From inside the Rocket Hideout (B1F), descend the spinner floors B2F/B3F to B4F and get the
    /// **Lift Key**. B2F/B3F are **spinner-tile floors** (arrow tiles force a fixed slide, modelled in
    /// the BFS via `MetaTileMap::spinners`). B4F is split — the stairs land in a left room; beating
    /// Rocket 3 isn't enough, his **after-battle text** (a second talk) sets EVENT_ROCKET_DROPPED_LIFT_KEY
    /// and `ShowObject`s the Lift Key ball at (10,2). He stays put after defeat, so Interact him a few
    /// times (battle, then the reveal talk), then grab the key (`CollectItem` waits for the ball to
    /// appear — see `collect_item_seen`).
    pub fn lift_key_steps() -> Vec<Self> {
        let mut s = vec![
            Self::enter(Map::RocketHideoutB2F),
            Self::enter(Map::RocketHideoutB3F),
            Self::enter(Map::RocketHideoutB4F),
        ];
        s.extend(std::iter::repeat(Self::Interact(MapSprite::ROCKETHIDEOUTB4F_ROCKET3)).take(3));
        s.push(Self::CollectItem(MapSprite::ROCKETHIDEOUTB4F_LIFT_KEY));
        s
    }

    /// From inside the Rocket Hideout (B1F), get the **Silph Scope** (needed to see the Pokémon Tower
    /// ghosts → Poké Flute). First get the Lift Key (`lift_key_steps`), then take the **elevator** to
    /// Giovanni's split-off B4F room.
    ///
    /// Two runtime door blocks gate this (modelled via `MetaTileMap` door overlays so BFS avoids them
    /// until open): (1) the **B1F elevator door** stays shut until Rocket 5 is beaten — so we enter the
    /// elevator from **B2F** instead (its own elevator warp is ungated), and the BFS reroutes there
    /// automatically. (2) On B4F the elevator lands in the lower room, walled off from Giovanni by a
    /// **door that opens only after both Rockets (trainers 0 & 1) are beaten** — so fight them first,
    /// which drops the wall on the post-battle map reload. Then beat Giovanni (Grass starter is 4×
    /// on his Ground/Rock team; he vanishes on defeat and `ShowObject`s the Scope ball at (25,2)).
    pub fn silph_scope_steps() -> Vec<Self> {
        let mut s = Self::lift_key_steps();
        s.extend([
            // Back up to B2F (spinner nav works both ways) and into the elevator (B2F's warp is not
            // gated by the Rocket-5 door, unlike B1F's).
            Self::enter(Map::RocketHideoutB3F),
            Self::enter(Map::RocketHideoutB2F),
            Self::enter(Map::RocketHideoutElevator),
            // Panel bg-event at (1,1); floors are B1F(0)/B2F(1)/B4F(2) — pick B4F. The menu redirects
            // the exit warp to B4F (25,15) in Giovanni's lower room.
            Self::UseElevator { panel: Point8 { x: 1, y: 1 }, floor: 2 },
            // Beat both Rockets to open the door up to Giovanni (single Interact each — trainers stay
            // put after defeat, so a lone talk suffices and the step pops once it issues the walk).
            Self::Interact(MapSprite::ROCKETHIDEOUTB4F_ROCKET1),
            Self::Interact(MapSprite::ROCKETHIDEOUTB4F_ROCKET2),
            // Beat Giovanni (single Interact — he vanishes on defeat, revealing the Scope), then collect.
            Self::Interact(MapSprite::ROCKETHIDEOUTB4F_GIOVANNI),
            Self::CollectItem(MapSprite::ROCKETHIDEOUTB4F_SILPH_SCOPE),
        ]);
        s
    }

    /// From inside the Rocket Hideout (post-Giovanni, holding the Silph Scope), get the **Poké Flute**:
    /// leave the hideout, travel to Lavender Town, climb **Pokémon Tower** to 7F, and rescue Mr. Fuji.
    ///
    /// Exit is via the elevator to **B2F** (Giovanni's B4F room is walled off; the B1F elevator warp
    /// lands behind the still-shut Rocket-5 door, so ride to B2F and take the stairs up to B1F instead),
    /// then out to the Game Corner and Celadon. Heal, then cross the **Route 7–8 Underground Path** to
    /// Lavender (reverse of `lavender_to_celadon_steps`). In the tower the Channelers engage by sight as
    /// the agent climbs; on 6F stepping toward the 7F stairs triggers the **ghost Marowak** (a scripted
    /// lv30 battle now visible thanks to the Scope); on 7F the three Rockets fall and then Mr. Fuji warps
    /// the player to his house, where talking to him hands over the Poké Flute.
    pub fn poke_flute_steps() -> Vec<Self> {
        let mut s = vec![
            // Leave the hideout: elevator (from Giovanni's isolated B4F room) down to B2F, up to B1F,
            // out to the Game Corner, into Celadon; then heal.
            Self::enter(Map::RocketHideoutElevator),
            Self::UseElevator { panel: Point8 { x: 1, y: 1 }, floor: 1 }, // B2F = menu index 1
            Self::enter(Map::RocketHideoutB1F),
            Self::enter(Map::GameCorner),
            Self::enter(Map::CeladonCity),
            Self::enter(Map::CeladonPokecenter),
            Self::Interact(MapSprite::CELADONPOKECENTER_NURSE),
            Self::enter(Map::CeladonCity),
        ];
        // Celadon → Lavender via the Route 7–8 Underground Path (reverse of lavender_to_celadon).
        s.extend([
            Self::enter(Map::Route7),
            Self::enter(Map::UndergroundPathRoute7),
            Self::enter(Map::UndergroundPathWestEast),
            Self::enter(Map::UndergroundPathRoute8),
            Self::enter(Map::Route8),
            Self::enter(Map::LavenderTown),
        ]);
        // Climb the tower. Each up-warp is at the same corner on consecutive floors; Channelers engage
        // by line of sight as the agent routes to each warp. On 6F the walk to the 7F stairs crosses the
        // ghost-Marowak trigger tile.
        s.extend([
            Self::enter(Map::PokemonTower1F),
            Self::enter(Map::PokemonTower2F),
            Self::enter(Map::PokemonTower3F),
            Self::enter(Map::PokemonTower4F),
            Self::enter(Map::PokemonTower5F),
            Self::enter(Map::PokemonTower6F),
            // The Rare Candy ball at (6,8) blocks the *only* chokepoint into the 6F sub-region that
            // holds the ghost-Marowak trigger and the 7F stairs — collect it to open the path.
            Self::CollectItem(MapSprite::POKEMONTOWER6F_RARE_CANDY),
            Self::enter(Map::PokemonTower7F),
        ]);
        // 7F: beat the three Rockets (they leave on defeat), then talk to Mr. Fuji — his script warps
        // the player to Mr. Fuji's house. There, talk to him again to receive the Poké Flute.
        s.extend([
            Self::Interact(MapSprite::POKEMONTOWER7F_ROCKET1),
            Self::Interact(MapSprite::POKEMONTOWER7F_ROCKET2),
            Self::Interact(MapSprite::POKEMONTOWER7F_ROCKET3),
            Self::Interact(MapSprite::POKEMONTOWER7F_MR_FUJI),
            Self::Interact(MapSprite::MRFUJISHOUSE_MR_FUJI),
        ]);
        s
    }

    /// With the Poké Flute, wake the **Snorlax** blocking **Route 12** (south of Lavender), opening the
    /// road toward Fuchsia. From Mr. Fuji's house: out to Lavender, south onto Route 12, then use the
    /// Poké Flute while facing the Snorlax — that starts a lv30 wild battle the party fights normally;
    /// the sprite is gone once it faints, which pops the `UseFieldItem` step.
    pub fn snorlax_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::LavenderTown), // leave Mr. Fuji's house
            // Heal at Lavender: the party has fought all through the tower with no rest, and the long
            // Route 12–15 trainer gauntlet ahead will black it out otherwise. Also makes Lavender the
            // fallback center for any low-PP heal-flee on the way south.
            Self::enter(Map::LavenderPokecenter),
            Self::Interact(MapSprite::LAVENDERPOKECENTER_NURSE),
            Self::enter(Map::LavenderTown),
            Self::enter(Map::Route12),      // south connection off Lavender (lands at the north tip)
            // The Route-12 Gate building blocks the road; pass through it (north warp → gate → south
            // warp). Disambiguate the two gate→Route12 warps by the south exit's raw landing (10,21),
            // else EnterMap would take the north warp we just came in on and loop.
            Self::enter(Map::Route12Gate1F),
            Self::EnterMap { to_map: Map::Route12, to_position: Some(Point8 { x: 10, y: 21 }) },
            Self::UseFieldItem { item: ItemId::PokeFlute, target: MapSprite::ROUTE12_SNORLAX },
        ]
    }

    /// Soul Badge (Koga, Fuchsia). With the Snorlax cleared, continue **Route 12 south → 13 → 14 → 15 →
    /// Fuchsia City** (all map connections; the Cool-Trainers/Bikers/Beauties on 13–15 engage by line of
    /// sight and are fought normally). Heal at the Fuchsia Center, then enter Koga's gym and beat him —
    /// his team is Poison (Koffing/Muk/Weezing ~lv37–43); a Grass starter resists Poison and leans on
    /// its Normal move + level lead. `DefeatGymLeader` persists through the invisible-wall maze + the six
    /// rocker junior trainers until the badge is won.
    pub fn soul_badge_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::Route13),
            // Cross into Route 14 at the OPEN row-8 landing (19,8): the nearest crossing lands at (19,6),
            // a dead-end pocket sealed by a south-facing Bird Keeper. Route 13 can reach the (0,9) west
            // edge which lands here.
            Self::EnterMap { to_map: Map::Route14, to_position: Some(Point8 { x: 19, y: 8 }) },
            Self::enter(Map::Route15),
            // Route 15 also has a gate building walling off the Fuchsia (west) connection. Enter its
            // east door (nearest), cross, and take the west exit (lands Route 15 (7,8), west of the
            // wall) before the Fuchsia connection is reachable.
            Self::enter(Map::Route15Gate1F),
            Self::EnterMap { to_map: Map::Route15, to_position: Some(Point8 { x: 7, y: 8 }) },
            Self::enter(Map::FuchsiaCity),
            Self::enter(Map::FuchsiaPokecenter),
            Self::Interact(MapSprite::FUCHSIAPOKECENTER_NURSE),
            Self::enter(Map::FuchsiaCity),
            Self::enter(Map::FuchsiaGym),
            Self::DefeatGymLeader { leader: MapSprite::FUCHSIAGYM_KOGA, badge: Badge::SoulBadge },
        ]
    }

    /// Safari Zone run for **HM03 Surf** + the **Gold Teeth** (→ HM04 Strength from the Warden). From
    /// Fuchsia: pay at the gate (the "would you like to join?" prompt auto-confirms on A-mash → 500 +
    /// 30 Safari Balls + a 500-step budget), cross Center → West, grab the Gold Teeth, and get Surf from
    /// the Secret House fishing guru. The deterministic policy RUNs from every Safari encounter (never
    /// costs a ball; the BALL/BAIT/ROCK options exist for a future hunting policy).
    pub fn safari_zone_surf_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::FuchsiaCity),       // out of Koga's gym
            Self::enter(Map::SafariZoneGate),
            Self::enter(Map::SafariZoneCenter),  // pays 500 via the join prompt, auto-walks in
            // The Center's West warp is across the central water; the item-bearing West area is reached
            // the long way round: Center → East → North → West (the only land route).
            Self::enter(Map::SafariZoneEast),
            Self::enter(Map::SafariZoneNorth),
            Self::enter(Map::SafariZoneWest),
            Self::CollectItem(MapSprite::SAFARIZONEWEST_GOLD_TEETH),
            Self::enter(Map::SafariZoneSecretHouse),
            Self::Interact(MapSprite::SAFARIZONESECRETHOUSE_FISHING_GURU), // hands over HM03 Surf
        ]
    }

    /// After the Surf run (holding the Gold Teeth): leave the Safari Zone and give the Gold Teeth to the
    /// **Warden** (Warden's House, Fuchsia) for **HM04 Strength**. Exiting navigates back to the gate;
    /// if the 500-step timer runs out first the game warps the player to the gate anyway, so either way
    /// the `enter(SafariZoneGate)` step resolves.
    pub fn safari_zone_strength_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::SafariZoneWest),    // out of the secret house
            // Center is split by water: the North entrance lands in a top pocket, walled off from the
            // gate. So retrace the full way in (West → North → East → Center) — East→Center lands at the
            // bottom region where the gate is.
            Self::enter(Map::SafariZoneNorth),
            Self::enter(Map::SafariZoneEast),
            Self::enter(Map::SafariZoneCenter),
            Self::enter(Map::SafariZoneGate),
            Self::enter(Map::FuchsiaCity),
            Self::enter(Map::WardensHouse),
            Self::Interact(MapSprite::WARDENSHOUSE_WARDEN), // give Gold Teeth → HM04 Strength
        ]
    }

    /// Enter Saffron (for Silph Co / the Marsh Badge): trek Fuchsia → Celadon, buy a Fresh Water from
    /// the Celadon Mart roof vending machine, then pass the Route-7 gate guard (who takes the drink and
    /// opens all four Saffron gates). Reverse of the soul-badge trek back to Lavender, then the Route
    /// 7–8 underground path to Celadon.
    pub fn saffron_entry_steps() -> Vec<Self> {
        let mut s = vec![
            Self::enter(Map::FuchsiaCity), // out of the Warden's house
            // Fuchsia → Lavender (reverse of the soul-badge routes; Snorlax already cleared).
            Self::enter(Map::Route15),      // from Fuchsia: lands on the west side of the Route-15 gate
            // Reverse the Route-15 gate: west door → east exit (lands Route 15 (14,8), east of the wall).
            Self::enter(Map::Route15Gate1F),
            Self::EnterMap { to_map: Map::Route15, to_position: Some(Point8 { x: 14, y: 8 }) },
            Self::enter(Map::Route14),
            Self::enter(Map::Route13),
            Self::enter(Map::Route12),      // from Route 13: lands south of the Route-12 gate
            // Reverse the Route-12 gate: south door → north exit (lands Route 12 (10,15), north of it).
            Self::enter(Map::Route12Gate1F),
            Self::EnterMap { to_map: Map::Route12, to_position: Some(Point8 { x: 10, y: 15 }) },
            Self::enter(Map::LavenderTown),
            // The nearest Lavender→Route8 crossing (0,11) jams; take the (0,9) one (lands Route8 (59,8)).
            Self::EnterMap { to_map: Map::Route8, to_position: Some(Point8 { x: 59, y: 8 }) },
        ];
        // Lavender → Celadon via the Route 7–8 underground path (existing helper: heals at Celadon too).
        s.extend(Self::lavender_to_celadon_steps());
        // Into the Mart, up to the roof, buy a Fresh Water from the vending machine.
        s.extend([
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonMart2F),
            Self::enter(Map::CeladonMart3F),
            Self::enter(Map::CeladonMart4F),
            Self::enter(Map::CeladonMart5F),
            Self::enter(Map::CeladonMartRoof),
            Self::UseVendingMachine { at: Point8 { x: 10, y: 1 }, drink: ItemId::FreshWater },
            // Back down and out to Celadon, then east through the Route-7 gate into Saffron.
            Self::enter(Map::CeladonMart5F),
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonCity),
            Self::enter(Map::Route7),
            Self::enter(Map::Route7Gate),        // west door
            // Walk east through the gate to the east door (Route 7 (18,10), Saffron side). Crossing the
            // guard-trigger tile (3,4) hands over the Fresh Water (we have it → no push-back).
            Self::EnterMap { to_map: Map::Route7, to_position: Some(Point8 { x: 18, y: 10 }) },
            Self::enter(Map::SaffronCity),
        ]);
        s
    }

    /// Silph Co, part 1: from Saffron, enter Silph Co and ride the elevator to **5F** for the **Card
    /// Key** (which opens the locked doors throughout the building). The elevator works like the Rocket
    /// Hideout's (panel bg-event at (3,0), 11-floor menu: 1F=0 … 5F=4 … 11F=10, redirected exit warp).
    pub fn silph_co_card_key_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::SilphCo1F),
            // The elevator door (20,0) is a wall-embedded warp BFS thinks is reachable but the game
            // blocks, so use the **teleport pads** instead: 1F pad (16,10) → 3F, then a 3F pad → 5F.
            Self::enter(Map::SilphCo3F),
            Self::enter(Map::SilphCo5F),
            Self::CollectItem(MapSprite::SILPHCO5F_CARD_KEY),
        ]
    }

    /// The full deterministic playthrough. Every forward map transition is an explicit `EnterMap`;
    /// on-map tasks (`Interact`/`Buy`/`Grind`/`Catch`) self-route over the incrementally-observed
    /// graph. Starter is **Bulbasaur** — its Grass typing is super-effective against both Brock
    /// (Rock/Ground) and Misty (Water), the two badges this run proves.
    pub fn complete_game_steps() -> Vec<Self> {
        let mut steps = vec![
            // ── Pallet Town: fetch a starter ──
            Self::enter(Map::RedsHouse1F),
            Self::enter(Map::PalletTown),
            Self::soft_goto(Map::Route1),                        // Oak stops you → OaksLab
            Self::Interact(MapSprite::OAKSLAB_BULBASAUR_POKE_BALL), // pick Bulbasaur (+ rival battle)

            // ── Viridian Mart: pick up Oak's Parcel ──
            Self::enter(Map::PalletTown),
            Self::enter(Map::Route1),
            Self::enter(Map::ViridianCity),
            Self::enter(Map::ViridianMart),
            Self::Interact(MapSprite::VIRIDIANMART_CLERK),       // clerk hands over Oak's Parcel

            // ── Deliver the Parcel to Oak → Pokédex ──
            Self::enter(Map::ViridianCity),
            Self::enter(Map::Route1),
            Self::enter(Map::PalletTown),
            Self::enter(Map::OaksLab),
            Self::Interact(MapSprite::OAKSLAB_OAK1),

            // ── Town Map from Daisy ──
            Self::enter(Map::PalletTown),
            Self::enter(Map::BluesHouse),
            Self::Interact(MapSprite::BLUESHOUSE_DAISY1),

            // ── Stock up + heal in Viridian City ──
            Self::enter(Map::PalletTown),
            Self::enter(Map::Route1),
            Self::enter(Map::ViridianCity),
            Self::enter(Map::ViridianPokecenter),
            Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
            Self::enter(Map::ViridianCity),
            Self::enter(Map::ViridianMart),
            Self::Interact(MapSprite::VIRIDIANMART_CLERK),       // open the shop menu
            // Only ₽1500 is available here and the game silently rejects an unaffordable order, so buy
            // 7 Poké Balls (₽1400) — enough to catch a Pidgey. Viridian's Mart does not sell Potions.
            Self::BuyFromMart { item: BagItem::new(ItemId::PokeBall, 7), map: Map::ViridianMart },
            Self::enter(Map::ViridianCity),

            // ── Catch a Pidgey on Route 1 ──
            Self::enter(Map::Route1),
            Self::CatchPokemon { species: PokemonSpecies::Pidgey, on_map: Map::Route1 },
            // Heal after the catch: the catch battles leave the starter (and the just-caught,
            // low-HP Pidgey) badly hurt; grinding straight away death-spirals. Full-heal first.
            Self::enter(Map::ViridianCity),
            Self::enter(Map::ViridianPokecenter),
            Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
            Self::enter(Map::ViridianCity),
            // ── Grind the starter on Route 1 ──
            Self::enter(Map::Route1),
            Self::GrindUntilLevel { target_level: 13, on_map: Map::Route1 },
            Self::enter(Map::ViridianCity),
            Self::enter(Map::ViridianPokecenter),
            Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
            Self::enter(Map::ViridianCity),

            // ── Viridian Forest → Pewter City ──
            Self::enter(Map::Route2),
            Self::enter(Map::ViridianForestSouthGate),
            Self::enter(Map::ViridianForest),
            Self::enter(Map::ViridianForestNorthGate),
            Self::enter(Map::Route2),
            Self::enter(Map::PewterCity),
            Self::enter(Map::PewterPokecenter),
            Self::Interact(MapSprite::PEWTERPOKECENTER_NURSE),
            Self::enter(Map::PewterCity),

            // ── Defeat Brock (Boulder Badge) ──
            Self::DefeatGymLeader { leader: MapSprite::PEWTERGYM_BROCK, badge: Badge::BoulderBadge },
            // Exit the gym to the city first (a single warp): every forward `enter` must be one
            // direct transition. Jumping straight to the Pokécenter from inside the gym is a 2-hop
            // path that would rely on routing through a never-before-observed gym-exit landing.
            Self::enter(Map::PewterCity),
            Self::enter(Map::PewterPokecenter),
            Self::Interact(MapSprite::PEWTERPOKECENTER_NURSE),
            Self::enter(Map::PewterCity),

            // ── Route 3 grind → heal at the Mt Moon Pokécenter ──
            Self::enter(Map::Route3),
            Self::GrindUntilLevel { target_level: 18, on_map: Map::Route3 },
            Self::enter(Map::Route4),
            Self::enter(Map::MtMoonPokecenter),
            Self::Interact(MapSprite::MTMOONPOKECENTER_NURSE),
            Self::enter(Map::Route4),
            Self::enter(Map::MtMoon1F),
        ];

        // ── Cross Mt Moon → Cerulean City ──
        steps.extend(Self::mt_moon_traversal());

        steps.extend([
            // ── Heal in Cerulean, then beat Misty (Cascade Badge) ──
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
            Self::enter(Map::CeruleanCity),
            Self::DefeatGymLeader { leader: MapSprite::CERULEANGYM_MISTY, badge: Badge::CascadeBadge },
            // Exit the gym to the city (single warp) before entering the Pokécenter — see the
            // Pewter gym note above.
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
        ]);

        // ── Bill (SS Ticket) → trashed-house bridge → Vermilion City ──
        steps.extend(Self::cerulean_to_vermilion_steps());
        // ── S.S. Anne: clear every trainer, beat the rival, get HM01 Cut from the captain ──
        steps.extend(Self::ss_anne_steps());
        // ── Thunder Badge: teach Cut → cut the gym tree → trash-can puzzle → Lt. Surge ──
        steps.extend(Self::thunder_badge_steps());
        // ── Back to Cerulean (Underground Path in reverse) → Rock Tunnel → Lavender ──
        steps.extend(Self::back_to_cerulean_steps());
        steps.extend(Self::cerulean_to_lavender_steps());
        // ── Lavender → Celadon (Route 7–8 Underground Path) → Rainbow Badge (Erika) ──
        steps.extend(Self::lavender_to_celadon_steps());
        steps.extend(Self::celadon_rainbow_steps());

        steps
    }
}

pub struct DeterministicPolicy {
    rng: StdRng,
    queue: VecDeque<PolicyStep>,
    name_picker: PokemonNamePicker,
    /// The last Pokémon Center where the player was healed.
    pub last_pokemon_center: Option<Map>,
    /// Set to `Some(pokecenter)` when the active Pokémon's damaging moves are all at ≤10% PP
    /// and the policy decided to flee the current wild battle. The policy will navigate to that
    /// Pokémon Center and heal before resuming the main queue.
    heal_return: Option<Map>,
    /// Number of times the current `BuyFromMart` step has re-opened the shop without the purchase
    /// registering in the bag. The clerk-entry path occasionally drops the YES-confirm (no clean
    /// joypad rising edge), so the step verifies the bag and retries a few times before giving up
    /// (e.g. for an item the mart doesn't actually sell).
    mart_attempts: u32,
    /// True once the current `CollectItem` step's item sprite has been observed present (not hidden).
    /// The step then pops only when the item *disappears* (collected). Distinguishes "collected" from
    /// "not yet revealed" — some item balls stay hidden until their guard is beaten (e.g. the Rocket
    /// Hideout Lift Key / Silph Scope), and popping on the initial hidden state would skip them.
    collect_item_seen: bool,
}

impl DeterministicPolicy {
    /// How many times to re-open the shop for one `BuyFromMart` step before giving up.
    const MAX_MART_ATTEMPTS: u32 = 4;

    pub fn new(seed: u64, steps: impl IntoIterator<Item = PolicyStep>) -> Self {
        Self {
            rng: StdRng::seed_from_u64(seed),
            queue: steps.into_iter().collect(),
            name_picker: PokemonNamePicker::seed_from_u64(seed),
            last_pokemon_center: None,
            heal_return: None,
            mart_attempts: 0,
            collect_item_seen: false,
        }
    }

    /// Route one hop toward `target` over the **incremental** world graph.
    ///
    /// The graph only contains sections the agent has already visited (accurate, sprite-resolved),
    /// so this succeeds for backtracking / already-explored territory (heal-return, reaching a map
    /// the explicit `EnterMap` steps have already led through) and returns `None` for a not-yet-
    /// visited target — the signal that the deterministic policy is under-specified.
    fn route_toward(world_graph: &WorldGraph, actions: &[OverworldAction], target: Map) -> Option<OverworldAction> {
        world_graph.pick_shortest_path_action(actions, target)
    }

    /// The action that takes the warp/connection to `to_map` (matching raw `to_position` when
    /// given) from the current map, or `None` if no such transition is reachable here.
    fn enter_map_action(actions: &[OverworldAction], to_map: Map, to_position: Option<Point8>) -> Option<OverworldAction> {
        actions.iter().find(|a| match a.tile {
            MetaTile::Warp { to_map: m, to_position: p }
            | MetaTile::Connection { to_map: m, to_position: p } => {
                m == to_map && to_position.map_or(true, |want| want == p)
            }
            _ => false,
        }).cloned()
    }

    pub fn complete_game(seed: u64) -> Self {
        Self::new(seed, PolicyStep::complete_game_steps())
    }
}

impl Policy for DeterministicPolicy {

    fn pick_overworld_action(&mut self, state: &GameState, world_graph: &WorldGraph) -> Option<OverworldAction> {
        if state.map.map.is_pokemon_center() {
            self.last_pokemon_center = Some(state.map.map);
        }

        let actions = state.map.actions();

        // ── Heal-return detour ────────────────────────────────────────────────
        // When the active Pokémon ran low on PP in a wild battle we fled and
        // stored the target Pokémon Center in `heal_return`.  Route there over the
        // incrementally-built graph (the pokecenter and the way back are already known,
        // since we walked here) and talk to the Nurse before resuming the main queue.
        if let Some(pokecenter) = self.heal_return {
            return if state.map.map == pokecenter {
                // Arrived — find and interact with the Nurse.
                if let Some(action) = actions.iter().find(|a| a.tile == MetaTile::Sprite("Nurse")) {
                    self.heal_return = None;
                    Some(action.clone())
                } else {
                    // Pokecenter map but Nurse tile not visible yet — wait.
                    None
                }
            } else {
                // Still travelling — pick next step toward the pokecenter.
                Self::route_toward(world_graph, &actions, pokecenter)
            };
        }

        println!("[policy] map={} pos={} front={:?} queue_len={}",
            state.map.map, state.map.player_position, self.queue.front(), self.queue.len());
        loop {
            let step = self.queue.front()?.clone();
            return match step {
                PolicyStep::EnterMap { to_map, to_position } => {
                    if state.map.map == to_map {
                        self.queue.pop_front();
                        continue;
                    }
                    // Explicit single map transition: take exactly this warp/connection.
                    if let Some(action) = Self::enter_map_action(&actions, to_map, to_position) {
                        return Some(action);
                    }
                    // A specific connection landing that isn't the nearest crossing (which is all
                    // `actions()` emits) — build it directly (e.g. Route 13→14 open row, not the pocket).
                    if let Some(pos) = to_position {
                        if let Some(action) = state.map.connection_action(to_map, pos) {
                            return Some(action);
                        }
                    }
                    // Recovery: the direct transition isn't on the current map. This happens when a
                    // teleport back into already-explored territory desyncs the linear EnterMap
                    // script — a blackout (fainting) sends the player home, and the heal-flee detour
                    // moves them to a Pokémon Center. If the target map has already been observed,
                    // route back toward it over the incremental world graph (visited territory only).
                    // If it has NOT been observed this returns None and the agent stalls — the
                    // intended hard-fail for genuinely under-specified forward travel.
                    Self::route_toward(world_graph, &actions, to_map)
                },
                PolicyStep::Goto { map: target, strict } => {
                    if state.map.map == target {
                        self.queue.pop_front();
                        continue;
                    }
                    let action = Self::route_toward(world_graph, &actions, target);
                    if !strict && action.is_some() {
                        // a non-strict goto action can be interrupted
                        self.queue.pop_front();
                    }
                    action
                },
                PolicyStep::CatchPokemon { species, on_map } => {
                    if state.map.map != on_map {
                        let action = Self::route_toward(world_graph, &actions, on_map);
                        if action.is_none() {
                            println!("[policy] want to catch pokemon {} in {}, but no path there!", species, on_map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if state.pokedex_owned.contains(&species) {
                        // caught the pokemon (note this only works once for each species)
                        self.queue.pop_front();
                        continue;
                    } else if let Some(action) = actions.iter()
                        .find(|a| a.tile == MetaTile::Grass) {
                        if state.bag.best_pokeball().is_some() {
                            // walk in grass
                            Some(action.clone())
                        } else {
                            println!("[policy] want to catch a {}, but no Pokéballs left!", species);
                            self.queue.pop_front();
                            continue;
                        }
                    } else {
                        println!("[policy] want to catch a {}, but no grass nearby!", species);
                        self.queue.pop_front();
                        continue;
                    }
                },
                PolicyStep::GrindUntilLevel { target_level, on_map } => {
                    if state.map.map != on_map {
                        let action = Self::route_toward(world_graph, &actions, on_map);
                        if action.is_none() {
                            println!("[policy] want to grind until level {} in {}, but no path there!", target_level, on_map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if let Some(pokemon) = state.pokemon.get(0) {
                        if pokemon.level >= target_level {
                            self.queue.pop_front();
                            continue;
                        } else if let Some(action) = actions.iter()
                            .find(|a| a.tile == MetaTile::Grass) {
                            // walk in grass
                            Some(action.clone())
                        } else {
                            println!("[policy] cannot level up a Pokemon, no grass nearby!");
                            self.queue.pop_front();
                            continue;
                        }
                    } else {
                        println!("[policy] no Pokemon in party to level up");
                        self.queue.pop_front();
                        continue;
                    }
                },
                PolicyStep::DefeatGymLeader { leader, badge } => {
                    if state.badges.contains(badge) {
                        self.queue.pop_front();
                        continue;
                    } else if state.map.map != leader.map() {
                        let action = Self::route_toward(world_graph, &actions, leader.map());
                        if action.is_none() {
                            println!("[policy] want to defeat {} to obtain the {}, but no path there!", leader, badge);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else {
                        // Stay on this step until the badge is obtained — do not pop here.
                        // If the player loses and blacks out, the step remains and the agent
                        // navigates back to try again.
                        actions.iter()
                            .find(|a| a.tile == MetaTile::Sprite(leader.name))
                            .cloned()
                    }
                },
                PolicyStep::Interact(sprite) => {
                    // Prefer the sprite visible on the CURRENT map. Sprite identity is by name only,
                    // so sprites that recur across maps (Nurse, Clerk) cannot be disambiguated by
                    // `sprite.map()` — it returns the first map in enum order, misrouting every
                    // pokecenter heal but the first. The scripted `enter(map)` step preceding each
                    // `Interact` already places the agent on the intended map, so matching the
                    // visible sprite here by name is both correct and robust.
                    if let Some(action) = actions.iter().find(|a| a.tile == MetaTile::Sprite(sprite.name)) {
                        self.queue.pop_front();
                        return Some(action.clone());
                    }
                    let map = sprite.map();
                    if state.map.map == map {
                        // On the sprite's map but it isn't actionable yet (e.g. still walking on, or
                        // the sprite is briefly hidden by a script) — wait for it. (Do NOT pop when the
                        // sprite is hidden: some sprites hide transiently mid-script, e.g. Bill right
                        // after the PC, and popping would abort the interaction. Sprites that vanish
                        // permanently on defeat, like the Game Corner Rocket, are handled by a single
                        // non-retried `Interact` that pops the instant it issues the walk.)
                        None
                    } else {
                        let action = Self::route_toward(world_graph, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to interact with {} on {}, but no path there!", sprite, map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    }
                }
                PolicyStep::UsePc { map } => {
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to use the PC on {}, but no path there!", map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if let Some(action) = actions.iter().find(|a| a.tile == MetaTile::Pc) {
                        // On the PC's map and the PC is reachable — face it and press A, then advance.
                        self.queue.pop_front();
                        return Some(action.clone());
                    } else {
                        // On the map but the PC isn't reachable yet (e.g. a script is still running) —
                        // wait for it to become actionable.
                        None
                    }
                }
                PolicyStep::CollectItem(sprite) => {
                    let map = sprite.map();
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to collect {} on {}, but no path there!", sprite, map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else {
                        let present = state.map.sprites.iter().any(|s| !s.hidden && s.name == sprite.name);
                        if present { self.collect_item_seen = true; }
                        if !present && self.collect_item_seen {
                            // The item was here and is now gone — picked up (or removed by a script). Done.
                            self.collect_item_seen = false;
                            self.queue.pop_front();
                            continue;
                        }
                        if !present {
                            // Not yet revealed (an item ball hidden until its guard is beaten) — wait.
                            None
                        } else {
                            // Keep walking to and pressing A on the item until it disappears; do NOT
                            // pop on issue, so a battle/script interruption (Mt Moon Super Nerd) mid-walk
                            // doesn't abandon the pickup.
                            actions.iter()
                                .find(|a| a.tile == MetaTile::Sprite(sprite.name))
                                .cloned()
                        }
                    }
                }
                PolicyStep::BuyFromMart { item, map } => {
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to buy {} from {} but no path there!", item, map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if state.bag.iter().any(|i| i.id == item.id && i.quantity >= item.quantity) {
                        // Purchase registered (bag now holds ≥ the target quantity) — done.
                        self.mart_attempts = 0;
                        self.queue.pop_front();
                        continue;
                    } else if self.mart_attempts >= Self::MAX_MART_ATTEMPTS {
                        // The shop re-opened this many times without the item appearing — the mart
                        // probably doesn't sell it (e.g. Potion in Viridian). Give up on this step.
                        println!("[policy] gave up buying {} from {} after {} attempts", item, map, self.mart_attempts);
                        self.mart_attempts = 0;
                        self.queue.pop_front();
                        continue;
                    } else {
                        // If triggered in the overworld, talk to the "Clerk" sprite to (re)open the
                        // pokemart menu. `pick_mart_purchase` will drive the actual buy; we re-verify
                        // the bag on the next overworld tick and retry if the confirm was dropped.
                        let action = actions.iter()
                            .find(|a| matches!(a.tile, MetaTile::Sprite(sprite) if sprite == "Clerk"));

                        if action.is_none() {
                            println!("[policy] BuyFromMart step encountered in pick_overworld_action and no clerk available — skipping");
                            self.mart_attempts = 0;
                            self.queue.pop_front();
                            continue;
                        }

                        action.cloned()
                    }
                }
                PolicyStep::TeachMove { .. } => {
                    // Handled by `pick_field_move` (the agent calls it first). If we reach here the
                    // teach isn't ready yet — wait without advancing the queue.
                    None
                }
                PolicyStep::CutTree { map } => {
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to cut a tree on {map} but no path there!");
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if matches!(state.map.tile_in_front(), Some((_, MetaTile::CutTree))) {
                        // Facing a tree — `pick_field_move` performs the cut; just wait.
                        None
                    } else {
                        // Route to face a reachable tree; once none remain, the trees are cut — done.
                        match actions.iter().find(|a| a.tile == MetaTile::CutTree).cloned() {
                            Some(action) => Some(action),
                            None => { self.queue.pop_front(); continue; }
                        }
                    }
                }
                PolicyStep::SolveTrashCans => {
                    if state.map.map != Map::VermilionGym {
                        let action = Self::route_toward(world_graph, &actions, Map::VermilionGym);
                        if action.is_none() {
                            println!("[policy] want to solve trash cans but can't reach Vermilion Gym!");
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else {
                        // On the gym floor — `pick_field_move` drives checking the switch cans.
                        None
                    }
                }
                PolicyStep::FlipSwitch { map, .. } => {
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to flip a switch on {map} but can't reach it!");
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else {
                        // On the map — `pick_field_move` drives facing + pressing the switch.
                        None
                    }
                }
                PolicyStep::UseElevator { .. } => {
                    // Handled by `pick_field_move` once on the elevator map (an `enter(...Elevator)` step
                    // precedes this one). If we're not on it, the elevator can't be used — pop.
                    if state.map.map != Map::RocketHideoutElevator {
                        println!("[policy] UseElevator but not in the elevator room ({});", state.map.map);
                        self.queue.pop_front();
                        continue;
                    }
                    None
                }
                PolicyStep::UseFieldItem { .. } => {
                    // Facing the target and driving the bag menus is handled by `pick_field_move` /
                    // `UsingFieldItem` once the target sprite is observed on the current map (a preceding
                    // EnterMap places the agent on its map).
                    None
                }
                PolicyStep::UseVendingMachine { .. } => None, // driven by `pick_field_move`
            }
        }
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        let battle_state = state.battle.as_ref()?;
        let actions = battle_options(state)?;

        // Safari Zone: the deterministic policy never hunts — always RUN (a Safari run never fails), to
        // preserve steps/balls while it navigates to the items. (The BALL/BAIT/ROCK options are still in
        // `battle_options` so a future LLM policy can choose to catch.)
        if battle_state.battle_type == BattleType::Safari {
            return Some(BattleAction::Run);
        }

        if self.heal_return.is_some() && battle_state.battle_type == BattleType::Wild {
            // returning to the pokemon center, run from battles.
            if let Some(center) = self.last_pokemon_center {
                println!("[policy] PP critically low — fleeing and routing to {center} to heal");
            }
            return Some(BattleAction::Run);
        }

        // ── Low-PP flee ──────────────────────────────────────────────────────
        // If every damaging move the active Pokémon has is at ≤10% of its max PP,
        // run from wild battles and queue a detour to the last visited Pokémon Center.
        if battle_state.battle_type == BattleType::Wild
            && self.heal_return.is_none()
            && all_damaging_moves_low_pp(&actions)
        {
            if let Some(center) = self.last_pokemon_center {
                println!("[policy] PP critically low — fleeing and routing to {center} to heal");
                self.heal_return = Some(center);
                return Some(BattleAction::Run);
            } else {
                println!("[policy] PP critically low but no known Pokémon Center to return to — fighting on");
            }
        }

        // If the active Pokémon is fainted (forced switch screen), send the
        // healthiest available party member.
        if battle_state.player.current_hp == 0 {
            return actions.iter()
                .filter(|a| matches!(a, BattleAction::SwitchPokemon { .. }))
                .max_by_key(|a| match a {
                    BattleAction::SwitchPokemon { pokemon, .. } => pokemon.current_hp,
                    _ => 0,
                })
                .cloned();
        }

        // When catching, throw a Pokéball immediately if one is available.
        if let Some(PolicyStep::CatchPokemon { species, .. }) = self.queue.front() {
            if battle_state.battle_type == BattleType::Wild && battle_state.enemy.species == *species {
                if let Some(best_pokeball) = state.bag.best_pokeball() {
                    if let Some(use_pokeball_action) = actions.iter()
                        .find(|a| matches!(a, BattleAction::UseItem { item, .. } if item.id == best_pokeball.id )) {

                        // If enemy HP > 50%, try to weaken it first with the move that does
                        // the most damage without knocking the Pokémon out.
                        if battle_state.enemy.remaining_hp() > 0.5 {
                            if let Some(mv) = pick_best_move(&battle_state, &actions, true) {
                                println!("[policy] enemy HP > 50% — weakening before throwing ball");
                                return Some(mv);
                            }
                        }

                        return Some(use_pokeball_action.clone());
                    } else {
                        println!("[policy] want to catch a {}, but no use Pokéball actions were provided!", species);
                    }
                } else {
                    println!("[policy] want to catch a {}, but no Pokéballs left!", species);
                }
            }
        }

        // Use a healing item if HP is below 25% — prioritise max-heal items.
        if battle_state.player.remaining_hp() < 0.25 {
            let heal = actions.iter().find(|a| matches!(a,
                BattleAction::UseItem { item, .. }
                if matches!(item.id, ItemId::MaxPotion | ItemId::HyperPotion | ItemId::SuperPotion | ItemId::Potion)
            ));
            if let Some(heal_action) = heal {
                println!("[policy] HP critical ({:.0}%) — using healing item", battle_state.player.remaining_hp() * 100.0);
                return Some(*heal_action);
            }
        }

        // Switch to the healthiest party member if below 15% HP and a better option exists.
        // Exception: while grinding we deliberately want the lead (slot 0) to take the wild-battle
        // XP, so switching a healthy bench mon in would starve the lead of levels and the grind
        // would never finish. During a `GrindUntilLevel` step, keep the lead in (blackout recovery
        // heals and resumes if it faints).
        let grinding = matches!(self.queue.front(), Some(PolicyStep::GrindUntilLevel { .. }));
        if !grinding && battle_state.player.remaining_hp() < 0.15 {
            if let Some(switch) = actions.iter()
                .filter(|a| matches!(a, BattleAction::SwitchPokemon { .. }))
                .max_by_key(|a| match a {
                    BattleAction::SwitchPokemon { pokemon, .. } => pokemon.current_hp,
                    _ => 0,
                })
            {
                if let BattleAction::SwitchPokemon { pokemon, .. } = switch {
                    // Only switch to a member that is a *genuine* alternative: meaningfully healthy
                    // (>50% of its own max HP) AND at least the active mon's level. A low-level bench
                    // mon (e.g. a lv4 Pidgey behind a lv18 Ivysaur) is a sacrificial weakling — even
                    // at full HP it faints immediately, so swapping it into a trainer battle just
                    // hands over a Pokémon and stalls the run (observed: the Mt Moon Super Nerd fight
                    // never cleared, so the fossil was never collected). This mirrors the original
                    // lone-Ivysaur behaviour: fight on and rely on blackout recovery when there is no
                    // real switch, but take a strong, healthy team-mate when one exists.
                    let healthy_enough = pokemon.stats.hp > 0
                        && pokemon.current_hp as u32 * 2 > pokemon.stats.hp as u32;
                    let strong_enough = pokemon.level >= battle_state.player.level;
                    if healthy_enough && strong_enough && pokemon.current_hp > battle_state.player.current_hp {
                        println!("[policy] HP critical — switching to {} (lv{} {}/{}hp)",
                            pokemon.species, pokemon.level, pokemon.current_hp, pokemon.stats.hp);
                        return Some(*switch);
                    }
                }
            }
        }

        // 1. pick the strongest move
        let result = pick_best_move(&battle_state, &actions, false);
        if result.is_some() {
            return result;
        }

        // No damaging move on the active Pokémon (out of PP, or all resisted to 0 damage).
        // Prefer switching to a party member that CAN damage the enemy, rather than spamming a
        // status move — especially Leech Seed, whose HP drain keeps the active Pokémon alive
        // indefinitely and deadlocks the whole battle (observed Nugget-Bridge stall).
        if let Some(switch) = actions.iter()
            .filter_map(|a| match a {
                BattleAction::SwitchPokemon { pokemon, .. } => {
                    let best = pokemon.moves.iter().flatten()
                        .filter(|m| m.pp > 0)
                        .filter_map(|m| expected_damage(pokemon, m.name, &battle_state.enemy))
                        .max().unwrap_or(0);
                    (best > 0).then_some((best, a))
                }
                _ => None,
            })
            .max_by_key(|(dmg, _)| *dmg)
            .map(|(_, a)| a)
        {
            println!("[policy] no damaging move available — switching to an attacker");
            return Some(switch.clone());
        }

        // No party member can damage the enemy. Avoid self-healing moves (Leech Seed) so the
        // battle actually resolves (faint → black-out recovery) instead of stalling forever.
        if let Some(a) = actions.iter()
            .filter(|a| matches!(a,
                BattleAction::Fight { battle_move, .. } if battle_move.name != PokemonMoveName::LeechSeed))
            .choose(&mut self.rng)
        {
            return Some(a.clone());
        }

        // Last resort: any fight action, else any action.
        actions.iter().find(|a| matches!(a, BattleAction::Fight { .. }))
            .or_else(|| actions.iter().next())
            .cloned()
    }

    fn pick_nickname(&mut self, _species: PokemonSpecies) -> Option<Option<String>> {
        let name = self.name_picker.pick().to_string();
        println!("[policy] pick name={}", name);
        Some(Some(name))
    }

    fn pick_move_to_forget(&mut self, current_moves: &[PokemonMove], new_move: PokemonMoveName)
        -> Option<Option<usize>>
    {
        // Forget the *weakest* move rather than the default slot-0 (which was silently discarding
        // Tackle). Value = base power, with any damaging move ranked above every status move (the +1
        // tie-break also covers fixed-damage moves like Seismic Toss that list no power); HM moves are
        // given max value so they are never forgotten (needed for field use). Because status moves
        // rank lowest, a mixed moveset keeps its damaging moves — e.g. Ivysaur learning Poisonpowder
        // forgets Growl/Leech Seed, not Tackle or Vine Whip. We always learn into the weakest slot
        // (never decline) to avoid the fragile "abandon learning?" YES/NO flow; the only mild loss is
        // a Pokémon that already knows four damaging moves learning a weak one, which is rare.
        let is_hm = |m: PokemonMoveName| matches!(m, PokemonMoveName::Cut | PokemonMoveName::Fly
            | PokemonMoveName::Surf | PokemonMoveName::Strength | PokemonMoveName::Flash);
        let value = |m: PokemonMoveName| if is_hm(m) { u16::MAX }
            else { m.metadata().power.unwrap_or(0) as u16 + if is_damaging_move(m) { 1 } else { 0 } };

        let slot = current_moves.iter().enumerate()
            .min_by_key(|(_, m)| value(m.name))
            .map(|(i, _)| i)?;
        println!("[policy] learning {new_move:?} — forgetting slot {slot} ({:?})",
            current_moves.get(slot).map(|m| m.name));
        Some(Some(slot))
    }

    fn pick_field_move(&mut self, state: &GameState) -> Option<FieldMove> {
        // Cut a tree the player is already facing (routed there by the CutTree overworld action).
        if let Some(&PolicyStep::CutTree { map }) = self.queue.front() {
            if state.map.map == map
                && matches!(state.map.tile_in_front(), Some((_, MetaTile::CutTree)))
            {
                return Some(FieldMove::CutTree);
            }
        }
        if let Some(&PolicyStep::TeachMove { item, target_slot }) = self.queue.front() {
            let already_knows = hm_move(item).map_or(false, |mv| {
                state.pokemon.get(target_slot as usize)
                    .map_or(false, |p| p.moves.iter().flatten().any(|m| m.name == mv))
            });
            if already_knows {
                println!("[policy] TeachMove: slot {target_slot} already knows the move — done");
                self.queue.pop_front();
                return None;
            }
            return Some(FieldMove::TeachMove { item, target_slot });
        }
        if let Some(&PolicyStep::SolveTrashCans) = self.queue.front() {
            if let Some(puzzle) = &state.trash_cans {
                if puzzle.second_opened {
                    println!("[policy] SolveTrashCans: both locks open — door unlocked");
                    self.queue.pop_front();
                    return None;
                }
                let target = if puzzle.first_opened { puzzle.second_target } else { puzzle.first_target };
                return Some(FieldMove::CheckTrashCan { target });
            }
        }
        if let Some(&PolicyStep::FlipSwitch { map, at, reveals }) = self.queue.front() {
            if state.map.map == map {
                // Done once the switch's event fires (the warp itself is always in the static map, so
                // "is the warp reachable" can't tell flipped from not — check the game event instead).
                let done = match reveals {
                    Map::RocketHideoutB1F => state.found_rocket_hideout,
                    _ => false,
                };
                if done {
                    println!("[policy] FlipSwitch: {reveals} passage revealed — done");
                    self.queue.pop_front();
                    return None;
                }
                // Reuse the trash-can face-and-press mechanism to press A on the bg-event tile.
                return Some(FieldMove::CheckTrashCan { target: at });
            }
        }
        if let Some(&PolicyStep::UseElevator { panel, floor }) = self.queue.front() {
            // The step completes once we've ridden the elevator out to another floor.
            if state.map.map != Map::RocketHideoutElevator {
                self.queue.pop_front();
                return None;
            }
            return Some(FieldMove::UseElevator { panel, floor });
        }
        if let Some(&PolicyStep::UseFieldItem { item, target }) = self.queue.front() {
            let present = state.map.sprites.iter().any(|s| !s.hidden && s.name == target.name);
            if present { self.collect_item_seen = true; }
            // Done once the target has been seen and is now gone (the item's effect — e.g. waking then
            // defeating the Snorlax — removed it).
            if !present && self.collect_item_seen {
                self.collect_item_seen = false;
                println!("[policy] UseFieldItem: {} gone — done", target.name);
                self.queue.pop_front();
                return None;
            }
            if !present { return None; } // target not yet observed on this map — keep walking/waiting
            let pos = state.map.sprites.iter()
                .find(|s| !s.hidden && s.name == target.name)
                .map(|s| s.position)?;
            return Some(FieldMove::UseFieldItem { item, target: pos });
        }
        if let Some(&PolicyStep::UseVendingMachine { at, drink }) = self.queue.front() {
            if state.bag.contains(&drink) {
                println!("[policy] UseVendingMachine: bought {drink:?} — done");
                self.queue.pop_front();
                return None;
            }
            // Reuse the face-a-bg-event-and-press-A mechanism; the vending menu opens with the cheapest
            // drink at the cursor, so A-mashing buys it. Persists until the drink is in the bag.
            return Some(FieldMove::CheckTrashCan { target: at });
        }
        None
    }

    fn pick_mart_purchase(&mut self, _state: &GameState) -> Option<Option<BagItem>> {
        let result = match self.queue.front() {
            Some(PolicyStep::BuyFromMart { item, .. }) => {
                // Count this shop-open as an attempt. The `BuyFromMart` overworld arm pops the step
                // once the bag reflects the purchase (or after MAX_MART_ATTEMPTS), so we do NOT pop
                // here — a dropped YES-confirm re-opens the shop and retries.
                self.mart_attempts += 1;
                println!("[policy] BuyFromMart: {:?} (attempt {})", item, self.mart_attempts);
                Some(*item)
            }
            _ => {
                println!("[policy] pick_mart_purchase called but no BuyFromMart step queued — returning None");
                None
            },
        };

        Some(result)
    }

    fn is_exhausted(&self) -> bool {
        self.queue.is_empty()
    }

    fn steps_remaining(&self) -> Option<usize> {
        Some(self.queue.len())
    }

    fn current_step_is_long_running(&self) -> bool {
        matches!(
            self.queue.front(),
            Some(PolicyStep::GrindUntilLevel { .. })
                | Some(PolicyStep::CatchPokemon { .. })
                // Collecting the Mt Moon fossil means crossing a battle-heavy floor: each wild
                // encounter interrupts the walk, and with a real (non-pimped) party those battles
                // are slow, so the single CollectItem step legitimately sits for a long while.
                | Some(PolicyStep::CollectItem(_))
                // A gym-leader fight sits on one step for the whole battle, and self-heals + re-routes
                // on a blackout (queue unchanged the whole time) — legitimately long-running.
                | Some(PolicyStep::DefeatGymLeader { .. })
        )
    }
}
#[cfg(test)]
mod move_learn_tests {
    use super::*;
    use crate::pokemon::move_name::PokemonMoveName::*;

    fn mv(name: crate::pokemon::move_name::PokemonMoveName) -> PokemonMove {
        PokemonMove::with_max_pp(name)
    }

    #[test]
    fn keeps_damaging_moves_when_learning_status() {
        let mut p = DeterministicPolicy::new(0, Vec::<PolicyStep>::new());
        // Ivysaur: [Tackle(dmg), Growl(status), LeechSeed(status), VineWhip(dmg)] learning Poisonpowder.
        let moves = [mv(Tackle), mv(Growl), mv(LeechSeed), mv(VineWhip)];
        let slot = p.pick_move_to_forget(&moves, Poisonpowder).flatten().expect("should pick a slot");
        assert!(slot == 1 || slot == 2,
            "forgot slot {slot} ({:?}) — must forget a status move, not Tackle/Vine Whip", moves[slot].name);
    }

    #[test]
    fn learns_strong_move_over_status() {
        let mut p = DeterministicPolicy::new(0, Vec::<PolicyStep>::new());
        let moves = [mv(Tackle), mv(Growl), mv(LeechSeed), mv(VineWhip)];
        // Learning Razor Leaf (strong) should still forget a status slot, keeping both damaging moves.
        let slot = p.pick_move_to_forget(&moves, RazorLeaf).flatten().unwrap();
        assert!(slot == 1 || slot == 2, "should forget a status move to learn Razor Leaf");
    }

    #[test]
    fn never_forgets_hm() {
        let mut p = DeterministicPolicy::new(0, Vec::<PolicyStep>::new());
        let moves = [mv(Cut), mv(Growl), mv(LeechSeed), mv(Poisonpowder)];
        let slot = p.pick_move_to_forget(&moves, Poisonpowder).flatten().unwrap();
        assert_ne!(moves[slot].name, Cut, "must never forget an HM move (Cut)");
    }
}
