//! **W4 / §7.4–7.5** — the tool surface: what the model is offered, how a call is answered, and how
//! an answer becomes a decision.
//!
//! Two kinds of tool, and the difference is the whole turn contract:
//!
//! - **Read tools** answer a question and the turn continues. They are serviced at the policy poll
//!   from the [`observe`](crate::pokemon::observe) facade, against **one** `GameState`, so several
//!   reads in one assistant message cannot disagree with each other.
//! - **Terminal tools** end the turn. Exactly one per turn, and the array offered is **scoped to the
//!   decision kind being asked** — a battle turn is never sent `choose_action`, so the model cannot
//!   end a turn the wrong way because the wrong way is not there.
//!
//! ⚠️ **An id is never a list index.** `MetaTileMap::actions()` is `sort()`ed by `MetaTile` and can
//! reorder between the tick that rendered the menu and the tick the answer lands on;
//! `ConsolePolicy` learned this the hard way and matches by tile. The ids here are stable composite
//! keys — `"{map}:{x},{y}:{tile}"` for the overworld, `"fight:{move}"` / `"item:{item}"` /
//! `"switch:{slot}"` for a battle — and they are re-resolved against a **freshly recomputed** action
//! list at the moment the decision is applied. An id that no longer matches is a tool error fed back
//! to the model, never a panic and never a silent no-op.
//!
//! **W5** added the rest of it: `screenshot`, `press_buttons`, `use_field_move`, `set_nickname`,
//! `buy_item` and `forget_move`, and the three decision kinds the last three answer.

use serde_json::{Value, json};

use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::llm::prompt::ApiSnapshot;
use crate::llm::battle_script::MAX_SOURCE as MAX_BATTLE_SCRIPT;

/// How many *extra* kinds one `buy_item` may order in a single mart visit.
///
/// ⚠️ **Three, matching `MAX_CHAINED_ACTIONS`' tail rather than being reasoned out afresh.** A
/// stocked-up trip is Balls, Potions and an Antidote or two; past that the money is gone anyway, and
/// a long list is a long way for one mistyped name to waste.
const MAX_CHAINED_PURCHASES: usize = 3;
use crate::llm::todo::{MAX_ITEMS as MAX_TODO_ITEMS, MAX_TEXT as MAX_TODO_TEXT, TodoCall};
use crate::llm::protocol::{ToolCall, ToolSpec};
use crate::llm::worker::ToolAnswer;
use crate::pokemon::GameState;
use crate::pokemon::PokemonApi;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::agent::MANUAL_INPUT_CAPACITY;
use crate::pokemon::bag::BagItem;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::item::ItemId;
use crate::pokemon::map::Map;
use crate::pokemon::map_metadata::PlayerFacingDirection;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::observe;
use crate::pokemon::policy::{FieldMove, battle_options, field_move_index};
use crate::pokemon::world_graph::WorldGraph;

/// Which question the agent is asking. A turn is keyed by this, and a poll for a different kind
/// cancels the turn in flight (§7.2).
///
/// These are exactly the agent's five policy poll sites, and there is no sixth: `Policy` has
/// `pick_overworld_action`, `pick_battle_action`, `pick_nickname`, `pick_mart_purchase` and
/// `pick_move_to_forget`, and each of them is one question with one answer.
///
/// ⚠️ **`pick_field_move` is not a kind and must never become one.** It is called on every idle
/// overworld tick immediately before `pick_overworld_action`; given its own kind the two would
/// cancel each other fifty times a second and no turn would ever finish. A field move is one
/// possible *outcome* of an overworld turn ([`Terminal::UseFieldMove`]), not a turn of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecisionKind {
    Overworld,
    Battle,
    /// The naming screen is open for a Pokémon just caught, hatched or given.
    Nickname,
    /// A mart's Buy/Sell/Quit menu just opened.
    MartPurchase,
    /// "Which move should be forgotten?" — a level-up learn, or teaching an HM to a mon that already
    /// knows four moves.
    ///
    /// ⚠️ **This legitimately pre-empts `Battle`**: the prompt fires mid-fight through the agent's
    /// global handler, and the prompt is the live question. Cancelling the battle turn to answer it
    /// is correct, and a fresh battle turn starts afterwards.
    ForgetMove,
    /// **W9 / §14** — the sixth kind, and the only one that is not a poll site: the agent has
    /// reached *no* decision point for `GB_STUCK_TIMEOUT_SECS` of emulated time and the watchdog is
    /// asking on its behalf.
    ///
    /// It is the exception that proves the rule above. The other five are questions the agent knows
    /// how to carry out an answer to; this one is "the agent is wedged", so the only terminal tools
    /// are `press_buttons` — which goes round the state machine entirely — and `wait`.
    Stuck,
}

impl DecisionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overworld => "overworld",
            Self::Battle => "battle",
            Self::Nickname => "nickname",
            Self::MartPurchase => "mart",
            Self::ForgetMove => "forget-move",
            Self::Stuck => "stuck",
        }
    }

    /// Whether the `GameState` cannot tell that this is the question being asked, so the only
    /// evidence is which poll site ran last — see `LlmPolicy::observed_kind`.
    ///
    /// True of the three transient menu prompts (a naming screen, a mart's Buy/Sell menu and the
    /// forget-move prompt all look like an ordinary overworld or battle state) and of **W9's
    /// `Stuck`**, which looks like whatever the agent was doing when it wedged. Getting this wrong
    /// is not a wasted round trip but an infinite loop: every read batch cancelled, every turn
    /// restarted.
    pub fn is_inferred_from_the_site(self) -> bool {
        matches!(self, Self::Nickname | Self::MartPurchase | Self::ForgetMove | Self::Stuck)
    }
}

/// A terminal tool call, parsed. Resolving it against the live game is [`resolve_overworld`],
/// [`resolve_battle`] and [`resolve_field_move`] — done at the poll, not here, because the world may
/// have moved since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminal {
    /// One overworld action, and — since the whole point of the agent is that a decision is bigger
    /// than a button — optionally the next few after it.
    ///
    /// ⚠️ **`then` is not a route and must not be read as one.** Each id is resolved against the
    /// live game at the moment its turn comes round ([`resolve_overworld`]), exactly as `id` is, so
    /// a chain is a sequence of independent decisions taken without asking again rather than a plan
    /// the agent commits to. Anything that stops one stops the rest — see
    /// [`LlmPolicy::advance_queue`](crate::pokemon::llm_policy::LlmPolicy).
    ChooseAction {
        id: String,
        /// Further ids from the **same** turn's menu, in the order they are to be taken. At most
        /// [`MAX_CHAINED_ACTIONS`] counting `id` itself.
        then: Vec<String>,
        /// A battle interrupting this action does not end it: once the battle is over the same
        /// action is taken again rather than the decision being handed back. Off by default, and
        /// only ever a battle — see the policy for why no other interruption may resume.
        resume_after_battle: bool,
    },
    ChooseBattleAction { id: String },
    /// Something the agent does *without* walking: cut a tree, teach an HM, push a boulder. Stashed
    /// by the policy and handed to the agent at the next `pick_field_move`, which is the tick after.
    UseFieldMove(FieldMoveRequest),
    /// The escape hatch (§17 risk 1): raw joypad presses, delivered ahead of the state machine.
    PressButtons { buttons: Vec<JoypadButton> },
    SetNickname { name: Option<String> },
    BuyItem {
        item: Option<BagItem>,
        /// More kinds to buy in the **same** visit, in order. See `buy_item`'s description.
        then: Vec<BagItem>,
    },
    ForgetMove { slot: Option<u8> },
    /// Do nothing for this many agent ticks (20 ms of emulated time each). The honest answer when
    /// the game is mid-animation, and the forced answer when a model will not call anything else.
    Wait { ticks: u16 },
}

/// How many overworld actions one `choose_action` may carry, `id` included.
///
/// ⚠️ **The bound is how far ahead the menu stays true, not how much a model might want to queue.**
/// Every action changes the map the next id was minted against — a warp changes it entirely — so a
/// chain is only worth anything where each step is still on the menu after the one before it: heal
/// and then leave the Centre, pick the item up and then take the door. Four is about as far as that
/// holds, and a chain that over-reaches is not wrong so much as wasted: it stops at the first id
/// that no longer resolves and the model is told where it got to.
pub const MAX_CHAINED_ACTIONS: usize = 4;

/// A cap, because `wait { ticks: 100000 }` is a model stalling its own run and there is no legitimate
/// reason to sit out more than a few seconds of game time in one decision.
pub const MAX_WAIT_TICKS: u16 = 150;

/// The longest nickname the naming screen's buffer holds.
pub const MAX_NICKNAME: usize = 10;

/// What one call in an assistant message turned out to be.
pub enum CallKind {
    /// A read tool. Answer it at the policy poll and keep going.
    Read,
    /// `screenshot`. Answered by the **worker**, from the frame the host already published — see
    /// [`crate::llm::screenshot`]. It never reaches the emulator thread.
    Screenshot,
    /// **W6b** — a TODO operation. Answered by the worker too: none of it needs the emulator, so
    /// making it a batch for `service_tools` would cost a round trip for a file write.
    Todo(TodoCall),
    /// `report_issue`: the model believes the agent is wrong. Answered by the worker as well — it is
    /// a file write and a screenshot from the frame the host already published — and, like a todo
    /// call, it does **not** end the turn. See [`report_issue_spec`] for why that matters.
    Issue(String),
    /// A battle-script operation. Answered by the worker like a todo call, and like one it does
    /// **not** end the turn: setting a script is not a decision about the game.
    BattleScript(BattleScriptCall),
    /// The turn is over.
    Terminal(Terminal),
    /// Nothing this turn can use — an unknown name, a terminal tool belonging to the other decision
    /// kind, or arguments that would not parse. The string is the message the model is shown, and it
    /// is shown *as a tool result* so the turn can recover rather than being thrown away.
    Rejected(String),
}

impl CallKind {
    /// The discriminant, for the page. Not `strum`'s derive: `Todo(TodoCall)` and
    /// `Terminal(Terminal)` would drag their payloads' names into a string the client matches on,
    /// and these four words are a wire contract with `api.ts`.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Read | Self::Screenshot => "read",
            Self::Todo(_) => "todo",
            Self::Issue(_) => "issue",
            // ⚠️ Deliberately **not** a fifth word. `label` is a wire contract with `api.ts`, and a
            // battle-script call is a non-terminal side effect that reads back as a sentence —
            // exactly what the page already draws a `todo` row as.
            Self::BattleScript(_) => "todo",
            Self::Terminal(_) => "terminal",
            Self::Rejected(_) => "rejected",
        }
    }
}

// ── Field moves ──────────────────────────────────────────────────────────────────────────────────

/// A `use_field_move` call, parsed but not yet resolved. [`resolve_field_move`] turns one of these
/// into a [`FieldMove`] against the live state, because two of them need the party to do it.
///
/// **This is a chosen subset of [`FieldMove`], not all of it.** The variants left out —
/// `Fish`, `UseItemPc`, `UsePcBox`, `SellToMart`, `RedeemPrize`, `UsePartyScript`, `UseElevator` —
/// are postgame mechanisms whose arguments are internal types (a `PcBoxOp`, a `Prize`, a
/// `PartyScript`) rather than anything a model could name from what it is shown, and none of them is
/// on the path to the Hall of Fame. Anything genuinely unreachable is what `press_buttons` is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldMoveRequest {
    /// Cut the tree the player is **currently facing** — so this is the second half of a pair: walk
    /// to the tree with `choose_action`, then cut it.
    Cut,
    /// A field move used from the party menu: Strength, Flash, Dig, Teleport, Softboiled. `slot` is
    /// optional — without it, the first party member that knows the move is used.
    ///
    /// Surf is deliberately not here: the agent mounts it by itself the moment a route steps onto
    /// water, so asking for it explicitly is at best redundant.
    PartyMove { name: PokemonMoveName, slot: Option<u8> },
    Fly { to: Map },
    /// Teach an HM or TM in the bag to a party member.
    Teach { item: ItemId, slot: u8 },
    /// Use an evolution stone from the bag on a party member.
    Evolve { stone: ItemId, slot: u8 },
    /// Face `target` and use a bag item on it — the Poké Flute on a sleeping Snorlax, the Card Key
    /// on a door.
    UseItem { item: ItemId, target: Point8 },
    /// Throw an item away to free one of the bag's 20 slots.
    TossItem { item: ItemId },
    /// Shove a boulder one tile. Strength must already be armed (use it from the party menu first).
    PushBoulder { boulder: Point8, direction: JoypadButton },
    /// Rearrange the party so `slot` leads. Instant — the agent writes it straight to RAM.
    ReorderParty { slot: u8 },
    /// Face a tile and press A. Every "hidden object" in the game is this.
    ///
    /// ⚠️ **The ones a playthrough needs are in the action menu now** — `MetaTile::Switch`, the bins
    /// and the drink machines and the statues and the poster — so what is left for this is the
    /// hidden *items*, which are invisible by construction and cannot be offered as a row without
    /// giving them away. It stays the escape hatch for a hidden object the table has not got.
    Interact { target: Point8, facing: Option<PlayerFacingDirection> },
    /// Move a Pokémon between the party and the open PC box, or open a different box.
    ///
    /// ⚠️ **The agent walks to the PC itself**, from `pc_locations_for` — so unlike `Interact` this
    /// is not "press the thing at these coordinates" and needs no menu row. That is the whole
    /// difference between a PC and the hidden objects beside it: one press is the entire interaction
    /// for a statue, and for a PC the press is only the way in.
    UsePcBox { op: crate::pokemon::postgame::pc_box::PcBoxOp },
    /// Move items between the bag and PC item storage. Same walk, different menu.
    UseItemPc { op: crate::pokemon::postgame::item_storage::PcItemOp, item: ItemId, qty: u8 },
    /// Ride the lift the player is standing in to `to`.
    UseElevator { to: Map },
}

/// The five HM field moves and the badge each one needs before the game will let it be used outside
/// battle, transcribed from `.outOfBattleMovePointers` in `engine/menus/start_sub_menus.asm`: every
/// arm opens `bit BIT_<something>BADGE, a` / `jp z, .newBadgeRequired`. Dig, Teleport and Softboiled
/// are on that same table and take no badge, which is why they are not here.
const HM_BADGES: &[(PokemonMoveName, crate::pokemon::badge::Badge)] = &[
    (PokemonMoveName::Flash, crate::pokemon::badge::Badge::BoulderBadge),
    (PokemonMoveName::Cut, crate::pokemon::badge::Badge::CascadeBadge),
    (PokemonMoveName::Fly, crate::pokemon::badge::Badge::ThunderBadge),
    (PokemonMoveName::Strength, crate::pokemon::badge::Badge::RainbowBadge),
    (PokemonMoveName::Surf, crate::pokemon::badge::Badge::SoulBadge),
];

/// Whether `name` is one of the five HM moves, and which HM teaches it.
///
/// ⚠️ **The point of asking is that an HM move cannot be un-taught.** Gen 1 has no move deleter, and
/// the machine is a one-way write — so a Pokémon that forgets Cut cannot get it back, and the run is
/// behind the terrain that Cut clears until something else in the party learns it. `HM_BADGES` is the
/// list either way, so this and the field-move gate cannot drift apart.
pub fn hm_move(name: PokemonMoveName) -> Option<PokemonMoveName> {
    HM_BADGES.iter().find(|(hm, _)| *hm == name).map(|(hm, _)| *hm)
}

/// Refuse a field move the game itself would refuse, and say which half is missing.
///
/// ⚠️ **A refused field move is not a wasted turn, it is a wedged run.** Every one of these is
/// reached through the party menu, and pokered answers a missing badge with `.newBadgeRequired` →
/// `jp .loop` — back to the same menu, cursor untouched. The agent's driver is mashing A at that
/// point and has no exit condition but "we came back to the overworld", which never happens; the
/// only thing that ends it is `DRIVER_ESCAPE_SILENCE` sixty seconds later. The deployed run spent
/// eleven turns and two `report_issue` filings on exactly this, with no badges at all, and came away
/// believing the emulator was broken: *"since Cut pathing/tiles are buggy, we'll reset from a
/// different map position"*.
///
/// So it is checked here, in the turn, where a rejection is an ordinary tool result the model can
/// still act on — the same argument [`not_on_the_menu`] is built on.
fn hm_available(state: &GameState, name: PokemonMoveName) -> Result<(), String> {
    let Some(&(_, badge)) = HM_BADGES.iter().find(|(hm, _)| *hm == name) else { return Ok(()) };
    let known = (0..state.pokemon.len() as u8).any(|slot| knows(state, slot, name));
    match (known, state.badges.contains(badge)) {
        (true, true) => Ok(()),
        (false, true) => Err(format!(
            "No Pokémon in the party knows {name}, so it cannot be used. {name} is taught by an HM \
             you have to find first."
        )),
        (true, false) => Err(format!(
            "{name} needs the {badge} before the game will let it be used outside battle, and you \
             do not have it yet. Win the gym badge first."
        )),
        (false, false) => Err(format!(
            "{name} cannot be used yet: no Pokémon in the party knows it, and it also needs the \
             {badge}, which you do not have. Both have to come first."
        )),
    }
}

/// Turn a request into the [`FieldMove`] the agent executes, or into the sentence the model is told
/// instead. Everything that can be checked from the state is checked here rather than left to fail
/// silently three seconds later inside a menu driver.
/// The PC on the map the player is standing on, or the reason there is not one.
///
/// ⚠️ **The coordinate is never asked of the model.** `pc_locations_for` is a transcribed table
/// because a PC is a hidden event drawn as the wall it sits in, so a model naming coordinates would
/// be guessing at something it is shown nowhere; the agent walks there, faces up and presses A on
/// its own. Every Pokémon Centre has one in the same place.
fn the_pc_here(state: &GameState) -> Result<Point8, String> {
    crate::pokemon::tile_map::pc_locations_for(state.map.map).first().copied().ok_or_else(|| {
        format!(
            "There is no PC on {}. Every Pokémon Centre has one, and so does the player's bedroom.",
            state.map.map,
        )
    })
}

pub fn resolve_field_move(state: &GameState, request: &FieldMoveRequest) -> Result<FieldMove, String> {
    let party_slot = |slot: u8| -> Result<u8, String> {
        match (slot as usize) < state.pokemon.len() {
            true => Ok(slot),
            false => Err(format!(
                "There is no party member in slot {slot} — the party has {} (slots 0–{}).",
                state.pokemon.len(),
                state.pokemon.len().saturating_sub(1),
            )),
        }
    };
    let held = |item: ItemId| -> Result<ItemId, String> {
        match state.bag.iter().any(|entry| entry.id == item) {
            true => Ok(item),
            false => Err(format!("There is no {item} in the bag. `read_bag` lists what is there.")),
        }
    };

    Ok(match request {
        FieldMoveRequest::Cut => {
            hm_available(state, PokemonMoveName::Cut)?;
            // The driver cuts whatever is in front of the player, so a player facing anything else
            // walks into a menu it cannot use and comes back out having achieved nothing.
            match state.map.tile_in_front() {
                Some((_, crate::pokemon::tile::MetaTile::CutTree)) => FieldMove::CutTree,
                _ => {
                    return Err(
                        "Cut works on the tree the player is facing, and there is not one there. \
                         Use `choose_action` on a `:CutTree` entry in the action menu first — that \
                         walks up to a tree and faces it — then call `use_field_move` with `cut`."
                            .to_string(),
                    );
                }
            }
        }
        FieldMoveRequest::PartyMove { name, slot } => {
            hm_available(state, *name)?;
            let slot = match slot {
                Some(slot) => {
                    let slot = party_slot(*slot)?;
                    if !knows(state, slot, *name) {
                        return Err(format!("The Pokémon in slot {slot} does not know {name}."));
                    }
                    slot
                }
                None => match (0..state.pokemon.len() as u8).find(|&slot| knows(state, slot, *name)) {
                    Some(slot) => slot,
                    None => return Err(format!("No Pokémon in the party knows {name}.")),
                },
            };
            // ⚠️ The party menu lists a mon's field moves in **its own move-slot order**, so the
            // index depends on what else that mon knows. It is computed, never assumed.
            FieldMove::UseFieldMove { slot, move_index: field_move_index(state, slot, *name) }
        }
        FieldMoveRequest::Fly { to } => {
            hm_available(state, PokemonMoveName::Fly)?;
            FieldMove::Fly { to: *to }
        }
        FieldMoveRequest::Teach { item, slot } => {
            let item = held(*item)?;
            let slot = party_slot(*slot)?;
            // ⚠️ **A machine the game will refuse is the `CutTree` gate again, and it wedges the same
            // way.** `MonCannotLearnMachineMoveText` drops back to the party menu with the cursor
            // untouched (`engine/items/item_effects.asm`), and `TeachingMove`'s only exit is the mon
            // knowing the move, so the attempt ends 60 s later at `DRIVER_ESCAPE_SILENCE` and the
            // model, told only that the game stopped answering, asks for the identical teach again.
            // Refused here it costs no round trip and the answer names which slot to aim at instead.
            if state.pokemon.get(slot as usize)
                .is_some_and(|mon| !crate::pokemon::learnset::can_learn(mon.species, item)) {
                return Err(crate::pokemon::learnset::teach_refusal(state, item, slot));
            }
            FieldMove::TeachMove { item, target_slot: slot }
        }
        FieldMoveRequest::Evolve { stone, slot } => {
            let slot = party_slot(*slot)?;
            // Completion is "this slot's species changed", so the driver needs the species it
            // started from — which the model has no way to supply and no business supplying.
            let evolve_from = state
                .pokemon
                .get(slot as usize)
                .map(|mon| mon.species)
                .ok_or_else(|| format!("Slot {slot} is empty."))?;
            FieldMove::EvolveWithStone { stone: held(*stone)?, target_slot: slot, evolve_from }
        }
        FieldMoveRequest::UseItem { item, target } => {
            // ⚠️ **`CutTree`'s gate and `Teach`'s, for an item the game will not use at all.**
            // `UnusableItem` is `jp ItemUseNotTime` — "This isn't the time to use that!" and back to
            // the bag list, cursor untouched — and `UsingFieldItem`'s only completion is "we are in
            // the overworld again", which that never reaches, so the attempt is 60 s of A-mashing
            // ended by `DRIVER_ESCAPE_SILENCE`. The deployed run of 2026-08-27 alternated a
            // `use_item HelixFossil` with talking to the Mt Moon Rocket who says "if you find a
            // fossil, give it to me", which is flavour rather than a handoff. Refused here it costs
            // no round trip and the answer says the thing nothing else in the turn would: that the
            // item is carried rather than used, so there is nothing to retry.
            let item = held(*item)?;
            if let Some(refusal) = crate::pokemon::item_use::field_use_refusal(item) {
                return Err(refusal);
            }
            FieldMove::UseFieldItem { item, target: *target }
        }
        FieldMoveRequest::TossItem { item } => FieldMove::TossItem { item: held(*item)? },
        FieldMoveRequest::PushBoulder { boulder, direction } => {
            // Strength is not *used* on a boulder, it is armed once per map from the party menu and
            // then every push works — `BIT_STRENGTH_ACTIVE`, cleared on every map change. A push
            // before it is armed moves nothing and reports nothing, so the model retries the same
            // shove until something else stops it. Say which half is missing.
            hm_available(state, PokemonMoveName::Strength)?;
            if !state.strength_active {
                return Err(
                    "Strength is not armed on this map, so a boulder will not move. Use \
                     `use_field_move` with `strength` first — it has to be done again after every \
                     map change — then push."
                        .to_string(),
                );
            }
            FieldMove::PushBoulder { boulder: *boulder, dir: *direction }
        }
        FieldMoveRequest::ReorderParty { slot } => FieldMove::ReorderParty { slot: party_slot(*slot)? },
        FieldMoveRequest::Interact { target, facing } => {
            FieldMove::CheckTrashCan { target: *target, facing: *facing }
        }
        FieldMoveRequest::UsePcBox { op } => {
            let pc = the_pc_here(state)?;
            // ⚠️ **`blocked_by` is the game's own refusals, asked before a button is pressed.**
            // pokered answers every one of these with a message and a bounce straight back to the
            // Bill's-PC menu (`CantDepositLastMonText`, `BoxFullText`, `NoMonText`), from which a
            // driver that re-picked the same entry loops until `DRIVER_ESCAPE_SILENCE` — the closed
            // loop `MetaTile::Pc` and the TM learnset gate are both about. Asking here costs no
            // round trip and the reason is the one the model can act on.
            if let Some(refusal) = op.blocked_by(
                state.pokemon.len() as u8,
                state.boxed_pokemon.len() as u8,
                state.current_box,
            ) {
                return Err(format!("The game will not do that: {refusal}."));
            }
            FieldMove::UsePcBox { op: *op, pc }
        }
        FieldMoveRequest::UseItemPc { op, item, qty } => {
            use crate::pokemon::postgame::item_storage::PcItemOp;
            let pc = the_pc_here(state)?;
            if *qty == 0 {
                return Err("A quantity of 0 moves nothing.".to_string());
            }
            // Withdrawing reads PC storage rather than the bag, so `held` is the wrong question for
            // half of this and would refuse every withdrawal of something not also carried.
            if matches!(op, PcItemOp::Deposit) { held(*item)?; }
            FieldMove::UseItemPc { op: *op, item: *item, qty: *qty, pc }
        }
        FieldMoveRequest::UseElevator { to } => {
            let Some((panel, floors)) = crate::pokemon::tile_map::elevator_for(state.map.map) else {
                return Err(format!(
                    "There is no lift on {}. A lift is a room of its own, reached by a warp: the \
                     Rocket Hideout's, Celadon Mart's and Silph Co's are the three in the game.",
                    state.map.map,
                ));
            };
            let Some(floor) = floors.iter().position(|floor| floor == to) else {
                return Err(format!(
                    "This lift does not stop at {to}. It serves {}.",
                    floors.iter().map(|floor| floor.to_string()).collect::<Vec<_>>().join(", "),
                ));
            };
            // ⚠️ **The Rocket Hideout's lift is the one that needs a key**, and it is the one that
            // matters: `RocketHideoutElevatorText` opens `ld b, LIFT_KEY`, and B4F — Giovanni, so
            // the Silph Scope — is only reachable through it. Without the key the panel prints a
            // message and no menu opens, which the driver would sit through for a minute.
            if state.map.map == Map::RocketHideoutElevator {
                held(ItemId::LiftKey).map_err(|_| {
                    "This lift needs the LIFT_KEY, which is somewhere in the hideout. Without it \
                     the panel does nothing.".to_string()
                })?;
            }
            FieldMove::UseElevator { panel, floor: floor as u8 }
        }
    })
}

