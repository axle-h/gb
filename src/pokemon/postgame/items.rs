//! Workstream **I — the rest of the item-use table**. See `docs/postgame-coverage-plan.md` §8-I.
//!
//! The checklist is not a walkthrough, it is the ROM's own dispatch table: `ItemUsePtrTable` in
//! `engine/items/item_effects.asm` has one entry per distinct effect, and this module covers the
//! entries that had no driver — medicine out of battle (I1), the PP restores and PP Up (I2), the
//! in-battle stat items (I3), the Poké Doll (I4), the Repel family (I5), the Bicycle (I6) and the
//! Itemfinder (I7).
//!
//! # One driver, four menu shapes
//!
//! §8-I says all of I1/I2/I7 ride the START → ITEM → bag → USE chain that `TeachMove`,
//! `EvolveWithStone` and `UseRareCandy` already drive, and that the work is "mostly the extra menu
//! each item opens". That is right, and the shapes are **not** guessable from the item — they come
//! from two flat lists in `data/items/use_party.asm` and `data/items/use_overworld.asm` that
//! `StartMenu_Item` matches against (`engine/menus/start_sub_menus.asm:386-410`):
//!
//! ```text
//! Bicycle       START → ITEM → row → A                       ⚠️ NO USE/TOSS menu (special-cased at
//!                                                               :341) and it CLOSES the start menu
//! Itemfinder    START → ITEM → row → A → USE → text          `UsableItems_CloseMenu` → overworld
//! Repel         START → ITEM → row → A → USE → text          neither list → back to the BAG, B out
//! Potion/Revive START → ITEM → row → A → USE → party → mon   `UsableItems_PartyMenu` → back to the BAG
//! Ether/PP Up   …as above, then one more: a MOVE menu
//! ```
//!
//! So "where does it leave you" is three different answers and each needs a different completion
//! test. [`Effect`] is that distinction, and it is derived from the item rather than passed in.
//!
//! # The move menu is 1-indexed
//!
//! `ItemUsePPRestore` opens `MoveSelectionMenu` with `wMoveMenuType = 2`, the *relearn* layout
//! (`engine/battle/core.asm:2519-2531`): `wTopMenuItemX = 5`, `wTopMenuItemY = 7`, and the moves are
//! drawn from row 8 — so cursor index **1** is the first move, exactly like the battle move list
//! (which `menu.rs` already documents as 1-indexed). Targeting `move_index` directly restores the PP
//! of the *previous* move, silently, and a driver watching "some move's PP went up" would not notice.
//!
//! # The failure that reads like success
//!
//! ⚠️ A potion on a full-HP mon, an Antidote on an unpoisoned one, an Ether on a full-PP move: all
//! print *"It won't have any effect."* (`.healingItemNoEffect`), leave
//! `wActionResultOrTookBattleTurn = 0` and **do not consume the item**. From the driver's side that
//! is indistinguishable from a use that has not happened yet, so it retries for ever — the same
//! family as the full-bag trap in §10. [`blocked`] is the answer: the policy refuses to issue a use
//! whose effect the game would decline, and says which precondition failed.

use crate::joypad::JoypadButton;
use crate::mmu::MMU;
use crate::pokemon::agent::{start_menu_row, AgentEvent, AgentState, PokemonAgent, StartMenuRow};
use crate::pokemon::encoding::GameMode;
use crate::pokemon::item::ItemId;
use crate::pokemon::map::Map;
use crate::pokemon::menu::{TextBoxId, START_MENU_ORIGIN};
use crate::pokemon::policy::{FieldMove, PolicyStep};
use crate::pokemon::status::PokemonStatus;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::{GameState, PokemonApi, PokemonApiTrait};

/// What a bag item is used **on** — i.e. how many menus the chain has after `USE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseTarget {
    /// Nothing: the Repel family, the Bicycle, the Itemfinder. The chain ends at `USE`.
    Nothing,
    /// A party member — medicine and the vitamins. One extra menu.
    Party { slot: u8 },
    /// A party member's move — the PP restores and PP Up. Two extra menus, and the second is the
    /// 1-indexed one (see the module docs). `move_index` is the **move slot**, 0–3.
    Move { slot: u8, move_index: u8 },
}

impl UseTarget {
    /// The party slot this use lands on, if any — what the completion tests read.
    pub const fn slot(self) -> Option<u8> {
        match self {
            Self::Nothing => None,
            Self::Party { slot } | Self::Move { slot, .. } => Some(slot),
        }
    }
}

/// Where an item's effect shows up, which is also where the menus leave you.
///
/// Derived from the item, not chosen by the caller: the ROM's own two lists decide it, and getting
/// it wrong is a driver that never finishes rather than one that misbehaves visibly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// The item is used up. True of everything except the two key items below, and the only
    /// completion test that needs no extra RAM read: `RemoveUsedItem` is the ROM's own "it worked".
    Consumed,
    /// The Bicycle: `ItemUseBicycle` **toggles** `wWalkBikeSurfState` between 0 and 1, so the
    /// completion test is "it changed", not "it is 1". Getting that backwards makes the dismount
    /// unexpressible — the step would be satisfied before it started and pop without pressing
    /// anything. Never consumed; it is a key item.
    TogglesBicycle,
    /// The Itemfinder: a text box and nothing else. Never consumed, and there is no RAM flag to
    /// read, so one successful trip through the menus **is** the completion.
    OneShot,
}

