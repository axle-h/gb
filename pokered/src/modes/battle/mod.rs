//! The battle: `InitBattle` to `EndOfBattle` over the arithmetic in `systems::battle`, popping
//! `wBattleResult` as `Outcome::Chosen`.
//!
//! The mode keeps the cartridge's control flow as two queues. `Step` is where its code is, a label
//! at a time, with a stack for the few routines called from more than one place; a step does the
//! arithmetic and lines up what the player is shown as `Present`s, which run in order, each taking
//! the frames it takes, before the next step. Nothing random is drawn by a `Present`, so the order
//! of random bytes is the steps' alone; and a `Present` carries what it shows rather than reading
//! the battle, which a later step may already have moved on.
//!
//! The animations are `animation.rs`'s and the transition and the opening slide `transition.rs`'s,
//! each a `Present` that plays out over its frames. Link battles are not recreated.

mod animate;
pub mod animation;
mod boxes;
mod flow;
pub mod hud;
mod items;
mod menus;
mod present;
mod safari;
mod text;
pub mod transition;

use std::collections::VecDeque;
use poke_core::species::PokemonSpecies;
use serde::{Deserialize, Serialize};
use crate::command::{Command, Decision, Drive, Refusal};
use crate::gfx::ui::UiSurface;
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::systems::battle::Battle;
use crate::world::World;
use flow::Step;
use menus::Menu;
use present::{HpBarColours, Present, Waiting};

/// Who the player faces: a wild mon, or a trainer from `TrainerDataPointers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Opponent {
    /// `wCurOpponent` below `OPP_ID_OFFSET` and `wCurEnemyLevel`.
    Wild { species: PokemonSpecies, level: u8 },
    /// `wTrainerClass` and `wTrainerNo` from 1; `wLoneAttackNo` for a gym leader, else 0; and
    /// `wRivalStarter`, which the champion's moves depend on.
    Trainer { class: u8, number: u8, lone_attack: u8, rival_starter: u8 },
}

/// `wBattleType`: the Safari Zone's and the old man's run through `StartBattle`'s safari loop,
/// with no mon of the player's sent out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BattleType {
    #[default]
    Normal,
    OldMan,
    Safari,
}

/// `wBattleResult`, which the battle pops as `Outcome::Chosen`.
pub mod result {
    pub const WON: u8 = 0;
    pub const LOST: u8 = 1;
    pub const DRAW: u8 = 2;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BattleMode {
    opponent: Opponent,
    battle: Option<Battle>,
    /// `wBattleMonNick` and `wEnemyMonNick`, unterminated.
    player_nick: Vec<u8>,
    enemy_nick: Vec<u8>,
    num_run_attempts: u8,
    /// `wActionResultOrTookBattleTurn`.
    action_taken: bool,
    battle_result: u8,
    in_handle_player_mon_fainted: bool,
    low_health_alarm_disabled: bool,
    /// `wMenuItemToSwap` for the move menu, from 1.
    move_to_swap: u8,
    /// `wCurPartySpecies` as the battle leaves it, which `EvolutionAfterBattle` reads as the item.
    cur_party_species: u8,
    /// `wTileMapBackup` and `wTileMapBackup2`.
    buffer1: Option<UiSurface>,
    buffer2: Option<UiSurface>,
    steps: Vec<Step>,
    queue: VecDeque<Present>,
    waiting: Waiting,
    /// What the last child mode popped with.
    outcome: Option<Outcome>,
    menu: Option<Menu>,
    /// `wBattleType`.
    #[serde(default)]
    battle_type: BattleType,
    /// `wPlayerName` as the old man's demo found it, kept while he is named instead.
    #[serde(default)]
    saved_player_name: Option<Vec<u8>>,
    /// `DisplayBattleMenu`'s carry: the player ran.
    ran: bool,
    /// `wNumMovesMinus1`.
    num_moves_minus_one: u8,
    /// `wCurrentMenuItem` and `wMaxMenuItem` as the move menu left them, which disobedience reads.
    move_menu: crate::systems::battle::turn::MoveMenu,
    /// `wTrainerName`, and `wAmountMoneyWon` as `ReadTrainer` works it out.
    trainer_name: Vec<u8>,
    prize_money: [u8; 3],
    /// `wFirstMonsNotOutYet`: no shift prompt before either side's first mon is out, or for a mon
    /// the AI switches in.
    first_mons_not_out_yet: bool,
    /// `wLastSwitchInEnemyMonHP`, which the player's retreat line reads.
    last_switch_in_enemy_hp: u16,
    /// What the trainer says on losing, which `PrintEndBattleText` prints from the map's own text
    /// pointers; nothing without it.
    end_battle_text: Option<Vec<poke_core::text_script::TextCommand>>,
    /// The party as an item with a party menu found it.
    party_before: Option<items::PartyBefore>,
    /// `wCapturedMonSpecies`, as a flag, and the catch on its way into the party.
    captured: bool,
    catch: Option<items::Catch>,
    /// The `b` `ExecutePlayerMove` returns: whether the target is still standing.
    target_standing: bool,
    answered: u32,
    /// `wAnimationType` as the last routine to write it left it.
    #[serde(default)]
    animation_type: u8,
    /// `hWhoseTurn` as last written, which the routines that never set it read.
    #[serde(default = "player_side")]
    whose_turn: crate::systems::battle::Side,
    /// The OAM block `BattleTransition` leaves for the enemy trainer's sprite.
    #[serde(default)]
    trainer_oam_block: Option<u8>,
    /// `MoveAnimation` and `BattleTransition` returned from unentered.
    #[serde(default)]
    skip_move_animations: bool,
    /// `hSCX`, hidden under the window but for the effects that move it aside.
    #[serde(default)]
    h_scx: u8,
    /// `wPlayerHPBarColor` and `wEnemyHPBarColor`, as the HUDs last drew them.
    #[serde(default)]
    hp_bar_colours: HpBarColours,
    /// `wBattleMonSpecies` and `wEnemyMonSpecies2` as `SetPal_Battle` reads them, which the mons
    /// do not keep: 0 before a side's first mon is loaded, and the enemy's 0 again once a trainer's
    /// pic scrolls back in.
    #[serde(default)]
    pal_species: [u8; 2],
}

fn player_side() -> crate::systems::battle::Side {
    crate::systems::battle::Side::Player
}

impl BattleMode {
    /// `InitBattle` for a wild encounter.
    pub fn wild(species: PokemonSpecies, level: u8) -> Self {
        Self::new(Opponent::Wild { species, level })
    }

