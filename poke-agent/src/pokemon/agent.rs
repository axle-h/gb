use std::collections::VecDeque;
use std::fmt::Display;
use std::time::Duration;
use gb::cycles::MachineCycles;
use poke_core::geometry::Point8;
use gb::joypad::JoypadButton;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::{GameState, PokemonApi, PokemonApiTrait};
use crate::pokemon::bag::BagItem;
use crate::pokemon::delay::DelayContext;
use crate::pokemon::encoding::GameMode;
use crate::pokemon::map::Map;
use crate::pokemon::tile::MetaTile;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::menu::{is_forget_move_prompt, is_start_menu, BattleMenuState, START_MENU_ORIGIN};
use crate::pokemon::learnset::teach_refusal;
use crate::pokemon::pokedex::PokedexReader;
use crate::pokemon::policy::{Jam, Policy, RandomPolicy};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::text::PokemonTextReader;
use crate::pokemon::world_graph::WorldGraph;

/// Trim an order to what the wallet actually covers, or `None` when it covers nothing.
fn affordable(api: &PokemonApi<'_>, money: u32, item: BagItem) -> Option<BagItem> {
    let item = match api.item_price(item.id) {
        Some(price) => BagItem::new(item.id, item.quantity.min((money / price) as u8)),
        // Not in the price table (a key item, or a mart-specific TM): order as asked.
        None => item,
    };
    (item.quantity > 0).then_some(item)
}

// Longer and the player veers off course on the overworld; shorter and the game has no chance to
// update values between turns.
pub const AGENT_RESOLUTION: MachineCycles = MachineCycles::from_duration(Duration::from_millis(20));

/// Manual presses [`PokemonAgent::queue_manual_input`] will hold; the state machine does not run
/// while they drain.
pub const MANUAL_INPUT_CAPACITY: usize = 16;

/// Agent ticks a manual press is held before being released.
const MANUAL_INPUT_HOLD_TICKS: u8 = 2;

/// Total ticks one queued press occupies: the hold, plus one tick released.
pub const MANUAL_INPUT_TICKS_PER_PRESS: u8 = MANUAL_INPUT_HOLD_TICKS + 1;

/// `wMovementFlags` bit 7: set for exactly as long as an arrow tile is sliding the player.
const BIT_SPINNING: u8 = 1 << 7;

fn player_is_spinning(api: &PokemonApi) -> bool {
    api.mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wMovementFlags)
        & BIT_SPINNING != 0
}

/// From the last Pokémon fainting until the black-out warp has landed.
fn blackout_in_flight(api: &PokemonApi) -> bool {
    api.mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wIsInBattle)
        == crate::pokemon::battle::LOST_BATTLE
}

/// A walk that never arrives is silence, and silence is what the watchdog reads.
const MAX_MOVEMENT_SILENCE: Duration = Duration::from_secs(60);

/// Consecutive ticks the chosen row may be missing from `actions()` before a `NoRoute` abort.
const MAX_ROUTE_LOST_TICKS: u16 = 250;

/// The same bound once `MetaTileMap::row_blocked_by_people` says someone is standing on the route.
const MAX_ROUTE_BLOCKED_TICKS: u16 = 1500;

/// Ticks of successful pacing in grass or on cave floor before giving up on an encounter.
const PACING_BUDGET_TICKS: u16 = 3000;

/// [`PACING_BUDGET_TICKS`] in seconds of game time, for the sentence that reports it.
const PACING_BUDGET_SECS: u64 = {
    let nanos = PACING_BUDGET_TICKS as u64 * AGENT_RESOLUTION.to_duration().as_nanos() as u64;
    (nanos + 500_000_000) / 1_000_000_000
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OverworldActionAbortedReason {
    Unknown,
    Script,
    Battle,
    Textbox,
    NamingScreen,
    WrongMap(Map),
    NoAdjacentGrass,
    NoRoute(MetaTile),
    /// The walk ran for `MAX_MOVEMENT_SILENCE` and never arrived.
    DidNotArrive,
    /// A boulder goal that kept finding a plan and kept shoving without the boulder arriving.
    PuzzleRanLong { pushes: u8 },
    /// A boulder goal whose floor has no solution left from where the player is standing.
    PuzzleUnsolvable,
    /// The grass or cave floor was paced to the end of its budget and nothing turned up.
    NothingAppeared,
    /// The fishing row's walk arrived and the cast was refused before the bag was opened.
    CastRefused(crate::pokemon::postgame::fishing::CastRefusal),
    /// A cast that opened the bag and never saw the rod leave the water.
    CastNeverFinished,
}

impl Display for OverworldActionAbortedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => write!(f, "it stopped making progress"),
            Self::Script => write!(f, "the game took over"),
            Self::Battle => write!(f, "a battle started"),
            // Not "it was interrupted", which reads as a fault.
            Self::Textbox => write!(f, "the game stopped you to say something"),
            Self::NamingScreen => write!(f, "the naming screen opened"),
            Self::WrongMap(map) => write!(f, "it ended up on {map}"),
            Self::NoAdjacentGrass => write!(f, "there is no grass to step into"),
            Self::NoRoute(tile) => write!(f, "there is no route to {tile}"),
            // A time bound, not `NoRoute`: says the walk is what failed.
            Self::DidNotArrive => write!(
                f, "the walk was given up after {} seconds of game time without getting there",
                MAX_MOVEMENT_SILENCE.as_secs()),
            Self::PuzzleUnsolvable => write!(
                f, "no boulder on this floor can be pushed onto it from where you are standing any \
                    more; leaving this floor and coming back puts every boulder where it started"),
            Self::PuzzleRanLong { pushes } => write!(
                f, "the boulder was pushed {pushes} times without reaching its target, so it was \
                    given up; leaving this floor and coming back puts every boulder where it started"),
            // Says what happened, or a model goes hunting for a broken action.
            Self::NothingAppeared => write!(
                f, "nothing appeared after {PACING_BUDGET_SECS} seconds of game time walking about in it"),
            Self::CastRefused(why) => write!(f, "the cast was refused because {why}"),
            Self::CastNeverFinished => write!(
                f, "the cast never finished after {} seconds of game time",
                crate::pokemon::postgame::fishing::CAST_BUDGET_SECS),
        }
    }
}

impl OverworldActionAbortedReason {
    pub fn from_game_mode(game_mode: GameMode) -> Self {
        match game_mode {
            GameMode::Overworld => Self::Unknown,
            GameMode::TrainerBattle | GameMode::WildBattle => Self::Battle,
            GameMode::TextBox => Self::Textbox,
            GameMode::Script => Self::Script,
            GameMode::NamingScreen => Self::NamingScreen,
        }
    }
}

#[derive(Debug)]
pub enum AgentEvent {
    /// A walk was issued.
    StartedOverworldAction { destination: MetaTile, id: String },
    /// `at` is where the walk stopped, in the coordinates the action ids use.
    OverworldActionAborted {
        destination: MetaTile,
        reason: OverworldActionAbortedReason,
        at: Option<Point8>,
    },
    OverworldActionCompleted { destination: MetaTile },
    /// The agent walked to something interactable, pressed A, and the game answered.
    OverworldInteractionCompleted { target: MetaTile },
    /// An item on the ground was walked up to and is still lying there.
    OverworldPickupFailed { target: MetaTile },
    BattleStarted,
    /// `actor` and `opponent` are carried because the host formats events off the emulator thread,
    /// after the battle has moved on.
    BattleActionStarted { actor: String, opponent: String, action: BattleAction },
    BattleEnded,
    TextBox { message: String },
    /// No decision point for `stuck_for` of emulated time, so the watchdog woke the policy.
    WatchdogFired { agent_state: String, stuck_for: Duration },
    /// The game was beaten.
    HallOfFame {
        /// `wNumHoFTeams` after the increment.
        teams: u8,
        /// `HH:MM:SS` off the cartridge's own clock.
        playtime: String,
        playtime_seconds: u32,
        badges: u8,
        /// The winning party, nicknames in slot order.
        party: Vec<String>,
    },
}

impl AgentEvent {
    pub fn text_box_from_reader(reader: &PokemonTextReader) -> Self {
        Self::TextBox { message: reader.to_string() }
    }

    fn is_worth_reporting(&self) -> bool {
        match self {
            Self::TextBox { message } => !message.trim().is_empty(),
            _ => true,
        }
    }
}

impl Display for AgentEvent {
    /// A sentence for the page's log and, via `describe_event`, the model.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentEvent::StartedOverworldAction { destination, .. } =>
                write!(f, "→ heading for {destination}"),
            AgentEvent::OverworldActionAborted { destination, reason, at } => match at {
                Some(at) => write!(f, "✗ gave up on {destination} at ({}, {}): {reason}", at.x, at.y),
                None => write!(f, "✗ gave up on {destination}: {reason}"),
            },
            // A cut is not an arrival.
            AgentEvent::OverworldActionCompleted { destination: MetaTile::Cut { at } } =>
                write!(f, "✓ cut down the tree at ({}, {})", at.x, at.y),
            AgentEvent::OverworldActionCompleted { destination } =>
                write!(f, "✓ reached {destination}"),
            // The page drops this kind: the dialogue that follows says the same.
            AgentEvent::OverworldInteractionCompleted { target } => match target {
                MetaTile::Sprite(name) => write!(f, "✓ talked to {name}"),
                other => write!(f, "✓ used {other}"),
            },
            // Says what is true rather than that something failed.
            AgentEvent::OverworldPickupFailed { target } => match target {
                MetaTile::Sprite(name) => {
                    write!(f, "✗ nothing was picked up: the {name} is still lying there. ")?;
                    write!(f, "The message above says why.")
                }
                other => write!(f, "✗ nothing was picked up from {other}"),
            },
            AgentEvent::BattleStarted =>
                write!(f, "battle started"),
            AgentEvent::BattleActionStarted { actor, opponent, action } => match action {
                // The action being started, not its outcome.
                BattleAction::Fight { battle_move, .. } => write!(f, "{actor} used {} on {opponent}", battle_move.name),
                BattleAction::UseItem { item, .. } if thrown_at_the_enemy(item.id) =>
                    write!(f, "threw a {} at {opponent}", item.id),
                BattleAction::UseItem { item, .. } => write!(f, "used {} on {actor}", item.id),
                BattleAction::SwitchPokemon { pokemon, .. } => write!(f, "sent out {} against {opponent}", pokemon.species),
                BattleAction::Run => write!(f, "{actor} tried to run from {opponent}"),
                BattleAction::SafariBall => write!(f, "threw a Safari Ball at {opponent}"),
                BattleAction::SafariBait => write!(f, "threw bait to {opponent}"),
                BattleAction::SafariRock => write!(f, "threw a rock at {opponent}"),
            },
            AgentEvent::BattleEnded =>
                write!(f, "battle ended"),
            AgentEvent::TextBox { message } =>
                write!(f, "📖 {message}"),
            AgentEvent::WatchdogFired { agent_state, stuck_for } =>
                write!(f, "⚠️ stuck in `{agent_state}` for {}s of game time without asking for a \
                           decision; asking for a nudge", stuck_for.as_secs()),
            AgentEvent::HallOfFame { playtime, badges, party, .. } =>
                write!(f, "🏆 entered the HALL OF FAME with {} badges after {playtime} of play, with {}",
                       badges.count_ones(), party.join(", ")),
        }
    }
}

/// What the player's side of the battle is called, for [`AgentEvent::BattleActionStarted`].
fn active_pokemon_name(state: &GameState) -> String {
    let Some(battle) = state.battle.as_ref() else { return String::new() };
    state
        .pokemon
        .get(battle.active_party_slot as usize)
        .map(|mon| mon.nickname.to_default_string())
        .unwrap_or_else(|| format!("{}", battle.player.species))
}

/// What the other side is called, for [`AgentEvent::BattleActionStarted`].
fn opponent_pokemon_name(state: &GameState) -> String {
    match state.battle.as_ref() {
        Some(battle) => format!("{}", battle.enemy.species),
        // Unreachable from the one caller, but the name goes into a sentence.
        None => "the foe".to_string(),
    }
}

/// Whether the battle menu, or the Safari Zone's, is what the screen shows.
fn battle_menu_is_showing(api: &PokemonApi) -> bool {
    let screen = api.on_screen_text(false).unwrap_or_default();
    (screen.contains("FIGHT") && screen.contains("RUN"))
        || matches!(api.menu_state().and_then(|m| m.battle_menu_state()),
            Some(BattleMenuState::Fight | BattleMenuState::SafariBall
                 | BattleMenuState::SafariBait | BattleMenuState::SafariRock))
}

/// Whether this battle item is aimed at the enemy rather than the active party member.
fn thrown_at_the_enemy(item: crate::pokemon::item::ItemId) -> bool {
    use crate::pokemon::item::ItemId;
    matches!(item, ItemId::PokeBall | ItemId::GreatBall | ItemId::UltraBall | ItemId::MasterBall | ItemId::SafariBall)
}

/// Ticks a freshly opened battle sub-menu is believed from RAM rather than from the screen.
const CONFIRMING_TICKS: u16 = 15;

/// Ticks of B-mashing [`BattleState::WaitingForMenu`] does after the game refuses a move.
const BACKING_OUT_TICKS: u16 = 100;

/// Silence after which [`AgentState::ReadingTextBox`] tries to leave rather than confirm.
const TEXT_BOX_ESCAPE_SILENCE: Duration = Duration::from_secs(30);

/// Ticks after a text box opens in which it may still be a menu the agent inherited, which is
/// closed rather than confirmed.
const MENU_HANDOVER_TICKS: u16 = 50;

/// Silence after which a driver of its own menus ([`drives_its_own_menus`]) is abandoned.
pub(crate) const DRIVER_ESCAPE_SILENCE: Duration = Duration::from_secs(60);

/// Consecutive overworld ticks a finished Surf mount waits for before handing the walk back.
const MOUNT_SETTLE_TICKS: u8 = 15;

/// How long a stopped walk waits to see whether the game walks the player back off the square.
const TURN_BACK_WATCH_TICKS: u16 = 150;

/// A walk stopped by a text box on `tile`, and the square it stepped there from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TurnBackWatch {
    map: Map,
    tile: Point8,
    came_from_raw: Point8,
    ticks: u16,
}

/// Overworld ticks after a pickup's text box closes before asking whether the item is still there.
const PICKUP_SETTLE_TICKS: u16 = 10;

/// Ceiling on [`PokemonAgent::blackout_ticks`].
const MAX_BLACKOUT_WAIT_TICKS: u16 = 1500;

/// What [`BattleState::Navigating`] lined up, so the confirming press is issued once and then
/// checked against what the game selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Confirm {
    /// Handed back to `Navigating` if the cursor is not where it believed.
    action: BattleAction,
    /// The move slot the policy chose.
    slot: u8,
    /// Whether the single confirming press has been issued.
    pressed: bool,
    waited: u16,
    /// `Navigating`'s elapsed budget, carried so a hand-back cannot reset it.
    navigating_ticks: u16,
    /// How many presses have been issued.
    attempts: u8,
}

/// How long the confirm waits for the game to act on its one press before trying again.
const CONFIRM_ACK_TICKS: u16 = 3;

/// Confirming presses before backing out to the main menu.
const CONFIRM_ATTEMPTS: u8 = 12;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum BattleState {
    /// Waiting for the battle menu (TextBoxID 0x0B/0x1B) to appear.
    WaitingForMenu {
        reader: PokemonTextReader,
        delay: DelayContext,
        backing_out: u16,
        confirming: u16,
        /// Set only by [`Self::confirming`], i.e. only when `Navigating` lined a move list up.
        confirm: Option<Confirm>,
    },

    /// Battle menu is up but policy hasn't returned an action yet.
    AwaitingPolicy {
        delay: DelayContext,
        /// Consecutive unanswered ticks the menu has been off the screen.
        menu_gone: u8,
    },

    /// Navigating the menus.
    Navigating {
        action: BattleAction,
        delay: DelayContext,
        ticks: u16,
        /// Consecutive ticks the target menu has been observed.
        stable: u16,
    },

    /// Dedicated driver for using any bag item in battle.
    /// `aim` is the party slot the item goes to, and `entry_hp` that member's HP when it was chosen.
    UsingItem { item: crate::pokemon::item::ItemId, aim: u8, start_qty: u8, entry_hp: u16, press: bool, confirmed: bool, delay: DelayContext, ticks: u16, reader: PokemonTextReader },
}

impl Default for BattleState {
    fn default() -> Self {
        Self::WaitingForMenu {
            reader: PokemonTextReader::message_box_only(),
            delay: DelayContext::default(),
            backing_out: 0,
            confirming: 0,
            confirm: None,
        }
    }
}

impl BattleState {
    /// [`Self::default`], but already latched into backing out of whatever sub-menu is open.
    fn backing_out() -> Self {
        Self::WaitingForMenu {
            reader: PokemonTextReader::message_box_only(),
            delay: DelayContext::default(),
            backing_out: BACKING_OUT_TICKS,
            confirming: 0,
            confirm: None,
        }
    }

    /// [`Self::default`], but starting from whatever a sub-state had already read.
    fn carrying(reader: PokemonTextReader) -> Self {
        Self::WaitingForMenu {
            reader,
            delay: DelayContext::default(),
            backing_out: 0,
            confirming: 0,
            confirm: None,
        }
    }

    /// [`Self::backing_out`], carrying what a sub-state had already read.
    fn backing_out_carrying(reader: PokemonTextReader) -> Self {
        Self::WaitingForMenu {
            reader,
            delay: DelayContext::default(),
            backing_out: BACKING_OUT_TICKS,
            confirming: 0,
            confirm: None,
        }
    }

    /// Whatever a sub-state has read so far, taken out of it.
    fn take_reader(&mut self) -> PokemonTextReader {
        match self {
            // A full-screen reader splices a battle's two HUDs into every message.
            Self::UsingItem { reader, .. } | Self::WaitingForMenu { reader, .. } =>
                std::mem::replace(reader, PokemonTextReader::message_box_only()),
            _ => PokemonTextReader::message_box_only(),
        }
    }

    /// [`Self::default`], told that a sub-menu was just opened on purpose and awaits confirming.
    fn confirming(action: BattleAction, navigating_ticks: u16) -> Self {
        Self::WaitingForMenu {
            reader: PokemonTextReader::message_box_only(),
            delay: DelayContext::default(),
            backing_out: 0,
            confirming: CONFIRMING_TICKS,
            // Only a move list is confirmed by slot.
            confirm: match action {
                BattleAction::Fight { slot, .. } => Some(Confirm { action, slot, pressed: false, waited: 0, attempts: 0, navigating_ticks }),
                _ => None,
            },
        }
    }
}

/// The game's own refusals inside a battle, as they read once fully rendered.
const BATTLE_REFUSALS: &[&str] = &[
    "isn't the time",     // a key item or a TM in battle: "OAK: <PLAYER>! This isn't the time…"
    "won by using",       // "You can't win by using that!"
    "isn't yours",        // someone else's item
    "any effect",         // "It won't have any effect."
    "blocked the BALL",   // a ball thrown at a trainer's Pokémon
    "no will to fight",   // a fainted Pokémon chosen from the party menu
];

/// Whether the battle screen's box is the game talking rather than a menu the agent drives.
fn reading_dialogue(menu_state: &crate::pokemon::menu::MenuState, confirming: u16) -> bool {
    confirming == 0 && menu_state.text_box_id == crate::pokemon::menu::TextBoxId::MessageBox
}

fn shows_battle_refusal(screen: &str) -> bool {
    BATTLE_REFUSALS.iter().any(|r| screen.contains(r))
}

/// States that drive their own menus, under which the generic text reader must not press A.
fn drives_its_own_menus(state: &AgentState) -> bool {
    matches!(state,
        AgentState::PokemartShopping(_) | AgentState::TeachingMove { .. } | AgentState::CuttingTree { .. }
        | AgentState::Surfing { .. } | AgentState::UsingFieldMove { .. } | AgentState::TossingItem { .. }
        | AgentState::UsingItemPc(_) | AgentState::UsingPcBox(_) | AgentState::Flying { .. }
        | AgentState::Fishing(_) | AgentState::SellingToMart(_) | AgentState::UsingPartyScript(_)
        | AgentState::RedeemingPrize(_) | AgentState::CheckingTrashCan { .. } | AgentState::UsingElevator { .. }
        | AgentState::UsingFieldItem { .. } | AgentState::UsingBagItem(_) | AgentState::PushingBoulder { .. })
}

