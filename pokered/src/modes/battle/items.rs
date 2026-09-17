//! ITEM: `BagWasSelected`, `UseBagItem`, and `UseItem_` for what a battle can use. The routines with
//! a party menu are `UseItem`'s, and the battle mon is brought back into line with the party
//! afterwards as `ItemUseMedicine` and `ItemUsePPRestore` do it.

use poke_core::item::{self, ItemId};
use poke_core::map::Map;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::{pokered_events, pokered_symbols};
use poke_core::text_script::TextBuffer;
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::mode::{Ctx, Mode, Outcome};
use crate::modes::list_menu::{remove_from_bag, ListMenu};
use crate::modes::naming_screen::{NamingScreen, NamingScreenType};
use crate::modes::pokedex::PokedexMenu;
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::modes::use_item::UseItem;
use crate::party::{Named, PARTY_LENGTH};
use crate::systems::add_mon::{add_party_mon, new_party_mon, Origin};
use crate::systems::ball::{throw, BallInput, Throw};
use crate::systems::battle::effects::stat_modifiers::stat_modifier_up_effect;
use crate::systems::battle::effects::BattleText;
use crate::systems::battle::enemy::load_enemy_mon_data;
use crate::systems::battle::{effect, status, BattleKind, Side, Status2, Status3};
use crate::systems::item_use::ItemUse;
use super::animation::{anim, animation_type, AnimBattle, Routine};
use super::flow::Step;
use super::present::Present;
use super::text::{far, local};
use super::{BattleMode, BattleType};

/// `OldManItemList`.
const OLD_MAN_ITEM_LIST: [(ItemId, u8); 1] = [(ItemId::PokeBall, 50)];
/// `XSTATITEM_ANIM`.
const XSTATITEM_ANIM: u8 = 0xAE;
/// `hlcoord 14, 7`, where `AskName` asks.
const NICKNAME_AT: (usize, usize) = (14, 7);

/// The party as an item found it, for bringing the battle mon back into line afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PartyBefore {
    item: ItemId,
    hp: Vec<u16>,
    status: Vec<u8>,
    pp: Vec<[u8; 4]>,
}

/// A caught mon on its way into the party: what `_AddPartyMon` copies from the enemy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Catch {
    species: PokemonSpecies,
    level: u8,
    origin: Origin,
    /// The enemy mon as `SendNewMonToBox` copies it.
    enemy: crate::systems::battle::BattleMon,
}

impl BattleMode {
    /// `BagWasSelected` and `DisplayPlayerBag`.
    pub(super) fn bag_was_selected(&mut self, ctx: &mut Ctx) {
        self.push(Present::LoadScreen1);
        if self.battle_type == BattleType::Normal {
            self.push_hud(Side::Player);
            self.push_hud(Side::Enemy);
        }
        let bag = if self.battle_type == BattleType::OldMan {
            ListMenu::old_man(OLD_MAN_ITEM_LIST.to_vec())
        } else {
            ListMenu::bag(ctx.menu.bag_saved, ctx.menu.list_scroll)
        };
        self.push(Present::Push(Box::new(Mode::ListMenu(bag))));
        self.goto(Step::BagChosen);
    }

