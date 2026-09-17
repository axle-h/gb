//! The battle's steps, a label at a time, from `InitBattle` to `EndOfBattle`.

use poke_core::item::ItemId;
use poke_core::move_name::PokemonMoveName;
use poke_core::moves::MoveData;
use poke_core::species::PokemonSpecies;
use poke_core::text_script::{TextBuffer, TextCommand, TextMoney, TextNumber, TextSound};
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::gfx::ui::{SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, Outcome, Transition};
use crate::modes::evolution::Evolution;
use crate::modes::learn_move::LearnMove;
use crate::modes::party_menu::{PartyMenu, PartyMenuType};
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::party::PartyMon;
use crate::systems::battle::apply::{apply_attack_to_pokemon, apply_damage_to_pokemon, handle_building_rage};
use crate::systems::battle::damage::{calc_move_damage, MoveDamage};
use crate::systems::battle::effects::{jump_move_effect, BattleText, MimicMenu};
use crate::systems::battle::effects::volatile::{mimic_lands, player_mimic_copy};
use crate::systems::battle::enemy::load_enemy_mon_data;
use crate::systems::battle::escape::try_running_from_battle;
use crate::systems::battle::trainer::{read_trainer, RIVAL3};
use crate::systems::battle::experience::{gain_experience, ExpEvent};
use crate::systems::battle::modified_stats::{apply_badge_stat_boosts, apply_burn_and_paralysis_penalties};
use crate::systems::battle::turn::{check_for_disobedience, check_num_attacks_left, check_status_conditions,
                                    decrement_pp, decrease_own_hp, metronome_pick_move, mirror_move_copy_move,
                                    Continuation, MoveMenu};
use crate::systems::battle::turn_order::first_to_move;
use crate::systems::battle::ai::{select_enemy_move, trainer_ai, AiAction, CANNOT_MOVE};
use crate::systems::battle::{effect, status, Battle, BattleKind, BattleMon, CriticalHitOrOhko, Side,
                             Status1, Status2, Status3, BASE_STAT_LEVEL, PP_MASK};
use crate::systems::learn_move::format_moves_string;
use crate::systems::math::{add_bcd, divide, multiply};
use crate::systems::pp::max_pp;
use crate::systems::print_num::{print_number, NumberFormat};
use crate::systems::status_screen::type_name;
use super::animate::Before;
use super::animation::{anim, animation_type, Routine};
use super::hud::{self, BACK_PIC_TILE, FRONT_PIC_TILE};
use super::transition::Choice;
use super::menus::Menu;
use super::present::{BattleBox, Present};
use super::text::{chain, far, spliced};
use super::{result, BattleMode, BattleType, Opponent};

const RIVAL1: u8 = 0x19;
const RIVAL2: u8 = 0x2A;
const LANCE: u8 = 0x2F;
/// `EFFECTIVE`.
const EFFECTIVE: u8 = 10;
const TERMINATOR: u8 = 0x50;
const NEXT: u8 = 0x4E;
/// `STRUGGLE`.
const STRUGGLE: u8 = PokemonMoveName::Struggle as u8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    InitBattle,
    /// `_InitBattleCommon` after `PrintBeginningBattleText`.
    AfterBeginningText,
    StartBattle,
    /// `.playerSendOutFirstMon` once the trainer's picture has slid away.
    SendOutFirstMon,
    SendOutMon,
    MainInBattleLoop,
    /// After `DisplayBattleMenu` returns, where `ran` is its carry.
    AfterBattleMenu,
    DisplayBattleMenu,
    BattleMenuChosen(u8),
    MoveSelectionMenu,
    /// `MoveSelectionMenu` with `wMoveMenuType` 1, from the player's `MimicEffect`.
    MimicMoveSelectionMenu,
    SelectMenuItem,
    AfterMoveSelection { chosen: bool },
    SelectEnemyMove,
    PlayerFirst(u8),
    EnemyFirst(u8),
    Execute(Side),
    HasNoSpecialConditions(Side),
    CheckIfNeedsToChargeUp(Side),
    CanExecuteMove(Side),
    CalcMoveDamage(Side),
    HandleIfMoveMissed(Side),
    GetAnimationType(Side),
    PlayMoveAnimation(Side),
    FlyOrChargeEffect(Side),
    MirrorMoveCheck(Side),
    NotDone(Side),
    ExecuteOtherEffects(Side),
    ExecuteDone(Side),
    HandleEnemyMonFainted,
    AfterFaintEnemy,
    HandlePlayerMonFainted,
    AfterRemoveFaintedPlayerMon,
    UseNextMonAnswered,
    ChooseNextMon,
    NextMonChosen,
    EndOfBattle,
    EndOfBattleAfterEvolution,
    /// `EnemySendOut`, or `EnemySendOutFirstMon` for the first.
    EnemySendOut { first: bool },
    /// `SwitchPlayerMon`'s `SaveScreenTilesToBuffer1` after `SendOutMon`.
    SaveScreenAfterSwitch,
    SafariMenu,
    AfterSafariMenu,
    /// `StartBattle.checkAnyPartyAlive`, back to the Safari Zone's menu.
    SafariCheckAnyPartyAlive,
    BagWasSelected,
    BagChosen,
    /// `UseBagItem` after `UseItem`.
    AfterUseBagItem,
    /// A party-menu item's return, and the battle mon brought into line.
    ItemUsedFromParty,
    AskName,
    NicknameAnswered,
    NicknameEntered,
    /// `PartyMenuOrRockOrRun.partyMenuWasSelected`.
    PartyMenuFromBattle,
    /// `.checkIfPartyMonWasSelected`.
    PartyMonSelected,
    /// `TrainerAboutToUseText`'s yes or no.
    ShiftAnswered,
    ShiftMonChosen,
    /// `EnemySendOut` from `.next4`: `switch` is the party slot the player shifts to.
    EnemySentOut { switch: Option<u8> },
    /// `StartBattle` once the trainer's first mon is out.
    StartBattleAfterEnemy,
    /// `ChooseNextMon`'s return: the enemy's mon still standing, or one to replace.
    AfterChooseNextMon,
    ReplaceFaintedEnemyMon,
    AfterReplaceFaintedEnemyMon,
    /// `SwitchEnemyMon`'s return to `TrainerAI`'s caller, `wFirstMonsNotOutYet` cleared.
    AiSwitched { player_first: bool },
    /// The battle has returned out of `MainInBattleLoop`.
    BattleOver,
}

impl BattleMode {
    pub(super) fn push(&mut self, present: Present) {
        self.queue.push_back(present);
    }

    /// `DrawPlayerHUDAndHPBar` or `DrawEnemyHUDAndHPBar` of the mon as it stands at this step.
    pub(super) fn push_hud(&mut self, side: Side) {
        let mon = Box::new(self.b().side(side).mon.clone());
        self.push(match side {
            Side::Player => Present::DrawPlayerHud { mon, nick: self.player_nick.clone() },
            Side::Enemy => Present::DrawEnemyHud { mon, nick: self.enemy_nick.clone() },
        });
    }

    /// `GoBackToPartyMenu`.
    pub(super) fn go_back_to_party_menu(&mut self, kind: PartyMenuType) {
        self.push(Present::Push(Box::new(Mode::PartyMenu(PartyMenu::again(kind)))));
    }

    pub(super) fn goto(&mut self, step: Step) {
        self.steps.push(step);
    }

    /// `call routine`, carrying on at `then` when it returns.
    pub(super) fn call(&mut self, routine: Step, then: Step) {
        self.steps.push(then);
        self.steps.push(routine);
    }

    pub(super) fn b(&self) -> &Battle {
        self.battle.as_ref().expect("the battle has started")
    }

    /// `PrintText` with `<USER>` as `whose_turn`'s mon.
    fn text(&mut self, commands: Vec<TextCommand>, whose_turn: Side) {
        let commands = spliced(commands, whose_turn, &self.player_nick, &self.enemy_nick);
        self.push(Present::Text(commands));
    }

    pub(super) fn far_text(&mut self, label: &str, whose_turn: Side) {
        self.text(far(label), whose_turn);
    }

    /// `PrintEmptyString`: the message box, and nothing in it.
    pub(super) fn empty_text(&mut self) {
        self.push(Present::Text(vec![TextCommand::Text(vec![])]));
    }

    /// A battle routine's text for `whose_turn`, with whatever its `text_asm` appends and the buffers
    /// it reads as the battle stands now.
    fn battle_text(&mut self, text: BattleText, whose_turn: Side) {
        let effect = self.b().side(whose_turn).current_move.effect;
        self.battle_text_for(text, whose_turn, effect);
    }

    /// `battle_text` for a stat text whose move effect has been swapped out for the moment, as
    /// Rage borrows `ATTACK_UP1_EFFECT`.
    pub(super) fn battle_text_for(&mut self, text: BattleText, whose_turn: Side, move_effect: u8) {
        use BattleText::*;
        let battle = self.b();
        let me = battle.side(whose_turn);
        let target = battle.side(whose_turn.other());
        let move_name = |id: u8| PokemonMoveName::from_repr(id).map_or(vec![], PokemonMoveName::name);
        let mut commands = far(&format!("_{text:?}"));
        let mut strings = vec![];
        match text {
            MonsStatsRoseText | MonsStatsFellText => {
                let effect = move_effect;
                strings.push((TextBuffer::StringBuffer, stat_name(effect)));
                let greatly = match text {
                    MonsStatsRoseText => effect >= effect::ATTACK_DOWN1_EFFECT,
                    _ => (effect::BIDE_EFFECT..effect::ATTACK_DOWN_SIDE_EFFECT).contains(&effect),
                };
                let (greatly_label, plain) = if text == MonsStatsRoseText {
                    ("_GreatlyRoseText", "_RoseText")
                } else {
                    ("_GreatlyFellText", "_FellText")
                };
                if greatly {
                    commands.push(TextCommand::Pause);
                    commands.extend(far(greatly_label));
                }
                commands.extend(far(plain));
            }
            GettingPumpedText => commands.insert(0, TextCommand::Pause),
            ChargeMoveEffectText => {
                use PokemonMoveName as M;
                let label = match PokemonMoveName::from_repr(me.current_move.animation) {
                    Some(M::RazorWind) => "_MadeWhirlwindText",
                    Some(M::Solarbeam) => "_TookInSunlightText",
                    Some(M::SkullBash) => "_LoweredItsHeadText",
                    Some(M::SkyAttack) => "_SkyAttackGlowingText",
                    Some(M::Fly) => "_FlewUpHighText",
                    _ => "_DugAHoleText",
                };
                commands.extend(far(label));
            }
            MimicLearnedMoveText => {
                let slot = match whose_turn {
                    Side::Player => self.move_menu.current,
                    Side::Enemy => me.move_list_index,
                } as usize;
                let id = me.mon.moves[slot].map_or(0, |name| name as u8);
                strings.push((TextBuffer::NameBuffer, move_name(id)));
            }
            MoveIsDisabledText => strings.push((TextBuffer::NameBuffer, move_name(me.disabled_move_number))),
            MoveWasDisabledText => strings.push((TextBuffer::NameBuffer, move_name(target.disabled_move_number))),
            TransformedText => strings.push((TextBuffer::NameBuffer, target.mon.species.name())),
            BeganToNapText | IgnoredOrdersText | LoafingAroundText | TurnedAwayText | WontObeyText =>
                strings.push((TextBuffer::BattleMonNick, self.player_nick.clone())),
            AIBattleWithdrawText => strings.push((TextBuffer::EnemyMonNick, self.enemy_nick.clone())),
            _ => {}
        }
        let commands = spliced(commands, whose_turn, &self.player_nick, &self.enemy_nick);
        self.push(Present::TextWith { commands, strings, numbers: vec![] });
    }

    fn party(ctx: &Ctx) -> Vec<PartyMon> {
        ctx.world.party.iter().map(|named| named.mon.clone()).collect()
    }

    fn store_party(ctx: &mut Ctx, party: Vec<PartyMon>) {
        for (named, mon) in ctx.world.party.iter_mut().zip(party) {
            named.mon = mon;
        }
    }

    /// Runs `f` over the battle and the party, as the arithmetic takes them.
    fn with_party<R>(&mut self, ctx: &mut Ctx, f: impl FnOnce(&mut Battle, &mut [PartyMon], &mut Ctx) -> R) -> R {
        let mut party = Self::party(ctx);
        let answer = f(self.battle.as_mut().expect("the battle has started"), &mut party, ctx);
        Self::store_party(ctx, party);
        answer
    }