/// [`Effect`] of `item`. The two exceptions are both key items; everything else is used up.
pub const fn effect(item: ItemId) -> Effect {
    match item {
        ItemId::Bicycle => Effect::TogglesBicycle,
        ItemId::Itemfinder => Effect::OneShot,
        _ => Effect::Consumed,
    }
}

/// `wRepelRemainingSteps` — **I5**'s whole observable. Repel sets 100, Super Repel 200, Max Repel 250
/// (`item_effects.asm:1532`, `:1622`, `:1626`), and the counter ticks down one per overworld step.
pub fn repel_steps(mmu: &MMU) -> u8 {
    mmu.read_pointer(&pokered_symbols::wRepelRemainingSteps)
}

/// True while the player is on the Bicycle — **I6**'s observable. `wWalkBikeSurfState` is 0 walking,
/// 1 cycling, 2 surfing (`item_effects.asm:638-665`).
pub fn on_bicycle(mmu: &MMU) -> bool {
    mmu.read_pointer(&pokered_symbols::wWalkBikeSurfState) == 1
}

/// ⚠️ **`PokemonMove::pp` is the raw PP byte, not the PP.** `encoding.rs` reads it unmasked, and the
/// ROM packs the **PP Up count into bits 6–7** (`PP_UP_MASK`) with the PP itself in bits 0–5. So a
/// move with one PP Up applied reads 64 higher than it "is", and a naive `pp >= max` test refuses
/// every use on it. Two consequences worth carrying: the mask below is mandatory anywhere PP is
/// compared, and bits 6–7 jumping by one is exactly how **I2's PP Up** is observed — there is no
/// other RAM to read for it.
pub const PP_MASK: u8 = 0b0011_1111;

/// The move's actual PP, with the PP Up count masked off.
pub fn move_pp(mv: &crate::pokemon::move_name::PokemonMove) -> u8 { mv.pp & PP_MASK }

/// How many PP Ups have been spent on `mv` (0–3).
pub fn pp_ups(mv: &crate::pokemon::move_name::PokemonMove) -> u8 { mv.pp >> 6 }

/// `mv`'s maximum PP, including the bonus its PP Ups add. `AddBonusPP` gives `base / 5` per PP Up.
pub fn max_pp(mv: &crate::pokemon::move_name::PokemonMove) -> u8 {
    let base = mv.name.metadata().pp;
    base + (base / 5) * pp_ups(mv)
}

/// How many of `item` the bag holds. Zero when it is not in it.
///
/// ⚠️ Reads [`GameState::bag`], which drops ids [`ItemId`] cannot name — fine here, because every
/// item this workstream uses has a name. Menu *navigation* still goes through the raw
/// `bag_item_position` (§10).
pub fn bag_quantity(state: &GameState, item: ItemId) -> u8 {
    state.bag.iter().find(|i| i.id == item).map_or(0, |i| i.quantity)
}

/// The value [`goal_met`] compares against, captured when the step begins.
///
/// Not always a bag count: for the Bicycle it is the *mount state*, because the bike is a toggle and
/// is never consumed. One function so the policy and the driver cannot capture different things.
pub fn baseline(state: &GameState, item: ItemId) -> u8 {
    match effect(item) {
        Effect::TogglesBicycle => state.on_bicycle as u8,
        _ => bag_quantity(state, item),
    }
}

/// Has the use landed? `baseline` comes from [`baseline`]; `attempts` is how many times the driver
/// has been handed the job.
pub fn goal_met(state: &GameState, item: ItemId, baseline: u8, attempts: u32) -> bool {
    match effect(item) {
        Effect::Consumed => bag_quantity(state, item) < baseline,
        Effect::TogglesBicycle => state.on_bicycle as u8 != baseline,
        Effect::OneShot => attempts > 0,
    }
}