    /// `DisplayBagMenu`'s return: the battle menu again for B, else `UseBagItem`.
    pub(super) fn bag_chosen(&mut self, ctx: &mut Ctx) {
        ctx.menu.bag_saved = ctx.menu.chosen_item;
        let Some(Outcome::Chosen(slot)) = self.outcome.take() else {
            return self.goto(Step::DisplayBattleMenu);
        };
        let item = if self.battle_type == BattleType::OldMan { OLD_MAN_ITEM_LIST[slot as usize].0 } else { ctx.world.bag.items[slot as usize].id };
        let name = item::name(item);
        ctx.world.text.strings.insert(TextBuffer::NameBuffer, name.clone());
        ctx.world.text.strings.insert(TextBuffer::StringBuffer, name);
        // `UseItem_` starts from success.
        self.action_taken = true;
        match ItemUse::of(item) {
            ItemUse::Ball if self.battle_type == BattleType::OldMan => self.item_use_ball(item, None, ctx),
            ItemUse::Ball => self.item_use_ball(item, Some(slot), ctx),
            ItemUse::Medicine | ItemUse::PpRestore => {
                let party = &ctx.world.party;
                self.party_before = Some(PartyBefore {
                    item,
                    hp: party.iter().map(|mon| mon.mon.mon.hp).collect(),
                    status: party.iter().map(|mon| mon.mon.mon.status).collect(),
                    pp: party.iter().map(|mon| mon.mon.mon.pp).collect(),
                });
                let flow = UseItem::in_battle(item, slot).expect("medicine and PP are `UseItem`'s");
                self.push(Present::Push(Box::new(Mode::UseItem(flow))));
                self.goto(Step::ItemUsedFromParty);
            }
            ItemUse::XStat => self.item_use_x_stat(item, slot, ctx),
            ItemUse::XAccuracy | ItemUse::GuardSpec | ItemUse::DireHit => {
                let flag = match item {
                    ItemId::XAccuracy => Status2::USING_X_ACCURACY,
                    ItemId::GuardSpec => Status2::PROTECTED_BY_MIST,
                    _ => Status2::GETTING_PUMPED,
                };
                self.battle_mut().player.status2.insert(flag);
                self.print_item_use_text_and_remove_item(slot, ctx);
                self.goto(Step::AfterUseBagItem);
            }
            ItemUse::PokeDoll if self.b().kind == BattleKind::Wild => {
                self.battle_mut().escaped_from_battle = true;
                self.print_item_use_text_and_remove_item(slot, ctx);
                self.goto(Step::AfterUseBagItem);
            }
            ItemUse::PokeFlute => self.item_use_poke_flute(ctx),
            ItemUse::Pokedex => {
                self.push(Present::Push(Box::new(Mode::Pokedex(PokedexMenu::new()))));
                self.goto(Step::AfterUseBagItem);
            }
            ItemUse::OaksParcel => self.item_use_failed("_ItemUseNotYoursToUseText"),
            _ => self.item_use_failed("_ItemUseNotTimeText"),
        }
    }

    /// `ItemUseFailed`: the text, and nothing used.
    fn item_use_failed(&mut self, label: &str) {
        self.action_taken = false;
        self.push(Present::Text(far(label)));
        self.goto(Step::AfterUseBagItem);
    }

    /// `PrintItemUseTextAndRemoveItem`.
    fn print_item_use_text_and_remove_item(&mut self, slot: u8, ctx: &mut Ctx) {
        self.item_use_text(ctx);
        self.push(Present::Sound(sounds::SFX_HEAL_AILMENT));
        self.push(Present::WaitButton);
        remove_from_bag(ctx, slot as usize, 1);
    }

    /// `ItemUseText00`: the player used the item in `wStringBuffer`.
    fn item_use_text(&mut self, ctx: &Ctx) {
        let name = ctx.world.text.string(TextBuffer::StringBuffer);
        self.push(Present::TextWith {
            commands: local(pokered_symbols::ItemUseText00),
            strings: vec![(TextBuffer::StringBuffer, name)],
            numbers: vec![],
        });
    }

    /// `UseBagItem` once `UseItem` has returned.
    pub(super) fn after_use_bag_item(&mut self, ctx: &mut Ctx) {
        super::hud::load_hud_tiles(&mut ctx.screen.tiles);
        crate::gfx::mon_icons::clear_sprites(&mut ctx.screen.sprites);
        if !self.action_taken {
            return self.goto(Step::BagWasSelected);
        }
        let battle = self.battle_mut();
        if battle.player.status1.contains(crate::systems::battle::Status1::USING_TRAPPING_MOVE) {
            battle.player.num_attacks_left = battle.player.num_attacks_left.wrapping_sub(1);
            if battle.player.num_attacks_left == 0 {
                battle.player.status1.remove(crate::systems::battle::Status1::USING_TRAPPING_MOVE);
            }
        }
        if self.captured {
            // `.returnAfterCapturingMon`: the battle ends as a draw, through `DisplayBattleMenu`'s carry.
            self.captured = false;
            self.battle_result = super::result::DRAW;
            self.ran = true;
            return;
        }
        if self.battle_type == BattleType::Safari {
            return;
        }
        self.push(Present::LoadScreen1);
        self.push_hud(Side::Player);
        self.push_hud(Side::Enemy);
    }