    /// One step. `Some` for the battle ending.
    pub(super) fn step(&mut self, step: Step, ctx: &mut Ctx) -> Option<Transition> {
        match step {
            Step::InitBattle => self.init_battle(ctx),
            Step::AfterBeginningText => {
                // `PrintText` of nothing, then the screen saved, cleared and put back.
                self.empty_text();
                self.push(Present::SaveScreen1);
                self.push(Present::Clear { x: 0, y: 0, width: SCREEN_TILES_X, height: SCREEN_TILES_Y });
                self.push(Present::LoadScreen1);
                self.push(Present::Clear { x: 9, y: 7, width: 10, height: 5 });
                self.push(Present::Clear { x: 1, y: 0, width: 10, height: 4 });
                self.push(Present::ClearSprites);
                if self.b().kind == BattleKind::Wild {
                    self.push_hud(Side::Enemy);
                }
                self.goto(Step::StartBattle);
            }
            Step::StartBattle => {
                let battle = self.battle_mut();
                battle.gain_exp_flags = 0;
                battle.fought_current_enemy_flags = 0;
                self.action_taken = false;
                self.first_mons_not_out_yet = true;
                if self.b().kind == BattleKind::Trainer {
                    self.call(Step::EnemySendOut { first: true }, Step::StartBattleAfterEnemy);
                } else {
                    self.goto(Step::StartBattleAfterEnemy);
                }
            }
            Step::StartBattleAfterEnemy => {
                self.push(Present::Frames(40));
                self.push(Present::SaveScreen1);
                if !ctx.world.party.iter().any(|mon| mon.mon.mon.hp != 0) {
                    self.goto(Step::BattleOver);
                    return None;
                }
                self.push(Present::LoadScreen1);
                if self.battle_type != BattleType::Normal {
                    return { self.goto(Step::SafariMenu); None };
                }
                self.push(Present::SlideTrainerOff { column: 0 });
                self.push(Present::SaveScreen1);
                self.goto(Step::SendOutFirstMon);
            }
            Step::SendOutFirstMon => {
                let slot = ctx.world.party.iter().position(|mon| mon.mon.mon.hp != 0).expect("a mon that can fight");
                let battle = self.battle_mut();
                battle.player_mon_number = slot as u8;
                battle.gain_exp_flags |= 1 << slot;
                battle.fought_current_enemy_flags |= 1 << slot;
                self.load_battle_mon_from_party(ctx);
                self.push(Present::LoadScreen1);
                self.call(Step::SendOutMon, Step::MainInBattleLoop);
            }
            Step::SendOutMon => self.send_out_mon(ctx),
            Step::MainInBattleLoop => self.main_in_battle_loop(ctx),
            Step::DisplayBattleMenu => self.display_battle_menu(ctx),
            Step::BattleMenuChosen(id) => self.battle_menu_chosen(id, ctx),
            Step::AfterBattleMenu => self.after_battle_menu(),
            Step::MoveSelectionMenu => self.move_selection_menu(ctx),
            Step::MimicMoveSelectionMenu => self.mimic_move_selection_menu(ctx),
            Step::SelectMenuItem => self.select_menu_item(ctx),
            Step::AfterMoveSelection { chosen } => {
                self.push(Present::LoadScreen1);
                self.push_hud(Side::Player);
                self.push_hud(Side::Enemy);
                self.goto(if chosen { Step::SelectEnemyMove } else { Step::MainInBattleLoop });
            }
            Step::SelectEnemyMove => {
                let battle = self.battle.as_mut().expect("a battle");
                select_enemy_move(battle, ctx.rng);
                let first = first_to_move(battle, ctx.rng);
                self.goto(match first {
                    Side::Player => Step::PlayerFirst(0),
                    Side::Enemy => Step::EnemyFirst(0),
                });
            }
            Step::PlayerFirst(stage) => self.player_first(stage, ctx),
            Step::EnemyFirst(stage) => self.enemy_first(stage, ctx),
            Step::Execute(side) => self.execute(side, ctx),
            Step::HasNoSpecialConditions(side) => self.has_no_special_conditions(side, ctx),
            Step::CheckIfNeedsToChargeUp(side) => {
                let move_effect = self.b().side(side).current_move.effect;
                if matches!(move_effect, effect::CHARGE_EFFECT | effect::FLY_EFFECT) {
                    self.jump_move_effect(side, ctx);
                } else {
                    self.goto(Step::CanExecuteMove(side));
                }
            }
            Step::CanExecuteMove(side) => self.can_execute_move(side, ctx),
            Step::CalcMoveDamage(side) => {
                let exit = self.with_party(ctx, |battle, party, ctx| calc_move_damage(battle, party, side, ctx.rng));
                self.goto(match exit {
                    MoveDamage::NoDamage => Step::FlyOrChargeEffect(side),
                    _ => Step::HandleIfMoveMissed(side),
                });
            }
            Step::HandleIfMoveMissed(side) => {
                let battle = self.b();
                self.goto(match (battle.move_missed, battle.side(side).current_move.effect) {
                    (false, _) => Step::GetAnimationType(side),
                    (true, effect::EXPLODE_EFFECT) => Step::PlayMoveAnimation(side),
                    (true, _) => Step::FlyOrChargeEffect(side),
                });
            }
            Step::GetAnimationType(side) => {
                use animation_type::*;
                let with_effect = self.b().side(side).current_move.effect != 0;
                let kind = match (side, with_effect) {
                    (Side::Player, false) => BLINK_ENEMY_MON_SPRITE,
                    (Side::Player, true) => SHAKE_SCREEN_HORIZONTALLY_LIGHT,
                    (Side::Enemy, false) => SHAKE_SCREEN_VERTICALLY,
                    (Side::Enemy, true) => SHAKE_SCREEN_HORIZONTALLY_HEAVY,
                };
                self.move_animation(side, kind);
                self.goto(Step::MirrorMoveCheck(side));
            }
            Step::PlayMoveAnimation(side) => {
                self.move_animation(side, animation_type::NONE);
                self.goto(Step::MirrorMoveCheck(side));
            }
            Step::FlyOrChargeEffect(side) => {
                self.push(Present::Frames(30));
                if matches!(self.b().side(side).current_move.effect, effect::FLY_EFFECT | effect::CHARGE_EFFECT) {
                    self.play_battle_animation(anim::STATUS_AFFECTED_ANIM, side);
                }
                self.goto(Step::MirrorMoveCheck(side));
            }
            Step::MirrorMoveCheck(side) => self.mirror_move_check(side, ctx),
            Step::NotDone(side) => self.not_done(side, ctx),
            Step::ExecuteOtherEffects(side) => {
                let move_effect = self.b().side(side).current_move.effect;
                if move_effect != 0 && !SPECIAL_EFFECTS.contains(&move_effect) {
                    self.jump_move_effect(side, ctx);
                } else {
                    self.goto(Step::ExecuteDone(side));
                }
            }
            Step::ExecuteDone(side) => {
                if side == Side::Player {
                    self.action_taken = false;
                }
                self.target_standing = true;
            }
            Step::HandleEnemyMonFainted => {
                self.in_handle_player_mon_fainted = false;
                self.call_faint_enemy_pokemon(ctx);
                self.goto(Step::AfterFaintEnemy);
            }
            Step::AfterFaintEnemy => self.after_faint_enemy(ctx),
            Step::HandlePlayerMonFainted => {
                self.in_handle_player_mon_fainted = true;
                self.remove_fainted_player_mon(ctx);
                self.goto(Step::AfterRemoveFaintedPlayerMon);
            }
            Step::AfterRemoveFaintedPlayerMon => self.after_remove_fainted_player_mon(ctx),
            Step::UseNextMonAnswered => {
                // `CHOSE_SECOND_ITEM` is NO, and B.
                if self.outcome.take() == Some(Outcome::Chosen(0)) && ctx.menu.exit_method == crate::modes::menu_input::MenuExit::Chose {
                    self.goto(Step::ChooseNextMon);
                } else {
                    let speed = ctx.world.party[0].mon.stats[3];
                    let mut attempts = self.num_run_attempts;
                    let ghost = self.is_ghost_battle(ctx);
                let run = try_running_from_battle(self.b(), speed, ghost, &mut attempts, ctx.rng);
                    self.num_run_attempts = attempts;
                    self.present_run(&run);
                    if !run.escaped {
                        self.goto(Step::ChooseNextMon);
                    }
                }
            }
            Step::ChooseNextMon => {
                self.push(Present::Push(Box::new(Mode::PartyMenu(PartyMenu::new(PartyMenuType::Battle)))));
                self.goto(Step::NextMonChosen);
            }
            Step::NextMonChosen => self.next_mon_chosen(ctx),
            Step::BattleOver => self.goto(Step::EndOfBattle),
            Step::EnemySendOut { first } => self.enemy_send_out(first, ctx),
            Step::SaveScreenAfterSwitch => self.push(Present::SaveScreen1),
            Step::SafariMenu => self.call(Step::DisplayBattleMenu, Step::AfterSafariMenu),
            Step::AfterSafariMenu => self.after_safari_menu(ctx),
            Step::SafariCheckAnyPartyAlive => {
                if !ctx.world.party.iter().any(|mon| mon.mon.mon.hp != 0) {
                    return { self.goto(Step::BattleOver); None };
                }
                self.push(Present::LoadScreen1);
                self.goto(Step::SafariMenu);
            }
            Step::BagWasSelected => self.bag_was_selected(ctx),
            Step::BagChosen => self.bag_chosen(ctx),
            Step::AfterUseBagItem => self.after_use_bag_item(ctx),
            Step::ItemUsedFromParty => self.item_used_from_party(ctx),
            Step::AskName => self.ask_name(),
            Step::NicknameAnswered => self.nickname_answered(ctx),
            Step::NicknameEntered => self.nickname_entered(ctx),
            Step::PartyMenuFromBattle => {
                self.outcome = None;
                self.push(Present::LoadScreen1);
                self.move_to_swap = 0;
                self.push(Present::Push(Box::new(Mode::PartyMenu(PartyMenu::new(PartyMenuType::Normal)))));
                self.goto(Step::PartyMonSelected);
            }
            Step::PartyMonSelected => match self.outcome.take() {
                Some(Outcome::Chosen(slot)) => {
                    self.push(Present::TextBox(BattleBox::SwitchStatsCancel));
                    self.menu = Some(Menu::switch_stats_cancel(slot));
                    self.push(Present::Menu);
                }
                _ => {
                    self.push(Present::ClearSprites);
                    self.push(Present::LoadScreen2);
                    self.goto(Step::DisplayBattleMenu);
                }
            },
            Step::ShiftAnswered => {
                let yes = self.outcome.take() == Some(Outcome::Chosen(0))
                    && ctx.menu.exit_method == crate::modes::menu_input::MenuExit::Chose;
                if yes {
                    self.push(Present::Push(Box::new(Mode::PartyMenu(PartyMenu::new(PartyMenuType::Battle)))));
                    self.goto(Step::ShiftMonChosen);
                } else {
                    self.goto(Step::EnemySentOut { switch: None });
                }
            }
            Step::ShiftMonChosen => {
                let Some(Outcome::Chosen(slot)) = self.outcome.take() else {
                    self.push(Present::LoadScreen1);
                    return { self.goto(Step::EnemySentOut { switch: None }); None };
                };
                if slot == self.b().player_mon_number {
                    self.push(Present::TextWith {
                        commands: far("_AlreadyOutText"),
                        strings: vec![(TextBuffer::BattleMonNick, self.player_nick.clone())],
                        numbers: vec![],
                    });
                }
                if slot == self.b().player_mon_number || ctx.world.party[slot as usize].mon.mon.hp == 0 {
                    self.go_back_to_party_menu(PartyMenuType::Battle);
                    return { self.goto(Step::ShiftMonChosen); None };
                }
                self.push(Present::LoadScreen1);
                self.goto(Step::EnemySentOut { switch: Some(slot) });
            }
            Step::EnemySentOut { switch } => self.enemy_sent_out(switch, ctx),
            Step::AfterChooseNextMon => {
                if self.b().enemy.mon.hp != 0 {
                    self.goto(Step::MainInBattleLoop);
                } else {
                    self.goto(Step::ReplaceFaintedEnemyMon);
                }
            }
            Step::ReplaceFaintedEnemyMon => {
                self.action_taken = true;
                self.push(Present::DrawEnemyPokeballs);
                self.call(Step::EnemySendOut { first: false }, Step::AfterReplaceFaintedEnemyMon);
            }
            Step::AfterReplaceFaintedEnemyMon => {
                let battle = self.battle_mut();
                battle.enemy.current_move.animation = 0;
                battle.ai_layer2_encouragement = 0;
                self.action_taken = false;
                self.goto(Step::MainInBattleLoop);
            }
            Step::AiSwitched { player_first } => {
                self.first_mons_not_out_yet = false;
                self.goto(if player_first { Step::PlayerFirst(3) } else { Step::EnemyFirst(2) });
            }
            Step::EndOfBattle => self.end_of_battle(ctx),
            Step::EndOfBattleAfterEvolution => return Some(self.finish(ctx)),
        }
        None
    }