/// Why the game would refuse this use, or `None` when it would take it.
///
/// This is the guard that turns §8-I1's warning into behaviour. Every branch below is an item effect
/// that prints a text box, declines, and leaves the item in the bag — which the driver cannot tell
/// from "not used yet", so without this the step retries until the leg's budget runs out.
///
/// Deliberately conservative: an item whose preconditions are not modelled (PP Up's three-use cap,
/// the vitamins' stat ceilings) returns `None` and is simply attempted, because a wrong *refusal*
/// here is a silently skipped step, which is worse than a loud retry.
pub fn blocked(state: &GameState, item: ItemId, target: UseTarget) -> Option<String> {
    if bag_quantity(state, item) == 0 {
        return Some(format!("{item:?} is not in the bag"));
    }
    let mon = target.slot().and_then(|s| state.pokemon.get(s as usize));
    match item {
        // `.healHP` — a Revive wants a *fainted* target and a potion wants a live, damaged one. Each
        // refuses the other's target with "it won't have any effect" (`item_effects.asm:.notFullHP`).
        ItemId::Revive | ItemId::MaxRevive => match mon {
            Some(p) if p.current_hp > 0 => Some(format!("slot {:?} has not fainted", target.slot())),
            _ => None,
        },
        ItemId::Potion | ItemId::SuperPotion | ItemId::HyperPotion | ItemId::MaxPotion
        | ItemId::FullRestore | ItemId::FreshWater | ItemId::SodaPop | ItemId::Lemonade => match mon {
            Some(p) if p.current_hp == 0 => Some("a potion cannot revive a fainted mon".into()),
            Some(p) if p.current_hp >= p.stats.hp => Some("already at full HP".into()),
            _ => None,
        },
        // `.cureStatusAilment` — the item's own status bit has to be set, and a Full Heal takes any.
        ItemId::Antidote | ItemId::BurnHeal | ItemId::IceHeal | ItemId::Awakening
        | ItemId::ParlyzHeal | ItemId::FullHeal => match mon {
            Some(p) if !cures(item, p.status) => Some(format!("no {item:?}-curable status")),
            _ => None,
        },
        // `.useEther` — the chosen move must actually be missing PP.
        ItemId::Ether | ItemId::MaxEther => {
            let UseTarget::Move { move_index, .. } = target else {
                return Some("a PP restore needs a move target".into());
            };
            match mon.and_then(|p| p.moves.get(move_index as usize).and_then(|m| m.as_ref())) {
                None => Some(format!("slot {:?} has no move {move_index}", target.slot())),
                Some(mv) if move_pp(mv) >= max_pp(mv) =>
                    Some(format!("{:?} is already at full PP ({}/{})", mv.name, move_pp(mv), max_pp(mv))),
                _ => None,
            }
        }
        // ⚠️ **An Elixer's precondition is the *mon*, not the move, and applying Ether's here
        // refused perfectly good uses.** `.useElixir` decrements the item id, loops all four slots
        // and only reaches `ItemUseNoEffect` when **none** of them took any PP
        // (`item_effects.asm:.elixirLoop`) — it never reads `wCurrentMenuItem` as a choice, which is
        // why it skips the move menu at all. So an Elixer named against a move that happens to be
        // full was declined here while the cartridge would have restored the other three. The
        // `move_index` still rides along because `use_pp_restore` builds one target for both
        // families and the menu it names simply never appears.
        ItemId::Elixer | ItemId::MaxElixer => {
            let Some(mon) = mon else {
                return Some("a PP restore needs a party target".into());
            };
            mon.moves.iter().flatten().any(|mv| move_pp(mv) < max_pp(mv))
                .then_some(())
                .map_or(Some(format!("every move on slot {:?} is already at full PP", target.slot())), |()| None)
        }
        // `.usePPUp` — three is the cap, and a fourth prints `PPMaxedOutText` and keeps the item.
        ItemId::PpUp => {
            let UseTarget::Move { move_index, .. } = target else {
                return Some("a PP Up needs a move target".into());
            };
            match mon.and_then(|p| p.moves.get(move_index as usize).and_then(|m| m.as_ref())) {
                None => Some(format!("slot {:?} has no move {move_index}", target.slot())),
                Some(mv) if pp_ups(mv) >= 3 => Some(format!("{:?} is already PP-maxed", mv.name)),
                _ => None,
            }
        }
        // ⚠️ `IsBikeRidingAllowed` (`home/overworld.asm:842`) refuses everywhere but Route 23, Indigo
        // Plateau and the five `BikeRidingTilesets`, and `ItemUseBicycle` refuses while surfing —
        // both with `ItemUseNotTime`, which consumes nothing, so both are endless retries.
        ItemId::Bicycle if !bike_riding_allowed(state) =>
            Some(format!("cycling is not allowed on {} (tileset {:?})", state.map.map, state.map.tileset)),
        _ => None,
    }
}

/// `IsBikeRidingAllowed`, decoded rather than transcribed: Route 23 and Indigo Plateau are named
/// special cases, and everything else is a `BikeRidingTilesets` membership test.
pub fn bike_riding_allowed(state: &GameState) -> bool {
    if matches!(state.map.map, Map::Route23 | Map::IndigoPlateau) {
        return true;
    }
    rom_list(&pokered_symbols::BikeRidingTilesets).contains(&(state.map.tileset as u8))
}

/// The `$FF`-terminated byte list at `ptr`. ⚠️ Bank 0 is *not* windowed — its addresses are already
/// file offsets, while a banked pointer addresses `$4000..$7FFF` through the window. Getting that
/// wrong reads a different table entirely and silently.
fn rom_list(ptr: &crate::pokemon::symbols::DmgPointer) -> Vec<u8> {
    let crate::pokemon::symbols::DmgBank::ROM { bank } = ptr.bank else { panic!("{ptr:?} is not in ROM") };
    let offset = if bank == 0 { ptr.address as usize }
                 else { bank as usize * 0x4000 + (ptr.address as usize - 0x4000) };
    crate::pokemon::roms::POKERED[offset..].iter().copied().take_while(|&b| b != 0xFF).collect()
}