    /// The old man's catching demo in Viridian City: `BATTLE_TYPE_OLD_MAN` against a wild mon.
    pub fn old_man(species: PokemonSpecies, level: u8) -> Self {
        Self { battle_type: BattleType::OldMan, ..Self::wild(species, level) }
    }

    /// `InitOpponent` for a trainer.
    pub fn trainer(class: u8, number: u8, lone_attack: u8, rival_starter: u8) -> Self {
        Self::new(Opponent::Trainer { class, number, lone_attack, rival_starter })
    }

    fn new(opponent: Opponent) -> Self {
        Self {
            opponent,
            battle: None,
            player_nick: vec![],
            enemy_nick: vec![],
            num_run_attempts: 0,
            action_taken: false,
            battle_result: 0,
            in_handle_player_mon_fainted: false,
            low_health_alarm_disabled: false,
            move_to_swap: 0,
            cur_party_species: 0,
            buffer1: None,
            buffer2: None,
            steps: vec![Step::InitBattle],
            queue: VecDeque::new(),
            waiting: Waiting::Nothing,
            outcome: None,
            menu: None,
            battle_type: BattleType::Normal,
            saved_player_name: None,
            ran: false,
            party_before: None,
            captured: false,
            catch: None,
            trainer_name: vec![],
            prize_money: [0; 3],
            first_mons_not_out_yet: false,
            last_switch_in_enemy_hp: 0,
            end_battle_text: None,
            num_moves_minus_one: 0,
            move_menu: crate::systems::battle::turn::MoveMenu { current: 0, max: 0 },
            target_standing: true,
            answered: 0,
            animation_type: 0,
            whose_turn: crate::systems::battle::Side::Player,
            trainer_oam_block: None,
            skip_move_animations: false,
            h_scx: 0,
            hp_bar_colours: HpBarColours::default(),
            pal_species: [0; 2],
        }
    }

    /// The trainer's words on being beaten, printed between the prize and the pic scrolling back in.
    pub fn with_end_battle_text(mut self, commands: Vec<poke_core::text_script::TextCommand>) -> Self {
        self.end_battle_text = Some(commands);
        self
    }

    /// `BattleTransition` keeps OAM block `block`, where `hSpriteIndex`'s trainer is drawn.
    pub fn with_trainer_oam_block(mut self, block: u8) -> Self {
        self.trainer_oam_block = Some(block);
        self
    }

    /// `MoveAnimation` and `BattleTransition` return at once, as a caller that skips both on the
    /// cartridge needs them to.
    pub fn without_move_animations(mut self) -> Self {
        self.skip_move_animations = true;
        self
    }

    /// Presses answered at a prompt the battle waits at itself, so a driver can see its press land.
    pub fn answered(&self) -> u32 {
        self.answered
    }

    /// Whether an animation, the transition or the opening slide is playing.
    pub fn animating(&self) -> bool {
        matches!(self.waiting, Waiting::Animation(_) | Waiting::Transition(_) | Waiting::Silhouettes(_))
    }

    /// Whether an HP bar is moving.
    pub fn draining_hp_bar(&self) -> bool {
        matches!(self.waiting, Waiting::HpBar(_))
    }

    /// The cursor of whichever of the battle's own menus is up.
    pub fn selected(&self) -> u8 {
        self.menu.as_ref().map_or(0, Menu::selected)
    }

    /// The enemy's moves Mimic's menu offers.
    pub fn mimic_rows(&self) -> u8 {
        self.num_moves_minus_one + 1
    }

    /// The battle's state, once it has started.
    pub fn battle(&self) -> Option<&Battle> {
        self.battle.as_ref()
    }

    /// `wBattleType`.
    pub fn battle_type(&self) -> BattleType {
        self.battle_type
    }

    pub(super) fn battle_mut(&mut self) -> &mut Battle {
        self.battle.as_mut().expect("the battle has started")
    }

    /// Runs presentation and then steps until something waits a frame.
    fn run(&mut self, ctx: &mut Ctx) -> Transition {
        for _ in 0..100_000 {
            match self.wait(ctx) {
                Some(Transition::Stay) => return Transition::Stay,
                Some(transition) => return transition,
                None => {}
            }
            if let Some(present) = self.queue.pop_front() {
                if let Some(transition) = self.present(present, ctx) {
                    return transition;
                }
                continue;
            }
            let step = self.steps.pop().expect("the battle always has a next step");
            if let Some(transition) = self.step(step, ctx) {
                return transition;
            }
        }
        panic!("the battle never waited");
    }
}

impl ModeUpdate for BattleMode {
    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        self.run(ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        self.run(ctx)
    }

    fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        self.outcome = Some(outcome);
        self.waiting = Waiting::Nothing;
        self.run(ctx)
    }

    fn status(&self) -> Status {
        match &self.waiting {
            Waiting::Menu if self.menu.as_ref().is_some_and(Menu::is_polling) =>
                Status::Waiting(self.menu.as_ref().expect("a menu").decision()),
            Waiting::Button { polled: true } => Status::Waiting(Decision::Text),
            _ => Status::Busy,
        }
    }
}

/// Carries out `Fight`, `Run` and `SwitchPokemon` through the battle's menus, and the party menu the
/// battle opens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BattleDriver {
    command: Command,
    released: bool,
    /// The press that answers the command has gone in.
    answered: bool,
    /// A in the party menu has gone in, which answers unless SWITCH is asked next.
    chose_in_party: bool,
}

fn battle_below(modes: &[Mode]) -> Option<&BattleMode> {
    modes.iter().rev().find_map(|mode| match mode {
        Mode::Battle(battle) => Some(battle),
        _ => None,
    })
}