/// The first character of `name` the cartridge's charmap has no byte for, if there is one.
///
/// Asked by round-tripping through [`PokemonString`] rather than by listing the allowed characters a
/// second time — the same argument
/// `policy::every_name_a_policy_can_choose_is_one_the_game_will_take` is built on, and for the same
/// reason: `/` is a perfectly writable `$F3`, so "is it alphanumeric" is the wrong question.
fn unencodable(name: &str) -> Option<char> {
    name.chars().find(|c| {
        crate::pokemon::strings::PokemonString::from_string(&c.to_string()).0.first() == Some(&0x00)
    })
}

fn knows(state: &GameState, slot: u8, name: PokemonMoveName) -> bool {
    state
        .pokemon
        .get(slot as usize)
        .is_some_and(|mon| mon.moves.iter().flatten().any(|m| m.name == name))
}

/// The moves `use_field_move` accepts under [`FieldMoveRequest::PartyMove`], with what each one is
/// for. Also the tool description's own list, so the two cannot drift.
const PARTY_MOVES: &[(&str, PokemonMoveName, &str)] = &[
    ("strength", PokemonMoveName::Strength, "arm Strength so boulders can be pushed"),
    ("flash", PokemonMoveName::Flash, "light a dark map (Rock Tunnel)"),
    ("dig", PokemonMoveName::Dig, "warp straight out of a cave or dungeon"),
    ("teleport", PokemonMoveName::Teleport, "warp back to the last Pokémon Center"),
    ("softboiled", PokemonMoveName::Softboiled, "heal another party member from Chansey's HP"),
];

// ── The catalogue ────────────────────────────────────────────────────────────────────────────────

pub struct ReadTool {
    pub name: &'static str,
    pub description: &'static str,
    /// ⚠️ **Which turns this read is offered in, and it is not "all of them".** Every kind used to
    /// carry every read: a battle turn paid for `read_map`, a naming screen paid for the whole
    /// catalogue in order to answer with a word. Worse than the tokens is what an irrelevant tool
    /// invites — `read_battle` in the overworld can only ever answer `null`, and a model that calls
    /// it has spent a round trip finding that out.
    pub kinds: &'static [DecisionKind],
    /// `None` for the reads that take no arguments, which is all of them but [`READ_ROUTE`].
    pub parameters: Option<fn() -> Value>,
}

/// Non-terminal, callable any number of times within a turn. Most turns should need none of them —
/// the turn request already carries the situation (§7.1) — so these are for what does not fit or is
/// rarely wanted.
///
/// ⚠️ **Nothing here may duplicate the situation.** `read_screen_text` and `read_trainer` were both
/// deleted for it: the first answered from the very same `observe::screen_text` the turn already
/// renders under `### On screen`, and everything the second returned but the Pokédex counts was in
/// the turn's header. A read whose answer the model was already holding is a round trip bought for
/// nothing, and it teaches the model that reading is how a turn starts.
pub const READ_TOOLS: &[ReadTool] = &[
    ReadTool {
        name: "read_map",
        description: "A picture of the whole map, drawn from the game's own graphics: everyone \
                      where they stand and face, warps and map edges labelled with where they lead, \
                      unreachable ground dimmed, and a coordinate ruler along the top and left. It \
                      arrives as an image after the result, with everyone on the map and the warps \
                      as data. The actions you can take are in the turn's action menu, not here.",
        // Not in a battle: there is no map on screen and nothing on it can be acted on.
        kinds: &[DecisionKind::Overworld, DecisionKind::Stuck],
        parameters: None,
    },
    ReadTool {
        name: "read_party",
        description: "Every party member: species, nickname, level, HP, status, types, stats and \
                      all four moves with their remaining PP.",
        kinds: &[
            DecisionKind::Overworld,
            DecisionKind::Battle,
            DecisionKind::Nickname,
            DecisionKind::MartPurchase,
            DecisionKind::ForgetMove,
        ],
        parameters: None,
    },
    ReadTool {
        name: "read_pc",
        description: "What is in the PC: the Pokémon in the open box with the slot numbers \
                      `use_field_move` wants, what is in PC item storage, and which of the twelve \
                      boxes is open. Only the open box can be read.",
        kinds: &[DecisionKind::Overworld],
        parameters: None,
    },
    ReadTool {
        name: "read_bag",
        description: "Every item in the bag with its quantity and shop price, plus money and how \
                      many of the bag's 20 slots are used.",
        // The one read the situation genuinely cannot supply: the bag is nowhere in a turn request,
        // and `use_field_move` needs an item named exactly as the bag names it.
        kinds: &[DecisionKind::Overworld, DecisionKind::Battle, DecisionKind::MartPurchase],
        parameters: None,
    },
    ReadTool {
        name: "read_battle",
        description: "The live battle: both sides' species, level, HP, status, types and moves, the \
                      enemy's catch rate, and which of your moves Disable has locked out. The \
                      turn's battle menu already costs your moves against it; this is the detail.",
        // ⚠️ `ForgetMove` legitimately fires mid-fight, and which move to drop is a battle question.
        kinds: &[DecisionKind::Battle, DecisionKind::ForgetMove],
        parameters: None,
    },
    ReadTool {
        name: READ_ROUTE,
        description: "How to get somewhere you have already been. With `to`, the sequence of maps \
                      from here to that one, each saying which warp or edge of the map before it \
                      to leave by; without it, every map you have set foot on. It knows \
                      only what has been walked, so a map missing from it means 'not visited yet', \
                      never 'does not exist'.",
        kinds: &[DecisionKind::Overworld],
        parameters: Some(read_route_arguments),
    },
    ReadTool {
        name: READ_GUIDE,
        description: "The walkthrough for the stretch of the game you are in now: where to go, in \
                      order, what is blocking the way and what the next Gym Leader has. It is chosen \
                      from your badges and it does not change until you win the next one, so there \
                      is no reason to ask twice. Place names in it are spelled exactly as the \
                      action menu and `read_route` spell them.",
        // Overworld and Stuck only: it answers "where am I supposed to be going", which is not a
        // question a battle, a nickname, a mart or a move to forget can raise.
        kinds: &[DecisionKind::Overworld, DecisionKind::Stuck],
        parameters: None,
    },
    ReadTool {
        name: SCREENSHOT,
        description: "A picture of the Game Boy screen as it is right now. Everything the agent can \
                      read for you — the map, the party, the text on screen — is cheaper and more \
                      precise as one of the other reads; ask for this when you want to see \
                      something they do not model, such as an unfamiliar menu or an animation you \
                      are not sure has finished.",
        // Every kind: it is the only tool that can answer "what on earth is on screen", which is
        // exactly the question a nickname prompt, a mart menu or a wedged agent raises.
        kinds: &ALL_KINDS,
        parameters: None,
    },
];

/// Answered by the worker rather than at the policy poll, because PNG encoding does not belong on
/// the emulator thread. See [`CallKind::Screenshot`].
pub const SCREENSHOT: &str = "screenshot";

/// **The world graph, asked the question a model actually has.** It replaced `read_world_graph`,
/// which serialised every visited `(map, entry)` node with all of its edges — unbounded by
/// construction, and by the late game large enough to be a meaningful fraction of the window in a
/// single call. Nothing wanted the adjacency list; what a turn wants is "which way is Celadon", so
/// the routing runs here, where the graph already is, and what crosses into the context is the
/// answer.
pub const READ_ROUTE: &str = "read_route";

/// **The walkthrough, cut to where the player actually is.** Everything a run needs to know about
/// the order of this game is in [`crate::llm::guide`], and the chapter is picked from the badges the
/// turn is already reading — so the tool takes no arguments and cannot be asked the wrong question.
///
/// ⚠️ **It answers with markdown rather than JSON**, which is the one read that does. A chapter is
/// prose meant to be read, and `\n` escaping it into a JSON string makes it unreadable to a human
/// reviewing the turn for no gain to the model.
pub const READ_GUIDE: &str = "read_guide";

fn read_route_arguments() -> Value {
    json!({
        "type": "object",
        "properties": {
            "to": {
                "type": "string",
                "description": "A map to route to, e.g. `CeruleanCity`. Omit to list the maps you \
                                have visited.",
            }
        },
        "additionalProperties": false,
    })
}

/// Every [`DecisionKind`], for the reads that are offered in all of them — and, in the tests, so a
/// loop that meant "all of them" cannot quietly stop meaning it when a seventh is added.
pub const ALL_KINDS: [DecisionKind; 6] = [
    DecisionKind::Overworld,
    DecisionKind::Battle,
    DecisionKind::Nickname,
    DecisionKind::MartPurchase,
    DecisionKind::ForgetMove,
    DecisionKind::Stuck,
];

fn read_tool(name: &str) -> Option<&'static ReadTool> {
    READ_TOOLS.iter().find(|tool| tool.name == name)
}

fn reads_for(kind: DecisionKind) -> impl Iterator<Item = &'static ReadTool> {
    READ_TOOLS.iter().filter(move |tool| tool.kinds.contains(&kind))
}

// ── W6b: the plan (§10) ──────────────────────────────────────────────────────────────────────────

/// The two TODO tools, by name. Non-terminal like the reads, and named in the turn contract for the
/// same reason: a model that thinks `todo_set` ended its turn stops playing.
///
/// ⚠️ **There were four.** `memory_write` and `memory_read` sat beside these, doing the same job in
/// a different shape — see [`crate::llm::todo`]'s module docs for why one mechanism beat two. And
/// `todo_set` used to be `todo_add`, which could only append: revising a plan was a delete the
/// catalogue did not offer plus an add, so wrong items were completed — or kept — instead.
pub const TODO_TOOL_NAMES: &[&str] = &["todo_set", "todo_complete"];

/// Their specs. A function rather than a const because a JSON Schema is not a `const` expression.
pub fn todo_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "todo_set",
            format!(
                "Add, rewrite or delete one item on your plan: no `id` adds one on the end, an \
                 `id` rewrites that item where it is, an `id` with no `text` deletes it. The order \
                 is kept. Room for {MAX_TODO_ITEMS}, finished ones included. This is the only thing \
                 you write that outlives the conversation, so give the reason with the intent: \
                 `come back to Route 12 with the Poké Flute, the Snorlax blocks the path south`. At \
                 most {MAX_TODO_TEXT} characters."
            ),
            json!({
                "type": "object",
                "properties": {
                    // ⚠️ **The cap is in the schema as well as in the prose, and it was not.**
                    // `TodoList` truncates at `MAX_TEXT` on the way in, so an over-long item was
                    // silently cut mid-sentence — the exact schema-says-one-thing-parser-does-another
                    // shape that left 543 of 749 `why`s null. `maximum` on `id` is the same fix:
                    // the list can never hold more than `MAX_ITEMS`, so an edit past it was a round
                    // trip spent finding that out.
                    "id": { "type": "integer", "minimum": 1, "maximum": MAX_TODO_ITEMS,
                            "description": "An existing item's number, to replace or delete it. Omit to add." },
                    "text": { "type": "string", "maxLength": MAX_TODO_TEXT,
                              "description": "What to do, and why. Omit to delete `id`." },
                },
                "additionalProperties": false,
            }),
        ),
        ToolSpec::new(
            "todo_complete",
            "Mark one item on your plan done, by the number shown beside it.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "integer", "minimum": 1, "maximum": MAX_TODO_ITEMS,
                                        "description": "The item's number." } },
                "required": ["id"],
                "additionalProperties": false,
            }),
        ),
    ]
}

// ── The battle script ────────────────────────────────────────────────────────────────────────────

/// One tool call against the model's battle script, parsed. Answered on the **worker thread** —
/// validation runs the script over six hand-built states and none of it needs the emulator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BattleScriptCall {
    /// `get_battle_script_docs`: the API reference, verbatim.
    Docs,
    /// `read_battle_script`: what is installed, and whether it is armed.
    Read,
    /// `set_battle_script`: `None` unsets. Validated before it is armed.
    Set(Option<String>),
}

/// The three, by name. Non-terminal, so they are named in the turn contract beside the reads.
pub const BATTLE_SCRIPT_TOOL_NAMES: &[&str] =
    &["get_battle_script_docs", "read_battle_script", "set_battle_script"];

/// ⚠️ **Offered on `Overworld` and nowhere else, which is a scoping decision rather than an
/// oversight.** With a working script there *are* no battle turns to carry them on; when one fails,
/// the fallback battle turn is about winning the battle in front of you, not about writing code —
/// and the failure is waiting in the next overworld situation either way. It also keeps
/// `DecisionKind::Battle`'s array where it is, which had ~380 bytes of headroom.
fn offers_battle_script(kind: DecisionKind) -> bool {
    kind == DecisionKind::Overworld
}

/// Their specs. A function for the reason [`todo_tools`] is one.
pub fn battle_script_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "get_battle_script_docs",
            "How to write a battle script: the language, everything a script can read about the battle, and a worked example. Read this before `set_battle_script`.",
            no_arguments(),
        ),
        ToolSpec::new(
            "read_battle_script",
            "The battle script you have installed, and whether it is still deciding your battle turns. Every run starts on a default one that decides nothing, so this always has something to show you.",
            no_arguments(),
        ),
        ToolSpec::new(
            "set_battle_script",
            format!(
                "Install a script that decides your battle turns for you, in Rhai. A turn it answers costs no request at all, so a routine wild encounter becomes free. It is run against six example battles before it is installed and you are told what it chose in each; if it later fails it is disarmed and that turn comes back to you with the reason. Omit `script` to go back to the default, which hands you every turn. At most {MAX_BATTLE_SCRIPT} characters. Call `get_battle_script_docs` first."
            ),
            json!({
                "type": "object",
                "properties": {
                    "script": { "type": "string",
                                "description": "The script. Omit to go back to the default one." },
                },
                "additionalProperties": false,
            }),
        ),
    ]
}

fn classify_battle_script(name: &str, arguments: &Value) -> Option<CallKind> {
    let call = match name {
        "get_battle_script_docs" => BattleScriptCall::Docs,
        "read_battle_script" => BattleScriptCall::Read,
        // ⚠️ **`null` and an absent `script` mean the same thing and both have to work.** "Omit to
        // remove" is what the schema says, but a model that has just been told a script can be
        // removed writes `{"script": null}` at least as often.
        "set_battle_script" => BattleScriptCall::Set(
            arguments.get("script").and_then(Value::as_str).map(str::to_string),
        ),
        _ => return None,
    };
    Some(CallKind::BattleScript(call))
}

fn classify_todo(name: &str, arguments: &Value) -> Option<CallKind> {
    let call = match name {
        // `todo_add` is the old name, accepted so a resumed run imitating the calls in its own
        // history is serviced rather than lectured. It is not advertised, and both arguments are
        // optional here because the empty shapes get a better answer from the list itself.
        "todo_set" | "todo_add" => TodoCall::Set {
            id: arguments.get("id").and_then(Value::as_u64).map(|id| id.min(u64::from(u32::MAX)) as u32),
            text: arguments.get("text").and_then(Value::as_str).map(str::to_string),
        },
        "todo_complete" => match arguments.get("id").and_then(Value::as_u64) {
            Some(id) => TodoCall::Complete { id: id.min(u64::from(u32::MAX)) as u32 },
            None => return Some(CallKind::Rejected("`todo_complete` needs the item's `id`.".to_string())),
        },
        _ => return None,
    };
    Some(CallKind::Todo(call))
}

/// The `tools` array for one decision kind — §7.5's first line of defence.
pub fn for_kind(kind: DecisionKind) -> Vec<ToolSpec> {
    let mut tools: Vec<ToolSpec> = reads_for(kind)
        .map(|tool| {
            ToolSpec::new(tool.name, tool.description, tool.parameters.map_or_else(no_arguments, |f| f()))
        })
        .collect();
    tools.extend(todo_tools());
    if offers_battle_script(kind) {
        tools.extend(battle_script_tools());
    }

    match kind {
        DecisionKind::Overworld => {
            tools.push(ToolSpec::new(
                "choose_action",
                format!(
                    "ENDS THE TURN. Walk to and take one of the actions listed in the turn's action \
                     menu. `id` is the id from that menu, copied exactly — never a position in the \
                     list. `then` chains up to {} more ids from this same menu, taken in order \
                     without asking you again; that is worth doing where each is still true after \
                     the one before, as in heal then leave. It stops at the first that will not \
                     resolve or is stopped, and says where it got to. `resume_after_battle` is \
                     true unless you say otherwise: a battle interrupting the action does not end \
                     it, and it is taken up again after, up to {} times.",
                    MAX_CHAINED_ACTIONS - 1,
                    crate::pokemon::llm_policy::MAX_BATTLE_RESUMES,
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "An id from the action menu." },
                        "then": {
                            "type": "array",
                            "items": { "type": "string" },
                            "maxItems": MAX_CHAINED_ACTIONS - 1,
                            "description": "More ids from this menu, in order.",
                        },
                        "resume_after_battle": {
                            "type": "boolean",
                            "description": "Default true. False to be asked again instead.",
                        },
                    },
                    "required": ["id"],
                    "additionalProperties": false,
                }),
            ));
            tools.push(use_field_move_spec());
        }
        DecisionKind::Battle => {
            tools.push(ToolSpec::new(
                "choose_battle_action",
                "ENDS THE TURN. Take one of the actions listed in the turn's battle menu. `id` is the \
                 id from that menu, copied exactly.",
                json!({
                    "type": "object",
                    "properties": { "id": { "type": "string", "description": "An id from the battle menu." } },
                    "required": ["id"],
                    "additionalProperties": false,
                }),
            ));
        }
        DecisionKind::Nickname => tools.push(ToolSpec::new(
            "set_nickname",
            format!(
                "ENDS THE TURN. **Give this Pokémon a nickname**: one that says what you make of \
                 it — how you came by it, what you mean to use it for, what it reminds you of. It \
                 is what every message about it will call it from now on. Omit `name` only if \
                 nothing comes to mind. At most {MAX_NICKNAME} characters; letters, digits, spaces \
                 and `.,:;'-?!()[]/` only."
            ),
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "maxLength": MAX_NICKNAME,
                        "description": "The nickname you have chosen. Omit to keep the species name.",
                    }
                },
                "additionalProperties": false,
            }),
        )),
        DecisionKind::MartPurchase => tools.push(ToolSpec::new(
            "buy_item",
            "ENDS THE TURN. Buy from the mart, then leave. `item` is a name from the stock list, \
             copied exactly, and `quantity` how many. `then` buys more kinds in the same visit \
             without asking you again — Poké Balls and Potions in one stop rather than two. Omit \
             `item` to walk away without buying anything. Each order is trimmed to what the money \
             covers, in order, because Gen 1 sells you nothing at all for an order you cannot \
             afford; the row for each item says how many you already have.",
            json!({
                "type": "object",
                "properties": {
                    "item": { "type": "string", "description": "A name from the stock list." },
                    "quantity": {
                        "type": "integer", "minimum": 1, "maximum": 99, "default": 1,
                        "description": "How many of `item` to buy.",
                    },
                    "then": {
                        "type": "array",
                        "description": "More kinds to buy in this same visit, in order.",
                        "maxItems": MAX_CHAINED_PURCHASES,
                        "items": {
                            "type": "object",
                            "properties": {
                                "item": { "type": "string" },
                                "quantity": { "type": "integer", "minimum": 1, "maximum": 99, "default": 1 },
                            },
                            "required": ["item"],
                            "additionalProperties": false,
                        },
                    },
                },
                "additionalProperties": false,
            }),
        )),
        // **W9.** The one turn that offers `press_buttons` at all: there is no menu here, so
        // raw input and doing nothing are the whole of it. It used to be pushed by the `Overworld`
        // and `Battle` arms above as well; see [`press_buttons_spec`] for what that cost.
        DecisionKind::Stuck => tools.push(press_buttons_spec()),
        DecisionKind::ForgetMove => tools.push(ToolSpec::new(
            "forget_move",
            "ENDS THE TURN. Answer the 'which move should be forgotten?' prompt. `slot` is the move \
             slot to replace, from the list in the turn. Omit `slot` to decline the new move and \
             keep all four.",
            json!({
                "type": "object",
                "properties": {
                    "slot": { "type": "integer", "minimum": 0, "maximum": 3, "description": "The move slot to forget." },
                },
                "additionalProperties": false,
            }),
        )),
    }

    if offers_issue_report(kind) {
        tools.push(report_issue_spec());
    }

    tools.push(ToolSpec::new(
        "wait",
        format!(
            "ENDS THE TURN. Do nothing for `ticks` agent ticks (20 ms of game time each, so 50 is \
             one second) and then decide again. Use it when the game is mid-animation or mid-text \
             and the right move is to let it finish. Maximum {MAX_WAIT_TICKS}."
        ),
        json!({
            "type": "object",
            "properties": {
                "ticks": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_WAIT_TICKS,
                    "description": "How many 20 ms ticks to wait.",
                }
            },
            "required": ["ticks"],
            "additionalProperties": false,
        }),
    ));

    for tool in &mut tools {
        if terminal_names(kind).contains(&tool.function.name) {
            add_summary_argument(tool);
        }
    }
    tools
}

/// How long a turn summary may be. Long enough for the intent *and* the reason — "heading to
/// Viridian for Poké Balls, I have none and the grass north of here is where I can catch a second
/// mon" — and short enough that carrying one per turn for the length of a run is not what fills the
/// context window.
pub const MAX_SUMMARY: usize = 300;

/// How long `press_buttons`' `why` may be. Shorter than a summary on purpose: it answers one narrow
/// question — which action was looked for and not found — and a model given room to write an essay
/// there writes one instead of reconsidering.
pub const MAX_REASON: usize = 200;

/// Bolt a required `summary` onto a terminal tool's schema.
///
/// ⚠️ **This is the only thing the model says that survives its own turn.** A reasoning model's
/// thinking arrives on a channel of its own and is deliberately never sent back (it is billed as
/// completion tokens once, and a copy in the history pays for it again every turn afterwards), and
/// most models emit no `content` at all beside a tool call. So the assistant side of the history was
/// a column of bare JSON: what was done, never once why. A model reading that back has no record of
/// having *tried* anything, which is exactly the state in which it walks into the same building for
/// the fourth time.
///
/// It rides on the terminal call's own arguments rather than in a message of its own because that
/// is the one place a sentence can go that costs no extra round trip, cannot be separated from the
/// decision it explains, and lands in the history by itself — `Message::assistant` already carries
/// `tool_calls` verbatim, arguments included.
///
/// ⚠️ **Required in the schema *and* enforced by [`classify`], and the second half is new.** The
/// argument for tolerating its absence was that a rejected call does not end the turn — it becomes
/// another tool result and spends another of `GB_MAX_TOOL_STEPS`, so a model that forgot it would be
/// pushed towards the forced `wait` rather than towards remembering. What settled it was measuring
/// the cost rather than reasoning about it: across the deployed run's **2427 decisions only 98
/// carried no summary and every one was a `wait`** — the *synthesised* fallback wait, which never
/// reaches `classify`. The model already fills it in on every real action, so the rule costs that
/// model nothing; `press_buttons`' `why` is the counter-example that made it worth closing, having
/// been left null on 543 of 749 calls. See `call_summary` and
/// `tests::a_terminal_call_must_say_what_it_is_doing`.
fn add_summary_argument(tool: &mut ToolSpec) {
    let Some(properties) = tool.function.parameters.get_mut("properties").and_then(Value::as_object_mut)
    else {
        return;
    };
    properties.insert(
        "summary".to_string(),
        json!({
            "type": "string",
            "maxLength": MAX_SUMMARY,
            "description": "One or two sentences, in your own words, saying what you are doing and \
                            why. This is the only note you keep: your thinking is not retained, so \
                            on later turns this sentence is all you will have of this one. Say what \
                            you expect to happen, so a turn that did not work is one you can \
                            recognise instead of repeating.",
        }),
    );
    match tool.function.parameters.get_mut("required").and_then(Value::as_array_mut) {
        Some(required) => required.push(json!("summary")),
        None => {
            tool.function.parameters["required"] = json!(["summary"]);
        }
    }
}