/// Which status an item's `.checkMonStatus` branch tests (`item_effects.asm:869-889`).
fn cures(item: ItemId, status: PokemonStatus) -> bool {
    match item {
        ItemId::Antidote => status == PokemonStatus::Poisoned,
        ItemId::BurnHeal => status == PokemonStatus::Burned,
        ItemId::IceHeal => status == PokemonStatus::Frozen,
        ItemId::Awakening => matches!(status, PokemonStatus::Asleep { .. }),
        ItemId::ParlyzHeal => status == PokemonStatus::Paralyzed,
        ItemId::FullHeal => status != PokemonStatus::None,
        _ => false,
    }
}

// ── The driver ───────────────────────────────────────────────────────────────────────────────────

/// A wedged use reports itself rather than mashing for the rest of the leg. Generous: the longest
/// chain here is five menus, a cursor walk down a twenty-row bag, and two text boxes — and a use that
/// *does* land near the end of the budget is worse than one that never lands, because the generic
/// A-mash then finishes it and the log claims a wedge that did not happen (K hit the same thing).
const TICK_BUDGET: u32 = 1800;

/// Live state of an in-progress bag-item use. Carried in [`AgentState::UsingBagItem`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BagItemState {
    pub item: ItemId,
    pub target: UseTarget,
    /// What completion is measured against — see [`baseline`]. A bag count for everything but the
    /// Bicycle, whose baseline is its mount state.
    pub baseline: u8,
    /// Press/release alternation, so every input is a fresh rising edge.
    pub press: bool,
    pub entered_menu: bool,
    /// Consecutive stable-overworld ticks once the effect has landed — the same anti-flicker wait
    /// `AgentState::TeachingMove` uses, and for the same reason: Gen 1 drops back into the bag after
    /// a use, so finishing on a one-tick gap between closing menus hands a live menu to the generic
    /// A-mash and uses a second one.
    pub settle: u8,
    pub ticks: u32,
}

impl BagItemState {
    pub fn new(item: ItemId, target: UseTarget, api: &PokemonApi<'_>) -> Self {
        Self {
            item, target,
            baseline: match effect(item) {
                Effect::TogglesBicycle => on_bicycle(api.mmu()) as u8,
                _ => api.bag_item_quantity(item),
            },
            press: true, entered_menu: false, settle: 0, ticks: 0,
        }
    }

    /// Has the effect landed? See [`Effect`] — three items, three different answers.
    fn done(&self, api: &PokemonApi<'_>) -> bool {
        match effect(self.item) {
            Effect::Consumed => self.entered_menu && api.bag_item_quantity(self.item) < self.baseline,
            Effect::TogglesBicycle => on_bicycle(api.mmu()) as u8 != self.baseline,
            // The menus close themselves (`UsableItems_CloseMenu`), so the return to the overworld
            // *is* the effect having been printed.
            Effect::OneShot =>
                self.entered_menu && api.game_mode().unwrap_or(GameMode::Overworld) == GameMode::Overworld,
        }
    }
}

/// **I** — the policy's half: what to hand the driver, or why not to.
///
/// Returns `Err(reason)` when the step should be popped rather than issued — either because the
/// effect has already landed, or because [`blocked`] says the game would decline it.
pub fn pick(state: &GameState, item: ItemId, target: UseTarget, baseline: u8, attempts: u32)
    -> Result<FieldMove, String> {
    if goal_met(state, item, baseline, attempts) {
        return Err(format!("{item:?} used"));
    }
    if let Some(why) = blocked(state, item, target) {
        return Err(format!("{item:?} would have no effect — {why}"));
    }
    Ok(FieldMove::UseBagItem { item, target })
}

