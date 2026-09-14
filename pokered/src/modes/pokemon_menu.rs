//! `StartMenu_Pokemon`: the party menu the start menu's `POKéMON` row opens, the submenu a chosen
//! mon opens over it, and the screens its rows lead to.
//!
//! `STATS` runs both status screen pages and then opens the party menu again from the top, cleared
//! and redrawn. `SWITCH` marks the mon and reopens the list over itself; with a party of one it too
//! starts again from the top. B on the submenu goes back to the list without clearing it, and
//! `CANCEL` or B on the list goes back to the start menu in the same frame: the delays of
//! `GBPalWhiteOutWithDelay3` and `RestoreScreenTilesAndReloadTilePatterns` are loading.
//!
//! The field moves are the overworld's: choosing one answers with the move, from [`PokemonMenu::
//! field_move`], and pops back to the start menu where the cartridge would carry it out.

use poke_core::move_name::PokemonMoveName;
use serde::{Deserialize, Serialize};
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::field_move_menu::{FieldMoveChoice, FieldMoveMenu};
use crate::modes::party_menu::{PartyMenu, PartyMenuType};
use crate::modes::status_screen::StatusScreen;
use crate::systems::field_moves::{field_moves, FieldMoves};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PokemonMenu {
    phase: Phase,
    /// `wWhichPokemon`.
    slot: u8,
    /// What `GetMonFieldMoves` found for that mon, which gives the submenu's rows their meaning.
    moves: FieldMoves,
    field_move: Option<PokemonMoveName>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    #[default]
    PartyMenu,
    FieldMoveMenu,
    StatusScreen,
}

impl PokemonMenu {
    pub fn new() -> Self {
        Self::default()
    }

    /// The field move chosen, for the overworld to carry out.
    pub fn field_move(&self) -> Option<PokemonMoveName> {
        self.field_move
    }

    /// `DisplayPartyMenu`, from the top of `StartMenu_Pokemon`.
    fn from_the_top(&mut self) -> Transition {
        self.phase = Phase::PartyMenu;
        Transition::Push(Mode::PartyMenu(PartyMenu::new(PartyMenuType::Normal)))
    }

    /// `.exitMenu`.
    fn leave(&mut self) -> Transition {
        Transition::Pop(Outcome::Done)
    }

    fn chose(&mut self, row: u8, ctx: &mut Ctx) -> Transition {
        match FieldMoveMenu::new(self.moves.clone()).choice(row) {
            FieldMoveChoice::Cancel => self.leave(),
            // `.choseSwitch` refuses a party of one by starting the whole menu over.
            FieldMoveChoice::Switch if ctx.world.party.len() < 2 => self.from_the_top(),
            FieldMoveChoice::Switch => {
                self.phase = Phase::PartyMenu;
                Transition::Push(Mode::PartyMenu(PartyMenu::swapping(self.slot)))
            }
            FieldMoveChoice::Stats => {
                self.phase = Phase::StatusScreen;
                let mon = ctx.world.party[self.slot as usize].clone();
                Transition::Push(Mode::StatusScreen(StatusScreen::new(mon)))
            }
            FieldMoveChoice::Move(field_move) => {
                self.field_move = Some(field_move);
                Transition::Pop(Outcome::Chosen(field_move as u8))
            }
        }
    }
}