/// The model's own account of a terminal call, if it gave one.
///
/// Trimmed and length-capped here rather than trusted: `maxLength` in a schema is a request, not a
/// guarantee, and this string goes to the page, the transcript and every later request.
pub fn call_summary(call: &ToolCall) -> Option<String> {
    call_string(call, "summary", MAX_SUMMARY)
}

/// `press_buttons`' `why`: what the model thinks is on the screen and what it is pressing at.
///
/// ⚠️ **Required in the schema and enforced by [`classify`]**, unlike the trade this used to make
/// with `call_summary`. Tolerating its absence was justified on the grounds that a rejection spends
/// another `GB_MAX_TOOL_STEPS` and pushes the model towards the forced `wait` — and it was measured
/// on the deployed run: **543 of 749 presses left it null**, so the record it exists to make
/// readable was three quarters blank. It is now asked on the single turn that offers the tool at
/// all, so the friction falls where a press is already the right answer.
pub fn call_reason(call: &ToolCall) -> Option<String> {
    call_string(call, "why", MAX_REASON)
}

/// One free-text argument, trimmed and length-capped rather than trusted: `maxLength` in a schema is
/// a request, not a guarantee, and these strings go to the page, the transcript, the record on disk
/// and every later request.
fn call_string(call: &ToolCall, field: &str, cap: usize) -> Option<String> {
    let value = call.arguments().ok()?.get(field)?.as_str()?.trim().to_string();
    if value.is_empty() {
        return None;
    }
    Some(match value.char_indices().nth(cap) {
        Some((cut, _)) => value[..cut].to_string(),
        None => value,
    })
}

/// A zero-parameter tool still needs a schema, and an empty object is what every endpoint accepts.
fn no_arguments() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

/// One tool for every non-walking field action, discriminated by `move`, because a dozen separate
/// tools would be a dozen entries in every request's `tools` array for the sake of one call a
/// hundred turns.
fn use_field_move_spec() -> ToolSpec {
    let party_moves: Vec<String> =
        PARTY_MOVES.iter().map(|(name, _, why)| format!("`{name}` — {why}")).collect();
    ToolSpec::new(
        "use_field_move",
        format!(
            "ENDS THE TURN. Do something that is not walking. `move` picks which, and decides which \
             of the other arguments are needed:\n\
             - `cut` — cut down the tree the player is **facing**. Walk to a `:CutTree` action first.\n\
             - {}. Each takes an optional `slot`; without one the first Pokémon that knows the move \
             is used.\n\
             - `fly` — fly to `map`, which must be a town you have already visited with a Pokémon \
             Center.\n\
             - `teach` — teach the HM or TM `item` to the Pokémon in `slot`.\n\
             - `evolve` — use the evolution stone `item` on the Pokémon in `slot`.\n\
             - `use_item` — face `target` and use bag `item` on it (the Poké Flute on Snorlax, the \
             Card Key on a door).\n\
             - `toss_item` — throw `item` away to free a bag slot. The bag holds only 20 kinds.\n\
             - `push_boulder` — shove the boulder at `target` one tile in `direction`. Strength must \
             be armed first.\n\
             - `reorder_party` — make the Pokémon in `slot` the party leader.\n\
             - `interact` — stand next to `target`, face it and press A. For a hidden item; the \
             bins, drink machines, statues and the poster are rows in the action menu.\n\
             - `pc_pokemon` — at a PC: `op` is `deposit` (party `slot` → box), `withdraw` or \
             `release` (a `box_slot`), or `change_box` (`box`, 1-12, which also saves the game). \
             Only the open box can be read; `read_pc` shows it.\n\
             - `pc_items` — at a PC: `op` `deposit` or `withdraw` moves `quantity` of `item` \
             between the bag and PC storage. The bag holds only 20 kinds.\n\
             - `elevator` — inside a lift, ride it to `map`. The three lifts are in the Rocket \
             Hideout, Celadon Mart and Silph Co.\n\
             Surf is not here — the agent mounts it by itself as soon as a route crosses water.\n\
             `cut`, `fly`, `strength` and `flash` each need a Pokémon taught that HM *and* a \
             particular badge; until you have both the game refuses them, and retrying will not \
             change it.",
            party_moves.join("\n- "),
        ),
        json!({
            "type": "object",
            "properties": {
                "move": {
                    "type": "string",
                    "enum": field_move_names(),
                    "description": "Which field action to take.",
                },
                "slot": { "type": "integer", "minimum": 0, "maximum": 5, "description": "A party slot, 0-based." },
                "item": { "type": "string", "description": "A bag item, named as `read_bag` names it." },
                "map": { "type": "string", "description": "A map name, for `fly` and `elevator`." },
                "target": {
                    "type": "object",
                    "properties": { "x": { "type": "integer" }, "y": { "type": "integer" } },
                    "required": ["x", "y"],
                    "additionalProperties": false,
                    "description": "A tile on the current map, in the coordinates `read_map` uses.",
                },
                "direction": {
                    "type": "string", "enum": ["up", "down", "left", "right"],
                    "description": "For `push_boulder`: which way to shove it.",
                },
                "facing": {
                    "type": "string",
                    "enum": ["up", "down", "left", "right"],
                    "description": "For `interact`: approach so the player ends up facing this way. \
                                    Rarely needed; the Pokémon Mansion switches want `up`.",
                },
                "op": {
                    "type": "string",
                    "enum": ["deposit", "withdraw", "release", "change_box"],
                    "description": "For `pc_pokemon` and `pc_items`: which way things move.",
                },
                "box_slot": { "type": "integer", "minimum": 0, "maximum": 19, "description": "A slot in the open box, 0-based." },
                "box": { "type": "integer", "minimum": 1, "maximum": 12, "description": "Which box to open, for `change_box`." },
                "quantity": { "type": "integer", "minimum": 1, "maximum": 99, "description": "How many, for `pc_items`. Default 1." },
            },
            "required": ["move"],
            "additionalProperties": false,
        }),
    )
}

fn field_move_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = PARTY_MOVES.iter().map(|(name, _, _)| *name).collect();
    names.extend([
        "cut", "fly", "teach", "evolve", "use_item", "toss_item", "push_boulder", "reorder_party",
        "interact", "pc_pokemon", "pc_items", "elevator",
    ]);
    names
}

/// ⚠️ **The last resort, and it is offered as one.** Raw presses pre-empt the whole state machine and
/// reset it to idle afterwards, so a model that reaches for this instead of the action menu will
/// walk the player into a wall. It exists because the action menu is the agent's model of the game
/// rather than the game — §17's risk 1 — and somewhere it is incomplete a raw button is the only way
/// through.
///
/// ⚠️ **`why` is friction, and that is the whole of its job.** Saying "a last resort" in prose was
/// not enough on the deployed run: the model pressed buttons on ordinary turns that had a perfectly
/// good menu. A required `why` was the next attempt, and it did not work either — **72% of 749
/// presses left it null**, because a field the schema calls required and the parser treats as
/// optional is a field a weak model simply omits.
///
/// ⚠️ **So the hatch is no longer offered on a turn that has a menu at all.** It survives only at
/// [`DecisionKind::Stuck`], which is the failure it was built for: the agent has reached no decision
/// point for `GB_STUCK_TIMEOUT_SECS`, there is no menu, and a raw press is the only thing that can
/// move the game. What replaced it on the two kinds that *do* have a menu is [`report_issue_spec`] —
/// which files the complaint and then makes the model choose an action anyway.
///
/// The deployed run is what settled it: 91 consecutive turns of `press_buttons` walking into ledges
/// on Route 3, while `Route3:0,10:Connection — walk into PewterCity` sat in the menu on every one of
/// them. The last `choose_action` before that run succeeded, so nothing had failed; the model had
/// simply learned to reach past the menu, and every turn of history it read back taught it again.
///
/// ⚠️ **`why` stays required and is now enforced** ([`call_reason`]): here it is asked on the one
/// turn where a press is the right answer, so a model that cannot say what it is pressing at is a
/// model whose press is worth refusing.
/// The name of the tool that replaced the escape hatch on every turn that has a menu.
pub const REPORT_ISSUE: &str = "report_issue";

/// How long an issue report may be. Longer than [`MAX_REASON`] and longer than [`MAX_SUMMARY`],
/// because unlike either of those it is not carried in the history or re-read every turn: it is
/// written to disk once and read by a person, so the only thing length costs is the completion that
/// wrote it.
pub const MAX_ISSUE: usize = 1_000;

/// `report_issue`: what the model says when it believes the *agent* is wrong.
///
/// ⚠️ **It does not end the turn, and that is the whole design.** `press_buttons` was reached for on
/// ordinary turns because it was the one way to finish a turn without choosing from the menu — so
/// the replacement is deliberately not a way to finish a turn at all. The model files the complaint
/// and then still has to call `choose_action`, `wait`, or whatever its kind offers. A terminal
/// issue report would be `press_buttons` with a different name and the same gravity.
///
/// What it buys over the old `why`: the reason used to ride on a call that had already decided to
/// bypass the agent, so filing one and doing something sensible were mutually exclusive. Now the
/// model can say "the menu will not let me do X" *and* try Y, which is the behaviour worth having.
///
/// ⚠️ **The message is enforced** — see [`classify`]. An issue with no message is not an issue, and
/// this is one of the two places where a rejection is worth the tool step it spends, because the
/// tool does nothing else: rejecting it costs the turn a round trip and costs the model nothing it
/// was going to do anyway.
fn report_issue_spec() -> ToolSpec {
    ToolSpec::new(
        REPORT_ISSUE,
        format!(
            "Report a problem with the agent itself: something it will not let you do, an action \
             menu that does not describe what is on the screen, a choice that keeps failing for a \
             reason you cannot see. **A developer reads these.** Write it as a bug report: what you \
             were trying to do, what you expected, and what happened instead. The game's state, the \
             screen and a save state are filed alongside it automatically, so describe rather than \
             transcribe. This does NOT end your turn and does NOT fix anything now: after filing \
             it, carry on and try a different way. At most {MAX_ISSUE} characters."
        ),
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "maxLength": MAX_ISSUE,
                    "description": "The report: what you tried, what you expected, what happened.",
                },
            },
            "required": ["message"],
            "additionalProperties": false,
        }),
    )
}

/// A `report_issue` call's message, trimmed and capped. Unlike [`call_summary`] and [`call_reason`]
/// this has no "absent" case to be tolerant of: [`classify`] rejects a call without one, so by the
/// time anything reads it there is a message.
pub fn issue_message(call: &ToolCall) -> Option<String> {
    call_string(call, "message", MAX_ISSUE)
}

fn press_buttons_spec() -> ToolSpec {
    ToolSpec::new(
        "press_buttons",
        format!(
            "ENDS THE TURN. Press these buttons in order, one at a time, then hand control back \
             to the agent. You are being offered this because the agent has stopped reaching \
             decision points on its own: there is no menu to choose from and a raw press is the way \
             out. Work out from the screen what is in front of you. B backs out of most menus and \
             closes most boxes; A advances text. Up to {MANUAL_INPUT_CAPACITY} presses; anything \
             past that is dropped."
        ),
        json!({
            "type": "object",
            "properties": {
                "buttons": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MANUAL_INPUT_CAPACITY,
                    "items": { "type": "string", "enum": ["up", "down", "left", "right", "a", "b", "start", "select"] },
                },
                "why": {
                    "type": "string",
                    "maxLength": MAX_REASON,
                    "description": "What you think is on the screen, and what these presses are \
                                    meant to do about it. Required: every press is recorded and \
                                    read afterwards by a person.",
                },
            },
            "required": ["buttons", "why"],
            "additionalProperties": false,
        }),
    )
}

/// Every tool that does **not** end a turn, *for this kind*: the reads this kind is offered, the
/// screenshot and W6b's TODO tools. The contract at the bottom of each turn names them all, because
/// a model that believes `todo_set` was its terminal call simply stops playing.
///
/// ⚠️ **Per kind, since the reads are.** A contract that named a read the request did not carry
/// would be inviting exactly the call `classify` has to reject.
pub fn non_terminal_names(kind: DecisionKind) -> Vec<&'static str> {
    reads_for(kind)
        .map(|tool| tool.name)
        .chain(TODO_TOOL_NAMES.iter().copied())
        .chain(offers_battle_script(kind).then(|| BATTLE_SCRIPT_TOOL_NAMES.iter().copied()).into_iter().flatten())
        .chain(offers_issue_report(kind).then_some(REPORT_ISSUE))
        .collect()
}

/// Which kinds can report that the agent itself is wrong.
///
/// The three that can be wedged by something the agent models badly: the two with an action menu,
/// where the complaint is "the menu does not describe this", and the watchdog's own turn, where the
/// complaint is the turn. The transient prompts — a naming screen, a mart, a forget-move — are a
/// single question with a single answer and nothing for the agent to get wrong.
pub fn offers_issue_report(kind: DecisionKind) -> bool {
    matches!(kind, DecisionKind::Overworld | DecisionKind::Battle | DecisionKind::Stuck)
}

pub fn terminal_names(kind: DecisionKind) -> &'static [&'static str] {
    match kind {
        DecisionKind::Overworld => &["choose_action", "use_field_move", "wait"],
        DecisionKind::Battle => &["choose_battle_action", "wait"],
        DecisionKind::Nickname => &["set_nickname", "wait"],
        DecisionKind::MartPurchase => &["buy_item", "wait"],
        DecisionKind::ForgetMove => &["forget_move", "wait"],
        // **W9.** There is no menu to choose from and no action the agent could execute, so the
        // escape hatch and doing nothing are the whole of it.
        DecisionKind::Stuck => &["press_buttons", "wait"],
    }
}

// ── Classification ───────────────────────────────────────────────────────────────────────────────

/// The complaint to make when the model chose an id this turn never offered, or `None` when it did.
///
/// ⚠️ **This is a *rejection*, not a terminal call, and that is the whole saving.** An id the menu
/// does not carry cannot resolve, so the alternative is a decision that is accepted here, published
/// as a `Decision`, sent to the policy, and refused by `resolve_overworld` — a turn paid for in full
/// that moves nothing. Caught here it costs one more completion inside the same turn, and the model
/// still gets to act. The deployed run spent **59 of 934 `choose_action` decisions** this way, every
/// one of them an id whose map was a map the player had already left.
///
/// ⚠️ **`resolve_overworld` stays the authority and this does not replace it.** The menu is minted
/// when the turn is built and re-minted when the answer lands; this catches what the model was never
/// *offered*, and that one still catches what stopped being true in between.
///
/// ⚠️ **An empty menu checks nothing.** `Nickname`, `ForgetMove` and `Stuck` have no menu at all, and
/// a check that treated "no menu" as "nothing is allowed" would reject every answer they give.
fn not_on_the_menu(id: &str, menu: &[String]) -> Option<String> {
    if menu.is_empty() || menu.iter().any(|offered| offered == id) {
        return None;
    }
    // ⚠️ Every overworld id is `{map}:{x},{y}:{kind}`, so the menu already names the map the player
    // is on and nothing has to carry it here separately — which is also what stops the complaint and
    // the situation disagreeing about where the player is. A battle menu's ids have no prefix, and
    // then this says nothing about maps at all: a battle id going stale is a different mistake.
    let here = menu[0].contains(':').then(|| menu[0].split(':').next()).flatten();
    let elsewhere = match (here, id.split_once(':')) {
        (Some(here), Some((named, _))) if named != here => format!(
            " That id is for `{named}` and you are in `{here}`; ids are minted for the map you are \
             standing on, so one you read on an earlier turn never resolves."
        ),
        _ => String::new(),
    };
    Some(format!(
        "`{id}` is not one of this turn's actions.{elsewhere} The ids that work are the ones in the \
         list you were given: {}. Pick one of those.",
        menu.iter().map(|offered| format!("`{offered}`")).collect::<Vec<_>>().join(", "),
    ))
}

/// Decide what a call is, without touching the game.
///
/// Everything recoverable becomes [`CallKind::Rejected`] carrying a sentence for the model, rather
/// than an error that ends the turn: a model that reaches for `choose_action` in a battle should be
/// told the battle menu is over there, not have its turn silently discarded.
///
/// `menu` is the list of ids the turn's situation offered, in the order it offered them — see
/// [`not_on_the_menu`]. Empty means "this kind has no menu", which is the three single-question
/// prompts and W9's `Stuck`, and then nothing is checked against it.
pub fn classify(kind: DecisionKind, call: &ToolCall, menu: &[String]) -> CallKind {
    match classify_call(kind, call, menu) {
        // ⚠️ **`summary` is now enforced, and the old doc argued it should not be.** That argument
        // was that a rejection does not end the turn — it spends another `GB_MAX_TOOL_STEPS` and
        // pushes a forgetful model towards the forced `wait`. What settled it is that the cost was
        // measured and is nearly zero: across 2427 turns of the deployed run only 98 decisions
        // carried no summary and **every one of them was a `wait`**, which is the *synthesised*
        // fallback wait and never goes through here at all. The model already fills it in on every
        // real action; enforcing it closes the hole for the model that would not.
        CallKind::Terminal(_) if call_summary(call).is_none() => CallKind::Rejected(format!(
            "`{}` needs a `summary`: one or two sentences saying what you are doing and why. Your \
             thinking is not kept, so it is the only note you will have of this turn. Call it again \
             with one.",
            call.function.name,
        )),
        classified => classified,
    }
}

fn classify_call(kind: DecisionKind, call: &ToolCall, menu: &[String]) -> CallKind {
    let name = call.function.name.as_str();
    // ⚠️ A read that exists but is not offered in *this* kind is answered like a terminal tool from
    // the wrong kind: named, with the reason. Falling through to "there is no tool called
    // `read_map`" would be a lie, and one a model in a battle could not act on.
    if let Some(tool) = read_tool(name) {
        if !tool.kinds.contains(&kind) {
            return CallKind::Rejected(format!(
                "`{name}` is not available in a {} turn. The reads you have here are {}.",
                kind.label(),
                non_terminal_names(kind).join(", "),
            ));
        }
        return match name == SCREENSHOT {
            true => CallKind::Screenshot,
            false => CallKind::Read,
        };
    }

    let arguments = match call.arguments() {
        Ok(arguments) => arguments,
        Err(failure) => {
            return CallKind::Rejected(format!(
                "{failure}. Send the arguments as a JSON object and try again."
            ));
        }
    };

    if let Some(todo) = classify_todo(name, &arguments) {
        return todo;
    }

    if BATTLE_SCRIPT_TOOL_NAMES.contains(&name) {
        // ⚠️ Named with the reason, like a read from the wrong kind above. A battle turn *can*
        // reach here — the script is disarmed mid-battle and the model reaches for the fix — and
        // "there is no such tool" would be a lie it cannot act on.
        if !offers_battle_script(kind) {
            return CallKind::Rejected(format!(
                "`{name}` is only offered on an overworld turn: mid-battle is not the moment to be                  writing one. Decide this turn, and set the script when you are back outside.",
            ));
        }
        if let Some(call) = classify_battle_script(name, &arguments) {
            return call;
        }
    }

    if name == REPORT_ISSUE {
        if !offers_issue_report(kind) {
            return CallKind::Rejected(format!(
                "`{REPORT_ISSUE}` is not available in a {} turn. The tools that do not end the turn \
                 are {}.",
                kind.label(),
                non_terminal_names(kind).join(", "),
            ));
        }
        // ⚠️ Enforced, unlike `summary` used to be. A report is *only* its message: rejecting an
        // empty one costs a tool step and loses nothing, where accepting it would write a directory
        // to disk saying a person should read nothing.
        return match issue_message(call) {
            Some(message) => CallKind::Issue(message),
            None => CallKind::Rejected(format!(
                "`{REPORT_ISSUE}` needs a `message` saying what you tried, what you expected and \
                 what happened. It does not end your turn: file it, then take an action."
            )),
        };
    }

    match name {
        "choose_action" if kind == DecisionKind::Overworld => match chosen_actions(&arguments, menu) {
            Ok(terminal) => CallKind::Terminal(terminal),
            Err(complaint) => CallKind::Rejected(complaint),
        },
        "choose_battle_action" if kind == DecisionKind::Battle => match string_argument(&arguments, "id") {
            Ok(id) => match not_on_the_menu(&id, menu) {
                None => CallKind::Terminal(Terminal::ChooseBattleAction { id }),
                Some(complaint) => CallKind::Rejected(complaint),
            },
            Err(complaint) => CallKind::Rejected(complaint),
        },
        "use_field_move" if kind == DecisionKind::Overworld => match field_move_arguments(&arguments) {
            Ok(request) => CallKind::Terminal(Terminal::UseFieldMove(request)),
            Err(complaint) => CallKind::Rejected(complaint),
        },
        // ⚠️ **`Stuck` only.** On a turn with a menu the answer is in the menu; the tool is not
        // in that request's catalogue and the fall-through arm below names what to call instead.
        "press_buttons" if kind == DecisionKind::Stuck => match button_arguments(&arguments) {
            Ok(buttons) => match call_reason(call) {
                Some(_) => CallKind::Terminal(Terminal::PressButtons { buttons }),
                // Enforced here and nowhere else in the catalogue's history: this is the one turn
                // that offers the hatch, so this is the one place the record can still be made
                // worth reading. See `press_buttons_spec`.
                None => CallKind::Rejected(
                    "`press_buttons` needs a `why`: what you think is on the screen, and what \
                     these presses are meant to do about it. Every press is filed and read."
                        .to_string(),
                ),
            },
            Err(complaint) => CallKind::Rejected(complaint),
        },
        "set_nickname" if kind == DecisionKind::Nickname => {
            // An absent `name` is the answer "keep the default", and so is an empty string — the
            // naming screen treats an empty buffer as a decline, so agreeing with it here means the
            // two cannot disagree.
            let name = arguments
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(|name| name.chars().take(MAX_NICKNAME).collect::<String>());
            // ⚠️ **The name is written straight into the naming screen's buffer, so nothing else
            // checks it.** `PokemonString::from_string` maps anything it does not know to `0x00` —
            // not the terminator (`0x50`), a control byte — so an accented letter or an emoji does
            // not fail, it writes a Pokémon whose name is garbage for the rest of the run. The
            // encodable set is exactly what the game's own naming screen offers, and a rejection
            // here costs one tool step and gets a real name.
            match name.as_deref().and_then(unencodable) {
                Some(bad) => CallKind::Rejected(format!(
                    "`{bad}` is not a character this game can write. A nickname may use letters, \
                     digits, spaces and `.,:;'-?!()[]/`, and nothing else — pick a name out of those \
                     and call `set_nickname` again."
                )),
                None => CallKind::Terminal(Terminal::SetNickname { name }),
            }
        }
        "buy_item" if kind == DecisionKind::MartPurchase => {
            let Some(name) = arguments.get("item").and_then(Value::as_str).filter(|n| !n.is_empty())
            else {
                return CallKind::Terminal(Terminal::BuyItem { item: None, then: Vec::new() });
            };
            let head = match purchase(name, arguments.get("quantity")) {
                Ok(item) => item,
                Err(failure) => return CallKind::Rejected(failure),
            };
            // ⚠️ **Every chained order is parsed here, before any of them happens** — the same rule
            // `chosen_actions` follows for `then`. A chain accepted by the parser and refused on the
            // third order has already spent the money on the first two, and reports the mistake in
            // the *next* turn's situation rather than as a tool result this turn can still act on.
            let mut then = Vec::new();
            if let Some(more) = arguments.get("then") {
                let Some(list) = more.as_array() else {
                    return CallKind::Rejected(
                        "`then` has to be a list of `{item, quantity}` objects.".to_string());
                };
                if list.len() > MAX_CHAINED_PURCHASES {
                    return CallKind::Rejected(format!(
                        "`then` takes at most {MAX_CHAINED_PURCHASES} more kinds and you gave {}. \
                         Nothing was bought — ask again with a shorter list.", list.len()));
                }
                for entry in list {
                    let Some(name) = entry.get("item").and_then(Value::as_str) else {
                        return CallKind::Rejected(
                            "Every entry in `then` needs an `item` name from the stock list.".to_string());
                    };
                    match purchase(name, entry.get("quantity")) {
                        Ok(item) => then.push(item),
                        Err(failure) => return CallKind::Rejected(failure),
                    }
                }
            }
            CallKind::Terminal(Terminal::BuyItem { item: Some(head), then })
        }
        "forget_move" if kind == DecisionKind::ForgetMove => {
            match arguments.get("slot").and_then(Value::as_u64) {
                None => CallKind::Terminal(Terminal::ForgetMove { slot: None }),
                Some(slot) if slot < 4 => CallKind::Terminal(Terminal::ForgetMove { slot: Some(slot as u8) }),
                Some(slot) => CallKind::Rejected(format!(
                    "There is no move slot {slot}; a Pokémon has four, numbered 0 to 3. Omit `slot` \
                     to decline the new move instead."
                )),
            }
        }
        "wait" => match arguments.get("ticks").and_then(Value::as_u64) {
            Some(ticks) => CallKind::Terminal(Terminal::Wait {
                ticks: ticks.clamp(1, u64::from(MAX_WAIT_TICKS)) as u16,
            }),
            None => CallKind::Rejected("`wait` needs a whole number of `ticks`.".to_string()),
        },
        // ⚠️ Not the generic sentence below. A model pressing buttons on a turn with a menu is
        // not confused about which kind of turn it is on; it is going round the agent. What it
        // needs is both halves: the action is in the menu, and if it really is not, say so.
        "press_buttons" => CallKind::Rejected(format!(
            "`press_buttons` is not available on a turn that has a menu; the agent presses the \
             buttons. Take one of: {}. If what you want genuinely is not in the menu, call \
             `{REPORT_ISSUE}` to say so and then take the closest action there is.",
            terminal_names(kind).join(", "),
        )),
        // A terminal tool from another decision kind. It exists, so saying "unknown tool" would be
        // actively misleading; what the model needs is the name of the one that does apply.
        "choose_action" | "choose_battle_action" | "use_field_move"
        | "set_nickname" | "buy_item" | "forget_move" => CallKind::Rejected(format!(
            "`{name}` is not available in a {} turn. End this turn with one of: {}.",
            kind.label(),
            terminal_names(kind).join(", "),
        )),
        other => CallKind::Rejected(format!(
            "There is no tool called `{other}`. The tools that do not end the turn are {}; end the \
             turn with one of: {}.",
            non_terminal_names(kind).join(", "),
            terminal_names(kind).join(", "),
        )),
    }
}