    /// `ItemUseMedicine` and `ItemUsePPRestore`'s writes to the battle mon, done by `UseItem` to
    /// the party alone and brought across here from what changed.
    pub(super) fn item_used_from_party(&mut self, ctx: &mut Ctx) {
        let used = !matches!(self.outcome.take(), Some(Outcome::Chosen(crate::modes::use_item::NOT_USED)));
        self.action_taken = used;
        let before = self.party_before.take().expect("the party was noted before the item");
        let out = self.b().player_mon_number as usize;
        for (slot, named) in ctx.world.party.iter().enumerate() {
            let mon = &named.mon.mon;
            let (old_hp, old_status) = (before.hp[slot], before.status[slot]);
            // `.updateInBattleFaintedData`: a revived mon that fought this enemy gains experience.
            let battle = self.battle.as_mut().expect("a battle");
            if old_hp == 0 && mon.hp != 0 && battle.fought_current_enemy_flags & 1 << slot != 0 {
                battle.gain_exp_flags |= 1 << slot;
            }
            if slot != out {
                continue;
            }
            let player = &mut battle.player;
            if mon.hp != old_hp {
                player.mon.hp = mon.hp;
                if before.item == ItemId::FullRestore {
                    player.mon.status = 0;
                }
            } else if old_status != 0 && mon.status == 0 {
                // `.cureStatusAilment`: the party's stats over the battle mon's, stages and all.
                player.mon.status = 0;
                player.status3.remove(Status3::BADLY_POISONED);
                player.mon.stats = named.mon.stats;
            }
            if mon.pp != before.pp[slot] {
                player.mon.pp = mon.pp;
            }
        }
        self.goto(Step::AfterUseBagItem);
    }

    /// `ItemUseXStat`: the stat move effect run as the player's, with its text first.
    fn item_use_x_stat(&mut self, item: ItemId, slot: u8, ctx: &mut Ctx) {
        let stat_effect = effect::ATTACK_UP1_EFFECT + (item as u8 - ItemId::XAttack as u8);
        self.print_item_use_text_and_remove_item(slot, ctx);
        self.push(Present::LoadScreen1);
        let badges = ctx.world.badges;
        let battle = self.battle_mut();
        let saved = battle.player.current_move;
        battle.player.current_move.effect = stat_effect;
        battle.player.current_move.animation = XSTATITEM_ANIM;
        let texts = stat_modifier_up_effect(battle, Side::Player, badges);
        for text in texts {
            if text == BattleText::MonsStatsRoseText {
                self.play_current_move_animation(Side::Player);
            }
            self.battle_text_for(text, Side::Player, stat_effect);
        }
        let battle = self.battle_mut();
        battle.player.current_move.effect = saved.effect;
        battle.player.current_move.animation = saved.animation;
        self.goto(Step::AfterUseBagItem);
    }

    /// `ItemUsePokeFlute.inBattle`: every sleeping mon on both sides woken, a trainer's party too.
    fn item_use_poke_flute(&mut self, ctx: &mut Ctx) {
        let battle = self.battle.as_mut().expect("a battle");
        let mut woke = false;
        let mut wake = |status: &mut u8| {
            woke |= *status & status::SLP_MASK != 0;
            *status &= !status::SLP_MASK;
        };
        for named in &mut ctx.world.party {
            wake(&mut named.mon.mon.status);
        }
        if battle.kind == BattleKind::Trainer {
            for mon in &mut battle.enemy_party {
                wake(&mut mon.mon.status);
            }
        }
        battle.player.mon.status &= !status::SLP_MASK;
        battle.enemy.mon.status &= !status::SLP_MASK;
        self.push(Present::LoadScreen2);
        if !woke {
            self.push(Present::Text(far("_PlayedFluteNoEffectText")));
            return self.goto(Step::AfterUseBagItem);
        }
        self.push(Present::Text(local(pokered_symbols::PlayedFluteHadEffectText)));
        self.push(Present::PokeFluteInBattle);
        self.push(Present::Text(far("_FluteWokeUpText")));
        self.goto(Step::AfterUseBagItem);
    }