    fn init_battle(&mut self, ctx: &mut Ctx) {
        // `InitBattleVariables`: the party's, the bag's and the battle menu's saved cursors, and the
        // move list's, which the battle keeps.
        ctx.menu.party_and_bills = 0;
        ctx.menu.bag_saved = 0;
        ctx.menu.battle_and_start = 0;
        ctx.menu.list_scroll = 0;
        let (species, level) = match self.opponent.clone() {
            Opponent::Wild { species, level } => (species, level),
            Opponent::Trainer { class, number, lone_attack, rival_starter } =>
                return self.init_opponent(class, number, lone_attack, rival_starter, ctx),
        };
        // `PlayBattleMusic`.
        self.push(Present::Sound(SoundId::STOP_ALL_MUSIC));
        self.push(Present::Frames(1));
        self.push(Present::Music(sounds::MUSIC_WILD_BATTLE));

        if super::safari::is_safari_map(ctx.world.location.map) {
            self.battle_type = BattleType::Safari;
        }
        let mut battle = Battle::new(BattleKind::Wild, &ctx.world.party[0].mon, vec![]);
        load_enemy_mon_data(&mut battle, &mut ctx.world.pokedex, species, level, 0, ctx.rng);
        self.enemy_nick = species.name();
        self.battle = Some(battle);
        // `RESTLESS_SOUL` is Marowak's own constant, so every Marowak is shown as a ghost.
        let ghost_pic = species == PokemonSpecies::Marowak || self.is_ghost_battle(ctx);
        if ghost_pic {
            self.enemy_nick = poke_core::charmap::encode("GHOST").expect("encodes");
        }

        // `DoBattleTransitionAndInitBattleVariables`: a frame, the transition, and the screen cleared.
        self.push(Present::Frames(1));
        self.push(Present::Transition(Self::transition_choice(false, level, ctx)));
        self.push(Present::LoadHudAndHpBarTiles);
        self.push(Present::Clear { x: 0, y: 0, width: SCREEN_TILES_X, height: SCREEN_TILES_Y });
        self.push(if ghost_pic { Present::LoadGhostPic } else { Present::LoadFrontPic(species) });
        self.push(Present::Pic { x: 12, y: 0, first: FRONT_PIC_TILE });

        // `SlidePlayerAndEnemySilhouettesOnScreen`, whose sliding is a seam.
        self.silhouettes();

        // `PrintBeginningBattleText`: from the tower's third floor a ghost is either unveiled, and
        // loaded again, or not identified at all.
        use poke_core::map::Map;
        let upper_tower = (Map::PokemonTower3F as u8..=Map::PokemonTower7F as u8).contains(&(ctx.world.location.map as u8));
        if upper_tower {
            let scope = ctx.world.bag.quantity_of(ItemId::SilphScope) > 0;
            let appeared = |nick: &[u8]| Present::TextWith {
                commands: far("_EnemyAppearedText"),
                strings: vec![(TextBuffer::EnemyMonNick, nick.to_vec())],
                numbers: vec![],
            };
            if !scope {
                self.push(appeared(&self.enemy_nick));
                self.far_text("_GhostCantBeIDdText", Side::Enemy);
                return self.goto(Step::AfterBeginningText);
            }
            if species == PokemonSpecies::Marowak {
                self.push(appeared(&self.enemy_nick));
                self.far_text("_UnveiledGhostText", Side::Enemy);
            }
            load_enemy_mon_data(self.battle.as_mut().expect("a battle"), &mut ctx.world.pokedex, species, level, 0, ctx.rng);
            self.enemy_nick = species.name();
            if species == PokemonSpecies::Marowak {
                // `MarowakAnim`, then the appearance with no cry and no party HUD, falling through
                // into the trainer's `.playSFX`.
                self.animate(Routine::Marowak, Side::Enemy);
                self.push(Present::LoadFrontPic(species));
                self.push(Present::TextWith {
                    commands: far("_WildMonAppearedText"),
                    strings: vec![(TextBuffer::EnemyMonNick, self.enemy_nick.clone())],
                    numbers: vec![],
                });
                self.push(Present::Modifiers { frequency: 0, tempo: 0x80 });
        self.push(Present::Sound(sounds::SFX_TRAINER_APPEARED));
                self.push(Present::WaitForSound);
                return self.goto(Step::AfterBeginningText);
            }
        }
        self.push(Present::Cry(species as u8));
        self.push(Present::DrawAllPokeballs);
        self.push(Present::TextWith {
            commands: far("_WildMonAppearedText"),
            strings: vec![(TextBuffer::EnemyMonNick, self.enemy_nick.clone())],
            numbers: vec![],
        });
        self.goto(Step::AfterBeginningText);
    }

    /// `SlidePlayerAndEnemySilhouettesOnScreen`: the player's back pic, the text box, the head's
    /// rows cleared for OAM to draw, and the slide.
    fn silhouettes(&mut self) {
        self.push(Present::LoadPlayerBackPic { old_man: self.battle_type == BattleType::OldMan });
        self.push(Present::Pic { x: 1, y: 5, first: BACK_PIC_TILE });
        self.push(Present::TextBox(BattleBox::MessageBox));
        self.push(Present::Clear { x: 1, y: 5, width: 7, height: 3 });
        self.push(Present::Silhouettes);
        self.push(Present::Pic { x: 1, y: 5, first: BACK_PIC_TILE });
    }

    /// What `BattleTransition` reads: the opponent, `wCurEnemyLevel`, the first party mon that can
    /// fight, and the map.
    fn transition_choice(trainer: bool, enemy_level: u8, ctx: &Ctx) -> Choice {
        let player_level = ctx.world.party.iter().find(|mon| mon.mon.mon.hp != 0).map_or(0, |mon| mon.mon.level);
        Choice { trainer, enemy_level, player_level, map: ctx.world.location.map }
    }

    /// `InitOpponent` for a trainer: the music, the party and the prize, the pic, and the trainer's
    /// challenge.
    fn init_opponent(&mut self, class: u8, number: u8, lone_attack: u8, rival_starter: u8, ctx: &mut Ctx) {
        // `PlayBattleMusic`, reading a gym leader's `wGymLeaderNo` from its lone move.
        let music = match class {
            _ if lone_attack != 0 => sounds::MUSIC_GYM_LEADER_BATTLE,
            RIVAL3 => sounds::MUSIC_FINAL_BATTLE,
            LANCE => sounds::MUSIC_GYM_LEADER_BATTLE,
            _ => sounds::MUSIC_TRAINER_BATTLE,
        };
        self.push(Present::Sound(SoundId::STOP_ALL_MUSIC));
        self.push(Present::Frames(1));
        self.push(Present::Music(music));

        // `GetTrainerInformation` and `ReadTrainer`.
        self.trainer_name = match class {
            RIVAL1 | RIVAL2 | RIVAL3 => ctx.world.rival_name.clone(),
            _ => hud::trainer_class_name(class),
        };
        let party = read_trainer(class, number, lone_attack, rival_starter, ctx.world.player_id);
        self.prize_money = party.money;
        let mut battle = Battle::new(BattleKind::Trainer, &ctx.world.party[0].mon, party.mons);
        battle.trainer_class = class;
        battle.ai_count = 0xFF;
        battle.enemy.mon.party_pos = 0xFF;
        self.battle = Some(battle);

        self.push(Present::Frames(1));
        let last_level = self.b().enemy_party.last().map_or(0, |mon| mon.level);
        self.push(Present::Transition(Self::transition_choice(true, last_level, ctx)));
        self.push(Present::LoadHudAndHpBarTiles);
        self.push(Present::Clear { x: 0, y: 0, width: SCREEN_TILES_X, height: SCREEN_TILES_Y });
        self.push(Present::LoadTrainerPic(class));
        self.push(Present::Pic { x: 12, y: 0, first: FRONT_PIC_TILE });

        self.silhouettes();

        // `PrintBeginningBattleText` for a trainer.
        self.push(Present::Modifiers { frequency: 0, tempo: 0x80 });
        self.push(Present::Sound(sounds::SFX_TRAINER_APPEARED));
        self.push(Present::WaitForSound);
        self.push(Present::Frames(20));
        self.push(Present::DrawAllPokeballs);
        self.push(Present::TextWith {
            commands: far("_TrainerWantsToFightText"),
            strings: vec![(TextBuffer::TrainerName, self.trainer_name.clone())],
            numbers: vec![],
        });
        self.goto(Step::AfterBeginningText);
    }

    /// `EnemySendOut` to `.next4`: the trainer's pic away, the next mon with HP loaded, and the
    /// shift prompt where the options ask for it.
    fn enemy_send_out(&mut self, first: bool, ctx: &mut Ctx) {
        let battle = self.battle_mut();
        if !first {
            let bit = 1 << battle.player_mon_number;
            battle.gain_exp_flags = bit;
            battle.fought_current_enemy_flags = bit;
        }
        battle.enemy.status1 = Status1::empty();
        battle.enemy.status2 = Status2::empty();
        battle.enemy.status3 = Status3::empty();
        battle.enemy.disabled_move = 0;
        battle.enemy.disabled_move_number = 0;
        battle.enemy.minimized = 0;
        battle.player.used_move = 0;
        battle.enemy.used_move = 0;
        battle.ai_count = 0xFF;
        battle.player.status1.remove(Status1::USING_TRAPPING_MOVE);
        self.push(Present::SlideEnemyTrainerOff { column: 0 });
        self.empty_text();
        self.push(Present::SaveScreen1);

        let battle = self.b();
        let out = battle.enemy.mon.party_pos;
        let which = (0..battle.enemy_party.len() as u8)
            .find(|&slot| slot != out && battle.enemy_party[slot as usize].mon.hp != 0)
            .expect("a mon to send out");
        let next = &battle.enemy_party[which as usize];
        let (species, level) = (next.mon.species, next.level);
        load_enemy_mon_data(self.battle.as_mut().expect("a battle"), &mut ctx.world.pokedex, species, level, which, ctx.rng);
        self.enemy_nick = species.name();
        self.last_switch_in_enemy_hp = self.b().enemy.mon.hp;

        let shift = !self.first_mons_not_out_yet && ctx.world.party.len() > 1
            && ctx.world.options.battle_style == crate::world::BattleStyle::Shift;
        if !shift {
            return self.goto(Step::EnemySentOut { switch: None });
        }
        self.push(Present::TextWith {
            commands: far("_TrainerAboutToUseText"),
            strings: vec![(TextBuffer::TrainerName, self.trainer_name.clone()),
                          (TextBuffer::EnemyMonNick, self.enemy_nick.clone())],
            numbers: vec![],
        });
        self.push(Present::Push(Box::new(Mode::TwoOptionMenu(TwoOptionMenu::new(TwoOptionMenuId::YesNo, (0, 7), false)))));
        self.goto(Step::ShiftAnswered);
    }

    /// `EnemySendOut` from `.next4`: the mon sent out, and the player's shift if it chose one.
    fn enemy_sent_out(&mut self, switch: Option<u8>, ctx: &mut Ctx) {
        self.push(Present::ClearSprites);
        self.push(Present::Clear { x: 0, y: 0, width: 11, height: 4 });
        self.push(Present::TextWith {
            commands: far("_TrainerSentOutText"),
            strings: vec![(TextBuffer::TrainerName, self.trainer_name.clone()),
                          (TextBuffer::EnemyMonNick, self.enemy_nick.clone())],
            numbers: vec![],
        });
        let species = self.b().enemy.mon.species;
        self.push(Present::LoadFrontPic(species));
        self.push(Present::SendingOut { stage: 0, ball: 0x4C + 2, at: (15, 6), base: FRONT_PIC_TILE.wrapping_sub(BACK_PIC_TILE) });
        self.push(Present::Cry(species as u8));
        self.push_hud(Side::Enemy);
        let Some(slot) = switch else { return };
        let battle = self.battle_mut();
        battle.gain_exp_flags = 0;
        battle.fought_current_enemy_flags = 0;
        self.push(Present::SaveScreen1);
        self.switch_player_mon(slot, ctx);
    }

    /// `SwitchPlayerMon`: the mon out called back, and `slot`'s sent out.
    fn switch_player_mon(&mut self, slot: u8, ctx: &mut Ctx) {
        // `RetreatMon`: how much of the enemy's HP has gone since it came out decides the word.
        let enemy = &self.b().enemy.mon;
        let lost = self.last_switch_in_enemy_hp.wrapping_sub(enemy.hp);
        let product = multiply(lost as u32 & 0xFFFF, 25).to_be_bytes();
        let divisor = ((enemy.stats[0] >> 2) & 0xFF) as u8;
        let ratio = divide(product, divisor, 4).0[3];
        let word = match ratio {
            0 => "_EnoughText",
            1..=29 => "_ComeBackText",
            30..=69 => "_OKExclamationText",
            _ => "_GoodText",
        };
        let mut commands = far("_PlayerMon2Text");
        commands.extend(far(word));
        if word != "_ComeBackText" {
            commands.extend(far("_ComeBackText"));
        }
        self.push(Present::TextWith {
            commands,
            strings: vec![(TextBuffer::BattleMonNick, self.player_nick.clone())],
            numbers: vec![],
        });
        self.push(Present::Frames(50));
        self.push(Present::Retreating { stage: 0 });
        let battle = self.battle_mut();
        battle.player_mon_number = slot;
        battle.gain_exp_flags |= 1 << slot;
        battle.fought_current_enemy_flags |= 1 << slot;
        self.load_battle_mon_from_party(ctx);
        self.call(Step::SendOutMon, Step::SaveScreenAfterSwitch);
    }