/// One `{item, quantity}` order, resolved against the game's item list.
fn purchase(name: &str, quantity: Option<&Value>) -> Result<BagItem, String> {
    let Some(item) = item_by_name(name) else {
        return Err(format!(
            "`{name}` is not an item this game has. Copy a name from the stock list exactly."));
    };
    let quantity = quantity.and_then(Value::as_u64).unwrap_or(1).clamp(1, 99) as u8;
    Ok(BagItem::new(item, quantity))
}

fn string_argument(arguments: &Value, key: &str) -> Result<String, String> {
    match arguments.get(key).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Ok(value.to_string()),
        _ => Err(format!("`{key}` is required and must be a non-empty string.")),
    }
}

// ── Parsing the awkward arguments ────────────────────────────────────────────────────────────────

/// A `choose_action` call: the id, whatever is chained behind it, and whether a battle ends it.
///
/// ⚠️ **Every id in the chain is checked against this turn's menu, on the same rule as `id`.** The
/// alternative — take them on trust and let the policy find out — spends the whole chain before the
/// mistake is visible and reports it a turn later, which is the exact shape
/// [`not_on_the_menu`] was written to close. Rejecting here costs one tool step and the model still
/// acts in this turn.
///
/// ⚠️ **Over-length is a rejection rather than a truncation.** Silently keeping the first three of
/// six would carry out half of something the model asked for whole and say so nowhere.
fn chosen_actions(arguments: &Value, menu: &[String]) -> Result<Terminal, String> {
    let id = string_argument(arguments, "id")?;
    if let Some(complaint) = not_on_the_menu(&id, menu) {
        return Err(complaint);
    }

    let malformed = || {
        "`then` is a list of further ids from this turn's action menu, as strings; omit it to take \
         one action."
            .to_string()
    };
    let then: Vec<String> = match arguments.get("then") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item.as_str().map(str::trim).filter(|next| !next.is_empty()) {
                Some(next) => match not_on_the_menu(next, menu) {
                    None => Ok(next.to_string()),
                    Some(complaint) => Err(complaint),
                },
                None => Err(malformed()),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(malformed()),
    };
    if then.len() + 1 > MAX_CHAINED_ACTIONS {
        return Err(format!(
            "That chains {} actions and one call may carry at most {MAX_CHAINED_ACTIONS}. Keep the \
             first {MAX_CHAINED_ACTIONS} and ask again once they are done; you will be shown a \
             fresh menu then anyway.",
            then.len() + 1,
        ));
    }

    Ok(Terminal::ChooseAction {
        id,
        then,
        // ⚠️ **Defaults to `true`, and it used to default to `false`.** A battle interrupting a
        // walk says nothing about the walk: the battle ends by itself, the world is where it was,
        // and the walk was going to be re-issued word for word. Left opt-in, the deployed run of
        // 2026-09-01 never once set it, so every wild encounter on a route cost a fresh overworld
        // turn *and* dropped the model back into a situation whose action it had already chosen —
        // which is where it loses track of what it was doing. The conservative direction is still
        // available and is now the thing that has to be asked for.
        //
        // ⚠️ **This changes what an *omitted* field means, so it is the one place a default can be
        // wrong in a way nothing else notices.** `MAX_BATTLE_RESUMES` and `drop_queue`'s rules are
        // unchanged: only a *battle* is ever resumed through, a text box or a guard still ends the
        // action, and the budget still runs out.
        resume_after_battle: arguments
            .get("resume_after_battle")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

fn field_move_arguments(arguments: &Value) -> Result<FieldMoveRequest, String> {
    let which = string_argument(arguments, "move")?;
    let which = which.trim().to_ascii_lowercase();

    let slot = || -> Result<u8, String> {
        match arguments.get("slot").and_then(Value::as_u64) {
            Some(slot) if slot < 6 => Ok(slot as u8),
            Some(slot) => Err(format!("There is no party slot {slot}; a party has 0 to 5.")),
            None => Err(format!("`{which}` needs a `slot` — which party member to use it on.")),
        }
    };
    let item = |key: &str| -> Result<ItemId, String> {
        let name = string_argument(arguments, key)?;
        item_by_name(&name)
            .ok_or_else(|| format!("`{name}` is not an item this game has. `read_bag` names them as they are spelled."))
    };
    let target = || -> Result<Point8, String> {
        let target = arguments
            .get("target")
            .ok_or_else(|| format!("`{which}` needs a `target` tile, as `{{\"x\": …, \"y\": …}}`."))?;
        let coordinate = |axis: &str| {
            target
                .get(axis)
                .and_then(Value::as_u64)
                .filter(|value| *value < 256)
                .ok_or_else(|| format!("`target.{axis}` must be a tile coordinate on the current map."))
        };
        Ok(Point8 { x: coordinate("x")? as u8, y: coordinate("y")? as u8 })
    };

    if let Some((_, name, _)) = PARTY_MOVES.iter().find(|(label, _, _)| *label == which) {
        let slot = match arguments.get("slot") {
            Some(Value::Null) | None => None,
            Some(_) => Some(slot()?),
        };
        return Ok(FieldMoveRequest::PartyMove { name: *name, slot });
    }

    match which.as_str() {
        "cut" => Ok(FieldMoveRequest::Cut),
        "fly" => {
            let name = string_argument(arguments, "map")?;
            map_by_name(&name)
                .map(|to| FieldMoveRequest::Fly { to })
                .ok_or_else(|| format!("`{name}` is not a map. `read_world_graph` lists the ones you know."))
        }
        "teach" => Ok(FieldMoveRequest::Teach { item: item("item")?, slot: slot()? }),
        "evolve" => Ok(FieldMoveRequest::Evolve { stone: item("item")?, slot: slot()? }),
        "use_item" => Ok(FieldMoveRequest::UseItem { item: item("item")?, target: target()? }),
        "toss_item" => Ok(FieldMoveRequest::TossItem { item: item("item")? }),
        "pc_pokemon" => {
            use crate::pokemon::postgame::pc_box::{PcBoxOp, BOX_CAPACITY, BOX_COUNT};
            let op = string_argument(arguments, "op")?;
            let box_slot = || -> Result<u8, String> {
                match arguments.get("box_slot").and_then(Value::as_u64) {
                    Some(slot) if (slot as usize) < BOX_CAPACITY => Ok(slot as u8),
                    Some(slot) => Err(format!("There is no box slot {slot}; a box holds {BOX_CAPACITY}.")),
                    None => Err(format!("`{op}` needs a `box_slot` — `read_pc` numbers them.")),
                }
            };
            Ok(FieldMoveRequest::UsePcBox {
                op: match op.trim().to_ascii_lowercase().as_str() {
                    "deposit" => PcBoxOp::Deposit { slot: slot()? },
                    "withdraw" => PcBoxOp::Withdraw { box_slot: box_slot()? },
                    "release" => PcBoxOp::Release { box_slot: box_slot()? },
                    "change_box" => match arguments.get("box").and_then(Value::as_u64) {
                        // 1-based for the model, because the game's own menu is; 0-based inside.
                        Some(n) if (1..=u64::from(BOX_COUNT)).contains(&n) => PcBoxOp::ChangeBox { n: n as u8 - 1 },
                        Some(n) => return Err(format!("There is no box {n}; there are {BOX_COUNT}.")),
                        None => return Err("`change_box` needs a `box`, 1 to 12.".to_string()),
                    },
                    other => return Err(format!(
                        "`{other}` is not a PC operation: deposit, withdraw, release or change_box.")),
                },
            })
        }
        "pc_items" => {
            use crate::pokemon::postgame::item_storage::PcItemOp;
            let op = string_argument(arguments, "op")?;
            Ok(FieldMoveRequest::UseItemPc {
                op: match op.trim().to_ascii_lowercase().as_str() {
                    "deposit" => PcItemOp::Deposit,
                    "withdraw" => PcItemOp::Withdraw,
                    other => return Err(format!("`{other}` is not a PC item operation: deposit or withdraw.")),
                },
                item: item("item")?,
                qty: match arguments.get("quantity").and_then(Value::as_u64) {
                    Some(qty) if (1..=99).contains(&qty) => qty as u8,
                    Some(qty) => return Err(format!("{qty} is not a quantity the game can move; 1 to 99.")),
                    None => 1,
                },
            })
        }
        "elevator" => {
            let name = string_argument(arguments, "map")?;
            map_by_name(&name)
                .map(|to| FieldMoveRequest::UseElevator { to })
                .ok_or_else(|| format!("`{name}` is not a map. The lift's own panel lists the floors it serves."))
        }
        "push_boulder" => {
            let direction = string_argument(arguments, "direction")?;
            Ok(FieldMoveRequest::PushBoulder {
                boulder: target()?,
                direction: button_by_name(&direction)
                    .filter(|button| {
                        matches!(button, JoypadButton::Up | JoypadButton::Down | JoypadButton::Left | JoypadButton::Right)
                    })
                    .ok_or_else(|| format!("`{direction}` is not a direction: up, down, left or right."))?,
            })
        }
        "reorder_party" => Ok(FieldMoveRequest::ReorderParty { slot: slot()? }),
        "interact" => Ok(FieldMoveRequest::Interact {
            target: target()?,
            facing: match arguments.get("facing").and_then(Value::as_str) {
                None => None,
                Some(facing) => Some(facing_by_name(facing).ok_or_else(|| {
                    format!("`{facing}` is not a direction: up, down, left or right.")
                })?),
            },
        }),
        other => Err(format!(
            "`{other}` is not one of the field moves. They are: {}.",
            field_move_names().join(", "),
        )),
    }
}

fn button_arguments(arguments: &Value) -> Result<Vec<JoypadButton>, String> {
    let list = arguments
        .get("buttons")
        .and_then(Value::as_array)
        .ok_or_else(|| "`press_buttons` needs a `buttons` array.".to_string())?;
    if list.is_empty() {
        return Err("`buttons` was empty, so nothing would have been pressed.".to_string());
    }
    list.iter()
        .map(|button| {
            let name = button.as_str().unwrap_or_default();
            button_by_name(name).ok_or_else(|| {
                format!("`{name}` is not a button: up, down, left, right, a, b, start or select.")
            })
        })
        // Silently dropping the tail would be a lie about what was pressed, so say so instead.
        .take(MANUAL_INPUT_CAPACITY)
        .collect()
}

/// Compare two names the way a model spells them against the way the code spells them: `"HM01 Cut"`,
/// `"hm01_cut"` and `"Hm01Cut"` are all the same item, and none of the three is worth a rejection.
fn same_name(a: &str, b: &str) -> bool {
    let normalise = |name: &str| -> String {
        name.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
    };
    normalise(a) == normalise(b)
}

/// ⚠️ `ItemId` has no `FromStr`, and giving it one would mean a `strum` derive on an enum three
/// hundred other lines already index by discriminant. Scanning 255 discriminants once per tool call
/// is free by comparison, and it cannot go stale.
pub fn item_by_name(name: &str) -> Option<ItemId> {
    (0..=u8::MAX).filter_map(ItemId::from_repr).find(|item| same_name(name, &item.to_string()))
}

pub fn map_by_name(name: &str) -> Option<Map> {
    use strum::IntoEnumIterator;
    Map::iter().find(|map| same_name(name, &map.to_string()))
}

fn button_by_name(name: &str) -> Option<JoypadButton> {
    use strum::IntoEnumIterator;
    JoypadButton::iter().find(|button| same_name(name, &button.to_string()))
}

fn facing_by_name(name: &str) -> Option<PlayerFacingDirection> {
    [
        PlayerFacingDirection::Up,
        PlayerFacingDirection::Down,
        PlayerFacingDirection::Left,
        PlayerFacingDirection::Right,
    ]
    .into_iter()
    .find(|facing| same_name(name, &facing.to_string()))
}

// ── Servicing a read ─────────────────────────────────────────────────────────────────────────────

/// Answer one read tool from the triple the policy holds at a poll.
///
/// ⚠️ Every call in a batch is answered from the **same** `state`, which is what makes `read_party`
/// and `read_map` in one assistant message guaranteed to agree.
pub fn service_read(
    call: &ToolCall,
    state: &GameState,
    api: &PokemonApi<'_>,
    graph: &WorldGraph,
) -> ToolAnswer {
    // ⚠️ **The picture is not drawn here.** `read_map` hands the worker the map it already has and
    // the worker renders it — see [`crate::llm::map_image`]'s module note on why a PNG encode must
    // not happen on the thread running the game. The clone is of a `MetaTileMap` the policy is
    // already cloning once per poll.
    let map = match call.function.name.as_str() {
        "read_map" => Some(Box::new(state.map.clone())),
        _ => None,
    };
    // ⚠️ Answered before the JSON arms below and returned as raw text: see [`READ_GUIDE`].
    if call.function.name == READ_GUIDE {
        return ToolAnswer::text(crate::llm::guide::chapter(state.badges));
    }
    let value = match call.function.name.as_str() {
        "read_map" => serde_json::to_value(observe::map_view(state)),
        "read_party" => serde_json::to_value(observe::party(state)),
        "read_bag" => serde_json::to_value(observe::bag(state, api)),
        "read_pc" => serde_json::to_value(observe::pc(state, api)),
        "read_battle" => serde_json::to_value(observe::battle(state)),
        READ_ROUTE => serde_json::to_value(route_answer(call, state, graph)),
        other => Ok(json!({ "error": format!("`{other}` is not a read tool") })),
    };
    match value.and_then(|value| serde_json::to_string(&value)) {
        Ok(json) => ToolAnswer { json, map, is_dark: state.map_is_dark },
        // Serialising a view cannot fail in practice, but a tool result is a string and the
        // alternative to this line is an `unwrap` on the worker's critical path.
        Err(failure) => ToolAnswer::text(
            format!("{{\"error\": \"could not encode the result: {failure}\"}}")),
    }
}

/// [`READ_ROUTE`], answered. Four outcomes, and each is a different thing for the model to do next,
/// which is why none of them is an `error` string:
///
/// - **no `to`** — the maps that have been walked, which is the only set `to` can be drawn from.
/// - **a name that is not a map** — a spelling problem, and correctable.
/// - **a map that has not been visited** — genuinely useful: the way there has to be *found*, not
///   recalled, and the graph is saying so rather than failing.
/// - **a route** — the maps in order, with how each one is entered.
fn route_answer(call: &ToolCall, state: &GameState, graph: &WorldGraph) -> Value {
    let requested = call
        .arguments()
        .ok()
        .and_then(|arguments| arguments.get("to").and_then(Value::as_str).map(str::to_string))
        .filter(|name| !name.trim().is_empty());

    let visited = || -> Vec<String> {
        observe::known_maps(graph).into_iter().map(|map| format!("{map}")).collect()
    };

    let Some(requested) = requested else {
        return json!({ "from": format!("{}", state.map.map), "visited": visited() });
    };
    let Some(to) = map_by_name(&requested) else {
        return json!({
            "to": requested,
            "error": format!("`{requested}` is not a map in this game. `{READ_ROUTE}` with no `to` \
                              lists the ones you have visited."),
        });
    };

    match observe::route(graph, state.map.map, to) {
        Some(hops) => json!({ "from": format!("{}", state.map.map), "to": format!("{to}"), "route": hops }),
        None => json!({
            "to": format!("{to}"),
            "reachable": false,
            "note": format!("You have not been to {to} — or no route to it crosses ground you have \
                             already walked. You will have to explore towards it. `{READ_ROUTE}` \
                             with no `to` lists where you have been."),
        }),
    }
}

// ── Menus and ids ────────────────────────────────────────────────────────────────────────────────

/// One row of the menu the turn request renders, and the only place an id is minted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub id: String,
    pub description: String,
}

/// The id of one overworld action: stable across a re-sort, unique within a map, and readable
/// enough that a model quoting it back is obviously quoting the right thing.
///
/// ⚠️ **`MetaTile::id_kind`, never its `Display`.** The `Display` is prose written for the status
/// log ("the warp to OaksLab") and is free to be reworded; an id is a key that a model quotes back
/// and that is re-resolved by string equality, so it takes the variant name — except for a person,
/// who is named instead of being called a "sprite". See `MetaTile::id_kind`.
///
/// ⚠️ **The map prefix looks redundant beside the turn's own header and is not.** `resolve_overworld`
/// re-mints ids against whatever map the player is on *now*, and the answer to a turn can land after
/// a warp — so without the prefix, `5,6:Warp` chosen in Oak's lab could match a warp that happens to
/// sit at (5, 6) in Pallet Town and be carried out silently. With it, a stale id simply fails to
/// resolve, which is a sentence the model is told.
pub fn overworld_id(state: &GameState, action: &OverworldAction) -> String {
    let destination = action.destination;
    format!("{}:{},{}:{}", state.map.map, destination.x, destination.y, action.tile.id_kind())
}

/// What one menu row says *beyond* its id: the action, in words.
///
/// ⚠️ **Not `OverworldAction`'s `Display`.** That prose is written for a person reading the SDL
/// console, names the tile and its coordinates, and beside an id that already ends in `:Warp` it
/// repeats the row's own key. This says what choosing the row *does* — `take the warp to
/// PalletTown`, `talk to Mom`, `pick up the Potion` — because a row that relied on the id for that
/// was misread: the deployed run took `` `MtMoonB2F:15,23:Rocket2` — 5 steps `` for a warp, forty-five
/// times. A person's name is therefore said once more here on purpose; it is the verb that matters.
///
/// ⚠️ **Whether a sprite is a person is decided by its `PictureId`, never by its name.** Every map
/// object the player can face is a sprite to the game — items on the ground, boulders, the fossils
/// in Mt Moon, the Pokédex in Oak's lab, Snorlax — and the name alone cannot tell `Potion1` from
/// `Hiker`. The picture can, for all of them, ahead of time.
///
/// ⚠️ **No distance, but the landing coordinate is back.** The step count was nine characters per
/// row for a number the model never used. The warp's `to_position` was dropped once as "a
/// coordinate on a map the model cannot see"; now that every map picture labels its warps with
/// their coordinates, it is the one thing that joins two floors — the ladder at `(17,11)` on 1F
/// comes out on the `(17,11)` plate of the B1F picture. ⚠️ It is shifted by the destination's
/// connection strips (`map_header::strip_offset`), so it is in that map's *picture* coordinates,
/// not the raw warp table's.
fn overworld_description(state: &GameState, action: &OverworldAction) -> String {
    use crate::pokemon::sprite::PictureId;
    use crate::pokemon::tile::{HiddenObject, MetaTile};
    match action.tile {
        MetaTile::Warp { to_map, to_position } => {
            let (dx, dy) = crate::pokemon::map_header::strip_offset(to_map);
            format!("take the warp to {to_map}, arriving at ({}, {})", to_position.x + dx, to_position.y + dy)
        }
        MetaTile::Connection { to_map, .. } => format!("walk into {to_map}"),
        MetaTile::ConnectionWater(to_map) => format!("surf into {to_map}"),
        MetaTile::Grass => "walk into tall grass to find wild Pokémon".to_string(),
        MetaTile::Pc => "use the PC".to_string(),
        // One press of A each; what it does is the cartridge's business. The verb has to carry the
        // *point* rather than the mechanism, because these are the rows a model has no prior idea
        // exist — a bin it is told to search gets searched, a bin it is told it can press does not.
        MetaTile::Switch { object, .. } => match object {
            HiddenObject::TrashCan => "search this bin for one of the gym's two switches",
            HiddenObject::VendingMachine => "buy the cheapest drink from this machine",
            HiddenObject::Poster => "look behind the poster",
            HiddenObject::Statue => "press this statue's switch",
            HiddenObject::CellSeparator => "run the cell separator to turn Bill back into a person",
        }.to_string(),
        MetaTile::CutTree => "walk up to a tree that Cut can clear".to_string(),
        // The row only exists when a rod is in the bag and this map has water to cast at, so what it
        // needs to say is what fishing is *for* rather than that it is possible: a wild battle with
        // something that lives in the water, without walking anywhere.
        MetaTile::Fish { rod } => format!(
            "fish at the water's edge with the {} to find wild water Pokemon", rod.name()),
        MetaTile::Sprite(name) => {
            let picture = state.map.sprites.iter().find(|s| s.name == name).map(|s| s.picture_id);
            match picture {
                Some(PictureId::PokeBall) => format!("pick up the {}", name.trim_end_matches(|c: char| c.is_ascii_digit()).trim()),
                Some(PictureId::Boulder) => "walk up to a boulder that Strength can push".to_string(),
                Some(PictureId::Fossil | PictureId::OldAmber | PictureId::UnusedOldAmber) => format!("examine the {name}"),
                Some(PictureId::Paper | PictureId::Pokedex | PictureId::Clipboard) => format!("read the {name}"),
                Some(PictureId::Snorlax) => "walk up to the sleeping Snorlax blocking the way".to_string(),
                _ => format!("talk to {name}"),
            }
        }
        other => format!("walk to {other}"),
    }
}

/// Everything reachable from where the player is standing. Sorted, so two reads of an unchanged map
/// produce the same menu — `actions()` walks a `HashSet` and would otherwise reshuffle, which reads
/// to a model as the world having moved.
///
/// `arrival` marks the way back: the warp the player came in by says so on its row, and so does a
/// map edge leading to the map they came from. On a floor with three ladders all `to MtMoonB1F`
/// it is the only thing that tells the one just climbed from the two not yet tried.
pub fn overworld_menu(state: &GameState, arrival: Option<crate::pokemon::world_graph::Arrival>) -> Vec<MenuItem> {
    use crate::pokemon::tile::MetaTile;
    let arrival = arrival.filter(|a| a.map == state.map.map);
    let way_back = |action: &OverworldAction| -> bool {
        let Some(arrival) = arrival else { return false };
        match action.tile {
            MetaTile::Warp { .. } => action.destination == arrival.at,
            MetaTile::Connection { to_map, .. } | MetaTile::ConnectionWater(to_map) => Some(to_map) == arrival.from,
            _ => false,
        }
    };
    let mut actions = state.map.actions();
    // ⚠️ **The PC is withheld, and the reason changed on 2026-08-27 without the answer changing.**
    // It used to be that nothing could be done behind that menu at all: `FieldMoveRequest` carried
    // no `UsePcBox` or `UseItemPc`, so choosing the row walked over, pressed A, reported
    // `✓ used the PC` and accomplished nothing observable. It carries both now — so the row is no
    // longer a dead end, it is a *duplicate*. Every PC operation is a field move that walks to the
    // PC itself (`the_pc_here`, off `pc_locations_for`), which means a row whose whole contract is
    // "walk there and press A" leaves the agent holding an open storage menu with no operation
    // chosen and nothing but `BACKING_OUT_TICKS` to get out of it. One way in, not two.
    //
    // ⚠️ **Bill's cell separator is not this and must not be folded back into it.** It is the same
    // tile and a different thing: one press, no menu, and the only route to the S.S. Ticket. It is
    // offered as `MetaTile::Switch(CellSeparator)` while its event window is open — see
    // `MetaTileMap::bill_cell_separator` — which is why withholding `Pc` no longer costs the run the
    // game. Before that gate existed this line was a hard progression blocker: a deployed run sat in
    // Cerulean for four and a half hours of cartridge time with no way to press it.
    //
    // `MetaTile::Pc` stays in `actions()` for the scripted policies, which drive the boxes through
    // `FieldMove` directly.
    actions.retain(|action| !matches!(action.tile, MetaTile::Pc));
    // `id_kind`, not `kind`: two people can share the tile an action approaches them from, and
    // "Sprite" == "Sprite" leaves that pair to `sort_by_key`'s stability over a `HashSet` walk.
    actions.sort_by_key(|action| (action.destination.y, action.destination.x, action.tile.id_kind()));
    actions
        .iter()
        .map(|action| MenuItem {
            id: overworld_id(state, action),
            description: match way_back(action) {
                true => format!("{}; the way you came in", overworld_description(state, action)),
                false => overworld_description(state, action),
            },
        })
        .collect()
}

/// Match an id against a **freshly recomputed** action list. `None` means the action is gone, which
/// is a thing the model is told rather than a thing that crashes.
pub fn resolve_overworld(state: &GameState, id: &str) -> Option<OverworldAction> {
    state.map.actions().into_iter().find(|action| overworld_id(state, action) == id)
}

/// The id of one battle action. Keyed on what the action *is* rather than where it sat: a bag slot
/// shifts the moment an item runs out, and a move's PP — which is in `BattleAction`'s `Display` —
/// changes the moment it is used.
pub fn battle_id(action: &BattleAction) -> String {
    match action {
        BattleAction::Fight { battle_move, .. } => format!("fight:{}", battle_move.name),
        BattleAction::UseItem { item, .. } => format!("item:{:?}", item.id),
        BattleAction::SwitchPokemon { slot, .. } => format!("switch:{slot}"),
        BattleAction::Run => "run".to_string(),
        BattleAction::SafariBall => "ball".to_string(),
        BattleAction::SafariBait => "bait".to_string(),
        BattleAction::SafariRock => "rock".to_string(),
    }
}

/// The battle menu, with every `fight:` row costed against the Pokémon actually in front of you.
///
/// ⚠️ **The type chart is the agent's job, not the model's, and this was the one decision kind
/// where it was not.** `set_battle_script` hands a script `mv.damage` and `mv.effectiveness` on the
/// argument that "a script that had to carry its own type chart is one no model would get right from
/// memory" — and every word of that applies to a model answering the turn by hand, which had neither.
/// Worse, the system prompt tells it in as many words that **prior knowledge of Pokémon Red is not
/// evidence**, so the only source left for a matchup was the one source the prompt forbids. Both
/// numbers come from [`type_multiplier`] and [`expected_damage`], the same two functions the script
/// reads, so a fallback turn and a scripted one cannot disagree.
///
/// ⚠️ **On the row rather than in `BattleAction`'s `Display`.** That `Display` is a *menu row* shared
/// with `battle_report::intent`'s neighbour and the battle-script validation table, and neither of
/// those has a defender to cost a move against; it also has no `GameState`. The damage belongs to the
/// pairing, not to the action, so it is appended here where both sides are in hand.
///
/// ⚠️ **A status move gets no number at all**, the `34 -> 34` rule from `battle_report`: Growl has no
/// expected damage, and printing `0 dmg` beside it reads as "this move is useless" rather than "this
/// move does something other than damage".
pub fn battle_menu(state: &GameState) -> Vec<MenuItem> {
    let sides = state.battle.as_ref().map(|battle| (&battle.player, &battle.enemy));
    battle_options(state)
        .unwrap_or_default()
        .iter()
        .map(|action| {
            let mut description = format!("{action}");
            if let (BattleAction::Fight { battle_move, .. }, Some((me, foe))) = (action, sides) {
                description.push_str(&fight_row_note(battle_move.name, me, foe));
            }
            MenuItem { id: battle_id(action), description }
        })
        .collect()
}

/// What a `fight:` row says beyond its name and PP: roughly what it would take off, and the
/// cartridge's own words for the multiplier when there is one to report.
fn fight_row_note(
    name: crate::pokemon::move_name::PokemonMoveName,
    me: &crate::pokemon::pokemon::PokemonSummary,
    foe: &crate::pokemon::pokemon::PokemonSummary,
) -> String {
    use crate::pokemon::damage::{effectiveness_phrase, expected_damage, is_damaging_move, type_multiplier};
    // ⚠️ **The multiplier is only reported for a move that deals damage**, and this gate is the
    // whole of that. Growl is a Normal move and Normal has no effect on Ghost, so a chart consulted
    // blindly labels it "no effect" against a Gastly — which is false: Gen 1 applies type immunity
    // to the damage calculation, and a status move lands regardless. Printing it would send the model
    // hunting for a different debuff for the one matchup where debuffing is the plan.
    if !is_damaging_move(name) {
        return String::new();
    }
    match (expected_damage(me, name, foe), effectiveness_phrase(type_multiplier(name, foe))) {
        // ⚠️ The immune case has no damage *and* a phrase, and it is the one row where the phrase
        // is the entire decision — so it must not fall through to the arm below.
        (_, Some(phrase @ "no effect")) => format!(" — {phrase}"),
        (Some(damage), Some(phrase)) => format!(
            " — ~{damage} damage ({}% of its HP), {phrase}", percent_of(damage, foe.stats.hp)),
        (Some(damage), None) => format!(
            " — ~{damage} damage ({}% of its HP)", percent_of(damage, foe.stats.hp)),
        // A damaging move the estimator declines to price (it has the immunity arm above covered).
        (None, _) => String::new(),
    }
}

/// Damage as a share of the defender's *maximum* HP, which is the figure that says "this is a
/// two-hit kill" without the model doing the division. Capped at 100 because an overkill reported as
/// 240% reads as a bug rather than as certainty.
fn percent_of(damage: u16, max_hp: u16) -> u16 {
    match max_hp {
        0 => 0,
        max => ((damage as u32 * 100 / max as u32) as u16).min(100),
    }
}

pub fn resolve_battle(state: &GameState, id: &str) -> Option<BattleAction> {
    battle_options(state)?.into_iter().find(|action| battle_id(action) == id)
}

/// What the mart in front of the player sells, read from its own ROM list at the poll (see
/// [`ApiSnapshot`]). The id is the item's name, because that is what `buy_item` takes.
///
/// ⚠️ **How many you already hold is on the row, and its absence was the module's own rule broken on
/// the one turn it mattered most.** "Anything a read can answer from the situation should be in the
/// situation": a mart turn is entirely the question *what am I short of*, and the answer lived behind
/// `read_bag` — so playing this turn properly cost a round trip, every time, and the deployed run
/// bought nothing at a mart across 429 decisions. ⚠️ **Zero is printed, not omitted**: "you have
/// none" is the row that decides a purchase, and leaving it blank makes the commonest reason to buy
/// the one thing the menu does not say.
pub fn mart_menu(snapshot: &ApiSnapshot, state: &GameState) -> Vec<MenuItem> {
    snapshot
        .mart_stock
        .iter()
        .map(|(item, price)| {
            let held = state.bag.iter().find(|entry| entry.id == *item).map_or(0, |e| e.quantity);
            MenuItem {
                id: item.to_string(),
                description: format!(
                    "{} — you have {held}",
                    match price {
                        // Every mart item has a price; a missing one means the ROM's table did not
                        // have it, which is worth showing rather than hiding behind a plausible
                        // number.
                        Some(price) => format!("¥{price}"),
                        None => "price unknown".to_string(),
                    },
                ),
            }
        })
        .collect()
}

/// The four moves the forget prompt is choosing between. The id is the slot, which is what
/// `forget_move` takes — there is no reordering hazard here, because the prompt itself is indexed by
/// slot and lives for as long as the question does.
/// The four moves the forget prompt is choosing between, each costed so the choice can be made from
/// the row.
///
/// ⚠️ **A row used to be a name and its PP, and that is not enough to answer with.** Which of four
/// moves to lose is decided on power, on type coverage and on whether the move is an HM — none of
/// which was here, so the model either guessed from the name or spent `read_party` on a turn where
/// the whole question is four moves it is already being shown. ⚠️ The **HM marker is the one that
/// matters**: Gen 1 has no move deleter, so forgetting Cut or Surf is not a trade, it is a loss, and
/// `DeterministicPolicy` has protected against it since long before the model was ever asked.
pub fn forget_menu(current: &[PokemonMove]) -> Vec<MenuItem> {
    current
        .iter()
        .enumerate()
        .map(|(slot, known)| {
            let metadata = known.name.metadata();
            MenuItem {
                id: slot.to_string(),
                description: format!(
                    "{} — {}, {}, {}/{} pp{}",
                    known.name,
                    metadata.move_type,
                    match metadata.power {
                        Some(power) => format!("{power} power"),
                        None => "no damage".to_string(),
                    },
                    known.pp,
                    metadata.pp,
                    match hm_move(known.name) {
                        Some(_) => " — ⚠️ an HM move, and it cannot be re-learnt",
                        None => "",
                    },
                ),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::protocol::FunctionCall;

    /// A call with a `summary` filled in, since [`classify`] now refuses a terminal call without
    /// one and almost every fixture here is about some *other* argument. Anything that is not a JSON
    /// object is passed through untouched — several tests hand this deliberate rubbish. Use
    /// [`bare`] to write a call that genuinely says nothing.
    fn call(name: &str, arguments: &str) -> ToolCall {
        let arguments = match serde_json::from_str::<Value>(arguments) {
            Ok(Value::Object(mut object)) => {
                object.entry("summary").or_insert_with(|| json!("doing the thing"));
                Value::Object(object).to_string()
            }
            _ => arguments.to_string(),
        };
        bare(name, &arguments)
    }

    /// A call exactly as written, `summary` and all.
    fn bare(name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: "c".into(),
            kind: "function".into(),
            function: FunctionCall { name: name.into(), arguments: arguments.into() },
        }
    }

    fn names(kind: DecisionKind) -> Vec<&'static str> {
        for_kind(kind).into_iter().map(|tool| tool.function.name).collect()
    }

    /// Oak's lab just after the starter is taken: a party of one, an ordinary bag, and a map with
    /// no trees on it — which between them exercise every check [`resolve_field_move`] makes.
    fn fixture_state() -> GameState {
        let mut gb = crate::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/oaks-lab-just-got-squirtle.bin"))
            .expect("the committed fixture loads");
        { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }.expect("the fixture has a readable state")
    }

    /// ⚠️ **Every one of these is refused inside the turn, which is the whole point of resolving
    /// here rather than letting the policy find out.** A PC operation with no PC, or a lift that is
    /// not in the room, would otherwise be published as a `Decision`, sent to the policy and dropped
    /// there — and the complaint rides on the *next* turn, a second full prefill of the history, by
    /// which time the model has moved on. The wording matters as much as the refusal: each says what
    /// to do instead, which is what `learnset::teach_refusal` had to be taught.
    #[test]
    fn a_pc_or_a_lift_that_is_not_here_is_refused_inside_the_turn() {
        use crate::pokemon::postgame::item_storage::PcItemOp;
        use crate::pokemon::postgame::pc_box::PcBoxOp;
        // Oak's lab: no PC, no lift.
        let state = fixture_state();

        let refused = |request| resolve_field_move(&state, &request).expect_err("refused");
        let no_pc = refused(FieldMoveRequest::UsePcBox { op: PcBoxOp::Deposit { slot: 0 } });
        assert!(no_pc.contains("no PC on"), "says which map has none: {no_pc}");
        assert!(no_pc.contains("Pokémon Centre"), "and where one is: {no_pc}");
        let no_pc_items = refused(FieldMoveRequest::UseItemPc {
            op: PcItemOp::Deposit, item: ItemId::Potion, qty: 1,
        });
        assert!(no_pc_items.contains("no PC on"), "{no_pc_items}");

        let no_lift = refused(FieldMoveRequest::UseElevator { to: Map::SilphCo5F });
        assert!(no_lift.contains("no lift on"), "{no_lift}");
        assert!(no_lift.contains("Silph"), "names where the three lifts are: {no_lift}");
    }

    /// ⚠️ **A floor the lift does not stop at is a different refusal from a lift that is not
    /// here**, and it has to list the floors — the model cannot see the panel, and guessing again is
    /// the retry loop the whole catalogue is arranged to avoid.
    #[test]
    fn a_lift_says_which_floors_it_serves() {
        let mut state = fixture_state();
        state.map.map = Map::RocketHideoutElevator;
        let wrong_floor = resolve_field_move(&state, &FieldMoveRequest::UseElevator { to: Map::SilphCo5F })
            .expect_err("this lift does not go to Silph Co");
        assert!(wrong_floor.contains("does not stop at"), "{wrong_floor}");
        assert!(wrong_floor.contains("RocketHideoutB4F"), "lists the floors: {wrong_floor}");

        // ⚠️ The hideout's lift is the one that needs the key, and B4F behind it is Giovanni — so
        // the Silph Scope, the Poké Flute and both Snorlax. `RocketHideoutElevatorText` opens
        // `ld b, LIFT_KEY`; without it the panel prints a line and no menu opens at all.
        let no_key = resolve_field_move(&state, &FieldMoveRequest::UseElevator { to: Map::RocketHideoutB4F })
            .expect_err("no Lift Key in Oak's lab");
        assert!(no_key.contains("LIFT_KEY"), "names the key: {no_key}");
    }

    /// ⚠️ **`read_pc` is the one read added since the "nothing may duplicate the situation" rule was
    /// written that had to argue against it.** A deposit names a party slot every turn already
    /// lists; a *withdrawal* names a box slot, and no section of the situation carries the box — so
    /// without this the model picks a number blind. It is Overworld-only for the ordinary reason: a
    /// battle, a naming screen and a mart cannot act on it.
    #[test]
    fn read_pc_is_offered_on_the_only_turn_that_can_use_it() {
        assert!(names(DecisionKind::Overworld).contains(&"read_pc"));
        for elsewhere in [DecisionKind::Battle, DecisionKind::Nickname, DecisionKind::MartPurchase,
                          DecisionKind::ForgetMove, DecisionKind::Stuck] {
            assert!(!names(elsewhere).contains(&"read_pc"), "{elsewhere:?} must not offer read_pc");
        }
    }

    /// The three verbs the model could not reach at all before 2026-08-27, parsed off the wire the
    /// way a model writes them.
    #[test]
    fn the_pc_and_the_lift_are_field_moves_a_model_can_name() {
        use crate::pokemon::postgame::item_storage::PcItemOp;
        use crate::pokemon::postgame::pc_box::PcBoxOp;
        let request = |arguments: &str| field_move_arguments(
            &serde_json::from_str::<Value>(arguments).expect("valid JSON")).expect("parses");

        assert_eq!(request(r#"{"move":"pc_pokemon","op":"deposit","slot":2}"#),
                   FieldMoveRequest::UsePcBox { op: PcBoxOp::Deposit { slot: 2 } });
        assert_eq!(request(r#"{"move":"pc_pokemon","op":"withdraw","box_slot":7}"#),
                   FieldMoveRequest::UsePcBox { op: PcBoxOp::Withdraw { box_slot: 7 } });
        // ⚠️ 1-based on the wire, 0-based inside, because the cartridge's own CHANGE BOX menu counts
        // from one and the model is reading the same numbers a player would.
        assert_eq!(request(r#"{"move":"pc_pokemon","op":"change_box","box":1}"#),
                   FieldMoveRequest::UsePcBox { op: PcBoxOp::ChangeBox { n: 0 } });
        assert_eq!(request(r#"{"move":"pc_items","op":"withdraw","item":"Potion","quantity":3}"#),
                   FieldMoveRequest::UseItemPc { op: PcItemOp::Withdraw, item: ItemId::Potion, qty: 3 });
        // A quantity nobody gave is one, which is what a model omitting it means.
        assert_eq!(request(r#"{"move":"pc_items","op":"deposit","item":"Potion"}"#),
                   FieldMoveRequest::UseItemPc { op: PcItemOp::Deposit, item: ItemId::Potion, qty: 1 });
        assert_eq!(request(r#"{"move":"elevator","map":"SilphCo5F"}"#),
                   FieldMoveRequest::UseElevator { to: Map::SilphCo5F });

        // A misspelt operation is named rather than falling through to a wrong default.
        let bad = field_move_arguments(&serde_json::from_str::<Value>(
            r#"{"move":"pc_pokemon","op":"store","slot":0}"#).expect("valid JSON")).expect_err("refused");
        assert!(bad.contains("deposit, withdraw, release or change_box"), "{bad}");
    }

    /// ⚠️ **Two menu rows may never share an id, and hidden objects broke it the day they were
    /// added.** An id is `{map}:{x},{y}:{kind}` where the coordinate is the tile the player stands
    /// on to act, not the thing acted on — so two objects a tile apart share one. Vermilion Gym's
    /// bins at (9, 7) and (9, 9) are both reached from (9, 8), and both rows minted
    /// `VermilionGym:9,8:TrashCan`; `resolve_overworld` matches by string equality, so the second
    /// bin was unreachable and nothing said so. It is the failure `MapSprite`'s `Rocket1`/`Rocket2`
    /// numbering already prevents for people, which is why the fix is the same one.
    ///
    /// ⚠️ **The gym is the fixture because it is the worst case in the game** — fifteen identical
    /// objects on a 10-wide map — and it is also the map where a silently unreachable row costs a
    /// badge: two of those bins hold the switches and which two is re-rolled on every attempt.
    #[test]
    fn no_two_menu_rows_can_share_an_id() {
        let mut gb = crate::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/gym-trash-solved.bin")).expect("the fixture loads");
        let state = { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }
            .expect("a readable state");
        let menu = overworld_menu(&state, None);
        assert!(
            menu.iter().filter(|row| row.id.contains(":TrashCan")).count() >= 15,
            "all fifteen bins are rows, or this proves nothing: {menu:#?}",
        );

        let mut seen = std::collections::HashSet::new();
        for row in &menu {
            assert!(seen.insert(row.id.clone()), "`{}` is offered twice; one of them can never be chosen", row.id);
        }

        // ⚠️ And every one of them still resolves. Uniqueness alone would be satisfied by ids that
        // are unique and wrong — the pair this caught differed only in a suffix, so a mis-numbered
        // ordinal would pass the check above and match nothing here.
        for row in &menu {
            assert!(resolve_overworld(&state, &row.id).is_some(), "`{}` is offered but does not resolve", row.id);
        }
    }

    /// Every kind, so a loop that meant "all of them" cannot quietly stop meaning it when a seventh
    /// is added.
    const KINDS: [DecisionKind; 6] = ALL_KINDS;

    /// **A menu row carries what its id cannot, and nothing else.**
    ///
    /// Three ladders on one floor are three rows `to MtMoonB1F`, and the one the player just came
    /// up is indistinguishable from the two not yet tried — which is the loop the deployed run sat
    /// in for days. The arrival marks it, and only it.
    #[test]
    fn the_warp_the_player_came_in_by_says_so() {
        use crate::pokemon::world_graph::Arrival;
        let state = fixture_state();
        let door = crate::geometry::Point8 { x: 5, y: 11 };
        let marked = overworld_menu(&state, Some(Arrival { map: state.map.map, at: door, from: Some(Map::PalletTown) }));
        let row = |menu: &[MenuItem], id: &str| menu.iter().find(|m| m.id == id).expect(id).description.clone();
        assert_eq!(row(&marked, "OaksLab:5,11:Warp"), "take the warp to PalletTown, arriving at (12, 12); the way you came in");
        assert_eq!(marked.iter().filter(|m| m.description.contains("came in")).count(), 1, "{marked:#?}");

        // An arrival on some other map says nothing about this one.
        let elsewhere = overworld_menu(&state, Some(Arrival { map: Map::PalletTown, at: door, from: None }));
        assert_eq!(row(&elsewhere, "OaksLab:5,11:Warp"), "take the warp to PalletTown, arriving at (12, 12)");
    }

    /// A row says what choosing it *does*, as a verb phrase — the id is a key, not a description,
    /// and a model that had to parse the action out of the key took a Rocket for a warp. What it
    /// must not carry is noise: a step count, a landing coordinate, or the word "sprite".
    #[test]
    fn a_menu_row_explains_the_action_in_words() {
        let state = fixture_state();
        let menu = overworld_menu(&state, None);
        let rows: Vec<String> =
            menu.iter().map(|item| format!("- `{}` — {}", item.id, item.description)).collect();

        assert!(rows.contains(&"- `OaksLab:5,11:Warp` — take the warp to PalletTown, arriving at (12, 12)".to_string()), "{rows:#?}");
        // ⚠️ A person is named by the id *and* by the row: the name is what the verb needs. But the
        // verb is the game's own kind of thing — this one is a bookcase, by its picture — and never
        // "Sprite", the emulator's word for anything that moves.
        assert!(rows.contains(&"- `OaksLab:2,2:Pokedex1` — read the Pokedex 1".to_string()), "{rows:#?}");
        assert!(!rows.iter().any(|row| row.contains("Sprite")),
                "no row may call a person a sprite: {rows:#?}");

        for item in &menu {
            assert!(item.description.starts_with(|c: char| c.is_ascii_lowercase()), "{item:?}: not a verb phrase");
            assert!(!item.description.contains("steps"), "{item:?} still carries a distance");
        }
    }

    /// A warp's landing coordinate is in the destination's *picture* coordinates — the ones its
    /// own warps are labelled and listed in — which differ from the warp table's by the connection
    /// strips that map draws. Pallet Town has strips north and south, so the lab door the player
    /// comes out on is `(12, 12)` on its picture and `(12, 11)` in the ROM.
    #[test]
    fn a_landing_coordinate_is_where_the_destination_picture_puts_it() {
        let mut gb = crate::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/pallet-town-state.bin")).expect("the committed fixture loads");
        let pallet = { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }.expect("readable");
        let lab_door = crate::pokemon::observe::map_view(&pallet).warps.into_iter()
            .find(|w| w.to_map == format!("{}", Map::OaksLab)).expect("Pallet Town has a door into the lab");
        let menu = overworld_menu(&fixture_state(), None);
        let out = menu.iter().find(|m| m.id == "OaksLab:5,11:Warp").expect("the lab's door").description.clone();
        assert_eq!(out, format!("take the warp to PalletTown, arriving at ({}, {})", lab_door.at.x, lab_door.at.y));
        assert_eq!(crate::pokemon::map_header::strip_offset(Map::MtMoon1F), (0, 0));
    }

    /// The verb comes from the sprite's picture, so an item on the ground is picked up and a person
    /// is spoken to, whatever either is called.
    #[test]
    fn a_sprite_row_is_verbed_by_its_picture() {
        let mut gb = crate::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/mt-moon.bin")).expect("the committed fixture loads");
        let state = { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }.expect("readable");
        let menu = overworld_menu(&state, None);
        let find = |needle: &str| menu.iter().find(|m| m.id.ends_with(needle)).unwrap_or_else(|| panic!("{needle}: {menu:#?}")).description.clone();
        assert_eq!(find(":Potion1"), "pick up the Potion");
        assert_eq!(find(":TMWaterGun"), "pick up the TM Water Gun");
        assert_eq!(find(":Hiker"), "talk to Hiker");
    }

    /// **`read_bag` has to agree with the bag the game is holding, row for row.**
    ///
    /// ⚠️ **It did not, and the deployed run of 2026-08-27 saw both halves.** `observe::bag` reads
    /// `GameState::bag`, which is a [`Bag`](crate::pokemon::bag::Bag), which **drops every id
    /// `ItemId` cannot name**. Only twelve of the fifty TMs were named, so a bag holding
    /// `TM34, HELIX FOSSIL, MOON STONE, TM01` was answered as three items with `slots_used: 3`, and
    /// the run then found TM01 on screen with nothing having ever mentioned it and filed a bug about
    /// the "unrelated TM34/bag menu prompt" it thought it had caused.
    ///
    /// ⚠️ **The count is the dangerous half, not the missing row.** The bag holds twenty kinds and a
    /// pickup into a full one is refused in a way that looks from outside exactly like a pickup that
    /// worked, which is what `toss_item` exists to avoid; a model told it has four free slots when
    /// it has three walks into that hole with the tool in its hand.
    ///
    /// The fixture is chosen for holding two TMs, an HM and a key item at once. Both sides are read
    /// from the same cartridge: `wNumBagItems` raw against what the tool would answer.
    #[test]
    fn read_bag_counts_every_slot_the_game_counts() {
        use crate::pokemon::PokemonApiTrait;
        use crate::pokemon::symbols::DmgPointerRead;
        let mut gb = crate::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/post-ss-anne.bin")).expect("the committed fixture loads");
        let mut api = crate::pokemon::PokemonApi::new(&mut gb);
        let state = api.game_state().expect("readable");
        let view = crate::pokemon::observe::bag(&state, &api);

        let raw = api.mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wNumBagItems) as usize;
        assert_eq!(view.slots_used, raw, "read_bag says {} of the bag's slots are used, the game says {raw}", view.slots_used);
        assert_eq!(view.items.len(), raw, "and every one of them has to be listed: {:?}", view.items);

        // The machines are the ones that went missing, and the name has to be the one the model can
        // quote back into `toss_item` or `teach`.
        let named: Vec<&str> = view.items.iter().map(|i| i.item.as_str()).collect();
        assert!(named.contains(&"Tm34Bide"), "{named:?}");
        assert!(named.contains(&"Hm01Cut"), "{named:?}");
        for name in &named {
            assert!(item_by_name(name).is_some(), "{name} is listed but cannot be named back");
        }

        // ⚠️ **This fixture cannot prove the bug on its own** and the loop below is what does. Its
        // bag holds TM11, TM28 and TM34, which were three of the twelve machines that *were* named,
        // so the agreement above passed throughout. What made the deployed run's bag disagree was
        // TM01. So the assertion is over the whole machine range rather than over one save: every
        // id `$C4`-`$FA` has to be nameable, and nameable *back*, since the name is the handle the
        // model quotes into `toss_item` and `teach`.
        for id in 0xC4..=0xFAu8 {
            let item = ItemId::from_repr(id)
                .unwrap_or_else(|| panic!("${id:02X} is a machine and `read_bag` would drop it"));
            let name = item.to_string();
            assert_eq!(item_by_name(&name), Some(item), "{name} does not resolve back to ${id:02X}");
        }
    }

    /// **What the `tools` array costs, per kind, with a ceiling on each.**
    ///
    /// ⚠️ **It is paid per *completion*, not per turn.** The whole array goes out again with every
    /// request, and a turn that reads before it decides is several — so a tool description is
    /// multiplied by `GB_MAX_TOOL_STEPS` before anything the model actually says is counted.
    ///
    /// The ceilings are generous enough that rewording a description never trips them and tight
    /// enough that adding a tool to every kind, or unscoping the reads again, has to be a deliberate
    /// edit to this list. They are bytes of JSON — roughly four to the token — because that is what
    /// is measurable here; the token count depends on the endpoint's tokeniser.
    #[test]
    fn the_tool_array_stays_within_its_budget() {
        // Overworld is the big one: it carries `use_field_move`, which is a dozen field actions
        // behind one `move` discriminant precisely so it is one entry rather than twelve.
        for (kind, ceiling) in [
            // Measured 2026-08-19, after `todo_add` became `todo_set`: 9064, 5391, 3543, 3965,
            // 3861, 4304. Each ceiling has ~8-10% of headroom for rewording.
            //
            // ⚠️ **`todo_set` cost 238 bytes on every kind, and that is the price of one tool, not
            // of the catalogue.** An optional `id` property plus the sentence saying which of the
            // three edits each argument shape means — add, rewrite, delete. What it buys is a plan
            // that can be *revised* in one call: under `todo_add` a wrong item could only be
            // completed or kept, and the deployed model did exactly that. (The 2026-08-14 figures,
            // after `press_buttons` was asked to say `why`: 8826, 5153, 3305, 3727, 3623, 4066.)
            //
            // ⚠️ **`why` cost 239 bytes and no ceiling had to move, and that was worked for.** The
            // first draft of the reword spent 403 — the tool description had grown a clause listing
            // what the action menu covers, which is the menu said twice, and the `why` description
            // had grown a sentence explaining what "why" means. It is one property on **one** tool,
            // so unlike `summary` below it scales with nothing at all; that is the whole reason it
            // was affordable as a lock-down when more prose was not.
            //
            // ⚠️ **Two ceilings came *down* on 2026-08-25 and that is the point of them.** Measured
            // after `press_buttons` left the two kinds that have a menu and `report_issue` replaced
            // it: 8646, 4908, 3543, 3965, 3861, 5144. Overworld and Battle each shed ~450 bytes,
            // because the hatch carried a `why`, a `summary` and a paragraph talking the model out
            // of using it, and the tool that replaced it is not terminal so it pays for no summary.
            // `Stuck` is the only kind that grew: it keeps the hatch *and* gains `report_issue`,
            // which is right — it is the turn where the agent is most likely to be genuinely wrong,
            // and the one place a press is still the correct answer. The unchanged three are the
            // single-question prompts, which carry neither tool.
            //
            // ⚠️ **The ceilings were lowered to match rather than left where they were.** A ceiling
            // with 20% of slack stops being a ratchet; leaving Overworld at 9700 would have let the
            // 418 bytes just recovered be spent again without anybody deciding to.
            //
            // ⚠️ **The jump from the 2026-08-12 figures (6875, 3773, 2530, 2952, 2848, 2877) is
            // `add_summary_argument`, and it is bought rather than leaked.** It is one property
            // repeated across every terminal tool a kind offers — `Stuck` has two and pays twice —
            // so it is the one addition here that scales with the *number* of terminals rather than
            // with the catalogue. What it buys is the only sentence the model keeps about its own
            // turn; see that function.
            //
            // ⚠️ **The HM gate and the nickname reword cost 176 and 141 bytes and no ceiling moved,
            // which is what the slack is for.** Measured the same day, after: 8822, 4908, 3684,
            // 3965, 3861, 5144. `use_field_move` gained the sentence saying an HM needs its badge as
            // well as the move — the thing the deployed run spent eleven turns and two issue reports
            // failing to work out — and `set_nickname` stopped telling the model that keeping the
            // default "is the ordinary answer", which is what every naming screen of both deployed
            // runs did. Both drafts were about twice the length first; the tool description says the
            // rule once and the turn's own situation carries the argument, which is the split that
            // kept them affordable.
            //
            // ⚠️ **Overworld's ceiling moved for the first time on 2026-08-26, and 489 bytes of
            // what forced it had already been spent without anybody recording it.** Re-measured
            // across all six: 9828, 4869, 3645, 3926, 3822, 5595. Two additions are in that
            // Overworld figure. `read_guide` is one whole tool — 489 bytes, on Overworld alone —
            // added after the figures above were taken, which is what quietly ate the headroom;
            // it is written down here now because a ceiling nobody re-measures stops being a
            // ratchet and becomes a surprise. `choose_action`'s chain is the other, at **558**: an
            // optional `then` array and a `resume_after_battle` flag (207) and the sentences saying
            // where a chain is worth taking and what ends one (351). What it buys is turns rather
            // than tokens — heal and then leave the Centre is one request instead of two — so it is
            // the one addition here that pays for itself in the thing the whole catalogue is
            // rationing. Drafted twice as long first: the tool description says the rule and the
            // policy's note says what became of a chain that stopped, which is the same split that
            // kept the HM gate affordable.
            //
            // ⚠️ **The other five came down to match, as they did last time.** All five had
            // drifted ~40 bytes *smaller* and none was near its ceiling; leaving them where they
            // were would bank slack nobody decided to spend.
            //
            // ⚠️ **Overworld's ceiling moved again on 2026-08-26, for the battle script, and this
            // is the one entry here whose spend is measured against *requests* rather than
            // against bytes.** Re-measured across all six: 11 200, 4869, 3645, 3926, 3822, 5595 —
            // so the three tools cost **1372 bytes**, all of it on Overworld. Roughly: 236 for
            // `get_battle_script_docs`, 202 for `read_battle_script` and 934 for
            // `set_battle_script`, whose description has to say what a script *is*, that it is
            // validated before it is armed, and what happens when it fails — because a model that
            // installs one without knowing it can be disarmed reads a fallback battle turn as the
            // feature being broken.
            //
            // ⚠️ **The arithmetic that justifies it is not the usual one.** Every other entry above
            // trades bytes on every request for behaviour; this one trades ~340 tokens per
            // overworld request against **whole requests removed**. A battle is 5 to 30 turns and
            // each is a full prefill of a ~50 k-token history, so one scripted wild encounter pays
            // for the addition several hundred times over. That is also why the three are scoped to
            // Overworld and Battle's ceiling *fell*: see `offers_battle_script`.
            //
            // ⚠️ **The other five came down again**, for the reason they came down last time.
            //
            // ⚠️ **`MartPurchase` moved 4050 → 4500 on 2026-08-27, and it is the second entry here
            // whose spend is measured in requests rather than in bytes.** Re-measured across all
            // six: 11 188, 4926, 3685, 4425, 3879, 5635. `buy_item` gained a `then` — up to
            // `MAX_CHAINED_PURCHASES` more kinds bought in the same visit — for **375 bytes**, and
            // what it buys is the arithmetic `choose_action`'s chain buys: Poké Balls *and* Potions
            // is one mart turn instead of two mart turns and the two overworld turns that reach
            // them, on the errand the prompt now tells the model to run at every mart it passes.
            // The deployed run bought nothing at a mart across 429 decisions, so the thing being
            // optimised is a turn count that was zero; the row-level "you have N" that goes with it
            // is in the *situation* and costs this array nothing.
            //
            // ⚠️ **The inner `then` items carry no descriptions and that is deliberate.** The first
            // draft repeated "A name from the stock list" and "How many to buy" inside the array
            // items as well as beside `item` and `quantity`, which is the same two sentences three
            // times for 77 bytes; the outer pair say it once and the array's own description says
            // what the array is. Same split that kept the HM gate and the chain affordable.
            //
            // ⚠️ **Overworld came *down* 11 400 → 11 350 while gaining two things**, which is worth
            // a line because it looks like nothing happened. `todo_set` gained `maxLength` and
            // `maximum` and `use_field_move`'s `direction` gained the description it never had
            // (+~99), and three battle-script descriptions lost 111 bytes of continuation
            // whitespace that had been baked into their string literals — runs of spaces the model
            // was reading. A cleanup that pays for a fix is still a cleanup nobody measured, so it
            // is measured here.
            // ⚠️ **Overworld moved 11 350 → 12 750 on 2026-08-27, for the PC and the lift, and this
            // is the third entry here whose spend is measured in something other than bytes: it is
            // measured in *whether the game can be finished at all*. Re-measured across all six:
            // 12 598, 4926, 3685, 4425, 3879, 5635 — so the addition is **1410 bytes**, all of it on
            // Overworld, split 1076 for the three `use_field_move` verbs and **334** for `read_pc`.
            //
            // What forced it: `overworld_menu` withheld every `MetaTile::Pc` row and
            // `FieldMoveRequest` carried no PC or lift operation, so a model could not press Bill's
            // cell separator. No press, no S.S. Ticket, so `EVENT_GOT_SS_TICKET` never fires, so
            // `CERULEANCITY_GUARD2` never stops standing on the only approach to the Trashed House
            // door — and that door is the only crossing between Cerulean's two terraces. The
            // deployed run of 2026-08-27 walked laps of Cerulean and Routes 24/25 for **four and a
            // half hours of cartridge time**, filed six issue reports about it, and correctly
            // worked out that it was in a loop it could not leave. The same gap held the Rocket
            // Hideout lift, and with it Giovanni, the Silph Scope, the Poké Flute and both Snorlax.
            //
            // ⚠️ **The verbs are the cheap half and `read_pc` is the one that had to be argued
            // for.** A deposit names a party slot the turn already lists, but a withdrawal names a
            // *box* slot, and the box is in none of the situation's sections — so without the read
            // the model would be picking a number blind, which is the one thing `READ_TOOLS`'
            // "nothing here may duplicate the situation" rule has never had to worry about.
            //
            // ⚠️ **Three additions were paid for out of the same change rather than added to it.**
            // The `interact` line lost the clause listing the gym bins and the Mansion switches,
            // because both are rows in the action menu now (`MetaTile::Switch`) and a schema that
            // still pointed at coordinates would be teaching the harder way round; `map`'s
            // description is shared with `elevator` rather than duplicated; and the four new
            // properties carry one sentence each rather than restating what `op` means per verb.
            //
            // ⚠️ **The other five did not move, because they did not drift.** 4926/3685/4425/3879/
            // 5635 are byte-for-byte the figures recorded above, and none of this reaches them:
            // `read_pc` is Overworld-only and `use_field_move` is a terminal only Overworld offers.
            (DecisionKind::Overworld, 12_750),
            (DecisionKind::Battle, 4_950),
            (DecisionKind::Nickname, 3_750),
            (DecisionKind::MartPurchase, 4_500),
            (DecisionKind::ForgetMove, 3_950),
            (DecisionKind::Stuck, 5_700),
        ] {
            let bytes = serde_json::to_string(&for_kind(kind)).expect("the specs serialise").len();
            assert!(bytes <= ceiling, "{kind:?}'s tools are {bytes} bytes, over the {ceiling} budget");
        }
    }

    /// [`READ_ROUTE`]'s four answers, which is the whole of it — and none of them is an `error`
    /// string, because each is a different thing for the model to do next.
    ///
    /// ⚠️ **An empty graph is the interesting case.** The tool replaced one that dumped every
    /// visited node, and the guarantee both share is *negative*: nothing here has been walked, so
    /// every route is `reachable: false` — which means "you have not been there", never "it does not
    /// exist". A run that read this as unreachable would stop exploring.
    #[test]
    fn a_route_answers_the_four_questions_and_never_bluffs() {
        let state = fixture_state();
        let graph = WorldGraph::new();
        let ask = |arguments: &str| -> Value {
            route_answer(&call(READ_ROUTE, arguments), &state, &graph)
        };

        // No `to`: what has been walked. Empty here, and an empty list is an answer.
        assert_eq!(ask("{}")["visited"], json!([]));
        assert_eq!(ask("{}")["from"], json!(format!("{}", state.map.map)));
        assert_eq!(ask(r#"{"to":""}"#)["visited"], json!([]), "a blank name is no name");

        // A name that is not a map at all: correctable, and it says how.
        let nonsense = ask(r#"{"to":"Kanto Safari Wildlife Park"}"#);
        assert!(nonsense["error"].as_str().expect("a sentence").contains("not a map"), "{nonsense}");

        // A real map nobody has walked to. ⚠️ Not an error: the way there has to be *found*.
        let unwalked = ask(r#"{"to":"CeruleanCity"}"#);
        assert_eq!(unwalked["reachable"], json!(false));
        assert_eq!(unwalked["to"], json!(format!("{}", Map::CeruleanCity)));
        assert!(unwalked["note"].as_str().expect("a sentence").contains("not been to"), "{unwalked}");

        // ⚠️ Spelled the way a model spells things, not the way the enum does — `map_by_name`
        // normalises, and a rejection over a space would be a rejection over nothing.
        assert_eq!(ask(r#"{"to":"cerulean city"}"#), unwalked);

        // The whole graph is never serialised, whatever is asked. That was the point.
        assert!(!ask("{}").to_string().contains("edges"));
    }

    /// ⚠️ **A battle menu row is prose, and `BattleAction`'s `Display` is what makes it so.**
    ///
    /// The switch rows were `{:?}` — `PKMN   PokemonSummary { species: Charizard, current_hp: 360,
    /// status: None, types: [Fire, Flying], moves: [Some(PokemonMove { name: Flamethrower, pp: 15
    /// }), …] }` — which is around 500 bytes of Rust syntax per switchable party member, in the menu
    /// of every battle turn for the length of a run. Same class of bug as `MetaTile`'s and
    /// `PokemonStatus`' old `strum` derives, and found the same way one would hope: by reading
    /// `prompt::tests::probe_turn_requests`' output.
    /// A `PokemonSummary` for the two costing tests below. `stats` are flat so the arithmetic in
    /// `expected_damage` is the type chart and the base power and nothing else.
    fn summary(
        species: crate::pokemon::species::PokemonSpecies,
        types: [crate::pokemon::pokemon::PokemonType; 2],
        moves: &[PokemonMoveName],
    ) -> crate::pokemon::pokemon::PokemonSummary {
        let mut slots = [None, None, None, None];
        for (slot, name) in moves.iter().enumerate() {
            slots[slot] = Some(PokemonMove { name: *name, pp: 20 });
        }
        crate::pokemon::pokemon::PokemonSummary {
            species,
            current_hp: 100,
            status: crate::pokemon::status::PokemonStatus::None,
            types,
            level: 25,
            moves: slots,
            stats: crate::pokemon::pokemon::PokemonStats {
                attack: 50, defense: 50, speed: 50, special: 50, hp: 100,
            },
            disabled_move_slot: None,
        }
    }

    /// ⚠️ **The type chart is the agent's job on this turn too, and for a long time it was not.**
    /// `set_battle_script` hands a script `mv.damage` and `mv.effectiveness` on the stated argument
    /// that no model gets a Gen 1 type chart right from memory — and a model answering the turn by
    /// hand had neither, on a turn where the system prompt tells it in as many words that prior
    /// knowledge of Pokémon Red is not evidence. Both numbers come from the same two functions the
    /// script reads, so a scripted turn and a fallback turn cannot disagree.
    #[test]
    fn a_fight_row_is_costed_against_the_pokemon_in_front_of_it() {
        use crate::pokemon::pokemon::PokemonType::*;
        use crate::pokemon::species::PokemonSpecies;

        let me = summary(PokemonSpecies::Charmander, [Fire, Fire],
                         &[PokemonMoveName::Ember, PokemonMoveName::Growl, PokemonMoveName::Scratch]);
        let foe = summary(PokemonSpecies::Bulbasaur, [Grass, Poison], &[PokemonMoveName::Tackle]);

        let ember = fight_row_note(PokemonMoveName::Ember, &me, &foe);
        assert!(ember.contains("super effective"), "Fire on Grass is doubled: {ember}");
        assert!(ember.contains("damage") && ember.contains("% of its HP"),
                "a number and what share of the target it is: {ember}");

        // ⚠️ **A status move gets no number, the `34 -> 34` rule.** Growl has no power, and `0
        // damage` beside it reads as "this move is useless" rather than "this move is not damage".
        assert_eq!(fight_row_note(PokemonMoveName::Growl, &me, &foe), "",
                   "a status move is not priced");

        // ⚠️ **And the multiplier is withheld from a status move as well, which is the subtler
        // half.** Growl is Normal and Normal has *no effect* on Ghost, so a chart consulted blindly
        // labels it "no effect" against a Gastly — false: Gen 1 applies type immunity to the damage
        // calculation and a debuff lands regardless. Printing it would send the model looking for a
        // different debuff in the one matchup where debuffing is the whole plan.
        let ghost = summary(PokemonSpecies::Gastly, [Ghost, Poison], &[PokemonMoveName::Lick]);
        assert_eq!(fight_row_note(PokemonMoveName::Growl, &me, &ghost), "",
                   "a status move is never labelled by the type chart");

        // A damaging move that genuinely cannot land says so, and says only that.
        let normal = summary(PokemonSpecies::Rattata, [Normal, Normal], &[PokemonMoveName::Tackle]);
        let row = fight_row_note(PokemonMoveName::Tackle, &normal, &ghost);
        assert_eq!(row, " — no effect", "immunity is the whole row: {row}");
    }

    /// ⚠️ **The PC is the one menu row that lied by succeeding.** `actions()` offers it in every
    /// Pokémon Centre, which is where the prompt now sends the model constantly — and choosing it
    /// walks over, presses A, reports `✓ used the PC` and accomplishes nothing observable, because
    /// `FieldMoveRequest` deliberately carries no `UsePcBox` or `UseItemPc`. That is the inverse of
    /// the rule the `CutTree` and `ConnectionWater` gates keep, and the worse half of it: a refusal
    /// at least tells the model something. ⚠️ **Withheld from the menu, not from `actions()`** —
    /// the scripted policies drive the boxes through `FieldMove` directly and must keep seeing it.
    #[test]
    fn the_menu_does_not_offer_a_pc_nothing_can_use() {
        use crate::pokemon::tile::MetaTile;
        let mut fixture = crate::pokemon::integration_tests::fixture::TestFixture::new(
            include_bytes!("../pokemon/data/at-celadon.bin"),
            std::time::Duration::from_secs(10),
            vec![],
        );
        let state = fixture.game_state();
        assert!(
            !overworld_menu(&state, None).iter().any(|row| row.id.ends_with(":Pc")),
            "no row offers the PC",
        );
        // The action itself is untouched, or every scripted deposit in `postgame::pc_box` breaks.
        assert_eq!(
            MetaTile::Pc.id_kind(), "Pc",
            "the id form stays, because `actions()` still yields it for the scripted policies",
        );
    }

    /// ⚠️ **Four move names and their PP is not enough to choose between four moves.** Power and
    /// type decide it, and the HM marker decides it outright: Gen 1 has no move deleter, so
    /// forgetting Cut or Surf is a loss rather than a trade, and `DeterministicPolicy` has given HM
    /// moves max value — never forget one — since long before a model was ever asked this question.
    #[test]
    fn a_forget_row_says_what_losing_the_move_would_cost() {
        let moves = [
            PokemonMove { name: PokemonMoveName::Tackle, pp: 30 },
            PokemonMove { name: PokemonMoveName::Cut, pp: 30 },
            PokemonMove { name: PokemonMoveName::Growl, pp: 40 },
        ];
        let rows = forget_menu(&moves);
        assert!(rows[0].description.contains("Normal") && rows[0].description.contains("power"),
                "type and power: {}", rows[0].description);
        assert!(rows[1].description.contains("HM move"), "the HM is marked: {}", rows[1].description);
        assert!(!rows[0].description.contains("HM move"), "and only the HM is");
        assert!(rows[2].description.contains("no damage"),
                "a status move says so rather than showing 0 power: {}", rows[2].description);
    }

    #[test]
    fn a_battle_menu_row_is_a_sentence_and_not_a_debug_dump() {
        let switch = BattleAction::SwitchPokemon {
            slot: 1,
            pokemon: crate::pokemon::pokemon::PokemonSummary {
                species: crate::pokemon::species::PokemonSpecies::Charizard,
                current_hp: 200,
                status: crate::pokemon::status::PokemonStatus::None,
                types: [crate::pokemon::pokemon::PokemonType::Fire; 2],
                level: 100,
                moves: [None, None, None, None],
                stats: crate::pokemon::pokemon::PokemonStats {
                    attack: 1, defense: 1, speed: 1, special: 1, hp: 360,
                },
                disabled_move_slot: None,
            },
        };
        assert_eq!(format!("{switch}"), "PKMN   Charizard Lv100 — 200/360 HP");

        // A healthy Pokémon says nothing about its status; `PokemonStatus`' own `Display` is
        // `strum`'s, so an unconditional one would read `, None` — a missing value, not good news.
        assert!(!format!("{switch}").contains("None"));
        let poisoned = match switch {
            BattleAction::SwitchPokemon { slot, mut pokemon } => {
                pokemon.status = crate::pokemon::status::PokemonStatus::Poisoned;
                BattleAction::SwitchPokemon { slot, pokemon }
            }
            other => other,
        };
        assert_eq!(format!("{poisoned}"), "PKMN   Charizard Lv100 — 200/360 HP, Poisoned");
    }

    /// **Every terminal tool asks for a summary, and nothing else does.**
    ///
    /// ⚠️ This is the only sentence the model keeps about its own turn. Reasoning arrives on a
    /// channel that is never sent back, and most models emit no `content` beside a tool call, so
    /// without it the assistant side of the history is a column of bare JSON: what was done, never
    /// why. A model reading that back has no record of having *tried* anything.
    ///
    /// It is not on the reads because a read is not a decision — one per turn is the point, and
    /// asking for one on `read_party` would buy the same sentence three times at three times the
    /// price.
    #[test]
    fn every_terminal_tool_asks_the_model_to_say_why() {
        for kind in ALL_KINDS {
            for tool in for_kind(kind) {
                let has_summary = tool.function.parameters["properties"].get("summary").is_some();
                let required = tool.function.parameters["required"]
                    .as_array()
                    .is_some_and(|required| required.iter().any(|name| name == "summary"));
                match terminal_names(kind).contains(&tool.function.name) {
                    true => {
                        assert!(has_summary, "{kind:?}'s `{}` has no summary", tool.function.name);
                        assert!(required, "{kind:?}'s `{}` does not require it", tool.function.name);
                    }
                    false => assert!(!has_summary, "`{}` is not a decision", tool.function.name),
                }
                // ⚠️ Every terminal schema is `additionalProperties: false`, so an argument that is
                // not declared is not merely ignored — the call is schema-invalid.
                assert_eq!(tool.function.parameters["additionalProperties"], json!(false),
                           "`{}` would accept an undeclared argument", tool.function.name);
            }
        }
    }

    /// ⚠️ **Required of the model, optional to the parser**, and the asymmetry is deliberate:
    /// rejecting a terminal call for a missing summary would not end the turn — it becomes another
    /// tool result and spends another of `GB_MAX_TOOL_STEPS` — so a model that forgot it would be
    /// pushed towards the forced `wait` rather than towards remembering.
    #[test]
    fn a_summary_is_read_off_the_call_and_never_demanded() {
        let call = |arguments: &str| ToolCall {
            id: "call_1".to_string(),
            kind: "function".to_string(),
            function: crate::llm::protocol::FunctionCall {
                name: "wait".to_string(),
                arguments: arguments.to_string(),
            },
        };

        assert_eq!(
            call_summary(&call(r#"{"ticks": 5, "summary": "  letting the battle text finish  "}"#)),
            Some("letting the battle text finish".to_string()),
            "trimmed, because it is printed on a page",
        );
        // A turn that omits it is still a turn: the decision is carried out either way.
        assert_eq!(call_summary(&call(r#"{"ticks": 5}"#)), None);
        assert_eq!(call_summary(&call(r#"{"ticks": 5, "summary": "   "}"#)), None, "blank is absent");
        assert_eq!(call_summary(&call("not json")), None, "and a broken call is not a panic");

        // `maxLength` in a schema is a request. This string reaches the page, the transcript and
        // every later request, so the cap is applied here rather than trusted.
        let long = "x".repeat(MAX_SUMMARY * 2);
        let capped = call_summary(&call(&format!(r#"{{"summary": "{long}"}}"#))).expect("present");
        assert_eq!(capped.chars().count(), MAX_SUMMARY);
    }

    /// `press_buttons`' `why`: the headline of the record `llm::incident` writes, on the one turn
    /// that still offers the tool.
    #[test]
    fn a_press_without_a_why_is_refused() {
        let schema = &press_buttons_spec().function.parameters;
        assert_eq!(schema["required"], json!(["buttons", "why"]), "the model is asked for it");

        let press = |arguments: &str| call("press_buttons", arguments);
        assert_eq!(
            call_reason(&press(r#"{"buttons":["a"], "why": "  no action opens the PC  "}"#)),
            Some("no action opens the PC".to_string()),
        );

        assert!(matches!(
            classify(DecisionKind::Stuck, &press(r#"{"buttons":["a"], "why": "a stuck text box"}"#), &[]),
            CallKind::Terminal(Terminal::PressButtons { .. }),
        ));
        // ⚠️ **Refused now, where it used to be carried out anyway.** The old trade — ask in the
        // schema, tolerate the absence in the parser, on the grounds that a rejection spends a
        // `GB_MAX_TOOL_STEPS` — was measured on the deployed run and lost: **543 of 749 presses
        // left `why` null**. A field that is optional in practice is a field a weak model omits,
        // and the record it was meant to make readable was three quarters blank.
        for arguments in [r#"{"buttons":["a"]}"#, r#"{"buttons":["a"], "why": " "}"#] {
            let CallKind::Rejected(complaint) = classify(DecisionKind::Stuck, &press(arguments), &[]) else {
                panic!("a press with nothing to say is refused: {arguments}");
            };
            assert!(complaint.contains("why"), "{complaint}");
        }

        let long = "x".repeat(MAX_REASON * 2);
        let capped = call_reason(&press(&format!(r#"{{"why": "{long}"}}"#))).expect("present");
        assert_eq!(capped.chars().count(), MAX_REASON, "a schema's maxLength is a request");
    }

    /// **Every terminal call has to say what it is doing**, on every kind — the one note the model
    /// keeps about its own turn.
    ///
    /// ⚠️ **The doc on `add_summary_argument` used to argue against enforcing this**, on the grounds
    /// that a rejection does not end the turn and so pushes a forgetful model towards the forced
    /// `wait`. What settled it was measuring the cost on the deployed run: of 2427 decisions only 98
    /// carried no summary and **all 98 were `wait`** — the *synthesised* fallback wait, which never
    /// goes through `classify` at all. The model already fills it in on every real action, so the
    /// rule costs that model nothing and closes the hole for the one that would not.
    #[test]
    fn a_terminal_call_must_say_what_it_is_doing() {
        for (kind, name, arguments) in [
            (DecisionKind::Overworld, "choose_action", r#"{"id":"PalletTown:5,6:Warp"}"#),
            (DecisionKind::Battle, "choose_battle_action", r#"{"id":"fight:Tackle"}"#),
            (DecisionKind::Nickname, "set_nickname", r#"{"name":"Bubbles"}"#),
            (DecisionKind::MartPurchase, "buy_item", "{}"),
            (DecisionKind::ForgetMove, "forget_move", r#"{"slot":1}"#),
            (DecisionKind::Overworld, "wait", r#"{"ticks":5}"#),
        ] {
            let CallKind::Rejected(complaint) = classify(kind, &bare(name, arguments), &[]) else {
                panic!("{name} on a {kind:?} turn must be made to say what it is doing");
            };
            assert!(complaint.contains("summary") && complaint.contains(name), "{complaint}");
            // …and the same call *with* one goes through, so the rule is the only thing being tested.
            assert!(
                matches!(classify(kind, &call(name, arguments), &[]), CallKind::Terminal(_)),
                "{name} with a summary is fine",
            );
        }
    }

    /// The tool that replaced the escape hatch on every turn that has a menu.
    ///
    /// ⚠️ **It must not end the turn**, and that is the whole design rather than an implementation
    /// detail: `press_buttons` was reached for on ordinary turns because it was the one way to
    /// finish a turn without choosing from the menu, so a terminal replacement would be the same
    /// tool under a new name. The model files the complaint and still has to decide.
    #[test]
    fn an_issue_report_does_not_end_the_turn_and_must_carry_a_message() {
        for kind in [DecisionKind::Overworld, DecisionKind::Battle, DecisionKind::Stuck] {
            assert!(offers_issue_report(kind));
            assert!(names(kind).contains(&REPORT_ISSUE), "{kind:?} offers it");
            assert!(
                !terminal_names(kind).contains(&REPORT_ISSUE),
                "{kind:?} must not let a turn end on it",
            );
            assert!(non_terminal_names(kind).contains(&REPORT_ISSUE), "the contract names it");

            let good = bare(REPORT_ISSUE, r#"{"message":"  the ladder is not in the menu  "}"#);
            let CallKind::Issue(message) = classify(kind, &good, &[]) else {
                panic!("{kind:?} should file it");
            };
            assert_eq!(message, "the ladder is not in the menu", "trimmed, like every other string");

            // ⚠️ Enforced, unlike `summary` used to be: a report is *only* its message.
            for arguments in ["{}", r#"{"message":"   "}"#] {
                let CallKind::Rejected(complaint) = classify(kind, &bare(REPORT_ISSUE, arguments), &[])
                else {
                    panic!("an empty report is not a report: {arguments}");
                };
                assert!(complaint.contains("message"), "{complaint}");
            }
        }

        // The single-question prompts have nothing for the agent to get wrong, so they do not carry
        // it — and a call there is named and explained rather than falling through to "no such tool".
        for kind in [DecisionKind::Nickname, DecisionKind::MartPurchase, DecisionKind::ForgetMove] {
            assert!(!offers_issue_report(kind));
            assert!(!names(kind).contains(&REPORT_ISSUE), "{kind:?} does not offer it");
            let call = bare(REPORT_ISSUE, r#"{"message":"x"}"#);
            let CallKind::Rejected(complaint) = classify(kind, &call, &[]) else {
                panic!("{kind:?} does not offer it and must say so");
            };
            assert!(complaint.contains(REPORT_ISSUE), "{complaint}");
        }
    }

    /// §7.5's first line of defence: the model cannot end a turn the wrong way because the wrong way
    /// is not in the array it was sent.
    #[test]
    fn terminal_tools_are_scoped_per_kind() {
        let overworld = names(DecisionKind::Overworld);
        let battle = names(DecisionKind::Battle);

        assert!(overworld.contains(&"choose_action"));
        assert!(!overworld.contains(&"choose_battle_action"));
        assert!(battle.contains(&"choose_battle_action"));
        assert!(!battle.contains(&"choose_action"));

        // The three menu prompts are single-question turns: their one terminal tool, and `wait`.
        // Offering `choose_action` at a naming screen would let a turn end in a way the poll site
        // cannot carry out.
        for kind in [DecisionKind::Nickname, DecisionKind::MartPurchase, DecisionKind::ForgetMove] {
            let offered = names(kind);
            for elsewhere in
                ["choose_action", "choose_battle_action", "use_field_move", "press_buttons", REPORT_ISSUE]
            {
                assert!(!offered.contains(&elsewhere), "{kind:?} must not offer {elsewhere}");
            }
        }
        assert!(names(DecisionKind::Nickname).contains(&"set_nickname"));
        assert!(names(DecisionKind::MartPurchase).contains(&"buy_item"));
        assert!(names(DecisionKind::ForgetMove).contains(&"forget_move"));
        // ⚠️ **`press_buttons` is not offered on a turn that has a menu at all**, which is the
        // change the deployed run bought: 738 of its 749 presses were overworld turns with a
        // perfectly good menu, and 91 of them were consecutive. What is offered instead is
        // `report_issue`, which says the menu is wrong *and leaves the model having to choose*.
        for kind in [DecisionKind::Overworld, DecisionKind::Battle] {
            assert!(!names(kind).contains(&"press_buttons"), "{kind:?} must not offer the hatch");
            assert!(names(kind).contains(&REPORT_ISSUE), "{kind:?} offers the replacement");
        }

        // **W9.** The watchdog's turn is the one where `press_buttons` is not a last resort but the
        // only resort: there is no menu, because the agent is not offering one. Anything else on
        // offer would be a turn ending in a decision a wedged agent cannot carry out.
        let stuck = names(DecisionKind::Stuck);
        assert_eq!(
            stuck.iter().filter(|name| terminal_names(DecisionKind::Stuck).contains(name)).count(),
            2,
        );
        assert!(stuck.contains(&"press_buttons") && stuck.contains(&"wait"));
        for elsewhere in ["choose_action", "choose_battle_action", "use_field_move", "set_nickname",
                          "buy_item", "forget_move"] {
            assert!(!stuck.contains(&elsewhere), "a stuck turn must not offer {elsewhere}");
        }
        // …and the reads are all there, because working out *why* it is stuck is the useful thing to
        // do before pressing anything.
        assert!(stuck.contains(&"read_map") && stuck.contains(&SCREENSHOT));
        assert!(!names(DecisionKind::Battle).contains(&"use_field_move"), "field moves are overworld-only");

        // ⚠️ **The battle-script tools are on the overworld turn and on no other, including the
        // battle turn they are about.** With an armed script there is no battle turn to carry them;
        // when one has failed, that turn is for winning the battle in front of you and the failure
        // is waiting in the next overworld situation regardless. It is also what kept Battle's
        // array where it was — see the ratchet in `the_tool_array_stays_within_its_budget`.
        for name in BATTLE_SCRIPT_TOOL_NAMES {
            assert!(names(DecisionKind::Overworld).contains(name), "the overworld turn writes the script");
            for elsewhere in [DecisionKind::Battle, DecisionKind::Nickname, DecisionKind::MartPurchase,
                              DecisionKind::ForgetMove, DecisionKind::Stuck] {
                assert!(!names(elsewhere).contains(name), "{elsewhere:?} must not offer {name}");
            }
        }

        for kind in KINDS {
            let offered = names(kind);
            assert!(offered.contains(&"wait"), "{kind:?} must always be able to wait");
            // The contract restated in the prompt has to match the array actually sent, or the two
            // drift and the model is told about a tool it does not have.
            for terminal in terminal_names(kind) {
                assert!(offered.contains(terminal), "{kind:?} promises {terminal} but does not offer it");
            }
            assert_eq!(
                offered.len(),
                reads_for(kind).count()
                    + TODO_TOOL_NAMES.len()
                    + match offers_battle_script(kind) { true => BATTLE_SCRIPT_TOOL_NAMES.len(), false => 0 }
                    + usize::from(offers_issue_report(kind))
                    + terminal_names(kind).len(),
                "a turn is offered its own reads, the TODO tools, the battle-script tools where \
                 they apply, `report_issue` where it applies, and its own terminal tools",
            );
        }
    }

    /// **The reads are scoped too, and the reason is not only tokens.** A tool that can only ever
    /// answer `null` — `read_battle` in the overworld, a map in a battle — is a round trip the model
    /// has to spend to find that out, and an invitation to spend it. A nickname prompt used to carry
    /// the whole catalogue in order to answer with one word.
    #[test]
    fn reads_are_scoped_per_kind_too() {
        assert!(!names(DecisionKind::Battle).contains(&"read_map"), "there is no map in a battle");
        assert!(!names(DecisionKind::Battle).contains(&READ_ROUTE));
        assert!(!names(DecisionKind::Overworld).contains(&"read_battle"), "it can only answer null");

        // ⚠️ The forget-move prompt legitimately fires mid-fight — it is the one menu kind that
        // pre-empts a battle turn — so which move to drop is a question the battle can answer.
        assert!(names(DecisionKind::ForgetMove).contains(&"read_battle"));

        // The screen is the only thing that can explain an unfamiliar menu or a wedged agent, so it
        // is the one read every kind keeps.
        for kind in KINDS {
            assert!(names(kind).contains(&SCREENSHOT), "{kind:?} cannot look at the screen");
        }

        // A single-question turn carries almost nothing. Stated as a number so that adding a read
        // back to every kind has to be a deliberate edit to this line: naming a Pokémon used to
        // arrive with all eight reads and four note tools — fourteen entries to answer with a word.
        assert_eq!(names(DecisionKind::Nickname), ["read_party", SCREENSHOT, "todo_set", "todo_complete",
                                                   "set_nickname", "wait"]);

        // ⚠️ A read that exists but is not offered *here* is told which turn it belongs to. Falling
        // through to "there is no tool called `read_map`" would be a lie the model cannot act on.
        let rejected = classify(DecisionKind::Battle, &call("read_map", "{}"), &[]);
        let CallKind::Rejected(complaint) = rejected else { panic!("read_map is not a battle read") };
        assert!(complaint.contains("not available in a battle turn"), "{complaint}");
        assert!(complaint.contains("read_battle"), "it has to name what *is* here: {complaint}");
        assert!(matches!(classify(DecisionKind::Overworld, &call("read_map", "{}"), &[]), CallKind::Read));
    }

    /// Every schema must be a JSON Schema object with the properties it claims — a malformed one is
    /// a 400 from the endpoint on the very first turn of a run.
    #[test]
    fn every_schema_is_a_well_formed_object() {
        for kind in KINDS {
            for tool in for_kind(kind) {
                let schema = &tool.function.parameters;
                assert_eq!(schema["type"], "object", "{}", tool.function.name);
                assert!(schema.get("properties").is_some(), "{}", tool.function.name);
                assert!(!tool.function.description.is_empty(), "{}", tool.function.name);
                for required in schema.get("required").and_then(Value::as_array).unwrap_or(&vec![]) {
                    let key = required.as_str().expect("required names are strings");
                    assert!(schema["properties"].get(key).is_some(),
                            "{} requires `{key}` but does not describe it", tool.function.name);
                }
            }
        }
    }

    /// Chaining: every id in a `then` is held to the same menu as the first, and an over-long chain
    /// is refused rather than quietly cut down to size.
    ///
    /// ⚠️ **The cheap alternative — take `then` on trust and let the policy find out — is the bug
    /// `not_on_the_menu` was written to close, one step further on.** A chain accepted here and
    /// rejected at the third hop has already carried out the first two, and reports the mistake in
    /// the *next* turn's situation rather than as a tool result this turn can still act on.
    #[test]
    fn every_id_in_a_chain_is_held_to_the_menu_the_turn_offered() {
        let menu = ["PalletTown:5,6:Warp".to_string(), "PalletTown:3,3:Mom".to_string()];
        let chain = |arguments: &str| classify(DecisionKind::Overworld, &call("choose_action", arguments), &menu);

        // The shape the whole feature is for, and both flags defaulted.
        let CallKind::Terminal(Terminal::ChooseAction { id, then, resume_after_battle }) = chain(
            r#"{"id":"PalletTown:5,6:Warp","then":["PalletTown:3,3:Mom"],"summary":"in, then talk"}"#,
        ) else {
            panic!("a chain of two ids from the menu is an ordinary call");
        };
        assert_eq!(id, "PalletTown:5,6:Warp");
        assert_eq!(then, ["PalletTown:3,3:Mom"]);
        // ⚠️ **Omitted means `true`, and it used to mean `false`.** A battle interrupting a walk
        // says nothing about the walk, and left opt-in the deployed run of 2026-09-01 never once
        // asked for it — so every wild encounter bought a fresh overworld turn describing a
        // situation whose action had already been chosen. The conservative direction is still there
        // and is now the one that has to be named.
        assert!(resume_after_battle, "a battle does not end the action unless the model says so");

        // ⚠️ The id is checked, not merely the count: a chained id from an earlier turn is exactly
        // the mistake `not_on_the_menu` exists for, and it must not be let through by being second.
        let CallKind::Rejected(complaint) = chain(
            r#"{"id":"PalletTown:5,6:Warp","then":["OaksLab:5,11:Warp"],"summary":"a stale id"}"#,
        ) else {
            panic!("a chained id that was never offered is refused");
        };
        assert!(complaint.contains("OaksLab:5,11:Warp"), "it names the one that failed: {complaint}");

        let CallKind::Rejected(complaint) = chain(&format!(
            r#"{{"id":"PalletTown:5,6:Warp","then":[{}],"summary":"far too many"}}"#,
            ["\"PalletTown:3,3:Mom\""; MAX_CHAINED_ACTIONS].join(","),
        )) else {
            panic!("a chain longer than the cap is refused");
        };
        assert!(complaint.contains(&MAX_CHAINED_ACTIONS.to_string()), "it says what the cap is: {complaint}");

        // Not a list of strings at all. One sentence, because there is one thing to fix.
        let CallKind::Rejected(complaint) =
            chain(r#"{"id":"PalletTown:5,6:Warp","then":"PalletTown:3,3:Mom","summary":"a bare string"}"#)
        else {
            panic!("`then` must be a list");
        };
        assert!(complaint.contains("list of further ids"), "{complaint}");

        let CallKind::Terminal(Terminal::ChooseAction { resume_after_battle, .. }) = chain(
            r#"{"id":"PalletTown:5,6:Warp","resume_after_battle":false,"summary":"stop and think"}"#,
        ) else {
            panic!("the flag is legal on its own");
        };
        assert!(!resume_after_battle, "and `false` is what turns it off");
    }

    /// A terminal call from the other kind is answerable, not fatal — and the answer names the tool
    /// that would have worked.
    #[test]
    fn a_terminal_tool_from_the_wrong_kind_is_rejected_with_the_right_one() {
        let CallKind::Rejected(complaint) =
            classify(DecisionKind::Battle, &call("choose_action", r#"{"id":"x"}"#), &[])
        else {
            panic!("choose_action must not end a battle turn");
        };
        assert!(complaint.contains("choose_battle_action"), "{complaint}");

        let CallKind::Rejected(complaint) = classify(DecisionKind::Overworld, &call("teleport", "{}"), &[]) else {
            panic!("an invented tool is rejected");
        };
        assert!(complaint.contains("teleport") && complaint.contains("read_map"), "{complaint}");
    }

    #[test]
    fn arguments_are_parsed_or_complained_about() {
        assert!(matches!(
            classify(DecisionKind::Overworld, &call("choose_action", r#"{"id":"PalletTown:5,6:Warp"}"#), &[]),
            CallKind::Terminal(Terminal::ChooseAction { ref id, .. }) if id == "PalletTown:5,6:Warp",
        ));
        assert!(matches!(
            classify(DecisionKind::Battle, &call("wait", r#"{"ticks":25}"#), &[]),
            CallKind::Terminal(Terminal::Wait { ticks: 25 }),
        ));
        // A model asking to sit out ten minutes of game time is stalling its own run.
        assert!(matches!(
            classify(DecisionKind::Battle, &call("wait", r#"{"ticks":99999}"#), &[]),
            CallKind::Terminal(Terminal::Wait { ticks: MAX_WAIT_TICKS }),
        ));
        assert!(matches!(classify(DecisionKind::Overworld, &call("wait", "{}"), &[]), CallKind::Rejected(_)));
        assert!(matches!(
            classify(DecisionKind::Overworld, &call("choose_action", r#"{"id":""}"#), &[]),
            CallKind::Rejected(_),
        ));
        assert!(matches!(
            classify(DecisionKind::Overworld, &call("choose_action", "not json at all"), &[]),
            CallKind::Rejected(_),
        ));
        // A zero-parameter read tool is routinely called with empty arguments rather than `{}`.
        assert!(matches!(classify(DecisionKind::Overworld, &call("read_map", ""), &[]), CallKind::Read));
    }

    /// A battle id must survive the thing that changes most often about a battle action: its PP,
    /// which `BattleAction`'s own `Display` includes.
    #[test]
    fn a_battle_id_ignores_the_volatile_parts() {
        use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
        let with_pp = |pp| BattleAction::Fight {
            slot: 0,
            battle_move: PokemonMove { name: PokemonMoveName::Tackle, pp },
        };
        assert_eq!(battle_id(&with_pp(35)), "fight:Tackle");
        assert_eq!(battle_id(&with_pp(1)), battle_id(&with_pp(35)));
        assert_ne!(format!("{}", with_pp(1)), format!("{}", with_pp(35)),
                   "…which the Display, and therefore a Display-keyed id, would not have");
        assert_eq!(battle_id(&BattleAction::Run), "run");
    }

    // ── W5 ───────────────────────────────────────────────────────────────────────────────────────

    /// A model writes `"HM01 Cut"`, the code writes `Hm01Cut`, and `read_bag` writes `Hm01Cut`. All
    /// three name the same thing, and rejecting two of them would be a rejection the model cannot
    /// learn its way out of — nothing it is shown spells the item any other way.
    #[test]
    fn names_are_matched_the_way_a_model_spells_them() {
        for spelling in ["Hm01Cut", "hm01_cut", "HM01 Cut", "hm01cut"] {
            assert_eq!(item_by_name(spelling), Some(ItemId::Hm01Cut), "{spelling}");
        }
        assert_eq!(item_by_name("Poke Ball"), Some(ItemId::PokeBall));
        assert_eq!(item_by_name("a potion of healing"), None, "close is not the same as right");
        assert_eq!(map_by_name("pallet town"), Some(Map::PalletTown));
        assert_eq!(button_by_name("START"), Some(JoypadButton::Start));
        assert_eq!(facing_by_name("up"), Some(PlayerFacingDirection::Up));
        assert_eq!(button_by_name("shoulder"), None);
    }

    /// The escape hatch has to reject what it cannot press rather than silently drop it — a queue
    /// that is one button short walks the player somewhere nobody asked for.
    ///
    /// ⚠️ **`Stuck` throughout, because that is the only kind that offers the tool now.** Written
    /// against `Overworld`, every case below would pass for the wrong reason: rejected because the
    /// turn has a menu, never reaching the argument parsing this is about.
    /// **The 59.** The deployed run chose `ViridianCity:33,8:Warp` while standing in
    /// `ViridianPokecenter` — an id it had read several turns earlier — 59 times in 934
    /// `choose_action` decisions. Every one was accepted here, published as a decision, and refused
    /// by `resolve_overworld` a thread later, which is a whole turn paid for and nothing moved.
    #[test]
    fn an_id_the_turn_never_offered_is_refused_before_it_costs_the_turn() {
        let menu = [
            "ViridianPokecenter:3,7:Warp".to_string(),
            "ViridianPokecenter:5,3:Nurse".to_string(),
        ];
        let chose = |id: &str| call("choose_action", &format!(r#"{{"id":"{id}","summary":"s"}}"#));

        assert!(
            matches!(
                classify(DecisionKind::Overworld, &chose("ViridianPokecenter:3,7:Warp"), &menu),
                CallKind::Terminal(Terminal::ChooseAction { .. }),
            ),
            "an id from this turn's own menu is the decision it always was",
        );

        let CallKind::Rejected(complaint) =
            classify(DecisionKind::Overworld, &chose("ViridianCity:33,8:Warp"), &menu)
        else {
            panic!("an id for a map the player is not on can never resolve");
        };
        // ⚠️ The complaint has to name the *right* mistake. "The game moved on" — which is what the
        // policy used to say — sends the model back to try the same id again.
        assert!(complaint.contains("ViridianCity") && complaint.contains("ViridianPokecenter"),
                "it must say which map the id is for and which map the player is on; got {complaint}");
        assert!(complaint.contains("ViridianPokecenter:3,7:Warp"),
                "and it must repeat what can be chosen instead; got {complaint}");

        // ⚠️ An empty menu checks nothing: `Nickname`, `ForgetMove` and `Stuck` have no menu, and a
        // check that read that as "nothing is allowed" would reject every answer they give.
        assert!(
            matches!(
                classify(DecisionKind::Overworld, &chose("anything at all"), &[]),
                CallKind::Terminal(_),
            ),
            "with no menu to check against, the id is the policy's to resolve as it always was",
        );
    }

    #[test]
    fn press_buttons_parses_a_sequence_and_refuses_what_is_not_one() {
        let press = |arguments: &str| call("press_buttons", arguments);
        let CallKind::Terminal(Terminal::PressButtons { buttons }) = classify(
            DecisionKind::Stuck,
            &press(r#"{"buttons":["b","b","start"],"why":"a menu nothing closes"}"#),
            &[],
        ) else {
            panic!("a list of real buttons is a decision");
        };
        assert_eq!(buttons, [JoypadButton::B, JoypadButton::B, JoypadButton::Start]);

        for bad in [r#"{"buttons":["b","x"],"why":"w"}"#, r#"{"buttons":[],"why":"w"}"#, r#"{"why":"w"}"#] {
            assert!(
                matches!(classify(DecisionKind::Stuck, &press(bad), &[]), CallKind::Rejected(_)),
                "{bad} should have been rejected",
            );
        }
        // …and the agent's own cap is the cap here, so a runaway list is trimmed rather than
        // half-delivered by a queue that silently stops accepting.
        let many = format!(r#"{{"buttons":{},"why":"a menu nothing closes"}}"#,
            serde_json::to_string(&vec!["a"; MANUAL_INPUT_CAPACITY * 2]).unwrap());
        let CallKind::Terminal(Terminal::PressButtons { buttons }) =
            classify(DecisionKind::Stuck, &press(&many), &[])
        else {
            panic!("an over-long list is still a decision");
        };
        assert_eq!(buttons.len(), MANUAL_INPUT_CAPACITY);
    }

    /// `screenshot` is a read as far as the turn contract goes, but it never reaches the emulator
    /// thread — the worker answers it. Classifying it as an ordinary `Read` would send it to the
    /// policy, which has no idea what to do with it and would answer "not a read tool".
    #[test]
    fn a_screenshot_is_classified_apart_from_the_other_reads() {
        assert!(matches!(classify(DecisionKind::Battle, &call(SCREENSHOT, "{}"), &[]), CallKind::Screenshot));
        assert!(matches!(classify(DecisionKind::Battle, &call("read_party", "{}"), &[]), CallKind::Read));
        assert!(READ_TOOLS.iter().any(|tool| tool.name == SCREENSHOT), "it is still offered as a read");
    }

    /// Every argument shape `use_field_move` accepts, and the complaint each malformed one earns.
    #[test]
    fn a_field_move_call_parses_into_the_move_it_names() {
        let parse = |arguments: &str| classify(DecisionKind::Overworld, &call("use_field_move", arguments), &[]);
        let request = |arguments: &str| match parse(arguments) {
            CallKind::Terminal(Terminal::UseFieldMove(request)) => request,
            CallKind::Rejected(complaint) => panic!("{arguments} was rejected: {complaint}"),
            _ => panic!("{arguments} did not end the turn"),
        };

        assert_eq!(request(r#"{"move":"cut"}"#), FieldMoveRequest::Cut);
        assert_eq!(
            request(r#"{"move":"strength"}"#),
            FieldMoveRequest::PartyMove { name: PokemonMoveName::Strength, slot: None },
            "an omitted slot means 'whoever knows it', not slot 0",
        );
        assert_eq!(
            request(r#"{"move":"flash","slot":2}"#),
            FieldMoveRequest::PartyMove { name: PokemonMoveName::Flash, slot: Some(2) },
        );
        assert_eq!(request(r#"{"move":"fly","map":"Pewter City"}"#), FieldMoveRequest::Fly { to: Map::PewterCity });
        assert_eq!(
            request(r#"{"move":"teach","item":"Hm03Surf","slot":0}"#),
            FieldMoveRequest::Teach { item: ItemId::Hm03Surf, slot: 0 },
        );
        assert_eq!(
            request(r#"{"move":"use_item","item":"PokeFlute","target":{"x":12,"y":9}}"#),
            FieldMoveRequest::UseItem { item: ItemId::PokeFlute, target: Point8 { x: 12, y: 9 } },
        );
        assert_eq!(
            request(r#"{"move":"push_boulder","target":{"x":4,"y":5},"direction":"left"}"#),
            FieldMoveRequest::PushBoulder { boulder: Point8 { x: 4, y: 5 }, direction: JoypadButton::Left },
        );
        assert_eq!(
            request(r#"{"move":"interact","target":{"x":1,"y":2},"facing":"up"}"#),
            FieldMoveRequest::Interact {
                target: Point8 { x: 1, y: 2 },
                facing: Some(PlayerFacingDirection::Up),
            },
        );
        assert_eq!(request(r#"{"move":"reorder_party","slot":3}"#), FieldMoveRequest::ReorderParty { slot: 3 });

        // Every one of these is answerable — the model is told what is missing and can try again in
        // the same turn, which is the whole reason a bad call is a tool result and not a dead turn.
        for (arguments, expected) in [
            (r#"{"move":"teleportation"}"#, "not one of the field moves"),
            (r#"{"move":"fly","map":"Atlantis"}"#, "is not a map"),
            (r#"{"move":"teach","item":"Hm03Surf"}"#, "needs a `slot`"),
            (r#"{"move":"toss_item","item":"Sandwich"}"#, "is not an item"),
            (r#"{"move":"use_item","item":"PokeFlute"}"#, "needs a `target`"),
            (r#"{"move":"push_boulder","target":{"x":1,"y":1},"direction":"north"}"#, "is not a direction"),
            (r#"{"move":"reorder_party","slot":9}"#, "no party slot 9"),
            ("{}", "`move` is required"),
        ] {
            let CallKind::Rejected(complaint) = parse(arguments) else {
                panic!("{arguments} should have been rejected");
            };
            assert!(complaint.contains(expected), "{arguments} → {complaint}");
        }
    }

    /// The menu-prompt tools, whose whole subtlety is that *omitting* the argument is a real answer
    /// rather than a malformed call: no nickname, no purchase, no move forgotten.
    #[test]
    fn omitting_the_argument_is_an_answer_for_the_three_menu_prompts() {
        let parse = |kind, name, arguments: &str| classify(kind, &call(name, arguments), &[]);

        assert!(matches!(
            parse(DecisionKind::Nickname, "set_nickname", "{}"),
            CallKind::Terminal(Terminal::SetNickname { name: None }),
        ));
        // An empty buffer is how the naming screen itself says "keep the default", so agreeing with
        // it here means a blank `name` and an absent one cannot mean different things.
        assert!(matches!(
            parse(DecisionKind::Nickname, "set_nickname", r#"{"name":"   "}"#),
            CallKind::Terminal(Terminal::SetNickname { name: None }),
        ));
        let CallKind::Terminal(Terminal::SetNickname { name: Some(name) }) =
            parse(DecisionKind::Nickname, "set_nickname", r#"{"name":"ABCDEFGHIJKLMNOP"}"#)
        else {
            panic!("a name is a name");
        };
        assert_eq!(name.chars().count(), MAX_NICKNAME, "the buffer is {MAX_NICKNAME} characters");

        assert!(matches!(
            parse(DecisionKind::MartPurchase, "buy_item", "{}"),
            CallKind::Terminal(Terminal::BuyItem { item: None, .. }),
        ));
        assert_eq!(
            match parse(DecisionKind::MartPurchase, "buy_item", r#"{"item":"Potion","quantity":4}"#) {
                CallKind::Terminal(Terminal::BuyItem { item, .. }) => item,
                _ => panic!("a stocked item is a purchase"),
            },
            Some(BagItem::new(ItemId::Potion, 4)),
        );
        // An omitted quantity is one, not zero — zero would be an order the mart silently refuses.
        assert!(matches!(
            parse(DecisionKind::MartPurchase, "buy_item", r#"{"item":"Potion"}"#),
            CallKind::Terminal(Terminal::BuyItem { item: Some(BagItem { quantity: 1, .. }), .. }),
        ));

        // ⚠️ **A chain is parsed whole before any of it is spent.** Gen 1 takes the money order by
        // order, so a `then` accepted here and refused on its third entry has already bought the
        // first two and can only complain about it on the *next* turn — the `chosen_actions` bug one
        // shop along. Both failure shapes are rejections that buy nothing.
        assert_eq!(
            match parse(DecisionKind::MartPurchase, "buy_item",
                        r#"{"item":"Potion","then":[{"item":"PokeBall","quantity":10}]}"#) {
                CallKind::Terminal(Terminal::BuyItem { then, .. }) => then,
                _ => panic!("a chained order is a purchase"),
            },
            vec![BagItem::new(ItemId::PokeBall, 10)],
        );
        assert!(matches!(
            parse(DecisionKind::MartPurchase, "buy_item",
                  r#"{"item":"Potion","then":[{"item":"Nonsense"}]}"#),
            CallKind::Rejected(_),
        ), "a name in the tail is checked exactly as the head is");
        assert!(matches!(
            parse(DecisionKind::MartPurchase, "buy_item",
                  r#"{"item":"Potion","then":[{"item":"Potion"},{"item":"Potion"},{"item":"Potion"},{"item":"Potion"}]}"#),
            CallKind::Rejected(_),
        ), "over-length is a rejection, not a truncation");

        assert!(matches!(
            parse(DecisionKind::ForgetMove, "forget_move", "{}"),
            CallKind::Terminal(Terminal::ForgetMove { slot: None }),
        ));
        assert!(matches!(
            parse(DecisionKind::ForgetMove, "forget_move", r#"{"slot":2}"#),
            CallKind::Terminal(Terminal::ForgetMove { slot: Some(2) }),
        ));
        assert!(
            matches!(parse(DecisionKind::ForgetMove, "forget_move", r#"{"slot":7}"#), CallKind::Rejected(_)),
            "a Pokémon has four move slots, and a cursor sent to a fifth never arrives",
        );
    }

    /// Resolution against a real game, which is where the checks that need the party and the bag
    /// live. `cut` is the sharp one: the driver cuts whatever is in front of the player, so a `cut`
    /// issued from the wrong tile opens a menu, achieves nothing and looks like the emulator hanging.
    #[test]
    fn a_field_move_is_resolved_against_the_party_and_the_bag_it_needs() {
        // Oak's lab: one Pokémon, no HMs, no trees.
        let state = fixture_state();
        let complaint = |request| match resolve_field_move(&state, &request) {
            Err(complaint) => complaint,
            Ok(resolved) => panic!("{request:?} should not have resolved to {resolved:?}"),
        };
        assert!(complaint(FieldMoveRequest::ReorderParty { slot: 3 }).contains("no party member in slot 3"));
        assert!(complaint(FieldMoveRequest::TossItem { item: ItemId::Hm01Cut }).contains("no Hm01Cut in the bag"));

        // The tile check is the one `cut` exists for, and it only becomes reachable once the HM gate
        // above it is satisfied — so it is asserted on a state that *can* cut.
        let mut able = fixture_state();
        able.badges |= crate::pokemon::badge::Badge::CascadeBadge;
        able.pokemon.get_mut(0).expect("the fixture has a starter").moves[1] =
            Some(PokemonMove::with_max_pp(PokemonMoveName::Cut));
        match resolve_field_move(&able, &FieldMoveRequest::Cut) {
            Err(complaint) => assert!(complaint.contains("facing"), "cut must check the tile in front: {complaint}"),
            Ok(resolved) => panic!("nothing in Oak's lab is a tree, but cut resolved to {resolved:?}"),
        }

        // …and the one that needs nothing but its own HM resolves as itself.
        let mut flier = fixture_state();
        flier.badges |= crate::pokemon::badge::Badge::ThunderBadge;
        flier.pokemon.get_mut(0).expect("the fixture has a starter").moves[1] =
            Some(PokemonMove::with_max_pp(PokemonMoveName::Fly));
        assert_eq!(
            resolve_field_move(&flier, &FieldMoveRequest::Fly { to: Map::PalletTown }),
            Ok(FieldMove::Fly { to: Map::PalletTown }),
        );
        assert_eq!(
            resolve_field_move(&state, &FieldMoveRequest::ReorderParty { slot: 0 }),
            Ok(FieldMove::ReorderParty { slot: 0 }),
        );
    }

    /// **The machine gate, which is the HM gate one menu further in.**
    ///
    /// A TM or HM aimed at a Pokémon outside its learnset is answered with
    /// `MonCannotLearnMachineMoveText` and `jr .chooseMon` (`engine/items/item_effects.asm`): back to
    /// the party menu, cursor untouched. `TeachingMove` has no exit from that, so the attempt is 60 s
    /// of A-mashing ended by `DRIVER_ESCAPE_SILENCE` and the model, told only that the game stopped
    /// answering, asks for the same teach again. The deployed run of 2026-08-27 lived in that loop.
    ///
    /// ⚠️ **What the complaint has to carry is the alternative, not the refusal.** The decision on
    /// the table is which party member takes the machine, so the sentence answers that; when nobody
    /// can, it says so outright rather than leaving it to be read out of a missing list.
    #[test]
    fn a_machine_no_one_in_the_party_can_learn_is_refused_here_instead() {
        use crate::pokemon::pokemon::Pokemon;
        use crate::pokemon::species::PokemonSpecies;
        let with_hm = |item: ItemId, party: &[PokemonSpecies]| {
            let mut state = fixture_state();
            state.bag.push(BagItem { id: item, quantity: 1 }).expect("the fixture's bag has room");
            state.pokemon = Default::default();
            for species in party {
                state.pokemon.push(Pokemon::maxed(*species, "MON", [PokemonMoveName::Tackle; 4], "AI", 1))
                    .expect("six is the limit and these are fewer");
            }
            state
        };
        let complaint = |state: &GameState, request| match resolve_field_move(state, &request) {
            Err(complaint) => complaint,
            Ok(resolved) => panic!("{request:?} should not have resolved to {resolved:?}"),
        };

        // Somebody can: the answer is which slot, which is the whole of what the model needs.
        let mixed = with_hm(ItemId::Hm01Cut, &[PokemonSpecies::Venusaur, PokemonSpecies::Pidgey]);
        let wrong_slot = complaint(&mixed, FieldMoveRequest::Teach { item: ItemId::Hm01Cut, slot: 1 });
        assert!(wrong_slot.contains("cannot learn Cut"), "{wrong_slot}");
        assert!(wrong_slot.contains("slot 0"), "it has to name who can: {wrong_slot}");

        // …and the slot that can resolves, so the gate is not simply refusing every teach.
        assert_eq!(
            resolve_field_move(&mixed, &FieldMoveRequest::Teach { item: ItemId::Hm01Cut, slot: 0 }),
            Ok(FieldMove::TeachMove { item: ItemId::Hm01Cut, target_slot: 0 }),
        );

        // Nobody can, which is the case the deployed run was actually in: no slot to redirect to, so
        // the sentence has to say that a different Pokémon is the only way past.
        let none = with_hm(ItemId::Hm01Cut, &[PokemonSpecies::Pidgey, PokemonSpecies::Zubat]);
        let hopeless = complaint(&none, FieldMoveRequest::Teach { item: ItemId::Hm01Cut, slot: 0 });
        assert!(hopeless.contains("nor can anything else in the party"), "{hopeless}");
        assert!(!hopeless.contains("In the party,"), "there is nobody to name: {hopeless}");

        // ⚠️ Not a machine, so not a question: a stone rides the same menu chain and a check written
        // for TMs must not refuse it. Eevee is in no learnset this test would find.
        let stone = with_hm(ItemId::WaterStone, &[PokemonSpecies::Eevee]);
        assert_eq!(
            resolve_field_move(&stone, &FieldMoveRequest::Evolve { stone: ItemId::WaterStone, slot: 0 }),
            Ok(FieldMove::EvolveWithStone { stone: ItemId::WaterStone, target_slot: 0,
                                            evolve_from: PokemonSpecies::Eevee }),
        );
    }

    /// **The item gate, which is the machine gate one bag row along.**
    ///
    /// `ItemUsePtrTable` sends most of the key items to `UnusableItem`, which is `jp ItemUseNotTime`:
    /// "This isn't the time to use that!" and back to the bag list, cursor untouched.
    /// `UsingFieldItem` has no exit from that either, so a `use_item` on one is 60 s of A-mashing
    /// ended by `DRIVER_ESCAPE_SILENCE`. The deployed run of 2026-08-27 spent turn after turn on it
    /// in Mt Moon, because the Rocket standing there says "if you find a fossil, give it to me" and
    /// that is flavour rather than a handoff.
    ///
    /// ⚠️ **Refused here it costs no round trip**, and the sentence has to say the thing the model
    /// cannot work out for itself: that the item is carried rather than used, so there is nothing to
    /// retry. `crate::pokemon::item_use` reads the table rather than transcribing it, and this test
    /// is about the gate rather than the list.
    #[test]
    fn an_item_the_game_will_never_use_is_refused_here_instead() {
        let holding = |item: ItemId| {
            let mut state = fixture_state();
            state.bag.push(BagItem { id: item, quantity: 1 }).expect("the fixture's bag has room");
            state
        };
        let at = Point8 { x: 5, y: 6 };
        let complaint = |state: &GameState, item: ItemId| {
            match resolve_field_move(state, &FieldMoveRequest::UseItem { item, target: at }) {
                Err(complaint) => complaint,
                Ok(resolved) => panic!("{item} should not have resolved to {resolved:?}"),
            }
        };

        let fossil = complaint(&holding(ItemId::HelixFossil), ItemId::HelixFossil);
        assert!(fossil.contains("no bag use for HelixFossil"), "{fossil}");
        assert!(fossil.contains("carry"), "it has to say what to do instead: {fossil}");
        assert!(complaint(&holding(ItemId::SilphScope), ItemId::SilphScope).contains("no bag use"));

        // A ball is the same `ItemUseNotTime` by a different route, and the alternative is a tool
        // rather than a shrug.
        let ball = complaint(&holding(ItemId::PokeBall), ItemId::PokeBall);
        assert!(ball.contains("choose_battle_action"), "{ball}");

        // A machine never reaches the table at all (`cp HM01 / jp nc, ItemUseTMHM`) and has its own
        // tool, so it is pointed at `teach` rather than called unusable.
        let machine = complaint(&holding(ItemId::Hm01Cut), ItemId::Hm01Cut);
        assert!(machine.contains("teach"), "{machine}");

        // ⚠️ **The gate is not "refuse every key item"**: the Poké Flute is a key item, is
        // `ItemUsePokeFlute`, and is the one use the scripted route actually makes. A check that
        // refused it would break Snorlax.
        assert!(ItemId::PokeFlute.is_key_item(), "the point of the case");
        assert_eq!(
            resolve_field_move(&holding(ItemId::PokeFlute), &FieldMoveRequest::UseItem { item: ItemId::PokeFlute, target: at }),
            Ok(FieldMove::UseFieldItem { item: ItemId::PokeFlute, target: at }),
        );

        // And an item that is not in the bag is still the bag's complaint, not the gate's: the
        // `held` check runs first, so "you do not have one" beats "it would not work".
        let empty = complaint(&fixture_state(), ItemId::HelixFossil);
        assert!(empty.contains("no HelixFossil in the bag"), "{empty}");
    }

    /// **The HM gate, and why it is worth a test of its own.**
    ///
    /// Every HM field move is used from the party menu, and pokered answers a missing badge with
    /// `jp .loop` — the same menu, cursor untouched. The agent's driver has no exit condition for
    /// that, so a `cut` with no Cut is sixty seconds of A-mashing ended only by
    /// `DRIVER_ESCAPE_SILENCE`. The deployed run did it eleven times on Route 2 with no badges at
    /// all and filed two `report_issue`s calling the game broken.
    ///
    /// ⚠️ **Both halves are checked and they are different complaints**, because they need different
    /// things done about them: find the HM, or go and win a gym.
    #[test]
    fn an_hm_the_game_would_refuse_is_refused_here_instead() {
        let none = fixture_state();
        let complaint = |state: &GameState, request| match resolve_field_move(state, &request) {
            Err(complaint) => complaint,
            Ok(resolved) => panic!("{request:?} should not have resolved to {resolved:?}"),
        };

        // Neither the move nor the badge — the deployed run's exact position on Route 2.
        let both = complaint(&none, FieldMoveRequest::Cut);
        assert!(both.contains("no Pokémon in the party knows it"), "{both}");
        assert!(both.contains("CascadeBadge"), "{both}");

        // The move but not the badge: the gym is the thing to go and do.
        let mut taught = fixture_state();
        taught.pokemon.get_mut(0).expect("the fixture has a starter").moves[1] =
            Some(PokemonMove::with_max_pp(PokemonMoveName::Cut));
        let unbadged = complaint(&taught, FieldMoveRequest::Cut);
        assert!(unbadged.contains("CascadeBadge"), "{unbadged}");
        assert!(!unbadged.contains("knows"), "the move is known; only the badge is missing: {unbadged}");

        // The badge but not the move: the HM is the thing to go and find.
        let mut badged = fixture_state();
        badged.badges |= crate::pokemon::badge::Badge::CascadeBadge;
        let untaught = complaint(&badged, FieldMoveRequest::Cut);
        assert!(untaught.contains("HM"), "{untaught}");
        assert!(!untaught.contains("CascadeBadge"), "the badge is held: {untaught}");

        // Every HM the game gates, not just the one that broke.
        for (name, badge) in HM_BADGES {
            let request = match name {
                PokemonMoveName::Cut => FieldMoveRequest::Cut,
                PokemonMoveName::Fly => FieldMoveRequest::Fly { to: Map::PalletTown },
                other => FieldMoveRequest::PartyMove { name: *other, slot: None },
            };
            assert!(
                complaint(&none, request).contains(&badge.to_string()),
                "{name} must name the {badge} it needs",
            );
        }

        // A boulder needs Strength *armed*, which is a third thing again: the move, the badge, and
        // then a trip through the party menu on this map. A push before that moves nothing at all.
        let mut strong = fixture_state();
        strong.badges |= crate::pokemon::badge::Badge::RainbowBadge;
        strong.pokemon.get_mut(0).expect("the fixture has a starter").moves[1] =
            Some(PokemonMove::with_max_pp(PokemonMoveName::Strength));
        assert!(!strong.strength_active, "the fixture has not armed Strength");
        let unarmed = complaint(&strong, FieldMoveRequest::PushBoulder {
            boulder: Point8 { x: 1, y: 1 },
            direction: JoypadButton::Up,
        });
        assert!(unarmed.contains("not armed"), "{unarmed}");
    }

    /// **The naming screen asks for a name, and takes only names the cartridge can write.**
    ///
    /// Two separate faults, and the second was made likelier by fixing the first. The tool used to
    /// say that keeping the default "is the ordinary answer", and across the two deployed runs all
    /// four naming screens took it — *"Keep the default name ZUBAT for the newly caught Pokémon"* —
    /// which makes the whole decision kind a round trip nobody needed to buy.
    ///
    /// ⚠️ **And the name is written straight into the naming screen's buffer**, so nothing between
    /// here and RAM checks it: `PokemonString::from_string` maps a character it does not know to
    /// `0x00`, which is not the terminator (`0x50`) but a control byte, so an accented letter or an
    /// emoji does not fail — it names the Pokémon something unreadable for the rest of the run.
    #[test]
    fn a_nickname_is_asked_for_and_has_to_be_one_the_game_can_write() {
        let spec = for_kind(DecisionKind::Nickname);
        let described = spec.iter().find(|tool| tool.function.name == "set_nickname")
            .map(|tool| tool.function.description.clone())
            .expect("the naming turn offers set_nickname");
        assert!(described.contains("nickname"), "{described}");
        assert!(!described.contains("ordinary answer"),
                "the tool must not talk the model out of the one thing this turn is for: {described}");

        let parse = |arguments: &str| classify(DecisionKind::Nickname, &call("set_nickname", arguments), &[]);
        let named = |name: &str| parse(&json!({"name": name}).to_string());
        assert!(matches!(named("Rocky"), CallKind::Terminal(Terminal::SetNickname { name: Some(_) })));
        // `/` is `$F3` and a space is `$7F` — "is it alphanumeric" is the wrong question, which is
        // why the check round-trips through the charmap instead of listing characters twice.
        assert!(matches!(named("MT/MOON"), CallKind::Terminal(Terminal::SetNickname { name: Some(_) })));
        assert!(
            matches!(named("Poké"), CallKind::Rejected(_)),
            "an accented letter has no byte in this charmap and must not reach the buffer",
        );
        assert!(matches!(named("🔥"), CallKind::Rejected(_)));
        // An omitted or blank name is still the decline, because the naming screen reads an empty
        // buffer as one — the two must not be able to disagree.
        assert!(matches!(
            parse("{}"),
            CallKind::Terminal(Terminal::SetNickname { name: None }),
        ));
        assert!(matches!(named("   "), CallKind::Terminal(Terminal::SetNickname { name: None })));
    }

    /// **A tree nobody can cut is not an action.**
    ///
    /// A `:CutTree` row is a walk that ends *facing* a tree and nothing else — the cut itself is a
    /// separate `use_field_move` — so offered without Cut it is a menu entry whose only follow-up
    /// the game refuses. The deployed run took it eleven times on Route 2 with no badges at all.
    ///
    /// ⚠️ **The gate is on the map rather than in the menu builder** (`MetaTileMap::can_cut`, set by
    /// `game_state()` alongside `can_surf`), so the scripted policy is held to it too: its own
    /// `PolicyStep::CutTree` fallback used to be able to push a step that could never complete.
    #[test]
    fn a_cut_tree_is_not_offered_to_a_party_that_cannot_cut() {
        let mut gb = crate::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/at-vermilion.bin"))
            .expect("the committed fixture loads");
        let mut state = { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }
            .expect("the fixture has a readable state");
        assert!(
            state.map.meta_tiles.contains(&crate::pokemon::tile::MetaTile::CutTree),
            "this fixture's map has to have a tree on it for the test to mean anything",
        );

        let cut_rows = |state: &GameState| -> usize {
            overworld_menu(state, None).iter().filter(|item| item.id.ends_with(":CutTree")).count()
        };
        assert!(!state.can_use_cut, "the fixture reaches Vermilion before the HM");
        assert_eq!(cut_rows(&state), 0, "a tree is not an action without Cut");

        state.map.can_cut = true;
        assert!(cut_rows(&state) > 0, "and it is one with Cut — or this test would pass by accident");
    }

    /// ⚠️ The party menu lists a mon's field moves in **its own move-slot order**, so the index of
    /// the one being asked for depends on what else that mon knows. Assuming zero works for an HM
    /// slave and silently uses the wrong move for anything else.
    #[test]
    fn a_party_field_moves_index_is_computed_from_the_moves_it_knows() {
        let mut state = fixture_state();
        // The HM gate sits above the index arithmetic, so this mon has to be able to use Strength
        // at all before the index is the thing under test.
        state.badges |= crate::pokemon::badge::Badge::RainbowBadge;
        state.pokemon.get_mut(0).expect("the fixture has a starter").moves = [
            Some(PokemonMove::with_max_pp(PokemonMoveName::Tackle)),
            Some(PokemonMove::with_max_pp(PokemonMoveName::Cut)),
            Some(PokemonMove::with_max_pp(PokemonMoveName::Strength)),
            None,
        ];

        assert_eq!(
            resolve_field_move(&state, &FieldMoveRequest::PartyMove {
                name: PokemonMoveName::Strength,
                slot: None,
            }),
            // Cut is a field move and sits in an earlier move slot, so Strength is the *second* row
            // of the field-move box — not the third, and not the first.
            Ok(FieldMove::UseFieldMove { slot: 0, move_index: 1 }),
        );
    }
}