impl BattleDriver {
    pub fn accept(command: &Command, modes: &[Mode], world: &World) -> Result<Self, Refusal> {
        let Some(battle) = battle_below(modes) else {
            return Err(Refusal::Invalid("no battle is up".into()));
        };
        let top = modes.last().map(Mode::status);
        let at_menu = matches!(top, Some(Status::Waiting(Decision::BattleMenu | Decision::BattleMoves)))
            && matches!(modes.last(), Some(Mode::Battle(_)));
        let safari = battle.battle_type == BattleType::Safari;
        match command {
            Command::SafariBall | Command::SafariBait | Command::SafariRock if !safari =>
                return Err(Refusal::Invalid("only the Safari Zone has that menu".into())),
            Command::Fight(_) | Command::SwitchPokemon(_) | Command::UseItem { .. } if safari =>
                return Err(Refusal::Invalid("the Safari Zone's menu is BALL, BAIT, THROW ROCK and RUN".into())),
            Command::SafariBall if at_menu && world.safari_balls == 0 =>
                return Err(Refusal::Invalid("no Safari Balls left".into())),
            Command::SafariBall | Command::SafariBait | Command::SafariRock if at_menu => {}
            Command::Fight(slot) if at_menu => {
                let battle = battle.battle().expect("a battle waiting has started");
                if battle.player.mon.moves.get(*slot as usize).copied().flatten().is_none() {
                    return Err(Refusal::Invalid(format!("no move in slot {}", slot + 1)));
                }
            }
            Command::Run if at_menu => {}
            Command::SwitchPokemon(slot) => {
                let in_party = matches!(modes.last(), Some(Mode::PartyMenu(_))) && top == Some(Status::Waiting(Decision::PartyMenu));
                let at_submenu = matches!(modes.last(), Some(Mode::Battle(_))) && top == Some(Status::Waiting(Decision::SwitchStatsCancel));
                if !(at_menu || in_party || at_submenu) {
                    return Err(Refusal::Invalid("no battle menu or party menu is waiting".into()));
                }
                let Some(mon) = world.party.get(*slot as usize) else {
                    return Err(Refusal::Invalid(format!("the party has no slot {}", slot + 1)));
                };
                if mon.mon.mon.hp == 0 {
                    return Err(Refusal::Invalid("that mon has no will to fight".into()));
                }
                let out = battle.battle().map(|battle| battle.player_mon_number);
                if out == Some(*slot) && battle.battle().is_some_and(|battle| battle.player.mon.hp != 0) {
                    return Err(Refusal::Invalid("that mon is already out".into()));
                }
            }
            Command::UseItem { item, target } if at_menu => {
                if !world.bag.items.iter().any(|slot| slot.id == *item) {
                    return Err(Refusal::Invalid(format!("the bag has no {item:?}")));
                }
                use crate::systems::item_use::ItemUse;
                let asks = matches!(ItemUse::of(*item), ItemUse::Medicine | ItemUse::PpRestore);
                match (asks, target) {
                    (true, None) => return Err(Refusal::Invalid("that item is used on a party mon: say which".into())),
                    (false, Some(_)) => return Err(Refusal::Invalid("that item is not used on a party mon".into())),
                    (true, Some(slot)) if *slot as usize >= world.party.len() =>
                        return Err(Refusal::Invalid(format!("the party has no slot {}", slot + 1))),
                    _ => {}
                }
            }
            _ => return Err(Refusal::Invalid("no battle menu is waiting".into())),
        }
        Ok(Self { command: command.clone(), released: true, answered: false, chose_in_party: false })
    }