    /// `ItemUseBall`.
    /// `slot` is the ball's place in the bag, and `None` for the Safari Zone's, which are counted apart.
    pub(super) fn item_use_ball(&mut self, ball: ItemId, slot: Option<u8>, ctx: &mut Ctx) {
        if self.b().kind == BattleKind::Trainer {
            // `ThrowBallAtTrainerMon`.
            self.push(Present::LoadScreen1);
            self.toss(ball, 0, self.whose_turn);
            self.push(Present::Text(far("_ThrowBallAtTrainerMonText1")));
            self.push(Present::Text(far("_ThrowBallAtTrainerMonText2")));
            if let Some(slot) = slot {
                remove_from_bag(ctx, slot as usize, 1);
            }
            return self.goto(Step::AfterUseBagItem);
        }
        let old_man = self.battle_type == BattleType::OldMan;
        if !old_man && ctx.world.party.len() == PARTY_LENGTH && super::boxes::box_is_full(ctx) {
            return self.item_use_failed("_BoxFullCannotThrowBallText");
        }
        if self.battle_type == BattleType::Safari {
            super::boxes::use_safari_ball(ctx);
        }
        self.push(Present::LoadScreen1);
        self.item_use_text(ctx);
        let enemy = self.b().enemy.mon.clone();
        let restless_soul = ctx.world.location.map == Map::PokemonTower6F && enemy.species == PokemonSpecies::Marowak;
        let outcome = if old_man {
            // The name the menu borrowed goes back, and the ball always holds.
            ctx.world.player_name = self.saved_player_name.take().expect("the old man's menu saved the name");
            Some(Throw::Caught)
        } else if self.is_ghost_battle(ctx) || restless_soul {
            None
        } else {
            let input = BallInput { ball, catch_rate: self.b().enemy_exp.catch_rate, hp: enemy.hp, max_hp: enemy.stats[0], status: enemy.status };
            Some(throw(&input, ctx.rng))
        };
        self.push(Present::Frames(20));
        self.animation_type = animation_type::NONE;
        self.battle_mut().damage_multipliers = 0;
        let data = match outcome {
            None => 0x10,
            Some(Throw::Broke { shakes: 0 }) => 0x20,
            Some(Throw::Broke { shakes }) => 0x60 | shakes,
            Some(Throw::Caught) => 0x43,
        };
        self.toss(ball, data, Side::Player);
        let label = match outcome {
            None => Some("_ItemUseBallText00"),
            Some(Throw::Broke { shakes: 0 }) => Some("_ItemUseBallText01"),
            Some(Throw::Broke { shakes: 1 }) => Some("_ItemUseBallText02"),
            Some(Throw::Broke { shakes: 2 }) => Some("_ItemUseBallText03"),
            Some(Throw::Broke { .. }) => Some("_ItemUseBallText04"),
            Some(Throw::Caught) => None,
        };
        if let Some(slot) = slot {
            remove_from_bag(ctx, slot as usize, 1);
        }
        if let Some(label) = label {
            self.push(Present::Text(far(label)));
            return self.goto(Step::AfterUseBagItem);
        }
        self.caught(ctx);
    }

    /// `MoveAnimation` of `TOSS_ANIM` for `ball`, `wPokeBallAnimData` being `data`.
    fn toss(&mut self, ball: ItemId, data: u8, turn: Side) {
        self.whose_turn = turn;
        let battle = AnimBattle { item: ball as u8, ball_data: data, ..self.anim_battle() };
        self.push(Present::Animation { routine: Routine::MoveAnimation { id: anim::TOSS_ANIM, kind: self.animation_type }, turn, battle });
    }

