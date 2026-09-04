use std::collections::VecDeque;
use std::fmt::Display;
use std::time::Duration;
use crate::cycles::MachineCycles;
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
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
///
/// ⚠️ **Gen 1 does not sell you as many as you can afford.** An unaffordable quantity gets "You
/// don't have enough money!" and hands over *nothing*, which from outside the menu is
/// indistinguishable from a dropped YES-confirm — so the policy retries, hits `MAX_MART_ATTEMPTS`,
/// prints "gave up" and the leg walks on with an empty bag. `silph_co_card_key_steps` had been
/// asking for 15 Hyper Potions (¥18,000) on ¥7,838 and buying zero of them every run since it was
/// written, which is what left `can_beat_silph_giovanni` blacking out on Silph 11F. The price comes
/// from the ROM's own `ItemPrices` table.
///
/// ⚠️ **A function rather than the two inline copies it replaced**: both the first purchase of a
/// visit and every one after it go through here, and a second purchase that forgot the trim would
/// reproduce that bug exactly, in the arm nobody was looking at.
fn affordable(api: &PokemonApi<'_>, money: u32, item: BagItem) -> Option<BagItem> {
    let item = match api.item_price(item.id) {
        Some(price) => BagItem::new(item.id, item.quantity.min((money / price) as u8)),
        // Not in the price table (a key item, or a mart-specific TM): order as asked.
        None => item,
    };
    (item.quantity > 0).then_some(item)
}

// too long and player veers off course on the overworld, too short and the game doesn't get chance to update values between turns
pub const AGENT_RESOLUTION: MachineCycles = MachineCycles::from_duration(Duration::from_millis(20));

/// How many manual button presses [`PokemonAgent::queue_manual_input`] will hold. A bound rather
/// than a tuned number: each press costs [`MANUAL_INPUT_TICKS_PER_PRESS`] agent ticks, so a full
/// queue is ~1 s of emulated time during which the state machine is not running.
pub const MANUAL_INPUT_CAPACITY: usize = 16;

/// Agent ticks a manual press is **held** before being released.
///
/// One tick is 20 ms, longer than a frame, and yet holding for a single tick is **not** reliable:
/// measured against START in Pallet Town it opens the menu at 11 of 16 successive tick alignments and
/// silently does nothing at the other 5, because pokered does not sample the pad on every frame in
/// the overworld. Two ticks lands at all 16 (`probe_manual_input_hold_length`), so this is a measured
/// number, not a guess — and a dropped press is the worst possible failure here, since the LLM has no
/// way to tell one from a button the game ignored on purpose.
const MANUAL_INPUT_HOLD_TICKS: u8 = 2;

/// Total ticks one queued press occupies: the hold, plus one tick released.
///
/// The released tick is what makes repeats work — a held button is one continuous press, and pokered
/// drives menus off *newly* pressed bits, so without a gap "A, A" would land as a single A and every
/// run of identical buttons would be delivered once.
pub const MANUAL_INPUT_TICKS_PER_PRESS: u8 = MANUAL_INPUT_HOLD_TICKS + 1;

/// `wMovementFlags` bit 7 — set for exactly as long as an **arrow tile** is sliding the player, and
/// cleared the moment they come to rest (`engine/overworld/movement.asm:62`,
/// `home/overworld.asm:270`).
///
/// ⚠️ **The neighbouring bit is the ledge hop** (`BIT_LEDGE_OR_FISHING`, 6 — see
/// `postgame::fishing`), which is the case `RunningScript`'s rollback window was sized for. The
/// cartridge tells the two apart in one byte, so nothing here has to guess from a duration.
const BIT_SPINNING: u8 = 1 << 7;

/// True while an arrow tile is carrying the player.
///
/// ⚠️ **Read from the cartridge rather than from `MetaTileMap::spinners`, though both were
/// available.** The table is decoded from the ROM's movement RLE and says which tiles slide and
/// where to; this says whether one is sliding *now*, which is the question, and it cannot drift
/// from the table because neither is derived from the other. Matching a position against the table
/// would also have to be right about *when* the coordinate updates mid-slide, and it does not:
/// measured, the position at the first spinning tick is the arrow itself and at the last is the
/// destination, with the tiles between them not in the table at all.
fn player_is_spinning(api: &PokemonApi) -> bool {
    api.mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wMovementFlags)
        & BIT_SPINNING != 0
}

/// True from the moment the player's last Pokémon faints until the black-out warp has put them
/// outside a Pokémon Centre. See [`crate::pokemon::battle::LOST_BATTLE`] for the byte.
///
/// ⚠️ **A blackout ends the battle a second or two before it moves the player, and the agent used
/// to ask the model what to do in that gap.** The battle engine returns as soon as the party is
/// down, so `wIsInBattle` leaves its wild/trainer value and [`Self::assert_battle_state`] correctly
/// reports the battle over and drops to `Idle`; `AwaitingOverworldAction` then counts down
/// [`DelayContext::long`] and polls. Meanwhile `HandleBlackOut` is still fading the screen. The
/// decision that came out of that gap described a world that no longer existed by the time it was
/// answered — deployed on 2026-09-02, the model was shown `ViridianForest at (26, 19)`, a party at
/// `0/25` and `0/21` HP, ¥1960, and the nine Viridian Forest actions, while the game was busy
/// putting it outside the Viridian Pokémon Centre with a full party and ¥980. It read the blackout
/// out of the text and answered sensibly — take the south gate out of the forest — and the id was
/// rejected on arrival, because ids are minted for the map you are standing on. **31 of the
/// previous run's 38 blackouts spent a whole request that way**, and the Cerulean Gym stretch is a
/// loop of them.
///
/// ⚠️ **Nothing here is a timeout and nothing needs one.** `$ff` is written and cleared inside a
/// single `HandleBlackOut` call chain that takes no player input, so waiting on it cannot wedge;
/// the [`MAX_BLACKOUT_WAIT_TICKS`] bound below exists for the same reason every other wait in this
/// file has one, not because a path is known that would reach it.
///
/// ⚠️ **The poison black-out is deliberately *not* covered.** `wOutOfBattleBlackout`
/// (`engine/events/poison.asm:112`) says the same thing for a party finished off by poison in the
/// field, but it is only rewritten on a **step** — so it stays `$ff` from the warp until the player
/// walks again, and an agent that waits on it waits for a step it is the one refusing to take.
/// That path is also far less exposed: the player is mid-`OverworldMovement`, and the walk aborts
/// on the map change, which is evidence the new map has already loaded.
fn blackout_in_flight(api: &PokemonApi) -> bool {
    api.mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wIsInBattle)
        == crate::pokemon::battle::LOST_BATTLE
}

/// ⚠️ **A walk that never arrives is silence, and silence is what the watchdog reads.**
/// The route is re-derived every tick, so a destination that stays reachable-looking but
/// is never reached — an NPC parked on the only approach tile, a route that re-plans
/// into the same blocked step — presses buttons for ever without the policy being asked
/// anything. `soak` found it in Oak's Lab: 300 s of game time walking at a sprite.
///
/// ⚠️ **Measured as silence rather than as ticks in this state, because something else
/// resets the state.** This was a `state_ticks` counter on the agent, and
/// `OverworldMovement` was believed to be the one state where that works, since it does
/// not rebuild itself. It has a second way of being torn down: anything that *interrupts*
/// it. Seafoam Islands B4F is the case — the water current takes the player every few
/// seconds, the agent passes through `RunningScript` and back, and each pass hands the
/// walk a fresh 90 s. `soak` found it swimming into the same current for the rest of the
/// run, 119 s of silence and counting. `cycles_since_poll` is cleared by exactly one
/// thing: the policy being asked something, which is what this bound exists to force.
///
/// 60 s of game time is 200 tile steps — further than any single map is wide — and a
/// fifth of the watchdog. It is lower than the 90 s of ticks it replaces for a reason
/// beyond taste: `stalls` fails a case that takes 90 s to escape, so a bound at 90 s
/// could not be regression-tested by the tier built to regression-test it. An
/// interrupted walk is *more* generous than it was, not less: a battle in the middle
/// polls, which resets the clock. Giving up aborts the action, which returns to the
/// policy for a new one.
///
/// ⚠️ **Reported as [`OverworldActionAbortedReason::DidNotArrive`], never as `NoRoute`.** The two
/// are opposite diagnoses and the model acts on them differently: `NoRoute` says the action is gone
/// from `actions()` and something else should be chosen, while this says the route was there and the
/// walk simply never got to the end of it. Reporting the bound as `NoRoute` sent the deployed run of
/// 2026-09-02 hunting a pathfinder bug that did not exist — it read "there is no route to the warp
/// to Route8Gate" while standing two tiles from that warp, and filed an issue about it.
const MAX_MOVEMENT_SILENCE: Duration = Duration::from_secs(60);

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
    /// The walk ran for [`MAX_MOVEMENT_SILENCE`] and never arrived. Not a routing failure: the
    /// route existed on every tick, it just never ended.
    DidNotArrive,
}

impl Display for OverworldActionAbortedReason {
    /// Why the walk stopped, in the words a viewer would use. The `{:?}` this replaced put
    /// `NoAdjacentGrass` and `WrongMap(ViridianForest)` in the log, which read as internals because
    /// they were.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => write!(f, "it stopped making progress"),
            Self::Script => write!(f, "the game took over"),
            Self::Battle => write!(f, "a battle started"),
            // ⚠️ **Not "something was said".** This reason is only ever reached by a box the walk
            // did *not* go looking for (a landed interaction is
            // `OverworldInteractionCompleted`), and the case that reaches it most is a script
            // stopping the player mid-step: the rival on the way out of Oak's lab, Oak at the edge
            // of Route 1. Reporting that as what was said described the symptom and buried the
            // fact the model needs, which is that the walk was cut short by something outside it.
            //
            // ⚠️ **And not "it was interrupted" either, which is what this said for months.** The
            // fact was right and the word was not: an interruption reads as a malfunction, and
            // being stopped by a guard, a locked door or a scripted scene is the game working
            // exactly as designed. The deployed run walked at the Viridian Gym door without the
            // badges to open it, got "✗ gave up on the warp to ViridianGym: it was interrupted"
            // with "The GYM's doors are locked..." on the line below, and filed a `report_issue`
            // asking a developer to "verify action targeting for gym entrance vs warp". Nothing had
            // gone wrong; the sentence just told it something had. So it now names the *cause* —
            // the game chose to speak — and points at the message that follows, which is the thing
            // to act on. `prompt::SYSTEM_PROMPT` says the same in full, once.
            Self::Textbox => write!(f, "the game stopped you to say something"),
            Self::NamingScreen => write!(f, "the naming screen opened"),
            Self::WrongMap(map) => write!(f, "it ended up on {map}"),
            Self::NoAdjacentGrass => write!(f, "there is no grass to step into"),
            Self::NoRoute(tile) => write!(f, "there is no route to {tile}"),
            // ⚠️ **Says what it is, and says the walk is what failed.** This shared `NoRoute`'s
            // sentence until 2026-09-03, and "there is no route to the warp to Route8Gate" is a
            // claim about the pathfinder that was false every time it was printed: the route was
            // found, re-derived on every tick, and walked for a solid minute of game time. A model
            // told the pathfinder is broken goes looking for the bug rather than for another way
            // round, which is what the run did for nine of these. The coordinate the event prints
            // beside it is where the walk got to, which is the other half of the answer.
            Self::DidNotArrive => write!(
                f, "the walk was given up after {} seconds of game time without getting there",
                MAX_MOVEMENT_SILENCE.as_secs()),
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
    StartedOverworldAction { destination: MetaTile },
    /// ⚠️ **`at` is where the walk actually stopped, in the *expanded* coordinates everything else
    /// the model reads uses** — the ids in the action menu, the ruler on the map picture, the
    /// `Location:` line of every turn. Never `raw_player_coords`, which is the same square in a
    /// different space and would have the model reasoning about two maps at once.
    ///
    /// It is here because the reason on its own was not enough to act on. The deployed run
    /// aborted on `the way into Route2` **143 times** — the Viridian old man, who blocks the north
    /// exit until Oak's Parcel is delivered, so the walk was impossible every time and said only
    /// that something stopped it. Repeating the same square is the fact that makes a blocked route
    /// tell itself apart from bad luck, and ⚠️ **working that out is deliberately left to the
    /// model**: nothing here counts aborts, refuses a target that keeps failing, or hides one from
    /// the menu. `None` where the position could not be read; the sentence simply omits it.
    OverworldActionAborted {
        destination: MetaTile,
        reason: OverworldActionAbortedReason,
        at: Option<Point8>,
    },
    OverworldActionCompleted { destination: MetaTile },
    /// The agent walked to something interactable — a person, a PC — pressed A, and the game
    /// answered.
    ///
    /// ⚠️ **A text box is the *only* success signal an interaction has**, which is why this is its
    /// own event rather than an `OverworldActionCompleted`. The route to a sprite ends in an A
    /// press, and it is re-derived every tick, so once the player is facing the sprite the route is
    /// `[A]` for ever and the "route ran out" branch that completes a walk is never reached. A text
    /// box opening *is* the arrival. It used to be reported as
    /// `OverworldActionAborted { reason: Textbox }` — "✗ gave up on Mom: it was interrupted" —
    /// which read as a failure on the page and, worse, told the model its own successful
    /// conversation had failed, in the one field the prompt calls the most useful thing the agent
    /// can say.
    OverworldInteractionCompleted { target: MetaTile },
    /// An item lying on the ground was walked up to and is **still lying there**, so nothing was
    /// picked up.
    ///
    /// ⚠️ **The refusal and the success are the same three events, which is why this had to be its
    /// own one.** Every ball on the floor is a sprite: the agent walks to it, the game opens a text
    /// box, the box closes and the walk is reported as `✓ talked to Charmander Poke Ball`. When the
    /// pickup *worked* the game `HideObject`s the sprite; when it did not, the sprite is still in
    /// the map and the only difference is a sentence the model has to infer the meaning of. The
    /// deployed run of 2026-08-27 read `Those are POKé BALLs. They contain POKéMON!` six times in
    /// Oak's lab, spent turns 7 to 24 there, and filed a `report_issue` saying its party and its
    /// bag were both empty. The starter balls refuse until Oak has offered them, a ball already
    /// taken keeps its row, and ⚠️ **a bag with twenty kinds in it refuses every pickup in the game
    /// in exactly the same shape** — which is the case this is really for, since nothing else
    /// anywhere reports it.
    OverworldPickupFailed { target: MetaTile },
    BattleStarted,
    /// ⚠️ **`actor` and `opponent` are carried rather than looked up later** because nothing
    /// downstream can look them up: the host formats events off the emulator thread and the battle
    /// has moved on by then. `actor` is the acting party member's nickname, which is what turns
    /// `Fight { slot: 1, battle_move: PokemonMove { name: Growl, pp: 40 } }` into `BULBASAUR used
    /// Growl`; `opponent` is what it was aimed at, which is the only thing in the whole battle log
    /// that says what the run is fighting.
    BattleActionStarted { actor: String, opponent: String, action: BattleAction },
    BattleEnded,
    TextBox { message: String },
    /// **W9 / §14** — the agent went `stuck_for` of emulated time without reaching a decision point
    /// of any kind, and the watchdog woke the policy up.
    ///
    /// ⚠️ **This is a bug report, not a status line.** In a healthy run it never happens: the states
    /// that never consult the policy are meant to be transient, so one of these means the agent got
    /// wedged in `agent_state` and something outside it had to intervene. It goes to the model, to
    /// the UI, to the transcript and to stdout — all four, so that a watchdog that quietly rescues a
    /// run cannot turn an agent bug into an invisible ongoing cost.
    WatchdogFired { agent_state: String, stuck_for: Duration },
    /// **The game was beaten.** `wNumHoFTeams` went up, which happens exactly once per Hall of Fame
    /// ceremony, at its very first frame — see [`crate::pokemon::GameState::hall_of_fame_teams`] for
    /// why that byte and not another.
    ///
    /// ⚠️ **Fired at the *top* of the ceremony**, before the party parade, the credits, the game's
    /// own save and its soft reset back to the title screen. Everything after this point is three
    /// emulated minutes of cutscene that the run may or may not survive; the record is banked first.
    ///
    /// ⚠️ **The payload is carried rather than looked up later**, for the reason
    /// [`Self::BattleActionStarted`] carries `actor`: this is read by the host and the archiver *off*
    /// the emulator thread, and by the time either of them gets to it the ceremony has cleared the
    /// party out of the display and the clock has moved on.
    HallOfFame {
        /// `wNumHoFTeams` after the increment. `1` is a first championship; `2` a second one in the
        /// same save.
        teams: u8,
        /// `HH:MM:SS` off the cartridge's own clock. `playtime_seconds` is the same value in the
        /// form anything that ranks runs must use — see [`crate::pokemon::observe::playtime_seconds`].
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

    /// Whether this is worth saying at all.
    ///
    /// ⚠️ **The text reader emits a great many empty boxes.** A textbox is detected before its
    /// characters have been drawn, so an ordinary battle turn produces several `📖` events with
    /// nothing after them — on the deployed run they were most of the log. Dropping them at
    /// [`PokemonAgent::event`] rather than in the page keeps the transcript clean too, which is what
    /// `/api/history` replays into a browser that has just loaded.
    fn is_worth_reporting(&self) -> bool {
        match self {
            Self::TextBox { message } => !message.trim().is_empty(),
            _ => true,
        }
    }
}

impl Display for AgentEvent {
    /// **What the web UI puts in its log**, via `format!("{event}")` in `host.rs` — so this is
    /// prose, not a debug dump. The structured form is still there for anyone who wants it: the
    /// page keeps the JSON it received and shows it on demand.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentEvent::StartedOverworldAction { destination } =>
                write!(f, "→ heading for {destination}"),
            AgentEvent::OverworldActionAborted { destination, reason, at } => match at {
                Some(at) => write!(f, "✗ gave up on {destination} at ({}, {}): {reason}", at.x, at.y),
                None => write!(f, "✗ gave up on {destination}: {reason}"),
            },
            AgentEvent::OverworldActionCompleted { destination } =>
                write!(f, "✓ reached {destination}"),
            // Not rendered by the page — `useEventStream`'s `fold` drops the kind, because the
            // dialogue that follows says everything this line does. It is still said to the model
            // and still written to the transcript, which is the whole reason it is an event.
            AgentEvent::OverworldInteractionCompleted { target } => match target {
                MetaTile::Sprite(name) => write!(f, "✓ talked to {name}"),
                other => write!(f, "✓ used {other}"),
            },
            // ⚠️ **Says what is true rather than that something failed.** "the pickup failed" is
            // the `it was interrupted` wording all over again: it reads as a malfunction, and the
            // commonest causes are the game working — a full bag, or an item the script has not
            // released yet. What the model can act on is that the thing is still on the floor, and
            // the box it just read says why.
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
                // The tense is deliberate: this is the action being *started*, not its outcome, so
                // "used" is as far as it goes — whether the move hit is the next line's business.
                BattleAction::Fight { battle_move, .. } => write!(f, "{actor} used {} on {opponent}", battle_move.name),
                // ⚠️ **A ball is thrown at the *enemy*.** Everything in the bag used to read as
                // "used X on {actor}", so every Poké Ball in the log claimed to have been used on
                // the party member throwing it. The split is by what the item does, not by where it
                // sits in the bag.
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
            // The one line in the log that is not narration of a step but the end of the story, so
            // it says the three things someone reading it later will want: that it happened, how
            // long it took, and who did it.
            AgentEvent::HallOfFame { playtime, badges, party, .. } =>
                write!(f, "🏆 entered the HALL OF FAME with {} badges after {playtime} of play, with {}",
                       badges.count_ones(), party.join(", ")),
        }
    }
}

/// What the player's side of the battle is called, for [`AgentEvent::BattleActionStarted`].
///
/// The party nickname rather than `battle.player.species`, because that is what the game itself puts
/// on screen and what the model's own notes will be using. Falls back to the species when the slot
/// cannot be read — a Safari battle or a mid-transition state — rather than reporting an empty name.
fn active_pokemon_name(state: &GameState) -> String {
    let Some(battle) = state.battle.as_ref() else { return String::new() };
    state
        .pokemon
        .get(battle.active_party_slot as usize)
        .map(|mon| mon.nickname.to_default_string())
        .unwrap_or_else(|| format!("{}", battle.player.species))
}

/// What the other side is called, for [`AgentEvent::BattleActionStarted`].
///
/// The species rather than a nickname, because an enemy has none the player can see. Read at the
/// **decision point**, which is the only moment it is trustworthy: `InitWildBattle` sets
/// `wIsInBattle` *before* `LoadEnemyMonData` (`engine/battle/core.asm:6695`) and a trainer's first
/// Pokémon is not loaded until they send it out, so anything reading the enemy at the moment the
/// battle *starts* would report whatever the last battle left behind. By the time the battle menu is
/// up and the policy is being asked, it is loaded.
fn opponent_pokemon_name(state: &GameState) -> String {
    match state.battle.as_ref() {
        Some(battle) => format!("{}", battle.enemy.species),
        // Not reachable from the one caller — the battle menu is up — but a name is going into a
        // sentence, and "used Growl on " is worse than a vague noun.
        None => "the foe".to_string(),
    }
}

/// Whether using this item in battle aims it at the *enemy* rather than at the party member whose
/// turn it is. Balls are the whole of it: everything else the agent can use in a battle — potions,
/// the X items, the Poké Doll — acts on the player's own side.
fn thrown_at_the_enemy(item: crate::pokemon::item::ItemId) -> bool {
    use crate::pokemon::item::ItemId;
    matches!(item, ItemId::PokeBall | ItemId::GreatBall | ItemId::UltraBall | ItemId::MasterBall | ItemId::SafariBall)
}

/// Ticks a freshly opened battle sub-menu is believed from RAM rather than from the screen. See
/// [`BattleState::confirming`] for why the two disagree at all.
///
/// 15 ticks is 300 ms of game time, against the ≤3 frames (~50 ms) `AutoBgMapTransfer` takes to get
/// the whole tilemap into VRAM — comfortable margin, and still far inside the seconds a battle turn
/// takes to resolve, so a main menu that has genuinely come back is never held up for long.
const CONFIRMING_TICKS: u16 = 15;

/// Ticks of B-mashing [`BattleState::WaitingForMenu`] does after the game refuses a move. See the
/// ⚠️ at the top of that arm for why it is a countdown rather than a flag.
const BACKING_OUT_TICKS: u16 = 100;

/// How long the agent may reach **no decision point at all** before [`AgentState::ReadingTextBox`]
/// stops confirming what is on screen and starts trying to leave it.
///
/// ⚠️ **Measured as silence, not as time in the state.** The obvious version — a counter in the
/// `ReadingTextBox` variant — was written first and does not work: escaping a menu takes the game
/// through the one it was opened from, so the state flickers to `Idle`/`AwaitingOverworldAction` for
/// a tick or two on the way out, the counter restarts, and the reader goes back to mashing A into the
/// START menu it had almost left. The Surf refusal loops through exactly that flicker every 30 s.
/// [`PokemonAgent::since_last_policy_poll`] cannot be reset by anything except the thing that means
/// the escape worked — the policy being asked something — so it survives the whole way out.
///
/// It is also the same measure the watchdog and `soak` read, which is what makes the number
/// defensible: 30 s is well past every scripted conversation `full_playthrough` crosses at the
/// cartridge's own medium text speed, well past ordinary play's ~6 s longest silence, and well inside
/// both the 90 s the `stalls` tier allows and the watchdog's 300 s.
const TEXT_BOX_ESCAPE_SILENCE: Duration = Duration::from_secs(30);

/// Agent ticks (20 ms each) after a text box opens in which it may still be recognised as a **menu
/// the agent inherited** rather than a conversation. See [`PokemonAgent::menu_handover_ticks`] for
/// the measurements that set it: the menu is not drawn for ~18 ticks and the reader's first A lands
/// on ~26, so this has to sit between them with room either side. One second of game time.
const MENU_HANDOVER_TICKS: u16 = 50;

/// How long a driver that runs its own menus ([`drives_its_own_menus`]) may go without the agent
/// reaching a decision point before it is abandoned and the policy asked again. See the ⚠️ at the
/// call site for why this is one rule rather than nineteen tick budgets.
///
/// 60 s matches the walk's bound and leaves the slowest legitimate driver — a multi-item mart trip, a
/// PC deposit, a Fly — far inside it; the leg chain and `full_playthrough` are what say so. It is a
/// *net*, not a schedule: anything that needs it has already failed to notice something.
const DRIVER_ESCAPE_SILENCE: Duration = Duration::from_secs(60);

/// Consecutive overworld ticks a finished Surf mount waits for before handing the walk back.
///
/// The same 300 ms `TeachingMove`'s `settle` uses and for the same reason: "the overworld is back" is
/// briefly true between the party menu closing and the mount's own text box being drawn, and a walk
/// resumed into that gap is aborted by the box a tick later. Any tick that is not the overworld resets
/// it, so this is a bound on the *flicker*, not on how long the box stays up.
const MOUNT_SETTLE_TICKS: u8 = 15;

/// How long a stopped walk waits to see whether the game walks the player back off the square.
///
/// The push-back is not immediate and does not follow the text box directly: the box is read, the
/// policy is asked (from the square the player has not been moved off yet), and only then does
/// `StartSimulatingJoypadStates` run. Measured on Cinnabar's gym doorstep that is a little over a
/// second, so this is three — long enough to span the poll, short enough that an unrelated walk later
/// in the same map cannot be mistaken for the game's own shove.
const TURN_BACK_WATCH_TICKS: u16 = 150;

/// A walk stopped by a text box on `tile`, and the square it stepped there from.
///
/// `came_from_raw` is raw (`wXCoord`/`wYCoord`) so the per-tick check is two RAM reads rather than a
/// map decode; `tile` is expanded, because that is what `observe_state` overlays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TurnBackWatch {
    map: Map,
    tile: Point8,
    came_from_raw: Point8,
    ticks: u16,
}

/// Overworld ticks between a pickup's text box closing and asking the map whether the item is still
/// there. See [`PokemonAgent::pending_pickup`] — 10 ticks is 200 ms of game time, far longer than
/// the `HideObject` at the end of the script needs and far shorter than a decision.
const PICKUP_SETTLE_TICKS: u16 = 10;

/// How long [`PokemonAgent::blackout_ticks`] will hold an overworld decision back while
/// [`blackout_in_flight`] says the black-out warp has not landed — 1500 ticks, 30 s of game time.
///
/// ⚠️ **A ceiling on a wait that ends by itself, not a tuned duration.** The measured window is a
/// fade, a money halving, a party heal and a map load — a couple of seconds — and the byte it waits
/// on is cleared by the cartridge with no input from anyone. Thirty seconds is far past anything
/// `HandleBlackOut` can take and still well inside `GB_STUCK_TIMEOUT_SECS` (300), so if this ever
/// does trip, the run asks the question late rather than not at all and the watchdog stays quiet.
const MAX_BLACKOUT_WAIT_TICKS: u16 = 1500;

/// What [`BattleState::Navigating`] lined up, carried into the confirm so the press that follows is
/// issued **once** and then checked against the game's own record of what it selected.
///
/// ⚠️ **This is the wrong-move bug, and it was a press too many rather than a cursor read gone
/// wrong.** The confirm arm used to `toggle_button(A)` on every tick of the
/// [`CONFIRMING_TICKS`] window — fifteen presses — and presses still in flight from the *previous*
/// turn's confirm landed in a list that had only just opened, so the move was chosen by a press
/// belonging to a different menu. Traced on a captured mainline state: a leftover `A` on
/// `MoveList{1}`, two more opening the list, then fifteen in the window, with `BUBBLE` committed
/// long before the cursor reached slot 1 and was correctly observed there (`wCurrentMenuItem = 2`).
///
/// ⚠️ **It is usually invisible, which is why it survived.** The list opens on the move used last,
/// so asking for the same slot twice running has the stray press confirm the right move by luck. It
/// bites only when the chosen slot differs from the previous one: **18 of 781 battle turns** across
/// `full_playthrough`, gym fights among them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Confirm {
    /// The action being confirmed, so a cursor that turns out not to be where `Navigating` believed
    /// can be handed straight back to it rather than wedging.
    action: BattleAction,
    /// The move slot the policy chose.
    slot: u8,
    /// Whether the single confirming press has been issued.
    pressed: bool,
    /// Ticks since it was issued, so a press the game never acted on can be retried rather than
    /// hanging. See [`CONFIRM_ACK_TICKS`].
    waited: u16,
    /// `Navigating`'s elapsed tick budget, carried across so a confirm that hands back cannot reset
    /// it. ⚠️ **Without this the two states ping-pong for ever**: the recovery below rebuilds
    /// `Navigating` with `ticks: 0`, so `MAX_NAVIGATING_TICKS` never accumulates and the battle
    /// never hands back to the policy.
    navigating_ticks: u16,
    /// How many presses have been issued. ⚠️ **A press can legitimately need repeating** — the list
    /// is drawn before it takes input, so the first one can land in that gap and do nothing — but a
    /// press is only ever repeated *after* `wPlayerMoveListIndex` has been checked and found
    /// unchanged, which is what makes repeating safe here and unsafe in the old per-tick loop.
    attempts: u8,
}