    /// `.partyMonWasSelected` after the status screens: the enemy's picture loaded again, as the
    /// doll or minimized on whichever side `hWhoseTurn` names.
    fn reload_enemy_pic(&mut self) {
        let enemy = &self.b().enemy;
        let turn = self.whose_turn;
        if enemy.status2.contains(Status2::HAS_SUBSTITUTE_UP) {
            self.animate(Routine::Substitute, turn);
        } else if enemy.minimized != 0 {
            self.animate(Routine::MinimizeMon, turn);
        } else {
            self.push(Present::LoadFrontPic(enemy.mon.species));
        }
    }

    /// `LoadBattleMonFromParty` for `wPlayerMonNumber`.
    fn load_battle_mon_from_party(&mut self, ctx: &mut Ctx) {
        let slot = self.b().player_mon_number as usize;
        let named = ctx.world.party[slot].clone();
        self.player_nick = named.nick.clone();
        let badges = ctx.world.badges;
        let battle = self.battle_mut();
        battle.player.mon = BattleMon::from_party(&named.mon);
        battle.player.unmodified_level = named.mon.level;
        battle.player.unmodified_stats = named.mon.stats;
        apply_burn_and_paralysis_penalties(battle, Side::Player);
        apply_badge_stat_boosts(battle, badges);
        battle.player.stat_mods = [BASE_STAT_LEVEL; 6];
    }

    /// `SendOutMon` and `PrintSendOutMonMessage`.
    fn send_out_mon(&mut self, ctx: &mut Ctx) {
        let enemy = self.b().enemy.mon.clone();
        let label = if enemy.hp == 0 {
            "_GoText"
        } else {
            self.last_switch_in_enemy_hp = enemy.hp;
            let product = multiply(enemy.hp as u32, 25).to_be_bytes();
            let divisor = ((enemy.stats[0] >> 2) & 0xFF) as u8;
            let ratio = divide(product, divisor, 4).0[3];
            match ratio {
                70.. => "_GoText",
                40..=69 => "_DoItText",
                10..=39 => "_GetmText",
                _ => "_EnemysWeakText",
            }
        };
        let mut commands = far(label);
        commands.extend(far("_PlayerMon1Text"));
        self.push(Present::TextWith {
            commands,
            strings: vec![(TextBuffer::BattleMonNick, self.player_nick.clone())],
            numbers: vec![],
        });
        if self.b().enemy.mon.hp != 0 {
            self.push_hud(Side::Enemy);
        }
        self.push_hud(Side::Player);
        self.push(Present::Clear { x: 1, y: 5, width: 8, height: 7 });
        self.push(Present::LoadBackPic(self.b().player.mon.species));
        ctx.menu.battle_and_start = 0;
        let battle = self.battle_mut();
        battle.player.move_list_index = 0;
        battle.damage_multipliers = 0;
        battle.player.current_move.animation = 0;
        battle.player.used_move = 0;
        battle.enemy.used_move = 0;
        battle.player.status1 = Status1::empty();
        battle.player.status2 = Status2::empty();
        battle.player.status3 = Status3::empty();
        battle.player.disabled_move = 0;
        battle.player.disabled_move_number = 0;
        battle.player.minimized = 0;
        battle.enemy.status1.remove(Status1::USING_TRAPPING_MOVE);
        let species = battle.player.mon.species;
        // `wBoostExpByExpAll` is `wAnimationType`, zeroed just above.
        self.animation_type = animation_type::NONE;
        self.play_move_animation(anim::POOF_ANIM, Side::Enemy);
        let ball = 0x4C + match self.b().kind { BattleKind::Lost => 0xFF, BattleKind::Wild => 1, BattleKind::Trainer => 2 };
        self.push(Present::SendingOut { stage: 0, ball, at: (4, 11), base: 0 });
        self.push(Present::Cry(species as u8));
        self.empty_text();
        self.push(Present::SaveScreen1);
    }

    fn main_in_battle_loop(&mut self, ctx: &mut Ctx) {
        self.read_player_mon_cur_hp_and_status(ctx);
        if self.b().player.mon.hp == 0 {
            self.goto(Step::HandlePlayerMonFainted);
            return;
        }
        if self.b().enemy.mon.hp == 0 {
            self.goto(Step::HandleEnemyMonFainted);
            return;
        }
        self.push(Present::SaveScreen1);
        self.first_mons_not_out_yet = false;
        let battle = self.battle_mut();
        if battle.player.status2.intersects(Status2::NEEDS_TO_RECHARGE | Status2::USING_RAGE) {
            self.goto(Step::SelectEnemyMove);
            return;
        }
        battle.enemy.status1.remove(Status1::FLINCHED);
        battle.player.status1.remove(Status1::FLINCHED);
        if battle.player.status1.intersects(Status1::THRASHING_ABOUT | Status1::CHARGING_UP) {
            self.goto(Step::SelectEnemyMove);
            return;
        }
        self.ran = false;
        self.call(Step::DisplayBattleMenu, Step::AfterBattleMenu);
    }

    fn after_battle_menu(&mut self) {
        if self.ran || self.b().escaped_from_battle {
            self.goto(Step::BattleOver);
            return;
        }
        let battle = self.battle_mut();
        if battle.player.mon.status & (status::FRZ | status::SLP_MASK) != 0
            || battle.player.status1.intersects(Status1::STORING_ENERGY | Status1::USING_TRAPPING_MOVE) {
            self.goto(Step::SelectEnemyMove);
            return;
        }
        if battle.enemy.status1.contains(Status1::USING_TRAPPING_MOVE) {
            battle.player.selected_move = CANNOT_MOVE;
            self.goto(Step::SelectEnemyMove);
            return;
        }
        if self.action_taken {
            self.goto(Step::SelectEnemyMove);
            return;
        }
        self.move_to_swap = 0;
        self.goto(Step::MoveSelectionMenu);
    }

    fn read_player_mon_cur_hp_and_status(&mut self, ctx: &mut Ctx) {
        let battle = self.b();
        let mon = &battle.player.mon;
        if let Some(slot) = ctx.world.party.get_mut(battle.player_mon_number as usize) {
            slot.mon.mon.hp = mon.hp;
            slot.mon.mon.box_level = mon.party_pos;
            slot.mon.mon.status = mon.status;
        }
    }

    fn display_battle_menu(&mut self, ctx: &mut Ctx) {
        match self.battle_type {
            BattleType::Safari => return self.display_safari_battle_menu(ctx),
            BattleType::OldMan => return self.display_old_man_battle_menu(ctx),
            BattleType::Normal => {}
        }
        self.push(Present::LoadScreen1);
        self.push_hud(Side::Player);
        self.push_hud(Side::Enemy);
        self.empty_text();
        self.push(Present::SaveScreen1);
        self.push(Present::TextBox(BattleBox::BattleMenu));
        let saved = ctx.menu.battle_and_start;
        ctx.menu.last_item = saved;
        let right = saved >= 2;
        let current = if right { saved - 2 } else { saved };
        if right {
            ctx.menu.last_item = current;
        }
        self.open_battle_menu_column(right, current);
    }

    fn open_battle_menu_column(&mut self, right: bool, current: u8) {
        let (clear_x, _) = if right { (9, 15) } else { (15, 9) };
        self.push(Present::Clear { x: clear_x, y: 14, width: 1, height: 1 });
        self.push(Present::Clear { x: clear_x, y: 16, width: 1, height: 1 });
        self.menu = Some(Menu::battle(right, current));
        self.push(Present::Menu);
    }

    /// A menu of the battle's own has returned with `keys`.
    pub(super) fn menu_answered(&mut self, keys: Joypad, ctx: &mut Ctx) {
        match self.menu.clone().expect("a menu answered") {
            Menu::Battle { input, right } => {
                let column = (!right && keys.contains(Joypad::RIGHT)) || (right && keys.contains(Joypad::LEFT));
                let safari = self.battle_type == BattleType::Safari;
                if column && safari {
                    return self.open_safari_menu_column(!right, input.current, ctx);
                }
                if column {
                    self.open_battle_menu_column(!right, input.current);
                    return;
                }
                ctx.menu.unfilled_cursor(&mut ctx.screen.ui);
                let id = input.current + if right { 2 } else { 0 };
                ctx.menu.battle_and_start = id;
                if safari {
                    self.menu = None;
                    return self.safari_menu_chosen(id, ctx);
                }
                let chosen = match id {
                    1 => 2,
                    2 => 1,
                    id => id,
                };
                self.menu = None;
                self.goto(Step::BattleMenuChosen(chosen));
            }
            Menu::Moves { input } => self.move_menu_answered(keys, input.current, ctx),
            Menu::Mimic { input } => self.mimic_menu_answered(keys, input.current, ctx),
            Menu::SwitchStatsCancel { input, slot } => {
                self.menu = None;
                if keys.contains(Joypad::B) {
                    return self.party_mon_deselected();
                }
                ctx.menu.unfilled_cursor(&mut ctx.screen.ui);
                match input.current {
                    0 => self.switch_chosen(slot, ctx),
                    1 => {
                        let named = ctx.world.party[slot as usize].clone();
                        self.push(Present::ClearSprites);
                        self.push(Present::Push(Box::new(Mode::StatusScreen(crate::modes::status_screen::StatusScreen::new(named)))));
                        self.reload_enemy_pic();
                        self.goto(Step::PartyMenuFromBattle);
                    }
                    _ => {
                        self.push(Present::ClearSprites);
                        self.push(Present::LoadScreen2);
                        self.goto(Step::DisplayBattleMenu);
                    }
                }
            }
        }
    }

    fn battle_menu_chosen(&mut self, id: u8, ctx: &mut Ctx) {
        match id {
            0 => {
                self.num_run_attempts = 0;
                self.push(Present::LoadScreen1);
            }
            3 => {
                self.push(Present::LoadScreen1);
                let speed = self.b().player.mon.stats[3];
                let mut attempts = self.num_run_attempts;
                let ghost = self.is_ghost_battle(ctx);
                let run = try_running_from_battle(self.b(), speed, ghost, &mut attempts, ctx.rng);
                self.num_run_attempts = attempts;
                self.present_run(&run);
                self.action_taken = run.took_turn;
                if !run.escaped && !run.took_turn {
                    self.goto(Step::DisplayBattleMenu);
                }
            }
            1 => {
                self.push(Present::SaveScreen2);
                self.goto(Step::PartyMenuFromBattle);
            }
            _ => {
                self.push(Present::SaveScreen2);
                self.goto(Step::BagWasSelected);
            }
        }
    }

    /// `.partyMonDeselected`: the submenu blanked, and the party menu again.
    fn party_mon_deselected(&mut self) {
        self.push(Present::ClearRun { x: 11, y: 11, count: 6 * SCREEN_TILES_X + 9 });
        self.go_back_to_party_menu(PartyMenuType::Normal);
        self.goto(Step::PartyMonSelected);
    }

    /// `.switchMon`: not the mon out, and not one that has fainted.
    fn switch_chosen(&mut self, slot: u8, ctx: &mut Ctx) {
        if slot == self.b().player_mon_number {
            self.push(Present::TextWith {
                commands: far("_AlreadyOutText"),
                strings: vec![(TextBuffer::BattleMonNick, self.player_nick.clone())],
                numbers: vec![],
            });
            return self.party_mon_deselected();
        }
        if ctx.world.party[slot as usize].mon.mon.hp == 0 {
            return self.party_mon_deselected();
        }
        self.action_taken = true;
        self.push(Present::ClearSprites);
        self.push(Present::LoadScreen1);
        self.switch_player_mon(slot, ctx);
    }

    /// What `TryRunningFromBattle` shows, and the result an escape leaves.
    pub(super) fn present_run(&mut self, run: &crate::systems::battle::escape::Run) {
        if run.escaped {
            self.battle_result = result::DRAW;
            self.ran = true;
            self.push(Present::SoundAfterCurrent(sounds::SFX_RUN));
            self.battle_text(BattleText::GotAwayText, Side::Player);
            self.push(Present::WaitForSound);
            self.push(Present::SaveScreen1);
        } else {
            for &text in &run.texts {
                self.battle_text(text, Side::Player);
            }
            self.push(Present::SaveScreen1);
        }
    }