/// One agent tick of the bag-item chain. Called from `agent.rs` via a single delegating match arm.
pub fn tick(agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: BagItemState) -> Result<(), String> {
    let game_mode = api.game_mode().unwrap_or(GameMode::Overworld);

    // ── Done: back out of whatever menu the use left open, then settle in the overworld ─────────
    if s.done(api) {
        if game_mode != GameMode::Overworld {
            api.release_all_buttons();
            if s.press { api.press_button(JoypadButton::B); }
            agent.set_state(AgentState::UsingBagItem(BagItemState { press: !s.press, settle: 0, ..s }));
            return Ok(());
        }
        if s.settle < 15 {
            api.release_all_buttons();
            agent.set_state(AgentState::UsingBagItem(BagItemState { settle: s.settle + 1, ..s }));
            return Ok(());
        }
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox { message: format!("Used {:?} ({:?})", s.item, s.target) });
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    // ── Fizzled: back in the overworld with nothing to show. Drop to Idle so the policy decides
    //    whether to re-issue — it counts attempts, so this cannot spin for ever. ─────────────────
    if s.entered_menu && game_mode == GameMode::Overworld {
        api.release_all_buttons();
        agent.set_state(AgentState::Idle);
        return Ok(());
    }
    if s.ticks > TICK_BUDGET {
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox {
            message: format!("bag-item: {:?} did nothing in {TICK_BUDGET} ticks", s.item) });
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    let s = BagItemState { entered_menu: s.entered_menu || game_mode != GameMode::Overworld,
                           ticks: s.ticks + 1, ..s };
    if !s.press {
        api.release_all_buttons();
        agent.set_state(AgentState::UsingBagItem(BagItemState { press: true, ..s }));
        return Ok(());
    }

    let (top_x, top_y, current, scroll) = api.menu_geometry();
    let tbid = api.menu_state().map(|m| m.text_box_id);
    let text = api.on_screen_text(false).unwrap_or_default();
    let nav = |cur: u8, target: u8| -> JoypadButton {
        if cur < target { JoypadButton::Down }
        else if cur > target { JoypadButton::Up }
        else { JoypadButton::A }
    };

    let button = if game_mode == GameMode::Overworld {
        JoypadButton::Start
    } else if (top_x, top_y) == START_MENU_ORIGIN {
        // The Pokédex is owned by construction here, but the index is asked for rather than
        // assumed — see `start_menu_row`; a seventh copy of the literal is how this drifts.
        nav(current, start_menu_row(api, StartMenuRow::Item))
    } else if text.to_ascii_uppercase().contains("TECHNIQUE") {
        // `MoveSelectionMenu`'s relearn layout — the PP-restore move list.
        //
        // ⚠️ **Detected by its prompt, and neither of the two obvious alternatives works.**
        // `wTextBoxID` is no use: `MoveSelectionMenu` never calls `DisplayTextBoxID`, so it still
        // reads `ListMenuBox` from the bag underneath — and with the bag branch first the driver
        // walks this list toward a bag row number and never presses A. Its **geometry** (5,7) is no
        // use either, and that one is nastier: `SelectMenuItem` *decrements* `wCurrentMenuItem` when
        // the move is chosen (`engine/battle/core.asm:2623-2625`) and leaves the geometry behind, so
        // a geometry-keyed branch sees `cur` drop from 1 to 0 and spends the rest of the leg pressing
        // Down at the "PP was restored." prompt, which only takes A. Both failures look identical in
        // the log — a normal-looking driver tick, for the whole budget — and both end with the
        // generic A-mash finishing the job after the abort, so the item *does* get used and the run
        // reports a wedge that did not happen.
        //
        // ⚠️ **And the prompt is lower-case** — `_RaisePPWhichTechniqueText` is *"Raise PP of which
        // technique?"* (`data/text/text_6.asm:130`). The upper-case `WhichTechniqueString` that
        // `SelectMenuItem` places is only for `wMoveMenuType == 1`, the Mimic menu, which this is not
        // (`engine/battle/core.asm:2580-2586`). Matching `"TECHNIQUE"` therefore never fires, the
        // driver falls through to the trailing `A`, and A on the cursor's **starting** row picks move
        // slot 0 — which looks perfectly correct for as long as the caller only ever asks for slot 0,
        // and silently restores the wrong move the moment it asks for another.
        //
        // The cursor is **1-indexed**: the list is drawn from row 8 under a `wTopMenuItemY` of 7, and
        // that same decrement turns the choice back into a 0-based move slot for
        // `GetSelectedMoveOffset`.
        match s.target {
            UseTarget::Move { move_index, .. } => nav(current, move_index + 1),
            _ => JoypadButton::B,
        }
    } else if tbid == Some(TextBoxId::ListMenuBox) {
        // The bag. ⚠️ The row comes from the **raw** `wBagItems` (§10) — `GameState::bag` drops the
        // TMs and would shift every index below them.
        match api.bag_item_position(s.item) {
            Some(row) => nav(current + scroll, row),
            None => JoypadButton::B,
        }
    } else if tbid == Some(TextBoxId::UseTossMenuTemplate) {
        nav(current, 0) // USE / TOSS → USE. (The Bicycle never shows this menu.)
    } else if tbid == Some(TextBoxId::MessageBox) && top_x == 0 && (top_y == 1 || top_y == 3) {
        // "Use item on which POKéMON?" — the same party menu the teach chain drives.
        match s.target.slot() {
            Some(slot) => nav(current, slot),
            None => JoypadButton::B,
        }
    } else {
        JoypadButton::A // transitional text
    };

    // A per-tick trace of every menu this chain walks through, off unless `ITEMS_TRACE` is set. Two
    // of the three bugs in this driver were menus that *looked* right from the outside — a stale
    // `text_box_id`, a cursor the ROM decrements behind your back — and neither was visible in the
    // agent log. This is how they were found; leave it here for the next one.
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *TRACE.get_or_init(|| std::env::var("ITEMS_TRACE").is_ok()) {
        println!("[items] t{} mode {game_mode:?} tbid {tbid:?} geom ({top_x},{top_y}) cur {current} \
                  scroll {scroll} text {:?} -> {button:?}", s.ticks, text.chars().take(40).collect::<String>());
    }
    api.release_all_buttons();
    api.press_button(button);
    agent.set_state(AgentState::UsingBagItem(BagItemState { press: false, ..s }));
    Ok(())
}

// ── I3 / I4: the in-battle items ─────────────────────────────────────────────────────────────────