/// How long the confirm waits for the game to act on its one press before trying again.
///
/// ⚠️ **The agent observes 20 ms *after* its own press** — `gb.run(AGENT_RESOLUTION)` runs before
/// `agent.step` — so "press until it looks right" over-presses by construction. The wait is what
/// turns a press into an edge. It is a ceiling, not a delay: it ends the moment
/// `wPlayerMoveListIndex` is written or the list closes, which is the next tick or two.
///
/// ⚠️ **Scoped to this one press.** [`AGENT_RESOLUTION`] is unchanged and the overworld pathfinding
/// tuned against it is untouched; what changes is only whether a press is *repeated*.
const CONFIRM_ACK_TICKS: u16 = 3;

/// How many times the confirming press is repeated before giving up and backing out to the main
/// menu. Each attempt is separated by [`CONFIRM_ACK_TICKS`] and by a fresh reading of
/// `wPlayerMoveListIndex`, so this is bounded work with a check between every press rather than the
/// per-tick mashing it replaces.
const CONFIRM_ATTEMPTS: u8 = 12;

#[derive(Debug, Clone, Eq, PartialEq)]
enum BattleState {
    /// Waiting for the battle menu (TextBoxID 0x0B/0x1B) to appear.
    WaitingForMenu {
        reader: PokemonTextReader,
        delay: DelayContext,
        backing_out: u16,
        confirming: u16,
        /// Set only by [`Self::confirming`], i.e. only when `Navigating` lined a move list up. Every
        /// other way into this state leaves it `None` and confirms exactly as it always did.
        confirm: Option<Confirm>,
    },

    /// Battle menu is up but policy hasn't returned an action yet.
    AwaitingPolicy { delay: DelayContext },

    /// Navigating the menus. `ticks` bounds how long that may take — see the ⚠️ in the tick, which is
    /// the difference between "this decision is taking a while" and "the policy is never asked again".
    Navigating {
        action: BattleAction,
        delay: DelayContext,
        ticks: u16,
        /// Consecutive ticks the target menu has been observed. See the ⚠️ at the hand-off.
        stable: u16,
    },

    /// Dedicated driver for using **any** bag item in battle. The generic navigator can't handle the
    /// bag list — it walks menu *geometry*, and a bag list is a scrolling list of the player's own
    /// items — so this walks the whole flow with clean press/release rising edges: battle grid →
    /// ITEM → bag list → the item (found by [`PokemonApiTrait::bag_item_position`], not by cursor
    /// luck) → the "use on which POKéMON?" party menu where there is one → confirm. `start_qty` is
    /// the item's count on entry (completion = the count dropped). `confirmed` marks that the active
    /// mon has been selected, so we don't re-press A during the heal-animation lag and burn the whole
    /// stack. `press` gives the two-tick press/release cadence.
    ///
    /// ⚠️ **Every `UseItem` comes here, not just the ones that work.** It used to take a hand-written
    /// list of item ids and let everything else fall through to `Navigating`, which hands the open bag
    /// list to `WaitingForMenu`, which pressed A on **whatever the cursor happened to be sitting on**.
    /// On a bag holding a TM or a key item that is "OAK: <PLAYER>! This isn't the time to use that!",
    /// which drops straight back to the bag list with the cursor untouched — the PC menus' closed loop
    /// in a third costume — and reachable from any bag holding a TM or a key item, which is to say
    /// nearly all of them. Selecting by id also means a refusal is the *chosen* item's refusal, so the
    /// policy learns something from it.
    UsingItem { item: crate::pokemon::item::ItemId, start_qty: u8, entry_hp: u16, press: bool, confirmed: bool, delay: DelayContext, ticks: u16 },
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
    ///
    /// ⚠️ **This is what a give-up path owes the next state.** Every driver here can stop early — a
    /// navigation that got nowhere, an item the game refused, a tick budget spent — and each used to
    /// hand back a plain [`Self::default`]. But `WaitingForMenu` starts by *pressing A*, and a bag or
    /// party list is still on screen at that moment, so "give up" meant "select whatever is under the
    /// cursor" and the loop began again one item along. Handing back latched means the same B-mash
    /// that gets out of a refused move gets out of a refused item.
    fn backing_out() -> Self {
        Self::WaitingForMenu {
            reader: PokemonTextReader::message_box_only(),
            delay: DelayContext::default(),
            backing_out: BACKING_OUT_TICKS,
            confirming: 0,
            confirm: None,
        }
    }

    /// [`Self::default`], but told that a sub-menu was *just* opened on purpose and is waiting to be
    /// confirmed.
    ///
    /// ⚠️ **This exists because the screen the agent reads lags the game's own tilemap by up to three
    /// frames, and the two can disagree about which menu is up.** `AutoBgMapTransfer`
    /// (`home/vcopy.asm`) copies `wTileMap` into VRAM **one third of the screen per V-blank**, so for
    /// a frame or two after `MoveSelectionMenu` draws the move list the bottom third of VRAM still
    /// holds `FIGHT`/`PKMN`/`ITEM`/`RUN` — while `wTopMenuItemX/Y` already say (5, 12), the move
    /// list. The text test in [`Self::WaitingForMenu`] reads that stale third, concludes the main
    /// battle menu is back and hands the turn to the policy — **abandoning the move the policy chose
    /// one tick earlier and asking it the same question again**. On the deployed run that was a
    /// second paid LLM turn per battle turn, 1.1 s after the first, with the move menu open on
    /// screen; the model quite reasonably answered `wait`.
    ///
    /// So for a short window after `Navigating` hands over, RAM wins: the geometry is what the game
    /// wrote for the menu it is about to take input on, and the screen is a stale copy of the one
    /// before it. ⚠️ **Bounded rather than latched to the confirm**, because the arms below can
    /// legitimately fail to press anything (a refusal, a message box) — after the window the
    /// ordinary text test resumes and a genuinely returned main menu is handed to the policy as
    /// before, one window late.
    fn confirming(action: BattleAction, navigating_ticks: u16) -> Self {
        Self::WaitingForMenu {
            reader: PokemonTextReader::message_box_only(),
            delay: DelayContext::default(),
            backing_out: 0,
            confirming: CONFIRMING_TICKS,
            // ⚠️ Only a move list is confirmed by slot. Everything else keeps the old behaviour,
            // including the item and party sub-menus, which have their own drivers and their own
            // ⚠️s about what a leaked list means.
            confirm: match action {
                BattleAction::Fight { slot, .. } => Some(Confirm { action, slot, pressed: false, waited: 0, attempts: 0, navigating_ticks }),
                _ => None,
            },
        }
    }
}

/// The game's own refusals inside a battle, as they read once fully rendered.
///
/// Every one drops back to the menu that offered the choice with the cursor untouched, so every one is
/// a closed loop under A. ⚠️ **A refusal is not a reliable *detector*** — the box renders a character
/// at a time, so most ticks of one see a prefix that matches nothing — which is why the caller latches
/// [`BACKING_OUT_TICKS`] rather than pressing B once.
///
/// ⚠️ **Dismissing one is not optional even when the driver "knows" what to do next**, which is the
/// trap the last entry cost. A message box over a menu swallows *directional* input, so the
/// forced-switch arm below — which correctly wants to walk the cursor off a fainted Pokémon — pressed
/// Up into a message box that only A or B can clear, for ever. `soak` hit that from four different
/// seeds. Clearing the box first is what makes the cursor arm's presses land.
const BATTLE_REFUSALS: &[&str] = &[
    "isn't the time",     // a key item or a TM in battle: "OAK: <PLAYER>! This isn't the time…"
    "won by using",       // "You can't win by using that!"
    "isn't yours",        // someone else's item
    "any effect",         // "It won't have any effect."
    "blocked the BALL",   // a ball thrown at a trainer's Pokémon
    "no will to fight",   // a fainted Pokémon chosen from the party menu
];

/// Whether the box at the bottom of a battle screen is the game **talking**, rather than a menu the
/// agent is driving.
///
/// ⚠️ **`wTopMenuItemX/Y` linger and `wTextBoxID` does not, and that difference is the whole of
/// this.** Whoever is about to call `HandleMenuInput` writes the geometry, and nobody clears it
/// afterwards — so for the entire time a turn is resolving, `battle_menu_state()` still reports the
/// move list the policy chose from several seconds earlier. Measured over six Mt Moon battles:
/// **6155 ticks** of turn resolution reporting `Some(MoveList)`, every one of them with
/// `wTextBoxID` reading `MessageBox` and the sentence on screen; against **77 ticks** of a
/// genuinely open menu, every one reading `BattleMenuTemplate`. The two never overlapped.
///
/// ⚠️ **What it cost was every word the game says in a battle.** The arms that see lingering
/// geometry pressed A and deliberately did not read ("battle menu is showing, do not read the
/// text"), so `PokemonTextReader` was fed on no tick of any turn: across 11 battle turns not one
/// `AgentEvent::TextBox` was emitted. "It's super effective!", "ENEMY ZUBAT used LEECH LIFE!",
/// "CHARMANDER fainted!" and "gained 56 EXP" reach the model through that event or not at all — so
/// the model had **no way to see what the other Pokémon did**, and a battle report was a list of
/// its own move names.
///
/// ⚠️ **Reading here is timing-neutral, and that is why it is safe.**
/// [`PokemonTextReader::update`] presses A itself before it reads, so swapping
/// `api.toggle_button(JoypadButton::A)` for `reader.update(api)` is the *same single press* on the
/// same tick. That matters more here than almost anywhere: `with_original_battle_timing`'s ⚠️
/// records that re-rolling the battle RNG stream broke `can_reach_lavender`, and handing extra
/// menu states to the policy once blacked `full_playthrough` out in Mt Moon.
///
/// ⚠️ **And it waits out [`BattleState::confirming`], which is not belt-and-braces.** `wTextBoxID`
/// flips to `MessageBox` the moment the move is confirmed, but `AutoBgMapTransfer` copies the
/// tilemap into VRAM a third of the screen per V-blank — so for a few ticks the move list the player
/// just chose from is *still drawn* under a box that RAM already calls dialogue, and the reader
/// swallows it. It shows up as every quoted line in a report opening
/// `"TACKLE TAIL WHIP BUBBLE WATER GUN Celina used TACKLE! …"`. `confirming` is the existing
/// countdown for exactly this window, which is why it is reused rather than a second one invented.
///
/// ⚠️ **It must not be used to decide a *refusal*.** "No PP left" and the Disable message are
/// `MessageBox` too, and their handling latches `backing_out` — so this is checked **after** those,
/// never instead of them.
fn reading_dialogue(menu_state: &crate::pokemon::menu::MenuState, confirming: u16) -> bool {
    confirming == 0 && menu_state.text_box_id == crate::pokemon::menu::TextBoxId::MessageBox
}

/// Whether the screen is showing one of [`BATTLE_REFUSALS`].
///
/// ⚠️ **Not battle-only, despite the list's name.** `UsingFieldItem` asks the same question of the
/// overworld bag, because "OAK: <PLAYER>! This isn't the time to use that!" is the *same sentence*
/// from the same `ItemUseNotTime` and drops back to the same kind of list. The two battle-only
/// entries cannot appear outside a fight, so one list serves both and there is no second one to
/// drift. See the ⚠️ on [`BATTLE_REFUSALS`] about it being a poor detector on any single tick.
fn shows_battle_refusal(screen: &str) -> bool {
    BATTLE_REFUSALS.iter().any(|r| screen.contains(r))
}

/// States that press their own buttons through their own menus, and so must not have the generic
/// text reader pressing A underneath them.
///
/// One list, two readers: the text-box handover skips these, and the give-up net bounds them. They
/// were the same list written twice until the net was added, which is the kind of pair that drifts.
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
enum PokemartState {
    /// The Buy/Sell/Quit menu is up and the policy has not said what to buy yet.
    ///
    /// ⚠️ **Without this the shop is handed to the generic text reader, which mashes A through
    /// it.** [`drives_its_own_menus`] is what keeps [`PokemonAgent::assert_text_box_state`] off a
    /// driver's menus, and it keys on the *state* — so a mart that is not yet in a
    /// `PokemartShopping` state is not covered. `assert_pokemart_state` used to enter one only when
    /// the policy **answered**, which every scripted policy does on the first poll (the trait
    /// default is `Some(None)`) and `LlmPolicy` does not do for the whole time the model is
    /// thinking. In that window the reader owned the shop and pressed A: BUY, the first item in the
    /// stock list, quantity 1, confirm, YES, and round again.
    ///
    /// ⚠️ **It buys.** Reproduced from `viridian-city-pokemart-shopping.bin` with an LLM's latency:
    /// on ¥137 it merely loops on "You don't have enough money.", which is what the deployed run of
    /// 2026-08-27 was doing and why this looked like a stuck menu — but seeded with ¥1200 the same
    /// run ends `money now 0, bag [(TownMap, 1), (Potion, 1), (PokeBall, 6)]`. Six Poké Balls
    /// nobody ordered and an empty wallet. The fifth closed-loop-under-A in this codebase and the
    /// first one that spends money.
    ///
    /// ⚠️ **The poll happens here rather than at the transition into it**, so one seam asks and one
    /// seam answers; and it is [`PokemonAgent::poll_policy`], not `service_tools` directly, or the
    /// watchdog believes the run has been wedged since the shop opened.
    AwaitingPolicy,
    /// Buy/Sell/Quit menu visible. Keep pressing A on "Buy" (position 0) until item list appears.
    ChoosingBuyOption(BagItem),
    /// Item list visible. Navigate to the target item.
    ChoosingItem(BagItem),
    /// A pressed on the target item; waiting for wMaxItemQuantity==99 (qty selector opening).
    /// Buttons are released here to avoid the qty selector auto-confirming a spurious A press.
    AwaitingQtySelector(BagItem),
    /// Quantity selector active (wItemQuantity > 0). Adjust then press A.
    /// `qty_last` / `stall_ticks` track consecutive ticks where wItemQuantity didn't change,
    /// detecting the post-confirm price-text box that appears before the Yes/No prompt.
    ChoosingQuantity { item: BagItem, qty_last: u8, stall_ticks: u32 },
    /// Yes/No confirmation visible. Press A on "Yes" if wItemQuantity matches the target,
    /// otherwise press B to cancel and retry from the item list.
    ConfirmingPurchase(BagItem),
    /// YES was selected — purchase is being processed.  Pulse B (via `ticks` flip-flop) to
    /// advance post-purchase text and close the item list that buyMenuLoop re-opens.
    PurchasedItem { ticks: u32 },
    /// All items bought. Navigate cursor to "Quit" and press A.
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
    /// A map script or NPC scripted walk is running.  The player is frozen; the agent
    /// toggles A each tick to advance the script and any subsequent dialogue.
    /// To avoid triggering this on ledge jumps, if the delay is not triggerred before the game
    /// state changes, the agent restores to its previous state instead.
    RunningScript { rollback_deadline: DelayContext },
    /// The Pokémon nickname entry screen is active.
    /// `decided` is false while waiting for the policy; once true the name has been
    /// written to the naming buffer and START is toggled each tick until the screen exits.
    /// ⚠️ `ticks` is the bound, and it is not bookkeeping. Once `decided`, the exit test is "the game
    /// mode has left the naming/battle family" — which never becomes true if the naming screen was
    /// never really up. `GameMode::NamingScreen` is inferred, and workstream **J** (battle animations
    /// off) shifted the frame timing enough for it to fire spuriously **mid-battle**: the agent wrote
    /// a nickname for the mon that was already out, then pulsed A at a battle that was waiting for a
    /// move, for the rest of the run. Two previously-green legs died that way and neither log said
    /// anything except `name:Venusaur`. §10's rule applies to this wait like any other.
    NamingPokemon { species: PokemonSpecies, decided: bool, ticks: u16 },

    /// Player alternates between two adjacent tiles until a wild battle triggers — grass above ground,
    /// plain cave floor underground (see `adjacent_pacing_pair`). Either way it is the per-step encounter
    /// check that does the work, so any pair of walkable tiles will do.
    ///
    /// ⚠️ `stalled` is not optional bookkeeping. The pair is chosen from the meta-tile grid the
    /// *instant* the walk ends, and on map entry that grid is briefly unsettled — a trainer standing
    /// on grass can still read as plain grass, so the pacer picks its tile and then walks into the
    /// sprite forever. Bumping is not a step, so the ROM never rolls for an encounter: no error, no
    /// state change, no log line, just a run that quietly does nothing until its budget expires. The
    /// counter turns that into a re-pick, by which time the sprites have settled.
    /// Walking back and forth between two tiles so the ROM rolls for a wild encounter on each step.
    /// `stalled` counts ticks spent failing to reach the target (walking into something); `paced`
    /// counts every tick, successful or not, and is what bounds the wait for an encounter that is
    /// never coming. See the two constants in the tick for why one cannot do both jobs.
    PacingForEncounters { map: Map, tile_a: Point8, tile_b: Point8, heading_to_b: bool, stalled: u16, paced: u16 },

    Battle(BattleState),

    /// Navigating the Pokémart buy flow.
    PokemartShopping(PokemartState),

    /// Teaching an HM/TM (a legitimately-obtained bag item) to a party member, via the real menus:
    /// START→ITEM→(bag, navigate to the HM)→USE→(party, choose the mon). The move-replace menu (if the
    /// mon knows 4 moves) is handled by the shared global forget-move handler. Uses the same plain
    /// press/release "mashing" + cursor-navigate pattern as `CuttingTree`. `entered_menu` tracks that
    /// we left the overworld (so a return to it ends the attempt).
    /// `evolve_from` set means the bag item is an evolution stone rather than an HM: the same menu
    /// chain (START→ITEM→bag→USE→party→pick) runs, but completion is "the slot's species changed away
    /// from `evolve_from`" (e.g. Eevee → Vaporeon) instead of "learned the move".
    TeachingMove { item: crate::pokemon::item::ItemId, target_slot: u8, press: bool, entered_menu: bool, settle: u8, evolve_from: Option<crate::pokemon::species::PokemonSpecies> },

    /// Using the Cut field move on the tree the player is facing: driving START→POKéMON→mon→CUT (the
    /// game's `UsedCut` then removes the tree). `press` alternates each tick — plain press/release
    /// "mashing", navigating each menu's cursor to its target index and then confirming with A (never
    /// carrying a held direction into the next menu). `entered_menu` tracks that we left the overworld
    /// (so returning to it means the cut animation finished). `tree_pos` is the tile being cut.
    CuttingTree { press: bool, entered_menu: bool, tree_pos: Point8, slot: u8, move_index: u8 },

    /// Mounting Surf to cross water: the route is about to step onto a `Water` tile while the player is
    /// on foot, so drive START→POKéMON→(surf mon at `slot`)→SURF (the game then mounts the player and
    /// auto-steps onto `water_pos`). Same press/release mashing + cursor-navigate pattern as
    /// `CuttingTree`; `entered_menu` tracks that we left the overworld, so a return to it with the surf
    /// state active means the mount finished. `water_pos` is the tile being surfed onto (used to face it).
    ///
    /// ⚠️ **`resume` is what stops a mount costing a paid turn, and it is the one field here that is
    /// not about menus.** Every other driver in this file is entered from `Idle` because the *policy*
    /// asked for it; this one is entered from the middle of a walk, by the route follower, without
    /// anybody deciding anything. Dropping to `Idle` when the mount finished therefore threw the walk
    /// away and put the identical question back to the policy — free for the scripted one, a whole
    /// request against a 100 k-token history for the model. The deployed run of 2026-09-03 crossed
    /// Route 21 that way, and paid for it again on Cinnabar Island getting to the Pokémon Mansion.
    /// So the interrupted `OverworldMovement` rides through the mount and is put back on success.
    /// `settle` counts consecutive overworld ticks since the mount took — see [`MOUNT_SETTLE_TICKS`].
    Surfing { press: bool, entered_menu: bool, water_pos: Point8, slot: u8, move_index: u8,
              resume: Option<(MetaTile, Map)>, settle: u8 },

    /// Using a party field move that has no target tile: drive START→POKéMON→(the mon at `slot`)→the
    /// field-move entry at `move_index`. STRENGTH arms `BIT_STRENGTH_ACTIVE` so boulders become
    /// pushable; DIG warps the player out of the cave. Same press/release mashing as
    /// `CuttingTree`/`Surfing`; `entered_menu` tracks that we left the overworld, so a return to it —
    /// or a move away from `from_map`, which is all DIG does — means the move finished.
    UsingFieldMove { press: bool, entered_menu: bool, slot: u8, move_index: u8, from_map: Map },

    /// Tossing a bag item to make room: START→ITEM→(bag list)→TOSS→quantity→YES. Gen 1's bag holds 20
    /// items and a mart purchase of a *new* item silently fails once it is full ("You can't carry any
    /// more items"), so a leg that must buy something has to free a slot first. Same press/release
    /// mashing as `TeachingMove`; the step is done when the item has left the bag.
    TossingItem { item: crate::pokemon::item::ItemId, press: bool, entered_menu: bool },
    /// Depositing an item into, or withdrawing it from, PC item storage. Phase 0 tasks
    /// 0.5/0.6 — the whole state machine lives in [`crate::pokemon::postgame::item_storage`].
    UsingItemPc(crate::pokemon::postgame::item_storage::ItemPcState),
    /// **Workstream A** — driving Bill's PC box menus (deposit / withdraw / release / change box). The
    /// whole state machine lives in [`crate::pokemon::postgame::pc_box`].
    UsingPcBox(crate::pokemon::postgame::pc_box::PcBoxState),

    // ── Postgame driver states (`docs/postgame-coverage-plan.md`) ────────────────────────────────
    // One variant per workstream driver; each arm delegates in one line to `postgame::<stream>::tick`
    // so this file stays mergeable while several streams are open (§4.1 of the plan).
    /// **Workstream B** — driving the Fly menu chain and the bespoke town-map screen. The whole state
    /// machine lives in [`crate::pokemon::postgame::fly_bike`].
    Flying(crate::pokemon::postgame::fly_bike::FlyState),
    /// **Workstream C** — one rod cast: the walk to the shore, the bag menu chain, and the wait on
    /// "not even a nibble". A bite becomes a wild battle and `assert_battle_state` takes over. The
    /// state machine lives in [`crate::pokemon::postgame::fishing`].
    Fishing(crate::pokemon::postgame::fishing::FishState),
    /// **Workstream F** — selling a bag item to a mart clerk. Owns the whole conversation rather than
    /// letting [`Self::PokemartShopping`] have it, because the Buy/Sell/Quit menu is shared and that
    /// state machine only knows how to buy. State machine in
    /// [`crate::pokemon::postgame::game_corner`].
    SellingToMart(crate::pokemon::postgame::game_corner::SellState),
    /// **Workstream G** — a one-NPC script that opens the party menu on a stale cursor (the Day Care,
    /// the Name Rater): the walk to the NPC and the menu. State machine in
    /// [`crate::pokemon::postgame::gifts`].
    UsingPartyScript(crate::pokemon::postgame::gifts::PartyScriptState),
    /// **Workstream F** — buying a Game Corner prize: the walk to a vendor bg-event and the bespoke
    /// prize menu. State machine in [`crate::pokemon::postgame::game_corner`].
    RedeemingPrize(crate::pokemon::postgame::game_corner::PrizeState),
    /// **Workstream I** — using a bag item from the overworld: the START → ITEM → bag → USE chain
    /// plus whichever menus that item opens afterwards (a party list, a move list, or neither). The
    /// state machine lives in [`crate::pokemon::postgame::items`].
    UsingBagItem(crate::pokemon::postgame::items::BagItemState),
    // (**H** reserved a `SearchingHiddenItem` state here. It is gone: collecting a hidden item is
    // `CheckingTrashCan` below, because the ROM dispatches both from `CheckForHiddenObject`.)
    // ─────────────────────────────────────────────────────────────────────────────────────────────

    /// Checking a Vermilion Gym trash can for a hidden switch: route to a tile adjacent to `target`
    /// and face it (recomputed each tick via `MetaTileMap::route_to_face`), then press A to trigger
    /// `GymTrashScript`. `press` alternates for plain press/release mashing. `checked` becomes true
    /// once the check text/menu has appeared, so returning to the overworld means the can was checked.
    CheckingTrashCan { target: Point8, checked: bool, press: bool, facing: Option<crate::pokemon::map_metadata::PlayerFacingDirection> },

    /// Using an elevator floor panel: face the panel + A to open the floor list-menu, navigate the
    /// cursor (`wCurrentMenuItem`) to `floor` + A to pick it, then ride the (runtime-redirected)
    /// elevator warp out. `selected` flips once the floor is confirmed; `press` alternates for plain
    /// press/release mashing. Finishes when the map changes (we've left the elevator).
    UsingElevator { panel: Point8, floor: u8, selected: bool, press: bool },

    /// Using a bag item on the field: route to face the sprite at `target`, then drive
    /// START→ITEM→(bag, navigate to the item)→USE. The item's field effect takes over (e.g. the Poké
    /// Flute wakes a Snorlax → a battle, which the battle handler wins). `press` alternates for plain
    /// press/release mashing; `entered_menu` tracks that we left the overworld into the bag menus.
    /// Uses a bag item on a tile: walk to face `target`, open START → ITEM → the item → USE.
    ///
    /// ⚠️ **`backing_out` is the refusal latch, and it is a countdown for
    /// [`BACKING_OUT_TICKS`]' reason.** A refused item drops back to the bag list with the cursor
    /// untouched, and one B dismisses the message onto that list, where this driver's own
    /// navigation would press A on the same row again. So the intent is latched and held, exactly
    /// as `BattleState::WaitingForMenu` holds it for a refused move.
    UsingFieldItem { item: crate::pokemon::item::ItemId, target: Point8, press: bool, entered_menu: bool, backing_out: u16 },

    /// Executing ONE Strength boulder push (the primitive behind `FieldMove::PushBoulder`): route the
    /// player to the tile behind the boulder at `boulder` (via `route_to`), face it, and hold `dir` — the
    /// game's double-press logic advances the boulder one tile (the dust animation locks input, so it
    /// never over-pushes). Done as soon as the boulder leaves `boulder` (moved one tile, or fell through a
    /// hole). Any interruption (wild battle, script) drops to Idle; the policy re-decides the next push.
    /// Planning which boulder to push where lives in the policy (`MetaTileMap::solve_boulder_push`), not here.
    PushingBoulder { boulder: Point8, dir: JoypadButton },
}

impl AgentState {
    pub fn battle_state_mut(&mut self) -> Result<&mut BattleState, String> {
        if let AgentState::Battle(s) = self {
            Ok(s)
        } else {
            Err("Not in battle".to_string())
        }
    }
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
            AgentState::PushingBoulder { boulder, dir } => write!(f, "push-boulder:{boulder}{dir:?}"),
        }
    }
}