    /// `MoveSelectionMenu` with `wMoveMenuType` 0.
    fn move_selection_menu(&mut self, ctx: &mut Ctx) {
        let battle = self.battle_mut();
        battle.player.selected_move = STRUGGLE;
        let pp = battle.player.mon.pp;
        let left = match battle.player.disabled_move >> 4 {
            0 => pp.iter().fold(0, |any, &pp| any | pp) & PP_MASK,
            disabled => pp.iter().enumerate().filter(|&(slot, _)| slot + 1 != disabled as usize).fold(0, |any, (_, &pp)| any | pp),
        };
        if left == 0 {
            self.far_text("_NoMovesLeftText", Side::Player);
            self.push(Present::Frames(60));
            self.goto(Step::AfterMoveSelection { chosen: true });
            return;
        }
        let moves = format_moves_string(&battle.player.mon.moves);
        let count = moves.num_moves_minus_one.unwrap_or(0);
        let index = battle.player.move_list_index;
        self.num_moves_minus_one = count;
        let ui = &mut ctx.screen.ui;
        ui.text_box_border(4, 12, 14, 4);
        ui.set(4, 12, 0x7A);
        ui.set(10, 12, 0x7E);
        for (row, line) in moves.string.split(|&byte| byte == NEXT || byte == TERMINATOR).take(4).enumerate() {
            ui.place(6, 13 + row, line);
        }
        ctx.menu.last_item = index + 1;
        self.menu = Some(Menu::moves(index + 1, count + 2));
        self.goto(Step::SelectMenuItem);
    }

    /// `SelectMenuItem` and `PrintMenuItem`, which also leave the move under the cursor as the
    /// player's selected and current move.
    fn select_menu_item(&mut self, ctx: &mut Ctx) {
        if matches!(self.menu, Some(Menu::Mimic { .. })) {
            ctx.screen.ui.place(1, 14, &poke_core::charmap::encode("WHICH TECHNIQUE?").expect("encodes"));
            self.push(Present::Menu);
            return;
        }
        let current = self.menu.as_ref().expect("the move menu").current();
        let battle = self.battle.as_mut().expect("a battle");
        let ui = &mut ctx.screen.ui;
        ui.text_box_border(0, 8, 9, 3);
        let disabled = battle.player.disabled_move >> 4;
        if disabled != 0 && disabled == current {
            ui.place(1, 10, &poke_core::charmap::encode("disabled!").expect("encodes"));
        } else {
            let slot = current.wrapping_sub(1) as usize;
            let id = battle.player.mon.moves.get(slot).copied().flatten().map_or(0, |name| name as u8);
            battle.player.selected_move = id;
            let pp = battle.player.mon.pp.get(slot).copied().unwrap_or(0);
            let max = PokemonMoveName::from_repr(id).map_or(0, |name| max_pp(name, pp));
            ui.place(1, 9, &poke_core::charmap::encode("TYPE").expect("encodes"));
            ui.set(7, 11, 0xF3);
            ui.set(5, 9, 0xF3);
            let format = NumberFormat { digits: 2, left_align: false, leading_zeroes: false };
            print_number(ui, 5 + 11 * SCREEN_TILES_X, (pp & PP_MASK) as u32, format);
            print_number(ui, 8 + 11 * SCREEN_TILES_X, max as u32, format);
            if id != 0 {
                battle.player.current_move = MoveData::of(id);
                ctx.world.text.strings.insert(TextBuffer::StringBuffer, PokemonMoveName::from_repr(id).expect("a move").name());
            }
            ui.place(2, 10, &type_name(battle.player.current_move.move_type));
        }
        if self.move_to_swap != 0 {
            ui.set(5, 13 + self.move_to_swap as usize - 1, 0xEC);
        }
        self.push(Present::Menu);
    }

    /// `SelectMenuItem_CursorUp` and `SelectMenuItem_CursorDown`: the cursor wrapped past either
    /// end, and back to `SelectMenuItem`.
    fn move_cursor_moved(&mut self, keys: Joypad, current: u8, ctx: &mut Ctx) -> bool {
        let count = self.num_moves_minus_one;
        let wrapped = if keys.contains(Joypad::UP) {
            (current == 0).then_some(count + 1)
        } else if keys.contains(Joypad::DOWN) {
            (current == count + 2).then_some(1)
        } else {
            return false;
        };
        if let Some(wrapped) = wrapped {
            ctx.menu.erase_cursor(&mut ctx.screen.ui);
            self.menu.as_mut().expect("a menu").set_current(wrapped);
        }
        self.goto(Step::SelectMenuItem);
        true
    }

    fn move_menu_answered(&mut self, keys: Joypad, current: u8, ctx: &mut Ctx) {
        let count = self.num_moves_minus_one;
        if self.move_cursor_moved(keys, current, ctx) {
            return;
        }
        if keys.contains(Joypad::SELECT) {
            self.swap_moves_in_menu(current, ctx);
            return;
        }
        self.move_to_swap = 0;
        let slot = current.wrapping_sub(1);
        self.move_menu = MoveMenu { current: slot, max: count + 2 };
        self.battle_mut().player.move_list_index = slot;
        self.menu = None;
        if keys.contains(Joypad::B) {
            self.goto(Step::AfterMoveSelection { chosen: false });
            return;
        }
        let battle = self.b();
        let pp = battle.player.mon.pp[slot as usize] & PP_MASK;
        let disabled = (battle.player.disabled_move >> 4).wrapping_sub(1);
        if pp == 0 || disabled == slot {
            let label = if pp == 0 { "_MoveNoPPText" } else { "_MoveDisabledText" };
            self.far_text(label, Side::Player);
            self.push(Present::LoadScreen1);
            self.goto(Step::MoveSelectionMenu);
            return;
        }
        let id = battle.player.mon.moves[slot as usize].map_or(0, |name| name as u8);
        self.battle_mut().player.selected_move = id;
        self.goto(Step::AfterMoveSelection { chosen: true });
    }

    /// `MoveSelectionMenu`'s Mimic menu: the enemy's moves in a box of their own, the cursor on the
    /// first.
    fn mimic_move_selection_menu(&mut self, ctx: &mut Ctx) {
        let moves = format_moves_string(&self.b().enemy.mon.moves);
        let count = moves.num_moves_minus_one.unwrap_or(0);
        self.num_moves_minus_one = count;
        let ui = &mut ctx.screen.ui;
        ui.text_box_border(0, 7, 14, 4);
        for (row, line) in moves.string.split(|&byte| byte == NEXT || byte == TERMINATOR).take(4).enumerate() {
            ui.place(2, 8 + row, line);
        }
        ctx.menu.last_item = 1;
        self.menu = Some(Menu::mimic(count + 2));
        self.goto(Step::SelectMenuItem);
    }

    /// The rest of the player's `MimicEffect`: the screen put back, the move copied into the slot
    /// the fight menu left, its animation and text. `wCurrentMenuItem` is left on the enemy's move.
    fn mimic_menu_answered(&mut self, keys: Joypad, current: u8, ctx: &mut Ctx) {
        if self.move_cursor_moved(keys, current, ctx) {
            return;
        }
        self.move_to_swap = 0;
        self.menu = None;
        let chosen = current.wrapping_sub(1);
        self.push(Present::LoadScreen1);
        let cursor = self.move_menu.current;
        player_mimic_copy(self.battle_mut(), MimicMenu { cursor, chosen });
        let before = Before::of(self.b());
        self.effect_animation(Side::Player, effect::MIMIC_EFFECT, BattleText::MimicLearnedMoveText, &before);
        self.battle_text(BattleText::MimicLearnedMoveText, Side::Player);
        self.move_menu = MoveMenu { current: chosen, max: self.num_moves_minus_one + 2 };
    }

    /// `SwapMovesInMenu`: the first SELECT marks a move, the second swaps the two, in battle and in
    /// the party, and follows the disabled move.
    fn swap_moves_in_menu(&mut self, current: u8, ctx: &mut Ctx) {
        self.menu = None;
        if self.move_to_swap == 0 {
            self.move_to_swap = current;
            self.goto(Step::MoveSelectionMenu);
            return;
        }
        let (a, b) = (self.move_to_swap as usize - 1, current as usize - 1);
        let battle = self.battle.as_mut().expect("a battle");
        battle.player.mon.moves.swap(a, b);
        battle.player.mon.pp.swap(a, b);
        let disabled = &mut battle.player.disabled_move;
        let slot = *disabled >> 4;
        if slot == current {
            *disabled = *disabled & 0xF | self.move_to_swap << 4;
        } else if slot == self.move_to_swap {
            *disabled = *disabled & 0xF | current << 4;
        }
        let party = &mut ctx.world.party[battle.player_mon_number as usize].mon.mon;
        party.moves.swap(a, b);
        party.pp.swap(a, b);
        self.move_to_swap = 0;
        self.goto(Step::MoveSelectionMenu);
    }

    fn player_first(&mut self, stage: u8, ctx: &mut Ctx) {
        match stage {
            0 => self.call(Step::Execute(Side::Player), Step::PlayerFirst(1)),
            1 => {
                if self.b().escaped_from_battle {
                    return self.goto(Step::BattleOver);
                }
                if !self.target_standing {
                    return self.goto(Step::HandleEnemyMonFainted);
                }
                if self.residual(Side::Player) {
                    return self.goto(Step::HandlePlayerMonFainted);
                }
                self.draw_huds();
                if self.enemy_trainer_ai(true, ctx) {
                    return;
                }
                self.call(Step::Execute(Side::Enemy), Step::PlayerFirst(2));
            }
            2 => {
                if self.b().escaped_from_battle {
                    return self.goto(Step::BattleOver);
                }
                if !self.target_standing {
                    return self.goto(Step::HandlePlayerMonFainted);
                }
                self.goto(Step::PlayerFirst(3));
            }
            _ => {
                if self.residual(Side::Enemy) {
                    return self.goto(Step::HandleEnemyMonFainted);
                }
                self.draw_huds();
                check_num_attacks_left(self.battle_mut());
                self.goto(Step::MainInBattleLoop);
            }
        }
    }

    fn enemy_first(&mut self, stage: u8, ctx: &mut Ctx) {
        match stage {
            0 => {
                if self.enemy_trainer_ai(false, ctx) {
                    return;
                }
                self.call(Step::Execute(Side::Enemy), Step::EnemyFirst(1));
            }
            1 => {
                if self.b().escaped_from_battle {
                    return self.goto(Step::BattleOver);
                }
                if !self.target_standing {
                    return self.goto(Step::HandlePlayerMonFainted);
                }
                self.goto(Step::EnemyFirst(2));
            }
            2 => {
                if self.residual(Side::Enemy) {
                    return self.goto(Step::HandleEnemyMonFainted);
                }
                self.draw_huds();
                self.call(Step::Execute(Side::Player), Step::EnemyFirst(3));
            }
            _ => {
                if self.b().escaped_from_battle {
                    return self.goto(Step::BattleOver);
                }
                if !self.target_standing {
                    return self.goto(Step::HandleEnemyMonFainted);
                }
                if self.residual(Side::Player) {
                    return self.goto(Step::HandlePlayerMonFainted);
                }
                self.draw_huds();
                check_num_attacks_left(self.battle_mut());
                self.goto(Step::MainInBattleLoop);
            }
        }
    }

    fn draw_huds(&mut self) {
        self.push_hud(Side::Player);
        self.push_hud(Side::Enemy);
    }

    /// `TrainerAI` before the enemy's move; true where it acted instead, having gone on to where its
    /// caller goes next.
    fn enemy_trainer_ai(&mut self, player_first: bool, ctx: &mut Ctx) -> bool {
        let badges = ctx.world.badges;
        let before = Before::of(self.b());
        let (action, texts) = trainer_ai(self.battle_mut(), badges, ctx.rng);
        let next = if player_first { Step::PlayerFirst(3) } else { Step::EnemyFirst(2) };
        match action {
            None => return false,
            Some(AiAction::Switch) => {
                // `SwitchEnemyMon`: the withdrawal, and a send-out with no shift prompt.
                self.push(Present::TextWith {
                    commands: far("_AIBattleWithdrawText"),
                    strings: vec![(TextBuffer::TrainerName, self.trainer_name.clone()),
                                  (TextBuffer::EnemyMonNick, self.enemy_nick.clone())],
                    numbers: vec![],
                });
                self.first_mons_not_out_yet = true;
                self.call(Step::EnemySendOut { first: false }, Step::AiSwitched { player_first });
                return true;
            }
            Some(AiAction::UseItem(item)) => {
                if matches!(item, ItemId::FullHeal | ItemId::GuardSpec) {
                    self.push(Present::SoundAfterCurrent(sounds::SFX_HEAL_AILMENT));
                }
                self.push(Present::TextWith {
                    commands: far("_AIBattleUseItemText"),
                    strings: vec![(TextBuffer::TrainerName, self.trainer_name.clone()),
                                  (TextBuffer::NameBuffer, poke_core::item::name(item)),
                                  (TextBuffer::EnemyMonNick, self.enemy_nick.clone())],
                    numbers: vec![],
                });
                let stat_effect = match item {
                    ItemId::XAttack => effect::ATTACK_UP1_EFFECT,
                    ItemId::XDefend => effect::DEFENSE_UP1_EFFECT,
                    ItemId::XSpeed => effect::SPEED_UP1_EFFECT,
                    _ => effect::SPECIAL_UP1_EFFECT,
                };
                for text in texts {
                    if text == BattleText::MonsStatsRoseText {
                        self.play_battle_animation(anim::XSTATITEM_DUPLICATE_ANIM, Side::Enemy);
                    }
                    self.battle_text_for(text, Side::Enemy, stat_effect);
                }
                self.hp_bars(before);
            }
        }
        self.goto(next);
        true
    }