/// The stat items, in the order [`PolicyStep::stat_item_steps`] spends them.
///
/// All seven are `wIsInBattle`-gated (`ItemUseXStat`, `ItemUseXAccuracy`, `ItemUseGuardSpec`,
/// `ItemUseDireHit`) — out of battle they answer `ItemUseNotTime` and set
/// `wActionResultOrTookBattleTurn = 2`, so there is no overworld branch to write. The three at the
/// end are not `XStat` entries at all: X Accuracy, Guard Spec. and Dire Hit set **bits in
/// `wPlayerBattleStatus2`** rather than a stat stage, which is why I3's observable is two reads and
/// not one.
pub const STAT_ITEMS: &[ItemId] = &[
    ItemId::XAttack, ItemId::XDefend, ItemId::XSpeed, ItemId::XSpecial,
    ItemId::XAccuracy, ItemId::GuardSpec, ItemId::DireHit,
];

/// The four `ItemUseXStat` stat stages, read straight from RAM. **7 is the neutral value**
/// (`StatModifierUpEffect`), so a successful X Attack reads 8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatMods { pub attack: u8, pub defense: u8, pub speed: u8, pub special: u8 }

impl StatMods {
    /// What the ROM resets them to at the start of every battle.
    pub const NEUTRAL: Self = Self { attack: 7, defense: 7, speed: 7, special: 7 };
}

/// The player's live stat stages. Only meaningful inside a battle.
pub fn stat_mods(mmu: &MMU) -> StatMods {
    StatMods {
        attack: mmu.read_pointer(&pokered_symbols::wPlayerMonAttackMod),
        defense: mmu.read_pointer(&pokered_symbols::wPlayerMonDefenseMod),
        speed: mmu.read_pointer(&pokered_symbols::wPlayerMonSpeedMod),
        special: mmu.read_pointer(&pokered_symbols::wPlayerMonSpecialMod),
    }
}

/// `wPlayerBattleStatus2`, whose three top bits are what X Accuracy, Guard Spec. and Dire Hit set
/// (`USING_X_ACCURACY`, `PROTECTED_BY_MIST`, `GETTING_PUMPED`).
pub fn player_battle_status2(mmu: &MMU) -> u8 {
    mmu.read_pointer(&pokered_symbols::wPlayerBattleStatus2)
}

/// Bit positions in `wPlayerBattleStatus2` (`constants/battle_constants.asm:90-99`).
///
/// ⚠️ `USING_X_ACCURACY` is bit **0**, not bit 6. Bit 6 is `USING_RAGE`. The first draft had it at 6
/// and the run reported `$07` — all three bits set and the assertion failing anyway — which is a
/// pleasant way to be wrong, because the mistake was in the *test's* idea of the RAM rather than in
/// the driver. Transcribe the const block, do not count from the name.
pub mod battle_status2 {
    pub const USING_X_ACCURACY: u8 = 1 << 0;
    pub const PROTECTED_BY_MIST: u8 = 1 << 1;
    pub const GETTING_PUMPED: u8 = 1 << 2;
}

// ── Step lists ───────────────────────────────────────────────────────────────────────────────────

impl PolicyStep {
    /// **I1** — use a medicine on the party member in `slot`.
    ///
    /// ⚠️ The step is refused, with a reason, when the target could not benefit — see [`blocked`].
    /// A potion on a full-HP mon is the plan's own example: the ROM prints a text box that reads
    /// exactly like success and keeps the item.
    pub const fn use_medicine(item: ItemId, slot: u8) -> Self {
        Self::UseBagItem { item, target: UseTarget::Party { slot } }
    }

    /// **I2** — restore PP to move `move_index` (0–3) of the party member in `slot`.
    ///
    /// Ether and Max Ether act on one move and so need `move_index`; Elixer and Max Elixer act on the
    /// whole mon and skip the move menu entirely (`item_effects.asm:.useElixir`), but passing an
    /// index for them is harmless — the menu simply never appears.
    pub const fn use_pp_restore(item: ItemId, slot: u8, move_index: u8) -> Self {
        Self::UseBagItem { item, target: UseTarget::Move { slot, move_index } }
    }

    /// **I5/I6/I7** — an item with no target: a Repel, the Bicycle, the Itemfinder.
    pub const fn use_item(item: ItemId) -> Self {
        Self::UseBagItem { item, target: UseTarget::Nothing }
    }