impl ModeUpdate for PokemonMenu {
    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        if ctx.world.party.is_empty() {
            return Transition::Pop(Outcome::Done);
        }
        self.from_the_top()
    }

    fn update(&mut self, _ctx: &mut Ctx) -> Transition {
        Transition::Stay
    }

    fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        match (self.phase, outcome) {
            (Phase::PartyMenu, Outcome::Chosen(slot)) => {
                self.slot = slot;
                let known = ctx.world.party[slot as usize].mon.mon.moves;
                self.moves = field_moves(known.map(|one| one.map_or(0, |one| one as u8)));
                self.phase = Phase::FieldMoveMenu;
                Transition::Push(Mode::FieldMoveMenu(FieldMoveMenu::new(self.moves.clone())))
            }
            (Phase::PartyMenu, _) => self.leave(),
            (Phase::FieldMoveMenu, Outcome::Chosen(row)) => self.chose(row, ctx),
            // `.loop`: `GoBackToPartyMenu`, over the list that is still on screen.
            (Phase::FieldMoveMenu, _) => {
                self.phase = Phase::PartyMenu;
                Transition::Push(Mode::PartyMenu(PartyMenu::again(PartyMenuType::Normal)))
            }
            // `ReloadMapData`, then `StartMenu_Pokemon` again.
            (Phase::StatusScreen, _) => {
                ctx.screen.tiles.load_text_box_tiles();
                if let Some(tileset) = ctx.screen.map.tileset {
                    ctx.screen.tiles.load_tileset(tileset);
                }
                self.from_the_top()
            }
        }
    }

    fn status(&self) -> Status {
        Status::Busy
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use poke_core::species::PokemonSpecies;
    use crate::command::{Command, Decision};
    use crate::input::Joypad;
    use crate::modes::start_menu::{StartMenu, StartMenuEntry};
    use crate::party::{Named, PartyMon};
    use crate::rng::GameRng;
    use crate::systems::add_mon::{new_party_mon, Origin};
    use crate::world::World;
    use crate::{Game, Input, Pacing};
    use super::*;

    fn mon(species: PokemonSpecies, nick: &str) -> Named<PartyMon> {
        let mon = new_party_mon(species, 20, 1, &Origin::Trainer, &mut GameRng::tape(vec![]));
        Named { mon, ot: encode("RED").unwrap(), nick: encode(nick).unwrap() }
    }

    fn game(party: Vec<Named<PartyMon>>) -> Game {
        let world = World { party, player_name: encode("RED").unwrap(), ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::StartMenu(StartMenu::new()));
        game
    }

    fn two() -> Vec<Named<PartyMon>> {
        vec![mon(PokemonSpecies::Pidgey, "BIRD"), mon(PokemonSpecies::Rattata, "RAT")]
    }

    fn until(game: &mut Game, decision: Decision) {
        for _ in 0..600 {
            if game.status() == Status::Waiting(decision.clone()) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("never waited for {decision:?}, stuck at {:?}", game.status());
    }

    fn press(game: &mut Game, button: Joypad) {
        game.frame(Input::Buttons(button));
        game.frame(Input::None);
    }

    fn command(game: &mut Game, command: Command) {
        let reply = game.frame(Input::Command(command.clone())).reply;
        assert_eq!(reply, Some(crate::command::Reply::Accepted), "{command:?}");
        for _ in 0..600 {
            if game.frame(Input::None).events.iter().any(|event| matches!(event, crate::Event::CommandDone(_))) {
                return;
            }
        }
        panic!("{command:?} never finished");
    }

    fn top(game: &Game) -> &Mode {
        game.modes().last().expect("a mode")
    }

    #[test]
    fn an_empty_party_goes_straight_back_to_the_start_menu() {
        let mut game = game(vec![]);
        until(&mut game, Decision::StartMenu);
        press(&mut game, Joypad::A);
        until(&mut game, Decision::StartMenu);
        assert_eq!(game.modes().len(), 1);
    }

    #[test]
    fn a_chosen_mon_opens_the_submenu_and_b_goes_back_to_the_list() {
        let mut game = game(two());
        until(&mut game, Decision::StartMenu);
        press(&mut game, Joypad::A);
        until(&mut game, Decision::PartyMenu);
        press(&mut game, Joypad::A);
        until(&mut game, Decision::FieldMoveMenu);
        press(&mut game, Joypad::B);
        until(&mut game, Decision::PartyMenu);
        assert!(matches!(top(&game), Mode::PartyMenu(_)));
        assert_eq!(game.ui().row(0)[3..7], encode("BIRD").unwrap()[..], "the list is still there");
    }

    #[test]
    fn stats_shows_both_pages_and_then_the_list_from_the_top() {
        let mut game = game(two());
        until(&mut game, Decision::StartMenu);
        command(&mut game, Command::ChooseStartMenuEntry(StartMenuEntry::Pokemon));
        until(&mut game, Decision::PartyMenu);
        command(&mut game, Command::ChooseOption(1));
        until(&mut game, Decision::FieldMoveMenu);
        command(&mut game, Command::ChooseOption(0));
        until(&mut game, Decision::StatusScreen);
        assert_eq!(game.ui().row(1)[9..12], encode("RAT").unwrap()[..], "the second mon's page");
        command(&mut game, Command::Advance);
        until(&mut game, Decision::StatusScreen);
        command(&mut game, Command::Advance);
        until(&mut game, Decision::PartyMenu);
        assert!(matches!(game.modes(), [Mode::StartMenu(_), Mode::PokemonMenu(_), Mode::PartyMenu(_)]));
    }

    #[test]
    fn switch_marks_the_mon_and_the_next_choice_swaps_it() {
        let mut game = game(two());
        until(&mut game, Decision::StartMenu);
        press(&mut game, Joypad::A);
        until(&mut game, Decision::PartyMenu);
        press(&mut game, Joypad::A);
        until(&mut game, Decision::FieldMoveMenu);
        command(&mut game, Command::ChooseOption(1));
        until(&mut game, Decision::PartyMenu);
        press(&mut game, Joypad::DOWN);
        until(&mut game, Decision::PartyMenu);
        press(&mut game, Joypad::A);
        until(&mut game, Decision::PartyMenu);
        assert_eq!(game.world().party[0].nick, encode("RAT").unwrap());
    }

    #[test]
    fn switch_with_one_mon_starts_the_menu_over() {
        let mut game = game(vec![mon(PokemonSpecies::Pidgey, "BIRD")]);
        until(&mut game, Decision::StartMenu);
        press(&mut game, Joypad::A);
        until(&mut game, Decision::PartyMenu);
        press(&mut game, Joypad::A);
        until(&mut game, Decision::FieldMoveMenu);
        command(&mut game, Command::ChooseOption(1));
        until(&mut game, Decision::PartyMenu);
        match top(&game) {
            Mode::PartyMenu(menu) => assert_eq!(menu.armed(), None, "nothing is marked"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn cancel_and_b_both_return_to_the_start_menu() {
        let mut game = game(two());
        until(&mut game, Decision::StartMenu);
        press(&mut game, Joypad::A);
        until(&mut game, Decision::PartyMenu);
        press(&mut game, Joypad::A);
        until(&mut game, Decision::FieldMoveMenu);
        command(&mut game, Command::ChooseOption(2));
        until(&mut game, Decision::StartMenu);
        assert_eq!(game.modes().len(), 1);

        press(&mut game, Joypad::A);
        until(&mut game, Decision::PartyMenu);
        press(&mut game, Joypad::B);
        until(&mut game, Decision::StartMenu);
        assert_eq!(game.modes().len(), 1);
    }

    #[test]
    fn a_field_move_is_answered_for_the_overworld() {
        let mut party = two();
        party[0].mon.mon.moves[0] = Some(PokemonMoveName::Fly);
        let mut game = game(party);
        until(&mut game, Decision::StartMenu);
        press(&mut game, Joypad::A);
        until(&mut game, Decision::PartyMenu);
        press(&mut game, Joypad::A);
        until(&mut game, Decision::FieldMoveMenu);
        press(&mut game, Joypad::A);
        until(&mut game, Decision::StartMenu);
    }

    #[test]
    fn a_save_mid_flow_resumes_identically() {
        let mut whole = game(two());
        until(&mut whole, Decision::StartMenu);
        press(&mut whole, Joypad::A);
        until(&mut whole, Decision::PartyMenu);
        press(&mut whole, Joypad::A);
        until(&mut whole, Decision::FieldMoveMenu);
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..200 {
            let input = || match frame {
                1 => Input::Buttons(Joypad::A),
                100 => Input::Buttons(Joypad::A),
                _ => Input::None,
            };
            let (a, b) = (whole.frame(input()), restored.frame(input()));
            assert_eq!((whole.ui(), a.events, a.status), (restored.ui(), b.events, b.status), "frame {frame}");
        }
    }
}