    /// `HandlePoisonBurnLeechSeed` for `side`, and whether the mon fainted.
    fn residual(&mut self, side: Side) -> bool {
        let battle = self.battle.as_mut().expect("a battle");
        let me = battle.side(side);
        if me.mon.status & (status::BRN | status::PSN) != 0 {
            let label = if me.mon.status & status::BRN != 0 { "_HurtByBurnText" } else { "_HurtByPoisonText" };
            let old = me.mon.hp;
            decrease_own_hp(battle, side);
            let new = battle.side(side).mon.hp;
            self.far_text(label, side);
            self.play_battle_animation(anim::BURN_PSN_ANIM, side);
            self.push(Present::HpBar { side, old, new });
        }
        let battle = self.battle.as_mut().expect("a battle");
        if battle.side(side).status2.contains(Status2::SEEDED) {
            let (old, other_old) = (battle.side(side).mon.hp, battle.side(side.other()).mon.hp);
            let drained = decrease_own_hp(battle, side);
            let other = &mut battle.side_mut(side.other()).mon;
            other.hp = other.hp.wrapping_add(drained);
            if other.hp >= other.stats[0] {
                other.hp = other.stats[0];
            }
            let (new, other_new) = (battle.side(side).mon.hp, battle.side(side.other()).mon.hp);
            self.play_battle_animation(anim::ABSORB, side.other());
            self.whose_turn = side;
            self.push(Present::HpBar { side, old, new });
            self.push(Present::HpBar { side: side.other(), old: other_old, new: other_new });
            self.far_text("_HurtByLeechSeedText", side);
        }
        if self.b().side(side).mon.hp == 0 {
            self.draw_huds();
            self.push(Present::Frames(20));
            return true;
        }
        false
    }

    /// `GetCurrentMove`.
    fn get_current_move(&mut self, side: Side, ctx: &mut Ctx) {
        let id = self.b().side(side).selected_move;
        self.battle_mut().side_mut(side).current_move = MoveData::of(id);
        ctx.world.text.strings.insert(TextBuffer::StringBuffer, PokemonMoveName::from_repr(id).map_or(vec![], |name| name.name()));
    }

    fn execute(&mut self, side: Side, ctx: &mut Ctx) {
        let battle = self.battle.as_mut().expect("a battle");
        if battle.side(side).selected_move == CANNOT_MOVE {
            return self.goto(Step::ExecuteDone(side));
        }
        match side {
            Side::Player => {
                battle.move_missed = false;
                battle.mon_is_disobedient = false;
                battle.move_didnt_miss = false;
                battle.damage_multipliers = EFFECTIVE;
                if self.action_taken {
                    return self.goto(Step::ExecuteDone(side));
                }
                if self.print_ghost_text(side, ctx) {
                    return self.goto(Step::ExecuteDone(side));
                }
            }
            Side::Enemy => {
                if self.print_ghost_text(side, ctx) {
                    return self.goto(Step::ExecuteDone(side));
                }
                let battle = self.battle.as_mut().expect("a battle");
                battle.ai_layer2_encouragement = battle.ai_layer2_encouragement.wrapping_add(1);
                battle.move_missed = false;
                battle.move_didnt_miss = false;
                battle.damage_multipliers = EFFECTIVE;
            }
        }
        let before = Before::of(self.b());
        let party = Self::party(ctx);
        let (continuation, texts) = check_status_conditions(self.battle_mut(), &party, side, ctx.rng);
        self.present_status_conditions(side, &texts, before);
        if continuation == Continuation::CanExecuteMove {
            // Rage skips `GetCurrentMove`, so it names itself: the buffer otherwise still holds the
            // move the other side just used.
            ctx.world.text.strings.insert(TextBuffer::StringBuffer, PokemonMoveName::Rage.name());
        }
        self.goto(match continuation {
            Continuation::Move => Step::HasNoSpecialConditions(side),
            Continuation::MoveDone => Step::ExecuteDone(side),
            Continuation::HandleIfMoveMissed => Step::HandleIfMoveMissed(side),
            Continuation::CalcMoveDamage => Step::CalcMoveDamage(side),
            Continuation::GetAnimationType => Step::GetAnimationType(side),
            Continuation::CanExecuteMove => Step::CanExecuteMove(side),
        });
    }

    /// `PrintGhostText`: in a ghost battle the player's mon is too scared to move unless it is asleep
    /// or frozen, and the ghost only tells it to get out. True where the move is over.
    fn print_ghost_text(&mut self, side: Side, ctx: &Ctx) -> bool {
        if !self.is_ghost_battle(ctx) {
            return false;
        }
        match side {
            Side::Player if self.b().player.mon.status & (status::FRZ | status::SLP_MASK) != 0 => false,
            Side::Player => {
                self.push(Present::TextWith {
                    commands: far("_ScaredText"),
                    strings: vec![(TextBuffer::BattleMonNick, self.player_nick.clone())],
                    numbers: vec![],
                });
                true
            }
            Side::Enemy => {
                self.far_text("_GetOutText", side);
                true
            }
        }
    }

    /// `IsGhostBattle`: a wild battle in the Pokémon Tower without the SILPH SCOPE.
    pub(super) fn is_ghost_battle(&self, ctx: &Ctx) -> bool {
        use poke_core::map::Map;
        let tower = (Map::PokemonTower1F as u8..=Map::PokemonTower7F as u8).contains(&(ctx.world.location.map as u8));
        self.b().kind == BattleKind::Wild && tower && ctx.world.bag.quantity_of(ItemId::SilphScope) == 0
    }

    /// What `CheckPlayerStatusConditions` shows along the way.
    fn present_status_conditions(&mut self, side: Side, texts: &[BattleText], before: Before) {
        for &text in texts {
            match (text, side) {
                (BattleText::FastAsleepText, Side::Player) => {
                    self.play_battle_animation(anim::SLP_PLAYER_ANIM, side);
                    self.battle_text(text, side);
                }
                (BattleText::FastAsleepText, Side::Enemy) => {
                    self.battle_text(text, side);
                    self.play_battle_animation(anim::SLP_ANIM, side);
                }
                (BattleText::IsConfusedText, _) => {
                    self.battle_text(text, side);
                    let id = match side { Side::Player => anim::CONF_PLAYER_ANIM, Side::Enemy => anim::CONF_ANIM };
                    self.play_battle_animation(id, side);
                }
                (BattleText::HurtItselfText, _) => {
                    self.battle_text(text, side);
                    self.hurt_itself_animation(side);
                    if side == Side::Player {
                        self.push_hud(Side::Player);
                    }
                    self.hp_bars(before);
                }
                (text, _) => self.battle_text(text, side),
            }
        }
        let stopped = texts.iter().any(|text| matches!(text, BattleText::FullyParalyzedText | BattleText::HurtItselfText));
        if stopped && matches!(self.b().side(side).current_move.effect, effect::FLY_EFFECT | effect::CHARGE_EFFECT) {
            self.play_battle_animation(anim::STATUS_AFFECTED_ANIM, side);
        }
    }

    /// `HandleSelfConfusionDamage`'s Pound, played as the other side's.
    fn hurt_itself_animation(&mut self, side: Side) {
        self.play_battle_animation(anim::POUND, side.other());
        self.whose_turn = side;
    }

    /// An `UpdateHPBar2` for each side whose HP has moved since `before`.
    fn hp_bars(&mut self, before: Before) {
        let after = Before::of(self.b());
        for (side, old, new) in [(Side::Player, before.hp[0], after.hp[0]), (Side::Enemy, before.hp[1], after.hp[1])] {
            if old != new {
                self.push(Present::HpBar { side, old, new });
            }
        }
    }

    fn has_no_special_conditions(&mut self, side: Side, ctx: &mut Ctx) {
        self.get_current_move(side, ctx);
        if self.b().side(side).status1.contains(Status1::CHARGING_UP) {
            let battle = self.battle_mut();
            battle.side_mut(side).status1.remove(Status1::CHARGING_UP | Status1::INVULNERABLE);
            return self.goto(Step::CanExecuteMove(side));
        }
        if side == Side::Player {
            let before = Before::of(self.b());
            let (player_id, badges) = (ctx.world.player_id, ctx.world.badges);
            let party = Self::party(ctx);
            let mut menu = self.move_menu;
            let obedience = check_for_disobedience(self.battle_mut(), &party, player_id, badges, &mut menu, ctx.rng);
            self.move_menu = menu;
            if !obedience.obeys {
                for &text in &obedience.texts {
                    self.battle_text(text, side);
                    if text == BattleText::HurtItselfText {
                        self.hurt_itself_animation(side);
                        self.push_hud(Side::Player);
                        self.hp_bars(before);
                    }
                }
                return self.goto(Step::ExecuteDone(side));
            }
        }
        self.goto(Step::CheckIfNeedsToChargeUp(side));
    }

    fn can_execute_move(&mut self, side: Side, ctx: &mut Ctx) {
        if side == Side::Enemy {
            self.battle_mut().mon_is_disobedient = false;
        }
        // `DisplayUsedMoveText`.
        let battle = self.battle_mut();
        let num = battle.side(side).current_move.animation;
        battle.side_mut(side).used_move = num;
        let disobedient = battle.mon_is_disobedient;
        let mut labels = vec!["_ActorNameText", "_UsedMove1Text"];
        if disobedient {
            labels.push("_UsedInsteadText");
        }
        labels.extend(["_MoveNameText", "_EndUsedMove1Text"]);
        let name = ctx.world.text.string(TextBuffer::StringBuffer);
        let commands = spliced(chain(&labels), side, &self.player_nick, &self.enemy_nick);
        self.push(Present::TextWith { commands, strings: vec![(TextBuffer::StringBuffer, name)], numbers: vec![] });
        if side == Side::Player {
            self.with_party(ctx, |battle, party, _| decrement_pp(battle, party));
        }
        let move_effect = self.b().side(side).current_move.effect;
        if RESIDUAL_EFFECTS_1.contains(&move_effect) {
            return self.jump_move_effect(side, ctx);
        }
        if SPECIAL_EFFECTS_CONT.contains(&move_effect) {
            self.call_jump_move_effect(side, ctx);
        }
        self.goto(Step::CalcMoveDamage(side));
    }

    /// `jp JumpMoveEffect`: the effect, and then the move is over.
    fn jump_move_effect(&mut self, side: Side, ctx: &mut Ctx) {
        // The player's Mimic waits on a menu between its hit test and the copy.
        if side == Side::Player && self.b().player.current_move.effect == effect::MIMIC_EFFECT {
            let before = Before::of(self.b());
            if !mimic_lands(self.battle_mut(), side, ctx.rng) {
                let texts = [BattleText::ButItFailedText];
                self.present_effect(side, effect::MIMIC_EFFECT, &texts, before, ctx.world.options.battle_animation);
                return self.goto(Step::ExecuteDone(side));
            }
            self.push(Present::Frames(50));
            return self.call(Step::MimicMoveSelectionMenu, Step::ExecuteDone(side));
        }
        self.call_jump_move_effect(side, ctx);
        self.goto(Step::ExecuteDone(side));
    }

    /// `call JumpMoveEffect`: the effect, with what it shows.
    fn call_jump_move_effect(&mut self, side: Side, ctx: &mut Ctx) {
        let before = Before::of(self.b());
        let move_effect = self.b().side(side).current_move.effect;
        let badges = ctx.world.badges;
        // Only the enemy's Mimic comes through here, and it reads no menu.
        let menu = MimicMenu { cursor: 0, chosen: 0 };
        let texts = self.with_party(ctx, |battle, party, ctx| jump_move_effect(battle, party, side, badges, menu, ctx.rng));
        self.present_effect(side, move_effect, &texts, before, ctx.world.options.battle_animation);
    }