    /// **I7 + I5** — press the **Itemfinder** where it can answer *yes*, and buy a Repel.
    ///
    /// The Itemfinder writes no RAM at all; its whole effect is which of two texts it prints
    /// (`ItemfinderFoundItemText` / `ItemfinderFoundNothingText`). So proving it works means standing
    /// somewhere it should say **yes** — and that turns out to be a surprisingly small set.
    ///
    /// ⚠️ **`HiddenItemNear` needs an *uncollected* item within x ± 5 and y − 5 .. + 4**
    /// (`engine/items/itemfinder.asm:11-41`), and the Fly stop is not it: Vermilion's hidden Max Ether
    /// sits at raw (14,11) while Fly lands the player at (11,4), seven rows north. `stand_near` is a
    /// door inside the window — stepping in and straight back out is the cheapest way to stand
    /// somewhere specific, because no step expresses "walk to this tile". `VermilionTradeHouse` (raw
    /// (15,13)) and the Fan Club (9,13) are the two that qualify; the mart at (23,13) does not.
    /// Measured by `probe_itemfinder_range`, not guessed: `MetaTileMap` offsets both the player and
    /// the item by the north connection strip, so "it looks close" is not an argument.
    ///
    /// ⚠️ **And this item can never be collected**, which is what makes it a *stable* test rather than
    /// a one-shot: the town-reachability probe (deleted with hidden items) showed it walled inside a fence block with no
    /// adjacent standable tile, so `wObtainedHiddenItemsFlags` bit 51 stays clear for ever. Cerulean's
    /// hidden Rare Candy is the same shape. Viridian's Potion and Celadon's PP Up are the reachable
    /// ones.
    pub fn press_the_itemfinder_steps(stand_near: Map) -> Vec<Self> {
        vec![
            Self::Fly { to: Map::VermilionCity },
            Self::enter(stand_near),
            Self::enter(Map::VermilionCity),
            Self::use_item(ItemId::Itemfinder),
            Self::enter(Map::VermilionMart),
            Self::BuyFromMart { item: crate::pokemon::bag::BagItem::new(ItemId::Repel, 1),
                                map: Map::VermilionMart },
            Self::enter(Map::VermilionCity),
            // …and the other branch. Fuchsia City has no hidden items at all, so the same item must
            // print the *other* text there — without which "it printed something" is all this proves.
            Self::Fly { to: Map::FuchsiaCity },
            Self::use_item(ItemId::Itemfinder),
        ]
    }

    /// **I2** — the PP leg: an Ether onto a spent move, then the hidden **PP Up** Celadon is sitting on.
    ///
    /// §8-I calls this the highest-value item in the workstream, and the archive says why: a **0-PP
    /// battle deadlock** is the failure that once made grinding look impossible, and the only cure
    /// today is a walk to a Pokémon Center.
    ///
    /// The two halves are the same ROM routine — `ItemUsePPUp` *falls through* into
    /// `ItemUsePPRestore` (`item_effects.asm:1949`), so both open the party menu and then the
    /// 1-indexed move menu — but they are observed differently: an Ether raises the PP in bits 0–5,
    /// a PP Up raises the **count in bits 6–7** and leaves the PP alone.
    ///
    /// ⚠️ **Both items are debug-seeded, and neither is laziness.** No mart in Kanto stocks an Ether
    /// (`data/items/marts.asm`), and every Ether/Elixer lying on the floor is behind a trek this leg
    /// would otherwise be about — Route 9's is across the Cerulean bridge, Route 25's is past the
    /// Nugget Bridge, Route 10's is in the *southern* section beyond Rock Tunnel, and Tower 5F needs
    /// the Silph Scope back out of the PC.
    ///
    /// ⚠️ **The PP Up used to be dug out of Celadon and is now seeded too** (2026-09-03). Every PP Up
    /// in the game is a hidden item, and hidden-item collection is gone from this codebase — see
    /// [`crate::pokemon::postgame::aides`] for why. What this leg is *about* is the shared ROM
    /// routine behind an Ether and a PP Up (`ItemUsePPUp` falls through into `ItemUsePPRestore`) and
    /// the two different observables it produces, and none of that changes with where the item came
    /// from. The `Fly` stays: this leg's output fixture is rooted in Celadon.
    pub fn pp_restore_steps(ether: ItemId, slot: u8, ether_move: u8, pp_up_move: u8) -> Vec<Self> {
        vec![
            Self::use_pp_restore(ether, slot, ether_move),
            Self::Fly { to: Map::CeladonCity },
            Self::use_pp_restore(ItemId::PpUp, slot, pp_up_move),
        ]
    }

    /// **I5** — set a Repel counter running, then walk far enough to watch it tick down.
    ///
    /// Outdoors on purpose: `wRepelRemainingSteps` is decremented by the **overworld step** handler,
    /// so a use indoors sets the counter and then nothing moves it.
    pub fn repel_steps(item: ItemId, walk_to: Map) -> Vec<Self> {
        vec![Self::use_item(item), Self::enter(walk_to)]
    }

    /// **I6** — mount the Bicycle, ride it somewhere, and get off again.
    ///
    /// ⚠️ Mounting and dismounting are the *same item*, because `ItemUseBicycle` toggles
    /// `wWalkBikeSurfState`. That is why [`Effect::TogglesBicycle`]'s completion test is "the mount
    /// state changed" rather than "we are on the bike": with the latter the third step below would be
    /// satisfied the moment it reached the front of the queue and pop without pressing anything.
    pub fn ride_bicycle_steps(ride_to: Map) -> Vec<Self> {
        vec![Self::use_item(ItemId::Bicycle), Self::enter(ride_to), Self::use_item(ItemId::Bicycle)]
    }