    pub fn drive(&mut self, modes: &[Mode], world: &World) -> Drive {
        if self.answered || battle_below(modes).is_none() {
            return Drive::Done;
        }
        let Some(top) = modes.last() else { return Drive::Done };
        let Status::Waiting(decision) = top.status() else {
            self.released = true;
            return Drive::Press(Joypad::empty());
        };
        if self.chose_in_party && decision != Decision::SwitchStatsCancel {
            return Drive::Done;
        }
        if !self.released {
            self.released = true;
            return Drive::Press(Joypad::empty());
        }
        let press = match (top, &self.command, decision) {
            (Mode::ListMenu(list), Command::UseItem { item, target }, Decision::List) => {
                let Some(index) = world.bag.items.iter().position(|slot| slot.id == *item) else { return Drive::Done };
                let press = match index.cmp(&list.selected()) {
                    std::cmp::Ordering::Less => Joypad::UP,
                    std::cmp::Ordering::Greater => Joypad::DOWN,
                    std::cmp::Ordering::Equal => Joypad::A,
                };
                self.answered = target.is_none() && press == Joypad::A;
                press
            }
            (Mode::PartyMenu(menu), Command::UseItem { target: Some(slot), .. }, Decision::PartyMenu) => {
                let press = match slot.cmp(&menu.selected()) {
                    std::cmp::Ordering::Less => Joypad::UP,
                    std::cmp::Ordering::Greater => Joypad::DOWN,
                    std::cmp::Ordering::Equal => Joypad::A,
                };
                self.answered = press == Joypad::A;
                press
            }
            (Mode::PartyMenu(menu), Command::SwitchPokemon(slot), Decision::PartyMenu) => {
                let press = match slot.cmp(&menu.selected()) {
                    std::cmp::Ordering::Less => Joypad::UP,
                    std::cmp::Ordering::Greater => Joypad::DOWN,
                    std::cmp::Ordering::Equal => Joypad::A,
                };
                self.chose_in_party = press == Joypad::A;
                press
            }
            (Mode::Battle(battle), command, decision) => {
                let Some(menu) = battle.menu.as_ref() else { return Drive::Done };
                let (press, answers) = match (command, decision) {
                    (Command::Fight(slot), Decision::BattleMoves) => (menu.press_toward(*slot), true),
                    (Command::Fight(_), Decision::BattleMenu) => (menu.press_toward(menus::FIGHT), false),
                    (Command::Run | Command::SwitchPokemon(_) | Command::UseItem { .. }, Decision::BattleMoves) => (Joypad::B, false),
                    (Command::UseItem { .. }, Decision::BattleMenu) => (menu.press_toward(menus::ITEM), false),
                    (Command::Run, Decision::BattleMenu) => (menu.press_toward(menus::RUN), true),
                    (Command::SwitchPokemon(_), Decision::BattleMenu) => (menu.press_toward(menus::PKMN), false),
                    (Command::SwitchPokemon(_), Decision::SwitchStatsCancel) => (menu.press_toward(0), true),
                    (Command::SafariBall, Decision::BattleMenu) => (menu.press_toward(safari::BALL), true),
                    (Command::SafariRock, Decision::BattleMenu) => (menu.press_toward(safari::ROCK), true),
                    (Command::SafariBait, Decision::BattleMenu) => (menu.press_toward(safari::BAIT), true),
                    _ => return Drive::Done,
                };
                self.answered = answers && press == Joypad::A;
                press
            }
            _ => return Drive::Done,
        };
        self.released = false;
        Drive::Press(press)
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use crate::command::Reply;
    use crate::party::Named;
    use crate::rng::GameRng;
    use crate::systems::add_mon::{new_party_mon, Origin};
    use crate::{Event, Game, Input, Pacing};
    use super::*;

    fn game(opponent: BattleMode) -> Game {
        let named = |species, level, nick| Named {
            mon: new_party_mon(species, level, 0, &Origin::Trainer, &mut GameRng::tape(vec![])),
            ot: encode("RED").unwrap(),
            nick: encode(nick).unwrap(),
        };
        let world = World {
            player_name: encode("RED").unwrap(),
            party: vec![named(PokemonSpecies::Pidgey, 50, "BIRD"), named(PokemonSpecies::Rattata, 40, "RAT")],
            ..World::default()
        };
        let mut game = Game::new(world, GameRng::seeded(7), Pacing::Faithful);
        game.push(Mode::Battle(opponent));
        game
    }

    /// Frames until something waits on the player, and what; `None` once the battle is over.
    fn settle(game: &mut Game) -> Option<Decision> {
        for _ in 0..5000 {
            if !game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))) {
                return None;
            }
            if let Status::Waiting(decision) = game.status() {
                return Some(decision);
            }
            game.frame(Input::None);
        }
        panic!("nothing ever waited: {:?}", game.modes().last().map(Mode::status));
    }

    fn command(game: &mut Game, command: Command) -> Reply {
        let reply = game.frame(Input::Command(command.clone())).reply.expect("a reply");
        if reply == Reply::Accepted {
            for _ in 0..3000 {
                if game.frame(Input::None).events.contains(&Event::CommandDone(command.clone())) {
                    return reply;
                }
            }
            panic!("{command:?} never finished");
        }
        reply
    }

    #[test]
    fn a_wild_battle_is_fought_switched_and_won_by_commands() {
        let mut game = game(BattleMode::wild(PokemonSpecies::Magikarp, 5));
        assert_eq!(settle(&mut game), Some(Decision::Text));
        command(&mut game, Command::Advance);
        assert_eq!(settle(&mut game), Some(Decision::BattleMenu));
        assert!(matches!(command(&mut game, Command::SwitchPokemon(0)), Reply::Refused(_)), "BIRD is out already");
        let moves = game.world().party[0].mon.mon.moves;
        if let Some(empty) = moves.iter().position(Option::is_none) {
            assert!(matches!(command(&mut game, Command::Fight(empty as u8)), Reply::Refused(_)), "no move there");
        }
        assert_eq!(command(&mut game, Command::SwitchPokemon(1)), Reply::Accepted);
        let exp_before = game.world().party[1].mon.mon.exp;
        let mut fights = 0;
        while let Some(decision) = settle(&mut game) {
            let next = match decision {
                Decision::Text => Command::Advance,
                Decision::BattleMenu | Decision::BattleMoves => {
                    fights += 1;
                    Command::Fight(0)
                }
                decision => panic!("the battle asked {decision:?}"),
            };
            assert_eq!(command(&mut game, next), Reply::Accepted);
            assert!(fights < 50, "the battle goes on");
        }
        assert!(fights > 0);
        assert!(game.world().party[1].mon.mon.exp > exp_before, "RAT fought and gained experience");
        assert!(game.world().party[0].mon.mon.exp > new_party_mon(PokemonSpecies::Pidgey, 50, 0, &Origin::Trainer,
            &mut GameRng::tape(vec![])).mon.exp, "BIRD fought too, and shares it");
    }

    #[test]
    fn a_safari_battle_is_played_by_rock_bait_and_ball() {
        let mut game = game(BattleMode::wild(PokemonSpecies::Rhyhorn, 25));
        let mut world = game.world().clone();
        world.location.map = poke_core::map::Map::SafariZoneCenter;
        world.safari_balls = 30;
        let mut game = Game::new(world, GameRng::seeded(7), Pacing::Faithful);
        game.push(Mode::Battle(BattleMode::wild(PokemonSpecies::Rhyhorn, 25)));
        let mut asked = vec![Command::SafariRock, Command::SafariBait];
        let mut thrown = 0;
        while let Some(decision) = settle(&mut game) {
            let next = match decision {
                Decision::Text => Command::Advance,
                Decision::TwoOption => Command::ChooseOption(1),
                Decision::PokedexData => Command::CloseDex,
                Decision::BattleMenu => {
                    assert!(matches!(command(&mut game, Command::Fight(0)), Reply::Refused(_)), "no FIGHT here");
                    let next = if asked.is_empty() { Command::SafariBall } else { asked.remove(0) };
                    thrown += (next == Command::SafariBall) as u8;
                    next
                }
                decision => panic!("the battle asked {decision:?}"),
            };
            assert_eq!(command(&mut game, next.clone()), Reply::Accepted, "{next:?} at {decision:?}");
            assert!(thrown < 30, "the battle goes on");
        }
        assert!(asked.is_empty(), "the rock and the bait were thrown");
        assert!(thrown > 0);
        assert_eq!(game.world().safari_balls, 30 - thrown, "a ball a throw");
    }

    #[test]
    fn a_potion_and_a_master_ball_are_used_by_commands() {
        use poke_core::bag::BagItem;
        use poke_core::item::ItemId;
        let game = game(BattleMode::wild(PokemonSpecies::Magikarp, 5));
        let world = game.world().clone();
        let mut hurt = world;
        hurt.party[0].mon.mon.hp = 20;
        hurt.bag = crate::systems::inventory::Inventory::bag(vec![BagItem::new(ItemId::Potion, 2), BagItem::new(ItemId::MasterBall, 1)]);
        let mut game = Game::new(hurt, GameRng::seeded(7), Pacing::Faithful);
        game.push(Mode::Battle(BattleMode::wild(PokemonSpecies::Magikarp, 5)));
        assert_eq!(settle(&mut game), Some(Decision::Text));
        command(&mut game, Command::Advance);
        assert_eq!(settle(&mut game), Some(Decision::BattleMenu));
        assert!(matches!(command(&mut game, Command::UseItem { item: ItemId::Potion, target: None }), Reply::Refused(_)));
        assert!(matches!(command(&mut game, Command::UseItem { item: ItemId::MasterBall, target: Some(0) }), Reply::Refused(_)));
        assert_eq!(command(&mut game, Command::UseItem { item: ItemId::Potion, target: Some(0) }), Reply::Accepted);
        let mut used_ball = false;
        while let Some(decision) = settle(&mut game) {
            let next = match decision {
                Decision::Text => Command::Advance,
                Decision::BattleMenu if !used_ball => {
                    used_ball = true;
                    let Some(Mode::Battle(battle)) = game.modes().iter().find(|mode| matches!(mode, Mode::Battle(_))) else { unreachable!() };
                    assert_eq!(battle.battle().unwrap().player.mon.hp, 40, "the potion reached the battle mon");
                    Command::UseItem { item: ItemId::MasterBall, target: None }
                }
                Decision::TwoOption => Command::ChooseOption(1),
                Decision::PokedexData => Command::CloseDex,
                decision => panic!("the battle asked {decision:?}"),
            };
            assert_eq!(command(&mut game, next.clone()), Reply::Accepted, "{next:?}");
        }
        assert!(used_ball);
        assert_eq!(game.world().party.len(), 3, "MAGIKARP joined the party");
        assert_eq!(game.world().party[2].nick, PokemonSpecies::Magikarp.name());
        assert!(game.world().bag.items.iter().all(|slot| slot.id != ItemId::MasterBall));
    }

    #[test]
    fn mimic_copies_the_enemy_move_chosen_by_command() {
        use poke_core::move_name::PokemonMoveName;
        let mut world = game(BattleMode::wild(PokemonSpecies::Rattata, 5)).world().clone();
        world.party[0].mon.mon.moves = [Some(PokemonMoveName::Mimic), None, None, None];
        world.party[0].mon.mon.pp = [10, 0, 0, 0];
        let mut game = Game::new(world, GameRng::seeded(7), Pacing::Faithful);
        game.push(Mode::Battle(BattleMode::wild(PokemonSpecies::Rattata, 5)));
        let mut guard = 0;
        loop {
            let next = match settle(&mut game).expect("the battle goes on") {
                Decision::Text => Command::Advance,
                Decision::BattleMenu | Decision::BattleMoves => Command::Fight(0),
                Decision::MimicMove => break,
                decision => panic!("the battle asked {decision:?}"),
            };
            assert_eq!(command(&mut game, next), Reply::Accepted);
            guard += 1;
            assert!(guard < 50, "Mimic never landed");
        }
        assert!(matches!(command(&mut game, Command::ChooseOption(2)), Reply::Refused(_)), "RATTATA knows two moves");
        assert!(matches!(command(&mut game, Command::CancelOption), Reply::Refused(_)), "B does nothing here");
        assert_eq!(command(&mut game, Command::ChooseOption(1)), Reply::Accepted);
        while settle(&mut game) == Some(Decision::Text) {
            command(&mut game, Command::Advance);
        }
        let Some(Mode::Battle(battle)) = game.modes().iter().find(|mode| matches!(mode, Mode::Battle(_))) else { unreachable!() };
        assert_eq!(battle.battle().unwrap().player.mon.moves[0], Some(PokemonMoveName::TailWhip));
        assert_eq!(game.world().party[0].mon.mon.moves[0], Some(PokemonMoveName::Mimic), "the party keeps MIMIC");
    }

    #[test]
    fn a_trainer_battle_is_won_for_its_prize_by_commands() {
        let mut game = game(BattleMode::trainer(1, 1, 0, 0));
        let mut guard = 0;
        while let Some(decision) = settle(&mut game) {
            let next = match decision {
                Decision::Text => Command::Advance,
                // WING ATTACK: a Whirlwind does nothing to a trainer.
                Decision::BattleMenu | Decision::BattleMoves => Command::Fight(1),
                Decision::TwoOption => Command::ChooseOption(1),
                decision => panic!("the battle asked {decision:?}"),
            };
            assert_eq!(command(&mut game, next.clone()), Reply::Accepted, "{next:?}");
            guard += 1;
            assert!(guard < 200, "the battle goes on");
        }
        assert_ne!(game.world().money, [0; 3], "the prize is paid");
    }

    /// Loses to `opponent` on `map` with a lone MAGIKARP and the bike forced on, and says which of
    /// `lines` the text box showed on the way.
    fn lose_on(map: poke_core::map::Map, opponent: BattleMode, lines: &[&str]) -> (Game, Vec<bool>) {
        let mut world = World {
            player_name: encode("RED").unwrap(),
            rival_name: encode("BLUE").unwrap(),
            party: vec![Named {
                mon: new_party_mon(PokemonSpecies::Magikarp, 2, 0, &Origin::Trainer, &mut GameRng::tape(vec![])),
                ot: encode("RED").unwrap(),
                nick: encode("FISH").unwrap(),
            }],
            ..World::default()
        };
        world.location.map = map;
        world.location.always_on_bike = true;
        let mut game = Game::new(world, GameRng::seeded(7), Pacing::Faithful);
        game.push(Mode::Battle(opponent));
        let mut seen = vec![false; lines.len()];
        let mut guard = 0;
        while let Some(decision) = settle(&mut game) {
            for (line, seen) in lines.iter().zip(seen.iter_mut()) {
                let words = encode(&format!("{line}@")).unwrap();
                let words = &words[..words.len() - 1];
                *seen |= game.screen().ui.row(14).windows(words.len()).any(|window| window == words);
            }
            let next = match decision {
                Decision::Text => Command::Advance,
                Decision::BattleMenu | Decision::BattleMoves => Command::Fight(0),
                decision => panic!("the battle asked {decision:?}"),
            };
            assert_eq!(command(&mut game, next.clone()), Reply::Accepted, "{next:?}");
            guard += 1;
            assert!(guard < 200, "the battle goes on");
        }
        (game, seen)
    }

    /// `HandlePlayerBlackOut`: the first rival gloats, and in Oak's lab that is the end of it.
    #[test]
    fn losing_to_the_rival_in_oaks_lab_is_no_blackout() {
        let (game, seen) = lose_on(poke_core::map::Map::OaksLab, BattleMode::trainer(0x19, 1, 0, 0), &["Yeah! Am", "is out of"]);
        assert_eq!(seen, [true, false]);
        assert!(game.world().location.always_on_bike, "only a blackout gets off the bike");
    }

    /// The first rival on Route 22 gloats and the player blacks out too.
    #[test]
    fn losing_to_the_rival_on_route_22_blacks_out_after_his_gloat() {
        let (game, seen) = lose_on(poke_core::map::Map::Route22, BattleMode::trainer(0x19, 1, 0, 0), &["Yeah! Am", "is out of"]);
        assert_eq!(seen, [true, true]);
        assert!(!game.world().location.always_on_bike, "a blackout gets off the bike");
    }

    #[test]
    fn losing_to_a_trainer_blacks_out_and_gets_off_the_bike() {
        let (game, seen) = lose_on(poke_core::map::Map::Route17, BattleMode::trainer(1, 1, 0, 0), &["Yeah! Am", "is out of"]);
        assert_eq!(seen, [false, true]);
        assert!(!game.world().location.always_on_bike);
        assert_eq!(game.screen().sgb.palette_ids(), [crate::gfx::sgb::PAL_BLACK; 4], "`HandlePlayerBlackOut`'s black");
    }

    /// The four `SuperPalettes` ids `SetPal_Battle` sends for these bars and species indices.
    fn battle_palettes(bars: [HpBarColour; 2], player: u8, enemy: u8) -> [u8; 4] {
        use crate::gfx::sgb::{determine_palette_id_out_of_battle as id, PAL_GREENBAR};
        [PAL_GREENBAR + bars[0] as u8, PAL_GREENBAR + bars[1] as u8, id(player), id(enemy)]
    }

    use crate::systems::hp_bar::HpBarColour::{self, Green, Red, Yellow};

    /// Each change of the SGB's palettes while the battle runs up to the next thing it waits on.
    fn settle_recording(game: &mut Game, seen: &mut Vec<[u8; 4]>) -> Option<Decision> {
        for _ in 0..5000 {
            if !game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))) {
                return None;
            }
            if let Status::Waiting(decision) = game.status() {
                return Some(decision);
            }
            game.frame(Input::None);
            let ids = game.screen().sgb.palette_ids();
            if seen.last() != Some(&ids) {
                seen.push(ids);
            }
        }
        panic!("nothing ever waited");
    }

    fn command_recording(game: &mut Game, command: Command, seen: &mut Vec<[u8; 4]>) {
        assert_eq!(game.frame(Input::Command(command.clone())).reply, Some(Reply::Accepted), "{command:?}");
        for _ in 0..3000 {
            let done = game.frame(Input::None).events.contains(&Event::CommandDone(command.clone()));
            let ids = game.screen().sgb.palette_ids();
            if seen.last() != Some(&ids) {
                seen.push(ids);
            }
            if done {
                return;
            }
        }
        panic!("{command:?} never finished");
    }

    /// `_InitBattleCommon` sends black before the slide and the battle's own packet after it, with
    /// no mon of the player's loaded yet; after that a HUD sends it again when its bar is a new
    /// colour, and `SendOutMon` for the mon.
    #[test]
    fn a_battle_opens_black_then_colours_each_mon_and_each_bar() {
        use crate::gfx::sgb::{PaletteCommand, SgbState};
        let mut world = game(BattleMode::wild(PokemonSpecies::Magikarp, 5)).world().clone();
        let bird = &mut world.party[0].mon;
        bird.mon.hp = bird.stats[0] * 2 / 5;
        let rat = &mut world.party[1].mon;
        rat.mon.hp = rat.stats[0] / 10;
        let mut game = Game::new(world, GameRng::seeded(7), Pacing::Faithful);
        game.push(Mode::Battle(BattleMode::wild(PokemonSpecies::Magikarp, 5)));
        let mut black = SgbState::default();
        black.run(&PaletteCommand::BattleBlack);
        let (bird, rat, karp) = (PokemonSpecies::Pidgey as u8, PokemonSpecies::Rattata as u8, PokemonSpecies::Magikarp as u8);

        let mut seen = vec![game.screen().sgb.palette_ids()];
        assert_eq!(settle_recording(&mut game, &mut seen), Some(Decision::Text));
        assert_eq!(seen[1..], [black.palette_ids(), battle_palettes([Green, Green], 0, karp)],
            "`wBattleMonSpecies` is still zero when the slide ends");

        // The HUD is drawn before `SendOutMon`'s own packet, and already has the mon.
        let mut seen = vec![game.screen().sgb.palette_ids()];
        command_recording(&mut game, Command::Advance, &mut seen);
        assert_eq!(settle_recording(&mut game, &mut seen), Some(Decision::BattleMenu));
        assert_eq!(seen, [battle_palettes([Green, Green], 0, karp), battle_palettes([Yellow, Green], bird, karp)]);

        let mut seen = vec![game.screen().sgb.palette_ids()];
        command_recording(&mut game, Command::SwitchPokemon(1), &mut seen);
        while settle_recording(&mut game, &mut seen) == Some(Decision::Text) {
            command_recording(&mut game, Command::Advance, &mut seen);
        }
        let mut party_menu = SgbState::default();
        party_menu.run(&PaletteCommand::PartyMenu);
        assert_eq!(seen, [
            battle_palettes([Yellow, Green], bird, karp),
            party_menu.palette_ids(),
            battle_palettes([Yellow, Green], bird, karp),
            battle_palettes([Red, Green], rat, karp),
        ], "`.notAlreadyOut` puts the battle back for the mon still out, and `SendOutMon`'s HUD brings the next");
    }

    /// Plays one frame with `buttons` held and one with them let go, recording the palettes.
    fn press_recording(game: &mut Game, buttons: crate::input::Joypad, seen: &mut Vec<[u8; 4]>) {
        for input in [Input::Buttons(buttons), Input::None] {
            game.frame(input);
            let ids = game.screen().sgb.palette_ids();
            if seen.last() != Some(&ids) {
                seen.push(ids);
            }
        }
    }

    /// `ItemUsePPRestore` from the battle's bag on BIRD, whose moves hold `pp`: the item on the first
    /// move, and back out of the bag it reopens onto. The palettes from the battle menu on, and the
    /// game.
    fn restore_pp_in_battle(item: poke_core::item::ItemId, pp: [u8; 4]) -> (Vec<[u8; 4]>, Game) {
        use poke_core::bag::BagItem;
        let mut world = game(BattleMode::wild(PokemonSpecies::Magikarp, 5)).world().clone();
        world.party[0].mon.mon.pp = pp;
        world.bag = crate::systems::inventory::Inventory::bag(vec![BagItem::new(item, 1)]);
        let mut game = Game::new(world, GameRng::seeded(7), Pacing::Faithful);
        game.push(Mode::Battle(BattleMode::wild(PokemonSpecies::Magikarp, 5)));
        assert_eq!(settle(&mut game), Some(Decision::Text));
        command(&mut game, Command::Advance);
        assert_eq!(settle(&mut game), Some(Decision::BattleMenu));
        let mut seen = vec![game.screen().sgb.palette_ids()];
        command_recording(&mut game, Command::UseItem { item, target: Some(0) }, &mut seen);
        let mut guard = 0;
        loop {
            match settle_recording(&mut game, &mut seen).expect("the battle goes on") {
                Decision::MoveMenu => press_recording(&mut game, crate::input::Joypad::A, &mut seen),
                Decision::Text => command_recording(&mut game, Command::Advance, &mut seen),
                Decision::List => press_recording(&mut game, crate::input::Joypad::B, &mut seen),
                Decision::BattleMenu => break,
                decision => panic!("the item asked {decision:?}"),
            }
            guard += 1;
            assert!(guard < 20, "the item never let go");
        }
        (seen, game)
    }

    /// `ItemUsePPRestore`'s `.restorePP` returns with the zero flag set for a move already full, to
    /// `.useEther`'s `jp .noEffect`, which falls into `.itemNotUsed` and its `RunDefaultPaletteCommand`;
    /// a Max Ether on a full move with PP Ups used takes `.done`'s. Either way the party menu's
    /// palette gives way to the battle's before the bag reopens.
    #[test]
    fn an_ether_that_does_nothing_puts_the_battle_palette_back_as_one_that_restores_does() {
        use poke_core::item::ItemId;
        use crate::gfx::sgb::{PaletteCommand, SgbState};
        use crate::systems::pp::max_pp;
        let bird = game(BattleMode::wild(PokemonSpecies::Magikarp, 5)).world().party[0].mon.mon.clone();
        let full = bird.pp;
        let first = bird.moves[0].expect("a first move");
        let pp_upped = 3 << 6 | max_pp(first, 3 << 6);
        let mut party_menu = SgbState::default();
        party_menu.run(&PaletteCommand::PartyMenu);
        let battle = battle_palettes([Green, Green], PokemonSpecies::Pidgey as u8, PokemonSpecies::Magikarp as u8);
        for (item, pp, used) in [
            (ItemId::Ether, full, false),
            (ItemId::MaxEther, full, false),
            (ItemId::Ether, [pp_upped, full[1], full[2], full[3]], false),
            // The Max Ether compares the PP Ups with the PP, so the move never reads as full.
            (ItemId::MaxEther, [pp_upped, full[1], full[2], full[3]], true),
            (ItemId::Ether, [0, full[1], full[2], full[3]], true),
        ] {
            let (seen, game) = restore_pp_in_battle(item, pp);
            let what = format!("{item:?} on PP {:#04X}", pp[0]);
            let menu = seen.iter().position(|&ids| ids == party_menu.palette_ids()).unwrap_or_else(|| panic!("{what}: the party menu"));
            assert_eq!(seen[menu + 1..], [battle], "{what}: the battle's palette straight after the party menu");
            assert_eq!(game.world().bag.items.is_empty(), used, "{what}: used");
        }
    }

    /// `TransformEffect_` sets `TRANSFORMED` only after the animation, so the `SET_PAL_BATTLE` that
    /// `ChangeMonPic` sends still has the mon's own palette, and the grey comes with the next packet:
    /// here `.notAlreadyOut`'s, putting the battle back behind the party menu.
    #[test]
    fn a_transformed_mon_turns_grey_at_the_next_set_pal_battle_not_at_its_own_picture() {
        use poke_core::move_name::PokemonMoveName;
        use crate::gfx::sgb::{determine_palette_id_out_of_battle as id, PaletteCommand, SgbState, PAL_GRAYMON};
        use crate::systems::battle::Status3;
        let (mew, karp) = (PokemonSpecies::Mew as u8, PokemonSpecies::Magikarp as u8);
        assert_ne!(id(mew), PAL_GRAYMON, "a mon that is not grey already");
        let mut party_menu = SgbState::default();
        party_menu.run(&PaletteCommand::PartyMenu);
        for animations in [true, false] {
            let mut world = game(BattleMode::wild(PokemonSpecies::Magikarp, 5)).world().clone();
            world.options.battle_animation = animations;
            world.party[0].mon = new_party_mon(PokemonSpecies::Mew, 30, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
            world.party[0].mon.mon.moves = [Some(PokemonMoveName::Transform), None, None, None];
            world.party[0].mon.mon.pp = [10, 0, 0, 0];
            let mut game = Game::new(world, GameRng::seeded(7), Pacing::Faithful);
            game.push(Mode::Battle(BattleMode::wild(PokemonSpecies::Magikarp, 5)));
            assert_eq!(settle(&mut game), Some(Decision::Text));
            command(&mut game, Command::Advance);
            assert_eq!(settle(&mut game), Some(Decision::BattleMenu));
            let mut seen = vec![game.screen().sgb.palette_ids()];
            command_recording(&mut game, Command::Fight(0), &mut seen);
            while settle_recording(&mut game, &mut seen) == Some(Decision::Text) {
                command_recording(&mut game, Command::Advance, &mut seen);
            }
            let Some(Mode::Battle(battle)) = game.modes().last() else { panic!("the battle is up") };
            assert!(battle.battle().unwrap().player.status3.contains(Status3::TRANSFORMED), "MEW transformed");
            assert!(seen.iter().all(|ids| ids[2] == id(mew)), "animations {animations}: MEW kept its palette through its turn: {seen:?}");

            let mut seen = vec![game.screen().sgb.palette_ids()];
            command_recording(&mut game, Command::SwitchPokemon(1), &mut seen);
            let menu = seen.iter().position(|&ids| ids == party_menu.palette_ids()).expect("the party menu");
            assert_eq!(seen[menu + 1][2..], [PAL_GRAYMON, id(karp)], "animations {animations}: the grey at the next packet");
        }
    }

    /// A fainted enemy's bar goes red, `ReplaceFaintedEnemyMon` puts it back to green before the
    /// next mon comes out in its own colours, and the trainer's pic scrolls back in as species 0.
    #[test]
    fn a_trainers_mons_are_coloured_as_each_comes_out() {
        let mut game = game(BattleMode::trainer(1, 1, 0, 0));
        let mut seen = vec![];
        let mut guard = 0;
        while let Some(decision) = settle_recording(&mut game, &mut seen) {
            let next = match decision {
                Decision::Text => Command::Advance,
                Decision::BattleMenu | Decision::BattleMoves => Command::Fight(1),
                Decision::TwoOption => Command::ChooseOption(1),
                decision => panic!("the battle asked {decision:?}"),
            };
            command_recording(&mut game, next, &mut seen);
            guard += 1;
            assert!(guard < 200, "the battle goes on");
        }
        let (bird, rattata, ekans) = (PokemonSpecies::Pidgey as u8, PokemonSpecies::Rattata as u8, PokemonSpecies::Ekans as u8);
        let battle = seen.iter().position(|ids| *ids == battle_palettes([Green, Green], 0, 0)).expect("the opening packet");
        assert_eq!(seen[battle..], [
            battle_palettes([Green, Green], 0, 0),
            battle_palettes([Green, Green], 0, rattata),
            battle_palettes([Green, Green], bird, rattata),
            battle_palettes([Green, Red], bird, rattata),
            battle_palettes([Green, Green], bird, rattata),
            battle_palettes([Green, Green], bird, ekans),
            battle_palettes([Green, Red], bird, ekans),
            battle_palettes([Green, Red], bird, 0),
        ]);
    }

    /// `HandlePlayerBlackOut` sends black only once the Oak's lab rival has had his say, and there
    /// not at all.
    #[test]
    fn losing_in_oaks_lab_leaves_the_rival_in_colour() {
        let (game, _) = lose_on(poke_core::map::Map::OaksLab, BattleMode::trainer(0x19, 1, 0, 0), &[]);
        let ids = game.screen().sgb.palette_ids();
        assert_eq!(ids[3], crate::gfx::sgb::determine_palette_id_out_of_battle(0), "the rival's pic, species 0");
        assert_ne!(ids, [crate::gfx::sgb::PAL_BLACK; 4]);
    }

    /// The overworld's `LoadMapData` puts the map's palette back once the battle is over.
    #[test]
    fn the_map_takes_its_own_palette_back_after_a_battle() {
        use poke_core::map::Map;
        use poke_core::sprite::SpriteFacing;
        use poke_core::symbols::pokered_events::EVENT_BEAT_ARTICUNO;
        use crate::gfx::sgb::PAL_CAVE;
        use crate::modes::overworld::Overworld;
        use crate::systems::overworld::Location;
        let mut world = game(BattleMode::wild(PokemonSpecies::Magikarp, 5)).world().clone();
        world.location = Location { map: Map::SeafoamIslandsB4F, x: 6, y: 2, facing: SpriteFacing::Up, last_map: Map::PalletTown, ..Location::default() };
        world.party = vec![Named {
            mon: new_party_mon(PokemonSpecies::Mewtwo, 70, 1, &Origin::Trainer, &mut GameRng::tape(vec![])),
            ot: encode("RED").unwrap(),
            nick: encode("MON").unwrap(),
        }];
        world.party[0].mon.mon.moves[0] = Some(poke_core::move_name::PokemonMoveName::Psychic);
        let mut game = Game::new(world, GameRng::seeded(5), Pacing::Faithful);
        game.push(Mode::Overworld(Overworld::new()));
        let cave = [PAL_CAVE, 0, 0, 0];
        let mut in_battle = false;
        let mut coloured_in_battle = false;
        for _ in 0..80_000 {
            let battling = game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_)));
            in_battle |= battling;
            coloured_in_battle |= battling && game.screen().sgb.palette_ids() != cave;
            if game.world().events.is_set(EVENT_BEAT_ARTICUNO) && game.status() == Status::Waiting(Decision::Overworld) {
                break;
            }
            let input = match game.status() {
                Status::Waiting(Decision::Overworld) if !in_battle => Input::Command(Command::Interact),
                Status::Waiting(Decision::Text) => Input::Command(Command::Advance),
                Status::Waiting(Decision::BattleMenu | Decision::BattleMoves) => Input::Command(Command::Fight(0)),
                Status::Waiting(Decision::TwoOption) => Input::Command(Command::ChooseOption(1)),
                _ => Input::None,
            };
            game.frame(input);
        }
        assert!(game.world().events.is_set(EVENT_BEAT_ARTICUNO), "Articuno was beaten");
        assert!(coloured_in_battle);
        assert_eq!(game.screen().sgb.palette_ids(), cave);
    }
}