    /// What a move effect shows around its texts: the waits before a failure, the animations, the HP
    /// bars, the HUDs.
    fn present_effect(&mut self, side: Side, move_effect: u8, texts: &[BattleText], before: Before, animations_on: bool) {
        use BattleText::*;
        let delay_before_failure = matches!(move_effect,
            effect::POISON_EFFECT | effect::PARALYZE_EFFECT | effect::CONFUSION_EFFECT | effect::SWITCH_AND_TELEPORT_EFFECT
            | effect::FOCUS_ENERGY_EFFECT | effect::LEECH_SEED_EFFECT | effect::REFLECT_EFFECT | effect::LIGHT_SCREEN_EFFECT
            | effect::HEAL_EFFECT);
        // `FreezeBurnParalyzeEffect` zeroes `wAnimationType` before it tests anything.
        if matches!(move_effect, effect::BURN_SIDE_EFFECT1 | effect::FREEZE_SIDE_EFFECT1 | effect::PARALYZE_SIDE_EFFECT1
            | effect::BURN_SIDE_EFFECT2 | effect::FREEZE_SIDE_EFFECT2 | effect::PARALYZE_SIDE_EFFECT2) {
            self.animation_type = animation_type::NONE;
        }
        if matches!(move_effect, effect::SUBSTITUTE_EFFECT | effect::MIMIC_EFFECT) {
            self.push(Present::Frames(50));
        }
        for &text in texts {
            match text {
                DidntAffectText | DoesntAffectMonText | ButItFailedText | IsUnaffectedText | EvadedAttackText
                    if delay_before_failure => {
                    self.push(Present::Frames(50));
                    self.battle_text(text, side);
                }
                ParalyzedMayNotAttackText if move_effect == effect::PARALYZE_EFFECT => {
                    self.push(Present::Frames(30));
                    self.play_current_move_animation(side);
                    self.battle_text(text, side);
                }
                SuckedHealthText | DreamWasEatenText => {
                    self.hp_bars(before);
                    self.draw_huds();
                    self.battle_text(text, side);
                }
                HitWithRecoilText => {
                    self.hp_bars(before);
                    self.battle_text(text, side);
                }
                RegainedHealthText => {
                    self.play_current_move_animation(side);
                    self.hp_bars(before);
                    self.draw_huds();
                    self.battle_text(text, side);
                }
                StartedSleepingEffect | FellAsleepBecameHealthyText => {
                    self.push(Present::Frames(50));
                    self.battle_text(text, side);
                }
                SubstituteText => {
                    if animations_on {
                        self.animation_type = animation_type::NONE;
                    }
                    let id = self.b().side(side).current_move.animation;
                    self.animate(Routine::SubstituteEffect { id }, side);
                    self.battle_text(text, side);
                    self.draw_huds();
                }
                RanFromBattleText | RanAwayScaredText | WasBlownAwayText => {
                    let id = self.b().side(side).current_move.animation;
                    self.play_battle_animation(id, side);
                    self.push(Present::Frames(20));
                    self.battle_text(text, side);
                }
                TransformedText => {
                    let type_before = self.animation_type;
                    self.effect_animation(side, move_effect, text, &before);
                    if !animations_on {
                        self.animation_type = type_before;
                    }
                    self.battle_text(text, side);
                }
                text => {
                    self.effect_animation(side, move_effect, text, &before);
                    self.battle_text(text, side);
                }
            }
        }
        self.silent_effect_animation(side, move_effect);
    }

    fn mirror_move_check(&mut self, side: Side, ctx: &mut Ctx) {
        let move_effect = self.b().side(side).current_move.effect;
        if move_effect == effect::MIRROR_MOVE_EFFECT {
            let (copied, texts) = self.with_party(ctx, |battle, party, _| mirror_move_copy_move(battle, party, side));
            for text in texts {
                self.battle_text(text, side);
            }
            if !copied {
                return self.goto(Step::ExecuteDone(side));
            }
            if side == Side::Player {
                self.battle_mut().mon_is_disobedient = false;
            }
            self.set_move_name(side, ctx);
            return self.goto(Step::CheckIfNeedsToChargeUp(side));
        }
        if move_effect == effect::METRONOME_EFFECT {
            self.play_battle_animation(anim::METRONOME, side);
            self.with_party(ctx, |battle, party, ctx| metronome_pick_move(battle, party, side, ctx.rng));
            self.set_move_name(side, ctx);
            return self.goto(Step::CheckIfNeedsToChargeUp(side));
        }
        if RESIDUAL_EFFECTS_2.contains(&move_effect) {
            return self.jump_move_effect(side, ctx);
        }
        if self.b().move_missed {
            self.print_move_failure_text(side);
            if move_effect != effect::EXPLODE_EFFECT {
                return self.goto(Step::ExecuteDone(side));
            }
            return self.goto(Step::NotDone(side));
        }
        let before = Before::of(self.b());
        let texts = apply_attack_to_pokemon(self.battle_mut(), side, ctx.rng);
        for &text in &texts {
            self.battle_text(text, side);
            if text == BattleText::SubstituteBrokeText {
                let minimized = self.b().side(side.other()).minimized != 0;
                self.animate(Routine::HideSubstituteShowMon { substitute_up: false, minimized }, side.other());
                self.whose_turn = side;
            }
        }
        if texts.is_empty() {
            self.hp_bars(before);
        }
        self.draw_huds();
        // `PrintCriticalOHKOText`.
        match self.b().critical_hit_or_ohko {
            CriticalHitOrOhko::CriticalHit => self.far_text("_CriticalHitText", side),
            CriticalHitOrOhko::SuccessfulOhko => self.far_text("_OHKOText", side),
            _ => {}
        }
        if matches!(self.b().critical_hit_or_ohko, CriticalHitOrOhko::CriticalHit | CriticalHitOrOhko::SuccessfulOhko) {
            self.battle_mut().critical_hit_or_ohko = CriticalHitOrOhko::Normal;
        }
        self.push(Present::Frames(20));
        // `DisplayEffectiveness`.
        let effectiveness = self.b().damage_multipliers & 0x7F;
        if effectiveness > EFFECTIVE {
            self.far_text("_SuperEffectiveText", side);
        } else if effectiveness < EFFECTIVE {
            self.far_text("_NotVeryEffectiveText", side);
        }
        self.battle_mut().move_didnt_miss = true;
        self.goto(Step::NotDone(side));
    }

    fn set_move_name(&mut self, side: Side, ctx: &mut Ctx) {
        let id = self.b().side(side).selected_move;
        ctx.world.text.strings.insert(TextBuffer::StringBuffer, PokemonMoveName::from_repr(id).map_or(vec![], |name| name.name()));
    }

    /// `PrintMoveFailureText`, and Jump Kick's crash.
    fn print_move_failure_text(&mut self, side: Side) {
        let battle = self.b();
        let label = if battle.damage_multipliers & 0x7F == 0 {
            "_DoesntAffectMonText"
        } else if battle.critical_hit_or_ohko == CriticalHitOrOhko::FailedOhko {
            "_UnaffectedText"
        } else {
            "_AttackMissedText"
        };
        self.far_text(label, side);
        self.battle_mut().critical_hit_or_ohko = CriticalHitOrOhko::Normal;
        if self.b().side(side).current_move.effect != effect::JUMP_KICK_EFFECT {
            return;
        }
        let battle = self.battle_mut();
        battle.damage = (battle.damage >> 3).max(1);
        let before = Before::of(self.b());
        self.far_text("_KeptGoingAndCrashedText", side);
        self.animate(Routine::ShakeScreenHorizontally(4), side);
        let texts = apply_damage_to_pokemon(self.battle_mut(), side, side);
        for text in texts {
            self.battle_text(text, side);
        }
        self.hp_bars(before);
        self.draw_huds();
    }

    fn not_done(&mut self, side: Side, ctx: &mut Ctx) {
        let move_effect = self.b().side(side).current_move.effect;
        if ALWAYS_HAPPEN_SIDE_EFFECTS.contains(&move_effect) {
            self.call_jump_move_effect(side, ctx);
        }
        if self.b().side(side.other()).mon.hp == 0 {
            self.target_standing = false;
            return;
        }
        let badges = ctx.world.badges;
        let texts = handle_building_rage(self.battle_mut(), side, badges);
        for text in texts {
            self.battle_text_for(text, side.other(), effect::ATTACK_UP1_EFFECT);
        }
        let battle = self.battle_mut();
        if battle.side(side).status1.contains(Status1::ATTACKING_MULTIPLE_TIMES) {
            let me = battle.side_mut(side);
            me.num_attacks_left = me.num_attacks_left.wrapping_sub(1);
            if me.num_attacks_left != 0 {
                return self.goto(Step::GetAnimationType(side));
            }
            me.status1.remove(Status1::ATTACKING_MULTIPLE_TIMES);
            let hits = me.num_hits();
            let (label, number) = match side {
                Side::Player => ("_MultiHitText", TextNumber::PlayerNumHits),
                Side::Enemy => ("_HitXTimesText", TextNumber::EnemyNumHits),
            };
            let commands = spliced(far(label), side, &self.player_nick, &self.enemy_nick);
            self.push(Present::TextWith { commands, strings: vec![], numbers: vec![(number, hits as u32)] });
            self.battle_mut().side_mut(side).set_num_hits(0);
        }
        self.goto(Step::ExecuteOtherEffects(side));
    }

    /// `FaintEnemyPokemon`, up to the experience, with `AfterFaintEnemy` carrying on.
    fn call_faint_enemy_pokemon(&mut self, ctx: &mut Ctx) {
        self.read_player_mon_cur_hp_and_status(ctx);
        let battle = self.battle_mut();
        if battle.kind != BattleKind::Wild {
            let pos = battle.enemy.mon.party_pos as usize;
            if let Some(mon) = battle.enemy_party.get_mut(pos) {
                mon.mon.hp = 0;
            }
        }
        battle.player.status1.remove(Status1::ATTACKING_MULTIPLE_TIMES);
        battle.player.bide_accumulated_damage &= 0xFF;
        battle.enemy.status1 = Status1::empty();
        battle.enemy.status2 = Status2::empty();
        battle.enemy.status3 = Status3::empty();
        battle.enemy.disabled_move = 0;
        battle.enemy.disabled_move_number = 0;
        battle.enemy.minimized = 0;
        battle.player.used_move = 0;
        battle.enemy.used_move = 0;
        self.push(Present::SlideDown { x: 12, y: 0, row: 0 });
        self.push(Present::Clear { x: 0, y: 0, width: 11, height: 4 });
        if self.b().kind == BattleKind::Wild {
            // `EndLowHealthAlarm` and `PlayBattleVictoryMusic`.
            self.low_health_alarm_disabled = true;
            self.push(Present::EndLowHealthAlarm);
            self.push(Present::SoundAfterCurrent(SoundId::STOP_ALL_MUSIC));
            self.push(Present::Music(sounds::MUSIC_DEFEATED_WILD_MON));
        } else {
            self.push(Present::Modifiers { frequency: 0, tempo: 0 });
            self.push(Present::SoundAfterCurrent(sounds::SFX_FAINT_FALL));
            self.push(Present::WaitForSound);
            self.push(Present::Sound(sounds::SFX_FAINT_THUD));
            self.push(Present::WaitForSound);
        }
        if self.b().player.mon.hp == 0 && !self.in_handle_player_mon_fainted {
            self.remove_fainted_player_mon(ctx);
        }
        if !ctx.world.party.iter().any(|mon| mon.mon.mon.hp != 0) {
            return;
        }
        self.push(Present::TextWith {
            commands: far("_EnemyMonFaintedText"),
            strings: vec![(TextBuffer::EnemyMonNick, self.enemy_nick.clone())],
            numbers: vec![],
        });
        self.empty_text();
        self.push(Present::SaveScreen1);
        self.battle_result = result::WON;
        let has_exp_all = ctx.world.bag.quantity_of(ItemId::ExpAll) > 0;
        if has_exp_all {
            let exp = &mut self.battle_mut().enemy_exp;
            for value in exp.base_stats.iter_mut().chain([&mut exp.catch_rate, &mut exp.base_exp]) {
                *value >>= 1;
            }
        }
        self.experience(ctx, false);
        if has_exp_all {
            let count = ctx.world.party.len();
            self.battle_mut().gain_exp_flags = ((1u16 << count) - 1) as u8;
            self.experience(ctx, true);
        }
    }

    /// `GainExperience` and what it prints.
    fn experience(&mut self, ctx: &mut Ctx, exp_all: bool) {
        let (player_id, badges) = (ctx.world.player_id, ctx.world.badges);
        let events = self.with_party(ctx, |battle, party, _| gain_experience(battle, party, player_id, badges));
        for event in events {
            match event {
                ExpEvent::Gained { slot, amount, boosted } => {
                    let nick = ctx.world.party[slot as usize].nick.clone();
                    let mut labels = vec!["_GainedText"];
                    labels.extend(if exp_all { vec!["_WithExpAllText"] } else if boosted { vec!["_BoostedText"] } else { vec![] });
                    labels.push("_ExpPointsText");
                    self.push(Present::TextWith {
                        commands: chain(&labels),
                        strings: vec![(TextBuffer::NameBuffer, nick)],
                        numbers: vec![(TextNumber::ExpAmountGained, amount as u32)],
                    });
                    self.cur_party_species = ctx.world.party[slot as usize].mon.mon.species as u8;
                }
                ExpEvent::GrewLevel { slot, level } => {
                    if slot == self.b().player_mon_number {
                        self.push_hud(Side::Player);
                        self.empty_text();
                        self.push(Present::SaveScreen1);
                    }
                    let nick = ctx.world.party[slot as usize].nick.clone();
                    self.push(Present::TextWith {
                        commands: [far("_GrewLevelText"), vec![TextCommand::Sound(TextSound::GetItem1)]].concat(),
                        strings: vec![(TextBuffer::NameBuffer, nick)],
                        numbers: vec![(TextNumber::CurEnemyLevel, level as u32)],
                    });
                    self.push(Present::StatsBox(slot));
                    self.push(Present::WaitButton);
                    self.push(Present::LoadScreen1);
                    if let Some(learn) = LearnMove::from_level_up(ctx.world, slot, level) {
                        self.push(Present::Push(Box::new(Mode::LearnMove(learn))));
                        self.push(Present::SyncLearnedMove(slot));
                    }
                    self.cur_party_species = ctx.world.party[slot as usize].mon.mon.species as u8;
                }
            }
        }
    }

