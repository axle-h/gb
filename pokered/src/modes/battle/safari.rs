//! The Safari Zone's battle: no mon is sent out, the menu is BALL, BAIT, THROW ROCK and RUN, and
//! after each turn the mon may eat, be angry, or run.

use poke_core::item::ItemId;
use poke_core::map::Map;
use poke_core::text_script::TextBuffer;
use crate::audio::data::sounds;
use crate::mode::Ctx;
use crate::systems::battle::safari::{safari_mon_runs, safari_zone_battle_text, throw_bait, throw_rock, SafariText};
use crate::systems::battle::Side;
use super::animation::{anim, Routine};
use super::flow::Step;
use super::menus::Menu;
use super::present::{BattleBox, Present};
use super::text::far;
use super::BattleMode;

const CURSOR: u8 = 0xED;
const UNFILLED_CURSOR: u8 = 0xEC;

/// The Safari menu's items, as `.handleMenuSelection` numbers them: nothing is swapped.
pub(super) const BALL: u8 = 0;
pub(super) const ROCK: u8 = 1;
pub(super) const BAIT: u8 = 2;

/// `InitBattleVariables`: a wild battle on the maps from `SAFARI_ZONE_EAST` up to the centre's rest
/// house is the Safari Zone's.
pub fn is_safari_map(map: Map) -> bool {
    (Map::SafariZoneEast as u8..Map::SafariZoneCenterRestHouse as u8).contains(&(map as u8))
}

impl BattleMode {
    /// `DisplayBattleMenu` for the old man's demo: he is named for the player, and the menu is
    /// worked by hand, ITEM after eighty frames on FIGHT and fifty on ITEM.
    pub(super) fn display_old_man_battle_menu(&mut self, ctx: &mut Ctx) {
        self.push(Present::LoadScreen1);
        self.push(Present::TextBox(BattleBox::BattleMenu));
        self.saved_player_name = Some(std::mem::replace(&mut ctx.world.player_name,
            poke_core::charmap::encode("OLD MAN").expect("encodes")));
        self.push(Present::Tile { x: 9, y: 14, tile: CURSOR });
        self.push(Present::Frames(80));
        self.push(Present::Tile { x: 9, y: 14, tile: crate::gfx::ui::UiSurface::BLANK });
        self.push(Present::Tile { x: 9, y: 16, tile: CURSOR });
        self.push(Present::Frames(50));
        self.push(Present::Tile { x: 9, y: 16, tile: UNFILLED_CURSOR });
        self.push(Present::SaveScreen2);
        self.goto(Step::BagWasSelected);
    }

    /// A text reading `wEnemyMonNick`.
    pub(super) fn enemy_text(&mut self, label: &str) {
        self.push(Present::TextWith {
            commands: far(label),
            strings: vec![(TextBuffer::EnemyMonNick, self.enemy_nick.clone())],
            numbers: vec![],
        });
    }

    /// `DisplayBattleMenu` for the Safari Zone, with the balls left printed.
    pub(super) fn display_safari_battle_menu(&mut self, ctx: &mut Ctx) {
        self.push(Present::LoadScreen1);
        self.push(Present::TextBox(BattleBox::SafariBattleMenu));
        let saved = ctx.menu.battle_and_start;
        ctx.menu.last_item = saved;
        let right = saved >= 2;
        let current = if right { saved - 2 } else { saved };
        if right {
            ctx.menu.last_item = current;
        }
        self.open_safari_menu_column(right, current, ctx);
    }

    pub(super) fn open_safari_menu_column(&mut self, right: bool, current: u8, ctx: &Ctx) {
        let clear_x = if right { 1 } else { 13 };
        self.push(Present::Clear { x: clear_x, y: 14, width: 1, height: 1 });
        self.push(Present::Clear { x: clear_x, y: 16, width: 1, height: 1 });
        self.push(Present::SafariBallCount(super::boxes::safari_balls(ctx)));
        self.menu = Some(Menu::safari(right, current));
        self.push(Present::Menu);
    }

    /// `.handleMenuSelection` in the Safari Zone: `BALL`, `ROCK`, `BAIT`, then RUN.
    pub(super) fn safari_menu_chosen(&mut self, id: u8, ctx: &mut Ctx) {
        self.action_taken = true;
        match id {
            0 => {
                let name = poke_core::item::name(ItemId::SafariBall);
                ctx.world.text.strings.insert(TextBuffer::NameBuffer, name.clone());
                ctx.world.text.strings.insert(TextBuffer::StringBuffer, name);
                self.item_use_ball(ItemId::SafariBall, None, ctx);
            }
            1 | 2 => {
                self.push(Present::SaveScreen2);
                let battle = self.battle.as_mut().expect("a battle");
                let label = if id == 1 {
                    throw_rock(battle, ctx.rng);
                    "_ThrewRockText"
                } else {
                    throw_bait(battle, ctx.rng);
                    "_ThrewBaitText"
                };
                self.push(Present::Text(far(label)));
                let id = if id == 1 { anim::ROCK_ANIM } else { anim::BAIT_ANIM };
                self.play_battle_animation(id, Side::Player);
                self.push(Present::Frames(70));
                self.goto(Step::AfterUseBagItem);
            }
            _ => {
                self.push(Present::LoadScreen1);
                let speed = self.b().player.mon.stats[3];
                let mut attempts = self.num_run_attempts;
                let run = crate::systems::battle::escape::try_running_from_battle(self.b(), speed, true, &mut attempts, ctx.rng);
                self.num_run_attempts = attempts;
                self.present_run(&run);
            }
        }
    }

    /// `StartBattle.displaySafariZoneBattleMenu`'s return.
    pub(super) fn after_safari_menu(&mut self, ctx: &mut Ctx) {
        if self.ran {
            return self.goto(Step::BattleOver);
        }
        if super::boxes::safari_balls(ctx) == 0 {
            self.push(Present::LoadScreen1);
            self.push(Present::Text(far("_OutOfSafariBallsText")));
            return self.goto(Step::BattleOver);
        }
        let battle = self.battle.as_mut().expect("a battle");
        if let Some(text) = safari_zone_battle_text(battle) {
            self.push(Present::LoadScreen1);
            self.enemy_text(match text {
                SafariText::Eating => "_SafariZoneEatingText",
                SafariText::Angry => "_SafariZoneAngryText",
            });
        }
        if !safari_mon_runs(self.b(), ctx.rng) {
            return self.goto(Step::SafariCheckAnyPartyAlive);
        }
        // `EnemyRan`.
        self.push(Present::LoadScreen1);
        self.enemy_text("_WildRanText");
        self.push(Present::SoundAfterCurrent(sounds::SFX_RUN));
        self.animate(Routine::SlideEnemyMonOff, Side::Player);
        self.goto(Step::BattleOver);
    }
}