/// State machine for navigating a Pokémart purchase sequence.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum PokemartState {
    /// The Buy/Sell/Quit menu is up and the policy has not said what to buy yet.
    AwaitingPolicy,
    /// Buy/Sell/Quit menu visible.
    ChoosingBuyOption(BagItem),
    /// Item list visible.
    ChoosingItem(BagItem),
    /// A pressed on the target item; waiting for wMaxItemQuantity==99 (qty selector opening).
    AwaitingQtySelector(BagItem),
    /// Quantity selector active (wItemQuantity > 0).
    ChoosingQuantity { item: BagItem, qty_last: u8, stall_ticks: u32 },
    /// Yes/No confirmation visible.
    ConfirmingPurchase(BagItem),
    /// YES was selected — purchase is being processed.
    PurchasedItem { ticks: u32 },
    /// All items bought.
    Quitting,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub(crate) enum AgentState {
    #[default]
    Idle,
    /// Policy returned None for an overworld action — waiting out a delay then re-polling.
    AwaitingOverworldAction { delay: DelayContext },
    OverworldMovement { destination: MetaTile, map: Map },
    ReadingTextBox { reader: PokemonTextReader },
    /// A map script or NPC scripted walk is running.
    RunningScript { rollback_deadline: DelayContext },
    /// The Pokémon nickname entry screen is active.
    NamingPokemon { species: PokemonSpecies, decided: bool, ticks: u16 },

    /// Stepping between two adjacent tiles until a wild battle starts (see `adjacent_pacing_pair`).
    PacingForEncounters { destination: MetaTile, map: Map, tile_a: Point8, tile_b: Point8, heading_to_b: bool, stalled: u16, paced: u16 },

    Battle(BattleState),

    PokemartShopping(PokemartState),

    /// Teaching an HM/TM to a party member: START→ITEM→bag→USE→party.
    TeachingMove { item: crate::pokemon::item::ItemId, target_slot: u8, press: bool, entered_menu: bool, settle: u8, evolve_from: Option<crate::pokemon::species::PokemonSpecies>, declined: bool },

    /// Cut on the tree the player is facing: START→POKéMON→mon→CUT.
    CuttingTree { press: bool, entered_menu: bool, tree_pos: Point8, slot: u8, move_index: u8, from_row: bool },

    /// Mounting Surf before the route steps onto `water_pos`: START→POKéMON→mon→SURF.
    Surfing { press: bool, entered_menu: bool, water_pos: Point8, slot: u8, move_index: u8,
              resume: Option<(MetaTile, Map)>, settle: u8 },

    /// A party field move with no target tile: START→POKéMON→mon at `slot`→entry at `move_index`.
    UsingFieldMove { press: bool, entered_menu: bool, slot: u8, move_index: u8, from_map: Map,
                     resume: Option<(Point8, JoypadButton)>, settle: u8 },

    /// Tossing a bag item to make room: START→ITEM→(bag list)→TOSS→quantity→YES.
    TossingItem { item: crate::pokemon::item::ItemId, press: bool, entered_menu: bool },
    /// Depositing an item into, or withdrawing it from, PC item storage.
    UsingItemPc(crate::pokemon::postgame::item_storage::ItemPcState),
    /// Bill's PC box menus: deposit, withdraw, release, change box.
    UsingPcBox(crate::pokemon::postgame::pc_box::PcBoxState),

    Flying(crate::pokemon::postgame::fly_bike::FlyState),
    /// One rod cast: the walk to the shore, the bag menus, and the wait on "not even a nibble".
    Fishing(crate::pokemon::postgame::fishing::FishState),
    /// Selling a bag item to a mart clerk.
    SellingToMart(crate::pokemon::postgame::game_corner::SellState),
    /// A one-NPC script that opens the party menu on a stale cursor (the Day Care, the Name Rater).
    UsingPartyScript(crate::pokemon::postgame::gifts::PartyScriptState),
    /// Buying a Game Corner prize from a vendor's bespoke menu.
    RedeemingPrize(crate::pokemon::postgame::game_corner::PrizeState),
    /// Using a bag item from the overworld: START→ITEM→bag→USE, then whatever menu the item opens.
    UsingBagItem(crate::pokemon::postgame::items::BagItemState),

    /// Checking a Vermilion Gym trash can: face `target` and press A.
    CheckingTrashCan { target: Point8, checked: bool, press: bool, facing: Option<crate::pokemon::map_metadata::PlayerFacingDirection> },

    /// An elevator panel: face it, pick `floor` from its list, then ride the redirected warp out.
    UsingElevator { panel: Point8, floor: u8, selected: bool, press: bool },

    /// Using a bag item on the sprite at `target`: face it, then START→ITEM→bag→USE.
    UsingFieldItem { item: crate::pokemon::item::ItemId, target: Point8, press: bool, entered_menu: bool, backing_out: u16 },

    /// One Strength shove: stand behind `boulder`, face it and hold `dir`. The dust animation locks
    /// input, so it never over-pushes.
    PushingBoulder { boulder: Point8, dir: JoypadButton, armed: bool },
    /// Carrying out a whole Strength puzzle: push boulders until one sits on `target`.
    SolvingBoulderPuzzle { boulder: Point8, target: Point8, hole: bool, pushes: u8, settle: u16 },
}

impl AgentState {
}

impl Display for PokemartState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PokemartState::AwaitingPolicy          => write!(f, "mart:policy"),
            PokemartState::ChoosingBuyOption(i)   => write!(f, "mart:buy({:?}×{})", i.id, i.quantity),
            PokemartState::ChoosingItem(i)         => write!(f, "mart:item({:?}×{})", i.id, i.quantity),
            PokemartState::AwaitingQtySelector(i)  => write!(f, "mart:await({:?}×{})", i.id, i.quantity),
            PokemartState::ChoosingQuantity { item, .. } => write!(f, "mart:qty({:?}×{})", item.id, item.quantity),
            PokemartState::ConfirmingPurchase(i)   => write!(f, "mart:confirm({:?}×{})", i.id, i.quantity),
            PokemartState::PurchasedItem { .. }    => write!(f, "mart:purchased"),
            PokemartState::Quitting                => write!(f, "mart:quit"),
        }
    }
}

impl AgentState {
    /// The row still open behind this state. The one list of states carrying a row: every exit from
    /// any of them must close it.
    fn open_overworld_action(&self) -> Option<MetaTile> {
        match self {
            AgentState::OverworldMovement { destination, .. }
            | AgentState::PacingForEncounters { destination, .. } => Some(*destination),
            // `resume` is the whole test.
            AgentState::Surfing { resume: Some((destination, _)), .. } => Some(*destination),
            _ => None,
        }
    }
}

impl Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentState::Idle                          => write!(f, "idle"),
            AgentState::AwaitingOverworldAction { .. } => write!(f, "wait"),
            AgentState::OverworldMovement { destination, map } => write!(f, "move→{destination}@{map:?}"),
            AgentState::ReadingTextBox { .. }         => write!(f, "text"),
            AgentState::RunningScript { .. }          => write!(f, "script"),
            AgentState::NamingPokemon { species, ticks, .. } => write!(f, "name:{species:?}@{ticks}"),
            AgentState::PacingForEncounters { .. }    => write!(f, "wander"),
            AgentState::Battle(s) => match s {
                BattleState::WaitingForMenu { .. } => write!(f, "battle:wait"),
                BattleState::AwaitingPolicy { .. } => write!(f, "battle:policy"),
                BattleState::Navigating { action, .. } => write!(f, "battle:{:?}", action),
                BattleState::UsingItem { item, confirmed, .. } => write!(f, "battle:item({item:?},confirmed={confirmed})"),
            },
            AgentState::PokemartShopping(s)           => write!(f, "{s}"),
            AgentState::TeachingMove { item, .. } => write!(f, "teach:{item:?}"),
            AgentState::CuttingTree { .. } => write!(f, "cut"),
            AgentState::Surfing { .. } => write!(f, "surf"),
            AgentState::UsingFieldMove { slot, move_index, .. } => write!(f, "fieldmove:slot{slot}#{move_index}"),
            AgentState::TossingItem { item, .. } => write!(f, "toss:{item:?}"),
            AgentState::UsingItemPc(s)           => write!(f, "itempc:{:?}:{:?}x{}", s.op, s.item, s.qty),
            AgentState::UsingPcBox(s)            => write!(f, "pcbox:{:?}", s.op),
            AgentState::Flying(s)                => write!(f, "fly→{:?}", s.to),
            AgentState::Fishing(s)               => write!(f, "fish:{:?}@{}", s.rod, s.at),
            AgentState::SellingToMart(s)         => write!(f, "sell:{:?}x{}", s.item.id, s.item.quantity),
            AgentState::UsingPartyScript(s)      => write!(f, "{:?}:slot{}", s.script, s.slot),
            AgentState::RedeemingPrize(s)        => write!(f, "prize:{:?}", s.prize),
            AgentState::UsingBagItem(s)          => write!(f, "use:{:?}→{:?}", s.item, s.target),
            AgentState::CheckingTrashCan { target, .. } => write!(f, "trash→{target}"),
            AgentState::UsingElevator { floor, selected, .. } => write!(f, "elevator→floor {floor} (sel={selected})"),
            AgentState::UsingFieldItem { item, .. } => write!(f, "use-item:{item:?}"),
            AgentState::PushingBoulder { boulder, dir, .. } => write!(f, "push-boulder:{boulder}{dir:?}"),
            AgentState::SolvingBoulderPuzzle { target, pushes, .. } => write!(f, "boulder-goal→{target}#{pushes}"),
        }
    }
}

/// The first party slot holding `species`, read from `wPartySpecies` so it works in a menu.
fn party_slot_of(api: &PokemonApi<'_>, species: PokemonSpecies) -> Option<u8> {
    use gb::ram::ROM;
    let count = api.mmu().read_pointer(&pokered_symbols::wPartyCount);
    let base = pokered_symbols::wPartySpecies.address;
    (0..count).find(|i| api.mmu().read(base + *i as u16) == species as u8)
}

/// A party menu a conversation opened, and what the agent answers it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PartyMenuAnswer {
    /// The species an in-game trade asked for; `None` when the agent has no answer.
    give: Option<PokemonSpecies>,
    /// Press/release alternation, so every cursor step is a fresh rising edge.
    press: bool,
    /// The button this menu has already been answered with, once it has been.
    answered: Option<JoypadButton>,
}

pub struct PokemonAgent {
    state: AgentState,
    /// Tick counter for the post-Champion cutscene cadence — see `drive_post_champion_cutscene`.
    post_champion_tick: u32,
    backup_state: Option<AgentState>,
    event_buffer: VecDeque<AgentEvent>,
    cycles: MachineCycles,
    policy: Box<dyn Policy>,
    /// Map graph built incrementally as the player traverses.
    world_graph: WorldGraph,
    /// The Strength goal being carried out: `(map, boulder, target, is_a_hole)`.
    boulder_goal: Option<(Map, Point8, Point8, bool)>,
    /// Shoves the current goal has spent.
    boulder_goal_pushes: u8,
    /// Shortest remaining plan this goal has seen, and shoves made since it last got shorter.
    boulder_goal_best: usize,
    boulder_goal_stale: u8,
    /// Shoves the game never answered ([`DRIVER_ESCAPE_SILENCE`] under `PushingBoulder`).
    boulder_goal_silences: u8,
    /// Consecutive `OverworldMovement` ticks on which the chosen row was absent from `actions()`.
    route_lost_ticks: u16,
    /// `MetaTileMap::row_blocked_by_people` for this walk's row, asked once when
    /// [`MAX_ROUTE_LOST_TICKS`] runs out and remembered until the row comes back.
    route_lost_to_people: bool,
    /// The map the agent was last on, to detect warp and connection landings.
    last_map: Option<Map>,
    /// Trees the agent has cut down, by `(map, expanded tile position)`.
    cut_tiles: std::collections::HashSet<(Map, Point8)>,
    /// The naming screen open now follows a catch, so its closing is the battle's end.
    naming_after_battle: bool,
    /// Silph Co door-graphic walls ($18/$24 that won't open), by `(map, tile position)`.
    blocked_tiles: std::collections::HashSet<(Map, Point8)>,
    /// Consecutive A presses on the card-key door in front of the player.
    door_open_attempts: u32,
    /// Squares this visit to this map has been turned away from, learned rather than known.
    turned_back_tiles: std::collections::HashSet<(Map, Point8)>,
    /// A walk stopped by a text box, waiting to see whether the game puts the player back.
    turn_back_watch: Option<TurnBackWatch>,
    /// The last two raw squares of the walk in flight, as `(previous, current)`.
    walk_squares: Option<(Point8, Point8)>,
    /// Raw presses from `queue_manual_input`, delivered ahead of the state machine.
    manual_input: VecDeque<JoypadButton>,
    /// Ticks left of the press currently being delivered — see [`MANUAL_INPUT_HOLD_TICKS`].
    manual_input_held: u8,
    /// Set once [`AgentState::ReadingTextBox`] stops confirming and starts leaving; B instead of A
    /// until the next decision point.
    escaping_menus: bool,
    /// The next YES/NO menu is answered NO: a quiz row chosen for that answer.
    answer_no: bool,
    /// A vending row chose this drink, by its row in the machine's menu.
    vending_pick: Option<u8>,

    /// Ticks in which a newly opened text box may still be a menu the agent inherited.
    menu_handover_ticks: u16,

    cycles_since_poll: MachineCycles,
    /// [`Policy::stuck_timeout`], read once when the agent was built.
    stuck_after: Option<MachineCycles>,
    /// `cycles_since_poll` at the last [`AgentEvent::WatchdogFired`], to report once per timeout.
    stuck_reported_at: MachineCycles,
    /// Emulated time since the game last answered a driver, in cycles.
    cycles_since_driver_answer: MachineCycles,

    forget_choice: Option<Option<usize>>,

    /// An item ball just pressed A on, waiting for the overworld to ask whether it is still there.
    pending_pickup: Option<(MetaTile, u16)>,
    /// The party menu the conversation in progress opened, if one has.
    party_menu: Option<PartyMenuAnswer>,

    /// Overworld ticks spent waiting out a black-out warp (see [`blackout_in_flight`]).
    blackout_ticks: u16,

    /// `wNumHoFTeams` as of the last tick; see [`Self::check_hall_of_fame`].
    hall_of_fame_teams: Option<u8>,
    /// How far through the ending the cartridge is, once it has started.
    ending: Option<Ending>,
}

/// What the cartridge is doing between winning the game and being playable again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// `wNumHoFTeams` has gone up and `jp Init` has not run: the ceremony, credits and save.
    BeforeTheReset,
    /// The reset has happened — WRAM is cleared and the title screen is up.
    AfterTheReset,
}

impl Default for PokemonAgent {
    fn default() -> Self { Self::new(Box::new(RandomPolicy::default())) }
}

impl PokemonAgent {
    pub fn new(policy: Box<dyn Policy>) -> Self {
        let stuck_after = policy
            .stuck_timeout()
            .filter(|timeout| !timeout.is_zero())
            .map(MachineCycles::from_duration);
        Self {
            cycles_since_poll: MachineCycles::ZERO,
            cycles_since_driver_answer: MachineCycles::ZERO,
            ending: None,
            stuck_after,
            stuck_reported_at: MachineCycles::ZERO,
            state: AgentState::default(),
            post_champion_tick: 0,
            backup_state: None,
            event_buffer: VecDeque::new(),
            cycles: MachineCycles::default(),
            policy,
            world_graph: WorldGraph::new(),
            boulder_goal: None,
            boulder_goal_pushes: 0,
            boulder_goal_best: usize::MAX,
            boulder_goal_stale: 0,
            boulder_goal_silences: 0,
            route_lost_ticks: 0,
            route_lost_to_people: false,
            last_map: None,
            cut_tiles: std::collections::HashSet::new(),
            naming_after_battle: false,
            blocked_tiles: std::collections::HashSet::new(),
            turned_back_tiles: std::collections::HashSet::new(),
            turn_back_watch: None,
            walk_squares: None,
            door_open_attempts: 0,
            manual_input: VecDeque::new(),
            manual_input_held: 0,
            escaping_menus: false,
            answer_no: false,
            vending_pick: None,
            menu_handover_ticks: 0,
            forget_choice: None,
            pending_pickup: None,
            party_menu: None,
            blackout_ticks: 0,
            hall_of_fame_teams: None,
        }
    }

    /// Throw away everything learned about the game so far, keeping only the policy.
    pub fn restart(&mut self, run_dir: Option<&std::path::Path>) {
        self.policy.restart(run_dir);
        self.state = AgentState::default();
        self.backup_state = None;
        self.post_champion_tick = 0;
        self.event_buffer.clear();
        self.cycles = MachineCycles::default();
        self.world_graph = WorldGraph::new();
        self.last_map = None;
        self.cut_tiles.clear();
        self.naming_after_battle = false;
        self.blocked_tiles.clear();
        self.turned_back_tiles.clear();
        self.turn_back_watch = None;
        self.walk_squares = None;
        self.door_open_attempts = 0;
        self.manual_input.clear();
        self.manual_input_held = 0;
        self.cycles_since_poll = MachineCycles::ZERO;
        self.cycles_since_driver_answer = MachineCycles::ZERO;
        self.stuck_reported_at = MachineCycles::ZERO;
        self.blackout_ticks = 0;
        // `None`, not `Some(0)`: the next tick re-seeds from the loaded cartridge.
        self.hall_of_fame_teams = None;
        self.ending = None;
    }

    /// `POST /api/clear` — pass the request straight through to the policy.
    pub fn clear_conversation(&mut self, run_dir: Option<&std::path::Path>) -> Result<(), String> {
        self.policy.clear_conversation(run_dir)
    }

    /// Queue raw button presses, one per agent tick, pre-empting the state machine.
    pub fn queue_manual_input(&mut self, buttons: impl IntoIterator<Item = JoypadButton>) {
        for button in buttons {
            if self.manual_input.len() >= MANUAL_INPUT_CAPACITY { break; }
            self.manual_input.push_back(button);
        }
        self.backup_state = None;
        self.set_state(AgentState::Idle);
    }

    /// Manual presses not yet delivered, including the one held; zero means the state machine runs.
    pub fn manual_input_pending(&self) -> usize {
        self.manual_input.len() + usize::from(self.manual_input_held > 0)
    }

    /// The incrementally-built world graph (exposed for tests/inspection).
    pub fn world_graph(&self) -> &WorldGraph {
        &self.world_graph
    }

    pub fn policy_exhausted(&self) -> bool {
        self.policy.is_exhausted()
    }

    pub fn policy_steps_remaining(&self) -> Option<usize> {
        self.policy.steps_remaining()
    }

    pub fn policy_current_step_is_long_running(&self) -> bool {
        self.policy.current_step_is_long_running()
    }

    /// True while a battle is fought, when the policy queue cannot advance.
    pub fn in_battle(&self) -> bool {
        matches!(self.state, AgentState::Battle(_))
    }

    /// Drains all buffered events and returns them.
    pub fn drain_events(&mut self) -> Vec<AgentEvent> {
        self.event_buffer.drain(..).collect()
    }

    /// Human-readable description of the agent's current state (for debugging/tests).
    pub fn state_debug(&self) -> String {
        format!("{}", self.state)
    }

    /// Every decision point goes through here, and it resets the clock the watchdog reads.
    fn poll_policy(&mut self, game_state: &GameState, api: &mut PokemonApi) {
        self.cycles_since_poll = MachineCycles::ZERO;
        // A decision point means no driver is wedged.
        self.cycles_since_driver_answer = MachineCycles::ZERO;
        // A decision point is what "out of the menus" means.
        self.escaping_menus = false;
        self.stuck_reported_at = MachineCycles::ZERO;
        // A decision point ends the deferral this counts.
        self.blackout_ticks = 0;
        self.policy.service_tools(game_state, api, &self.world_graph);
    }