    fn after_faint_enemy(&mut self, ctx: &mut Ctx) {
        if !ctx.world.party.iter().any(|mon| mon.mon.mon.hp != 0) {
            return self.handle_player_black_out();
        }
        if self.b().player.mon.hp != 0 {
            self.push_hud(Side::Player);
        }
        if self.b().kind == BattleKind::Wild {
            return self.goto(Step::BattleOver);
        }
        if !self.any_enemy_alive() {
            return self.trainer_battle_victory(ctx);
        }
        if self.b().player.mon.hp == 0 {
            // `DoUseNextMonDialogue`, which a trainer battle leaves at once.
            self.empty_text();
            self.push(Present::SaveScreen1);
            return self.goto(Step::ChooseNextMon);
        }
        self.goto(Step::ReplaceFaintedEnemyMon);
    }

    /// `AnyEnemyPokemonAliveCheck`.
    fn any_enemy_alive(&self) -> bool {
        self.b().enemy_party.iter().any(|mon| mon.mon.hp != 0)
    }

    /// `TrainerBattleVictory`: the music, the trainer back on screen, and the prize.
    fn trainer_battle_victory(&mut self, ctx: &mut Ctx) {
        self.low_health_alarm_disabled = true;
        self.push(Present::EndLowHealthAlarm);
        let Some(Opponent::Trainer { class, lone_attack, .. }) = Some(self.opponent.clone()) else { unreachable!() };
        let music = if lone_attack != 0 || class == RIVAL3 { sounds::MUSIC_DEFEATED_GYM_LEADER } else { sounds::MUSIC_DEFEATED_TRAINER };
        self.push(Present::SoundAfterCurrent(SoundId::STOP_ALL_MUSIC));
        self.push(Present::Music(music));
        self.push(Present::TextWith {
            commands: far("_TrainerDefeatedText"),
            strings: vec![(TextBuffer::TrainerName, self.trainer_name.clone())],
            numbers: vec![],
        });
        self.push(Present::LoadTrainerPic(class));
        self.push(Present::ScrollTrainerIn { columns: 1 });
        self.push(Present::Frames(40));
        if let Some(words) = self.end_battle_text.clone() {
            let mut commands = far("_TrainerNameText");
            commands.extend(words);
            self.push(Present::TextWith { commands, strings: vec![(TextBuffer::NameBuffer, self.trainer_name.clone())], numbers: vec![] });
            self.push(Present::WaitForSound);
        }
        ctx.world.text.money.insert(TextMoney::AmountMoneyWon, self.prize_money.to_vec());
        self.push(Present::Text(far("_MoneyForWinningText")));
        add_bcd(&mut ctx.world.money, &self.prize_money);
        self.goto(Step::BattleOver);
    }

    /// `RemoveFaintedPlayerMon`.
    fn remove_fainted_player_mon(&mut self, ctx: &mut Ctx) {
        let battle = self.battle_mut();
        battle.gain_exp_flags &= !(1 << battle.player_mon_number);
        battle.enemy.status1.remove(Status1::ATTACKING_MULTIPLE_TIMES);
        self.push(Present::DisableLowHealthAlarm);
        let battle = self.battle_mut();
        battle.enemy.bide_accumulated_damage = 0;
        battle.player.mon.status = 0;
        self.read_player_mon_cur_hp_and_status(ctx);
        self.push(Present::Clear { x: 9, y: 7, width: 11, height: 5 });
        self.push(Present::SlideDown { x: 1, y: 5, row: 0 });
        self.battle_result = result::LOST;
        if !self.in_handle_player_mon_fainted {
            return;
        }
        let species = self.b().player.mon.species;
        self.push(Present::Cry(species as u8));
        self.push(Present::TextWith {
            commands: far("_PlayerMonFaintedText"),
            strings: vec![(TextBuffer::BattleMonNick, self.player_nick.clone())],
            numbers: vec![],
        });
    }

    fn after_remove_fainted_player_mon(&mut self, ctx: &mut Ctx) {
        if !ctx.world.party.iter().any(|mon| mon.mon.mon.hp != 0) {
            return self.handle_player_black_out();
        }
        if self.b().enemy.mon.hp == 0 {
            self.call_faint_enemy_pokemon(ctx);
            if self.b().kind == BattleKind::Wild {
                return self.goto(Step::BattleOver);
            }
            if !self.any_enemy_alive() {
                return self.trainer_battle_victory(ctx);
            }
        }
        // `DoUseNextMonDialogue`.
        self.empty_text();
        self.push(Present::SaveScreen1);
        if self.b().kind != BattleKind::Wild {
            return self.goto(Step::ChooseNextMon);
        }
        self.far_text("_UseNextMonText", Side::Player);
        self.push(Present::Push(Box::new(Mode::TwoOptionMenu(TwoOptionMenu::new(TwoOptionMenuId::YesNo, (13, 9), false)))));
        self.goto(Step::UseNextMonAnswered);
    }

    fn next_mon_chosen(&mut self, ctx: &mut Ctx) {
        let Some(Outcome::Chosen(slot)) = self.outcome.take() else {
            // B goes back to the list: `GoBackToPartyMenu`.
            self.go_back_to_party_menu(PartyMenuType::Battle);
            return self.goto(Step::NextMonChosen);
        };
        if ctx.world.party[slot as usize].mon.mon.hp == 0 {
            self.go_back_to_party_menu(PartyMenuType::Battle);
            return self.goto(Step::NextMonChosen);
        }
        self.push(Present::ClearSprites);
        self.action_taken = false;
        let battle = self.battle_mut();
        battle.player_mon_number = slot;
        battle.gain_exp_flags |= 1 << slot;
        battle.fought_current_enemy_flags |= 1 << slot;
        self.load_battle_mon_from_party(ctx);
        hud::load_hud_tiles(&mut ctx.screen.tiles);
        self.push(Present::LoadScreen1);
        self.call(Step::SendOutMon, Step::AfterChooseNextMon);
    }

    /// `HandlePlayerBlackOut`, outside the first rival battle.
    fn handle_player_black_out(&mut self) {
        self.far_text("_PlayerBlackedOutText2", Side::Player);
        self.push(Present::Clear { x: 0, y: 0, width: SCREEN_TILES_X, height: SCREEN_TILES_Y });
        self.goto(Step::BattleOver);
    }

    fn end_of_battle(&mut self, ctx: &mut Ctx) {
        if self.battle_result == result::WON {
            let total = self.b().total_pay_day_money;
            if total != [0; 3] {
                add_bcd(&mut ctx.world.money, &total);
                self.push(Present::TextWith {
                    commands: far("_PickUpPayDayMoneyText"),
                    strings: vec![],
                    numbers: vec![],
                });
                ctx.world.text.money.insert(TextMoney::TotalPayDayMoney, total.to_vec());
            }
            let can_evolve = self.b().can_evolve_flags;
            self.push(Present::Push(Box::new(Mode::Evolution(Evolution::after_battle(can_evolve, self.cur_party_species)))));
        }
        self.goto(Step::EndOfBattleAfterEvolution);
    }

    fn finish(&mut self, ctx: &mut Ctx) -> Transition {
        // `EndOfBattle.resetVariables`.
        ctx.audio.end_low_health_alarm();
        ctx.menu.party_and_bills = 0;
        ctx.menu.bag_saved = 0;
        ctx.menu.battle_and_start = 0;
        ctx.menu.list_scroll = 0;
        Transition::Pop(Outcome::Chosen(self.battle_result))
    }
}

/// `ResidualEffects1`.
const RESIDUAL_EFFECTS_1: [u8; 16] = [effect::CONVERSION_EFFECT, effect::HAZE_EFFECT, effect::SWITCH_AND_TELEPORT_EFFECT,
    effect::MIST_EFFECT, effect::FOCUS_ENERGY_EFFECT, effect::CONFUSION_EFFECT, effect::HEAL_EFFECT,
    effect::TRANSFORM_EFFECT, effect::LIGHT_SCREEN_EFFECT, effect::REFLECT_EFFECT, effect::POISON_EFFECT,
    effect::PARALYZE_EFFECT, effect::SUBSTITUTE_EFFECT, effect::MIMIC_EFFECT, effect::LEECH_SEED_EFFECT,
    effect::SPLASH_EFFECT];
/// `ResidualEffects2`.
const RESIDUAL_EFFECTS_2: [u8; 27] = [effect::EFFECT_01, effect::ATTACK_UP1_EFFECT, effect::DEFENSE_UP1_EFFECT,
    effect::SPEED_UP1_EFFECT, effect::SPECIAL_UP1_EFFECT, effect::ACCURACY_UP1_EFFECT, effect::EVASION_UP1_EFFECT,
    effect::ATTACK_DOWN1_EFFECT, effect::DEFENSE_DOWN1_EFFECT, effect::SPEED_DOWN1_EFFECT, effect::SPECIAL_DOWN1_EFFECT,
    effect::ACCURACY_DOWN1_EFFECT, effect::EVASION_DOWN1_EFFECT, effect::BIDE_EFFECT, effect::SLEEP_EFFECT,
    effect::ATTACK_UP2_EFFECT, effect::DEFENSE_UP2_EFFECT, effect::SPEED_UP2_EFFECT, effect::SPECIAL_UP2_EFFECT,
    effect::ACCURACY_UP2_EFFECT, effect::EVASION_UP2_EFFECT, effect::ATTACK_DOWN2_EFFECT, effect::DEFENSE_DOWN2_EFFECT,
    effect::SPEED_DOWN2_EFFECT, effect::SPECIAL_DOWN2_EFFECT, effect::ACCURACY_DOWN2_EFFECT, effect::EVASION_DOWN2_EFFECT];
/// `AlwaysHappenSideEffects`.
const ALWAYS_HAPPEN_SIDE_EFFECTS: [u8; 10] = [effect::DRAIN_HP_EFFECT, effect::EXPLODE_EFFECT, effect::DREAM_EATER_EFFECT,
    effect::PAY_DAY_EFFECT, effect::TWO_TO_FIVE_ATTACKS_EFFECT, effect::EFFECT_1E, effect::ATTACK_TWICE_EFFECT,
    effect::RECOIL_EFFECT, effect::TWINEEDLE_EFFECT, effect::RAGE_EFFECT];
/// `SpecialEffects` and `SpecialEffectsCont` after it.
const SPECIAL_EFFECTS: [u8; 16] = [effect::DRAIN_HP_EFFECT, effect::EXPLODE_EFFECT, effect::DREAM_EATER_EFFECT,
    effect::PAY_DAY_EFFECT, effect::SWIFT_EFFECT, effect::TWO_TO_FIVE_ATTACKS_EFFECT, effect::EFFECT_1E,
    effect::CHARGE_EFFECT, effect::SUPER_FANG_EFFECT, effect::SPECIAL_DAMAGE_EFFECT, effect::FLY_EFFECT,
    effect::ATTACK_TWICE_EFFECT, effect::JUMP_KICK_EFFECT, effect::RECOIL_EFFECT,
    effect::THRASH_PETAL_DANCE_EFFECT, effect::TRAPPING_EFFECT];
const SPECIAL_EFFECTS_CONT: [u8; 2] = [effect::THRASH_PETAL_DANCE_EFFECT, effect::TRAPPING_EFFECT];

/// `PrintStatText`: the name of the stat a stat move's effect raises or lowers.
fn stat_name(move_effect: u8) -> Vec<u8> {
    const NAMES: [&str; 6] = ["ATTACK", "DEFENSE", "SPEED", "SPECIAL", "ACCURACY", "EVADE"];
    let index = match move_effect {
        effect::ATTACK_UP1_EFFECT..effect::ATTACK_DOWN1_EFFECT => move_effect - effect::ATTACK_UP1_EFFECT,
        effect::ATTACK_DOWN1_EFFECT..effect::BIDE_EFFECT => move_effect - effect::ATTACK_DOWN1_EFFECT,
        effect::ATTACK_UP2_EFFECT..effect::ATTACK_DOWN2_EFFECT => move_effect - effect::ATTACK_UP2_EFFECT,
        effect::ATTACK_DOWN2_EFFECT..effect::ATTACK_DOWN_SIDE_EFFECT => move_effect - effect::ATTACK_DOWN2_EFFECT,
        _ => move_effect.wrapping_sub(effect::ATTACK_DOWN_SIDE_EFFECT),
    };
    poke_core::charmap::encode(NAMES.get(index as usize).copied().unwrap_or("")).expect("encodes")
}