    /// `ItemUseBall` from `.captured` to `AddPartyMon` or `SendNewMonToBox`.
    fn caught(&mut self, ctx: &mut Ctx) {
        // The mon is loaded again as though transformed, so its DVs, HP and status stay; a mon that
        // really was transformed is taken to be a Ditto.
        let battle = self.battle.as_mut().expect("a battle");
        let (hp, status_byte) = (battle.enemy.mon.hp, battle.enemy.mon.status);
        let species = if battle.enemy.status3.contains(Status3::TRANSFORMED) {
            PokemonSpecies::Ditto
        } else {
            battle.enemy.status3.insert(Status3::TRANSFORMED);
            battle.transformed_enemy_original_dvs = battle.enemy.mon.dvs;
            battle.enemy.mon.species
        };
        let level = battle.enemy.mon.level;
        load_enemy_mon_data(battle, &mut ctx.world.pokedex, species, level, 0, ctx.rng);
        battle.enemy.mon.hp = hp;
        battle.enemy.mon.status = status_byte;
        self.enemy_nick = species.name();
        self.captured = true;
        self.push(Present::TextWith {
            commands: local(pokered_symbols::ItemUseBallText05),
            strings: vec![(TextBuffer::EnemyMonNick, self.enemy_nick.clone())],
            numbers: vec![],
        });
        if self.battle_type == BattleType::OldMan {
            self.push(Present::ClearSprites);
            return self.goto(Step::AfterUseBagItem);
        }
        let owned = ctx.world.pokedex.is_owned(species);
        ctx.world.pokedex.set_owned(species);
        if !owned {
            self.push(Present::TextWith {
                commands: local(pokered_symbols::ItemUseBallText06),
                strings: vec![(TextBuffer::EnemyMonNick, self.enemy_nick.clone())],
                numbers: vec![],
            });
            let dex = species.metadata().pokedex_number;
            self.push(Present::Push(Box::new(Mode::Pokedex(PokedexMenu::data_page(dex)))));
        }
        let enemy = &self.b().enemy.mon;
        let origin = Origin::Caught { dvs: enemy.dvs, hp: enemy.hp, status: enemy.status, stats: enemy.stats };
        self.catch = Some(Catch { species, level: enemy.level, origin, enemy: enemy.clone() });
        self.goto(Step::AskName);
    }

    /// `AskName`: in a wild battle the enemy's HUD corner is cleared, then the question.
    pub(super) fn ask_name(&mut self) {
        let species = self.catch.as_ref().expect("a catch to name").species;
        self.push(Present::SaveScreen1);
        self.push(Present::Clear { x: 0, y: 0, width: 11, height: 4 });
        self.push(Present::TextWith {
            commands: local(pokered_symbols::DoYouWantToNicknameText),
            strings: vec![(TextBuffer::NameBuffer, species.name())],
            numbers: vec![],
        });
        let menu = TwoOptionMenu::new(TwoOptionMenuId::YesNo, NICKNAME_AT, false);
        self.push(Present::Push(Box::new(Mode::TwoOptionMenu(menu))));
        self.goto(Step::NicknameAnswered);
    }

    pub(super) fn nickname_answered(&mut self, ctx: &mut Ctx) {
        let yes = self.outcome.take() == Some(Outcome::Chosen(0))
            && ctx.menu.exit_method == crate::modes::menu_input::MenuExit::Chose;
        if !yes {
            return self.add_caught_mon(None, ctx);
        }
        let species = self.catch.as_ref().expect("a catch to name").species;
        ctx.world.text.strings.insert(TextBuffer::StringBuffer, vec![]);
        self.push(Present::Push(Box::new(Mode::NamingScreen(NamingScreen::new(NamingScreenType::Mon, Some(species))))));
        self.push(Present::LoadScreen1);
        self.goto(Step::NicknameEntered);
    }

    pub(super) fn nickname_entered(&mut self, ctx: &mut Ctx) {
        self.outcome = None;
        let name = ctx.world.text.string(TextBuffer::StringBuffer);
        self.add_caught_mon((!name.is_empty()).then_some(name), ctx);
    }

    /// `AddPartyMon`, or `SendNewMonToBox` and where it went.
    fn add_caught_mon(&mut self, nick: Option<Vec<u8>>, ctx: &mut Ctx) {
        let Catch { species, level, origin, enemy } = self.catch.take().expect("a catch to add");
        let nick = nick.unwrap_or_else(|| species.name());
        let ot = ctx.world.player_name.clone();
        if ctx.world.party.len() < PARTY_LENGTH {
            let mon = new_party_mon(species, level, ctx.world.player_id, &origin, ctx.rng);
            add_party_mon(&mut ctx.world.party, Named { mon, ot, nick }, Some(&mut ctx.world.pokedex));
            return self.goto(Step::AfterUseBagItem);
        }
        super::boxes::send_new_mon_to_box(ctx, &enemy, ot, nick.clone());
        let label = if ctx.world.events.is_set(pokered_events::EVENT_MET_BILL as u16) { "_ItemUseBallText07" } else { "_ItemUseBallText08" };
        self.push(Present::TextWith {
            commands: far(label),
            strings: vec![(TextBuffer::BoxMonNicks, nick)],
            numbers: vec![],
        });
        self.goto(Step::AfterUseBagItem);
    }
}