    /// **I3/I4** — buy the stat items and a Poké Doll, then spend them in one wild battle.
    ///
    /// Every one of them is on a shelf: `CeladonMart5FClerk1Text` sells all seven stat items and
    /// `CeladonMart4FClerkText` sells the Poké Doll (`data/items/marts.asm:29,32`). ⚠️ **Nine bag
    /// rows have to be free before the first purchase** — a mart sale into a full bag prints "You
    /// can't carry any more items" and the `BuyFromMart` step gives up quietly (§10), so `shed` goes
    /// to the Celadon PC first and is not optional.
    ///
    /// The fight is on **Route 1**: its grass is one warp from a Fly stop, it is 50/50 Pidgey and
    /// Rattata at level 3, and neither can flee or threaten a level-71 lead — so the battle lasts
    /// exactly as long as it takes to spend eight items.
    pub fn stat_item_steps(shed: &[ItemId], on_map: Map, items: &'static [ItemId]) -> Vec<Self> {
        let mut s = vec![Self::Fly { to: Map::CeladonCity }, Self::enter(Map::CeladonPokecenter)];
        s.extend(shed.iter().map(|&item| Self::deposit_item(item, u8::MAX, Map::CeladonPokecenter)));
        s.extend([
            Self::enter(Map::CeladonCity),
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonMart2F),
            Self::enter(Map::CeladonMart3F),
            Self::enter(Map::CeladonMart4F),
            Self::BuyFromMart { item: crate::pokemon::bag::BagItem::new(ItemId::PokeDoll, 1),
                                map: Map::CeladonMart4F },
            Self::enter(Map::CeladonMart5F),
        ]);
        // ⚠️ Two clerks on 5F: `BuyFromMart` targets the sprite named "Clerk 1", which is the stat
        // items (Clerk 2 sells vitamins). The step list never has to say so — the policy's clerk
        // lookup already prefers "Clerk 1" — but the floor does have to be the right one.
        s.extend(STAT_ITEMS.iter().map(|&item| Self::BuyFromMart {
            item: crate::pokemon::bag::BagItem::new(item, 1), map: Map::CeladonMart5F }));
        s.extend([
            Self::enter(Map::CeladonMart4F),
            Self::enter(Map::CeladonMart3F),
            Self::enter(Map::CeladonMart2F),
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonCity),
            Self::Fly { to: Map::ViridianCity },
            Self::enter(on_map),
            Self::UseItemsInBattle { on_map, items },
            Self::enter(Map::ViridianCity),
        ]);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin [`effect`] against the ROM's own two lists — the ones `StartMenu_Item` matches on, and
    /// the reason the three menu shapes exist at all.
    ///
    /// `UsableItems_CloseMenu` is the list that returns to the overworld; anything not in either list
    /// stays in the bag. Getting this wrong is a driver that mashes B at a live menu, or one that
    /// waits in the overworld for a menu that closed minutes ago — neither of which errors.
    #[test]
    fn close_menu_items_match_the_rom() {
        let names: Vec<ItemId> = rom_list(&pokered_symbols::UsableItems_CloseMenu)
            .iter().filter_map(|&b| ItemId::from_repr(b)).collect();
        assert_eq!(names, vec![ItemId::EscapeRope, ItemId::Itemfinder, ItemId::PokeFlute,
                               ItemId::OldRod, ItemId::GoodRod, ItemId::SuperRod],
            "UsableItems_CloseMenu changed — `Effect::OneShot`'s premise is that the Itemfinder is \
             in it, i.e. that using it returns straight to the overworld");
        // …and the Bicycle is deliberately *not* in it: `StartMenu_Item` special-cases it earlier
        // (`:341`), which is why it skips the USE/TOSS menu that everything else shows.
        assert!(!names.contains(&ItemId::Bicycle));
    }

    /// Every item in `UsableItems_PartyMenu` opens the party menu, so every one of them needs a
    /// [`UseTarget`] with a slot. This checks the workstream's own items against that list rather
    /// than against a recollection of which items "obviously" need a target.
    #[test]
    fn party_menu_items_need_a_slot() {
        let party_items: Vec<ItemId> = rom_list(&pokered_symbols::UsableItems_PartyMenu)
            .into_iter().filter_map(ItemId::from_repr).collect();
        for item in [ItemId::Potion, ItemId::Revive, ItemId::FullHeal, ItemId::Ether,
                     ItemId::MaxElixer, ItemId::PpUp, ItemId::XAttack] {
            assert!(party_items.contains(&item), "{item:?} should open the party menu");
        }
        for item in [ItemId::Repel, ItemId::Itemfinder, ItemId::Bicycle, ItemId::PokeDoll,
                     ItemId::GuardSpec, ItemId::DireHit] {
            assert!(!party_items.contains(&item), "{item:?} should NOT open the party menu");
        }
    }

    /// The Poké Doll has to be last in any in-battle list — it ends the battle.
    #[test]
    fn no_stat_item_ends_the_battle() {
        assert!(!STAT_ITEMS.contains(&ItemId::PokeDoll),
            "the Poké Doll ends the battle (wEscapedFromBattle), so it belongs after STAT_ITEMS, \
             never inside it");
    }
}