pub struct PokemonAgent {
    state: AgentState,
    /// Tick counter for the post-Champion cutscene cadence — see `drive_post_champion_cutscene`.
    post_champion_tick: u32,
    backup_state: Option<AgentState>,
    event_buffer: VecDeque<AgentEvent>,
    cycles: MachineCycles,
    policy: Box<dyn Policy>,
    /// Map graph built **incrementally** as the player traverses. Each time the agent lands on a
    /// new map it records that section's live, sprite-resolved reachable warps/connections
    /// (`WorldGraph::observe`). Provided to the policy each overworld decision for backtracking
    /// (e.g. heal-return); forward travel is scripted with explicit `EnterMap` steps.
    world_graph: WorldGraph,
    /// The map the agent was last on, to detect map changes (warp/connection landings).
    last_map: Option<Map>,
    /// Trees the agent has cut down, by `(map, expanded tile position)`. The `MetaTileMap` is decoded
    /// from the static ROM map, so it always shows a `CutTree` there even after the game removed it;
    /// the agent remembers what it cut and treats those tiles as `Empty` for routing/movement.
    cut_tiles: std::collections::HashSet<(Map, Point8)>,
    /// Silph Co door-graphic *walls* ($18/$24 that won't open), by `(map, tile position)`. Card-key
    /// door graphics are placed into the map at *runtime* (a load script `ReplaceTileBlock`s them over
    /// floor tiles), so they're absent from the static ROM the `MetaTileMap` is decoded from — the
    /// agent would route straight into them and bump forever. When the agent faces one we first try to
    /// *open* it with the Card Key (press A); if it won't open after several tries it's a decorative
    /// wall, so we remember the faced tile and `observe_state` overlays it as an `Obstacle` for routing.
    blocked_tiles: std::collections::HashSet<(Map, Point8)>,
    /// Consecutive A-presses spent trying to open the card-key door currently in front of the player
    /// (reset when the player no longer faces a door). Once it exceeds the open threshold the tile is
    /// treated as an unopenable wall (added to `blocked_tiles`).
    door_open_attempts: u32,
    /// Squares **this visit to this map** has been turned away from — the third member of the
    /// `blocked_tiles` family, learned rather than known.
    ///
    /// ⚠️ **Gen 1 refuses a square by talking and then walking you off it, and no tile id says so.**
    /// A dozen map scripts do it (`grep StartSimulatingJoypadStates pokered/scripts`): Cinnabar's gym
    /// doorstep without the Secret Key, the Route 22 gate and Route 23's checkpoints without the
    /// badge, Viridian City, the Museum. The square is ordinary floor, the router walks onto it, the
    /// text box aborts the walk, the game pushes the player back, and the policy re-plans the
    /// identical route — for ever. [`PokemonAgent::turn_back_watch`] recognises it from the
    /// cartridge's own behaviour and `observe_state` overlays the result as an obstacle, exactly as
    /// it does for a card-key wall and a refused Surf.
    ///
    /// ⚠️ **Cleared on map re-entry, unlike `blocked_tiles`, and that is the whole safety argument.**
    /// What is being remembered is "the game turned me away *given what I am carrying*", and that
    /// expires: the Route 22 guard steps aside on the Boulder Badge, the gym door opens on the Secret
    /// Key. A permanent entry would wall the run out of somewhere it must later go, which is far
    /// worse than the loop this prevents — so the memory lasts one visit, and the cost of being
    /// wrong is one interrupted walk rather than an unwinnable game.
    turned_back_tiles: std::collections::HashSet<(Map, Point8)>,
    /// A walk stopped by a text box, waiting to see whether the game puts the player back where they
    /// came from. See [`PokemonAgent::turned_back_tiles`] and [`TURN_BACK_WATCH_TICKS`].
    turn_back_watch: Option<TurnBackWatch>,
    /// While a walk is in flight, the last two **raw** squares the player has stood on, as
    /// `(previous, current)`. Raw rather than expanded because the only reader is the turn-back watch,
    /// which compares against `wXCoord`/`wYCoord` — two RAM reads a tick rather than a map decode.
    /// Maintained by `AgentState::OverworldMovement` alone: a walk is the only thing a turn-back can
    /// interrupt, and it is the one arm that already holds a `GameState`.
    ///
    /// ⚠️ Reset by `take_overworld_action`, and the watch refuses to arm while the pair is still
    /// equal. Both are defensive rather than measured — no test fails without them — but the failure
    /// they prevent is a wall invented on a square the player never stepped from, which nothing
    /// downstream could tell from a real one.
    walk_squares: Option<(Point8, Point8)>,
    /// Raw button presses queued by `queue_manual_input`, delivered ahead of the state machine at
    /// [`MANUAL_INPUT_TICKS_PER_PRESS`] ticks each. Empty in every ordinary run — nothing in the agent
    /// or in any scripted policy ever enqueues.
    manual_input: VecDeque<JoypadButton>,
    /// Ticks left of the press currently being delivered — see [`MANUAL_INPUT_HOLD_TICKS`]. Zero
    /// means nothing is held and the next tick takes the head of the queue.
    manual_input_held: u8,
    /// Ticks spent in the current state, reset by [`Self::set_state`] — so a state machine that is
    /// *making progress* (any transition at all, including between battle sub-states) keeps it at
    /// zero, and only genuinely sitting still accumulates.
    ///
    /// ⚠️ **The states that need it are the ones that never poll the policy.** `soak` found four of
    /// them one at a time — `OverworldMovement`, `Navigating`, `UsingItem`, `PacingForEncounters`
    /// — and the shape was identical every time: a driver waiting for something that had already
    /// stopped coming, pressing buttons in silence until a deployed watchdog would have fired. Each
    /// bound below hands back to a state that re-reads the screen and asks the policy again, so the
    /// cost of being wrong is one re-decided turn.
    /// Set when the text reader decides it is looping inside menus rather than reading a conversation,
    /// cleared by [`Self::poll_policy`]. See the ⚠️ in the `ReadingTextBox` arm.
    escaping_menus: bool,

    /// Ticks remaining in which a newly-opened text box may still turn out to be a **menu the agent
    /// inherited** rather than a conversation it should confirm. Counted down in the `ReadingTextBox`
    /// arm, armed in [`Self::assert_text_box_state`], and consumed the moment
    /// [`open_menu_on_screen`] says yes.
    ///
    /// ⚠️ **A window rather than the single transition tick, and the length is measured rather than
    /// picked.** The obvious version tests once, where the box opens — and it answers no every time,
    /// because `wFontLoaded` flips **before the menu draws itself**. Measured on the START menu
    /// opened by `queue_manual_input`: the mode changes on tick 2, `wTopMenuItemX/Y` are still the
    /// *previous* menu's until tick 18, and `EXIT` does not reach the tile map until tick 21 — a
    /// third of a second in which there is nothing to recognise. The reader's first A lands on tick
    /// 26, so the window has to outlive the draw and beat the press.
    ///
    /// ⚠️ **Bounded, because "always" is a different and much worse rule.** A per-tick test has to be
    /// right about every screen of every conversation for the length of the run; this one only has to
    /// be right about the moment a box opens with no driver behind it. That is what makes it safe to
    /// act on immediately, where [`TEXT_BOX_ESCAPE_SILENCE`] has to wait 30 s for the same evidence.
    menu_handover_ticks: u16,

    // ── W9: the stuck-run watchdog ───────────────────────────────────────────────────────────────
    /// Emulated time since the policy was last polled, in cycles. Reset by [`Self::poll_policy`] and
    /// by nothing else — which is what makes it measure *decision points* rather than ticks.
    cycles_since_poll: MachineCycles,
    /// [`Policy::stuck_timeout`], read once when the agent was built. `None` — every scripted policy
    /// — makes the whole watchdog one comparison per tick.
    stuck_after: Option<MachineCycles>,
    /// `cycles_since_poll` as of the last [`AgentEvent::WatchdogFired`], so a jam is reported when it
    /// starts and once per timeout after that rather than fifty times a second.
    stuck_reported_at: MachineCycles,

    /// The slot the policy chose at the "Which move should be forgotten?" menu, held for as long as
    /// that menu is on screen.
    ///
    /// ⚠️ **Without it the policy is asked again on every tick of the menu, and for `LlmPolicy` an
    /// ask is a whole turn.** `drive_forget_menu` presses one button per tick — walk the cursor,
    /// then confirm — and re-polled before each one, so a menu that takes several ticks to drive
    /// spent several completions. The deployed run of 2026-09-01 was asked the same Growl-for-Rage
    /// question as **turns 232, 233 and 234**, answered `slot 1` all three times, and paid a full
    /// prefill of a ~30 k-token history for each. It is the same shape as `BattleState::Navigating`
    /// holding its `action`: a decision already taken is being carried out, not asked for.
    ///
    /// ⚠️ **Cleared when the menu goes**, not when the answer is used — the answer is needed on
    /// every tick until the menu closes, and a latch that outlived it would answer the *next*
    /// level-up with the last one's slot.
    forget_choice: Option<Option<usize>>,

    /// An item ball the agent has just walked up to and pressed A on, waiting for the overworld to
    /// come back so the map can be asked whether it is still there.
    ///
    /// ⚠️ **The answer is only readable once the game is back in the overworld.** The pickup runs as
    /// a script — text box, `GiveItem`, `HideObject` — so the sprite is still in the map for the
    /// whole of the conversation, and testing while the box is open reports every successful pickup
    /// as a failure. See [`AgentEvent::OverworldPickupFailed`].
    ///
    /// ⚠️ **With a settle counter, because "the overworld is back" is true a beat before the script
    /// has finished with it.** `HideObject` runs at the end of the pickup script and the mode can
    /// read `Overworld` in between, which would report every successful pickup as a failure — the
    /// worst possible way for this to be wrong, since it would put a false line under every item in
    /// the game.
    pending_pickup: Option<(MetaTile, u16)>,

    /// Overworld ticks spent so far waiting out a black-out warp — see [`blackout_in_flight`] for
    /// what is being waited for and [`MAX_BLACKOUT_WAIT_TICKS`] for the ceiling. Reset by
    /// [`Self::poll_policy`], i.e. by the agent reaching the decision point this defers.
    blackout_ticks: u16,

    // ── The end of the game ──────────────────────────────────────────────────────────────────────
    /// `wNumHoFTeams` as of the last tick, and `None` until the first one — see
    /// [`Self::check_hall_of_fame`], where the whole of the edge trigger lives.
    hall_of_fame_teams: Option<u8>,
}

impl Default for PokemonAgent {
    fn default() -> Self { Self::new(Box::new(RandomPolicy::default())) }
}

impl PokemonAgent {
    pub fn new(policy: Box<dyn Policy>) -> Self {
        // ⚠️ Asked once. A policy that changed its mind later would not be heard, which is documented
        // on the trait method — and a zero timeout is "off" rather than "fire on every tick".
        let stuck_after = policy
            .stuck_timeout()
            .filter(|timeout| !timeout.is_zero())
            .map(MachineCycles::from_duration);
        Self {
            cycles_since_poll: MachineCycles::ZERO,
            stuck_after,
            stuck_reported_at: MachineCycles::ZERO,
            state: AgentState::default(),
            post_champion_tick: 0,
            backup_state: None,
            event_buffer: VecDeque::new(),
            cycles: MachineCycles::default(),
            policy,
            world_graph: WorldGraph::new(),
            last_map: None,
            cut_tiles: std::collections::HashSet::new(),
            blocked_tiles: std::collections::HashSet::new(),
            turned_back_tiles: std::collections::HashSet::new(),
            turn_back_watch: None,
            walk_squares: None,
            door_open_attempts: 0,
            manual_input: VecDeque::new(),
            manual_input_held: 0,
            escaping_menus: false,
            menu_handover_ticks: 0,
            forget_choice: None,
            pending_pickup: None,
            blackout_ticks: 0,
            hall_of_fame_teams: None,
        }
    }