    /// [`Policy::name`], reported on every heartbeat and in a finished run's record.
    pub fn policy_name(&self) -> &'static str {
        self.policy.name()
    }

    /// The policy's name for the player on a new game; `None` keeps the starting state's.
    pub fn policy_player_name(&self) -> Option<String> {
        self.policy.player_name()
    }

    /// Emulated time without a decision point: zero on any tick that polled, whatever the answer.
    pub fn since_last_policy_poll(&self) -> Duration {
        self.cycles_since_poll.to_duration()
    }

    /// A shove of a boulder goal landed: count it, and record that the game answered.
    fn boulder_shove_landed(&mut self) {
        // The game moved a boulder, which is the only answer this driver ever gets.
        self.cycles_since_driver_answer = MachineCycles::ZERO;
        self.boulder_goal_pushes = self.boulder_goal_pushes.saturating_add(1);
        self.boulder_goal_stale = self.boulder_goal_stale.saturating_add(1);
    }

    /// Emulated time since the game last answered a driver: `DRIVER_ESCAPE_SILENCE`'s clock, not
    /// [`Self::since_last_policy_poll`]'s.
    pub fn since_driver_answer(&self) -> Duration {
        self.cycles_since_driver_answer.to_duration()
    }

    /// The watchdog: ask the policy for a nudge when nothing has asked it anything.
    fn run_watchdog(&mut self, api: &mut PokemonApi) -> bool {
        let Some(after) = self.stuck_after.filter(|after| self.cycles_since_poll >= *after) else {
            return false;
        };
        // A failed read must never make the watchdog break the tick.
        let Ok(game_state) = api.game_state() else { return false };

        let agent_state = self.state_debug();
        let stuck_for = self.cycles_since_poll.to_duration();
        if self.cycles_since_poll >= self.stuck_reported_at + after {
            self.stuck_reported_at = self.cycles_since_poll;
            self.event(AgentEvent::WatchdogFired { agent_state: agent_state.clone(), stuck_for });
        }

        let jam = Jam { agent_state: &agent_state, stuck_for };
        self.policy.service_tools(&game_state, api, &self.world_graph);
        self.policy.pick_unstick(&game_state, jam);
        true
    }

    /// Notice the game being beaten, and say so exactly once.
    fn check_hall_of_fame(&mut self, api: &mut PokemonApi) {
        use crate::pokemon::encoding::PokemonEncoding;
        use crate::pokemon::symbols::pokered_symbols as sym;
        let teams = api.mmu().read_pointer(&sym::wNumHoFTeams);
        match self.hall_of_fame_teams {
            None => {
                self.hall_of_fame_teams = Some(teams);
                return;
            }
            Some(seen) if teams > seen => self.hall_of_fame_teams = Some(teams),
            Some(_) => return,
        }

        // Read now: by the time the host formats the event, the ceremony has cleared the party.
        let playtime = crate::pokemon::observe::playtime(api);
        let playtime_seconds = crate::pokemon::observe::playtime_seconds(api);
        let badges = api.mmu().read_pointer(&sym::wObtainedBadges);
        let party = api
            .mmu()
            .read_player_pokemon_party()
            .map(|party| party.iter().map(|mon| mon.nickname.to_default_string()).collect())
            .unwrap_or_default();
        self.event(AgentEvent::HallOfFame { teams, playtime, playtime_seconds, badges, party });

        // From this frame the world in RAM is one the player has left.
        self.ending = Some(Ending::BeforeTheReset);
        api.release_all_buttons();
        self.backup_state = None;
        self.set_state(AgentState::Idle);
    }

    /// Is the cartridge between the end of one game and the start of the next?
    fn the_world_has_been_left(&mut self, api: &PokemonApi) -> bool {
        match self.ending {
            None => false,
            Some(Ending::BeforeTheReset) => {
                if !api.a_game_is_loaded() {
                    self.ending = Some(Ending::AfterTheReset);
                }
                true
            }
            Some(Ending::AfterTheReset) if api.a_game_is_loaded() => {
                // A save was loaded (CONTINUE) or a new game was named.
                self.ending = None;
                false
            }
            Some(Ending::AfterTheReset) => true,
        }
    }

    /// Emit an event: to the policy first, then to the buffer the host drains.
    pub(crate) fn event(&mut self, event: AgentEvent) {
        if !event.is_worth_reporting() {
            return;
        }
        println!("{:?}", event);
        self.policy.on_event(&event);
        self.event_buffer.push_back(event);
        while self.event_buffer.len() > 100 {
            self.event_buffer.pop_front();
        }
    }

    /// Report whatever the text reader has collected, and forget it.
    fn flush_text_reader(&mut self) {
        let AgentState::ReadingTextBox { reader } = &mut self.state else { return };
        let message = reader.take();
        // Most are empty, detected before their characters are drawn; `event` drops those.
        self.event(AgentEvent::TextBox { message });
    }

    fn backup_current_state(&mut self, new_state: AgentState) {
        // Before the clone, not after.
        self.flush_text_reader();
        self.backup_state = Some(self.state.clone());
        self.state = new_state;
    }

    fn restore_state_from_backup(&mut self) {
        if self.backup_state.is_none() {
            return;
        }
        self.state = self.backup_state.clone().unwrap();
        self.backup_state = None;
    }

    /// Drive the manual-input queue, returning true if this tick belonged to it.
    fn drive_manual_input(&mut self, api: &mut PokemonApi) -> bool {
        if self.manual_input_held > 0 {
            self.manual_input_held -= 1;
            if self.manual_input_held == 0 {
                api.release_all_buttons();
            }
            return true;
        }
        match self.manual_input.pop_front() {
            Some(button) => {
                api.release_all_buttons();
                api.press_button(button);
                self.manual_input_held = MANUAL_INPUT_HOLD_TICKS;
                true
            }
            None => false,
        }
    }

    /// Press A on a slow cadence through the post-Champion script chain; true if the tick was its.
    fn drive_post_champion_cutscene(&mut self, api: &mut PokemonApi) -> bool {
        use crate::pokemon::symbols::pokered_symbols as sym;
        // Not RIVAL_DEFEATED: the policy needs one ordinary tick in the rival's concession text to
        // pop its `BattleTrainer` step.
        const SCRIPT_OAK_ARRIVES: u8 = 4;
        const CYCLE: u32 = 75;
        const PRESS_AT: u32 = 50;
        const RELEASE_AT: u32 = 55;

        if api.game_state().map_or(true, |s| s.map.map != Map::ChampionsRoom) {
            self.post_champion_tick = 0;
            return false;
        }
        if api.mmu().read_pointer(&sym::wChampionsRoomCurScript) < SCRIPT_OAK_ARRIVES {
            self.post_champion_tick = 0;
            return false;
        }
        let phase = self.post_champion_tick % CYCLE;
        self.post_champion_tick = self.post_champion_tick.wrapping_add(1);
        match phase {
            0 => api.release_all_buttons(),
            PRESS_AT => api.press_button(JoypadButton::A),
            RELEASE_AT => api.release_all_buttons(),
            _ => {}
        }
        true
    }

    /// The game state with cut trees cleared and learned walls overlaid.
    pub(crate) fn observe_state(&self, api: &PokemonApi) -> Result<crate::pokemon::GameState, String> {
        let mut state = api.game_state()?;
        for &(map, pos) in &self.cut_tiles {
            if state.map.map == map {
                let idx = pos.x as usize + pos.y as usize * state.map.width;
                if state.map.meta_tiles.get(idx) == Some(&MetaTile::CutTree) {
                    state.map.meta_tiles[idx] = MetaTile::Empty;
                }
            }
        }
        // Door walls found at run time and squares this visit was turned back off are obstacles.
        for &(map, pos) in self.blocked_tiles.iter().chain(self.turned_back_tiles.iter()) {
            if state.map.map == map {
                let idx = pos.x as usize + pos.y as usize * state.map.width;
                if idx < state.map.meta_tiles.len() {
                    state.map.meta_tiles[idx] = MetaTile::Obstacle;
                }
            }
        }
        Ok(state)
    }

    /// A presses on a card-key door before treating it as a wall.
    const DOOR_OPEN_ATTEMPTS: u32 = 40;

    /// On Silph Co, the card-key-door tile ($18/$24) the player faces: a door or a wall that
    /// `ReplaceTileBlock` put in at run time, invisible to the `MetaTileMap`.
    fn handle_card_key_door(&mut self, api: &mut PokemonApi) -> bool {
        use crate::pokemon::map_metadata::{map_has_card_key_doors, PlayerFacingDirection};
        let Ok(state) = api.game_state() else { return false; };
        if !map_has_card_key_doors(state.map.map) { self.door_open_attempts = 0; return false; }
        let front = api.mmu().read_pointer(&pokered_symbols::wTileInFrontOfPlayer);
        // The Silph 11F gate is $5e, which `PrintCardKeyText` special-cases.
        let is_11f_gate = front == 0x5e && state.map.map == Map::SilphCo11F;
        if front != 0x18 && front != 0x24 && !is_11f_gate { self.door_open_attempts = 0; return false; }
        let p = state.map.player_position;
        let faced = match state.map.player_direction {
            PlayerFacingDirection::Up    => Point8 { x: p.x, y: p.y.wrapping_sub(1) },
            PlayerFacingDirection::Down  => Point8 { x: p.x, y: p.y + 1 },
            PlayerFacingDirection::Left  => Point8 { x: p.x.wrapping_sub(1), y: p.y },
            PlayerFacingDirection::Right => Point8 { x: p.x + 1, y: p.y },
        };
        // Giving up has to be remembered, or it is not giving up.
        if self.blocked_tiles.contains(&(state.map.map, faced)) {
            self.door_open_attempts = 0;
            return false;
        }
        self.door_open_attempts += 1;
        if self.door_open_attempts > Self::DOOR_OPEN_ATTEMPTS {
            // Won't open: a decorative wall.
            self.blocked_tiles.insert((state.map.map, faced));
            self.door_open_attempts = 0;
            false
        } else {
            // `PrintCardKeyText` opens a faced door while the Card Key is in the bag.
            api.toggle_button(JoypadButton::A);
            true
        }
    }

    pub(crate) fn set_state(&mut self, state: AgentState) {
        if self.state != state {
            // Whatever the reader had collected, said before its state goes.
            self.flush_text_reader();
            self.state = state;

            if self.state != AgentState::Idle {
                println!("{}", &self.state);
            }
        }
    }

    fn set_battle_state(&mut self, state: BattleState) {
        self.set_state(AgentState::Battle(state));
    }

    fn set_pokemart_state(&mut self, state: PokemartState) {
        self.set_state(AgentState::PokemartShopping(state));
    }

    fn drive_forget_menu(&mut self, api: &mut PokemonApi, cursor_index: u8) -> Result<(), String> {
        let game_state = api.game_state()?;
        self.poll_policy(&game_state, api);
        let which = api.learning_pokemon_index();
        let current_moves: Vec<_> = game_state.pokemon.get(which)
            .map(|p| p.moves.iter().flatten().copied().collect())
            .unwrap_or_default();
        match api.move_to_learn() {
            // Asked once per menu, then held.
            Some(new_move) => match match self.forget_choice {
                Some(held) => Some(held),
                None => {
                    let answer = self.policy.pick_move_to_forget(which, &current_moves, new_move);
                    self.forget_choice = answer;
                    answer
                }
            } {
                None => {} // still deciding — wait
                Some(Some(slot)) => {
                    let slot = slot as u8;
                    if cursor_index < slot { api.toggle_button(JoypadButton::Down); }
                    else if cursor_index > slot { api.toggle_button(JoypadButton::Up); }
                    else { api.toggle_button(JoypadButton::A); }
                }
                Some(None) => api.toggle_button(JoypadButton::B), // decline (best effort)
            },
            None => api.toggle_button(JoypadButton::A),
        }
        Ok(())
    }

    /// Where the player is, in the expanded coordinates the model reads everywhere else.
    pub(crate) fn player_at(&self, api: &PokemonApi) -> Option<Point8> {
        self.observe_state(api).ok().map(|state| state.map.player_position)
    }

    /// `pub(crate)` for the fishing driver, the tail of an overworld action.
    pub(crate) fn abort_overworld(
        &mut self,
        destination: MetaTile,
        reason: OverworldActionAbortedReason,
        at: Option<Point8>,
    ) {
        self.event(AgentEvent::OverworldActionAborted { destination, reason, at });
        self.set_state(AgentState::Idle);
    }

    pub fn take_overworld_action(&mut self, action: OverworldAction) {
        self.answer_no = matches!(action.tile,
            MetaTile::Switch { object: crate::pokemon::tile::HiddenObject::Quiz { yes: false }, .. });
        self.vending_pick = match action.tile {
            MetaTile::Switch { object: crate::pokemon::tile::HiddenObject::VendingMachine, ordinal } => Some(ordinal - 1),
            _ => None,
        };
        self.event(AgentEvent::StartedOverworldAction {
            destination: action.tile.clone(),
            id: action.id(),
        });
        // A fresh walk has no square behind it yet.
        self.walk_squares = None;
        // The previous walk's missing-row streak says nothing about this one.
        self.route_lost_ticks = 0;
        self.route_lost_to_people = false;
        self.set_state(AgentState::OverworldMovement { destination: action.tile, map: action.map });
    }

    /// Hold still for another tick rather than saying there is no route, and count this one.
    fn wait_for_the_route(&mut self, map: &crate::pokemon::tile_map::MetaTileMap, row: MetaTile) -> bool {
        self.route_lost_ticks = self.route_lost_ticks.saturating_add(1);
        if self.route_lost_ticks <= MAX_ROUTE_LOST_TICKS {
            return true;
        }
        if self.route_lost_ticks == MAX_ROUTE_LOST_TICKS + 1 {
            self.route_lost_to_people = map.row_blocked_by_people(row);
        }
        self.route_lost_to_people && self.route_lost_ticks <= MAX_ROUTE_BLOCKED_TICKS
    }

    /// Close the walk a Surf mount was carrying when the mount has ended on a different map.
    fn surf_crossed_into(&mut self, destination: MetaTile, now: Map) {
        let arrived = matches!(destination,
            MetaTile::Warp { .. } | MetaTile::Connection { .. } | MetaTile::ConnectionWater(_));
        match arrived {
            true => self.event(AgentEvent::OverworldActionCompleted { destination }),
            false => self.abort_overworld(
                destination, OverworldActionAbortedReason::WrongMap(now), None),
        }
        self.set_state(AgentState::Idle);
    }

    /// Checks if a battle has just started or finished.
    fn assert_battle_state(&mut self, game_mode: GameMode) {
        if matches!(game_mode, GameMode::WildBattle | GameMode::TrainerBattle) {
            match self.state {
                AgentState::Battle(_) => {}
                // The nickname screen after a catch runs while `wIsInBattle` is still 1.
                AgentState::NamingPokemon { .. } => {}
                // Every state that carries a walk.
                _ if self.state.open_overworld_action().is_some() => {
                    let d = self.state.open_overworld_action().expect("just checked");
                    self.abort_overworld(d, OverworldActionAbortedReason::Battle, None);
                    self.event(AgentEvent::BattleStarted);
                    self.set_battle_state(BattleState::default());
                }
                // A bite ends a cast the same way.
                AgentState::Fishing(state) => {
                    self.abort_overworld(
                        MetaTile::Fish { rod: state.rod },
                        OverworldActionAbortedReason::Battle, None);
                    self.event(AgentEvent::BattleStarted);
                    self.set_battle_state(BattleState::default());
                }
                _ => {
                    self.event(AgentEvent::BattleStarted);
                    self.set_battle_state(BattleState::default());
                }
            }
        } else if let AgentState::Battle(battle_state) = &self.state {
            if let BattleState::WaitingForMenu { reader, .. } = battle_state {
                self.event(AgentEvent::text_box_from_reader(reader));
            }

            self.event(AgentEvent::BattleEnded);
            self.set_state(AgentState::Idle);
            // A battle reloads the map, so any tree cut on it has regrown.
            self.cut_tiles.clear();
        }
    }

    /// The naming screen is gone; after a catch that is also the battle gone.
    fn naming_closed(&mut self) {
        self.set_state(AgentState::Idle);
        if std::mem::take(&mut self.naming_after_battle) {
            self.event(AgentEvent::BattleEnded);
            // A battle reloads the map, so any tree cut on it has regrown.
            self.cut_tiles.clear();
        }
    }

    /// Checks if the naming screen has just opened or closed.
    fn assert_naming_screen(&mut self, game_mode: GameMode, api: &mut PokemonApi) -> Result<(), String> {
        if game_mode == GameMode::NamingScreen {
            if !matches!(self.state, AgentState::NamingPokemon { .. }) {
                let species = api.naming_screen_species()?;
                api.release_all_buttons();
                // A catch: the battle ends when this screen closes, and what it said ends here.
                if let AgentState::Battle(battle_state) = &self.state {
                    if let BattleState::WaitingForMenu { reader, .. } = battle_state {
                        self.event(AgentEvent::text_box_from_reader(reader));
                    }
                    self.naming_after_battle = true;
                }
                self.set_state(AgentState::NamingPokemon { species, decided: false, ticks: 0 });
            }
        } else if matches!(self.state, AgentState::NamingPokemon { decided: false, .. }) {
            // Closed before the policy decided.
            self.naming_closed();
        }
        // Once decided, the naming screen reads as `TextBox`, so this does not fire.

        Ok(())
    }

    /// A script is believed only once it outlasts a rollback deadline; a shorter one restores the
    /// state it interrupted.
    fn assert_script_state(&mut self, game_mode: GameMode) {
        // These drivers run the script they trigger themselves.
        if matches!(self.state, AgentState::CheckingTrashCan { .. } | AgentState::UsingElevator { .. }
            | AgentState::UsingFieldMove { .. } | AgentState::Surfing { .. }) {
            return;
        }
        if game_mode == GameMode::Script {
            if !matches!(self.state, AgentState::RunningScript { .. }) {
                // A ledge jump fires Script for up to ~660 ms, so a walk waits the long deadline.
                let rollback_delay = match self.state {
                    AgentState::OverworldMovement { .. } | AgentState::PacingForEncounters { .. } => DelayContext::long(),
                    _ => DelayContext::short(),
                };

                self.backup_current_state(AgentState::RunningScript { rollback_deadline: rollback_delay });
            }
        } else if let AgentState::RunningScript { rollback_deadline: rollback_delay } = self.state {
            if rollback_delay.is_exhausted() {
                self.backup_state = None;
                // A Strength goal's shove comes back here and goes back to the solver.
                if let Some((_, boulder, target, hole)) = self.boulder_goal {
                    self.boulder_shove_landed();
                    let pushes = self.boulder_goal_pushes;
                    self.set_state(AgentState::SolvingBoulderPuzzle { boulder, target, hole, pushes, settle: 0 });
                    return;
                }
                self.set_state(AgentState::AwaitingOverworldAction {
                    delay: DelayContext::default(),
                });
            } else {
                // Transient, such as a ledge jump.
                self.restore_state_from_backup();
            }
        }
    }

    /// The Buy/Sell/Quit menu is up: ask what to buy.
    fn ask_mart_policy(&mut self, game_state: &GameState, api: &mut PokemonApi) {
        self.poll_policy(game_state, api);
        let Some(item) = self.policy.pick_mart_purchase(game_state) else { return };
        // Trimmed to the wallet, as every later purchase of the visit is.
        match item.and_then(|item| affordable(api, game_state.money, item)) {
            Some(item) => self.set_pokemart_state(PokemartState::ChoosingBuyOption(item)),
            None => self.set_pokemart_state(PokemartState::Quitting),
        }
    }

    /// Runs before `assert_text_box_state` so the mart flow takes priority.
    fn assert_pokemart_state(&mut self, game_mode: GameMode, api: &mut PokemonApi) -> Result<(), String> {
        if game_mode != GameMode::TextBox {
            if matches!(self.state, AgentState::PokemartShopping(_)) {
                api.release_all_buttons();
                self.set_state(AgentState::Idle);
            }
            return Ok(());
        }

        if let Some(menu) = api.menu_state() {
            // `SellingToMart` drives this menu itself; `PokemartState` would answer BUY.
            if menu.is_mart_buy_sell_menu() && !matches!(self.state, AgentState::PokemartShopping(_) | AgentState::SellingToMart(_)) {
                api.release_all_buttons();
                self.set_state(AgentState::PokemartShopping(PokemartState::AwaitingPolicy));
                let game_state = api.game_state()?;
                self.ask_mart_policy(&game_state, api);
            }
        }

        Ok(())
    }

    /// Whether the new text box answers the walk's interaction rather than interrupting it.
    fn interaction_landed(&self, destination: MetaTile, api: &PokemonApi) -> bool {
        if !matches!(destination, MetaTile::Sprite(_) | MetaTile::Pc | MetaTile::Switch { .. }) {
            return false;
        }
        let Ok(state) = self.observe_state(api) else { return false };
        let Some((at, tile)) = state.map.interaction_in_front() else { return false };
        match destination {
            MetaTile::Sprite(_) => tile == destination,
            // A PC is not in `meta_tiles` and the tile in front reads as `Obstacle`.
            MetaTile::Pc => crate::pokemon::tile_map::pc_locations_for(state.map.map).contains(&at),
            // Hidden events drawn as the scenery they hide in: only the coordinate identifies one.
            MetaTile::Switch { object, ordinal } => crate::pokemon::tile_map::hidden_objects_for(state.map.map)
                .iter()
                .enumerate()
                .any(|(index, site)| site.at == at && site.object == object && index as u8 + 1 == ordinal),
            _ => false,
        }
    }

    /// Is `destination` a sprite drawn as a Poké Ball?
    fn is_item_ball(&self, destination: MetaTile, api: &PokemonApi) -> bool {
        let MetaTile::Sprite(name) = destination else { return false };
        let Ok(state) = self.observe_state(api) else { return false };
        state
            .map
            .sprites
            .iter()
            .any(|sprite| sprite.name == name && sprite.picture_id == crate::pokemon::sprite::PictureId::PokeBall)
    }

    /// The overworld is back after a pickup: is the ball still on the floor?
    fn check_pending_pickup(&mut self, api: &PokemonApi) {
        let Some((target, settle)) = self.pending_pickup else { return };
        if settle > 0 {
            self.pending_pickup = Some((target, settle - 1));
            return;
        }
        let Ok(state) = self.observe_state(api) else { return };
        self.pending_pickup = None;
        let MetaTile::Sprite(name) = target else { return };
        // A picked-up missable object is still in the list, but hidden.
        if state.map.sprites.iter().any(|sprite| sprite.name == name && !sprite.hidden) {
            self.event(AgentEvent::OverworldPickupFailed { target });
        }
    }

    /// Answer an armed [`TurnBackWatch`]: has the cartridge walked the player back off the square?
    fn check_turn_back(&mut self, api: &PokemonApi) {
        let Some(watch) = self.turn_back_watch.as_mut() else { return };
        if watch.ticks == 0 || self.last_map != Some(watch.map) {
            self.turn_back_watch = None;
            return;
        }
        watch.ticks -= 1;
        if matches!(self.state, AgentState::OverworldMovement { .. }) { return }
        if api.raw_player_coords() != watch.came_from_raw { return }
        let (map, tile) = (watch.map, watch.tile);
        self.turn_back_watch = None;
        if self.turned_back_tiles.insert((map, tile)) {
            self.event(AgentEvent::TextBox { message: format!(
                "the game walked you back off {tile}, so it is being treated as a wall until you \
                 leave {map} and come back") });
        }
    }

    fn assert_text_box_state(&mut self, game_mode: GameMode, api: &PokemonApi) {
        // `NamingPokemon { decided: true }` handles its own exit.
        if matches!(self.state, AgentState::NamingPokemon { decided: true, .. }) {
            return;
        }
        if game_mode == GameMode::TextBox {
            if !matches!(self.state, AgentState::ReadingTextBox { .. }) {
                if let AgentState::PacingForEncounters { destination, .. } = self.state {
                    let at = self.player_at(api);
                    self.abort_overworld(destination, OverworldActionAbortedReason::Textbox, at);
                } else if let AgentState::OverworldMovement { destination, .. } = self.state {
                    // Talking to someone is this action succeeding.
                    if self.interaction_landed(destination, api) {
                        if self.is_item_ball(destination, api) {
                            self.pending_pickup = Some((destination, PICKUP_SETTLE_TICKS));
                        }
                        // Armed on who, because nothing else says a trade is happening.
                        if let MetaTile::Sprite(who) = destination
                            && let Some(map) = self.last_map
                            && let Some(trade) = crate::pokemon::postgame::trades::trade_at(map, who)
                        {
                            self.party_menu = Some(PartyMenuAnswer {
                                give: Some(trade.give), press: true, answered: None,
                            });
                        }
                        self.event(AgentEvent::OverworldInteractionCompleted { target: destination });
                    } else {
                        let at = self.player_at(api);
                        // Being stopped may mean "you may not stand there".
                        if let (Some(at), Some((came_from_raw, current_raw)), Some(map))
                            = (at, self.walk_squares, self.last_map)
                            // Without a step, "back where you came from" is the current square.
                            && came_from_raw != current_raw
                        {
                            self.turn_back_watch = Some(TurnBackWatch {
                                map, tile: at, came_from_raw, ticks: TURN_BACK_WATCH_TICKS });
                        }
                        self.abort_overworld(destination, OverworldActionAbortedReason::Textbox, at);
                    }
                }
                // Which reader is a fact about the game, not the agent's state.
                let in_battle = crate::pokemon::battle::BattleStateReader::read_battle_state(api.mmu())
                    .is_some();
                let reader = match in_battle {
                    true => PokemonTextReader::message_box_only(),
                    false => PokemonTextReader::default(),
                };
                // A menu the agent did not open is closed, not read as a conversation.
                self.menu_handover_ticks = MENU_HANDOVER_TICKS;
                self.set_state(AgentState::ReadingTextBox { reader });
            }
        } else if matches!(self.state, AgentState::ReadingTextBox { .. }) {
            self.party_menu = None;
            self.set_state(AgentState::Idle);
        }
    }

    /// Advance `gb` by at least `min_cycles`, ticking the state machine every [`AGENT_RESOLUTION`].
    /// Returns what was emulated and the last tick's result.
    pub fn run(&mut self, gb: &mut gb::game_boy::GameBoy,
               cache: &mut crate::pokemon::map_metadata::MapMetadataCache,
               min_cycles: MachineCycles) -> (MachineCycles, Result<(), String>) {
        let mut ran = MachineCycles::ZERO;
        let mut owed = min_cycles;
        let mut result = Ok(());
        while owed > MachineCycles::ZERO {
            // `gb.run` finishes its instruction, so `owed` hits zero only once all has run.
            let slice = gb.run(owed.min(AGENT_RESOLUTION));
            ran += slice;
            owed = owed.saturating_sub(slice);
            let mut api = PokemonApi::with_cache(gb, cache);
            result = self.update(&mut api, slice);
        }
        (ran, result)
    }

    pub fn update(&mut self, api: &mut PokemonApi, delta_cycles: MachineCycles) -> Result<(), String> {
        self.cycles += delta_cycles;
        if self.cycles < AGENT_RESOLUTION { return Ok(()); }

        let mut delta_cycles = MachineCycles::ZERO;
        while self.cycles >= AGENT_RESOLUTION {
            delta_cycles += AGENT_RESOLUTION;
            self.cycles -= AGENT_RESOLUTION;
        }
        self.cycles_since_poll += delta_cycles;
        self.cycles_since_driver_answer += delta_cycles;

        // The policy's escape hatch.
        let queued = self.policy.take_manual_input();
        if !queued.is_empty() {
            self.queue_manual_input(queued);
        }
        if self.drive_manual_input(api) {
            return Ok(());
        }

        self.check_hall_of_fame(api);
        if self.the_world_has_been_left(api) {
            return Ok(());
        }

        self.run_watchdog(api);

        self.check_turn_back(api);

        let game_mode = api.game_mode()
            .ok_or_else(|| "Not in game".to_string())?;

        // Beating the rival hands the game to a five-stage script chain ending in the Hall of Fame.
        if self.drive_post_champion_cutscene(api) {
            return Ok(());
        }

        // Silph Co's card-key doors are placed at run time, invisible to the `MetaTileMap`.
        if game_mode == GameMode::Overworld && self.handle_card_key_door(api) {
            return Ok(());
        }

        // A trainer has seen the player and the battle is starting on its own.
        if api.trainer_battle_pending() && game_mode != GameMode::TextBox {
            api.release_all_buttons();
            return Ok(());
        }

        {
            // Detected by its prompt, not stale menu geometry.
            let forget_showing = api.menu_state().is_some()
                && api.on_screen_text(true).map_or(false, |t| is_forget_move_prompt(&t));
            if forget_showing {
                let cursor = api.menu_geometry().2;
                self.drive_forget_menu(api, cursor)?;
                return Ok(());
            }
            self.forget_choice = None;
        }

        self.assert_naming_screen(game_mode, api)?;
        self.assert_script_state(game_mode);
        self.assert_battle_state(game_mode);
        self.assert_pokemart_state(game_mode, api)?;
        if !drives_its_own_menus(&self.state) {
            self.assert_text_box_state(game_mode, api);
        }

        // Below the asserts, so the text box and any script behind it have finished.
        if game_mode == GameMode::Overworld {
            self.check_pending_pickup(api);
        }

        // The net under every driver that drives its own menus.
        if drives_its_own_menus(&self.state)
            && self.cycles_since_driver_answer.to_duration() >= DRIVER_ESCAPE_SILENCE {
            let abandoned = format!("{} got no answer from the game for {:?}; starting over",
                                    self.state, DRIVER_ESCAPE_SILENCE);
            // Starting over is harmless only where something else is counting.
            if self.boulder_goal.is_some() && matches!(self.state, AgentState::PushingBoulder { .. }) {
                self.boulder_goal_silences = self.boulder_goal_silences.saturating_add(1);
            }
            api.release_all_buttons();
            self.set_state(AgentState::Idle);
            self.event(AgentEvent::TextBox { message: abandoned });
            return Ok(());
        }

        let mut new_events: Vec<AgentEvent> = vec!();

        match self.state {
            AgentState::Idle => {
                api.release_all_buttons();
                if game_mode == GameMode::Overworld
                    && let Some((map, boulder, target, hole)) = self.boulder_goal
                {
                    let here = self.observe_state(api)?;
                    if here.map.map == map {
                        let pushes = self.boulder_goal_pushes;
                        self.set_state(AgentState::SolvingBoulderPuzzle { boulder, target, hole, pushes, settle: 0 });
                        return Ok(());
                    }
                    self.boulder_goal = None;
                }
                match game_mode {
                    GameMode::TextBox => {
                        self.set_state(AgentState::ReadingTextBox { reader: PokemonTextReader::default() });
                    }
                    GameMode::Script => { /* assert_script_state moves to RunningScript */ }
                    GameMode::NamingScreen => {
                        self.set_state(AgentState::NamingPokemon {
                            species: api.naming_screen_species()?,
                            decided: false,
                            ticks: 0,
                        });
                    }
                    GameMode::WildBattle | GameMode::TrainerBattle => {
                        self.set_battle_state(BattleState::default());
                    }
                    GameMode::Overworld => {
                        self.set_state(AgentState::AwaitingOverworldAction { delay: DelayContext::long() });
                    }
                }
            }
            AgentState::RunningScript { rollback_deadline: ref mut rollback_delay } => {
                // An arrow tile is the walk, not a script that ended it.
                if player_is_spinning(api) {
                    *rollback_delay = DelayContext::long();
                }
                if rollback_delay.is_exhausted() {
                    api.toggle_button(JoypadButton::A);
                } else {
                    // Inside the rollback window: maybe a ledge jump.
                    let crossed = rollback_delay.tick(delta_cycles);
                    api.release_all_buttons();
                    if crossed {
                        if let Some(destination) =
                            self.backup_state.as_ref().and_then(AgentState::open_overworld_action)
                        {
                            let at = self.player_at(api);
                            self.event(AgentEvent::OverworldActionAborted {
                                destination,
                                reason: OverworldActionAbortedReason::Script,
                                at,
                            });
                        }
                    }
                }
            }
            AgentState::AwaitingOverworldAction { ref mut delay } => {
                let warping = blackout_in_flight(api) && self.blackout_ticks < MAX_BLACKOUT_WAIT_TICKS;
                if warping {
                    self.blackout_ticks += 1;
                    api.release_all_buttons();
                    *delay = DelayContext::long();
                }
                if !warping && delay.tick(delta_cycles) {
                    let game_state = self.observe_state(api)?;
                    self.poll_policy(&game_state, api);
                    // Keyed by the raw landing coords.
                    if self.last_map != Some(game_state.map.map) {
                        self.last_map = Some(game_state.map.map);
                        self.world_graph.observe(game_state.map.map, api.raw_player_coords(), &game_state.map);
                        // Cut trees regrow on map re-entry.
                        self.cut_tiles.clear();
                        // Turned-back squares may stop being true.
                        self.turned_back_tiles.clear();
                        self.turn_back_watch = None;
                    }
                    // A non-walking field action takes priority over walking.
                    match self.policy.pick_field_move(&game_state) {
                        Some(crate::pokemon::policy::FieldMove::ReorderParty { slot }) => {
                            api.release_all_buttons();
                            api.move_party_member_to_front(slot as usize)?;
                            self.event(AgentEvent::TextBox { message: format!("Moved party slot {slot} to the front") });
                            self.set_state(AgentState::Idle);
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::TeachMove { item, target_slot }) => {
                            api.release_all_buttons();
                            // This driver has no way back, so refuse here.
                            let incompatible = game_state.pokemon.get(target_slot as usize)
                                .is_some_and(|mon| !crate::pokemon::learnset::can_learn(mon.species, item));
                            if incompatible {
                                self.event(AgentEvent::TextBox { message: teach_refusal(&game_state, item, target_slot) });
                                self.set_state(AgentState::Idle);
                                return Ok(());
                            }
                            self.set_state(AgentState::TeachingMove { item, target_slot, press: true, entered_menu: false, settle: 0, evolve_from: None, declined: false });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::EvolveWithStone { stone, target_slot, evolve_from }) => {
                            api.release_all_buttons();
                            self.set_state(AgentState::TeachingMove { item: stone, target_slot, press: true, entered_menu: false, settle: 0, evolve_from: Some(evolve_from), declined: false });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::CutTree) => {
                            api.release_all_buttons();
                            // The driver has no way back, so refuse here.
                            if !game_state.can_use_cut {
                                self.event(AgentEvent::TextBox {
                                    message: "Cut needs a party member that knows it, and the \
CascadeBadge; not cutting".to_string(),
                                });
                                self.set_state(AgentState::Idle);
                                return Ok(());
                            }
                            let tree_pos = game_state.map.tile_in_front().map(|(p, _)| p)
                                .unwrap_or(game_state.map.player_position);
                            let Some((slot, move_index)) =
                                crate::pokemon::policy::field_move_carrier(&game_state, crate::pokemon::move_name::PokemonMoveName::Cut)
                            else {
                                self.event(AgentEvent::TextBox {
                                    message: "nobody in the party knows Cut; not cutting".to_string() });
                                self.set_state(AgentState::Idle);
                                return Ok(());
                            };
                            self.set_state(AgentState::CuttingTree { press: true, entered_menu: false, tree_pos, slot, move_index, from_row: false });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::CheckTrashCan { target, facing }) => {
                            api.release_all_buttons();
                            self.set_state(AgentState::CheckingTrashCan { target, checked: false, press: true, facing });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::UseFieldMove { slot, move_index }) => {
                            api.release_all_buttons();
                            self.set_state(AgentState::UsingFieldMove { press: true, entered_menu: false, slot, move_index, from_map: game_state.map.map, resume: None, settle: 0 });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::TossItem { item }) => {
                            api.release_all_buttons();
                            self.set_state(AgentState::TossingItem { item, press: true, entered_menu: false });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::UseItemPc { op, item, qty, pc }) => {
                            use crate::pokemon::postgame::item_storage::ItemPcState;
                            api.release_all_buttons();
                            // Baselined before any menu is touched.
                            let start_qty = match op {
                                crate::pokemon::postgame::item_storage::PcItemOp::Deposit => api.bag_item_quantity(item),
                                crate::pokemon::postgame::item_storage::PcItemOp::Withdraw => api.pc_box_item_quantity(item),
                            };
                            self.set_state(AgentState::UsingItemPc(ItemPcState::new(op, item, qty, pc, start_qty)));
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::PushBoulder { boulder, dir }) => {
                            api.release_all_buttons();
                            // The same last line of defence as `CutTree`.
                            if let Some(refusal) = game_state.map.boulder_push_refusal(boulder, dir) {
                                self.event(AgentEvent::TextBox { message: refusal });
                                self.set_state(AgentState::Idle);
                                return Ok(());
                            }
                            self.set_state(AgentState::PushingBoulder { boulder, dir, armed: false });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::UseElevator { panel, floor }) => {
                            api.release_all_buttons();
                            self.set_state(AgentState::UsingElevator { panel, floor, selected: false, press: true });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::UseFieldItem { item, target }) => {
                            api.release_all_buttons();
                            // The same last line of defence as `CutTree`.
                            if let Some(refusal) = crate::pokemon::item_use::field_use_refusal(item) {
                                self.event(AgentEvent::TextBox { message: refusal });
                                self.set_state(AgentState::Idle);
                                return Ok(());
                            }
                            self.set_state(AgentState::UsingFieldItem { item, target, press: true, entered_menu: false, backing_out: 0 });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::UsePcBox { op, pc }) => {
                            use crate::pokemon::postgame::pc_box::PcBoxState;
                            api.release_all_buttons();
                            self.set_state(AgentState::UsingPcBox(PcBoxState::new(op, pc, api)));
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::UsePartyScript { script, slot, npc }) => {
                            use crate::pokemon::postgame::gifts::PartyScriptState;
                            api.release_all_buttons();
                            self.set_state(AgentState::UsingPartyScript(PartyScriptState::new(script, slot, npc, api)));
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::UseBagItem { item, target }) => {
                            use crate::pokemon::postgame::items::BagItemState;
                            api.release_all_buttons();
                            self.set_state(AgentState::UsingBagItem(BagItemState::new(item, target, api)));
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::Fly { to }) => {
                            use crate::pokemon::postgame::fly_bike::FlyState;
                            api.release_all_buttons();
                            self.set_state(AgentState::Flying(FlyState::new(to)));
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::Fish { rod, at }) => {
                            use crate::pokemon::postgame::fishing::FishState;
                            api.release_all_buttons();
                            self.set_state(AgentState::Fishing(FishState::new(rod, at)));
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::SellToMart { item, clerk }) => {
                            use crate::pokemon::postgame::game_corner::SellState;
                            api.release_all_buttons();
                            self.set_state(AgentState::SellingToMart(SellState::new(item, clerk, api)));
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::RedeemPrize { prize }) => {
                            use crate::pokemon::postgame::game_corner::PrizeState;
                            api.release_all_buttons();
                            self.set_state(AgentState::RedeemingPrize(PrizeState::new(prize, api)));
                            return Ok(());
                        }
                        None => {}
                    }
                    if let Some(action) = self.policy.pick_overworld_action(&game_state, &self.world_graph) {
                        self.take_overworld_action(action);
                    }
                }
            }
            AgentState::OverworldMovement { destination, map: expected_map } => {
                if self.cycles_since_poll.to_duration() >= MAX_MOVEMENT_SILENCE {
                    api.release_all_buttons();
                    let at = self.player_at(api);
                    self.abort_overworld(destination, OverworldActionAbortedReason::DidNotArrive, at);
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                let game_state = self.observe_state(api)?;
                // The turn-back watch needs the square the player stepped from.
                let raw = api.raw_player_coords();
                self.walk_squares = match self.walk_squares {
                    Some((_, current)) if current != raw => Some((current, raw)),
                    Some(pair) => Some(pair),
                    None => Some((raw, raw)),
                };
                if game_state.mode != GameMode::Overworld {
                    let at = Some(game_state.map.player_position);
                    self.abort_overworld(
                        destination,
                        OverworldActionAbortedReason::from_game_mode(game_state.mode),
                        at,
                    );
                } else if game_state.map.map != expected_map {
                    // Success for warps and connections, which both leave the map.
                    if matches!(destination, MetaTile::Warp { .. } | MetaTile::Connection { .. } | MetaTile::ConnectionWater(_)) {
                        new_events.push(AgentEvent::OverworldActionCompleted { destination });
                        self.set_state(AgentState::Idle);
                    } else {
                        // No position: a coordinate would be read against the destination's map.
                        self.abort_overworld(
                            destination,
                            OverworldActionAbortedReason::WrongMap(game_state.map.map),
                            None,
                        );
                    }
                } else if let MetaTile::Warp { to_map, to_position } = destination
                    && to_map == game_state.map.map
                    && game_state.map.player_position == to_position
                {
                    // A teleport pad does not change the map.
                    new_events.push(AgentEvent::OverworldActionCompleted { destination });
                    self.set_state(AgentState::Idle);
                } else if matches!(destination, MetaTile::Warp { .. })
                    && game_state.map.player_tile() == destination
                    && is_on_map_border(&game_state.map)
                    // Not a tileset step-on warp,
                    && !game_state.map.is_step_on_warp(game_state.map.player_position)
                    // the player is on foot,
                    && !game_state.map.surfing
                    // and the collision path is armed.
                    && game_state.map.standing_on_warp
                {
                    // Standing on an edge warp.
                    let pos = game_state.map.player_position;
                    let h = game_state.map.height.saturating_sub(1) as u8;
                    let exit_dir = if pos.y == 0 { JoypadButton::Up }
                        else if pos.y == h { JoypadButton::Down }
                        else if pos.x == 0 { JoypadButton::Left }
                        else { JoypadButton::Right };
                    api.release_all_buttons();
                    api.press_button(exit_dir);
                } else if destination == MetaTile::Empty {
                    // A cave wander: keep walking so per-step encounters fire.
                    let pos = game_state.map.player_position;
                    match adjacent_pacing_pair(&game_state.map, pos) {
                        Some((tile_a, tile_b)) => self.set_state(AgentState::PacingForEncounters {
                            destination, map: game_state.map.map, tile_a, tile_b, heading_to_b: false, stalled: 0, paced: 0 }),
                        None => {
                            let at = Some(game_state.map.player_position);
                            self.abort_overworld(
                                destination,
                                OverworldActionAbortedReason::NoRoute(destination),
                                at,
                            );
                            self.set_state(AgentState::Idle);
                        }
                    }
                } else if game_state.map.player_tile() == destination && !matches!(destination, MetaTile::Warp { .. }) {
                    if destination == MetaTile::Grass {
                        let pos = game_state.map.player_position;
                        // Pace against a steppable grass neighbour, else any plain one.
                        let pair = adjacent_grass(&game_state.map, pos).map(|b| (pos, b))
                            .or_else(|| adjacent_pacing_pair(&game_state.map, pos));
                        if let Some((tile_a, tile_b)) = pair {
                            self.set_state(AgentState::PacingForEncounters { destination, map: game_state.map.map, tile_a, tile_b, heading_to_b: true, stalled: 0, paced: 0 });
                        } else {
                            // TODO: unreachable if `actions()` checks the neighbour.
                            let at = Some(game_state.map.player_position);
                            self.abort_overworld(destination, OverworldActionAbortedReason::NoAdjacentGrass, at);
                            self.set_state(AgentState::Idle);
                        }
                    } else {
                        new_events.push(AgentEvent::OverworldActionCompleted { destination });
                        self.set_state(AgentState::Idle);
                    }
                } else {
                    // Re-derived every tick and only its head pressed: no recipe may use its tail.
                    let action = game_state.map.actions().into_iter()
                        // Not `==`: a boulder goal's row names a moving boulder.
                        .find(|a| a.tile.is_same_row_as(&destination))
                        // `actions()` offers only the nearest crossing.
                        .or_else(|| match destination {
                            MetaTile::Connection { to_map, to_position } =>
                                game_state.map.connection_action(to_map, to_position),
                            // Nor is a water edge when a land crossing to the same map is nearer.
                            MetaTile::ConnectionWater(to_map) =>
                                game_state.map.water_connection_action(to_map),
                            _ => None,
                        });
                    // Reset when the row is back: the count is consecutive.
                    if action.is_some() {
                        self.route_lost_ticks = 0;
                        self.route_lost_to_people = false;
                    }
                    match action {
                        None if !game_state.map.position_settled => {}
                        // A route that has just gone may be blocked by people.
                        None if self.wait_for_the_route(&game_state.map, destination) => {
                            // Released, unlike the `position_settled` arm.
                            api.release_all_buttons();
                        }
                        None => self.abort_overworld(
                            destination,
                            OverworldActionAbortedReason::NoRoute(destination),
                            Some(game_state.map.player_position),
                        ),
                        Some(a) => match a.route.first() {
                            // Toggled so `hJoyPressed` fires every other tick.
                            Some(&JoypadButton::A) => api.toggle_button(JoypadButton::A),
                            Some(&btn) => {
                                let pos = game_state.map.player_position;
                                let next = step_pos(pos, btn);
                                let onto_water = !matches!(destination, MetaTile::Fish { .. })
                                    && matches!(
                                    next.and_then(|n| game_state.map.tile_at_checked(n)),
                                    Some(MetaTile::Water) | Some(MetaTile::ConnectionWater(_))
                                );
                                let surfing = api.mmu().read_pointer(&pokered_symbols::wWalkBikeSurfState) == 2;
                                if onto_water && !surfing {
                                    if let (Some(water_pos), Some((slot, move_index))) = (next,
                                        crate::pokemon::policy::field_move_carrier(&game_state, crate::pokemon::move_name::PokemonMoveName::Surf)) {
                                        api.release_all_buttons();
                                        // The walk rides through the mount; see `Surfing::resume`.
                                        self.set_state(AgentState::Surfing {
                                            press: true, entered_menu: false, water_pos, slot, move_index,
                                            resume: Some((destination, expected_map)), settle: 0 });
                                        return Ok(());
                                    }
                                }
                                api.release_all_buttons();
                                api.press_button(btn);
                            }
                            None => {
                                // The empty route is where a fishing row turns into a cast.
                                if let MetaTile::Fish { .. } = destination
                                    && let Some(rod) = crate::pokemon::postgame::fishing::Rod::best_in_bag(&game_state.bag)
                                    && let Some(at) = crate::pokemon::postgame::fishing::nearest_castable_water(&game_state.map)
                                {
                                    api.release_all_buttons();
                                    self.set_state(AgentState::Fishing(
                                        crate::pokemon::postgame::fishing::FishState::new(rod, at)));
                                    return Ok(());
                                }
                                // A cut tree and a boulder goal finish their own action too.
                                if let MetaTile::Cut { at: tree_pos } = destination
                                    && game_state.can_use_cut
                                    // The driver cuts whatever is in front.
                                    && game_state.map.tile_in_front()
                                        .is_some_and(|(at, tile)| at == tree_pos && tile == MetaTile::CutTree)
                                    && let Some((slot, move_index)) = crate::pokemon::policy::field_move_carrier(
                                        &game_state, crate::pokemon::move_name::PokemonMoveName::Cut)
                                {
                                    api.release_all_buttons();
                                    self.set_state(AgentState::CuttingTree {
                                        press: true, entered_menu: false, tree_pos, slot, move_index,
                                        from_row: true });
                                    return Ok(());
                                }
                                // A pacing row walks to its square, then paces it against a neighbour.
                                if let MetaTile::Pace { water } = destination
                                    && let pos = game_state.map.player_position
                                    && let Some(next) = game_state.map.pacing_neighbour(pos, pace_kind(water))
                                {
                                    api.release_all_buttons();
                                    self.set_state(AgentState::PacingForEncounters {
                                        destination, map: game_state.map.map, tile_a: pos, tile_b: next,
                                        heading_to_b: true, stalled: 0, paced: 0 });
                                    return Ok(());
                                }
                                if let MetaTile::BoulderGoal { boulder, at, hole } = destination {
                                    api.release_all_buttons();
                                    self.boulder_goal = Some((game_state.map.map, boulder, at, hole));
                                    self.boulder_goal_pushes = 0;
                                    self.boulder_goal_best = usize::MAX;
                                    self.boulder_goal_stale = 0;
                                    self.boulder_goal_silences = 0;
                                    self.set_state(AgentState::SolvingBoulderPuzzle {
                                        boulder, target: at, hole, pushes: 0, settle: 0 });
                                    return Ok(());
                                }
                                new_events.push(AgentEvent::OverworldActionCompleted { destination });
                                self.set_state(AgentState::Idle);
                            }
                        }
                    }
                }
            }
            AgentState::ReadingTextBox { ref mut reader } => {
                // B, not A, inside a PC menu: it is the only way out.
                let in_battle = api.mmu().read_pointer(&pokered_symbols::wIsInBattle) != 0;
                let handing_over = self.menu_handover_ticks > 0;
                self.menu_handover_ticks = self.menu_handover_ticks.saturating_sub(1);
                let timed_out = self.cycles_since_poll.to_duration() >= TEXT_BOX_ESCAPE_SILENCE;
                // The hand-over rule trusts only the screen; after the silence, the RAM ids too.
                let evidence = if timed_out { MenuEvidence::OrTheLingeringIds } else { MenuEvidence::OnScreen };
                if !in_battle && (handing_over || timed_out) && open_menu_on_screen(api, evidence) {
                    if handing_over && !self.escaping_menus {
                        new_events.push(AgentEvent::TextBox {
                            message: "a menu was left open; closing it rather than confirming it".to_string(),
                        });
                    }
                    self.escaping_menus = true;
                    self.menu_handover_ticks = 0;
                }
                // A party menu a conversation opened is answered here rather than A-mashed.
                if !in_battle
                    && let Some(screen) = api.on_screen_text(false)
                    && let (x, y, cursor, _) = api.menu_geometry()
                    && crate::pokemon::menu::is_normal_party_menu(x, y, &screen)
                {
                    reader.accumulate(api);
                    // Unarmed, a conversation's party list is the Day Care or the Name Rater.
                    let choice = self.party_menu.unwrap_or(PartyMenuAnswer {
                        give: None, press: true, answered: None,
                    });
                    let button = match (choice.answered, choice.give) {
                        // Keep giving the same answer until the list goes.
                        (Some(button), _) => button,
                        (None, Some(give)) => match party_slot_of(api, give) {
                            Some(slot) if slot == cursor => {
                                new_events.push(AgentEvent::TextBox { message: format!(
                                    "handed over the {give:?} in party slot {}", slot + 1) });
                                JoypadButton::A
                            }
                            Some(slot) if slot > cursor => JoypadButton::Down,
                            Some(_) => JoypadButton::Up,
                            // Backed out rather than offer the wrong one.
                            None => {
                                new_events.push(AgentEvent::TextBox { message: format!(
                                    "the trade wants a {give:?} and there is none in the party, so \
                                     nothing was handed over") });
                                JoypadButton::B
                            }
                        },
                        (None, None) => {
                            new_events.push(AgentEvent::TextBox { message:
                                "a party menu was left open with no way to answer it; closing it \
                                 rather than confirming it".to_string() });
                            JoypadButton::B
                        }
                    };
                    api.release_all_buttons();
                    if choice.press {
                        api.press_button(button);
                    }
                    let answered = choice.answered
                        .or_else(|| matches!(button, JoypadButton::A | JoypadButton::B).then_some(button));
                    self.party_menu = Some(PartyMenuAnswer { press: !choice.press, answered, ..choice });
                    for event in new_events {
                        self.event(event);
                    }
                    return Ok(());
                }
                // A quiz row chosen for NO: the cursor to NO, then A, once.
                if self.answer_no && let Some(menu) = api.menu_state().filter(|menu| menu.is_yes_no_menu()) {
                    let button = if menu.current_item == 0 { JoypadButton::Down } else { JoypadButton::A };
                    self.answer_no &= button == JoypadButton::Down;
                    reader.update_with(api, button);
                    return Ok(());
                }
                // A vending row names its drink: the cursor to it, then A, once. The menu is
                // `VendingMachineMenu`'s, drawn with its first row at (1, 5).
                if let Some(drink) = self.vending_pick
                    && let Some(menu) = api.menu_state()
                        .filter(|menu| (menu.top_menu_item_x, menu.top_menu_item_y) == (1, 5) && menu.current_item <= 3)
                {
                    let button = match menu.current_item.cmp(&drink) {
                        std::cmp::Ordering::Less => JoypadButton::Down,
                        std::cmp::Ordering::Greater => JoypadButton::Up,
                        std::cmp::Ordering::Equal => JoypadButton::A,
                    };
                    if button == JoypadButton::A {
                        self.vending_pick = None;
                    }
                    reader.update_with(api, button);
                    return Ok(());
                }
                let button = if api.in_pc_menu() || self.escaping_menus { JoypadButton::B } else { JoypadButton::A };
                reader.update_with(api, button);
            }
            AgentState::Battle(ref mut battle_state) => {
                // The game has refused what was just selected.
                if api.on_screen_text(false).map_or(false, |t| shows_battle_refusal(&t)) {
                    let mut carried = battle_state.take_reader();
                    // The refusal is read now, as far as it has been typed: backing out never reads.
                    if api.menu_state().is_some_and(|m| m.text_box_id == crate::pokemon::menu::TextBoxId::MessageBox) {
                        carried.accumulate(api);
                    }
                    api.toggle_button(JoypadButton::B);
                    self.set_battle_state(BattleState::backing_out_carrying(carried));
                    return Ok(());
                }
                match battle_state {
                    BattleState::WaitingForMenu { reader, delay, backing_out, confirming, confirm } => {
                        // First, because the A presses that defeat it come from other branches.
                        if *backing_out > 0 {
                            *backing_out -= 1;
                            api.toggle_button(JoypadButton::B);
                            return Ok(());
                        }
                        if let Some(menu_state) = api.menu_state() {
                            // A voluntary switch's SWITCH/STATS/CANCEL.
                            if menu_state.is_switch_stats_cancel_menu() {
                                if menu_state.current_item == 0 {
                                    api.toggle_button(JoypadButton::A);
                                } else {
                                    api.toggle_button(JoypadButton::Up);
                                }
                                return Ok(());
                            }
                            // After an item turn a leaked bag list and FIGHT both read as a list.
                            let screen = api.on_screen_text(false).unwrap_or_default();
                            if screen.contains("No PP left") {
                                // Latched here: this check returns before the `MoveList` arm could.
                                *backing_out = BACKING_OUT_TICKS;
                                api.toggle_button(JoypadButton::B);
                                return Ok(());
                            }
                            if screen.contains("CANCEL") || shows_battle_refusal(&screen) {
                                api.toggle_button(JoypadButton::B);
                                return Ok(());
                            }
                            // The SHIFT prompt.
                            if screen.contains("change") && menu_state.is_yes_no_menu() {
                                api.toggle_button(JoypadButton::B);
                                return Ok(());
                            }
                            // Only once the screen is trustworthy again.
                            if *confirming > 0 {
                                *confirming -= 1;
                            } else if screen.contains("FIGHT") && screen.contains("RUN") {
                                new_events.push(AgentEvent::text_box_from_reader(reader));
                                api.release_all_buttons();
                                self.set_battle_state(BattleState::AwaitingPolicy { delay: DelayContext::default(), menu_gone: 0 });
                                // Drained here because this arm returns.
                                for event in new_events {
                                    self.event(event);
                                }
                                return Ok(());
                            }
                            match menu_state.battle_menu_state() {
                                Some(BattleMenuState::Fight)
                                | Some(BattleMenuState::SafariBall) | Some(BattleMenuState::SafariBait)
                                | Some(BattleMenuState::SafariRock) => {
                                    new_events.push(AgentEvent::text_box_from_reader(reader));

                                    api.release_all_buttons();
                                    self.set_battle_state(BattleState::AwaitingPolicy { delay: DelayContext::default(), menu_gone: 0 });
                                }
                                Some(BattleMenuState::PokemonList { index }) => {
                                    // The geometry outlives an item's party menu, and what the game says
                                    // next, the item's effect and the enemy's reply, is typed under it.
                                    if menu_state.text_box_id == crate::pokemon::menu::TextBoxId::MessageBox {
                                        reader.accumulate(api);
                                    }
                                    // Only if the party list is what is on screen.
                                    if api.on_screen_text(false).map_or(0, |t| t.matches('/').count()) < 2 {
                                        api.toggle_button(JoypadButton::A);
                                        return Ok(());
                                    }
                                    // A party list is ours to drive only for a forced switch.
                                    let game_state = api.game_state()?;
                                    let active_fainted = game_state.battle.as_ref()
                                        .map_or(false, |b| b.player.current_hp == 0);
                                    if !active_fainted {
                                        api.toggle_button(JoypadButton::B);
                                    } else {
                                        let cursor_hp = game_state.pokemon
                                            .get(index as usize)
                                            .map_or(0, |p| p.current_hp);
                                        if cursor_hp > 0 {
                                            api.toggle_button(JoypadButton::A);
                                        } else {
                                            let target = game_state.pokemon.iter().enumerate()
                                                .find(|(_, p)| p.current_hp > 0)
                                                .map(|(i, _)| i as u8)
                                                .unwrap_or(0);
                                            if index < target {
                                                api.toggle_button(JoypadButton::Down);
                                            } else {
                                                api.toggle_button(JoypadButton::Up);
                                            }
                                        }
                                    }
                                }
                                Some(BattleMenuState::MoveList { index }) => {
                                    let disabled = api.game_state().ok()
                                        .and_then(|g| g.battle)
                                        .and_then(|b| b.player.disabled_move_slot) == Some(index);
                                    // Only the game's own message counts.
                                    let no_pp = api.on_screen_text(false).unwrap_or_default()
                                        .contains("No PP left");
                                    if disabled || no_pp {
                                        *backing_out = BACKING_OUT_TICKS;
                                        api.toggle_button(JoypadButton::B);
                                    } else if reading_dialogue(&menu_state, *confirming) {
                                        // Stale geometry: the game is talking.
                                        reader.update(api);
                                    } else if let Some(mut pending) = *confirm {
                                        let pending = &mut pending;
                                        // One press, then wait for the game's own record of it.
                                        let committed = api.mmu()
                                            .read_pointer(&crate::pokemon::symbols::pokered_symbols::wPlayerMoveListIndex);
                                        match pending.pressed {
                                            // Recorded, so finished.
                                            true if committed == pending.slot => {
                                                *confirm = None;
                                                api.release_all_buttons();
                                            }
                                            true if pending.waited < CONFIRM_ACK_TICKS => {
                                                pending.waited += 1;
                                                api.release_all_buttons();
                                            }
                                            // Not acted on: press again.
                                            true if pending.attempts < CONFIRM_ATTEMPTS => {
                                                pending.pressed = false;
                                                pending.waited = 0;
                                                api.release_all_buttons();
                                            }
                                            // Out of attempts: back out.
                                            true => {
                                                *backing_out = BACKING_OUT_TICKS;
                                                api.toggle_button(JoypadButton::B);
                                            }
                                            false if index == pending.slot => {
                                                pending.pressed = true;
                                                pending.waited = 0;
                                                pending.attempts += 1;
                                                api.toggle_button(JoypadButton::A);
                                            }
                                            false => {
                                                let (action, ticks) = (pending.action, pending.navigating_ticks);
                                                api.release_all_buttons();
                                                self.set_battle_state(BattleState::Navigating {
                                                    action, delay: DelayContext::default(), ticks, stable: 0,
                                                });
                                                return Ok(());
                                            }
                                        }
                                        // Write the record back unless the arm above cleared it.
                                        if confirm.is_some() {
                                            *confirm = Some(*pending);
                                        }
                                    } else {
                                        api.toggle_button(JoypadButton::A);
                                    }
                                },
                                Some(BattleMenuState::ItemList { .. }) => {
                                    // A bag list here is always a leak, and A is always wrong.
                                    api.toggle_button(JoypadButton::B);
                                },
                                Some(_) if reading_dialogue(&menu_state, *confirming) => {
                                    // Stale geometry again: the game is talking.
                                    reader.update(api);
                                },
                                Some(_) => {
                                    api.toggle_button(JoypadButton::A);
                                },
                                None => {
                                    // Wait for the text box to render before reading it.
                                    if delay.tick(delta_cycles) {
                                        reader.update(api);
                                    }
                                }
                            }
                        } else {
                            api.toggle_button(JoypadButton::A);
                        }
                    }

                    BattleState::AwaitingPolicy { delay, menu_gone } => {
                        let gone_before = *menu_gone;
                        if delay.tick(delta_cycles) {
                            let game_state = api.game_state()?;
                            self.poll_policy(&game_state, api);
                            if let Some(action) = self.policy.pick_battle_action(&game_state) {
                                let active = game_state.battle.as_ref().map(|b| b.active_party_slot).unwrap_or(0);
                                // An item is used on someone, who is the actor of the sentence.
                                let actor = match action {
                                    BattleAction::UseItem { target: Some(target), .. } => game_state.pokemon
                                        .get(target as usize)
                                        .map(|mon| mon.nickname.to_default_string())
                                        .unwrap_or_else(|| active_pokemon_name(&game_state)),
                                    _ => active_pokemon_name(&game_state),
                                };
                                new_events.push(AgentEvent::BattleActionStarted {
                                    actor,
                                    opponent: opponent_pokemon_name(&game_state),
                                    action,
                                });
                                if let BattleAction::UseItem { item, target, .. } = action {
                                    let start_qty = game_state.bag.iter()
                                        .find(|b| b.id == item.id).map(|b| b.quantity).unwrap_or(0);
                                    let aim = target.unwrap_or(active);
                                    let entry_hp = game_state.pokemon.get(aim as usize).map(|p| p.current_hp).unwrap_or(0);
                                    self.set_battle_state(BattleState::UsingItem { ticks: 0,
                                        item: item.id, aim, start_qty, entry_hp, press: true, confirmed: false,
                                        delay: DelayContext::default(),
                                        reader: PokemonTextReader::message_box_only(),
                                    });
                                    return Ok(());
                                }
                                self.set_battle_state(BattleState::Navigating { action, delay: DelayContext::default(), ticks: 0, stable: 0 });
                            } else {
                                // A menu drawn a frame too long after the last choice, a successful
                                // escape's say: whatever replaced it waits on A that only
                                // `WaitingForMenu` presses, and a policy that waits never would.
                                let gone = if battle_menu_is_showing(api) { 0 } else { gone_before + 1 };
                                if gone >= 2 {
                                    self.set_battle_state(BattleState::default());
                                } else if let AgentState::Battle(BattleState::AwaitingPolicy { menu_gone, .. }) = &mut self.state {
                                    *menu_gone = gone;
                                }
                            }
                        }
                    }

                    BattleState::Navigating { action, delay, ticks, stable } => {
                        if delay.tick(delta_cycles) {
                            // Nothing below polls the policy, so nothing below may run for ever.
                            const MAX_NAVIGATING_TICKS: u16 = 250;
                            *ticks += 1;
                            if *ticks >= MAX_NAVIGATING_TICKS {
                                // `action` borrows from the state about to go.
                                let gave_up = format!("battle navigation to {action:?} got nowhere in 5s; re-deciding");
                                api.release_all_buttons();
                                // Latched: `WaitingForMenu` opens by pressing A.
                                self.set_battle_state(BattleState::backing_out());
                                self.event(AgentEvent::TextBox { message: gave_up });
                                return Ok(());
                            }
                            if let crate::pokemon::battle::BattleAction::SwitchPokemon { slot: target, .. } = *action {
                                let active = api.game_state().ok()
                                    .and_then(|g| g.battle.map(|b| b.active_party_slot));
                                if active == Some(target) {
                                    api.release_all_buttons();
                                    self.set_battle_state(BattleState::default());
                                    return Ok(());
                                }
                                let raw = api.menu_state();
                                let bms = raw.and_then(|m| m.battle_menu_state());
                                if raw.map_or(false, |m| m.is_switch_stats_cancel_menu()) {
                                    let cur = raw.map_or(0, |m| m.current_item);
                                    api.toggle_button(if cur == 0 { JoypadButton::A } else { JoypadButton::Up });
                                    return Ok(());
                                }
                                if bms.is_none() || matches!(bms, Some(BattleMenuState::PokemonList { .. })) {
                                    // The in-battle party list, at either geometry.
                                    let cur = raw.map_or(0, |m| m.current_item);
                                    let btn = if cur == target { JoypadButton::A }
                                        else if cur < target { JoypadButton::Down }
                                        else { JoypadButton::Up };
                                    api.toggle_button(btn);
                                    return Ok(());
                                }
                                // Still at the main grid: the generic navigator opens PKMN.
                            }
                            let Some(menu_state) = api.menu_state().and_then(|s| s.battle_menu_state()) else {
                                api.release_all_buttons();
                                self.set_battle_state(BattleState::default());
                                return Ok(());
                            };
                            {
                                let menu_target = BattleMenuState::from_action(*action);

                                // A leaked sub-menu that is not this action's.
                                let wrong_submenu = match (menu_state, *action) {
                                    (BattleMenuState::ItemList { .. }, a) => !matches!(a, BattleAction::UseItem { .. }),
                                    (BattleMenuState::PokemonList { .. }, a) => !matches!(a, BattleAction::SwitchPokemon { .. }),
                                    _ => false,
                                };
                                if wrong_submenu {
                                    api.toggle_button(JoypadButton::B);
                                    return Ok(());
                                }

                                // Believed only once seen twice.
                                if menu_state == menu_target && *stable == 0 {
                                    *stable = 1;
                                    api.release_all_buttons();
                                    return Ok(());
                                }
                                if menu_state != menu_target {
                                    *stable = 0;
                                }
                                if menu_state == menu_target {
                                    // These options have no sub-menu.
                                    if matches!(menu_target, BattleMenuState::Run
                                        | BattleMenuState::SafariBall
                                        | BattleMenuState::SafariBait
                                        | BattleMenuState::SafariRock) {
                                        // A Gen 1 escape's odds improve with each try.
                                        api.toggle_button(JoypadButton::A);
                                    } else {
                                        api.release_all_buttons();
                                        let (chosen, elapsed) = (*action, *ticks);
                                        self.set_battle_state(BattleState::confirming(chosen, elapsed));
                                    }
                                } else {
                                    let resolved_target = if let Some(target_parent) = menu_target.parent() {
                                        if menu_state.parent() == Some(target_parent) {
                                            menu_target
                                        } else {
                                            target_parent
                                        }
                                    } else {
                                        menu_target
                                    };

                                    let target_location = resolved_target.location();
                                    let current_location = menu_state.location();

                                    let btn = if target_location == current_location {
                                        JoypadButton::A
                                    } else if target_location.x > current_location.x {
                                        JoypadButton::Right
                                    } else if target_location.x < current_location.x {
                                        JoypadButton::Left
                                    } else if target_location.y > current_location.y {
                                        JoypadButton::Down
                                    } else {
                                        JoypadButton::Up
                                    };

                                    api.toggle_button(btn);
                                }
                            }
                        }
                    }

                    BattleState::UsingItem { item, aim, start_qty, entry_hp, press, confirmed, delay: _, ticks, reader } => {
                        // Same bound as `Navigating`: six menus deep and nothing polled on the way.
                        const MAX_HEALING_TICKS: u16 = 250;
                        let ticks = *ticks;
                        // Read before anything is pressed, on every tick this state owns.
                        if api.menu_state()
                            .is_some_and(|m| m.text_box_id == crate::pokemon::menu::TextBoxId::MessageBox)
                        {
                            reader.accumulate(api);
                        }
                        let reader = std::mem::replace(reader, PokemonTextReader::message_box_only());
                        if ticks >= MAX_HEALING_TICKS {
                            api.release_all_buttons();
                            self.set_battle_state(BattleState::carrying(reader));
                            return Ok(());
                        }
                        use crate::pokemon::item::ItemId;
                        let item = *item;
                        let aim = *aim;
                        let start_qty = *start_qty;
                        let entry_hp = *entry_hp;
                        let press = *press;
                        let confirmed = *confirmed;

                        let raw = api.menu_state();
                        let bms = raw.and_then(|m| m.battle_menu_state());

                        // Keyed on the item's bag count and the HP of the member it is aimed at.
                        let gs = api.game_state().ok();
                        let aimed_hp = gs.as_ref().and_then(|g| g.pokemon.get(aim as usize).map(|p| p.current_hp)).unwrap_or(0);
                        let live = gs.as_ref()
                            .and_then(|g| g.bag.iter().find(|b| b.id == item).map(|b| b.quantity))
                            .unwrap_or(0);
                        if live < start_qty {
                            // Consumed: the heal is committed.
                            api.release_all_buttons();
                            self.set_battle_state(BattleState::carrying(reader));
                            return Ok(());
                        }
                        if aimed_hp > entry_hp {
                            // The HP bar is filling: leave the still-open party menu alone.
                            api.release_all_buttons();
                            self.set_battle_state(BattleState::UsingItem { item, aim, start_qty, entry_hp, press: !press, confirmed: true, delay: DelayContext::default(), ticks: ticks + 1, reader });
                            return Ok(());
                        }
                        // Press and release on alternate ticks for clean rising edges.
                        if !press {
                            api.release_all_buttons();
                            self.set_battle_state(BattleState::UsingItem { item, aim, start_qty, entry_hp, press: true, confirmed, delay: DelayContext::default(), ticks: ticks + 1, reader });
                            return Ok(());
                        }

                        let opens_party_menu = matches!(item,
                            ItemId::Potion | ItemId::SuperPotion | ItemId::HyperPotion | ItemId::MaxPotion
                            | ItemId::FullRestore | ItemId::FullHeal | ItemId::Antidote | ItemId::BurnHeal
                            | ItemId::IceHeal | ItemId::Awakening | ItemId::ParlyzHeal
                            | ItemId::Revive | ItemId::MaxRevive
                            | ItemId::Ether | ItemId::MaxEther | ItemId::Elixer | ItemId::MaxElixer);
                        let party_showing = opens_party_menu && (matches!(bms, Some(BattleMenuState::PokemonList { .. }))
                            || (bms.is_none() && api.on_screen_text(false).map_or(false, |t| t.matches('/').count() >= 2)));

                        let next_confirmed = confirmed;
                        let button: Option<JoypadButton> = if party_showing {
                            let cur = raw.map_or(0, |m| m.current_item);
                            if cur == aim { Some(JoypadButton::A) }
                            else if cur < aim { Some(JoypadButton::Down) }
                            else { Some(JoypadButton::Up) }
                        } else if let Some(BattleMenuState::ItemList { index }) = bms {
                            // The bag's raw order is the battle list's order.
                            match api.bag_item_position(item) {
                                Some(target) => Some(if index == target { JoypadButton::A }
                                    else if index < target { JoypadButton::Down } else { JoypadButton::Up }),
                                None => None,
                            }
                        } else if let Some(grid) = bms.filter(|b| matches!(b,
                            BattleMenuState::Fight | BattleMenuState::Item | BattleMenuState::Pokemon | BattleMenuState::Run)) {
                            let cur = grid.location();
                            let tgt = BattleMenuState::Item.location();
                            Some(if cur.x < tgt.x { JoypadButton::Right } else if cur.x > tgt.x { JoypadButton::Left }
                                else if cur.y < tgt.y { JoypadButton::Down } else if cur.y > tgt.y { JoypadButton::Up }
                                else { JoypadButton::A })
                        } else {
                            Some(JoypadButton::A) // resolution / heal text — advance
                        };

                        api.release_all_buttons();
                        if let Some(b) = button { api.press_button(b); }
                        self.set_battle_state(BattleState::UsingItem { item, aim, start_qty, entry_hp, press: false, confirmed: next_confirmed, delay: DelayContext::default(), ticks: ticks + 1, reader });
                    }
                }
            }
            AgentState::PacingForEncounters { destination, map, ref mut tile_a, ref mut tile_b, ref mut heading_to_b, ref mut stalled, ref mut paced } => {
                /// Overworld ticks on the same tile before the pair is declared unwalkable.
                const STALL_TICKS: u16 = 60;

                let game_state = api.game_state()?;
                if game_state.mode == GameMode::Overworld {
                    if game_state.map.map != map {
                        let at = Some(game_state.map.player_position);
                        self.abort_overworld(
                            destination,
                            OverworldActionAbortedReason::WrongMap(game_state.map.map),
                            at,
                        );
                        return Ok(());
                    }
                    *paced += 1;
                    if *paced >= PACING_BUDGET_TICKS {
                        api.release_all_buttons();
                        let at = Some(game_state.map.player_position);
                        self.abort_overworld(destination, OverworldActionAbortedReason::NothingAppeared, at);
                        return Ok(());
                    }
                    let pos = game_state.map.player_position;
                    let target = if *heading_to_b { *tile_b } else { *tile_a };
                    if pos == target {
                        *heading_to_b = !*heading_to_b;
                        *stalled = 0;
                    } else {
                        *stalled += 1;
                        if *stalled >= STALL_TICKS {
                            // The map moves under a pair, so ask for another before giving up.
                            let repicked = match destination {
                                MetaTile::Pace { water } =>
                                    game_state.map.pacing_neighbour(pos, pace_kind(water)).map(|b| (pos, b)),
                                _ => adjacent_grass(&game_state.map, pos).map(|b| (pos, b))
                                    .or_else(|| adjacent_pacing_pair(&game_state.map, pos)),
                            }.filter(|&(a, b)| (a, b) != (*tile_a, *tile_b));
                            if let Some((a, b)) = repicked {
                                *tile_a = a;
                                *tile_b = b;
                                *heading_to_b = true;
                                *stalled = 0;
                            } else {
                                // `Unknown`, which the coverage oracle scores as a defect.
                                api.release_all_buttons();
                                self.abort_overworld(
                                    destination,
                                    OverworldActionAbortedReason::Unknown,
                                    Some(pos),
                                );
                                return Ok(());
                            }
                        }
                    }
                    let next = if *heading_to_b { *tile_b } else { *tile_a };
                    if let Some(dir) = dir_to(pos, next) {
                        api.release_all_buttons();
                        api.press_button(dir);
                    }
                }
            }
            AgentState::PokemartShopping(ref pokemart_state) => {
                let menu = api.menu_state();

                match pokemart_state {
                    PokemartState::AwaitingPolicy => {
                        api.release_all_buttons();
                        let game_state = api.game_state()?;
                        self.ask_mart_policy(&game_state, api);
                    }

                    PokemartState::ChoosingBuyOption(item) => {
                        if let Some(menu) = menu {
                            if menu.is_mart_item_list() {
                                self.set_pokemart_state(PokemartState::ChoosingItem(*item));
                            } else if menu.is_mart_buy_sell_menu() {
                                if menu.current_item == 0 {
                                    api.toggle_button(JoypadButton::A);
                                } else {
                                    api.toggle_button(JoypadButton::Up);
                                }
                            } else {
                                // Some other text box, such as the greeting.
                                api.toggle_button(JoypadButton::A);
                            }
                        } else {
                            api.toggle_button(JoypadButton::A);
                        }
                    }

                    PokemartState::ChoosingItem(item) => {
                        if let Some(menu) = menu {
                            if menu.is_mart_item_list() {
                                let shop_items = api.mart_item_list();
                                let target_pos = shop_items.iter().position(|&id| id == item.id);
                                let current = menu.list_absolute_index() as usize;
                                match target_pos {
                                    None => {
                                        self.set_pokemart_state(PokemartState::Quitting);
                                    }
                                    Some(target_idx) => {
                                        if current == target_idx {
                                            // Clear a stale 99 so pokemart.asm's write is seen.
                                            api.write_max_item_quantity(0);
                                            api.toggle_button(JoypadButton::A);
                                            self.set_pokemart_state(PokemartState::AwaitingQtySelector(*item));
                                        } else if current < target_idx {
                                            api.toggle_button(JoypadButton::Down);
                                        } else {
                                            api.toggle_button(JoypadButton::Up);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // `wMaxItemQuantity` reaches 99 once pokemart.asm has accepted the item.
                    PokemartState::AwaitingQtySelector(item) => {
                        if api.mart_in_quantity_selector() {
                            api.release_all_buttons();
                            self.set_pokemart_state(PokemartState::ChoosingQuantity { item: *item, qty_last: 0, stall_ticks: 0 });
                        } else {
                            api.toggle_button(JoypadButton::A);
                        }
                    }

                    PokemartState::ChoosingQuantity { item, qty_last, stall_ticks } => {
                        let item = *item;
                        let qty_last = *qty_last;
                        let stall_ticks = *stall_ticks;

                        let yes_no = menu.map_or(false, |m| m.is_yes_no_menu());
                        let cur_qty = api.mart_item_quantity();

                        if yes_no {
                            // Released first, so the YES press is a fresh rising edge.
                            api.release_all_buttons();
                            self.set_pokemart_state(PokemartState::ConfirmingPurchase(item));
                            return Ok(());
                        }

                        if cur_qty == 0 {
                            return Ok(());
                        }
                        let target = item.quantity;

                        let (new_qty_last, new_stall_ticks) = if cur_qty == qty_last {
                            (qty_last, stall_ticks + 1)
                        } else {
                            (cur_qty, 0)
                        };
                        if let AgentState::PokemartShopping(PokemartState::ChoosingQuantity { qty_last: ref mut ql, stall_ticks: ref mut st, .. }) = self.state {
                            *ql = new_qty_last;
                            *st = new_stall_ticks;
                        }

                        // Stalled at the target: the price text.
                        if new_stall_ticks >= 8 && cur_qty == target {
                            api.toggle_button(JoypadButton::A);
                            return Ok(());
                        }

                        if cur_qty == target {
                            api.toggle_button(JoypadButton::A);
                        } else if cur_qty < target {
                            api.toggle_button(JoypadButton::Up);
                        } else {
                            api.toggle_button(JoypadButton::Down);
                        }
                    }

                    PokemartState::ConfirmingPurchase(item) => {
                        if menu.map_or(false, |m| m.is_mart_item_list()) {
                            // B from YES/NO on the wrong quantity lands back on the item list.
                            api.release_all_buttons();
                            self.set_pokemart_state(PokemartState::ChoosingItem(*item));
                            return Ok(());
                        }
                        if let Some(menu) = &menu {
                            if menu.is_yes_no_menu() {
                                let confirmed_qty = api.mart_item_quantity();
                                if confirmed_qty == item.quantity {
                                    if menu.current_item == 0 {
                                        api.release_all_buttons();
                                        api.toggle_button(JoypadButton::A);
                                        self.set_pokemart_state(PokemartState::PurchasedItem { ticks: 0 });
                                    } else {
                                        api.toggle_button(JoypadButton::Up);
                                    }
                                } else {
                                    api.toggle_button(JoypadButton::B);
                                }
                            } else if menu.is_mart_buy_sell_menu() {
                                self.set_pokemart_state(PokemartState::Quitting);
                            } else {
                                api.toggle_button(JoypadButton::A);
                            }
                        } else {
                            api.toggle_button(JoypadButton::A);
                        }
                    }

                    // B through the text, then cancel the re-opened item list.
                    PokemartState::PurchasedItem { ticks } => {
                        let ticks = *ticks;
                        if menu.map_or(false, |m| m.is_mart_buy_sell_menu()) {
                            api.release_all_buttons();
                            // One visit can be several purchases.
                            let money = api.game_state().map(|s| s.money).unwrap_or(0);
                            match self.policy.next_mart_purchase().and_then(|item| affordable(api, money, item)) {
                                Some(item) => self.set_pokemart_state(PokemartState::ChoosingBuyOption(item)),
                                None => self.set_pokemart_state(PokemartState::Quitting),
                            }
                        } else if menu.map_or(false, |m| m.is_yes_no_menu()) {
                            api.toggle_button(JoypadButton::A);
                        } else {
                            let new_ticks = ticks + 1;
                            if let AgentState::PokemartShopping(PokemartState::PurchasedItem { ticks: ref mut t }) = self.state {
                                *t = new_ticks;
                            }
                            api.release_all_buttons();
                            if new_ticks % 2 == 1 {
                                api.press_button(JoypadButton::B);
                            }
                        }
                    }

                    PokemartState::Quitting => {
                        if let Some(menu) = &menu {
                            if menu.is_mart_buy_sell_menu() {
                                match menu.current_item {
                                    2 => api.toggle_button(JoypadButton::A),
                                    n if n < 2 => api.toggle_button(JoypadButton::Down),
                                    _ => api.toggle_button(JoypadButton::Up),
                                }
                            } else {
                                api.toggle_button(JoypadButton::B);
                            }
                        } else {
                            api.toggle_button(JoypadButton::B);
                        }
                    }
                }
            }
            AgentState::TeachingMove { item, target_slot, press, entered_menu, settle, evolve_from, declined } => {
                use crate::pokemon::menu::TextBoxId;
                let gs = api.game_state()?;
                // Done once the move is known or, for a stone, the species has changed.
                let consumed = entered_menu && api.bag_item_position(item).is_none();
                let done = match evolve_from {
                    Some(from) => gs.pokemon.get(target_slot as usize).map_or(false, |p| p.species != from),
                    None => consumed || crate::pokemon::policy::hm_move(item).map_or(false, |mv|
                        gs.pokemon.get(target_slot as usize)
                            .map_or(false, |p| p.moves.iter().flatten().any(|m| m.name == mv))),
                };
                if done {
                    if game_mode != GameMode::Overworld {
                        api.release_all_buttons();
                        if press { api.press_button(JoypadButton::B); }
                        self.set_state(AgentState::TeachingMove { item, target_slot, press: !press, entered_menu, settle: 0, evolve_from, declined });
                        return Ok(());
                    }
                    if settle < 15 {
                        api.release_all_buttons();
                        self.set_state(AgentState::TeachingMove { item, target_slot, press, entered_menu, settle: settle + 1, evolve_from, declined });
                        return Ok(());
                    }
                    let msg = match evolve_from {
                        Some(_) => format!("Evolved party slot {target_slot}"),
                        None => format!("Taught the HM to party slot {target_slot}"),
                    };
                    self.event(AgentEvent::TextBox { message: msg });
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                // Declining the move to forget ends the teach: the game goes back to the bag, and
                // teaching again would only ask again.
                let declined = declined || api.on_screen_text(true).is_some_and(|text| text.contains("did not learn"));
                if declined {
                    api.release_all_buttons();
                    if game_mode != GameMode::Overworld {
                        if press { api.press_button(JoypadButton::B); }
                        self.set_state(AgentState::TeachingMove { item, target_slot, press: !press, entered_menu, settle: 0, evolve_from, declined });
                        return Ok(());
                    }
                    self.event(AgentEvent::TextBox {
                        message: format!("Party slot {target_slot} did not learn the move: no move was chosen to forget"),
                    });
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                // Back in the overworld without learning: start the chain afresh.
                if entered_menu && game_mode == GameMode::Overworld {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                let entered_menu = entered_menu || game_mode != GameMode::Overworld;

                // Press and release on alternate ticks, for a fresh rising edge every two.
                if !press {
                    api.release_all_buttons();
                    self.set_state(AgentState::TeachingMove { item, target_slot, press: true, entered_menu, settle: 0, evolve_from, declined });
                    return Ok(());
                }

                let (top_x, top_y, current, scroll) = api.menu_geometry();
                let tbid = api.menu_state().map(|m| m.text_box_id);
                let nav = |cur: u8, target: u8| -> JoypadButton {
                    if cur < target { JoypadButton::Down }
                    else if cur > target { JoypadButton::Up }
                    else { JoypadButton::A }
                };
                let button = if game_mode == GameMode::Overworld {
                    JoypadButton::Start // no menu yet → open START
                } else if (top_x, top_y) == START_MENU_ORIGIN {
                    // Not a literal 2; see `start_menu_row`.
                    nav(current, start_menu_row(api, StartMenuRow::Item))
                } else if tbid == Some(TextBoxId::ListMenuBox) {
                    // Absolute index is cursor plus scroll.
                    match api.bag_item_position(item) {
                        Some(target_idx) => nav(current + scroll, target_idx),
                        None => JoypadButton::B,
                    }
                } else if tbid == Some(TextBoxId::UseTossMenuTemplate) {
                    nav(current, 0) // USE/TOSS → USE (index 0)
                } else if tbid == Some(TextBoxId::TwoOptionMenu) {
                    nav(current, 0) // "Make room for CUT?" yes/no → YES (index 0)
                } else if tbid == Some(TextBoxId::MessageBox) && top_x == 0 && (top_y == 1 || top_y == 3) {
                    nav(current, target_slot) // party menu ("Teach to which POKéMON?") → the target mon
                } else {
                    JoypadButton::A // transitional text ("Booted up… / 1, 2, 3… and Poof!")
                };
                api.release_all_buttons();
                api.press_button(button);
                self.set_state(AgentState::TeachingMove { item, target_slot, press: false, entered_menu, settle: 0, evolve_from, declined });
            }
            AgentState::CuttingTree { press, entered_menu, tree_pos, slot, move_index, from_row } => {
                if entered_menu && game_mode == GameMode::Overworld {
                    self.cut_tiles.insert((api.game_state()?.map.map, tree_pos));
                    if from_row {
                        self.event(AgentEvent::OverworldActionCompleted {
                            destination: MetaTile::Cut { at: tree_pos } });
                    } else {
                        self.event(AgentEvent::TextBox { message: format!("Cut down the tree at {tree_pos}") });
                    }
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                let entered_menu = entered_menu || game_mode != GameMode::Overworld;

                if !press {
                    api.release_all_buttons();
                    self.set_state(AgentState::CuttingTree { press: true, entered_menu, tree_pos, slot, move_index, from_row });
                    return Ok(());
                }

                // Up or Down here would move the player off the tree.
                let button = if game_mode == GameMode::Overworld {
                    JoypadButton::Start // facing the tree, no menu yet → open START
                } else {
                    field_move_menu_button(api, slot, move_index)
                };
                api.release_all_buttons();
                api.press_button(button);
                self.set_state(AgentState::CuttingTree { press: false, entered_menu, tree_pos, slot, move_index, from_row });
            }
            AgentState::Surfing { press, entered_menu, water_pos, slot, move_index, resume, settle } => {
                if entered_menu && game_mode == GameMode::Overworld {
                    api.release_all_buttons();
                    let mounted = api.mmu().read_pointer(&pokered_symbols::wWalkBikeSurfState) == 2;
                    // Wait for a sustained overworld before handing back.
                    if mounted && settle < MOUNT_SETTLE_TICKS {
                        self.set_state(AgentState::Surfing {
                            press, entered_menu, water_pos, slot, move_index, resume, settle: settle + 1 });
                        return Ok(());
                    }
                    if mounted {
                        self.event(AgentEvent::TextBox { message: format!("Surfed onto the water at {water_pos}") });
                    }
                    // Back to the walk, not the policy.
                    let same_map = api.game_state().map(|g| g.map.map);
                    match resume {
                        Some((destination, map)) if mounted && same_map == Ok(map) =>
                            self.set_state(AgentState::OverworldMovement { destination, map }),
                        Some((destination, map)) if mounted && same_map.is_ok() =>
                            self.surf_crossed_into(destination, same_map.unwrap_or(map)),
                        _ => self.set_state(AgentState::Idle),
                    }
                    return Ok(());
                }
                // A refused surf is remembered, or the same tile is chosen again.
                if api.on_screen_text(false).map_or(false, |t| t.contains("No SURFing")) {
                    let now = api.game_state().map(|g| g.map.map).ok();
                    let map = now.unwrap_or(self.last_map.unwrap_or(Map::PalletTown));
                    self.blocked_tiles.insert((map, water_pos));
                    api.toggle_button(JoypadButton::B);
                    match resume {
                        Some((destination, from)) if now.is_some_and(|now| now != from) =>
                            self.surf_crossed_into(destination, map),
                        Some((destination, _)) => self.abort_overworld(
                            destination,
                            OverworldActionAbortedReason::Textbox,
                            api.game_state().map(|g| g.map.player_position).ok(),
                        ),
                        None => self.set_state(AgentState::Idle),
                    }
                    self.event(AgentEvent::TextBox {
                        message: format!("the game refused SURF at {water_pos}; treating it as land") });
                    return Ok(());
                }
                let entered_menu = entered_menu || game_mode != GameMode::Overworld;

                // Nothing is pressed while the cartridge is moving the player.
                if game_mode == GameMode::Script {
                    api.release_all_buttons();
                    self.set_state(AgentState::Surfing {
                        press: true, entered_menu, water_pos, slot, move_index, resume, settle: 0 });
                    return Ok(());
                }

                if !press {
                    api.release_all_buttons();
                    self.set_state(AgentState::Surfing {
                        press: true, entered_menu, water_pos, slot, move_index, resume, settle: 0 });
                    return Ok(());
                }

                let button = if game_mode == GameMode::Overworld {
                    // Face the water first, then open START.
                    let gs = self.observe_state(api)?;
                    let facing_water = gs.map.tile_in_front().map(|(p, _)| p == water_pos).unwrap_or(false);
                    if facing_water {
                        JoypadButton::Start
                    } else {
                        dir_to(gs.map.player_position, water_pos).unwrap_or(JoypadButton::Start)
                    }
                } else {
                    field_move_menu_button(api, slot, move_index)
                };
                api.release_all_buttons();
                api.press_button(button);
                self.set_state(AgentState::Surfing {
                    press: false, entered_menu, water_pos, slot, move_index, resume, settle: 0 });
            }
            AgentState::UsingFieldMove { press, entered_menu, slot, move_index, from_map, resume, settle } => {
                let map_changed = api.game_state().map_or(false, |s| s.map.map != from_map);
                if entered_menu && (game_mode == GameMode::Overworld || map_changed) {
                    api.release_all_buttons();
                    // Wait for a sustained overworld before handing back.
                    if resume.is_some() && !map_changed && settle < MOUNT_SETTLE_TICKS {
                        self.set_state(AgentState::UsingFieldMove {
                            press, entered_menu, slot, move_index, from_map, resume, settle: settle + 1 });
                        return Ok(());
                    }
                    // Arming Strength hands straight back to the push, not to a new decision.
                    match resume.filter(|_| !map_changed) {
                        Some((boulder, dir)) =>
                            self.set_state(AgentState::PushingBoulder { boulder, dir, armed: true }),
                        None => self.set_state(AgentState::Idle),
                    }
                    return Ok(());
                }
                let entered_menu = entered_menu || game_mode != GameMode::Overworld;

                if !press {
                    api.release_all_buttons();
                    self.set_state(AgentState::UsingFieldMove { press: true, entered_menu, slot, move_index, from_map, resume, settle: 0 });
                    return Ok(());
                }

                let button = if game_mode == GameMode::Overworld {
                    JoypadButton::Start // no target tile — just open START
                } else {
                    field_move_menu_button(api, slot, move_index)
                };
                api.release_all_buttons();
                api.press_button(button);
                self.set_state(AgentState::UsingFieldMove { press: false, entered_menu, slot, move_index, from_map, resume, settle: 0 });
            }
            AgentState::TossingItem { item, press, entered_menu } => {
                use crate::pokemon::menu::TextBoxId;
                if entered_menu && api.bag_item_position(item).is_none() {
                    if game_mode != GameMode::Overworld {
                        api.release_all_buttons();
                        if press { api.press_button(JoypadButton::B); }
                        self.set_state(AgentState::TossingItem { item, press: !press, entered_menu });
                        return Ok(());
                    }
                    api.release_all_buttons();
                    self.event(AgentEvent::TextBox { message: format!("Tossed {item:?} to free a bag slot") });
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                // Back in the overworld with the item still held: let the policy re-issue it.
                if entered_menu && game_mode == GameMode::Overworld {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                let entered_menu = entered_menu || game_mode != GameMode::Overworld;
                if !press {
                    api.release_all_buttons();
                    self.set_state(AgentState::TossingItem { item, press: true, entered_menu });
                    return Ok(());
                }
                let (top_x, top_y, current, scroll) = api.menu_geometry();
                let tbid = api.menu_state().map(|m| m.text_box_id);
                let nav = |cur: u8, target: u8| -> JoypadButton {
                    if cur < target { JoypadButton::Down }
                    else if cur > target { JoypadButton::Up }
                    else { JoypadButton::A }
                };
                let button = if game_mode == GameMode::Overworld {
                    JoypadButton::Start
                } else if (top_x, top_y) == START_MENU_ORIGIN {
                    // Not a literal 2; see `start_menu_row`.
                    nav(current, start_menu_row(api, StartMenuRow::Item))
                } else if tbid == Some(TextBoxId::ListMenuBox) {
                    match api.bag_item_position(item) {
                        Some(target_idx) => nav(current + scroll, target_idx),
                        None => JoypadButton::B,
                    }
                } else if tbid == Some(TextBoxId::UseTossMenuTemplate) {
                    nav(current, 1) // USE/TOSS → TOSS (index 1)
                } else if tbid == Some(TextBoxId::TwoOptionMenu) {
                    nav(current, 0) // "Is it OK to toss away …?" → YES (index 0)
                } else {
                    JoypadButton::A // quantity selector / "Threw away …" text
                };
                api.release_all_buttons();
                api.press_button(button);
                self.set_state(AgentState::TossingItem { item, press: false, entered_menu });
            }
            AgentState::SolvingBoulderPuzzle { boulder: which, target, hole, pushes, settle } => {
                /// Absolute ceiling on the shoves one goal may spend.
                const MAX_PUSHES: u8 = 120;
                /// The bound that protects: shoves since the plan last got shorter.
                const MAX_PUSHES_WITHOUT_PROGRESS: u8 = 12;
                /// Shoves the game never answered.
                const MAX_SILENT_SHOVES: u8 = 3;
                /// Ticks to let a shove's script and its dust settle before re-planning.
                const SETTLE_TICKS: u16 = 12;

                let game_state = self.observe_state(api)?;
                // Success first, because success is what changes the mode.
                if self.boulder_goal.is_some()
                    && pushes > 0
                    && !hole
                    && game_state.map.boulders().contains(&target)
                {
                    self.boulder_goal = None;
                    // `self.event`, not `new_events.push`: this arm returns before the drain.
                    self.event(AgentEvent::OverworldActionCompleted {
                        destination: MetaTile::BoulderGoal { boulder: which, at: target, hole } });
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                if game_state.mode != GameMode::Overworld {
                    // The goal survives the interruption.
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                if settle < SETTLE_TICKS {
                    api.release_all_buttons();
                    self.set_state(AgentState::SolvingBoulderPuzzle { boulder: which, target, hole, pushes, settle: settle + 1 });
                    return Ok(());
                }
                let live = game_state.map.boulders();
                // A push moves a boulder one tile, to a neighbour.
                let step_away = |b: &Point8| (b.x as i32 - which.x as i32).abs()
                                           + (b.y as i32 - which.y as i32).abs();
                let moved_to = live.iter().copied().min_by_key(step_away)
                    .filter(|b| step_away(b) <= 1);

                // A switch keeps its boulder and a hole swallows it, so they complete differently.
                let done = pushes > 0 && match hole {
                    false => live.contains(&target),
                    true => live.contains(&target) || (!live.contains(&which) && moved_to.is_none()),
                };
                if done {
                    self.boulder_goal = None;
                    // `self.event`, not `new_events.push`: this arm returns before the drain.
                    self.event(AgentEvent::OverworldActionCompleted {
                        destination: MetaTile::BoulderGoal { boulder: which, at: target, hole } });
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                if pushes >= MAX_PUSHES
                    || self.boulder_goal_stale >= MAX_PUSHES_WITHOUT_PROGRESS
                    || self.boulder_goal_silences >= MAX_SILENT_SHOVES
                {
                    self.boulder_goal = None;
                    self.abort_overworld(
                        MetaTile::BoulderGoal { boulder: which, at: target, hole },
                        OverworldActionAbortedReason::PuzzleRanLong { pushes },
                        Some(game_state.map.player_position),
                    );
                    return Ok(());
                }
                // Re-planned every time: the floor moves under a stored plan.
                let which = moved_to.unwrap_or(which);
                if let Some((map, _, target, hole)) = self.boulder_goal {
                    self.boulder_goal = Some((map, which, target, hole));
                }
                // The plan's length is the progress measure.
                let plan = game_state.map.solve_boulder_push_for(which, target);
                if let Some(steps) = plan.as_ref().map(|p| p.len())
                    && steps < self.boulder_goal_best
                {
                    self.boulder_goal_best = steps;
                    self.boulder_goal_stale = 0;
                }
                match plan.and_then(|plan| plan.into_iter().next())
                {
                    Some((boulder, push)) => {
                        api.release_all_buttons();
                        self.set_state(AgentState::PushingBoulder { boulder, dir: push, armed: false });
                    }
                    // No solution from this layout.
                    None => {
                        self.boulder_goal = None;
                        let at = Some(game_state.map.player_position);
                        self.abort_overworld(
                            MetaTile::BoulderGoal { boulder: which, at: target, hole },
                            OverworldActionAbortedReason::PuzzleUnsolvable,
                            at,
                        );
                    }
                }
            }
            AgentState::PushingBoulder { boulder, dir, armed } => {
                let game_state = self.observe_state(api)?;
                if game_state.mode != GameMode::Overworld {
                    api.release_all_buttons();
                    // The goal ends too: a battle may move the player off the floor.
                    self.boulder_goal = None;
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                let map = &game_state.map;
                let boulder_at = |p: Point8| map.sprites.iter()
                    .any(|s| s.name.starts_with("Boulder") && !s.hidden && s.position == p);

                // Done once the boulder leaves its tile, one step on or down a hole.
                if !boulder_at(boulder) {
                    api.release_all_buttons();
                    match self.boulder_goal {
                        Some((_, boulder, target, hole)) => {
                            self.boulder_shove_landed();
                            let pushes = self.boulder_goal_pushes;
                            self.set_state(AgentState::SolvingBoulderPuzzle { boulder, target, hole, pushes, settle: 0 });
                        }
                        None => self.set_state(AgentState::Idle),
                    }
                    return Ok(());
                }
                // Asked every tick: it ends this state on anything but success.
                if let Some(refusal) = map.boulder_push_refusal(boulder, dir) {
                    api.release_all_buttons();
                    self.event(AgentEvent::TextBox { message: refusal });
                    // A refused shove ends the goal rather than re-planning into it.
                    let at = map.player_position;
                    if let Some((_, which, target, hole)) = self.boulder_goal.take() {
                        self.abort_overworld(
                            MetaTile::BoulderGoal { boulder: which, at: target, hole },
                            OverworldActionAbortedReason::Unknown,
                            Some(at),
                        );
                        return Ok(());
                    }
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                // Strength is armed here rather than asked for.
                if !game_state.strength_active {
                    api.release_all_buttons();
                    let carrier = crate::pokemon::policy::field_move_carrier(
                        &game_state, crate::pokemon::move_name::PokemonMoveName::Strength)
                        .filter(|_| game_state.badges.contains(crate::pokemon::badge::Badge::RainbowBadge));
                    let Some((slot, move_index)) = carrier.filter(|_| !armed) else {
                        self.event(AgentEvent::TextBox { message: match carrier {
                            // Back from the party menu with the flag still clear.
                            Some(_) => format!(
                                "Strength did not arm, so the boulder at {boulder} was not pushed"),
                            None => "Strength needs a party member that knows it, and the \
                                     RainbowBadge; not pushing".to_string(),
                        }});
                        self.set_state(AgentState::Idle);
                        return Ok(());
                    };
                    self.set_state(AgentState::UsingFieldMove {
                        press: true, entered_menu: false, slot, move_index,
                        from_map: game_state.map.map, resume: Some((boulder, dir)), settle: 0 });
                    return Ok(());
                }
                // The push tile, one step behind the boulder.
                let opposite = match dir {
                    JoypadButton::Up => JoypadButton::Down, JoypadButton::Down => JoypadButton::Up,
                    JoypadButton::Left => JoypadButton::Right, JoypadButton::Right => JoypadButton::Left,
                    other => other,
                };
                let Some(behind) = step_pos(boulder, opposite) else {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                };

                if map.player_position != behind {
                    match map.route_to_push_tile(behind).and_then(|r| r.first().copied()) {
                        Some(btn) => { api.release_all_buttons(); api.press_button(btn); }
                        None => { api.release_all_buttons(); self.set_state(AgentState::Idle); return Ok(()); }
                    }
                } else {
                    api.release_all_buttons();
                    api.press_button(dir);
                }
                self.set_state(AgentState::PushingBoulder { boulder, dir, armed });
            }
            AgentState::UsingItemPc(s) => return crate::pokemon::postgame::item_storage::tick(self, api, s),
            AgentState::UsingPcBox(s) => return crate::pokemon::postgame::pc_box::tick(self, api, s),
            AgentState::UsingPartyScript(s) => return crate::pokemon::postgame::gifts::tick(self, api, s),
            AgentState::Flying(s) => return crate::pokemon::postgame::fly_bike::tick(self, api, s),
            AgentState::Fishing(s) => return crate::pokemon::postgame::fishing::tick(self, api, s),
            AgentState::SellingToMart(s) => return crate::pokemon::postgame::game_corner::sell_tick(self, api, s),
            AgentState::RedeemingPrize(s) => return crate::pokemon::postgame::game_corner::prize_tick(self, api, s),
            AgentState::UsingBagItem(s) => return crate::pokemon::postgame::items::tick(self, api, s),
            AgentState::CheckingTrashCan { target, checked, press, facing } => {
                // Checking a can runs `GymTrashScript`, which leaves the overworld.
                if checked && game_mode == GameMode::Overworld {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                if game_mode != GameMode::Overworld {
                    api.release_all_buttons();
                    if press { api.press_button(JoypadButton::A); }
                    self.set_state(AgentState::CheckingTrashCan { target, checked: true, press: !press, facing });
                    return Ok(());
                }
                let gs = self.observe_state(api)?;
                match gs.map.route_to_face_dir(target, facing).as_deref() {
                    Some([]) => {
                        api.release_all_buttons();
                        if press { api.press_button(JoypadButton::A); }
                        self.set_state(AgentState::CheckingTrashCan { target, checked, press: !press, facing });
                    }
                    Some(&[btn, ..]) => {
                        api.release_all_buttons();
                        api.press_button(btn);
                        self.set_state(AgentState::CheckingTrashCan { target, checked, press: true, facing });
                    }
                    _ => {
                        // No reachable tile beside the target.
                        let what = gs.map.tile_at_checked(target)
                            .map(|tile| format!("{tile}"))
                            .unwrap_or_else(|| "a square that is not on this map".to_string());
                        self.event(AgentEvent::TextBox {
                            message: format!("Could not get next to {what} at {target} to face it") });
                        api.release_all_buttons();
                        self.set_state(AgentState::Idle);
                    }
                }
            }
            AgentState::UsingElevator { panel, floor, selected, press } => {
                let gs = self.observe_state(api)?;
                // Rode the elevator out.
                let in_elevator = matches!(gs.map.map,
                    Map::RocketHideoutElevator | Map::SilphCoElevator | Map::CeladonMartElevator);
                if !in_elevator {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                const SPECIAL_LIST_MENU: u8 = 0x04;
                if !selected && api.list_menu_id() == SPECIAL_LIST_MENU {
                    // The floor menu scrolls, so compare the absolute index with the target floor.
                    let current = api.menu_state().map(|m| m.list_absolute_index()).unwrap_or(0);
                    api.release_all_buttons();
                    let mut selected = selected;
                    if press {
                        use std::cmp::Ordering;
                        match current.cmp(&floor) {
                            Ordering::Less    => api.press_button(JoypadButton::Down),
                            Ordering::Greater => api.press_button(JoypadButton::Up),
                            Ordering::Equal   => { api.press_button(JoypadButton::A); selected = true; }
                        }
                    }
                    self.set_state(AgentState::UsingElevator { panel, floor, selected, press: !press });
                    return Ok(());
                }
                if game_mode != GameMode::Overworld {
                    api.toggle_button(JoypadButton::A);
                    self.set_state(AgentState::UsingElevator { panel, floor, selected, press: true });
                    return Ok(());
                }
                if !selected {
                    match gs.map.route_to_face(panel).as_deref() {
                        Some([]) => {
                            api.release_all_buttons();
                            if press { api.press_button(JoypadButton::A); }
                            self.set_state(AgentState::UsingElevator { panel, floor, selected, press: !press });
                        }
                        Some(&[btn, ..]) => {
                            api.release_all_buttons();
                            api.press_button(btn);
                            self.set_state(AgentState::UsingElevator { panel, floor, selected, press: true });
                        }
                        _ => {
                            self.event(AgentEvent::TextBox { message: format!("Can't reach elevator panel at {panel}") });
                            api.release_all_buttons();
                            self.set_state(AgentState::Idle);
                        }
                    }
                } else {
                    // Floor picked and the exit warp redirected: step onto it.
                    let warp = gs.map.actions().into_iter()
                        .find(|a| matches!(a.tile, MetaTile::Warp { .. }));
                    match warp.as_ref().and_then(|a| a.route.first().copied()) {
                        Some(btn) => {
                            api.release_all_buttons();
                            api.press_button(btn);
                            self.set_state(AgentState::UsingElevator { panel, floor, selected, press: true });
                        }
                        None => {
                            self.event(AgentEvent::TextBox { message: "Can't reach the elevator exit warp".into() });
                            api.release_all_buttons();
                            self.set_state(AgentState::Idle);
                        }
                    }
                }
            }
            AgentState::UsingFieldItem { item, target, press, entered_menu, backing_out } => {
                use crate::pokemon::menu::TextBoxId;
                // Back in the overworld after the bag menus: done.
                if entered_menu && game_mode == GameMode::Overworld {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                if game_mode == GameMode::Overworld {
                    let gs = self.observe_state(api)?;
                    match gs.map.route_to_face(target).as_deref() {
                        Some([]) => {
                            api.release_all_buttons();
                            if press { api.press_button(JoypadButton::Start); }
                            self.set_state(AgentState::UsingFieldItem { item, target, press: !press, entered_menu, backing_out });
                        }
                        Some(&[btn, ..]) => {
                            api.release_all_buttons();
                            api.press_button(btn);
                            self.set_state(AgentState::UsingFieldItem { item, target, press: true, entered_menu, backing_out });
                        }
                        _ => {
                            self.event(AgentEvent::TextBox { message: format!("Can't reach the field-item target at {target}") });
                            api.release_all_buttons();
                            self.set_state(AgentState::Idle);
                        }
                    }
                    return Ok(());
                }
                // A contextual refusal `item_use::field_use_refusal` could not predict.
                if entered_menu && backing_out == 0
                    && api.on_screen_text(false).is_some_and(|t| shows_battle_refusal(&t)) {
                    self.event(AgentEvent::TextBox { message: format!(
                        "the game refused to use {item:?} here and put the bag back; it will refuse \
                         again wherever you are standing, so use the turn on something else") });
                    api.release_all_buttons();
                    self.set_state(AgentState::UsingFieldItem {
                        item, target, press: true, entered_menu, backing_out: BACKING_OUT_TICKS });
                    return Ok(());
                }
                if backing_out > 0 {
                    api.release_all_buttons();
                    if press { api.press_button(JoypadButton::B); }
                    self.set_state(AgentState::UsingFieldItem {
                        item, target, press: !press, entered_menu, backing_out: backing_out - 1 });
                    return Ok(());
                }
                if !press {
                    api.release_all_buttons();
                    self.set_state(AgentState::UsingFieldItem { item, target, press: true, entered_menu: true, backing_out });
                    return Ok(());
                }
                let (top_x, top_y, current, scroll) = api.menu_geometry();
                let tbid = api.menu_state().map(|m| m.text_box_id);
                let nav = |cur: u8, tgt: u8| -> JoypadButton {
                    if cur < tgt { JoypadButton::Down } else if cur > tgt { JoypadButton::Up } else { JoypadButton::A }
                };
                let button = if (top_x, top_y) == START_MENU_ORIGIN {
                    // Not a literal 2; see `start_menu_row`.
                    nav(current, start_menu_row(api, StartMenuRow::Item))
                } else if tbid == Some(TextBoxId::ListMenuBox) {
                    let target_idx = api.bag_item_position(item).unwrap_or(0);
                    nav(current + scroll, target_idx) // bag list → the item's row, then A
                } else if tbid == Some(TextBoxId::UseTossMenuTemplate) {
                    nav(current, 0) // USE/TOSS → USE (index 0)
                } else {
                    JoypadButton::A // transitional / "woke up" text
                };
                api.release_all_buttons();
                api.press_button(button);
                self.set_state(AgentState::UsingFieldItem { item, target, press: false, entered_menu: true, backing_out });
            }
            AgentState::NamingPokemon { species, decided, ticks } => {
                /// Ticks the submitted naming screen gets to close itself.
                const NAMING_BUDGET: u16 = 1500;
                if decided && ticks > NAMING_BUDGET {
                    self.event(AgentEvent::TextBox { message: format!(
                        "naming screen for {species:?} never closed in {NAMING_BUDGET} ticks; giving up") });
                    api.release_all_buttons();
                    self.naming_closed();
                    return Ok(());
                }
                if decided {
                    // The buffer is written: pulse START until `DisplayNamingScreen` exits.
                    let submitted = api.mmu().read_pointer(&pokered_symbols::wNamingScreenSubmitName) != 0;
                    api.toggle_button(if submitted { JoypadButton::A } else { JoypadButton::Start });
                    let still_in_naming = matches!(
                        game_mode,
                        GameMode::TextBox | GameMode::NamingScreen
                            | GameMode::WildBattle | GameMode::TrainerBattle
                    );
                    if !still_in_naming {
                        api.release_all_buttons();
                        self.naming_closed();
                    } else {
                        self.set_state(AgentState::NamingPokemon { species, decided, ticks: ticks + 1 });
                    }
                } else {
                    // The one poll site with no game state already in hand.
                    if let Ok(game_state) = api.game_state() {
                        self.poll_policy(&game_state, api);
                    }
                    if let Some(decision) = self.policy.pick_nickname(species) {
                        // Written straight into the buffer, bypassing the character grid.
                        api.write_naming_screen_buffer(decision.as_deref())?;
                        self.set_state(AgentState::NamingPokemon { species, decided: true, ticks: 0 });
                    } else {
                        api.release_all_buttons();
                    }
                }
            }
        }

        for x in new_events.into_iter() {
            self.event(x);
        }

        Ok(())
    }

}

/// Returns the direction to step from `from` to an orthogonally adjacent `to`.
fn dir_to(from: Point8, to: Point8) -> Option<JoypadButton> {
    match (to.x as i16 - from.x as i16, to.y as i16 - from.y as i16) {
        ( 1,  0) => Some(JoypadButton::Right),
        (-1,  0) => Some(JoypadButton::Left),
        ( 0,  1) => Some(JoypadButton::Down),
        ( 0, -1) => Some(JoypadButton::Up),
        _        => None,
    }
}

/// The tile one step from `from` towards `btn`, or `None` off the top or left edge.
fn step_pos(from: Point8, btn: JoypadButton) -> Option<Point8> {
    match btn {
        JoypadButton::Up    => (from.y > 0).then(|| Point8 { x: from.x, y: from.y - 1 }),
        JoypadButton::Down  => Some(Point8 { x: from.x, y: from.y + 1 }),
        JoypadButton::Left  => (from.x > 0).then(|| Point8 { x: from.x - 1, y: from.y }),
        JoypadButton::Right => Some(Point8 { x: from.x + 1, y: from.y }),
        _ => None,
    }
}

/// How much a menu-shape test is allowed to believe.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum MenuEvidence {
    /// Only what is drawn on the screen right now.
    OnScreen,
    /// The lingering RAM ids as well.
    OrTheLingeringIds,
}

/// True when the screen shows a menu the agent can leave with B, rather than a conversation to
/// confirm with A.
fn open_menu_on_screen(api: &PokemonApi<'_>, evidence: MenuEvidence) -> bool {
    if evidence == MenuEvidence::OrTheLingeringIds
        && api.menu_state().is_some_and(|m| m.is_list_menu() || m.is_field_move_menu()) {
        return true;
    }
    let (top_x, top_y, ..) = api.menu_geometry();
    let screen = api.on_screen_text(false).unwrap_or_default();
    screen.contains("CANCEL") || is_start_menu(top_x, top_y, &screen)
}

/// A row of the START menu, named because its index depends on the Pokédex.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum StartMenuRow {
    Pokemon,
    Item,
}

/// The cursor index (`wCurrentMenuItem`) that selects `row` of the START menu.
pub(crate) fn start_menu_row(api: &PokemonApi<'_>, row: StartMenuRow) -> u8 {
    let has_pokedex = api.mmu().read_has_pokedex();
    match row {
        StartMenuRow::Pokemon => u8::from(has_pokedex),
        StartMenuRow::Item => 1 + u8::from(has_pokedex),
    }
}

/// The button that advances START→POKéMON→mon at `slot`→field move at `move_index` from whatever
/// screen is up, and A for the text between.
pub(crate) fn field_move_menu_button(api: &PokemonApi<'_>, slot: u8, move_index: u8) -> JoypadButton {
    let (top_x, top_y, current, _) = api.menu_geometry();
    let nav = |target: u8| -> JoypadButton {
        if current < target { JoypadButton::Down }
        else if current > target { JoypadButton::Up }
        else { JoypadButton::A }
    };
    if (top_x, top_y) == START_MENU_ORIGIN {
        // Not a literal 1; see `start_menu_row`.
        nav(start_menu_row(api, StartMenuRow::Pokemon))
    } else if top_x == 0 && (top_y == 1 || top_y == 3) {
        nav(slot) // party menu → the mon that knows the move
    } else if api.menu_state().is_some_and(|m| m.is_field_move_menu()) {
        nav(move_index) // field-move box → the move itself
    } else {
        JoypadButton::A // transitional text ("used STRENGTH!", "can now use CUT!")
    }
}

/// Two plain floor tiles to pace between: a neighbour of `pos` and one of its neighbours.
fn adjacent_pacing_pair(map: &crate::pokemon::tile_map::MetaTileMap, pos: Point8) -> Option<(Point8, Point8)> {
    let plain = |p: Point8| (p.x as usize) < map.width && (p.y as usize) < map.height
        && map.meta_tiles[p.x as usize + p.y as usize * map.width] == MetaTile::Empty;
    let neighbours = |p: Point8| [
        Point8 { x: p.x, y: p.y.saturating_sub(1) }, Point8 { x: p.x, y: p.y.saturating_add(1) },
        Point8 { x: p.x.saturating_sub(1), y: p.y }, Point8 { x: p.x.saturating_add(1), y: p.y },
    ].into_iter().filter(move |&n| n != p && plain(n) && !map.pair_blocked(p, n));
    neighbours(pos).find_map(|a| neighbours(a).find(|&b| b != pos).map(|b| (a, b)))
}

/// The squares a [`MetaTile::Pace`] row paces on.
fn pace_kind(water: bool) -> MetaTile {
    if water { MetaTile::Water } else { MetaTile::Empty }
}

/// A grass tile next to `pos` that the player can actually step onto.
fn adjacent_grass(map: &crate::pokemon::tile_map::MetaTileMap, pos: Point8) -> Option<Point8> {
    let neighbors = [
        Point8 { x: pos.x,                  y: pos.y.saturating_sub(1) },
        Point8 { x: pos.x,                  y: pos.y.saturating_add(1) },
        Point8 { x: pos.x.saturating_sub(1),  y: pos.y                 },
        Point8 { x: pos.x.saturating_add(1),  y: pos.y                 },
    ];
    neighbors.into_iter().find(|&p| {
        p != pos
            && (p.x as usize) < map.width
            && (p.y as usize) < map.height
            && map.meta_tiles[p.x as usize + p.y as usize * map.width] == MetaTile::Grass
            && !map.pair_blocked(pos, p)
    })
}

/// True on the outermost row or column, where an edge warp fires by stepping off the map.
fn is_on_map_border(map: &crate::pokemon::tile_map::MetaTileMap) -> bool {
    let pos = map.player_position;
    pos.x == 0
        || pos.y == 0
        || pos.x as usize == map.width.saturating_sub(1)
        || pos.y as usize == map.height.saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::bag::BagItem;
    use crate::pokemon::item::ItemId;
    use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};

    /// A battle turn formats as a sentence.
    #[test]
    fn a_battle_turn_reads_as_a_sentence() {
        let say = |action| format!("{}", AgentEvent::BattleActionStarted {
            actor: "BULBASAUR".into(),
            opponent: "Pidgey".into(),
            action,
        });

        assert_eq!(
            say(BattleAction::Fight { slot: 1, battle_move: PokemonMove::with_max_pp(PokemonMoveName::VineWhip) }),
            "BULBASAUR used Vine Whip on Pidgey",
            "the move's own Display already spaces and capitalises it",
        );
        assert_eq!(
            say(BattleAction::UseItem { slot: 0, item: BagItem::new(ItemId::Potion, 1), target: Some(0) }),
            "used Potion on BULBASAUR",
        );
        assert_eq!(
            say(BattleAction::Run),
            "BULBASAUR tried to run from Pidgey",
            "…tried: this is the action starting, not landing",
        );
        assert_eq!(say(BattleAction::SafariBall), "threw a Safari Ball at Pidgey");
    }

    /// A win formats as a sentence.
    #[test]
    fn a_win_reads_as_a_sentence() {
        let said = format!("{}", AgentEvent::HallOfFame {
            teams: 1,
            playtime: "06:12:44".into(),
            playtime_seconds: 22_364,
            badges: 0xFF,
            party: vec!["VAPOREON".into(), "ARTICUNO".into()],
        });
        assert_eq!(said, "🏆 entered the HALL OF FAME with 8 badges after 06:12:44 of play, with VAPOREON, ARTICUNO");
    }

    /// A ball is thrown at the enemy; every other bag item is used on your own Pokémon.
    #[test]
    fn a_ball_is_thrown_at_the_enemy_and_a_potion_is_not() {
        let say = |item| format!("{}", AgentEvent::BattleActionStarted {
            actor: "BULBASAUR".into(),
            opponent: "Pidgey".into(),
            action: BattleAction::UseItem { slot: 0, item: BagItem::new(item, 1), target: None },
        });

        assert_eq!(say(ItemId::PokeBall), "threw a PokeBall at Pidgey");
        assert_eq!(say(ItemId::MasterBall), "threw a MasterBall at Pidgey");
        assert_eq!(say(ItemId::SuperPotion), "used SuperPotion on BULBASAUR");
        assert_eq!(say(ItemId::XAttack), "used XAttack on BULBASAUR", "an X item acts on your own side");
    }

    /// An abandoned walk says why, and where when it knows.
    #[test]
    fn an_abandoned_walk_says_why() {
        let event = AgentEvent::OverworldActionAborted {
            destination: MetaTile::Pc,
            reason: OverworldActionAbortedReason::Battle,
            at: None,
        };
        assert_eq!(format!("{event}"), "✗ gave up on the PC: a battle started");
        let blocked = AgentEvent::OverworldActionAborted {
            destination: MetaTile::Pc,
            reason: OverworldActionAbortedReason::Textbox,
            at: Some(Point8 { x: 19, y: 11 }),
        };
        assert_eq!(format!("{blocked}"), "✗ gave up on the PC at (19, 11): the game stopped you to say something");
        assert_eq!(
            format!("{}", AgentEvent::StartedOverworldAction { destination: MetaTile::Grass, id: String::new() }),
            "→ heading for tall grass",
        );
    }

    /// A pace in grass reports how it ended.
    #[test]
    fn walking_in_grass_says_what_became_of_it() {
        let nothing = AgentEvent::OverworldActionAborted {
            destination: MetaTile::Grass,
            reason: OverworldActionAbortedReason::NothingAppeared,
            at: Some(Point8 { x: 6, y: 29 }),
        };
        assert_eq!(
            format!("{nothing}"),
            "✗ gave up on tall grass at (6, 29): nothing appeared after 60 seconds of game time walking about in it",
        );
        assert!(format!("{nothing}").contains(&format!("{PACING_BUDGET_SECS} seconds")));

        let met_something = AgentEvent::OverworldActionAborted {
            destination: MetaTile::Grass,
            reason: OverworldActionAbortedReason::Battle,
            at: None,
        };
        assert_eq!(format!("{met_something}"), "✗ gave up on tall grass: a battle started");
    }

    /// A cast that never happened says so, in the sentence that closes the row.
    #[test]
    fn a_cast_that_could_not_be_made_says_why_rather_than_going_quiet() {
        use crate::pokemon::postgame::fishing::{CastRefusal, Rod};
        let edge = MetaTile::Fish { rod: Rod::Super };

        let surfing = AgentEvent::OverworldActionAborted {
            destination: edge,
            reason: OverworldActionAbortedReason::CastRefused(CastRefusal::Surfing),
            at: Some(Point8 { x: 9, y: 25 }),
        };
        assert_eq!(
            format!("{surfing}"),
            "✗ gave up on the water's edge, to fish with the Super Rod at (9, 25): \
             the cast was refused because you are surfing, and a cast is made from land",
        );

        // The rod is named: no rod at all is a different fact from this row's rod having gone.
        let no_rod = AgentEvent::OverworldActionAborted {
            destination: edge,
            reason: OverworldActionAbortedReason::CastRefused(CastRefusal::NoRod(Rod::Old)),
            at: None,
        };
        assert!(format!("{no_rod}").ends_with("the cast was refused because there is no Old Rod in the bag"),
                "{no_rod}");

        // Quoted from the constant.
        let wedged = AgentEvent::OverworldActionAborted {
            destination: edge,
            reason: OverworldActionAbortedReason::CastNeverFinished,
            at: Some(Point8 { x: 8, y: 25 }),
        };
        assert_eq!(
            format!("{wedged}"),
            "✗ gave up on the water's edge, to fish with the Super Rod at (8, 25): \
             the cast never finished after 30 seconds of game time",
        );
        assert!(format!("{wedged}").contains(&format!(
            "{} seconds", crate::pokemon::postgame::fishing::CAST_BUDGET_SECS)));

        // An unreachable shore uses the existing no-route sentence.
        let unreachable = AgentEvent::OverworldActionAborted {
            destination: edge,
            reason: OverworldActionAbortedReason::NoRoute(edge),
            at: Some(Point8 { x: 8, y: 25 }),
        };
        assert_eq!(
            format!("{unreachable}"),
            "✗ gave up on the water's edge, to fish with the Super Rod at (8, 25): \
             there is no route to the water's edge, to fish with the Super Rod",
        );
    }

    /// A pace and a Surf mount carrying a walk are open overworld actions; nothing else is.
    #[test]
    fn a_pace_is_an_open_overworld_action_like_the_walk_that_started_it() {
        let pacing = AgentState::PacingForEncounters {
            destination: MetaTile::Grass,
            map: Map::Route18,
            tile_a: Point8 { x: 39, y: 13 },
            tile_b: Point8 { x: 39, y: 14 },
            heading_to_b: true,
            stalled: 0,
            paced: 0,
        };
        assert_eq!(pacing.open_overworld_action(), Some(MetaTile::Grass));

        let walking = AgentState::OverworldMovement { destination: MetaTile::Grass, map: Map::Route18 };
        assert_eq!(walking.open_overworld_action(), Some(MetaTile::Grass));

        let mounting = AgentState::Surfing {
            press: true, entered_menu: false,
            water_pos: Point8 { x: 5, y: 24 }, slot: 1, move_index: 0,
            resume: Some((MetaTile::Sprite("Fisher 1"), Map::Route21)), settle: 0,
        };
        assert_eq!(mounting.open_overworld_action(), Some(MetaTile::Sprite("Fisher 1")));

        let asked_for = AgentState::Surfing {
            press: true, entered_menu: false,
            water_pos: Point8 { x: 5, y: 24 }, slot: 1, move_index: 0,
            resume: None, settle: 0,
        };
        assert_eq!(asked_for.open_overworld_action(), None);

        assert_eq!(AgentState::Idle.open_overworld_action(), None);
        assert_eq!(
            AgentState::ReadingTextBox { reader: PokemonTextReader::default() }.open_overworld_action(),
            None,
        );
    }

    /// A cut completes as the deed, not the journey.
    #[test]
    fn a_tree_that_was_cut_down_says_so_in_the_agents_own_voice() {
        let cut = AgentEvent::OverworldActionCompleted {
            destination: MetaTile::Cut { at: Point8 { x: 5, y: 8 } },
        };
        assert_eq!(format!("{cut}"), "✓ cut down the tree at (5, 8)");
        assert_eq!(
            format!("{}", AgentEvent::OverworldActionCompleted { destination: MetaTile::Pc }),
            "✓ reached the PC",
        );
    }

    #[test]
    fn a_walk_that_never_arrived_does_not_claim_there_was_no_route() {
        let warp = MetaTile::Warp { to_map: Map::Route8Gate, to_position: Point8 { x: 5, y: 3 } };
        let gave_up = AgentEvent::OverworldActionAborted {
            destination: warp,
            reason: OverworldActionAbortedReason::DidNotArrive,
            at: Some(Point8 { x: 10, y: 9 }),
        };
        let said = format!("{gave_up}");
        assert_eq!(
            said,
            "✗ gave up on the warp to Route8Gate at (10, 9): \
             the walk was given up after 60 seconds of game time without getting there",
        );
        assert!(!said.contains("no route"), "the route was there on every tick: {said}");
        assert!(!said.contains('—'), "no em dashes in what the agent generates: {said}");
    }

    /// A walk names its destination.
    #[test]
    fn a_walk_says_where_it_is_going() {
        let started = |destination| format!("{}", AgentEvent::StartedOverworldAction { destination, id: String::new() });
        let reached = |destination| format!("{}", AgentEvent::OverworldActionCompleted { destination });

        let warp = MetaTile::Warp { to_map: Map::OaksLab, to_position: Point8 { x: 5, y: 11 } };
        assert_eq!(started(warp), "→ heading for the warp to OaksLab");
        assert_eq!(reached(warp), "✓ reached the warp to OaksLab");

        let connection = MetaTile::Connection { to_map: Map::Route1, to_position: Point8 { x: 9, y: 0 } };
        assert_eq!(started(connection), "→ heading for the way into Route1");
        assert_eq!(started(MetaTile::ConnectionWater(Map::Route20)), "→ heading for the water crossing into Route20");

        assert_eq!(started(MetaTile::Sprite("Mom")), "→ heading for Mom");
        assert_eq!(
            format!("{}", OverworldActionAbortedReason::NoRoute(MetaTile::Sprite("Gym Guide"))),
            "there is no route to Gym Guide",
            "the same Display has to read as English inside the abort reason too",
        );
    }

    /// Talking to someone reads as the action succeeding.
    #[test]
    fn an_interaction_that_landed_reads_as_one() {
        assert_eq!(
            format!("{}", AgentEvent::OverworldInteractionCompleted { target: MetaTile::Sprite("Mom") }),
            "✓ talked to Mom",
        );
        assert_eq!(
            format!("{}", AgentEvent::OverworldInteractionCompleted { target: MetaTile::Pc }),
            "✓ used the PC",
            "the same event covers a PC, which is not someone you talk to",
        );
    }

    /// An id keeps the variant name the prose drops.
    #[test]
    fn an_id_keeps_the_variant_name_the_prose_left_behind() {
        assert_eq!(MetaTile::Warp { to_map: Map::OaksLab, to_position: Point8 { x: 5, y: 11 } }.kind(), "Warp");
        assert_eq!(MetaTile::Sprite("Mom").kind(), "Sprite");
        assert_eq!(MetaTile::ConnectionWater(Map::Route20).kind(), "ConnectionWater");
        assert_eq!(MetaTile::Pc.kind(), "Pc");
    }

    /// A person's id kind is their name; every other variant's is the variant.
    #[test]
    fn a_person_is_named_by_their_id_and_not_called_a_sprite() {
        assert_eq!(MetaTile::Sprite("Mom").id_kind(), "Mom");
        assert_eq!(MetaTile::Sprite("Middle Aged Woman").id_kind(), "MiddleAgedWoman");
        assert_eq!(MetaTile::Sprite("Pokedex 1").id_kind(), "Pokedex1");

        assert_eq!(MetaTile::Warp { to_map: Map::OaksLab, to_position: Point8 { x: 5, y: 11 } }.id_kind(), "Warp");
        assert_eq!(MetaTile::Pc.id_kind(), "Pc");
        assert_eq!(MetaTile::Grass.id_kind(), "Grass");

        assert_eq!(format!("{}", MetaTile::Sprite("Middle Aged Woman")), "Middle Aged Woman");
    }

    #[test]
    fn an_empty_text_box_is_not_worth_saying() {
        assert!(!AgentEvent::TextBox { message: String::new() }.is_worth_reporting());
        assert!(!AgentEvent::TextBox { message: "   \n ".into() }.is_worth_reporting());
        assert!(AgentEvent::TextBox { message: "Wild PIDGEY appeared!".into() }.is_worth_reporting());
        assert!(AgentEvent::BattleStarted.is_worth_reporting());
        assert!(AgentEvent::BattleEnded.is_worth_reporting());
    }

    /// Every event and abort reason formats to something.
    #[test]
    fn no_event_formats_to_nothing() {
        let events = [
            AgentEvent::StartedOverworldAction { destination: MetaTile::Pc, id: String::new() },
            AgentEvent::OverworldActionAborted { destination: MetaTile::Pc, reason: OverworldActionAbortedReason::Unknown, at: None },
            AgentEvent::OverworldActionCompleted { destination: MetaTile::Pc },
            AgentEvent::OverworldInteractionCompleted { target: MetaTile::Pc },
            AgentEvent::BattleStarted,
            AgentEvent::BattleActionStarted { actor: "X".into(), opponent: "Y".into(), action: BattleAction::SafariRock },
            AgentEvent::BattleEnded,
            AgentEvent::TextBox { message: "hello".into() },
            AgentEvent::WatchdogFired { agent_state: "wait".into(), stuck_for: Duration::from_secs(300) },
        ];
        for event in events {
            assert!(!format!("{event}").trim().is_empty(), "{event:?} formats to nothing");
        }
        for reason in [
            OverworldActionAbortedReason::Unknown,
            OverworldActionAbortedReason::Script,
            OverworldActionAbortedReason::Battle,
            OverworldActionAbortedReason::Textbox,
            OverworldActionAbortedReason::NamingScreen,
            OverworldActionAbortedReason::WrongMap(Map::PalletTown),
            OverworldActionAbortedReason::NoAdjacentGrass,
            OverworldActionAbortedReason::NoRoute(MetaTile::Grass),
        ] {
            assert!(!format!("{reason}").trim().is_empty(), "{reason:?} formats to nothing");
        }
    }
}
