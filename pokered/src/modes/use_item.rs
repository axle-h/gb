//! `UseItem_` out of battle for the items that ask which Pokémon: `ItemUseMedicine` (with
//! `ItemUseVitamin` and the Rare Candy inside it), `ItemUsePPUp`, `ItemUsePPRestore`,
//! `ItemUseEvoStone` and `ItemUseTMHM`.
//!
//! The arithmetic is `systems::item_use` and `systems::pp`, and exact. What this adds is the screens
//! in the cartridge's order and pacing: the party menu in `USE_ITEM_PARTY_MENU`, the no-effect and
//! result texts, the HP bar walking to the new HP, and for a healing item the party menu redrawn
//! with its message, 50 frames, and a press. The PP items ask for a move with
//! `MoveSelectionMenu`; backing out of that asks for a mon again, and a PP Up on a move with three
//! already asks which technique again.
//!
//! A Rare Candy redraws the list with its message, waits for a press with the level-up stats box
//! up, then offers the level's move through `LearnMove` and tries `TryEvolvingMon`, with the mon's
//! species as `wCurItem` because `LearnMoveFromLevelUp` leaves it there. A stone forces the
//! evolution with itself as the item and is only used up if one happened. A machine is booted up,
//! asked about, and taught through `LearnMove` to a mon from a party menu of `ABLE` and `NOT ABLE`;
//! a mon that cannot learn it or knows it already asks for another mon, and an HM is never used up.
//!
//! The answer is `wActionResultOrTookBattleTurn` as `Outcome::Chosen`: 1 used, 0 not, and 2 for an
//! item refused before any party menu came up. A vitamin or a Rare Candy with no effect, a machine
//! whose party menu is backed out of, and a machine not learned all still answer 1.
//!
//! Not modelled: `PlayDefaultMusic` after an evolution, since no map music is kept yet, and
//! `LoadScreenTilesFromBuffer1` when a machine's party menu is backed out of, which the palettes
//! have already whited out and the bag draws over.

use poke_core::item::{machine_move, ItemId};
use poke_core::move_name::PokemonMoveName;
use poke_core::symbols::pokered_symbols;
use poke_core::text_script::{decode, far_text, TextBuffer, TextCommand, TextNumber};
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::command::Decision;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::evolution::Evolution;
use crate::modes::learn_move::LearnMove;
use crate::modes::list_menu::remove_from_bag;
use crate::modes::move_selection_menu::MoveSelectionMenu;
use crate::modes::party_menu::{PartyMenu, PartyMenuType};
use crate::modes::text_box::TextBox;
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::systems::hp_bar::{HpBarAnimation, HpBarType};
use crate::systems::item_use::{can_learn_tm, use_medicine, use_rare_candy, use_vitamin, vitamin_stat_name, ItemUse,
                               Medicine, MedicineMessage};
use crate::systems::pp::{restore_pp, use_pp_up, MAX_PP_UPS, pp_ups};
use crate::systems::status_screen::{print_stats_box, StatsBox};

pub const USED: u8 = 1;
pub const NOT_USED: u8 = 0;
/// The item was refused before any menu came up.
pub const NO_MENU: u8 = 2;