    /// Throw away everything learned about the game so far, keeping only the policy.
    ///
    /// For `POST /api/new-run`, which reloads the emulator from the start-of-game state under a live
    /// process. Every field here is an observation *about that game* and is now wrong: the world
    /// graph was built from maps the player has not visited yet, `cut_tiles` and `blocked_tiles`
    /// remember terrain edits that have not happened, and a `state` mid-`OverworldMovement` is
    /// walking a route on a map that is no longer loaded.
    ///
    /// ⚠️ **`policy` and `stuck_after` are deliberately kept.** The policy is the *decider*, not
    /// state about the game — rebuilding it would mean rebuilding `LlmPolicy`'s channels to a worker
    /// thread that is still perfectly alive. It is told separately, via [`Policy::restart`], which is
    /// where anything it holds *about the game* goes. `stuck_after` is read from the policy once by
    /// contract (see [`Self::new`]), so re-reading it here would quietly make that a lie.
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
        self.blocked_tiles.clear();
        self.turned_back_tiles.clear();
        self.turn_back_watch = None;
        self.walk_squares = None;
        self.door_open_attempts = 0;
        self.manual_input.clear();
        self.manual_input_held = 0;
        self.cycles_since_poll = MachineCycles::ZERO;
        self.stuck_reported_at = MachineCycles::ZERO;
        self.blackout_ticks = 0;
        // Back to `None`, not to `Some(0)`: the next tick re-seeds from whatever the freshly loaded
        // cartridge says. `POST /api/new-run` loads the start-of-game state, so that is 0 — but a
        // future caller that restarts from something else must not be told it has just won.
        self.hall_of_fame_teams = None;
    }

    /// **`POST /api/clear`** — pass the request straight through to the policy.
    ///
    /// ⚠️ **Nothing on the agent is touched, and that is the whole point.** [`Self::restart`] above
    /// clears every field because the game underneath them is gone; here it is the same game on the
    /// same tile, so the world graph, the cut tiles and a walk half-finished are all still true. The
    /// only thing that has changed is what the *model* remembers, and the model is the policy's.
    pub fn clear_conversation(&mut self, run_dir: Option<&std::path::Path>) -> Result<(), String> {
        self.policy.clear_conversation(run_dir)
    }

    /// Queue raw button presses, one per agent tick, pre-empting the state machine.
    ///
    /// The escape hatch for a policy that needs to press a button the agent has no action for.
    /// `OverworldAction` cannot express "press B once" — the agent re-derives the action mid-walk and
    /// matches it by `MetaTile`, so a synthetic action fails with `NoRoute` — and the need is broader
    /// than the overworld anyway: the policy is never consulted at all while the agent believes it is
    /// mid-script, which is exactly when a wedged run needs a nudge.
    ///
    /// The current state is cleared to `Idle` (and any backed-up state dropped) so the next ordinary
    /// tick re-derives from scratch. Without that, a press landing mid-`OverworldMovement` corrupts
    /// the route and the walk resumes into a wall.
    ///
    /// The queue is capped at [`MANUAL_INPUT_CAPACITY`] buttons; anything past the cap is dropped, so
    /// a confused model cannot run away with the game.
    pub fn queue_manual_input(&mut self, buttons: impl IntoIterator<Item = JoypadButton>) {
        for button in buttons {
            if self.manual_input.len() >= MANUAL_INPUT_CAPACITY { break; }
            self.manual_input.push_back(button);
        }
        self.backup_state = None;
        self.set_state(AgentState::Idle);
    }

    /// Manual button presses not yet fully delivered, including the one currently being held. Zero
    /// means the state machine is back in charge (see [`Self::queue_manual_input`]).
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

    /// True while a battle is being fought. The policy queue cannot advance during one — a step only
    /// completes between battles — so a stall detector that counts "queue unchanged" time has to
    /// discount this, or a long fight (the Route-22 rival's six mons, an Elite Four room) reads as a
    /// deadlock. Any battle blocks the queue by design, whatever step happens to be at its front.
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

    /// **The** policy poll — every one of the agent's decision points goes through here.
    ///
    /// It exists for its first line. `cycles_since_poll` is what W9's watchdog measures, and if it
    /// were reset at each `service_tools` call site instead, a sixth decision point added later
    /// would look like a jam forever. One seam, and the compiler finds anything that bypasses it.
    fn poll_policy(&mut self, game_state: &GameState, api: &mut PokemonApi) {
        self.cycles_since_poll = MachineCycles::ZERO;
        // Reaching a decision point is what "out of the menus" means — see the ⚠️ in `ReadingTextBox`.
        self.escaping_menus = false;
        self.stuck_reported_at = MachineCycles::ZERO;
        // Cleared here rather than where the wait ends, for the same reason `escaping_menus` is:
        // the counter measures a deferral, and a decision point is what ends one.
        self.blackout_ticks = 0;
        self.policy.service_tools(game_state, api, &self.world_graph);
    }

    /// What the decider calls itself — [`Policy::name`], which the host reports on every heartbeat
    /// and writes into a finished run's record.
    pub fn policy_name(&self) -> &'static str {
        self.policy.name()
    }

    /// What the policy would like the player called on a new game — [`Policy::player_name`]. `None`
    /// keeps whatever the starting state was captured with.
    pub fn policy_player_name(&self) -> Option<String> {
        self.policy.player_name()
    }

    /// Emulated time the agent has gone without reaching a decision point of any kind. Zero on any
    /// tick that polled the policy.
    pub fn since_last_policy_poll(&self) -> Duration {
        self.cycles_since_poll.to_duration()
    }

    /// **W9 / §14** — the watchdog: ask the policy for a nudge when nothing has asked it anything.
    ///
    /// Returns whether it fired, which is only of interest to the tests — the answer travels the
    /// manual-input queue and arrives at the top of the next tick.
    ///
    /// ⚠️ **This must not go through [`Self::poll_policy`].** Resetting the clock here would clear
    /// the jam the instant it was noticed, and a turn that takes seconds of wall clock would never
    /// be polled a second time — so it would never be serviced, never answered, and the run would
    /// sit there with one abandoned turn per timeout. The jam ends when the *agent* reaches a real
    /// decision point again, and not before.
    fn run_watchdog(&mut self, api: &mut PokemonApi) -> bool {
        let Some(after) = self.stuck_after.filter(|after| self.cycles_since_poll >= *after) else {
            return false;
        };
        // The naming screen's ⚠️ applies here too, and more so: a failed read must never turn the
        // watchdog itself into the thing that breaks the tick.
        let Ok(game_state) = api.game_state() else { return false };

        // §14: **every firing is a bug report.** Emitted before the policy is asked, so the turn this
        // poll may start carries the line in its "since your last decision"; `event` also prints it
        // to stdout and the host publishes it to the UI and the transcript. Once when the jam starts
        // and once per timeout after that — at fifty polls a second, anything else is a flood.
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
    ///
    /// `wNumHoFTeams` is incremented at the top of `AnimateHallOfFame`
    /// (`engine/movie/hall_of_fame.asm:27-32`) — the first frame of the ceremony, before the party
    /// parade, the credits, the game's own save and its `jp Init` back to the title screen. So the
    /// rising edge of that byte *is* the moment of victory, and it is the only signal that is one:
    ///
    /// - **All eight badges is not it.** That is Viridian Gym, a good hour early.
    /// - **`map == Map::HallOfFame` is not it either.** That is a three-minute cutscene which ends
    ///   in a soft reset, so it is an edge at best and a level never — and the map is already
    ///   `HallOfFame` for three script stages *before* the counter moves
    ///   (`scripts/HallOfFame.asm`: the walk-in, Oak's congratulation, then
    ///   `HallOfFameResetEventsAndSaveScript` → `predef HallOfFamePC` → `AnimateHallOfFame`).
    ///
    /// ⚠️ **The first tick only seeds the baseline.** Resuming a save that has already won — every
    /// postgame fixture, and the deployed run on the day after it finished — must not re-announce a
    /// victory that happened in another process. Seeding from what is in RAM rather than from zero is
    /// the whole of that, and it is why this is an `Option` and not a `u8`.
    ///
    /// ⚠️ **Read straight from the MMU, not from `game_state()`.** That call is fallible and answers
    /// `Err` through exactly the screen transitions this has to fire during. The `GameState` field of
    /// the same name exists for the model and the tests, not for this.
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

        // ⚠️ Read *now*, on the emulator thread, and carried on the event: the host formats events
        // off-thread and `run::hall_of_fame` archives later still, by which time the ceremony has
        // cleared the party out of WRAM's display buffers.
        let playtime = crate::pokemon::observe::playtime(api);
        let playtime_seconds = crate::pokemon::observe::playtime_seconds(api);
        let badges = api.mmu().read_pointer(&sym::wObtainedBadges);
        let party = api
            .mmu()
            .read_player_pokemon_party()
            .map(|party| party.iter().map(|mon| mon.nickname.to_default_string()).collect())
            .unwrap_or_default();
        self.event(AgentEvent::HallOfFame { teams, playtime, playtime_seconds, badges, party });
    }

    /// Emit an event: to the policy first, then to the buffer the host drains.
    ///
    /// Every event goes through here, including the ones collected into `update`'s local
    /// `new_events` and drained at the end of the tick, so `on_event` cannot silently miss a class
    /// of them.
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
    ///
    /// ⚠️ **Reading a text box and reporting it are two moments, and treating them as one lost the
    /// most important text boxes in the game.** The buffer used to be emitted in exactly one place —
    /// [`Self::assert_text_box_state`]'s "the box closed" arm, which fires on the
    /// `TextBox → anything else` edge *while the agent is still* `ReadingTextBox`. But
    /// `assert_script_state` runs **before** it in `update` and swaps the state out for
    /// `RunningScript`, so any box the game follows with a script never reached that arm at all: the
    /// reader was dropped mid-sentence and the model was told only that its walk had stopped.
    ///
    /// ⚠️ **That is not an edge case, it is the shape of every blocker in Pokémon Red.** A guard,
    /// a locked door or a "you can't go through here" prints its message and then calls
    /// `StartSimulatingJoypadStates` to shove the player back a tile — `Route22GateGuardText`,
    /// `ViridianCityCheckGymOpenScript`, the Viridian old man. Measured on the deployed run of
    /// 2026-08-26: a landed conversation was followed by a `TextBox` event **31 times out of 38**,
    /// and an aborted walk **2 times out of 28** — and both of those two were multi-box story
    /// scripts (Oak at the edge of Route 1, the rival in the lab) rather than a blocker. So the run
    /// walked into the Route 22 gate, was told "✗ gave up on the warp to Route23 at (4, 2)" with no
    /// text whatsoever, talked to the guard **five times** trying to find out why, got nothing back
    /// each time, and filed a `report_issue`. The words it could not see were "Only truly skilled
    /// trainers are allowed through. You don't have the BOULDERBADGE yet!"
    ///
    /// Called from the two places that assign `self.state`, so "leaving the reader reports what it
    /// read" is structural rather than a list of call sites to keep up to date.
    fn flush_text_reader(&mut self) {
        let AgentState::ReadingTextBox { reader } = &mut self.state else { return };
        let message = reader.take();
        // Empty ones are dropped by `event` — a box is detected before its characters are drawn, so
        // most of these have nothing in them.
        self.event(AgentEvent::TextBox { message });
    }

    fn backup_current_state(&mut self, new_state: AgentState) {
        // ⚠️ **Before the clone, not after.** The backup is what `restore_state_from_backup` puts
        // back when a script turns out to have been a ledge jump, and a reader restored with its
        // buffer still in it would report the same sentence a second time when the box finally
        // closed.
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
    ///
    /// [`MANUAL_INPUT_TICKS_PER_PRESS`] ticks per button: the press, `MANUAL_INPUT_HOLD_TICKS - 1`
    /// more ticks holding it, then a tick with everything released. Both counts are load-bearing and
    /// both are documented where they are defined. The state machine resumes on the tick after the
    /// queue empties, from the `Idle` state `queue_manual_input` left behind.
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

    /// Read the game state, overriding tiles the agent has cut down to `Empty` (the ROM-decoded map
    /// still shows them as `CutTree`). Use this wherever the map is used for routing/movement.
    /// True while the post-Champion cutscene is running, in which case this has driven this tick.
    ///
    /// Detection is the Champion's room with its map script at or past OAK_ARRIVES; the chain ends by
    /// warping to the Hall of Fame, at which point the map is no longer ChampionsRoom and the agent
    /// resumes normally.
    ///
    /// What matters is the **early return**, not the cadence: with this handler in place even the text
    /// reader's fast 20 ms toggle walks the chain to the Hall of Fame, and without it no cadence does.
    /// The reason is the stray `release_all_buttons` calls the agent's ordinary per-tick machinery
    /// makes as it transitions between states. `toggle_button` flips relative to the *current* joypad,
    /// so a release landing between two toggles turns the alternation into two presses in a row — and
    /// A held across a tick boundary is exactly what pokered's `HoldTextDisplayOpen` spins on, since it
    /// holds the dialogue box open for as long as A is down. The deliberate press below is simply the
    /// clearest thing to send once the agent is out of its own way.
    fn drive_post_champion_cutscene(&mut self, api: &mut PokemonApi) -> bool {
        use crate::pokemon::symbols::pokered_symbols as sym;
        // Wait for OAK_ARRIVES rather than RIVAL_DEFEATED: the stage before it is the rival's own
        // concession text, and the policy needs one ordinary tick there to notice the trainer is
        // beaten and pop its `BattleTrainer` step. Taking over at stage 3 strands that step forever.
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
        // Overlay discovered runtime door-graphic walls ($18/$24) as obstacles (see `blocked_tiles`),
        // and the squares this visit has been walked back off (see `turned_back_tiles`). Both are the
        // same statement — the cartridge will not let the player stand here — so both are the same
        // overlay; they differ only in how long they are believed.
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

    /// Number of A-presses to spend trying to open a card-key door before giving up and treating the
    /// tile as an unopenable wall. Generous: opening plays a short "used the CARD KEY" text.
    const DOOR_OPEN_ATTEMPTS: u32 = 40;

    /// On Silph Co floors, handle the card-key-door graphic tile ($18/$24) the player is facing — a
    /// closed door or a decorative wall that the `MetaTileMap` can't see (it's `ReplaceTileBlock`'d in
    /// at runtime over a floor tile). Try to open it with the Card Key (press A); if it won't open
    /// after `DOOR_OPEN_ATTEMPTS` it's a wall — remember the faced tile so `observe_state` blocks it and
    /// the BFS routes around it. Returns `true` while actively trying to open (the caller should then
    /// skip normal movement so the A-press isn't overridden by a held direction).
    fn handle_card_key_door(&mut self, api: &mut PokemonApi) -> bool {
        use crate::pokemon::map_metadata::{map_has_card_key_doors, PlayerFacingDirection};
        let Ok(state) = api.game_state() else { return false; };
        if !map_has_card_key_doors(state.map.map) { self.door_open_attempts = 0; return false; }
        let front = api.mmu().read_pointer(&pokered_symbols::wTileInFrontOfPlayer);
        // Card-key door tiles are $18/$24, EXCEPT on Silph 11F where the gate blocking the President's
        // chamber (and the Giovanni trigger tiles inside it) is tile $5e (pokered `PrintCardKeyText`
        // special-cases SILPH_CO_11F + $5e). Facing it with the Card Key opens it.
        let is_11f_gate = front == 0x5e && state.map.map == Map::SilphCo11F;
        if front != 0x18 && front != 0x24 && !is_11f_gate { self.door_open_attempts = 0; return false; }
        let p = state.map.player_position;
        let faced = match state.map.player_direction {
            PlayerFacingDirection::Up    => Point8 { x: p.x, y: p.y.wrapping_sub(1) },
            PlayerFacingDirection::Down  => Point8 { x: p.x, y: p.y + 1 },
            PlayerFacingDirection::Left  => Point8 { x: p.x.wrapping_sub(1), y: p.y },
            PlayerFacingDirection::Right => Point8 { x: p.x + 1, y: p.y },
        };
        // ⚠️ **Giving up has to be remembered, or it is not giving up.** The branch below blocks the
        // tile and resets the counter — and the next overworld tick starts a fresh run of forty A
        // presses at the same door, because nothing here read what the last one concluded. Each press
        // prints "Darn! It needs a CARD KEY!", which is a text box, which is another tick of A from
        // the reader, and the run never leaves: `soak` found the agent doing this on Silph Co 2F from
        // four separate seeds, for the whole of every run. `blocked_tiles` already holds the verdict;
        // this is the line that consults it.
        if self.blocked_tiles.contains(&(state.map.map, faced)) {
            self.door_open_attempts = 0;
            return false;
        }
        self.door_open_attempts += 1;
        if self.door_open_attempts > Self::DOOR_OPEN_ATTEMPTS {
            // Won't open — it's a decorative wall. Block it and let the caller reroute.
            self.blocked_tiles.insert((state.map.map, faced));
            self.door_open_attempts = 0;
            false
        } else {
            // Pulse A facing the door — the game's `PrintCardKeyText` opens any faced $18/$24 while the
            // Card Key is in the bag (then it becomes floor and normal movement continues).
            api.toggle_button(JoypadButton::A);
            true
        }
    }

    pub(crate) fn set_state(&mut self, state: AgentState) {
        if self.state != state {
            // Whatever the reader had collected, said now — see [`Self::flush_text_reader`]. It is
            // a no-op from every other state, and `ReadingTextBox` is never replaced by another
            // `ReadingTextBox` while it holds anything: the two places that build one both check
            // first that the agent is not already reading.
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

    /// Drive the "Which move should be forgotten?" replace-move menu (appears on a level-up learn or
    /// when teaching an HM to a 4-move mon). `cursor_index` is the current cursor slot. Asks the
    /// policy which slot to forget (it keeps the strongest moves) and navigates there / presses A.
    fn drive_forget_menu(&mut self, api: &mut PokemonApi, cursor_index: u8) -> Result<(), String> {
        let game_state = api.game_state()?;
        self.poll_policy(&game_state, api);
        let which = api.learning_pokemon_index();
        let current_moves: Vec<_> = game_state.pokemon.get(which)
            .map(|p| p.moves.iter().flatten().copied().collect())
            .unwrap_or_default();
        match api.move_to_learn() {
            // ⚠️ **Asked once per menu, then held.** See `forget_choice`.
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

    /// Where the player is, in the **expanded** coordinates the model reads everywhere else.
    ///
    /// ⚠️ **Not `raw_player_coords`.** That is the same square before the connection-strip offsets
    /// `MetaTileMap` adds, so on any map with a connection the two differ — and a walk reported as
    /// stopping at a square the action ids and the map picture put somewhere else is worse than one
    /// that says nothing. Observing a whole `GameState` for two numbers is affordable because this
    /// runs once per abort, not once per tick.
    fn player_at(&self, api: &PokemonApi) -> Option<Point8> {
        self.observe_state(api).ok().map(|state| state.map.player_position)
    }

    fn abort_overworld(
        &mut self,
        destination: MetaTile,
        reason: OverworldActionAbortedReason,
        at: Option<Point8>,
    ) {
        self.event(AgentEvent::OverworldActionAborted { destination, reason, at });
        self.set_state(AgentState::Idle);
    }

    pub fn take_overworld_action(&mut self, action: OverworldAction) {
        self.event(AgentEvent::StartedOverworldAction { destination: action.tile.clone() });
        // A fresh walk has no square behind it yet. Without this the first tick of *this* walk would
        // pair up with the last square of the *previous* one, and a walk stopped immediately would
        // arm the turn-back watch on somewhere the player never stepped from.
        self.walk_squares = None;
        self.set_state(AgentState::OverworldMovement { destination: action.tile, map: action.map });
    }

    /// Checks if a battle has just started or finished
    fn assert_battle_state(&mut self, game_mode: GameMode) {
        if matches!(game_mode, GameMode::WildBattle | GameMode::TrainerBattle) {
            match self.state {
                AgentState::Battle(_) => {}
                // The nickname screen after a catch runs while wIsInBattle is still 1, so
                // game_mode stays WildBattle even though we're already in the naming flow.
                AgentState::NamingPokemon { .. } => {}
                AgentState::OverworldMovement { destination, .. } => {
                    // entering battle from the overworld
                    let d = destination;
                    // ⚠️ No position, and this arm takes no `PokemonApi` to read one from. It is
                    // also the one abort that needs none: "a battle started" is a complete account
                    // of why the walk stopped and says nothing about the route being wrong.
                    self.abort_overworld(d, OverworldActionAbortedReason::Battle, None);
                    self.event(AgentEvent::BattleStarted);
                    self.set_battle_state(BattleState::default());
                }
                _ => {
                    // entering battle from somewhere else, maybe a textbox
                    self.event(AgentEvent::BattleStarted);
                    self.set_battle_state(BattleState::default());
                }
            }
        } else if let AgentState::Battle(battle_state) = &self.state {
            // Leaving battle.
            if let BattleState::WaitingForMenu { reader, .. } = battle_state {
                // dump remaining text
                self.event(AgentEvent::text_box_from_reader(reader));
            }

            self.event(AgentEvent::BattleEnded);
            self.set_state(AgentState::Idle);
            // A battle reloads the map on the way out, so any tree cut on this map has regrown — drop
            // the "already cut" memory (same reason as the map-change clear above) so the agent re-cuts
            // it instead of routing through a regrown tree and jamming. This matters inside the Celadon
            // gym, whose garden cut-trees regrow after each junior-trainer battle mid-approach to Erika;
            // with a strong lone starter re-cutting them is cheap and keeps Erika reachable.
            self.cut_tiles.clear();
        }
    }

    /// Checks if the naming screen has just opened or closed
    fn assert_naming_screen(&mut self, game_mode: GameMode, api: &mut PokemonApi) -> Result<(), String> {
        if game_mode == GameMode::NamingScreen {
            if !matches!(self.state, AgentState::NamingPokemon { .. }) {
                // the naming screen has just opened
                let species = api.naming_screen_species()?;
                api.release_all_buttons();
                self.set_state(AgentState::NamingPokemon { species, decided: false, ticks: 0 });
            }
        } else if matches!(self.state, AgentState::NamingPokemon { decided: false, .. }) {
            // The naming screen closed before the policy reached a decision (unexpected).
            self.set_state(AgentState::Idle);
        }
        // If decided=true the strict NamingScreen detection no longer fires (buf0 is no
        // longer 0x50 after the name is written), so game_mode is TextBox.  The
        // NamingPokemon match branch handles its own exit once the font unloads.

        Ok(())
    }

    /// If a map script triggers, start a PendingScript countdown before committing to
    /// RunningScript.  If Script mode ends before the delay fires the agent restores
    /// its previous state (e.g. OverworldMovement for a ledge jump).
    fn assert_script_state(&mut self, game_mode: GameMode) {
        // Checking a trash can runs GymTrashScript (Script mode); the CheckingTrashCan state drives
        // and completes it itself, so don't hand it off to the RunningScript machinery. Field moves are
        // the same: DIG warps the player out of the cave, and that warp runs as a script — hand it to
        // RunningScript and the short rollback restores `UsingFieldMove` with `entered_menu` cleared, so
        // the agent re-opens the menu and tries to Dig again from wherever it landed, forever.
        //
        // ⚠️ **`Surfing` belongs on that list for the same reason, and leaving it off is what made a
        // Surf mount cost a paid turn.** `ItemUseSurfboard` ends with `.makePlayerMoveForward`, which
        // sets `BIT_SCRIPTED_MOVEMENT_STATE` and a simulated button press to step the player onto the
        // water — so every successful mount runs a script, from a state that is not
        // `OverworldMovement` and therefore gets `DelayContext::short`'s 40 ms rollback. The step is
        // much longer than that, so the script "committed", the backup was dropped and the agent was
        // put into `AwaitingOverworldAction` — i.e. the walk the mount was part of was thrown away and
        // the identical question put back to the policy. The driver below reaches the overworld on its
        // own and hands the walk back itself (`Surfing::resume`); it needs no rollback machinery.
        if matches!(self.state, AgentState::CheckingTrashCan { .. } | AgentState::UsingElevator { .. }
            | AgentState::UsingFieldMove { .. } | AgentState::Surfing { .. }) {
            return;
        }
        if game_mode == GameMode::Script {
            if !matches!(self.state, AgentState::RunningScript { .. }) {
                // A south-facing ledge jump fires Script for up to ~660 ms; use >800 ms when
                // navigating so no ledge jump ever commits.  From any other state (Idle,
                // AwaitingOverworldAction…) the script is external — commit in 40 ms.
                let rollback_delay = match self.state {
                    AgentState::OverworldMovement { .. } | AgentState::PacingForEncounters { .. } => DelayContext::long(),
                    _ => DelayContext::short(),
                };

                self.backup_current_state(AgentState::RunningScript { rollback_deadline: rollback_delay });
            }
        } else if let AgentState::RunningScript { rollback_deadline: rollback_delay } = self.state {
            if rollback_delay.is_exhausted() {
                // The script committed (ran long enough to be genuine).
                self.backup_state = None;
                self.set_state(AgentState::AwaitingOverworldAction {
                    delay: DelayContext::default(),
                });
            } else {
                // Script mode ended before the deadline — this was transient (e.g. a ledge
                // jump).  Restore the state the agent was in before the script fired.
                self.restore_state_from_backup();
            }
        }
    }

    /// Detects the Buy/Sell/Quit menu and transitions into PokemartShopping.
    /// Ask the policy what to buy, and start the order if it has an answer.
    ///
    /// ⚠️ **One helper because it is called from two ticks of the same visit** — the tick the shop
    /// opens (so a policy that answers at once loses no frames) and every tick after it while a
    /// slower one is still deciding. Two copies would be two places to forget the `affordable`
    /// trim, which is the bug its own ⚠️ describes.
    fn ask_mart_policy(&mut self, game_state: &GameState, api: &mut PokemonApi) {
        self.poll_policy(game_state, api);
        let Some(item) = self.policy.pick_mart_purchase(game_state) else { return };
        // Trim the order to the wallet before ordering — see `affordable`, which is also what the
        // second and later purchases of a visit go through.
        match item.and_then(|item| affordable(api, game_state.money, item)) {
            Some(item) => self.set_pokemart_state(PokemartState::ChoosingBuyOption(item)),
            None => self.set_pokemart_state(PokemartState::Quitting),
        }
    }

    /// Must run before assert_text_box_state so the mart flow takes priority.
    fn assert_pokemart_state(&mut self, game_mode: GameMode, api: &mut PokemonApi) -> Result<(), String> {
        if game_mode != GameMode::TextBox {
            // Mart interaction ended (returned to Overworld/Script after purchase).
            if matches!(self.state, AgentState::PokemartShopping(_)) {
                api.release_all_buttons();
                self.set_state(AgentState::Idle);
            }
            return Ok(());
        }

        // When the Buy/Sell/Quit menu appears for the first time, ask the policy what to buy.
        if let Some(menu) = api.menu_state() {
            // `SellingToMart` is excluded because it drives the *same* Buy/Sell/Quit menu itself:
            // `PokemartState` only knows how to buy, so letting it take over would answer a sell
            // step's menu with BUY. See `postgame::game_corner`.
            // ⚠️ **Taken on sight of the menu, never on the policy's answer.** Whoever owns the
            // shop has to own it from the moment it opens: the state is what `drives_its_own_menus`
            // keys on, so any tick spent outside `PokemartShopping` is a tick the generic text
            // reader spends pressing A through a shop. See `PokemartState::AwaitingPolicy` for what
            // that bought.
            //
            // ⚠️ **And asked in the same tick, which is not tidiness — it is the frame timing.** A
            // scripted policy answers on its first poll, so ordering here and ordering in the
            // driver's next tick differ by one agent tick per shop; six shops into
            // `full_playthrough` that is a different RNG line, and the run failed at 512/516 steps
            // waiting for a Machop on Victory Road that never appeared. Every mart visit had
            // succeeded on attempt 1. See the ⚠️ in the `test-suite` skill about
            // `can_reach_lavender`: a timing change is priced by `full_playthrough` and by nothing
            // else. Answering here leaves a scripted run byte-identical, and only a policy that
            // says `None` — the model, thinking — is left holding `AwaitingPolicy`.
            if menu.is_mart_buy_sell_menu() && !matches!(self.state, AgentState::PokemartShopping(_) | AgentState::SellingToMart(_)) {
                api.release_all_buttons();
                self.set_state(AgentState::PokemartShopping(PokemartState::AwaitingPolicy));
                let game_state = api.game_state()?;
                self.ask_mart_policy(&game_state, api);
            }
        }

        Ok(())
    }

    /// Whether the text box that has just opened is the **answer** to the interaction the agent
    /// walked over to perform, rather than something that interrupted the walk.
    ///
    /// ⚠️ **The destination kind alone is not enough.** A script can open a box mid-walk — Oak
    /// stopping the player at the edge of Route 1 is the famous one — and "I was on my way to a
    /// sprite" would call that a successful conversation. So the test is what the player is
    /// *facing*: the route to a sprite or a PC ends by turning to it and pressing A, so if the tile
    /// in front is the very thing that was routed to, this box is what the A press produced.
    ///
    /// ⚠️ **"In front" has to mean what the game means by it, which reaches *over* a counter.**
    /// A nurse, a mart clerk and every gym receptionist are talked to across a desk — the player
    /// never stands next to one — so a plain `tile_in_front` answers `Counter`, and every
    /// conversation in a Pokémon Centre was reported as "✗ gave up on Nurse: it was interrupted".
    /// [`MetaTileMap::interaction_in_front`] is the same reach the route that got here was built
    /// on.
    ///
    /// Unreadable state answers `false`: the abort is the safe report, since it only costs the
    /// policy another decision.
    fn interaction_landed(&self, destination: MetaTile, api: &PokemonApi) -> bool {
        if !matches!(destination, MetaTile::Sprite(_) | MetaTile::Pc | MetaTile::Switch { .. }) {
            return false;
        }
        let Ok(state) = self.observe_state(api) else { return false };
        let Some((at, tile)) = state.map.interaction_in_front() else { return false };
        match destination {
            MetaTile::Sprite(_) => tile == destination,
            // ⚠️ **A PC is not in `meta_tiles` and the tile in front reads as `Obstacle`.** It is a
            // hidden event, so nothing distinguishes it from the wall it is drawn on — which is the
            // whole reason `pc_locations` is a table at all (see `MetaTile::Pc`). Matching on the
            // tile would therefore never fire, silently, and only for PCs: the coordinate is the
            // only thing that identifies one.
            MetaTile::Pc => crate::pokemon::tile_map::pc_locations_for(state.map.map).contains(&at),
            // Same argument, same table shape: a bin, a drink machine, the poster and a statue are
            // all hidden events drawn as the scenery they hide in, so the coordinate is again the
            // only thing that identifies one. The *object* is compared too — a map can carry two
            // kinds, and reporting a press on a vending machine as a press on the poster would be
            // the `Obstacle` bug wearing a better disguise.
            MetaTile::Switch { object, ordinal } => crate::pokemon::tile_map::hidden_objects_for(state.map.map)
                .iter()
                .enumerate()
                .any(|(index, site)| site.at == at && site.object == object && index as u8 + 1 == ordinal),
            _ => false,
        }
    }

    /// Is `destination` an item lying on the floor, i.e. a sprite drawn as a Poké Ball?
    ///
    /// ⚠️ **The picture, never the name.** Everything the player can walk up to is a sprite to this
    /// game — people, boulders, fossils, the lab's Pokédex — and only `PictureId` separates
    /// `Potion1` from `Hiker`. This is the same test `llm::tools` verbs a menu row with, which is
    /// what keeps "pick up the Potion" and "nothing was picked up" talking about the same set.
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
    ///
    /// ⚠️ **Cleared whatever the answer is.** A latch that only cleared on success would report the
    /// same failure on every overworld tick for the rest of the run.
    fn check_pending_pickup(&mut self, api: &PokemonApi) {
        let Some((target, settle)) = self.pending_pickup else { return };
        if settle > 0 {
            self.pending_pickup = Some((target, settle - 1));
            return;
        }
        let Ok(state) = self.observe_state(api) else { return };
        self.pending_pickup = None;
        let MetaTile::Sprite(name) = target else { return };
        // ⚠️ **`hidden` is the whole answer, and leaving it out reported every successful pickup on
        // a missable object as a failure.** `HideObject` at the end of the pickup script sets the
        // object's bit in `wMissableObjectFlags`; the sprite stays in the map's own list, keeps its
        // name and keeps its Poké Ball picture, and only `Sprite::hidden` moves — measured on the
        // Mt Moon Moon Stone as `hidden=true picture=PokeBall on_screen=false`. So a name-only test
        // answers "still there" for a ball that has just gone into the bag, which is what the
        // deployed run of 2026-09-01 was told 200 ms after `AI found MOON STONE!`.
        // `MetaTileMap::actions` has always filtered on the same flag; this is the second reader.
        if state.map.sprites.iter().any(|sprite| sprite.name == name && !sprite.hidden) {
            self.event(AgentEvent::OverworldPickupFailed { target });
        }
    }

    /// Answer an armed [`TurnBackWatch`]: has the cartridge walked the player back off the square the
    /// last walk was stopped on?
    ///
    /// ⚠️ **"Was I moved" is not the test; "was I put back where I came from" is.** Plenty of scripts
    /// move the player and almost all of them are progress rather than refusal — Route 5's guard
    /// steps you *through* the gate once you hand over the drink, a Rocket Hideout arrow slides you
    /// across the room, a Seafoam hole drops you a floor, Oak marches you to his lab. Every one of
    /// those goes somewhere new. Only a refusal returns the player to the exact square they stepped
    /// from, and only that is treated as a wall.
    ///
    /// `OverworldMovement` is excluded so that a re-issued walk stepping back cannot be read as the
    /// game's own shove. ⚠️ **That guard is reasoning, not a measurement** — removing it breaks no
    /// test, because on Cinnabar's doorstep the shove lands while the agent is in `script`/`wait`,
    /// the policy having been asked from the square it has not been moved off yet. It is kept because
    /// the alternative failure is a wall invented on a square that has nothing wrong with it, and
    /// because nothing is lost by waiting: a missed shove just bounces once more and re-arms.
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
            // No em dash: this is one of the strings the agent generates.
            self.event(AgentEvent::TextBox { message: format!(
                "the game walked you back off {tile}, so it is being treated as a wall until you \
                 leave {map} and come back") });
        }
    }

    fn assert_text_box_state(&mut self, game_mode: GameMode, api: &PokemonApi) {
        // wFontLoaded=1 while the naming screen is active too; NamingPokemon{decided:true}
        // handles its own exit, so don't interfere.
        if matches!(self.state, AgentState::NamingPokemon { decided: true, .. }) {
            return;
        }
        if game_mode == GameMode::TextBox {
            if !matches!(self.state, AgentState::ReadingTextBox { .. }) {
                // text box opened
                if let AgentState::OverworldMovement { destination, .. } = self.state {
                    // Talking to someone *is* this action succeeding — see
                    // `AgentEvent::OverworldInteractionCompleted`. Either way the state is left
                    // behind: the `set_state(ReadingTextBox)` below is what clears it, exactly as
                    // `abort_overworld`'s own `set_state(Idle)` used to be overwritten by it.
                    if self.interaction_landed(destination, api) {
                        // ⚠️ **Armed here rather than answered here.** The pickup script has not
                        // run yet, so the sprite is still in the map whether or not it is about to
                        // be taken; the question is asked again by `check_pending_pickup` once the
                        // overworld is back. See [`AgentEvent::OverworldPickupFailed`].
                        if self.is_item_ball(destination, api) {
                            self.pending_pickup = Some((destination, PICKUP_SETTLE_TICKS));
                        }
                        self.event(AgentEvent::OverworldInteractionCompleted { target: destination });
                    } else {
                        let at = self.player_at(api);
                        // ⚠️ **This is where "the game stopped you to say something" might mean "and
                        // you may not stand there".** Arm the watch and let the cartridge answer —
                        // see [`PokemonAgent::turned_back_tiles`]. Nothing is concluded here: a guard
                        // who merely talks, an NPC, a sign all come through this line too, and none of
                        // them moves the player.
                        if let (Some(at), Some((came_from_raw, current_raw)), Some(map))
                            = (at, self.walk_squares, self.last_map)
                            // The walk has to have actually stepped somewhere, or "back where you
                            // came from" is the square you are already on and the watch answers
                            // itself on its first tick.
                            && came_from_raw != current_raw
                        {
                            self.turn_back_watch = Some(TurnBackWatch {
                                map, tile: at, came_from_raw, ticks: TURN_BACK_WATCH_TICKS });
                        }
                        self.abort_overworld(destination, OverworldActionAbortedReason::Textbox, at);
                    }
                }
                // ⚠️ **Which reader is a fact about the *game*, not about the agent's own state,
                // and reading it off the state was the bug.** A battle screen has the two HUDs on
                // it — `ONIX` and its level at y=0, ours at y=7 — and a full-screen reader splices
                // them into the front of every frame of the message. That is only correct while
                // `self.state` is `Battle(_)`, and it is not: the arm below drops to `Idle` when a
                // box closes, so the *first* box of a battle got the message-box reader and every
                // one after it got the full-screen one. Deployed, that read as
                // `Emb ONIX 14Ember ONIX 14Ember us …`. `wIsInBattle` is the condition that
                // actually says a HUD is on screen, and `read_battle_state` returns on that byte
                // before it reads anything else.
                let in_battle = crate::pokemon::battle::BattleStateReader::read_battle_state(api.mmu())
                    .is_some();
                let reader = match in_battle {
                    true => PokemonTextReader::message_box_only(),
                    false => PokemonTextReader::default(),
                };
                // ⚠️ **A menu the agent did not open is closed, not confirmed — it must never begin
                // reading one as though it were a conversation.** This is the transition where the
                // difference is knowable: everything that drives menus on purpose is excluded
                // from this function by `drives_its_own_menus`, so reaching it with a menu on screen
                // means something *left* one behind — a driver abandoned by
                // [`DRIVER_ESCAPE_SILENCE`], a `press_buttons` batch that opened the START menu, an
                // aborted PC. The old behaviour was to A-mash it, and every one of those menus is a
                // closed loop under A.
                //
                // ⚠️ **Armed for a short window here, not tested on every tick**, which is why it
                // can act immediately where the hatch below waits 30 s: a per-tick rule has to be
                // right about every screen of every conversation, this one only about the moment a
                // box opens with no driver behind it. ⚠️ It cannot be the single transition tick
                // either — `wFontLoaded` flips a third of a second before the menu draws itself, so
                // testing here and only here answers no every time. See
                // [`Self::menu_handover_ticks`] for the measurements.
                //
                // It reuses `escaping_menus` rather than a state of its own for the reason that
                // latch exists at all — getting out takes more menus than the one that gave it away
                // — and it is cleared where it means something, by `poll_policy`, i.e. by the agent
                // reaching a decision point.
                self.menu_handover_ticks = MENU_HANDOVER_TICKS;
                self.set_state(AgentState::ReadingTextBox { reader });
            }
        } else if matches!(self.state, AgentState::ReadingTextBox { .. }) {
            // Text box closed. `set_state` flushes the reader on the way out, which is the same
            // funnel a script or a battle stealing the state now goes through.
            self.set_state(AgentState::Idle);
        }
    }

    /// Advance `gb` by at least `min_cycles`, **ticking the state machine every
    /// [`AGENT_RESOLUTION`]** rather than once for the lot. Returns what was actually emulated and
    /// the last tick's result.
    ///
    /// ⚠️ **Every driver of a live emulator must come through here.** [`Self::update`] coalesces:
    /// hand it 250 ms and it runs the state machine *once*, having already missed 230 ms of the
    /// game. That is fine for the tests, which call `gb.run(AGENT_RESOLUTION)` and `update` in
    /// lockstep, and it was wrong everywhere else — `host.rs` and `render.rs` both paced themselves
    /// on wall clock, ran `gb.run(min_cycles)` for however long the last loop iteration took, and
    /// gave the agent one tick for it. So the agent's real decision rate in a deployment was the
    /// host's loop rate, and a descheduled thread, a checkpoint write or a busy node silently
    /// lowered it; `MAX_CATCHUP` (250 ms) is the cap, and a walking step is 267 ms.
    ///
    /// What that costs is turns. A held direction keeps walking — pokered starts another step
    /// whenever the pad is still down at the end of one — so the agent has one step's width to
    /// notice it has arrived at the corner and press the other way. Miss it and the player
    /// overshoots, the route recomputes from a tile one further on, and the walk oscillates about
    /// the turn until [`MAX_MOVEMENT_SILENCE`] gives up. The deployed run of 2026-09-03 did exactly
    /// that on Route 12's one-wide corridor west at (11, 63): three walks abandoned after 60 s each,
    /// then an issue report saying the connection to Route 11 was unreachable. It is reproduced by
    /// `mechanics::a_corner_is_turned_at_a_coarse_host_tick`, which paced (11, 62) ↔ (11, 64) for the
    /// whole of its budget at a 250 ms tick and now reaches Route 11 in the same 4.5 s of game time
    /// at every tick it is asked for.
    ///
    /// Slicing is free: `gb.run` is a `while cycles < min` loop over whole instructions, so twelve
    /// calls of 20 ms cost what one of 250 ms costs. In the ordinary case the host is a millisecond
    /// ahead of the clock and this runs its body once.
    pub fn run(&mut self, gb: &mut crate::game_boy::GameBoy,
               cache: &mut crate::pokemon::map_metadata::MapMetadataCache,
               min_cycles: MachineCycles) -> (MachineCycles, Result<(), String>) {
        let mut ran = MachineCycles::ZERO;
        let mut owed = min_cycles;
        let mut result = Ok(());
        while owed > MachineCycles::ZERO {
            // `gb.run` finishes the instruction it is in, so this returns *at least* the slice and
            // `owed` can only reach zero by having emulated the whole of `min_cycles`.
            let slice = gb.run(owed.min(AGENT_RESOLUTION));
            ran += slice;
            owed = owed.saturating_sub(slice);
            let mut api = PokemonApi::with_cache(gb, cache);
            // The *last* tick's result, which is what a single-tick call has always reported. Both
            // callers publish on change, so an intermediate failure that has cleared by the end of
            // the batch is one nobody needed to hear about.
            result = self.update(&mut api, slice);
        }
        (ran, result)
    }

    pub fn update(&mut self, api: &mut PokemonApi, delta_cycles: MachineCycles) -> Result<(), String> {
        // ── Throttled decision-making ─────────────────────────────────────────────
        self.cycles += delta_cycles;
        if self.cycles < AGENT_RESOLUTION { return Ok(()); }

        let mut delta_cycles = MachineCycles::ZERO;
        while self.cycles >= AGENT_RESOLUTION {
            delta_cycles += AGENT_RESOLUTION;
            self.cycles -= AGENT_RESOLUTION;
        }
        // **W9.** Counted here rather than at the bottom, because every early return below is a tick
        // that did not ask the policy anything — which is exactly what the watchdog measures.
        self.cycles_since_poll += delta_cycles;

        // ── Manual input ──────────────────────────────────────────────────────────
        // The policy's escape hatch (`queue_manual_input`). It pre-empts everything below, including
        // the post-Champion cutscene handler and the "not in game" check — a title screen or a
        // continue prompt is precisely somewhere the state machine cannot help and a raw button can.
        // Inert unless something enqueues, which nothing in the agent or in any scripted policy does.
        //
        // **W5** — the policy is asked first, because it is the only one that ever has anything:
        // `LlmPolicy` parks a `press_buttons` decision and this is where it is collected. The default
        // `take_manual_input` returns an empty `Vec`, which does not allocate, so this costs the
        // scripted policies one virtual call per tick.
        let queued = self.policy.take_manual_input();
        if !queued.is_empty() {
            self.queue_manual_input(queued);
        }
        if self.drive_manual_input(api) {
            return Ok(());
        }

        // ── W9: the stuck-run watchdog ────────────────────────────────────────────
        // Below the manual-input queue on purpose: while a nudge is being delivered the run is being
        // un-stuck, and asking for another on top of it would stack presses. Above everything else,
        // because a jam can be in any state — including the ones the checks below hand off to.
        //
        // Deliberately **not** an early return. The watchdog only ever *adds* a question; the state
        // machine keeps running underneath it, and if the jam clears itself the next ordinary poll
        // resets the clock and nothing more is said.
        self.run_watchdog(api);

        // ── Did the game just walk the player off a square? ───────────────────────
        self.check_turn_back(api);

        // ── The end of the game ───────────────────────────────────────────────────
        // ⚠️ **Above the `?` below, and that placement is the whole reason this is a separate
        // check.** `game_mode()` answers `None` through every screen transition, and a Hall of Fame
        // ceremony is made of them — a fade to black, a party parade, the credits. Anything hung off
        // `game_state()` or off the state machine below would miss the one frame that matters.
        self.check_hall_of_fame(api);

        let game_mode = api.game_mode()
            .ok_or_else(|| "Not in game".to_string())?;

        // ── Post-Champion cutscene ────────────────────────────────────────────────
        // Beating the rival hands the game to a five-stage script chain (Oak's congratulation, his
        // aside about the rival, "come with me", his exit, the player following him to the Hall of
        // Fame). It is not something to *play*: there is nothing to route to and no decision to make,
        // only text to sit through. Driving it with the agent's normal per-tick machinery wedges it
        // solid at stage 6 — verified at length in `probe_hall_of_fame_bisect`, which shows the same
        // A-button trace advancing the chain fine when the agent is not in the loop, so the cause is
        // something the agent does, not the input it sends. Rather than keep guessing, the agent
        // recognises the cutscene and gets out of its own way: hands off the joypad, one deliberate A
        // press per cycle, nothing else.
        if self.drive_post_champion_cutscene(api) {
            return Ok(());
        }

        // Silph Co's card-key doors/walls are placed into the map at runtime and are invisible to the
        // ROM-decoded `MetaTileMap`, so the agent routes straight into them. When facing one, try to
        // open it with the Card Key; unopenable ones get blocked so the BFS routes around (see
        // `handle_card_key_door`). While actively opening, skip normal handling so the A-press isn't
        // overridden by a held movement direction.
        if game_mode == GameMode::Overworld && self.handle_card_key_door(api) {
            return Ok(());
        }

        // A trainer has engaged (line of sight) and the battle is initialising on its own.
        // The agent must neither hold a direction (which wedges the trainer's walk-up) nor
        // mash A (which keeps re-triggering the pre-battle interaction and prevents InitBattle
        // from running).  Release everything and simply wait: once wIsInBattle flips,
        // trainer_battle_pending() goes false and assert_battle_state takes over.
        //
        // Exception: a *script*-triggered trainer (e.g. the Mt Moon Super Nerd, engaged by stepping
        // on a coord trigger rather than by line of sight) displays its challenge text in a text box
        // with `wCurOpponent` already set, and the battle only starts once that text is advanced.
        // In that case do NOT suppress input — fall through so the text-box handler mashes A and the
        // battle begins. There is no walk-up to wedge because the trainer is stationary.
        if api.trainer_battle_pending() && game_mode != GameMode::TextBox {
            api.release_all_buttons();
            return Ok(());
        }

        // ── Global "Which move should be forgotten?" handler ──────────────────────
        // The replace-move menu appears both in battle (a level-up learn) and in the overworld
        // (teaching an HM to a 4-move mon), so drive it here — before any state-specific handling,
        // including during TeachingMove — whenever its unique (5,8) geometry AND on-screen prompt
        // text are both present.
        {
            // Detect the forget menu by its on-screen prompt (robust against stale menu geometry) and
            // drive it from the live cursor (`wCurrentMenuItem`). This covers both the battle level-up
            // menu (origin (5,8)) and the overworld HM-teach menu (origin (15,8)), which
            // `battle_menu_state()` would not classify because it keys on x==5.
            let forget_showing = api.menu_state().is_some()
                && api.on_screen_text(true).map_or(false, |t| is_forget_move_prompt(&t));
            if forget_showing {
                let cursor = api.menu_geometry().2;
                self.drive_forget_menu(api, cursor)?;
                return Ok(());
            }
            // The menu has gone, so the answer it was for has been carried out. See `forget_choice`.
            self.forget_choice = None;
        }

        self.assert_naming_screen(game_mode, api)?;
        self.assert_script_state(game_mode);
        self.assert_battle_state(game_mode);
        self.assert_pokemart_state(game_mode, api)?;
        // Skip generic text-box handling while shopping or teaching a move — those state machines
        // drive their own menu input.
        if !drives_its_own_menus(&self.state) {
            self.assert_text_box_state(game_mode, api);
        }

        // ⚠️ **Below the asserts, so the text box and any script behind it have both finished.**
        // The pickup is a script: the box, `GiveItem`, then `HideObject`. Asking any earlier finds
        // the sprite still in the map and calls every successful pickup a failure.
        if game_mode == GameMode::Overworld {
            self.check_pending_pickup(api);
        }

        // ⚠️ **The net under every driver that drives its own menus.** Each of them is a private
        // conversation with the game: it presses its own buttons, it is excluded from the text reader
        // above, and it polls nothing. So each one is a place the run can stop for ever if the game
        // answers something the driver did not plan for — and the game refuses things constantly.
        // `soak` found the shape in `Surfing`: the policy picked SURF in Saffron City, the driver
        // opened the party menu and pressed it, the game said "No SURFing on <mon> here!", and the
        // driver's only exit (`entered_menu && back in the overworld`) never came round. It sat there
        // for the rest of the run.
        //
        // Rather than a tick budget per driver — nineteen of them, each needing its own number and
        // each a place to forget — this is one rule over the measure that already means *nothing is
        // happening*.
        //
        // ⚠️ **Dropping to `Idle` mid-menu is safe only because `assert_text_box_state` arms the
        // B latch on the way back in, and that is a recent repair rather than a standing property.**
        // This comment used to claim the hand-back "composes with the escape in `ReadingTextBox`",
        // which was true only when the abandoned driver happened to leave a *list* or a `CANCEL`
        // behind. Every driver here opens the START menu first, and a `UsingFieldItem` abandoned
        // standing on it left a screen the 30 s hatch could not see at all: the deployed run spent
        // 55 minutes in ViridianMart with the text reader A-mashing the trainer card open and shut.
        // The latch is the composition now; the leftover shape is not asked to be convenient.
        //
        // ⚠️ **It must stay well clear of the legitimately slow drivers**, which is what sets the
        // number rather than the jam: a mart shop of several items, a PC deposit, a Fly animation and
        // an elevator ride all run without a poll. 60 s is several times the slowest of those at the
        // cartridge's own text speed, and still well inside the watchdog's 300 s.
        if drives_its_own_menus(&self.state)
            && self.cycles_since_poll.to_duration() >= DRIVER_ESCAPE_SILENCE {
            let abandoned = format!("{} got no answer from the game for {:?}; starting over",
                                    self.state, DRIVER_ESCAPE_SILENCE);
            api.release_all_buttons();
            self.set_state(AgentState::Idle);
            self.event(AgentEvent::TextBox { message: abandoned });
            return Ok(());
        }

        let mut new_events: Vec<AgentEvent> = vec!();

        match self.state {
            AgentState::Idle => {
                api.release_all_buttons();
                match game_mode {
                    GameMode::TextBox => {
                        self.set_state(AgentState::ReadingTextBox { reader: PokemonTextReader::default() });
                    }
                    GameMode::Script => { /* assert_script_state will transition to PendingScript */ }
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
                // ⚠️ **An arrow tile is the walk, not a script that ended it, and this is the one
                // line that says so.** `MetaTileMap`'s BFS routes *through* a spinner deliberately,
                // treating the step onto it as an edge to wherever the slide deposits the player —
                // so the slide is the route being carried out. This arm saw it as `GameMode::Script`
                // and priced it against a window sized for a **ledge jump** (~660 ms, hence
                // `DelayContext::long`'s 1000), and a slide is 3.2 s to 13.9 s measured on Rocket
                // Hideout B2F. Every crossing therefore breached the deadline, dropped the backup
                // and reported "the game took over" — the planner and the executor disagreeing about
                // the same tile. Deployed, that was **535 of 678 walks on B2F and 423 of 646 on
                // B3F**, against 1 abort each on the two floors with no arrows; the run got across
                // only because each abort left it somewhere new and the next poll re-planned.
                //
                // ⚠️ **Re-armed every tick rather than given a bigger number.** A longer window is a
                // guess about the longest slide in the game, and it would still be a *deadline* — the
                // point is that a spin has no deadline, it ends when the cartridge says it ends. That
                // drops through to the `restore_state_from_backup` arm in `assert_script_state`,
                // which is the ledge jump's path and already the right one.
                if player_is_spinning(api) {
                    *rollback_delay = DelayContext::long();
                }
                if rollback_delay.is_exhausted() {
                    // script already breached rollback deadline, start mashing the A button
                    api.toggle_button(JoypadButton::A);
                } else {
                    // Still inside the rollback window (might be a transient ledge jump).
                    // Whatever the cause, release any movement button the agent was holding:
                    // a held direction during a trainer's scripted walk-up wedges the
                    // engagement so the battle never starts. A is not needed yet — genuine
                    // scripts are advanced by the A-mashing branch once the deadline passes.
                    let crossed = rollback_delay.tick(delta_cycles);
                    api.release_all_buttons();
                    if crossed {
                        // script has just breached rollback deadline, commit to RunningScript so we can start mashing next cycle
                        if let Some(AgentState::OverworldMovement { destination, .. }) = self.backup_state.as_ref() {
                            let destination = *destination;
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
                // ⚠️ **A black-out warp that has not landed yet is not a world to ask about.** See
                // [`blackout_in_flight`] for the whole argument and for what the model was sent
                // when this was missing. Re-arming the delay rather than skipping one poll is what
                // buys the settle: the wait ends where the cartridge clears the byte, and the
                // ordinary [`DelayContext::long`] then runs from *there*, giving the new map the
                // same second every other warp landing gets before anything reads it.
                //
                // ⚠️ Skipped with a flag rather than an early `return`, which would jump over the
                // `new_events` drain at the bottom of this function — the mistake the ⚠️ on the
                // battle reader's flush is about. Re-arming alone is *nearly* enough (a 1000 ms
                // delay does not fire on a 20 ms tick) but not quite: `update` coalesces every whole
                // [`AGENT_RESOLUTION`] it has been given, so one call after a long host stall can
                // carry more than a second and tick straight through.
                let warping = blackout_in_flight(api) && self.blackout_ticks < MAX_BLACKOUT_WAIT_TICKS;
                if warping {
                    self.blackout_ticks += 1;
                    api.release_all_buttons();
                    *delay = DelayContext::long();
                }
                if !warping && delay.tick(delta_cycles) {
                    let game_state = self.observe_state(api)?;
                    self.poll_policy(&game_state, api);
                    // Incrementally build the world graph: every time we settle in the overworld,
                    // record this section's live (sprite-resolved) reachable warps/connections, keyed
                    // by the raw landing coords (the space warp `to_position`s use). The player is
                    // stationary here so the raw coords are the landing position.
                    //
                    // ⚠️ **On arrival only, deliberately.** Re-observing on every settle looks like an
                    // improvement — a Strength puzzle opens a ladder the graph never learns about — but
                    // `observe` *overwrites* the node, and what is reachable depends on where the player
                    // is standing. On a map split by one-way ledges (Cerulean is the documented one) a
                    // re-observation from a terrace replaces the landing's rich edge set with that
                    // terrace's poor one, and cross-map routing that used to work stops working. Tried
                    // and reverted 2026-08-05; see the archive entry.
                    if self.last_map != Some(game_state.map.map) {
                        self.last_map = Some(game_state.map.map);
                        self.world_graph.observe(game_state.map.map, api.raw_player_coords(), &game_state.map);
                        // Cut trees regrow when you leave and re-enter a map, so a tree cut on a prior
                        // visit is standing again — drop the stale "already cut" memory or the agent
                        // will route through a regrown tree and jam (e.g. re-entering the Vermilion gym
                        // enclosure after Lt. Surge). `CuttingTree` cuts sequentially without changing
                        // maps, so this never fires mid-cut.
                        self.cut_tiles.clear();
                        // And the squares this map turned the player away from, for the opposite
                        // reason: they are remembered *because* they might stop being true, and a
                        // map change is the cheapest honest moment to ask again. See
                        // [`PokemonAgent::turned_back_tiles`].
                        self.turned_back_tiles.clear();
                        self.turn_back_watch = None;
                    }
                    // A non-walking field action (e.g. teach an HM) takes priority over walking.
                    match self.policy.pick_field_move(&game_state) {
                        Some(crate::pokemon::policy::FieldMove::ReorderParty { slot }) => {
                            // Direct RAM reorder — instant, no menus. Make `slot` the lead, then resume.
                            api.release_all_buttons();
                            api.move_party_member_to_front(slot as usize)?;
                            self.event(AgentEvent::TextBox { message: format!("Moved party slot {slot} to the front") });
                            self.set_state(AgentState::Idle);
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::TeachMove { item, target_slot }) => {
                            api.release_all_buttons();
                            // ⚠️ **The same last line of defence `CutTree` has below, for the same
                            // reason: this driver has no way back.** A TM or HM aimed at a Pokémon
                            // that is not in its learnset is answered with
                            // `MonCannotLearnMachineMoveText` and `jr .chooseMon`
                            // (`engine/items/item_effects.asm`) — back to the party menu, cursor
                            // untouched — so `TeachingMove` navigates to the same slot, presses A,
                            // and is refused again. Its only exit is "the mon knows the move", which
                            // never comes: the attempt is 60 s of A-mashing ended by
                            // `DRIVER_ESCAPE_SILENCE`, after which the policy asks for the identical
                            // teach and it all happens again. The deployed run of 2026-08-27 spent
                            // its life in that loop.
                            //
                            // ⚠️ **The message has to name the alternative, not just the refusal.**
                            // What the driver reported before this was "got no answer from the game
                            // for 60s; starting over", which reads as a malfunction and tells a model
                            // nothing it can act on — the same mistake "it was interrupted" made
                            // about a guard turning the player back. So it says who in the party
                            // *can* take it, or that nobody can, which is the whole decision.
                            //
                            // An *empty* slot is somebody else's problem and deliberately not
                            // refused here: `pick_field_move` holds a `TeachMove` back until the
                            // target is in the party, so a slot that is empty at this moment is a
                            // policy still waiting rather than a teach that cannot work.
                            let incompatible = game_state.pokemon.get(target_slot as usize)
                                .is_some_and(|mon| !crate::pokemon::learnset::can_learn(mon.species, item));
                            if incompatible {
                                self.event(AgentEvent::TextBox { message: teach_refusal(&game_state, item, target_slot) });
                                self.set_state(AgentState::Idle);
                                return Ok(());
                            }
                            self.set_state(AgentState::TeachingMove { item, target_slot, press: true, entered_menu: false, settle: 0, evolve_from: None });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::EvolveWithStone { stone, target_slot, evolve_from }) => {
                            api.release_all_buttons();
                            self.set_state(AgentState::TeachingMove { item: stone, target_slot, press: true, entered_menu: false, settle: 0, evolve_from: Some(evolve_from) });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::CutTree) => {
                            api.release_all_buttons();
                            // ⚠️ **The last line of defence, and it is here because the driver has
                            // no way back.** `CuttingTree` opens the party menu and mashes A until
                            // the overworld comes back; without the Cascade Badge pokered answers
                            // `.newBadgeRequired` → `jp .loop` and the overworld never comes back, so
                            // the whole attempt is sixty seconds ended by `DRIVER_ESCAPE_SILENCE`.
                            // Both policies are already stopped from asking — `MetaTileMap::can_cut`
                            // withholds the action, `tools::hm_available` refuses the tool call — but
                            // those are two places to forget and this is the one every request passes
                            // through.
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
                            // ⚠️ **Whoever in the party knows Cut, not the lead.** `can_use_cut` above
                            // has always asked the *party*, so a run whose Cut lives on an HM slave
                            // passed the gate and then drove the menu onto slot 0, which cannot cut and
                            // has no CUT row — the same closed loop the gate exists to prevent, one
                            // step further in.
                            let Some((slot, move_index)) =
                                crate::pokemon::policy::field_move_carrier(&game_state, crate::pokemon::move_name::PokemonMoveName::Cut)
                            else {
                                self.event(AgentEvent::TextBox {
                                    message: "nobody in the party knows Cut; not cutting".to_string() });
                                self.set_state(AgentState::Idle);
                                return Ok(());
                            };
                            self.set_state(AgentState::CuttingTree { press: true, entered_menu: false, tree_pos, slot, move_index });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::CheckTrashCan { target, facing }) => {
                            api.release_all_buttons();
                            self.set_state(AgentState::CheckingTrashCan { target, checked: false, press: true, facing });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::UseFieldMove { slot, move_index }) => {
                            api.release_all_buttons();
                            self.set_state(AgentState::UsingFieldMove { press: true, entered_menu: false, slot, move_index, from_map: game_state.map.map });
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
                            // Baseline the source inventory *now*, before any menu is touched, so the
                            // driver can tell "moved `qty`" from "was already short".
                            let start_qty = match op {
                                crate::pokemon::postgame::item_storage::PcItemOp::Deposit => api.bag_item_quantity(item),
                                crate::pokemon::postgame::item_storage::PcItemOp::Withdraw => api.pc_box_item_quantity(item),
                            };
                            self.set_state(AgentState::UsingItemPc(ItemPcState::new(op, item, qty, pc, start_qty)));
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::PushBoulder { boulder, dir }) => {
                            // Primitive: push the boulder at `boulder` one tile in `dir`. The policy plans
                            // *which* push (via `MetaTileMap::solve_boulder_push`); the agent just executes it.
                            api.release_all_buttons();
                            self.set_state(AgentState::PushingBoulder { boulder, dir });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::UseElevator { panel, floor }) => {
                            api.release_all_buttons();
                            self.set_state(AgentState::UsingElevator { panel, floor, selected: false, press: true });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::UseFieldItem { item, target }) => {
                            api.release_all_buttons();
                            // ⚠️ **The last line of defence `CutTree` and `TeachMove` have, for the
                            // third member of the same family.** `ItemUsePtrTable` sends a great many
                            // ids to `UnusableItem`, which is `jp ItemUseNotTime`: the bag list comes
                            // back with the cursor where it was, and this driver's only exit is
                            // `game_mode == Overworld`, which a refusal never reaches. So the attempt
                            // is 60 s of A-mashing ended by `DRIVER_ESCAPE_SILENCE`, reported as "got
                            // no answer from the game", which reads as a malfunction and has the
                            // policy ask for the identical use again. `tools::resolve_field_move`
                            // refuses it first; this is here because that is one place to forget.
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
                // The 60 s bound, and why it is silence rather than ticks: `MAX_MOVEMENT_SILENCE`.
                if self.cycles_since_poll.to_duration() >= MAX_MOVEMENT_SILENCE {
                    api.release_all_buttons();
                    let at = self.player_at(api);
                    self.abort_overworld(destination, OverworldActionAbortedReason::DidNotArrive, at);
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                let game_state = self.observe_state(api)?;
                // Keep the last two squares of the walk — the turn-back watch needs the one the
                // player stepped *from*, and this is the only arm that already holds a `GameState`.
                // See [`PokemonAgent::walk_squares`].
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
                    // Map changed — success for warps and connections (both take you off the map).
                    if matches!(destination, MetaTile::Warp { .. } | MetaTile::Connection { .. } | MetaTile::ConnectionWater(_)) {
                        new_events.push(AgentEvent::OverworldActionCompleted { destination });
                        self.set_state(AgentState::Idle);
                    } else {
                        // ⚠️ No position: the player is on a *different map*, so a coordinate here
                        // would be read against the map named in the destination and mean nothing.
                        self.abort_overworld(
                            destination,
                            OverworldActionAbortedReason::WrongMap(game_state.map.map),
                            None,
                        );
                    }
                } else if matches!(destination, MetaTile::Warp { .. })
                    && game_state.map.player_tile() == destination
                    && is_on_map_border(&game_state.map)
                    // …and it is *not* one of the tileset's step-on warp tiles. Being on the border is
                    // not enough: Victory Road 3F's ladder down to 2F sits at (2,0) and is a Cavern
                    // step-on tile, so the outward press just walks into the top wall — the agent sat
                    // there for the whole budget. Those go to the route branch below, which builds the
                    // step-off/step-on pair that re-fires them.
                    && !game_state.map.is_step_on_warp(game_state.map.player_position)
                {
                    // Player is standing on an EDGE warp tile (at y=0, y=max, x=0, or x=max).
                    // These only fire when the player presses the outward direction off the map
                    // edge — not just by standing on the tile.  Press that direction.
                    //
                    // Interior warps are handled by the route-execution branch below: a player
                    // already on an interior warp gets a [step-off, step-on] route, and stepping
                    // back onto the warp tile fires it (CheckWarpsNoCollision). Pressing a fixed
                    // geometric direction here would jam the player against a wall instead.
                    let pos = game_state.map.player_position;
                    let h = game_state.map.height.saturating_sub(1) as u8;
                    let exit_dir = if pos.y == 0 { JoypadButton::Up }
                        else if pos.y == h { JoypadButton::Down }
                        else if pos.x == 0 { JoypadButton::Left }
                        else { JoypadButton::Right };
                    api.release_all_buttons();
                    api.press_button(exit_dir);
                } else if destination == MetaTile::Empty {
                    // A cave "wander" (`MetaTileMap::wander_action`): the policy asked for a plain floor
                    // tile purely to keep the player walking so per-step wild encounters fire — there is
                    // no grass to stand in underground. Untyped tiles are never in `actions()`, so there
                    // is nothing to route to and nothing to "arrive" at; pace between two adjacent floor
                    // tiles instead, exactly as the grass case does. Pacing starts on a NEIGHBOUR, never
                    // the player's own tile, because a wander often begins on the warp the player just
                    // came in through — stepping back onto that would warp straight out again.
                    let pos = game_state.map.player_position;
                    match adjacent_pacing_pair(&game_state.map, pos) {
                        Some((tile_a, tile_b)) => self.set_state(AgentState::PacingForEncounters {
                            map: game_state.map.map, tile_a, tile_b, heading_to_b: false, stalled: 0, paced: 0 }),
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
                        // Pace against a grass neighbour if there is a *steppable* one, and against any
                        // plain neighbour if there is not. Either works: the ROM rolls for an encounter
                        // on the tile being stepped **onto**, so a grass↔plain pair still fires on every
                        // other step, whereas a pair the player cannot walk fires never.
                        let pair = adjacent_grass(&game_state.map, pos).map(|b| (pos, b))
                            .or_else(|| adjacent_pacing_pair(&game_state.map, pos));
                        if let Some((tile_a, tile_b)) = pair {
                            self.set_state(AgentState::PacingForEncounters { map: game_state.map.map, tile_a, tile_b, heading_to_b: true, stalled: 0, paced: 0 });
                        } else {
                            // TODO this should not happen, we shouldn't generate an action if this is true
                            //      the adjacent grass tile should be in the action
                            let at = Some(game_state.map.player_position);
                            self.abort_overworld(destination, OverworldActionAbortedReason::NoAdjacentGrass, at);
                            self.set_state(AgentState::Idle);
                        }
                    } else {
                        new_events.push(AgentEvent::OverworldActionCompleted { destination });
                        self.set_state(AgentState::Idle);
                    }
                } else {
                    // ⚠️ **The route is recomputed every tick and only ever its head is pressed, so
                    // no recipe here may depend on its own tail.** `actions()` is asked again from
                    // wherever the player now stands, `route[0]` is pressed, and the rest is thrown
                    // away; a two-step plan therefore only completes if step two is what the *next*
                    // recomputation independently decides to do first. That is usually true, because
                    // walking is memoryless. It was false for a warp entry the player was already
                    // standing on: `actions()` emitted step-off then step-back, and the tick after
                    // the step-off recomputed step-off again from the neighbour, so the pair
                    // oscillated until `MAX_MOVEMENT_SILENCE` gave up. The fix was to make that case
                    // a one-button route (`MetaTileMap::warp_trigger`) rather than to remember the
                    // tail, and the property is worth stating because the next such recipe will look
                    // just as reasonable.
                    let action = game_state.map.actions().into_iter()
                        .find(|a| a.tile == destination)
                        // A specific connection landing isn't in `actions()` (only the nearest crossing
                        // is) — re-derive its route each tick so the walk to it can still be tracked.
                        .or_else(|| match destination {
                            MetaTile::Connection { to_map, to_position } =>
                                game_state.map.connection_action(to_map, to_position),
                            // Nor is a water edge, when a land bridge to the same map is nearer —
                            // `actions()` emits one crossing per adjacent map. Without this the walk to
                            // one aborts on its first tick with `NoRoute` and the policy re-issues it
                            // for ever (Route 24 → Cerulean, the only way to Cerulean Cave).
                            MetaTile::ConnectionWater(to_map) =>
                                game_state.map.water_connection_action(to_map),
                            _ => None,
                        });
                    match action {
                        None => self.abort_overworld(
                            destination,
                            OverworldActionAbortedReason::NoRoute(destination),
                            Some(game_state.map.player_position),
                        ),
                        Some(a) => match a.route.first() {
                            // Pulse A via toggle so hJoyPressed fires every other tick —
                            // press_button (after release_all) would only fire once since A
                            // stays held and hJoyPressed goes dark on the next frame.
                            Some(&JoypadButton::A) => api.toggle_button(JoypadButton::A),
                            // Hold direction buttons for continuous walking.
                            Some(&btn) => {
                                // If this step would walk onto water while on foot, mount Surf first.
                                // (The BFS only routes over water when the player can Surf, so a water
                                // tile here means we intend to cross it.)
                                let pos = game_state.map.player_position;
                                let next = step_pos(pos, btn);
                                let onto_water = matches!(
                                    next.and_then(|n| game_state.map.tile_at_checked(n)),
                                    Some(MetaTile::Water) | Some(MetaTile::ConnectionWater(_))
                                );
                                let surfing = api.mmu().read_pointer(&pokered_symbols::wWalkBikeSurfState) == 2;
                                if onto_water && !surfing {
                                    if let (Some(water_pos), Some((slot, move_index))) = (next,
                                        crate::pokemon::policy::field_move_carrier(&game_state, crate::pokemon::move_name::PokemonMoveName::Surf)) {
                                        api.release_all_buttons();
                                        // The walk rides through the mount — see `Surfing::resume`.
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
                                // ⚠️ **The empty route is where a fishing row turns into a cast.** The
                                // walk to the shore ends facing the water, and from there the row's
                                // whole point — START → ITEM → the rod → USE — is a bag chain no
                                // route can express, so hand off to the driver rather than going
                                // Idle. Every policy gets this: the row is in `actions()`, so a
                                // scripted, random or model-driven run all fish the same way, with
                                // the rod re-resolved from the live bag rather than the one the row
                                // was minted with.
                                if let MetaTile::Fish { .. } = destination
                                    && let Some(rod) = crate::pokemon::postgame::fishing::Rod::best_in_bag(&game_state.bag)
                                    && let Some(at) = crate::pokemon::postgame::fishing::nearest_castable_water(&game_state.map)
                                {
                                    api.release_all_buttons();
                                    self.set_state(AgentState::Fishing(
                                        crate::pokemon::postgame::fishing::FishState::new(rod, at)));
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
                // ⚠️ **B, not A, inside a PC menu — it is the only way out.** Every Gen 1 PC menu is
                // a closed cycle under A (menu → first entry → refusal message → same menu, cursor
                // never moving), and it wedged the deployed run permanently at the item PC in Red's
                // House 2F, eight tiles from a fresh save. `in_pc_menu` has the full trace.
                //
                // This arm cannot collide with the drivers that use the PC on purpose:
                // `UsingPcBox` and `UsingItemPc` are both excluded from `assert_text_box_state`, so
                // neither ever reaches `ReadingTextBox`. It *does* cover their abort paths, which
                // drop to `Idle` while the menu is still open and would otherwise A-mash for ever.
                //
                // ⚠️ **The PC is not the only closed loop under A, so the second half of this is a
                // *rule* rather than another named place.** `soak` found two more in one run, and
                // naming them one at a time is how the PC fix aged badly:
                //
                //   • the man in the Cerulean badge house is a literal `.loop` — print "which BADGE
                //     should I describe?", show the list, describe the badge under the cursor, jump
                //     back — whose only exit is `jr c, .done`, i.e. **B**;
                //   • the party menu's field-move box is the same shape whenever the move is refused:
                //     A on SURF → "No SURFing on <mon> here!" → back to the party menu, cursor
                //     untouched. Every "you can't use that here" in the game is this.
                //
                // So after [`TEXT_BOX_ESCAPE_SILENCE`] without the agent reaching *any* decision point,
                // the reader stops confirming and starts leaving. B is safe to hold: in Gen 1 it
                // advances a text box exactly like A, and everywhere it differs — a list, a party
                // menu, a field-move box — differing is the point.
                //
                // ⚠️ **Silence alone is not the trigger, and this is the whole of what makes the
                // hatch safe.** The first version pressed B on *anything* once the agent had been
                // quiet for 30 s, on the reasoning that B advances a text box exactly like A. It does
                // — and it still took `full_playthrough` apart: the run blacked out in Mt Moon and sat
                // on step 52 of 516 until its budget ran out. Ordinary conversations are routinely
                // longer than 30 s (the TM man's "a new technique, pick the POKéMON…" is 40), so the
                // hatch was firing constantly, and "identical on a message box" turns out not to
                // survive contact with a whole run.
                //
                // So it fires only when there is something to *leave*: a **list** menu or the party
                // screen's **field-move box**. Those are the two shapes every closed loop returns to —
                // the badge house's list, the party menu behind a refused SURF — and neither is ever
                // on screen during a conversation that is merely long. A plain message box is left to
                // A, which is what a conversation needs.
                //
                // ⚠️ **And not in a battle.** A text box in a battle belongs to the battle driver —
                // the agent passes through here between turns — and B there is not "leave the
                // conversation", it is "cancel the move I am choosing". A gym leader is routinely
                // quieter than 30 s (`soak` measures `battle:policy` gaps of 25–40 s in ordinary
                // play). Battles do their own refusal handling in [`BattleState`]; this is for the
                // overworld.
                //
                // ⚠️ **Latched on the agent, because getting out takes more menus than the one that
                // gave it away.** The refused SURF is three deep — field-move box over party list over
                // START menu — and only the innermost of them is a shape worth reacting to. Deciding
                // per tick therefore gets one level out and A-mashes straight back in, which is what
                // it did: B on the field-move box, A on the party list underneath, and round again for
                // ever. Once the loop has been recognised the agent keeps pressing B until it reaches
                // a decision point, which is the definition of having got out — and is what clears the
                // flag, in `poll_policy`, the one place that means it.
                //
                //
                // The third shape has no list menu *and* no field-move box: Bill's own PC, whose
                // "Which POKéMON do you want to see?" is drawn in an ordinary message box. What it
                // does have is a **CANCEL entry**, which is what every Gen 1 pick-one-of-these menu
                // offers and what no conversation ever shows — so that word on screen is the third
                // way in. (`soak` found eleven seeds standing in Bill's house watching the Eevee
                // evolutions cycle.)
                //
                // ⚠️ **The fourth shape is the START menu, and it is the one nothing could see.**
                // `DrawStartMenu` calls `TextBoxBorder` directly and never writes `wTextBoxID`, so
                // `menu_state()` answers `None` and neither of the first two tests can fire; its
                // exit row reads `EXIT` rather than `CANCEL`, so the third cannot either. Left open
                // with the cursor on the player-name row it is a closed loop under A — the trainer
                // card opens, `WaitForTextScrollButtonPress` takes the next press, and
                // `RedisplayStartMenu` puts the cursor back — and that is what wedged the deployed
                // run in ViridianMart for 55 minutes. B is unconditionally right there:
                // `home/start_menu.asm`'s `.buttonPressed` does `and PAD_B | PAD_START` /
                // `jp nz, CloseStartMenu`. See [`is_start_menu`] for why it takes raw geometry and
                // a word on screen rather than a [`MenuState`] method.
                //
                // ⚠️ **And the same recognition runs once more, immediately, for the first
                // [`MENU_HANDOVER_TICKS`] of a box the agent did not open.** That is the hand-over
                // rule — see [`Self::menu_handover_ticks`]. Without it a leftover menu is A-mashed
                // for the full 30 s before anything looks, and for the START menu that is 30 s of
                // opening and shutting the trainer card.
                //
                // ⚠️ **Not in a battle, for either rule.** The bag inside a fight is a `ListMenuBox`
                // like any other, and B there is "cancel the move I am choosing" rather than
                // "leave"; a gym leader is routinely quieter than 30 s. Battles do their own refusal
                // handling in [`BattleState`].
                let in_battle = api.mmu().read_pointer(&pokered_symbols::wIsInBattle) != 0;
                let handing_over = self.menu_handover_ticks > 0;
                self.menu_handover_ticks = self.menu_handover_ticks.saturating_sub(1);
                let timed_out = self.cycles_since_poll.to_duration() >= TEXT_BOX_ESCAPE_SILENCE;
                // ⚠️ The hand-over rule believes only the screen; the 30 s rule may also believe the
                // lingering ids, because by then the silence has ruled out a conversation. See
                // [`MenuEvidence`] — getting this the same way round for both cost a nickname.
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
                let button = if api.in_pc_menu() || self.escaping_menus { JoypadButton::B } else { JoypadButton::A };
                reader.update_with(api, button);
            }
            AgentState::Battle(ref mut battle_state) => {
                // Global safety net: the game has refused the item that was just selected. Back out
                // from whichever sub-state is driving, rather than letting it spin.
                //
                // ⚠️ **Latched, not a single B.** This is the bug the whole `BATTLE_REFUSALS` comment is
                // about: dismissing the message with one B lands back on the *bag list*, and the plain
                // `BattleState::default()` this used to hand back presses A there — reselecting the
                // same item, for ever. `soak` caught it from eleven different starting states at once,
                // every one of them a bag with a TM or a key item in it.
                if api.on_screen_text(false).map_or(false, |t| shows_battle_refusal(&t)) {
                    api.toggle_button(JoypadButton::B);
                    self.set_battle_state(BattleState::backing_out());
                    return Ok(());
                }
                match battle_state {
                    BattleState::WaitingForMenu { reader, delay, backing_out, confirming, confirm } => {
                        // ⚠️ **Hoisted above everything, because the A presses that defeat it do not
                        // come from the branch that detects the problem.** A move the game refuses —
                        // out of PP, or disabled — drops back to the move list, and getting out takes
                        // more than one B: the first dismisses the message, the second leaves the list.
                        // In between, `battle_menu_state()` reads `None` while the message box is up
                        // and the arm below hands to the text reader, which mashes A and re-selects the
                        // same spent move. So the intent is latched here and held for a fixed run of
                        // ticks, ahead of every branch that could press A.
                        //
                        // 100 ticks is 2 s of game time — about 50 rising edges, far more than the two
                        // presses it takes. Overshooting is harmless: B at the main battle menu does
                        // nothing. Counting down rather than latching a `bool` is what bounds it, and
                        // if the list is *still* there afterwards the detection below simply re-latches.
                        if *backing_out > 0 {
                            *backing_out -= 1;
                            api.toggle_button(JoypadButton::B);
                            return Ok(());
                        }
                        if let Some(menu_state) = api.menu_state() {
                            // A voluntary switch (PKMN → pick mon) pops the SWITCH/STATS/CANCEL
                            // sub-menu, which isn't a battle_menu_state. Drive its cursor to SWITCH
                            // (index 0) and confirm, else the reader would wait on it forever.
                            if menu_state.is_switch_stats_cancel_menu() {
                                if menu_state.current_item == 0 {
                                    api.toggle_button(JoypadButton::A);
                                } else {
                                    api.toggle_button(JoypadButton::Up);
                                }
                                return Ok(());
                            }
                            // After an item-use turn the menu geometry/text_box_id are unreliable — the
                            // leaked battle bag list and the next-turn FIGHT menu can both read as
                            // (5,4)/ListMenuBox, so battle_menu_state misclassifies them. The on-screen
                            // text is authoritative, so resolve these two cases from it first:
                            //   • "FIGHT … RUN" → the main battle menu is up → hand to the policy.
                            //   • "CANCEL"      → a leaked bag list → back out with B to end the turn
                            //     (A-mashing it re-opens "use on which?" and burns the whole stack).
                            let screen = api.on_screen_text(false).unwrap_or_default();
                            // Bag first: while the leaked bag list is still on screen (its "CANCEL"
                            // entry visible, or an unusable-item message), back out with B. Checking
                            // this before the FIGHT test matters during the transition, when the bag and
                            // the next-turn FIGHT menu briefly render together — handing to the policy
                            // then would drive the move-select on top of the still-open bag.
                            // ⚠️ **"No PP left" belongs here for the same reason `CANCEL` does**, and it
                            // is the PC menu's trap in a different costume: the refusal drops back to
                            // the *move list* with the cursor still on the spent move, so A-mashing
                            // re-selects it and bounces again, for ever. B backs out to the main menu,
                            // where the policy gets to pick something else. `soak` found it in Viridian
                            // Forest — a Bulbasaur out of Vine Whip, 300 s of game time silent.
                            //
                            // The moves *offered* are already filtered on `pp > 0`
                            // (`Pokemon::available_battle_moves`), so this is the case where the game
                            // and the party data disagree — Disable, or PP spent since the read — which
                            // is exactly the case a filter cannot cover.
                            if screen.contains("No PP left") {
                                // ⚠️ **Latch here, not in the `MoveList` arm below** — this check runs
                                // first and returns, so the arm that "handles" a spent move is never
                                // reached while the message is on screen. That is why the handling
                                // already there (added after an earlier hours-long wedge) did not save
                                // this one: it only ever saw the ticks where the message had *gone*,
                                // and on those it pressed A and re-selected the same move.
                                *backing_out = BACKING_OUT_TICKS;
                                api.toggle_button(JoypadButton::B);
                                return Ok(());
                            }
                            // ⚠️ **Left unlatched deliberately.** A refusal never reaches here — the net
                            // at the top of the battle arm catches it first and latches — and the bag
                            // list is now caught by its own arm below, whatever it is scrolled to. So
                            // what is left is the ordinary end of an item turn, where one B per tick is
                            // exactly right and a 2 s latch is 2 s of the next turn's menu ignored.
                            if screen.contains("CANCEL") || shows_battle_refusal(&screen) {
                                api.toggle_button(JoypadButton::B);
                                return Ok(());
                            }
                            // ⚠️ **The SHIFT prompt, which only exists because the soak stopped using
                            // the harness's options.** With battle style SHIFT — the cartridge's own
                            // default, and therefore the deployment's — every enemy switch asks
                            // "<TRAINER> is about to use <MON>! Will <PLAYER> change POKéMON?" over a
                            // yes/no box. A opens the party menu, the party arm below backs out of
                            // it, and the prompt comes round again — for the rest of the battle.
                            //
                            // B is *no*, which is the right default: switching is a decision, and the
                            // policy makes it at the menu that follows. ⚠️ **Only this prompt, not
                            // yes/no in general** — "Use next #MON?" after a faint is also a yes/no
                            // box, and B there flees the battle rather than sending out the next mon.
                            if screen.contains("change") && menu_state.is_yes_no_menu() {
                                api.toggle_button(JoypadButton::B);
                                return Ok(());
                            }
                            // ⚠️ **Only once the screen is trustworthy again.** `confirming` is set
                            // when `Navigating` opens a sub-menu, and for as long as it is running the
                            // bottom third of VRAM may still be showing the main menu this test looks
                            // for — see [`BattleState::confirming`].
                            if *confirming > 0 {
                                *confirming -= 1;
                            } else if screen.contains("FIGHT") && screen.contains("RUN") {
                                new_events.push(AgentEvent::text_box_from_reader(reader));
                                api.release_all_buttons();
                                self.set_battle_state(BattleState::AwaitingPolicy { delay: DelayContext::default() });
                                // ⚠️ **Drained here because this arm returns, and nothing else in
                                // `update` does.** `new_events` is emptied at the very end of the
                                // tick (see `PokemonAgent::event`'s doc), so an early `return`
                                // throws the whole buffer on the floor — and of the six push sites
                                // this is the only one that returns. What it cost was every battle
                                // turn that handed back on the *text* test rather than on the menu
                                // geometry: three turns of a six-turn trainer battle, each holding
                                // the entire account of what the enemy did at the moment it was
                                // discarded ("Enemy ODDISH fainted! Celina gained 183 EXP. Points!
                                // LASS sent out BELLSPROUT!"). The reader was full and the flush
                                // was correct; the event simply never left this function.
                                for event in new_events {
                                    self.event(event);
                                }
                                return Ok(());
                            }
                            match menu_state.battle_menu_state() {
                                // The main menu is ready: the turn is the policy's to decide.
                                //
                                // ⚠️ **All three Safari positions, and FIGHT.** The Safari menu keeps
                                // its cursor between turns, so naming only the position a *fresh* menu
                                // opens on (BALL) left BAIT and ROCK to the `Some(_)` arm below, which
                                // presses A — re-throwing whatever the last turn had chosen with the
                                // policy never asked. `soak` found the agent baiting the same Rhyhorn
                                // for the rest of the run.
                                //
                                // ⚠️ **And deliberately *not* ITEM/PKMN/RUN in an ordinary battle**,
                                // though the symmetry is tempting: that menu resets its cursor to
                                // FIGHT, so those arms are only ever reached on a transient misread —
                                // and handing those to the policy re-times every battle in the game.
                                // Tried: `full_playthrough` blacked out in Mt Moon and stalled at step
                                // 87 of 516. The rule here is the ⚠️ from `with_original_battle_timing`
                                // — a change that re-rolls the RNG stream has to earn it.
                                Some(BattleMenuState::Fight)
                                | Some(BattleMenuState::SafariBall) | Some(BattleMenuState::SafariBait)
                                | Some(BattleMenuState::SafariRock) => {
                                    new_events.push(AgentEvent::text_box_from_reader(reader));

                                    api.release_all_buttons();
                                    self.set_battle_state(BattleState::AwaitingPolicy { delay: DelayContext::default() });
                                }
                                Some(BattleMenuState::PokemonList { index }) => {
                                    // ⚠️ **Only if the party list is what is actually on screen.**
                                    // `battle_menu_state` reads this off `wTopMenuItemX/Y`, which
                                    // *linger*: the geometry still says "party list at (0,1)" long
                                    // after a message box has been drawn over it. Every branch below
                                    // then presses a direction, which a message box swallows — the
                                    // screen freezes exactly as it did for "There's no will to fight!",
                                    // and `soak` found this half of it in a Ditto battle on Route 15,
                                    // 180 s on one frame reading "It's super effective!".
                                    //
                                    // The party list shows an HP bar per member, so two or more `/`
                                    // means a list; a battle message box shows the active mon's alone.
                                    // Same heuristic `UsingItem` uses to spot the "use on which?" menu,
                                    // and the same answer as the refusals: clear the box first, drive
                                    // the cursor once it is gone.
                                    if api.on_screen_text(false).map_or(0, |t| t.matches('/').count()) < 2 {
                                        api.toggle_button(JoypadButton::A);
                                        return Ok(());
                                    }
                                    // A party list is only ours to drive here for a FORCED switch — the
                                    // active mon fainted and the game demands a replacement. Send the
                                    // first alive member. If instead the active mon is ALIVE, this party
                                    // list is a leaked voluntary sub-menu (e.g. the "use on which?" menu
                                    // Gen 1 re-shows after an item heal); blindly confirming it would
                                    // re-heal/re-switch, so back out with B.
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
                                    // A move list is showing. Normally this is the move Navigating
                                    // highlighted, so confirm it with A. Two cases bounce straight back
                                    // to this menu instead, and confirming them again loops forever:
                                    //
                                    //   • the move is **Disabled** ("… is disabled!") — the disabled
                                    //     slot is already excluded from the available moves, so backing
                                    //     out with B lets the policy pick another;
                                    //   • the move is **out of PP** ("No PP left"). `available_battle_moves`
                                    //     filters pp == 0, so this only happens on a stale read — the
                                    //     policy spent the last PP and still sees the pre-turn value, then
                                    //     picks that move again. Backing out re-polls the policy against
                                    //     fresh PP. (Found by a grind wedging here for hours: the screen
                                    //     read "TYPE/ WATER 166/234 0/ 5 … No PP left" with the agent
                                    //     happily re-confirming Hydro Pump.)
                                    let disabled = api.game_state().ok()
                                        .and_then(|g| g.battle)
                                        .and_then(|b| b.player.disabled_move_slot) == Some(index);
                                    // Only the game's own message counts here. Reading the move's PP
                                    // out of RAM instead looked equivalent but is not: that copy is a
                                    // turn stale, so it reports 0 for moves the game will happily
                                    // accept, and backing out of those changes move choice all over
                                    // the run (it re-tuned the playthrough's whole RNG line, and it
                                    // broke in two different places before this was narrowed).
                                    //
                                    // ⚠️ **Sticky, because one B is not enough and the message is not
                                    // reliably on screen.** Two things defeated the plain check. The
                                    // refusal renders a character at a time — `soak`'s fixture spends
                                    // most of its ticks reading "No", "No P", "No PP lef" — so the
                                    // `contains` is *false* on most of the ticks that matter. And even
                                    // when it matched, the first B only dismissed the message and
                                    // returned to this very list, where the next tick saw no message,
                                    // pressed A, and re-selected the same spent move. Getting out takes
                                    // B until the list itself is gone, so the intent is latched once and
                                    // held until it is.
                                    let no_pp = api.on_screen_text(false).unwrap_or_default()
                                        .contains("No PP left");
                                    if disabled || no_pp {
                                        *backing_out = BACKING_OUT_TICKS;
                                        api.toggle_button(JoypadButton::B);
                                    } else if reading_dialogue(&menu_state, *confirming) {
                                        // The geometry is stale and the game is talking: this is the
                                        // turn resolving, not a move list. See `reading_dialogue`.
                                        reader.update(api);
                                    } else if let Some(mut pending) = *confirm {
                                        let pending = &mut pending;
                                        // ⚠️ **One press, then wait for the game's own record of it.**
                                        // See `Confirm`. `wPlayerMoveListIndex` is what
                                        // `MoveSelectionMenu` writes on the A press
                                        // (`wCurrentMenuItem - 1`, `engine/battle/core.asm`), so it
                                        // is the only witness that the press landed on the row that
                                        // was chosen rather than on one a stray press had left the
                                        // cursor on. Reading it is what turns "we pressed A at about
                                        // the right time" into "the game selected slot N".
                                        let committed = api.mmu()
                                            .read_pointer(&crate::pokemon::symbols::pokered_symbols::wPlayerMoveListIndex);
                                        match pending.pressed {
                                            // ⚠️ **Issued, recorded, and therefore *finished* — the
                                            // confirm has to get out of the way here.** Holding the
                                            // arm after the move is committed means nothing presses
                                            // A to advance the text the move produces, and the
                                            // battle stops progressing: eight tests timed out on
                                            // their cycle budget with the right move being used
                                            // every turn. Clearing it drops back to the ordinary
                                            // path on the next tick, which is what reads the turn
                                            // out.
                                            true if committed == pending.slot => {
                                                *confirm = None;
                                                api.release_all_buttons();
                                            }
                                            // Issued and not yet acted on. Waiting is the whole fix:
                                            // the agent observes 20 ms behind its own press, so a
                                            // second one here is what used to land in the next menu.
                                            true if pending.waited < CONFIRM_ACK_TICKS => {
                                                pending.waited += 1;
                                                api.release_all_buttons();
                                            }
                                            // The press was not acted on. The list is drawn a few
                                            // ticks before it takes input, so the first one can land
                                            // in that gap and simply do nothing.
                                            true if pending.attempts < CONFIRM_ATTEMPTS => {
                                                pending.pressed = false;
                                                pending.waited = 0;
                                                api.release_all_buttons();
                                            }
                                            // ⚠️ **Out of attempts, and this is the assertion that
                                            // catches a stray press *now*.** Before this the turn
                                            // proceeded with whatever move the game had taken and the
                                            // only evidence was the cartridge's own sentence. Back
                                            // out to the main menu and let `Navigating` line it up
                                            // again; `MAX_NAVIGATING_TICKS` bounds that and hands
                                            // back to the policy if it cannot.
                                            true => {
                                                *backing_out = BACKING_OUT_TICKS;
                                                api.toggle_button(JoypadButton::B);
                                            }
                                            // The press.
                                            false if index == pending.slot => {
                                                pending.pressed = true;
                                                pending.waited = 0;
                                                pending.attempts += 1;
                                                api.toggle_button(JoypadButton::A);
                                            }
                                            // ⚠️ **The cursor is not where `Navigating` believed it
                                            // was, so hand it back rather than confirm or stand
                                            // still.** `MoveSelectionMenu` initialises
                                            // `wCurrentMenuItem` to `wPlayerMoveListIndex + 1` — the
                                            // move used last — and a hand-off taken while that write
                                            // is in flight sees a value the menu is about to
                                            // overwrite. Confirming there takes the wrong move;
                                            // waiting there wedges the battle. `Navigating` owns
                                            // cursor movement and is bounded by
                                            // `MAX_NAVIGATING_TICKS`, so it re-drives from wherever
                                            // the cursor really is and hands back to the policy if
                                            // it cannot.
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
                                    // ⚠️ **A bag list here is always a leak, and A is always the wrong
                                    // button.** Nothing hands the bag to this state on purpose any
                                    // more: `UsingItem` owns it from the grid to the confirm, and
                                    // every give-up path backs out. So whatever is under the cursor is
                                    // an item *nobody chose* — and if it happens to be a TM or a key
                                    // item, A is the refusal loop that wedged eleven soak states.
                                    //
                                    // The "CANCEL" test above catches this only when the list is
                                    // scrolled far enough for CANCEL to be on screen, which is exactly
                                    // the kind of half-covering check that reads as handled. Keying on
                                    // the menu instead covers it wherever the list is scrolled to.
                                    //
                                    // ⚠️ **One B per tick, not a latch.** The list is what is being
                                    // detected, so the detection holds for as long as the problem does
                                    // — a latch buys nothing and costs everything if the read is
                                    // wrong. And it can be: the comment above records that a leaked
                                    // bag list and the next turn's FIGHT menu both read as
                                    // `(5,4)`/`ListMenuBox`, so a misclassified tick would otherwise
                                    // ignore two seconds of a perfectly good battle menu. The case a
                                    // latch is genuinely needed for — the refusal message covering the
                                    // list, so `menu_state` reads `MessageBox` — is the net's, above.
                                    api.toggle_button(JoypadButton::B);
                                },
                                Some(_) if reading_dialogue(&menu_state, *confirming) => {
                                    // Same as the move list above: the menu geometry has not caught
                                    // up, and what is on screen is the game talking.
                                    reader.update(api);
                                },
                                Some(_) => {
                                    // A menu really is showing. Confirm it; there is nothing to read.
                                    api.toggle_button(JoypadButton::A);
                                },
                                None => {
                                    // something other than the battle menu is showing — wait for
                                    // the text box to render before reading it
                                    if delay.tick(delta_cycles) {
                                        reader.update(api);
                                    }
                                }
                            }
                        } else {
                            // no menu is showing, click mashing the A button
                            api.toggle_button(JoypadButton::A);
                        }
                    }

                    BattleState::AwaitingPolicy { delay } => {
                        if delay.tick(delta_cycles) {
                            let game_state = api.game_state()?;
                            self.poll_policy(&game_state, api);
                            if let Some(action) = self.policy.pick_battle_action(&game_state) {
                                new_events.push(AgentEvent::BattleActionStarted {
                                    actor: active_pokemon_name(&game_state),
                                    opponent: opponent_pokemon_name(&game_state),
                                    action,
                                });
                                // In-battle item use goes through the dedicated `UsingItem` driver: it
                                // opens the bag, finds the item **by id** and confirms it, including
                                // the "use on which POKéMON?" party menu where the item opens one.
                                // Completion is the item's count in the bag dropping.
                                //
                                // ⚠️ **Every item, not a list of the ones known to work.** This used
                                // to gate on sixteen ids — the potions, the balls, the stat items —
                                // and let everything else fall through to `Navigating`. That path
                                // cannot select an item at all: it walks menu geometry, so it opened
                                // the bag and handed it to `WaitingForMenu`, which pressed A on
                                // whatever the cursor was resting on. So *use Antidote* meant "use
                                // whatever is under the cursor", and on a bag holding a TM or a key
                                // item it meant the "This isn't the time to use that!" loop. Reaching
                                // the driver by default makes the list a tuning knob (which items
                                // open a party menu) rather than the thing that decides whether an
                                // item can be used at all.
                                if let BattleAction::UseItem { item, .. } = action {
                                    let start_qty = game_state.bag.iter()
                                        .find(|b| b.id == item.id).map(|b| b.quantity).unwrap_or(0);
                                    let active = game_state.battle.as_ref().map(|b| b.active_party_slot).unwrap_or(0);
                                    let entry_hp = game_state.pokemon.get(active as usize).map(|p| p.current_hp).unwrap_or(0);
                                    self.set_battle_state(BattleState::UsingItem { ticks: 0,
                                        item: item.id, start_qty, entry_hp, press: true, confirmed: false,
                                        delay: DelayContext::default(),
                                    });
                                    return Ok(());
                                }
                                self.set_battle_state(BattleState::Navigating { action, delay: DelayContext::default(), ticks: 0, stable: 0 });
                            }
                        }
                    }

                    BattleState::Navigating { action, delay, ticks, stable } => {
                        if delay.tick(delta_cycles) {
                            // ⚠️ **Nothing below this line polls the policy, so nothing below it may
                            // run forever.** `Navigating` drives menus on the decision the policy has
                            // already made; every exit from it goes through `WaitingForMenu`, which is
                            // what leads back to `AwaitingPolicy` and the next poll. So a `Navigating`
                            // that cannot finish is indistinguishable, from the outside, from a dead
                            // run — and `soak` found exactly that: 300 s silent in `battle:Run` on
                            // Route 1, walking a cursor toward an option it never reached.
                            //
                            // 250 ticks is 5 s of game time. Real navigation is a handful of cursor
                            // steps and a confirm; even the slowest case here — driving the party list
                            // to slot 5 for a voluntary switch — is far inside it. Giving up hands back
                            // to `WaitingForMenu`, which re-reads the screen and asks the policy again,
                            // so the cost of being wrong is one re-decided turn rather than a wedge.
                            const MAX_NAVIGATING_TICKS: u16 = 250;
                            *ticks += 1;
                            if *ticks >= MAX_NAVIGATING_TICKS {
                                // Formatted before the state is touched: `action` borrows from the
                                // `battle_state` that `set_battle_state` replaces.
                                let gave_up = format!("battle navigation to {action:?} got nowhere in 5s; re-deciding");
                                api.release_all_buttons();
                                // Latched: whatever this was navigating may have left a sub-menu open,
                                // and `WaitingForMenu` opens by pressing A. See `BattleState::backing_out`.
                                self.set_battle_state(BattleState::backing_out());
                                self.event(AgentEvent::TextBox { message: gave_up });
                                return Ok(());
                            }
                            // A *voluntary* party switch (chosen from the battle PKMN menu) needs a
                            // dedicated driver: the in-battle party menu isn't a recognized
                            // `battle_menu_state` (its top-menu geometry is stale from the PKMN grid),
                            // so the generic navigator below bails and the party cursor defaults to the
                            // active mon → "… is already out!" → deadlock. Drive the party cursor by
                            // `current_item` straight to the target slot, confirm, then confirm the
                            // SWITCH/STATS/CANCEL sub-menu. Done once the active slot is the target.
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
                                    // SWITCH(0)/STATS(1)/CANCEL(2) → drive to SWITCH and confirm.
                                    let cur = raw.map_or(0, |m| m.current_item);
                                    api.toggle_button(if cur == 0 { JoypadButton::A } else { JoypadButton::Up });
                                    return Ok(());
                                }
                                if bms.is_none() || matches!(bms, Some(BattleMenuState::PokemonList { .. })) {
                                    // The in-battle party list — whether unrecognized (voluntary switch-in
                                    // geometry (11,2)) or recognized as PokemonList (low-HP switch geometry
                                    // (0,1)). Drive the cursor to the target slot; pressing A on it opens
                                    // SWITCH/STATS/CANCEL (handled above). Handle both here so the switch
                                    // never leaks to WaitingForMenu, whose leaked-party-menu B-backout
                                    // would otherwise cancel a legitimate voluntary switch.
                                    let cur = raw.map_or(0, |m| m.current_item);
                                    let btn = if cur == target { JoypadButton::A }
                                        else if cur < target { JoypadButton::Down }
                                        else { JoypadButton::Up };
                                    api.toggle_button(btn);
                                    return Ok(());
                                }
                                // else: still at the main battle grid — fall through to the generic
                                // navigator to open the PKMN menu.
                            }
                            let Some(menu_state) = api.menu_state().and_then(|s| s.battle_menu_state()) else {
                                // No battle menu is recognized — the turn is resolving (result text
                                // or animation), e.g. the "… is fast asleep!" message shown after a
                                // sleeping Pokémon's move is committed. Hand back to WaitingForMenu,
                                // which advances the text (pressing A) and re-detects the menu.
                                // Staying here would hang forever: Navigating issues no input while
                                // no battle menu is on screen, which deadlocks a sleep-locked battle.
                                api.release_all_buttons();
                                self.set_battle_state(BattleState::default());
                                return Ok(());
                            };
                            {
                                let menu_target = BattleMenuState::from_action(*action);

                                // If a leaked sub-menu is showing that doesn't belong to this action —
                                // a bag list (ItemList) while we're trying to FIGHT/switch, or a party
                                // list (PokemonList) while we're trying to FIGHT/use-item — back out
                                // with B instead of navigating it. Otherwise the generic nav walks the
                                // wrong menu (e.g. scrolls the bag to index 0 and selects a key item →
                                // "This isn't the time to use that!").
                                let wrong_submenu = match (menu_state, *action) {
                                    (BattleMenuState::ItemList { .. }, a) => !matches!(a, BattleAction::UseItem { .. }),
                                    (BattleMenuState::PokemonList { .. }, a) => !matches!(a, BattleAction::SwitchPokemon { .. }),
                                    _ => false,
                                };
                                if wrong_submenu {
                                    api.toggle_button(JoypadButton::B);
                                    return Ok(());
                                }

                                // ⚠️ **A sub-menu is only believed once it has been seen twice, and
                                // that is what stops the wrong move being taken.**
                                // `MoveSelectionMenu` initialises `wCurrentMenuItem` to
                                // `wPlayerMoveListIndex + 1` — the move used last — so for a tick
                                // around the list opening the byte still holds whatever the previous
                                // menu left there. Handing over on that transient lines the confirm
                                // up on a row the menu is about to move off, and the press takes the
                                // move the list actually opened on. Traced on a captured mainline
                                // state: hand-off at `wCurrentMenuItem = 2` (slot 1, the target),
                                // and one tick later the menu had written 3 (slot 2, the move used
                                // last). Two consecutive observations outlive that write; a value
                                // the game is settling on survives it.
                                //
                                // ⚠️ It is a *count of observations*, not a delay: the second tick is
                                // the ordinary next one, so nothing waits when the menu is already
                                // settled. `AGENT_RESOLUTION` is untouched.
                                if menu_state == menu_target && *stable == 0 {
                                    *stable = 1;
                                    api.release_all_buttons();
                                    return Ok(());
                                }
                                if menu_state != menu_target {
                                    *stable = 0;
                                }
                                if menu_state == menu_target {
                                    // RUN is a terminal menu option (no sub-menu): press A here to
                                    // actually flee. Handing off to WaitingForMenu instead would bounce
                                    // straight back to the policy on seeing the "FIGHT…RUN" main menu, so
                                    // the flee never fires (an infinite low-PP heal-flee loop). Stay in
                                    // Navigating so a failed escape ("Can't escape!") retries next turn.
                                    // Sub-menu targets (MoveList/ItemList/PokemonList) are confirmed by
                                    // their own WaitingForMenu handlers, so hand those off as before.
                                    //
                                    // **Every Safari option is terminal too** (workstream E): BALL,
                                    // BAIT and ROCK all resolve the turn on the spot, with no list to
                                    // confirm. Without them here a Safari hunt cannot throw anything —
                                    // the menu opens on BALL, so target == state on the very first
                                    // tick, and handing off would bounce straight back to the policy
                                    // with the ball unthrown, for ever. RUN worked only because it is
                                    // the one Safari option that was already in this test.
                                    if matches!(menu_target, BattleMenuState::Run
                                        | BattleMenuState::SafariBall
                                        | BattleMenuState::SafariBait
                                        | BattleMenuState::SafariRock) {
                                        // Re-confirmed for as long as the option keeps coming back: a
                                        // Gen 1 escape is a dice roll whose odds improve with each
                                        // attempt, so bouncing to the policy after one failure would
                                        // mean a flee rarely fires. `MAX_NAVIGATING_TICKS` above is
                                        // what stops that being unbounded.
                                        api.toggle_button(JoypadButton::A);
                                    } else {
                                        api.release_all_buttons();
                                        // ⚠️ **Not a plain `default()`.** The sub-menu is open per RAM
                                        // and may not be on screen yet, and `WaitingForMenu`'s text
                                        // test would read the stale third and hand the turn back to
                                        // the policy with the move unconfirmed. See
                                        // [`BattleState::confirming`].
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

                    BattleState::UsingItem { item, start_qty, entry_hp, press, confirmed, delay: _, ticks } => {
                        // ⚠️ Same bound and same reason as `Navigating`: this drives six menus deep on
                        // a decision the policy already made and polls nothing on the way. `soak` found
                        // it wedged at `confirmed=false` in Viridian Forest for 300 s — the bag list
                        // had moved under it, so the potion was never selected and the completion
                        // signal (the item count dropping) could not arrive.
                        //
                        // ⚠️ **Counted inside the variant, not on the agent.** This state rebuilds
                        // itself every tick with `press` flipped, so `set_state` sees a *different*
                        // state each time and resets anything counting from outside — which is exactly
                        // how the first attempt at this bound silently never fired. 250 ticks is 5 s
                        // of game time; a real heal is well inside it, animation included.
                        //
                        // ⚠️ **This budget must NOT back out latched, though a refused item makes it
                        // tempting.** It fires on a *slow* use as readily as on a refused one — a ball
                        // whose throw animation outruns 5 s is the common case — and at that moment the
                        // bag may still be open with the throw uncommitted, so B-mashing cancels it.
                        // Tried, and it cost `can_get_the_super_rod_and_catch_a_tentacool` all 60 of
                        // its casts: every ball backed out of the bag instead of being thrown, and the
                        // leg burned its 90-minute budget catching nothing. A refusal does not need
                        // this anyway — the refusal net above catches it in a fraction of the time.
                        const MAX_HEALING_TICKS: u16 = 250;
                        let ticks = *ticks;
                        if ticks >= MAX_HEALING_TICKS {
                            api.release_all_buttons();
                            self.set_battle_state(BattleState::default());
                            return Ok(());
                        }
                        use crate::pokemon::item::ItemId;
                        let item = *item;
                        let start_qty = *start_qty;
                        let entry_hp = *entry_hp;
                        let press = *press;
                        let confirmed = *confirmed;

                        let raw = api.menu_state();
                        let bms = raw.and_then(|m| m.battle_menu_state());

                        // Three phases, keyed on the potion's bag count and the active mon's HP:
                        //   • consumed (live < start_qty)  → the heal fully applied; back out of the
                        //     leftover bag/party menus (B) until the main grid reappears (next turn).
                        //   • animating (HP rose past entry but not yet consumed) → the heal bar is
                        //     filling; WAIT (don't re-press A, which would start a second potion).
                        //   • otherwise → navigate the menus to select the potion and confirm the mon.
                        let gs = api.game_state().ok();
                        let active = gs.as_ref().and_then(|g| g.battle.as_ref().map(|b| b.active_party_slot)).unwrap_or(0);
                        let active_hp = gs.as_ref().and_then(|g| g.pokemon.get(active as usize).map(|p| p.current_hp)).unwrap_or(0);
                        let live = gs.as_ref()
                            .and_then(|g| g.bag.iter().find(|b| b.id == item).map(|b| b.quantity))
                            .unwrap_or(0);
                        if live < start_qty {
                            // Potion consumed — the heal is committed. Hand back to WaitingForMenu, which
                            // advances the "recovered HP" / enemy-turn text and safely backs out of any
                            // leftover bag/party sub-menu (the PokemonList/ItemList B-backout gates) to
                            // reach the next turn's FIGHT menu.
                            api.release_all_buttons();
                            self.set_battle_state(BattleState::default());
                            return Ok(());
                        }
                        if active_hp > entry_hp {
                            // Heal bar filling — wait, don't touch the (still-open) party menu.
                            api.release_all_buttons();
                            self.set_battle_state(BattleState::UsingItem { item, start_qty, entry_hp, press: !press, confirmed: true, delay: DelayContext::default(), ticks: ticks + 1 });
                            return Ok(());
                        }
                        // Two-tick press/release cadence for clean rising edges.
                        if !press {
                            api.release_all_buttons();
                            self.set_battle_state(BattleState::UsingItem { item, start_qty, entry_hp, press: true, confirmed, delay: DelayContext::default(), ticks: ticks + 1 });
                            return Ok(());
                        }

                        // "Use on which POKéMON?" party menu — recognized PokemonList (0,1) or party text.
                        // A Poké Ball is thrown straight at the enemy, so for a ball this stays false
                        // (else the enemy-HP "n/m" bars would look like party text and drive a phantom
                        // party menu). The battle-screen slash count is compared while `bms` is
                        // momentarily unreadable.
                        //
                        // ⚠️ **The list is which items open a party menu, and it is not the potions.**
                        // In battle the status heals, the revives and the PP restores all ask "use on
                        // which POKéMON?" too — pokered's `UsableItems_PartyMenu` — and each of them
                        // reached this driver the moment `UseItem` stopped being gated on a list of
                        // sixteen ids. Leaving them out does not fail loudly: the party menu is not a
                        // recognized `battle_menu_state`, so the fallback below presses A and the item
                        // lands on whichever mon the cursor started on.
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
                            // Drive the cursor to the active mon and press A. Keep pressing until the
                            // potion is consumed (completion above fires the instant the bag drops);
                            // one clean press is unreliable, so re-press rather than gate on `confirmed`.
                            let cur = raw.map_or(0, |m| m.current_item);
                            if cur == active { Some(JoypadButton::A) }
                            else if cur < active { Some(JoypadButton::Down) }
                            else { Some(JoypadButton::Up) }
                        } else if let Some(BattleMenuState::ItemList { index }) = bms {
                            // Navigate to the potion by its position in the raw bag (== the battle list
                            // order). If the read momentarily fails, WAIT rather than pressing A on a
                            // fallback index 0 — that lands on an unusable key item ("This isn't the
                            // time to use that!") and deadlocks the bag.
                            match api.bag_item_position(item) {
                                Some(target) => Some(if index == target { JoypadButton::A }
                                    else if index < target { JoypadButton::Down } else { JoypadButton::Up }),
                                None => None,
                            }
                        } else if let Some(grid) = bms.filter(|b| matches!(b,
                            BattleMenuState::Fight | BattleMenuState::Item | BattleMenuState::Pokemon | BattleMenuState::Run)) {
                            // Battle grid → ITEM (0,1).
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
                        self.set_battle_state(BattleState::UsingItem { item, start_qty, entry_hp, press: false, confirmed: next_confirmed, delay: DelayContext::default(), ticks: ticks + 1 });
                    }
                }
            }
            AgentState::PacingForEncounters { map, tile_a, tile_b, ref mut heading_to_b, ref mut stalled, ref mut paced } => {
                /// Overworld ticks on the same tile before the pair is declared unwalkable. One tile
                /// step is ~13 ticks at `AGENT_RESOLUTION`, so this is several steps' worth of slack —
                /// long enough never to fire on healthy pacing, short enough to cost nothing.
                const STALL_TICKS: u16 = 60;

                /// Total ticks of *successful* pacing before giving up on the encounter ever coming.
                ///
                /// ⚠️ **`STALL_TICKS` cannot see this failure**, which is the whole reason this
                /// exists: it counts ticks spent failing to reach the target tile, so it catches
                /// "walking into a wall" and is reset on arrival by healthy pacing. An agent pacing
                /// perfectly in grass where nothing will ever appear resets it for ever.
                ///
                /// 3000 ticks is 60 s of game time.
                ///
                /// ⚠️ **This is not sized to guarantee an encounter, and the first attempt at it was.**
                /// That reasoning gave 180 s — three times the ~57 s in which 99.9% of encounters on
                /// the rarest map (Viridian Forest, 8/256 per step) should land. It was wrong twice
                /// over: the pair is often grass↔plain, so only every *other* step rolls, which
                /// doubles the figure; and more importantly, giving up is not a failure. The policy is
                /// simply asked again, and if it picks grass again the pace resumes — the only thing
                /// that changes is that the run stopped being silent.
                ///
                /// So what this actually bounds is **how long the agent may go without being asked
                /// anything**, and 180 s of that is 60% of the watchdog. `soak` measured exactly that:
                /// seed 1's worst healthy stretch was 182 s, which was this budget running to the end
                /// in Viridian Forest and nothing else.
                const PACING_BUDGET_TICKS: u16 = 3000;

                let game_state = api.game_state()?;
                if game_state.mode == GameMode::Overworld {
                    if game_state.map.map != map {
                        // Blackout or other warp moved us off the grass map — let policy re-route.
                        self.set_state(AgentState::Idle);
                        return Ok(());
                    }
                    *paced += 1;
                    if *paced >= PACING_BUDGET_TICKS {
                        self.event(AgentEvent::TextBox { message: format!(
                            "paced {tile_a}↔{tile_b} on {map} for {}s of game time with no encounter \
                             — re-deciding", PACING_BUDGET_TICKS as u64 * AGENT_RESOLUTION.to_duration().as_millis() as u64 / 1000) });
                        api.release_all_buttons();
                        self.set_state(AgentState::Idle);
                        return Ok(());
                    }
                    let pos = game_state.map.player_position;
                    let target = if *heading_to_b { tile_b } else { tile_a };
                    if pos == target {
                        *heading_to_b = !*heading_to_b;
                        *stalled = 0;
                    } else {
                        // Not there yet. Either we are mid-walk, or we are walking into something.
                        *stalled += 1;
                        if *stalled >= STALL_TICKS {
                            self.event(AgentEvent::TextBox { message: format!(
                                "pacing {tile_a}↔{tile_b} on {map} is blocked at {pos}; re-picking") });
                            api.release_all_buttons();
                            self.set_state(AgentState::Idle);
                            return Ok(());
                        }
                    }
                    let next = if *heading_to_b { tile_b } else { tile_a };
                    if let Some(dir) = dir_to(pos, next) {
                        api.release_all_buttons();
                        api.press_button(dir);
                    }
                }
            }
            AgentState::PokemartShopping(ref pokemart_state) => {
                let menu = api.menu_state();

                match pokemart_state {
                    // The shop is open and nobody has said what to buy. Hold it, pressing nothing,
                    // until the policy answers. See the ⚠️s on the variant.
                    PokemartState::AwaitingPolicy => {
                        api.release_all_buttons();
                        let game_state = api.game_state()?;
                        self.ask_mart_policy(&game_state, api);
                    }

                    // Keep navigating/pressing A on the Buy/Sell/Quit menu until the item list appears.
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
                                // Some other text box (e.g., greeting) — mash A.
                                api.toggle_button(JoypadButton::A);
                            }
                        } else {
                            api.toggle_button(JoypadButton::A);
                        }
                    }

                    // Navigate item list to target item, then press A to select it.
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
                                            // Clear stale wMaxItemQuantity==99 so AwaitingQtySelector
                                            // can detect the fresh write from pokemart.asm reliably.
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

                    // A was pressed on the target item; keep pressing A each tick until
                    // wMaxItemQuantity==99 (set by pokemart.asm right after item selection).
                    // DisplayListMenuID may still be initializing when the first A is sent, so we
                    // retry. As soon as we detect 99 we release all buttons in the same agent tick,
                    // so the qty selector's halt-based wait loop sees hJoyPressed=0 on its next
                    // VBlank (no rising edge while A is still held), preventing an auto-confirm.
                    PokemartState::AwaitingQtySelector(item) => {
                        if api.mart_in_quantity_selector() {
                            api.release_all_buttons();
                            self.set_pokemart_state(PokemartState::ChoosingQuantity { item: *item, qty_last: 0, stall_ticks: 0 });
                        } else {
                            api.toggle_button(JoypadButton::A);
                        }
                    }

                    // Adjust quantity with Up/Down then confirm with A.
                    // is_mart_item_list() is NOT used here because wTextBoxID stays ListMenuBox
                    // throughout the qty-selector phase; is_yes_no_menu() is the real signal.
                    PokemartState::ChoosingQuantity { item, qty_last, stall_ticks } => {
                        let item = *item;
                        let qty_last = *qty_last;
                        let stall_ticks = *stall_ticks;
                        // NLL: borrow of self.state via pokemart_state ends here (values copied)

                        let yes_no = menu.map_or(false, |m| m.is_yes_no_menu());
                        let cur_qty = api.mart_item_quantity();

                        if yes_no {
                            // Release all buttons before entering ConfirmingPurchase so the
                            // next gb.run has joypad=0, guaranteeing a fresh A rising edge
                            // for the YES confirmation (avoids a held-A false-no-edge).
                            api.release_all_buttons();
                            self.set_pokemart_state(PokemartState::ConfirmingPurchase(item));
                            return Ok(());
                        }

                        if cur_qty == 0 {
                            // Qty selector not yet initialized — wait.
                            return Ok(());
                        }
                        let target = item.quantity;

                        // Track consecutive ticks where wItemQuantity didn't change.
                        let (new_qty_last, new_stall_ticks) = if cur_qty == qty_last {
                            (qty_last, stall_ticks + 1)
                        } else {
                            (cur_qty, 0)
                        };
                        if let AgentState::PokemartShopping(PokemartState::ChoosingQuantity { qty_last: ref mut ql, stall_ticks: ref mut st, .. }) = self.state {
                            *ql = new_qty_last;
                            *st = new_stall_ticks;
                        }

                        // Stall AND at target → stuck in post-confirm price-text before Yes/No.
                        // Only mash A here; when qty != target we keep pressing Up/Down below.
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

                    // Select Yes on the Yes/No confirmation, navigate to YES and press A.
                    // On YES confirmed, transition to PurchasedItem to exit the post-purchase
                    // item list that .buyMenuLoop re-opens.
                    // Wrong qty (edge case): press B to cancel back to the item list.
                    PokemartState::ConfirmingPurchase(item) => {
                        if menu.map_or(false, |m| m.is_mart_item_list()) {
                            // B-cancel from Yes/No (wrong qty) sent us back to the item list.
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

                    // Purchase was confirmed; advance post-purchase text with B, then cancel
                    // the item list that .buyMenuLoop re-opens. If the yes/no box is still
                    // visible (HandleMenuInput.Delay3 hasn't read A yet), toggle A so the
                    // rising edge is detected correctly on the next joypad poll.
                    PokemartState::PurchasedItem { ticks } => {
                        let ticks = *ticks;
                        // NLL: borrow of self.state via pokemart_state ends here (value copied)
                        if menu.map_or(false, |m| m.is_mart_buy_sell_menu()) {
                            api.release_all_buttons();
                            // ⚠️ **One mart visit can be several purchases**, because Potions and
                            // Poké Balls are one errand and the model was paying two overworld turns
                            // and two mart turns for it. The Buy/Sell/Quit menu is already back on
                            // screen here, so the next order costs nothing but this branch.
                            // `next_mart_purchase` defaults to `None`, so every scripted policy
                            // still quits exactly here — see its ⚠️ for why re-asking
                            // `pick_mart_purchase` instead would double-buy.
                            let money = api.game_state().map(|s| s.money).unwrap_or(0);
                            match self.policy.next_mart_purchase().and_then(|item| affordable(api, money, item)) {
                                Some(item) => self.set_pokemart_state(PokemartState::ChoosingBuyOption(item)),
                                None => self.set_pokemart_state(PokemartState::Quitting),
                            }
                        } else if menu.map_or(false, |m| m.is_yes_no_menu()) {
                            // HandleMenuInput runs Delay3 (3 VBlanks ≈ 50ms) before reading
                            // joypad. Toggle A so hJoyLast[A] resets to 0 on one tick and
                            // produces a rising edge (hJoyPressed[A]=1 → YES) on the next.
                            api.toggle_button(JoypadButton::A);
                        } else {
                            // Post-purchase text or transitional state — pulse B to advance.
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
                                // Still in post-purchase text or a transition menu — press B.
                                api.toggle_button(JoypadButton::B);
                            }
                        } else {
                            api.toggle_button(JoypadButton::B);
                        }
                    }
                }
            }
            AgentState::TeachingMove { item, target_slot, press, entered_menu, settle, evolve_from } => {
                use crate::pokemon::menu::TextBoxId;
                let gs = api.game_state()?;
                // Done once the mon knows the move (teach) — or, for an evolution stone, once the slot's
                // species has changed away from `evolve_from` (the evolve animation/text has committed).
                // For a consumable used from the same menu chain that teaches no move and doesn't evolve
                // (a Rare Candy), "done" = the item has left the bag (been used up), detected once we've
                // opened the menu. HMs are never consumed, so this never fires for a real teach.
                let consumed = entered_menu && api.bag_item_position(item).is_none();
                let done = match evolve_from {
                    Some(from) => gs.pokemon.get(target_slot as usize).map_or(false, |p| p.species != from),
                    None => consumed || crate::pokemon::policy::hm_move(item).map_or(false, |mv|
                        gs.pokemon.get(target_slot as usize)
                            .map_or(false, |p| p.moves.iter().flatten().any(|m| m.name == mv))),
                };
                if done {
                    // Move learned / mon evolved — but Gen 1 returns to the bag/"use on which?" party menu
                    // after, so back out with B until a STABLE overworld. `settle` counts consecutive
                    // overworld ticks; if a menu re-appears it resets and B-mashing resumes. Requiring a
                    // sustained overworld avoids finishing on a one-tick flicker between closing menus
                    // (which would hand a still-open menu to the generic A-mash handler and re-use).
                    if game_mode != GameMode::Overworld {
                        api.release_all_buttons();
                        if press { api.press_button(JoypadButton::B); }
                        self.set_state(AgentState::TeachingMove { item, target_slot, press: !press, entered_menu, settle: 0, evolve_from });
                        return Ok(());
                    }
                    if settle < 15 {
                        api.release_all_buttons();
                        self.set_state(AgentState::TeachingMove { item, target_slot, press, entered_menu, settle: settle + 1, evolve_from });
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
                // Returned to the overworld without learning — this attempt fizzled (e.g. a mis-nav);
                // bail so `pick_field_move` re-issues TeachMove and we start the menu chain fresh.
                if entered_menu && game_mode == GameMode::Overworld {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                let entered_menu = entered_menu || game_mode != GameMode::Overworld;

                // Plain press/release "mashing" — a fresh rising edge every 2 ticks (see CuttingTree).
                if !press {
                    api.release_all_buttons();
                    self.set_state(AgentState::TeachingMove { item, target_slot, press: true, entered_menu, settle: 0, evolve_from });
                    return Ok(());
                }

                let (top_x, top_y, current, scroll) = api.menu_geometry();
                let tbid = api.menu_state().map(|m| m.text_box_id);
                let nav = |cur: u8, target: u8| -> JoypadButton {
                    if cur < target { JoypadButton::Down }
                    else if cur > target { JoypadButton::Up }
                    else { JoypadButton::A }
                };
                // Drive each menu of the teach chain to its target index, then confirm with A.
                let button = if game_mode == GameMode::Overworld {
                    JoypadButton::Start // no menu yet → open START
                } else if (top_x, top_y) == START_MENU_ORIGIN {
                    // ⚠️ Not a literal 2 — see `start_menu_row`. Without the Pokédex that index
                    // is the trainer card, which is a closed loop under A.
                    nav(current, start_menu_row(api, StartMenuRow::Item))
                } else if tbid == Some(TextBoxId::ListMenuBox) {
                    // Bag list → the item's row (absolute index = cursor + scroll), then A to select it.
                    // If the item is no longer in the bag (a consumable like a Rare Candy that was just
                    // used up), back out with B instead of selecting whatever sits at row 0.
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
                self.set_state(AgentState::TeachingMove { item, target_slot, press: false, entered_menu, settle: 0, evolve_from });
            }
            AgentState::CuttingTree { press, entered_menu, tree_pos, slot, move_index } => {
                use crate::pokemon::menu::TextBoxId;
                // A successful Cut opens the Pokémon menu, plays a fade/animation, then returns to the
                // overworld. So once we've entered a menu, the first return to the overworld means the
                // cut is done: record the tile (the ROM map still shows the tree) and finish. A failed
                // cut ("nothing to cut") stays in the menu, so it won't false-trigger this — and the
                // preconditions were verified before entering (facing a 0x3d tree, and a party that
                // can Cut at all).
                //
                // ⚠️ **That second precondition is two separate gates and both are outside this
                // file.** `MetaTileMap::can_cut` keeps `:CutTree` out of `actions()` and
                // `tools::hm_available` refuses a `use_field_move` that names it — because the party
                // menu a badgeless Cut opens has no CUT row in it, and this driver's only exit is
                // "we came back to the overworld", which never happens. What that costs when it is
                // not gated is sixty seconds of A-mashing per attempt, ended by
                // `DRIVER_ESCAPE_SILENCE` rather than by anything here.
                if entered_menu && game_mode == GameMode::Overworld {
                    self.cut_tiles.insert((api.game_state()?.map.map, tree_pos));
                    self.event(AgentEvent::TextBox { message: format!("Cut down the tree at {tree_pos}") });
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                let entered_menu = entered_menu || game_mode != GameMode::Overworld;

                // Plain press/release "mashing": press a button on a press tick, clear all on a
                // release tick. Each button therefore gives a fresh rising edge every 2 ticks, and a
                // held direction never carries into the next menu.
                if !press {
                    api.release_all_buttons();
                    self.set_state(AgentState::CuttingTree { press: true, entered_menu, tree_pos, slot, move_index });
                    return Ok(());
                }

                // In the overworld a Down/Up would move the player off the tree, so there we only open
                // the menu; from there the shared chain drives to CUT on the mon that knows it.
                let button = if game_mode == GameMode::Overworld {
                    JoypadButton::Start // facing the tree, no menu yet → open START
                } else {
                    field_move_menu_button(api, slot, move_index)
                };
                api.release_all_buttons();
                api.press_button(button);
                self.set_state(AgentState::CuttingTree { press: false, entered_menu, tree_pos, slot, move_index });
            }
            AgentState::Surfing { press, entered_menu, water_pos, slot, move_index, resume, settle } => {
                use crate::pokemon::menu::TextBoxId;
                // Mounting Surf opens the party menu, plays the field-move menu + a mount animation,
                // then returns to the overworld now surfing (the game auto-steps onto the water tile).
                // So once we've entered a menu, the first return to the overworld ends the mount: hand
                // back to the route follower (which walks the water if surfing, or re-derives/aborts if
                // the mount didn't take — we never spin in the menu).
                if entered_menu && game_mode == GameMode::Overworld {
                    api.release_all_buttons();
                    let mounted = api.mmu().read_pointer(&pokered_symbols::wWalkBikeSurfState) == 2;
                    // ⚠️ **A *sustained* overworld, for the reason `TeachingMove`'s `settle` gives.**
                    // `ItemUseSurfboard` sets the surf state, queues the simulated step onto the water
                    // and only then calls `PrintText`, and the screen lags RAM — so the overworld is
                    // reported for a tick or two before "<mon> got on!" is drawn. Leaving on that
                    // flicker hands the walk back with the box about to open on top of it, and a text
                    // box during a walk is an abort: the run paid for the mount and then paid again to
                    // be told the walk had stopped. Counting only fires on a mount that took, so a
                    // refusal still bails on the first overworld tick.
                    if mounted && settle < MOUNT_SETTLE_TICKS {
                        self.set_state(AgentState::Surfing {
                            press, entered_menu, water_pos, slot, move_index, resume, settle: settle + 1 });
                        return Ok(());
                    }
                    if mounted {
                        self.event(AgentEvent::TextBox { message: format!("Surfed onto the water at {water_pos}") });
                    }
                    // ⚠️ **Back to the walk, not to the policy** — see `Surfing::resume`. Only on a
                    // mount that took, and only on the map the walk was on: a mount that did not take
                    // has nothing to resume onto (the route's next step is still into water and the
                    // follower would re-enter this state for ever), and a changed map means the walk's
                    // destination is a tile on somewhere else — the mount's own step can cross a
                    // `ConnectionWater` seam, which is the walk arriving rather than being interrupted.
                    let same_map = api.game_state().map(|g| g.map.map);
                    match resume {
                        Some((destination, map)) if mounted && same_map == Ok(map) =>
                            self.set_state(AgentState::OverworldMovement { destination, map }),
                        _ => self.set_state(AgentState::Idle),
                    }
                    return Ok(());
                }
                // ⚠️ **A refused surf has to be *remembered*, or the same tile is chosen again.** "No
                // SURFing on <mon> here!" means the agent's map and the game disagree about that
                // tile — Saffron City has one the tile reader calls water — and nothing about
                // re-deciding changes the disagreement, so a random walker picks it again within the
                // minute. Blocking it is the same answer `handle_card_key_door` gives a door that will
                // not open, and it uses the same set, so `observe_state` overlays it as an obstacle
                // and the router (and the action list the policy is offered) stops naming it.
                //
                // ⚠️ It also has to be handled *here*, not left to the reader: this state drives its
                // own menus, so it is excluded from the text reader, and while the refusal box is up
                // the party-menu geometry is still what `field_move_menu_button` sees — it presses Up
                // and Down at a message box that only A or B can clear. That is the fainted-switch
                // lesson again, and without it the only thing that ends the attempt is
                // `DRIVER_ESCAPE_SILENCE`, a minute later.
                if api.on_screen_text(false).map_or(false, |t| t.contains("No SURFing")) {
                    let map = api.game_state().map(|g| g.map.map).unwrap_or(self.last_map.unwrap_or(Map::PalletTown));
                    self.blocked_tiles.insert((map, water_pos));
                    api.toggle_button(JoypadButton::B);
                    self.set_state(AgentState::Idle);
                    self.event(AgentEvent::TextBox {
                        message: format!("the game refused SURF at {water_pos}; treating it as land") });
                    return Ok(());
                }
                let entered_menu = entered_menu || game_mode != GameMode::Overworld;

                // ⚠️ **Nothing is pressed while the cartridge is moving the player.** The mount ends
                // in `.makePlayerMoveForward`, a simulated joypad step onto the water that runs in
                // `GameMode::Script`; the menu-navigation branch below reads `menu_geometry` and would
                // press Up/Down at a screen with no menu on it, and a held direction across a scripted
                // movement is the wedge `RunningScript` releases buttons for.
                if game_mode == GameMode::Script {
                    api.release_all_buttons();
                    self.set_state(AgentState::Surfing {
                        press: true, entered_menu, water_pos, slot, move_index, resume, settle: 0 });
                    return Ok(());
                }

                // Plain press/release mashing (see CuttingTree).
                if !press {
                    api.release_all_buttons();
                    self.set_state(AgentState::Surfing {
                        press: true, entered_menu, water_pos, slot, move_index, resume, settle: 0 });
                    return Ok(());
                }

                let button = if game_mode == GameMode::Overworld {
                    // No menu yet: face the water tile first (a walk step brought us adjacent but maybe
                    // facing another way), then open START. Pressing toward water while on foot only
                    // turns the player (water is impassable on foot), so this never moves us.
                    let gs = self.observe_state(api)?;
                    let facing_water = gs.map.tile_in_front().map(|(p, _)| p == water_pos).unwrap_or(false);
                    if facing_water {
                        JoypadButton::Start
                    } else {
                        dir_to(gs.map.player_position, water_pos).unwrap_or(JoypadButton::Start)
                    }
                } else {
                    // SURF is the surf mon's only field move, hence index 0.
                    field_move_menu_button(api, slot, move_index)
                };
                api.release_all_buttons();
                api.press_button(button);
                self.set_state(AgentState::Surfing {
                    press: false, entered_menu, water_pos, slot, move_index, resume, settle: 0 });
            }
            AgentState::UsingFieldMove { press, entered_menu, slot, move_index, from_map } => {
                use crate::pokemon::menu::TextBoxId;
                // A field move opens the party menu → field-move menu → the move, shows a confirmation
                // dialog ("… can now use STRENGTH!" / the Dig animation), then returns to the overworld
                // with the effect applied. So once we've entered a menu, the first return to the
                // overworld ends it — hand back to the policy, which re-checks the step's own condition
                // (`strength_active`, or the map having changed for Dig).
                //
                // DIG's whole effect *is* a warp, and it leaves a text box open on the far side. Waiting
                // for the overworld there means mashing buttons at whatever the player landed in front
                // of — on Cinnabar Island that is an NPC, whose dialogue then re-opens forever. So a
                // changed map ends this state too, and Idle's normal text handling takes over.
                let map_changed = api.game_state().map_or(false, |s| s.map.map != from_map);
                if entered_menu && (game_mode == GameMode::Overworld || map_changed) {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                let entered_menu = entered_menu || game_mode != GameMode::Overworld;

                // Plain press/release mashing (see CuttingTree).
                if !press {
                    api.release_all_buttons();
                    self.set_state(AgentState::UsingFieldMove { press: true, entered_menu, slot, move_index, from_map });
                    return Ok(());
                }

                let button = if game_mode == GameMode::Overworld {
                    JoypadButton::Start // no target tile — just open START
                } else {
                    field_move_menu_button(api, slot, move_index)
                };
                api.release_all_buttons();
                api.press_button(button);
                self.set_state(AgentState::UsingFieldMove { press: false, entered_menu, slot, move_index, from_map });
            }
            AgentState::TossingItem { item, press, entered_menu } => {
                use crate::pokemon::menu::TextBoxId;
                // Done once the item has left the bag. Gen 1 drops back to the bag list afterwards, so
                // back out with B until the overworld returns.
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
                // Back in the overworld with the item still held — the attempt fizzled; let the policy
                // re-issue it and start the menu chain fresh.
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
                    // ⚠️ Not a literal 2 — see `start_menu_row`. Without the Pokédex that index
                    // is the trainer card, which is a closed loop under A.
                    nav(current, start_menu_row(api, StartMenuRow::Item))
                } else if tbid == Some(TextBoxId::ListMenuBox) {
                    // Bag list → the item's row. This is also what the quantity selector reports, but
                    // the only items worth tossing are held singly, so its cursor cannot move anyway
                    // and the A press below confirms "1".
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
            AgentState::PushingBoulder { boulder, dir } => {
                let game_state = self.observe_state(api)?;
                // Any interruption (wild battle in the cave, a script/text box) — drop to Idle. The policy
                // decides the next push from the live positions, so partial progress is never lost.
                if game_state.mode != GameMode::Overworld {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                let map = &game_state.map;
                let boulder_at = |p: Point8| map.sprites.iter()
                    .any(|s| s.name.starts_with("Boulder") && !s.hidden && s.position == p);

                // Done the moment the boulder leaves its tile — it moved one step (into `boulder + dir`) or
                // fell through a hole. Either way this single push is complete; hand back to the policy.
                if !boulder_at(boulder) {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                // The tile the player must stand on to push: one step *behind* the boulder.
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
                    // Walk to the push tile. `route_to` is recomputed each tick and routes around the
                    // boulders (sprites/obstacles), so the approach never accidentally shoves one.
                    match map.route_to(behind).and_then(|r| r.first().copied()) {
                        Some(btn) => { api.release_all_buttons(); api.press_button(btn); }
                        None => { api.release_all_buttons(); self.set_state(AgentState::Idle); return Ok(()); }
                    }
                } else {
                    // Behind the boulder: face + hold the push direction. The game arms on the first
                    // attempt and advances the boulder one tile on the next; its dust animation then locks
                    // input, so we advance exactly one tile before the completion check above fires.
                    api.release_all_buttons();
                    api.press_button(dir);
                }
                self.set_state(AgentState::PushingBoulder { boulder, dir });
            }
            AgentState::UsingItemPc(s) => return crate::pokemon::postgame::item_storage::tick(self, api, s),
            AgentState::UsingPcBox(s) => return crate::pokemon::postgame::pc_box::tick(self, api, s),
            AgentState::UsingPartyScript(s) => return crate::pokemon::postgame::gifts::tick(self, api, s),
            AgentState::Flying(s) => return crate::pokemon::postgame::fly_bike::tick(self, api, s),
            AgentState::Fishing(s) => return crate::pokemon::postgame::fishing::tick(self, api, s),
            AgentState::SellingToMart(s) => return crate::pokemon::postgame::game_corner::sell_tick(self, api, s),
            AgentState::RedeemingPrize(s) => return crate::pokemon::postgame::game_corner::prize_tick(self, api, s),
            AgentState::UsingBagItem(s) => return crate::pokemon::postgame::items::tick(self, api, s),
            // Reserved seams (task 0.8) — one delegating line each; the bodies live with their owners.
            AgentState::CheckingTrashCan { target, checked, press, facing } => {
                // Checking a can triggers GymTrashScript, which prints a text box (leaving the
                // overworld). Once that has appeared (`checked`), the return to the overworld means
                // the check has resolved — finish so `pick_field_move` re-evaluates (advancing to the
                // second can, or popping the step once both locks are open).
                if checked && game_mode == GameMode::Overworld {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                if game_mode != GameMode::Overworld {
                    // The check's text box / script is up — advance it by mashing A.
                    api.release_all_buttons();
                    if press { api.press_button(JoypadButton::A); }
                    self.set_state(AgentState::CheckingTrashCan { target, checked: true, press: !press, facing });
                    return Ok(());
                }
                // In the overworld: route to a tile adjacent to the can and face it, then check it.
                let gs = self.observe_state(api)?;
                match gs.map.route_to_face_dir(target, facing).as_deref() {
                    Some([]) => {
                        // Adjacent and facing the can — mash A (clean press/release edges) to check it.
                        api.release_all_buttons();
                        if press { api.press_button(JoypadButton::A); }
                        self.set_state(AgentState::CheckingTrashCan { target, checked, press: !press, facing });
                    }
                    Some(&[btn, ..]) => {
                        // Walk/turn toward the can; hold the direction for continuous movement.
                        api.release_all_buttons();
                        api.press_button(btn);
                        self.set_state(AgentState::CheckingTrashCan { target, checked, press: true, facing });
                    }
                    _ => {
                        // No reachable tile adjacent to the target — abort so we don't spin forever.
                        //
                        // ⚠️ **W6 — this state had three callers and the message only ever knew
                        // about one.** `FieldMoveRequest::Interact` mapped onto
                        // `FieldMove::CheckTrashCan`, so a model that aimed `interact` at a hidden
                        // item, a statue or a door was told "Can't reach trash can at (23, 30)"
                        // wherever it was standing. Three of the ten issue reports the deployed run
                        // of 2026-09-02 filed in Cerulean City quote that line as proof the map
                        // model is broken, and Cerulean has no gym and no bin.
                        //
                        // ⚠️ **W10 then removed that caller**, so what is left is the gym-bin puzzle
                        // and the Mansion/Rocket switches, both of which do aim at a real object.
                        // The message stays specific anyway: the scripted policy can still reach
                        // here with a statue behind a gate that has not opened, and "trash can" on
                        // the Mansion B1F is the same lie in a quieter place.
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
                // Rode the elevator out — the map is no longer an elevator room. Done. (Any of the
                // game's elevators — Rocket Hideout, Silph Co, Celadon Mart — share this handler.)
                let in_elevator = matches!(gs.map.map,
                    Map::RocketHideoutElevator | Map::SilphCoElevator | Map::CeladonMartElevator);
                if !in_elevator {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                // Floor list-menu is up: navigate the cursor (wCurrentMenuItem) to `floor`, then A.
                // The elevator floor menu is a `SPECIALLISTMENU` (wListMenuID == 0x04) whose
                // `wTextBoxID` reads `MessageBox` (the "Which floor?" print), NOT `ListMenuBox` — so
                // detect it via the list-menu ID. Only while not yet selected: once picked, the menu
                // vars linger and re-driving them would trap us instead of walking out to the warp.
                const SPECIAL_LIST_MENU: u8 = 0x04;
                if !selected && api.list_menu_id() == SPECIAL_LIST_MENU {
                    // The floor menu scrolls (Silph Co has 11 floors), so the cursor position within
                    // the visible window (`current_item`) caps out — compare the *absolute* index
                    // (`current_item + scroll_offset`) against the target floor.
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
                // Non-overworld and not (yet) the floor list. Pulse A in both cases:
                //  - before selecting: the "Which floor?" message box precedes the SPECIALLISTMENU and
                //    waits for a button — pulse A to advance into the list.
                //  - after selecting: the pick's A-press can be swallowed if it lands mid-scroll, leaving
                //    the list menu open with the cursor already on the target floor; pulsing A re-confirms
                //    it (and is harmless during the subsequent shake / "arriving" text).
                if game_mode != GameMode::Overworld {
                    api.toggle_button(JoypadButton::A);
                    self.set_state(AgentState::UsingElevator { panel, floor, selected, press: true });
                    return Ok(());
                }
                if !selected {
                    // Face the panel and press A to open the floor menu.
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
                    // Floor picked and the menu redirected the exit warp — step onto it to ride out.
                    // (The static map still shows it leading back up, but stepping on fires the
                    // redirected warp; we finish on the map change at the top of this arm.)
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
                // Back in the overworld after entering the bag menus → this attempt has resolved (the
                // item's effect ran and, for the Poké Flute, its battle was fought). Bail to Idle so the
                // policy re-checks: target gone → the step pops; still present → it re-issues.
                if entered_menu && game_mode == GameMode::Overworld {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                // Still in the overworld: route to face the target sprite, then open the bag with START.
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
                // ⚠️ **The generic net for a refusal the table could not predict**, and the
                // reason it exists even though `item_use::field_use_refusal` already turns the
                // known ones away: a refusal can be *contextual*. `ItemUseEscapeRope` outside the
                // five tilesets in `EscapeRopeTilesets`, `ItemUseBicycle` indoors and a stone on
                // the wrong Pokémon are all `ItemUsePtrTable` entries that do something, and every
                // one of them ends at the same `ItemUseNotTime` and the same bag list with the
                // cursor untouched. `BattleState` has had this net since `soak` found the identical
                // loop inside a fight; this is the overworld half, and the same sentence.
                //
                // ⚠️ **Latched for [`BACKING_OUT_TICKS`], not a single B, and not a drop to
                // `Idle`.** One B dismisses the message onto the bag list, where the navigation
                // below would press A on the same row and start again. And dropping to `Idle` with
                // the bag still open hands a menu this driver opened to the generic reader, which
                // spends **33 s** of game time getting out of it and reports the whole screen it
                // walked through as one `TextBox` — measured, and it is where the deployed run's
                // "unrelated TM34/bag menu prompt" came from. B until the game is back in the
                // overworld, then the arm at the top of this state exits cleanly.
                //
                // ⚠️ **Reported once, on the tick the latch is set, and it quotes the outcome.**
                // What the model needs is the refusal; the alternative is the silence that reads as
                // a malfunction.
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
                // In the bag menus. Plain press/release mashing (fresh rising edge every 2 ticks).
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
                // Drive START → ITEM → (bag) item → USE. After USE the item's field effect starts a
                // battle (Snorlax); assert_battle_state then takes over, so here we just advance any
                // transitional / wake-up text with A.
                let button = if (top_x, top_y) == START_MENU_ORIGIN {
                    // ⚠️ Not a literal 2 — see `start_menu_row`. Without the Pokédex that index
                    // is the trainer card, which is a closed loop under A.
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
                /// Agent ticks (20 ms each) the *submitted* naming screen gets to close itself. A
                /// real one — grid, cry, "was transferred to BILL's PC!" — is well under a second of
                /// this; 30 s of game time is only reached when the screen was never there.
                const NAMING_BUDGET: u16 = 1500;
                if decided && ticks > NAMING_BUDGET {
                    // The screen has not closed and it is not going to. Hand back to the ordinary
                    // handlers rather than pulsing forever: the buffer still holds the name we wrote,
                    // so the (buffer-is-empty) naming detection cannot immediately re-fire.
                    self.event(AgentEvent::TextBox { message: format!(
                        "naming screen for {species:?} never closed in {NAMING_BUDGET} ticks; giving up") });
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                if decided {
                    // Buffer already written; keep pulsing START until DisplayNamingScreen
                    // exits (wFontLoaded → 0, so game_mode leaves TextBox/NamingScreen).
                    // Also stay while wIsInBattle is still 1 after a catch (game_mode stays
                    // WildBattle after the name is written but before EndOfBattle runs).
                    //
                    // ⚠️ **START submits the grid; it does not advance what comes after it.** Once the
                    // name is in, the game may still owe us text boxes, and `WaitForTextScrollButtonPress`
                    // only accepts **A or B** — a driver that keeps pulsing START there is deadlocked
                    // with the game, waiting for a mode change that its own input cannot cause. That is
                    // the "boxed catch wedges the agent" bug D reported and G-gifts could not reproduce
                    // (see §11 of the postgame plan): catching at party 6 sends the mon to the box, and
                    // `SendNewMonToBox` prints "<name> was transferred to BILL's PC!" *after* its
                    // `AskName` — a prompt that START cannot dismiss. A gift never hit it because
                    // `_GivePokemon`'s box branch has no trailing prompt.
                    //
                    // `wNamingScreenSubmitName` is the discriminator, not `game_mode`: writing the
                    // buffer directly stops the strict NamingScreen detection firing (it tests for an
                    // empty buffer), so from this state's point of view the mode reads `TextBox` both
                    // while the grid is still up and long after it is gone. The ROM's own submit flag
                    // does not have that ambiguity — 0 while the grid waits, 1 the moment it is taken.
                    let submitted = api.mmu().read_pointer(&pokered_symbols::wNamingScreenSubmitName) != 0;
                    api.toggle_button(if submitted { JoypadButton::A } else { JoypadButton::Start });
                    let still_in_naming = matches!(
                        game_mode,
                        GameMode::TextBox | GameMode::NamingScreen
                            | GameMode::WildBattle | GameMode::TrainerBattle
                    );
                    if !still_in_naming {
                        api.release_all_buttons();
                        self.set_state(AgentState::Idle);
                    } else {
                        self.set_state(AgentState::NamingPokemon { species, decided, ticks: ticks + 1 });
                    }
                } else {
                    // The only one of the five poll sites with no state of its own already in hand.
                    // Read one — but do not let a failed read become an error the other four cannot
                    // produce: the naming screen is the least ordinary place in the game to ask for a
                    // full `GameState`, and a policy that ignores tool calls (every policy today)
                    // must not start failing here.
                    if let Ok(game_state) = api.game_state() {
                        self.poll_policy(&game_state, api);
                    }
                    if let Some(decision) = self.policy.pick_nickname(species) {
                        // Write the nickname directly into the naming screen's string buffer,
                        // bypassing character-grid navigation.  The screen copies this buffer
                        // when START is pressed.  An empty/None nickname causes AskName to
                        // fall back to the default species name.
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

/// The tile one step from `from` in direction `btn` (None for a non-directional button or if the
/// step would underflow off the top/left edge).
fn step_pos(from: Point8, btn: JoypadButton) -> Option<Point8> {
    match btn {
        JoypadButton::Up    => (from.y > 0).then(|| Point8 { x: from.x, y: from.y - 1 }),
        JoypadButton::Down  => Some(Point8 { x: from.x, y: from.y + 1 }),
        JoypadButton::Left  => (from.x > 0).then(|| Point8 { x: from.x - 1, y: from.y }),
        JoypadButton::Right => Some(Point8 { x: from.x + 1, y: from.y }),
        _ => None,
    }
}

/// How much a menu-shape test is allowed to believe. See [`open_menu_on_screen`].
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum MenuEvidence {
    /// Only what is **drawn on the screen right now**.
    ///
    /// ⚠️ **`wTextBoxID` lingers, and on a rule that runs at every text box that is fatal.** It is
    /// written when a box is *drawn* and never cleared, so it goes on naming a menu that closed
    /// minutes and several maps ago. The hand-over rule armed on exactly that: the elevator's floor
    /// list left `ListMenuBox` behind, the agent then talked to the Silph worker, and B — correct
    /// for a list, an answer on a yes/no — said **no** to "Do you want to give a nickname to
    /// LAPRAS?". `a_full_party_sends_the_silph_lapras_to_the_box` is the test that caught it.
    /// The screen cannot lie the same way: closing a menu redraws it.
    OnScreen,
    /// The lingering RAM ids as well. Only safe once the silence is itself corroboration — see
    /// [`TEXT_BOX_ESCAPE_SILENCE`], where 30 s of the agent reaching no decision point at all has
    /// already ruled out an ordinary conversation.
    OrTheLingeringIds,
}

/// True when what is on screen is a **menu** — something the agent can leave with B — rather than a
/// conversation, which it has to confirm with A.
///
/// ⚠️ **`GameMode::TextBox` is not this test and must never be mistaken for it.** That is
/// `wFontLoaded`, which is equally true of every line of dialogue in the game; the first version of
/// the escape hatch effectively used it, pressed B on anything after 30 s of silence, and took
/// `full_playthrough` apart — the run blacked out in Mt Moon and sat on step 52 of 516. So this is
/// a list of *shapes*, and each one was paid for separately: a scrolling list, the party screen's
/// field-move box, a `CANCEL` entry (Bill's PC, whose prompt is an ordinary message box), and the
/// START menu (which advertises itself in neither `wTextBoxID` nor that word — see
/// [`is_start_menu`]).
///
/// ⚠️ **A yes/no is deliberately absent.** B there is an *answer*, not an exit: it takes the second
/// option. The reader confirms those with A and that is not this function's business to change.
///
/// The screen is decoded at most once. `on_screen_text` walks the tile map, and this runs on every
/// tick the agent has been silent past [`TEXT_BOX_ESCAPE_SILENCE`].
fn open_menu_on_screen(api: &PokemonApi<'_>, evidence: MenuEvidence) -> bool {
    if evidence == MenuEvidence::OrTheLingeringIds
        && api.menu_state().is_some_and(|m| m.is_list_menu() || m.is_field_move_menu()) {
        return true;
    }
    let (top_x, top_y, ..) = api.menu_geometry();
    let screen = api.on_screen_text(false).unwrap_or_default();
    screen.contains("CANCEL") || is_start_menu(top_x, top_y, &screen)
}

/// A row of the game's own START menu, named rather than numbered. See [`start_menu_row`] for why
/// the number is not a constant.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum StartMenuRow {
    Pokemon,
    Item,
}

/// The cursor index (`wCurrentMenuItem`) that selects `row` of the START menu.
///
/// ⚠️ **The menu is six rows before the Pokédex and seven after it, and the numbers are not the
/// same.** `DrawStartMenu` (`engine/menus/draw_start_menu.asm`) omits the POKéDEX row until
/// `EVENT_GOT_POKEDEX`, and `home/start_menu.asm`'s `.displayMenuItem` compensates at dispatch with
/// an `inc a`. So index 2 is ITEM *with* the Pokédex and the **player-name row** without it — which
/// is `StartMenu_TrainerInfo`, a screen `WaitForTextScrollButtonPress` leaves on A *or* B, straight
/// into `jp RedisplayStartMenu` with the cursor restored from `wBattleAndStartSavedMenuItem`.
/// Nothing in that cycle moves the cursor, so it is a closed loop under A.
///
/// That is not a corner: Oak's Parcel is delivered *before* the Pokédex, so **every run passes
/// through the window where the hardcoded 2 is wrong**. It wedged the deployed run in ViridianMart
/// for 55 minutes with the parcel still undelivered, the screen flashing white once a second as the
/// trainer card opened and closed (`GBPalWhiteOut` at each end of `StartMenu_TrainerInfo`).
///
/// ⚠️ **No test tier could have caught it.** `RandomPolicy` implements only `pick_overworld_action`
/// and `pick_battle_action`, so `soak` can never issue a field move and never enters the drivers
/// that navigate this menu; the leg chain and `full_playthrough` reach them only long after the
/// Pokédex. The pre-Pokédex window is reachable by an LLM policy and nothing else.
pub(crate) fn start_menu_row(api: &PokemonApi<'_>, row: StartMenuRow) -> u8 {
    let has_pokedex = api.mmu().read_has_pokedex();
    match row {
        StartMenuRow::Pokemon => u8::from(has_pokedex),
        StartMenuRow::Item => 1 + u8::from(has_pokedex),
    }
}

/// The party slot (0-based) of the first Pokémon that knows Surf — the mon the Surf-mount menu picks.
/// The button that advances the **field-move menu chain** — START → POKéMON → the mon in `slot` → the
/// field move at `move_index` — from whatever screen is currently up, plus A for the text in between.
///
/// Every field move is driven through the same three menus, so this is shared by `CuttingTree`,
/// `Surfing`, `UsingFieldMove` and [`crate::pokemon::postgame::fly_bike`]; the callers differ only in
/// their overworld behaviour (face a tree, face the water, just press START) and in the two indices.
/// Keeping it in one place is not only dedup: the field-move box needs a two-part test that took two
/// regressions to get right (see [`MenuState::is_field_move_menu`]), and four hand-copies of it drifted
/// apart the first time anything was learned about it.
///
/// The unambiguous menus are matched first, on geometry that each of them rewrites as it opens — the
/// START menu at (11,2), the party list at (0,1)/(0,3) — because `wTextBoxID` is *not* rewritten and can
/// still name a menu that closed minutes ago.
pub(crate) fn field_move_menu_button(api: &PokemonApi<'_>, slot: u8, move_index: u8) -> JoypadButton {
    let (top_x, top_y, current, _) = api.menu_geometry();
    let nav = |target: u8| -> JoypadButton {
        if current < target { JoypadButton::Down }
        else if current > target { JoypadButton::Up }
        else { JoypadButton::A }
    };
    if (top_x, top_y) == START_MENU_ORIGIN {
        // ⚠️ Not a literal 1 — see `start_menu_row`.
        nav(start_menu_row(api, StartMenuRow::Pokemon))
    } else if top_x == 0 && (top_y == 1 || top_y == 3) {
        nav(slot) // party menu → the mon that knows the move
    } else if api.menu_state().is_some_and(|m| m.is_field_move_menu()) {
        nav(move_index) // field-move box → the move itself
    } else {
        JoypadButton::A // transitional text ("used STRENGTH!", "can now use CUT!")
    }
}

fn surf_slot(state: &crate::pokemon::GameState) -> Option<u8> {
    crate::pokemon::policy::field_move_carrier(state, crate::pokemon::move_name::PokemonMoveName::Surf).map(|(slot, _)| slot)
}


/// Finds a grass tile orthogonally adjacent to `pos` in `map`, returning the first one found.
/// Two adjacent plain-floor tiles to pace between for cave encounters, the first of them next to
/// `pos`. Only `MetaTile::Empty` qualifies: water carries no encounters inside Seafoam, and warps,
/// ladders and holes would end the walk instead of continuing it. Tile-pair collisions (the Cavern
/// "cliffs") are honoured, so the pair is always genuinely walkable in both directions.
fn adjacent_pacing_pair(map: &crate::pokemon::tile_map::MetaTileMap, pos: Point8) -> Option<(Point8, Point8)> {
    let plain = |p: Point8| (p.x as usize) < map.width && (p.y as usize) < map.height
        && map.meta_tiles[p.x as usize + p.y as usize * map.width] == MetaTile::Empty;
    let neighbours = |p: Point8| [
        Point8 { x: p.x, y: p.y.saturating_sub(1) }, Point8 { x: p.x, y: p.y.saturating_add(1) },
        Point8 { x: p.x.saturating_sub(1), y: p.y }, Point8 { x: p.x.saturating_add(1), y: p.y },
    ].into_iter().filter(move |&n| n != p && plain(n) && !map.pair_blocked(p, n));
    neighbours(pos).find_map(|a| neighbours(a).find(|&b| b != pos).map(|b| (a, b)))
}

/// A grass tile next to `pos` that the player can actually **step onto**.
///
/// ⚠️ `pair_blocked` is not optional here, and leaving it out is a *silent* failure. A tile-pair
/// collision (a ledge, an elevation boundary) makes the move illegal, so the pacing driver presses
/// into it forever: the player never moves, the step counter never advances, and the ROM never rolls
/// for an encounter — a sweep or a grind that looks busy and does nothing at all, with no error and no
/// state change to log. Route 11's western grass runs right along a ledge row, which is where this was
/// found. Its sibling [`adjacent_pacing_pair`] had the check from the start.
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

/// True if the player is on the outermost row/column of the (expanded) map — i.e. an edge
/// warp/connection tile that fires by stepping off the map edge rather than by stepping on.
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

    /// ⚠️ **`impl Display for AgentEvent` is what the web UI puts in its log** — `host.rs` does
    /// `format!("{event}")` into `UiEventBody::Agent { text }` and the page prints it verbatim. It
    /// used to be `format!("battle: {action:?}")`, which put
    /// `Fight { slot: 1, battle_move: PokemonMove { name: Growl, pp: 40 } }` on screen for every
    /// turn of every battle. Nothing but this test looks at the prose, so nothing but this test
    /// would notice it turning back into a debug dump.
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
            say(BattleAction::UseItem { slot: 0, item: BagItem::new(ItemId::Potion, 1) }),
            "used Potion on BULBASAUR",
        );
        assert_eq!(
            say(BattleAction::Run),
            "BULBASAUR tried to run from Pidgey",
            "…tried: this is the action starting, not landing",
        );
        assert_eq!(say(BattleAction::SafariBall), "threw a Safari Ball at Pidgey");
    }

    /// The last line of a playthrough, and the one most likely to be read by someone who was not
    /// watching. Same contract as the battle prose above: the page prints this verbatim and
    /// `llm::prompt::describe_event` sends it to the model, so a `{:?}` here would tell the winner
    /// `HallOfFame { teams: 1, playtime: "06:12:44", .. }`.
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

    /// ⚠️ **A ball is thrown at the enemy, and every other bag item is used on your own Pokémon.**
    /// One arm covered both, so the log said a Poké Ball had been used *on the Pokémon throwing it*
    /// — which is not a wording problem but a wrong statement of what happened.
    #[test]
    fn a_ball_is_thrown_at_the_enemy_and_a_potion_is_not() {
        let say = |item| format!("{}", AgentEvent::BattleActionStarted {
            actor: "BULBASAUR".into(),
            opponent: "Pidgey".into(),
            action: BattleAction::UseItem { slot: 0, item: BagItem::new(item, 1) },
        });

        assert_eq!(say(ItemId::PokeBall), "threw a PokeBall at Pidgey");
        assert_eq!(say(ItemId::MasterBall), "threw a MasterBall at Pidgey");
        assert_eq!(say(ItemId::SuperPotion), "used SuperPotion on BULBASAUR");
        assert_eq!(say(ItemId::XAttack), "used XAttack on BULBASAUR", "an X item acts on your own side");
    }

    /// The abort reason is the most useful thing the agent says — it is what stops the model
    /// re-picking a route that cannot be walked — and it goes to the model as well as the page, so
    /// `NoRoute(Grass)` was costing both of them.
    #[test]
    fn an_abandoned_walk_says_why() {
        let event = AgentEvent::OverworldActionAborted {
            destination: MetaTile::Pc,
            reason: OverworldActionAbortedReason::Battle,
            at: None,
        };
        assert_eq!(format!("{event}"), "✗ gave up on the PC: a battle started");
        // ⚠️ **The square is the fact the model needs and the reason on its own was not.** The
        // deployed run gave up on `the way into Route2` 143 times with the bare reason and
        // nothing else, every one of them at the same tile — the Viridian old man, who blocks that
        // exit until Oak's Parcel is delivered. Working out that a repeated square means a blocked
        // route is deliberately the model's job; saying *which* square is ours.
        let blocked = AgentEvent::OverworldActionAborted {
            destination: MetaTile::Pc,
            reason: OverworldActionAbortedReason::Textbox,
            at: Some(Point8 { x: 19, y: 11 }),
        };
        assert_eq!(format!("{blocked}"), "✗ gave up on the PC at (19, 11): the game stopped you to say something");
        assert_eq!(
            format!("{}", AgentEvent::StartedOverworldAction { destination: MetaTile::Grass }),
            "→ heading for tall grass",
        );
    }

    /// **W4.** A walk given up on the 60 s bound and a walk whose action vanished are two different
    /// diagnoses, and the model acts on them differently — so they may not share a sentence.
    ///
    /// The deployed run of 2026-09-02 was told "there is no route to the warp to Route8Gate" while
    /// standing on the gate's own doorstep, oscillating between two tiles with a perfectly good
    /// route in hand, and went looking for a pathfinder bug. What it needed to read was that the
    /// walk had been abandoned, and where.
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
        // The house rule: this string reaches the model through `AgentEvent`'s `Display`.
        assert!(!said.contains('—'), "no em dashes in what the agent generates: {said}");
    }

    /// ⚠️ **A walk that does not say where it is going is the commonest line in the log and the
    /// least useful.** A random run's transcript was 23 × `→ heading for Warp`, 22 × `✓ reached
    /// Warp` and 14 × `→ heading for Sprite` — a third of everything the page showed, and not one of
    /// them said which warp, which map, or who was being talked to. The destination is a
    /// [`MetaTile`], so naming the target is that type's `Display` and this is what watches it.
    #[test]
    fn a_walk_says_where_it_is_going() {
        let started = |destination| format!("{}", AgentEvent::StartedOverworldAction { destination });
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

    /// ⚠️ **Talking to someone is that action *succeeding*.** It was reported as
    /// `OverworldActionAborted { reason: Textbox }` — "✗ gave up on Mom: it was interrupted" — for
    /// the whole of the run's history, which is not just a word: an abort reason is what the prompt
    /// calls the most useful thing the agent can tell the model, so every successful conversation
    /// was reported to it as a failed one.
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

    /// ⚠️ **The id a model quotes back is not the prose a viewer reads.** Both come off a
    /// `MetaTile`, and the moment `Display` started naming targets the ids would have become
    /// `PalletTown:5,11:the warp to OaksLab` — still self-consistent, so nothing would have failed,
    /// but every id in the transcript, in `Conversation.tsx` and in the model's own history would
    /// have changed shape.
    #[test]
    fn an_id_keeps_the_variant_name_the_prose_left_behind() {
        assert_eq!(MetaTile::Warp { to_map: Map::OaksLab, to_position: Point8 { x: 5, y: 11 } }.kind(), "Warp");
        assert_eq!(MetaTile::Sprite("Mom").kind(), "Sprite");
        assert_eq!(MetaTile::ConnectionWater(Map::Route20).kind(), "ConnectionWater");
        assert_eq!(MetaTile::Pc.kind(), "Pc");
    }

    /// ⚠️ **A person is named in the id; everything else is its variant.** "Sprite" is the
    /// emulator's word for a moving object on a screen and the model has no screen — it read as
    /// jargon, it was the same word for Professor Oak and for a boulder, and the menu row beside it
    /// then had to spend the name again to say who was there.
    ///
    /// The spaces go because the id is resolved by string equality: `"Middle Aged Woman"` in a key a
    /// model re-types is three chances to lose a walk to whitespace. `Display` keeps them, because
    /// that half is prose.
    #[test]
    fn a_person_is_named_by_their_id_and_not_called_a_sprite() {
        assert_eq!(MetaTile::Sprite("Mom").id_kind(), "Mom");
        assert_eq!(MetaTile::Sprite("Middle Aged Woman").id_kind(), "MiddleAgedWoman");
        assert_eq!(MetaTile::Sprite("Pokedex 1").id_kind(), "Pokedex1");

        // Every other variant is unchanged: the kind *is* what the model is choosing.
        assert_eq!(MetaTile::Warp { to_map: Map::OaksLab, to_position: Point8 { x: 5, y: 11 } }.id_kind(), "Warp");
        assert_eq!(MetaTile::Pc.id_kind(), "Pc");
        assert_eq!(MetaTile::Grass.id_kind(), "Grass");

        // …and the prose is untouched, spaces and all.
        assert_eq!(format!("{}", MetaTile::Sprite("Middle Aged Woman")), "Middle Aged Woman");
    }

    /// ⚠️ **A textbox is detected before its characters are drawn**, so the reader produces a stream
    /// of empty ones — on the deployed run they were most of the log, and they reach the transcript
    /// as well as the page. Dropped at the funnel every event goes through, so both are clean.
    #[test]
    fn an_empty_text_box_is_not_worth_saying() {
        assert!(!AgentEvent::TextBox { message: String::new() }.is_worth_reporting());
        assert!(!AgentEvent::TextBox { message: "   \n ".into() }.is_worth_reporting());
        assert!(AgentEvent::TextBox { message: "Wild PIDGEY appeared!".into() }.is_worth_reporting());
        // Nothing else is ever dropped: an event with no payload still says something happened.
        assert!(AgentEvent::BattleStarted.is_worth_reporting());
        assert!(AgentEvent::BattleEnded.is_worth_reporting());
    }

    /// Every variant has to produce *something* — an arm that fell through to an empty string would
    /// show as a blank row rather than as a failure.
    #[test]
    fn no_event_formats_to_nothing() {
        let events = [
            AgentEvent::StartedOverworldAction { destination: MetaTile::Pc },
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