/// `hlcoord 14, 7`, where `ItemUseTMHM` asks its yes/no.
const YES_NO_AT: (usize, usize) = (14, 7);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Flow {
    Medicine,
    Vitamin,
    RareCandy,
    PpUp,
    PpRestore,
    EvoStone,
    TmHm(PokemonMoveName),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UseItem {
    item: ItemId,
    flow: Flow,
    /// `wWhichPokemon` as the bag left it: the slot one item is taken from.
    slot: u8,
    /// `wUsedItemOnWhichPokemon`.
    mon: u8,
    phase: Phase,
    /// `wStatusFlags5`'s text-delay bit, which `RedrawPartyMenu_` pushes and pops.
    no_text_delay: bool,
    answered: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// A child mode is up; `resume` says what came back from it.
    Child(After),
    /// `PlaySoundWaitForCurrent`'s wait for the sound already playing.
    WaitForSound(Medicine),
    Bar(HpBarAnimation, MedicineMessage),
    /// `.showHealingItemMessage`'s `DelayFrames 50`, holding the message up before its press.
    Holding(u8),
    /// `WaitForTextScrollButtonPress`, and what follows the press.
    WaitButton(Then),
    /// `PlaySoundWaitForCurrent` of `sound`, and for a stone `WaitForSoundToFinish` after it.
    Sound { sound: Sound, playing: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Sound {
    /// `SFX_HEAL_AILMENT`, before a stone's evolution.
    Stone,
    /// `SFX_DENIED`, before a machine's refusal.
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Then {
    /// A healing item's press, after which it is done.
    Healed,
    /// The Rare Candy's message is up: the stats box, and its press.
    StatsBox,
    /// The press on the stats box: the level's move, then the evolution.
    LevelUp,
}

/// What to do when the child on top comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum After {
    ChooseMon,
    /// A text that ends the item's use with this answer, removing one first when it was used.
    Finish { result: u8, remove: bool },
    /// The "which technique?" text, before the move menu.
    AskMove,
    ChooseMove,
    /// `PPMaxedOutText`, after which `.chooseMove` asks which technique again.
    AskMoveAgain,
    Message,
    RareCandyMessage,
    RareCandyLearn,
    /// `TryEvolvingMon`, from a stone or a Rare Candy.
    Evolving,
    Booted,
    Teach,
    ConfirmTeach,
    /// A refusal, after which `.chooseMon` asks for a mon again.
    ChooseMonAgain,
    Learning,
}

impl UseItem {
    /// `None` for an item whose use is not one of these routines. `slot` is the item's place in
    /// the bag.
    pub fn new(item: ItemId, slot: u8) -> Option<Self> {
        let flow = match ItemUse::of(item) {
            ItemUse::Medicine => Flow::Medicine,
            ItemUse::Vitamin => Flow::Vitamin,
            ItemUse::RareCandy => Flow::RareCandy,
            ItemUse::PpUp => Flow::PpUp,
            ItemUse::PpRestore => Flow::PpRestore,
            ItemUse::EvoStone => Flow::EvoStone,
            ItemUse::TmHm => Flow::TmHm(machine_move(item).expect("a machine teaches a move")),
            _ => return None,
        };
        Some(Self { item, flow, slot, mon: 0, phase: Phase::Child(After::ChooseMon), no_text_delay: false, answered: 0 })
    }

    /// Presses answered at `WaitForTextScrollButtonPress`, so a driver can see its press land.
    pub fn answered(&self) -> u32 {
        self.answered
    }

    fn push_text(&mut self, script: Vec<TextCommand>, after: After) -> Transition {
        self.phase = Phase::Child(after);
        Transition::Push(Mode::TextBox(TextBox::script(script)))
    }

    fn text(&mut self, label: &str, after: After) -> Transition {
        self.push_text(far_text(label).expect("an item's texts are in the cartridge"), after)
    }

    fn choose_mon(&mut self) -> Transition {
        self.phase = Phase::Child(After::ChooseMon);
        let menu = match self.flow {
            Flow::EvoStone => PartyMenu::evo_stone(self.item),
            Flow::TmHm(mv) => PartyMenu::teaching(mv),
            _ => PartyMenu::new(PartyMenuType::UseItem),
        };
        Transition::Push(Mode::PartyMenu(menu))
    }

    fn remove_used_item(&self, ctx: &mut Ctx) {
        remove_from_bag(ctx, self.slot as usize, 1);
    }

    /// `ItemUseMedicine.done` and `ItemUsePPRestore.itemNotUsed`: the palettes go white, which is
    /// not modelled, and out of battle the map's tiles come back behind the menus.
    fn finish(&self, ctx: &mut Ctx, result: u8) -> Transition {
        ctx.screen.tiles.load_text_box_tiles();
        Transition::Pop(Outcome::Chosen(result))
    }

    fn nick(ctx: &Ctx, mon: u8) -> Vec<u8> {
        ctx.world.party[mon as usize].nick.clone()
    }

    /// Where the party menu has answered with a mon.
    fn chosen_mon(&mut self, ctx: &mut Ctx, mon: u8) -> Transition {
        self.mon = mon;
        let nick = Self::nick(ctx, mon);
        let party_mon = &mut ctx.world.party[mon as usize].mon;
        match self.flow {
            Flow::Medicine => match use_medicine(self.item, party_mon) {
                Medicine::NoEffect => self.text("_ItemUseNoEffectText", After::Finish { result: NOT_USED, remove: false }),
                used => {
                    self.remove_used_item(ctx);
                    self.phase = Phase::WaitForSound(used);
                    self.update(ctx)
                }
            },
            Flow::Vitamin => {
                let used = use_vitamin(self.item, party_mon);
                ctx.world.text.strings.insert(TextBuffer::NameBuffer, nick);
                if !used {
                    return self.text("_VitaminNoEffectText", After::Finish { result: USED, remove: false });
                }
                let name = poke_core::charmap::encode(vitamin_stat_name(self.item)).expect("a stat name encodes");
                ctx.world.text.strings.insert(TextBuffer::StringBuffer, name);
                ctx.audio.play_sound(sounds::SFX_HEAL_AILMENT);
                self.text("_VitaminStatRoseText", After::Finish { result: USED, remove: true })
            }
            Flow::RareCandy => {
                let used = use_rare_candy(party_mon);
                let level = party_mon.level;
                ctx.world.text.strings.insert(TextBuffer::NameBuffer, nick);
                if !used {
                    return self.text("_VitaminNoEffectText", After::Finish { result: USED, remove: false });
                }
                ctx.world.text.numbers.insert(TextNumber::CurEnemyLevel, level as u32);
                // `RedrawPartyMenu`, not `DrawPartyMenu`: the list goes back over itself uncleared.
                PartyMenu::redraw_entries(ctx);
                self.no_text_delay = ctx.world.no_text_delay;
                ctx.world.no_text_delay = true;
                let script = decode(pokered_symbols::RareCandyText).expect("the Rare Candy's text is in the cartridge");
                self.push_text(script, After::RareCandyMessage)
            }
            Flow::PpRestore if matches!(self.item, ItemId::Elixer | ItemId::MaxElixer) => {
                let full = self.item == ItemId::MaxElixer;
                let mut restored = false;
                for slot in 0..4 {
                    let Some(mv) = party_mon.mon.moves[slot] else { continue };
                    if let Some(pp) = restore_pp(party_mon.mon.pp[slot], mv, full) {
                        party_mon.mon.pp[slot] = pp;
                        restored = true;
                    }
                }
                self.after_restoring(ctx, restored)
            }
            Flow::PpUp | Flow::PpRestore => self.ask_move(),
            Flow::EvoStone => {
                self.phase = Phase::Sound { sound: Sound::Stone, playing: false };
                self.update(ctx)
            }
            Flow::TmHm(mv) => {
                let species = party_mon.mon.species;
                let known = party_mon.mon.moves.contains(&Some(mv));
                ctx.world.text.strings.insert(TextBuffer::NameBuffer, nick);
                if !can_learn_tm(species, mv) {
                    self.phase = Phase::Sound { sound: Sound::Denied, playing: false };
                    return self.update(ctx);
                }
                if known {
                    return self.text("_AlreadyKnowsText", After::ChooseMonAgain);
                }
                self.phase = Phase::Child(After::Learning);
                Transition::Push(Mode::LearnMove(LearnMove::new(mon, mv)))
            }
        }
    }

    fn ask_move(&mut self) -> Transition {
        let label = if self.flow == Flow::PpUp { "_RaisePPWhichTechniqueText" } else { "_RestorePPWhichTechniqueText" };
        self.text(label, After::AskMove)
    }

    fn chosen_move(&mut self, ctx: &mut Ctx, index: u8) -> Transition {
        let party_mon = &mut ctx.world.party[self.mon as usize].mon;
        let Some(mv) = party_mon.mon.moves[index as usize] else { return self.ask_move() };
        ctx.world.text.strings.insert(TextBuffer::StringBuffer, mv.name());
        let pp = &mut party_mon.mon.pp[index as usize];
        match self.flow {
            Flow::PpUp if pp_ups(*pp) >= MAX_PP_UPS => self.text("_PPMaxedOutText", After::AskMoveAgain),
            Flow::PpUp => {
                *pp = use_pp_up(*pp, mv).expect("fewer than three PP Ups");
                self.text("_PPIncreasedText", After::Finish { result: USED, remove: true })
            }
            _ => {
                let restored = restore_pp(*pp, mv, self.item == ItemId::MaxEther);
                if let Some(new) = restored {
                    *pp = new;
                }
                self.after_restoring(ctx, restored.is_some())
            }
        }
    }

    /// `.afterRestoringPP`, or `.noEffect` where nothing was.
    fn after_restoring(&mut self, ctx: &mut Ctx, restored: bool) -> Transition {
        if !restored {
            return self.text("_ItemUseNoEffectText", After::Finish { result: NOT_USED, remove: false });
        }
        ctx.audio.play_sound(sounds::SFX_HEAL_AILMENT);
        self.text("_PPRestoredText", After::Finish { result: USED, remove: true })
    }

    /// `.showHealingItemMessage`, once the sound and the bar are done: `ClearScreen`, and the list
    /// redrawn under the message in the same frame, since the `Delay3` between is loading.
    fn show_message(&mut self, ctx: &mut Ctx, message: MedicineMessage) -> Transition {
        ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
        PartyMenu::redraw_entries(ctx);
        let nick = Self::nick(ctx, self.mon);
        ctx.world.text.strings.insert(TextBuffer::NameBuffer, nick);
        self.no_text_delay = ctx.world.no_text_delay;
        ctx.world.no_text_delay = true;
        self.text(message.text(), After::Message)
    }

    /// `TryEvolvingMon`. A stone is the item and forces it; a Rare Candy's item is the species.
    fn try_evolving(&mut self, ctx: &Ctx) -> Transition {
        let stone = self.flow == Flow::EvoStone;
        let cur_item = if stone { self.item as u8 } else { ctx.world.party[self.mon as usize].mon.mon.species as u8 };
        self.phase = Phase::Child(After::Evolving);
        Transition::Push(Mode::Evolution(Evolution::try_evolving(self.mon, cur_item, stone)))
    }

    /// What follows a delay or a press.
    fn then(&mut self, ctx: &mut Ctx, then: Then) -> Transition {
        match then {
            Then::Healed => self.finish(ctx, USED),
            Then::StatsBox => {
                let stats = ctx.world.party[self.mon as usize].mon.stats;
                print_stats_box(&mut ctx.screen.ui, StatsBox::LevelUp, stats);
                self.phase = Phase::WaitButton(Then::LevelUp);
                self.update(ctx)
            }
            Then::LevelUp => {
                let level = ctx.world.party[self.mon as usize].mon.level;
                match LearnMove::from_level_up(ctx.world, self.mon, level) {
                    Some(learn) => {
                        self.phase = Phase::Child(After::RareCandyLearn);
                        Transition::Push(Mode::LearnMove(learn))
                    }
                    None => self.try_evolving(ctx),
                }
            }
        }
    }
}

impl ModeUpdate for UseItem {
    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        if let Flow::TmHm(mv) = self.flow {
            ctx.world.text.strings.insert(TextBuffer::StringBuffer, mv.name());
            let label = if self.item.is_hm() { "_BootedUpHMText" } else { "_BootedUpTMText" };
            return self.text(label, After::Booted);
        }
        let asks_for_a_party = matches!(self.flow, Flow::Medicine | Flow::Vitamin | Flow::RareCandy);
        if asks_for_a_party && ctx.world.party.is_empty() {
            self.phase = Phase::Child(After::Finish { result: NOT_USED, remove: false });
            let text = poke_core::charmap::encode("You don't have<LINE>any #MON!<PROMPT>").expect("the text encodes");
            return Transition::Push(Mode::TextBox(TextBox::new(text)));
        }
        self.choose_mon()
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase.clone() {
            Phase::Child(_) => Transition::Stay,
            Phase::WaitForSound(medicine) => {
                if !ctx.audio.sound_finished() {
                    return Transition::Stay;
                }
                match medicine {
                    Medicine::Cured(message) => {
                        ctx.audio.play_sound(sounds::SFX_HEAL_AILMENT);
                        self.show_message(ctx, message)
                    }
                    Medicine::Healed { old, new, message } => {
                        ctx.audio.play_sound(sounds::SFX_HEAL_HP);
                        ctx.world.text.numbers.insert(TextNumber::HpBarHpDifference, old.abs_diff(new) as u32);
                        let at = (1 + 2 * self.mon as usize) * SCREEN_TILES_X + 4;
                        let max = ctx.world.party[self.mon as usize].mon.stats[0];
                        let bar = HpBarAnimation::new(at, max, old, new, HpBarType::PartyMenu).expect("the HP moved");
                        self.phase = Phase::Bar(bar, message);
                        self.update(ctx)
                    }
                    Medicine::NoEffect => unreachable!("no effect is a text, not a sound"),
                }
            }
            Phase::Bar(mut bar, message) => {
                if bar.update(&mut ctx.screen.ui) {
                    return self.show_message(ctx, message);
                }
                self.phase = Phase::Bar(bar, message);
                Transition::Stay
            }
            Phase::Holding(frames) if frames > 1 => {
                self.phase = Phase::Holding(frames - 1);
                Transition::Stay
            }
            Phase::Holding(_) => {
                self.phase = Phase::WaitButton(Then::Healed);
                self.update(ctx)
            }
            Phase::WaitButton(then) => {
                if ctx.pad.low_sensitivity(ctx.frame_counter).intersects(Joypad::A | Joypad::B) {
                    self.answered += 1;
                    return self.then(ctx, then);
                }
                Transition::Stay
            }
            Phase::Sound { sound, playing: false } => {
                if !ctx.audio.sound_finished() {
                    return Transition::Stay;
                }
                match sound {
                    Sound::Stone => {
                        ctx.audio.play_sound(sounds::SFX_HEAL_AILMENT);
                        self.phase = Phase::Sound { sound, playing: true };
                        Transition::Stay
                    }
                    Sound::Denied => {
                        ctx.audio.play_sound(sounds::SFX_DENIED);
                        self.text("_MonCannotLearnMachineMoveText", After::ChooseMonAgain)
                    }
                }
            }
            Phase::Sound { .. } => {
                if !ctx.audio.sound_finished() {
                    return Transition::Stay;
                }
                self.try_evolving(ctx)
            }
        }
    }

    fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        let Phase::Child(after) = self.phase else { return Transition::Stay };
        match (after, outcome) {
            (After::ChooseMon, Outcome::Chosen(mon)) => self.chosen_mon(ctx, mon),
            // `.canceledItemUse` for a stone; a machine's `GBPalWhiteOutWithDelay3` is loading.
            (After::ChooseMon, _) if self.flow == Flow::EvoStone => Transition::Pop(Outcome::Chosen(NOT_USED)),
            (After::ChooseMon, _) if matches!(self.flow, Flow::TmHm(_)) => Transition::Pop(Outcome::Chosen(USED)),
            (After::ChooseMon, _) => self.finish(ctx, NOT_USED),
            (After::Finish { result, remove }, _) => {
                if remove {
                    self.remove_used_item(ctx);
                }
                self.finish(ctx, result)
            }
            (After::AskMoveAgain, _) => self.ask_move(),
            (After::AskMove, _) => {
                self.phase = Phase::Child(After::ChooseMove);
                let moves = ctx.world.party[self.mon as usize].mon.mon.moves;
                Transition::Push(Mode::MoveSelectionMenu(MoveSelectionMenu::relearn(moves)))
            }
            (After::ChooseMove, Outcome::Chosen(index)) => self.chosen_move(ctx, index),
            (After::ChooseMove, _) => self.choose_mon(),
            (After::Message, _) => {
                ctx.world.no_text_delay = self.no_text_delay;
                self.phase = Phase::Holding(50);
                Transition::Stay
            }
            (After::RareCandyMessage, _) => {
                ctx.world.no_text_delay = self.no_text_delay;
                self.then(ctx, Then::StatsBox)
            }
            (After::RareCandyLearn, _) => self.try_evolving(ctx),
            // A stone that evolved nothing has had no effect and is kept; a Rare Candy is used either way.
            (After::Evolving, outcome) if self.flow == Flow::EvoStone && outcome != Outcome::Chosen(1) =>
                self.text("_ItemUseNoEffectText", After::Finish { result: NOT_USED, remove: false }),
            (After::Evolving, _) => {
                self.remove_used_item(ctx);
                Transition::Pop(Outcome::Chosen(USED))
            }
            (After::Booted, _) => self.text("_TeachMachineMoveText", After::Teach),
            (After::Teach, _) => {
                self.phase = Phase::Child(After::ConfirmTeach);
                Transition::Push(Mode::TwoOptionMenu(TwoOptionMenu::new(TwoOptionMenuId::YesNo, YES_NO_AT, false)))
            }
            (After::ConfirmTeach, Outcome::Chosen(0)) => self.choose_mon(),
            (After::ConfirmTeach, _) => Transition::Pop(Outcome::Chosen(NO_MENU)),
            (After::ChooseMonAgain, _) => self.choose_mon(),
            (After::Learning, outcome) => {
                if outcome == Outcome::Chosen(1) && !self.item.is_hm() {
                    self.remove_used_item(ctx);
                }
                Transition::Pop(Outcome::Chosen(USED))
            }
        }
    }

    fn status(&self) -> Status {
        match self.phase {
            Phase::WaitButton(_) => Status::Waiting(Decision::Text),
            _ => Status::Busy,
        }
    }
}
