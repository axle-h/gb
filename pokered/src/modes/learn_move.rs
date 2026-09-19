//! `LearnMove`: a party mon is offered a move. With a slot free the move goes straight in; with four
//! known the player is asked whether to delete one, which of the four, and on backing out of
//! either whether to abandon learning altogether, which asks the whole question again on NO.
//!
//! The new move always gets the move's full PP, with any PP Ups the forgotten move had dropped. An
//! HM is refused with its own text and the list asked again. What was on screen when `LearnMove`
//! started (`SaveScreenTilesToBuffer1`) is put back the moment the list returns.
//!
//! The entry points are [`LearnMove::new`] for a move offered outright (a TM or HM) and
//! [`LearnMove::from_level_up`] for `LearnMoveFromLevelUp`. It answers `Outcome::Chosen(1)` when
//! the move was learned and `Chosen(0)` when not, the cartridge's `b`. In a battle the caller copies
//! the moves and PP to its battle mon when the mon is the one out.

use poke_core::move_name::PokemonMoveName;
use poke_core::moves::MoveData;
use poke_core::symbols::{pokered_symbols, DmgPointer};
use poke_core::text_script::{decode, TextBuffer, TextCommand};
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::command::Decision;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::menu_input::MenuInput;
use crate::modes::text_box::TextBox;
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::systems::evos_moves::level_up_move;
use crate::systems::learn_move::{format_moves_string, is_move_hm};
use crate::systems::status_screen::place_lines;
use crate::world::World;

/// `hlcoord 14, 7`, where both of its yes/no menus go.
const YES_NO_AT: (usize, usize) = (14, 7);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnMove {
    /// `wWhichPokemon`.
    slot: u8,
    /// `wMoveNum`.
    learning: PokemonMoveName,
    phase: Phase,
    /// `wTileMapBackup`, saved as `LearnMove` starts.
    saved: Option<UiSurface>,
    input: MenuInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Starting,
    TryingToLearn,
    AskingToDelete,
    WhichMoveToForget,
    ChoosingMove,
    HmCantDelete,
    /// The slot the move goes into, while `1, 2 and... Poof!` prints.
    Forgetting(u8),
    Learned,
    AbandonLearning,
    AskingToAbandon,
    DidNotLearn,
}

fn script(at: DmgPointer) -> Vec<TextCommand> {
    decode(at).expect("LearnMove's texts are in the cartridge")
}

impl LearnMove {
    pub fn new(slot: u8, learning: PokemonMoveName) -> Self {
        let input = MenuInput::new(0, 0, (5, 8), Joypad::A | Joypad::B);
        Self { slot, learning, phase: Phase::Starting, saved: None, input }
    }

    /// `LearnMoveFromLevelUp`: the move the mon's species learns at `level`, if it does not know it.
    pub fn from_level_up(world: &World, slot: u8, level: u8) -> Option<Self> {
        let mon = &world.party[slot as usize].mon.mon;
        level_up_move(mon.species, level, &mon.moves).map(|learning| Self::new(slot, learning))
    }

    /// The row under the cursor on the list of moves.
    pub fn selected(&self) -> u8 {
        self.input.current
    }

    /// Rows on the list of moves.
    pub fn rows(&self) -> u8 {
        self.input.max + 1
    }

    fn text(&mut self, phase: Phase, commands: Vec<TextCommand>) -> Transition {
        self.phase = phase;
        Transition::Push(Mode::TextBox(TextBox::script(commands)))
    }

    fn yes_no(&mut self, phase: Phase) -> Transition {
        self.phase = phase;
        Transition::Push(Mode::TwoOptionMenu(TwoOptionMenu::new(TwoOptionMenuId::YesNo, YES_NO_AT, false)))
    }

    /// `DontAbandonLearning`: a free slot takes the move, and four known moves ask.
    fn dont_abandon(&mut self, ctx: &mut Ctx) -> Transition {
        let moves = &ctx.world.party[self.slot as usize].mon.mon.moves;
        match moves.iter().position(Option::is_none) {
            Some(free) => self.learn(free as u8, ctx),
            None => self.text(Phase::TryingToLearn, script(pokered_symbols::TryingToLearnText)),
        }
    }

    /// `.next` and `PrintLearnedMove`.
    fn learn(&mut self, slot: u8, ctx: &mut Ctx) -> Transition {
        let mon = &mut ctx.world.party[self.slot as usize].mon.mon;
        mon.moves[slot as usize] = Some(self.learning);
        mon.pp[slot as usize] = MoveData::of_move(self.learning).pp;
        self.text(Phase::Learned, script(pokered_symbols::LearnedMove1Text))
    }

    /// `AbandonLearning`.
    fn abandon(&mut self) -> Transition {
        self.text(Phase::AbandonLearning, script(pokered_symbols::AbandonLearningText))
    }

    /// `TryingToLearn`'s `.loop`, after `WhichMoveToForgetText`: the four names single spaced in a
    /// box, with the cursor beside them.
    fn show_moves(&mut self, ctx: &mut Ctx) -> Transition {
        let moves = ctx.world.party[self.slot as usize].mon.mon.moves;
        let string = format_moves_string(&moves);
        ctx.screen.ui.text_box_border(4, 7, 14, 4);
        place_lines(&mut ctx.screen.ui, 8 * SCREEN_TILES_X + 6, &string.string, true);
        self.input = MenuInput::new(0, string.num_moves_minus_one.unwrap_or(0), (5, 8), Joypad::A | Joypad::B);
        self.input.single_spaced = true;
        ctx.menu.last_item = 0;
        self.input.call(ctx);
        self.phase = Phase::ChoosingMove;
        self.update(ctx)
    }

    fn chose_move(&mut self, keys: Joypad, ctx: &mut Ctx) -> Transition {
        if let Some(saved) = self.saved.clone() {
            ctx.screen.ui = saved;
        }
        if keys.contains(Joypad::B) {
            return self.abandon();
        }
        let row = self.input.current;
        let forgotten = ctx.world.party[self.slot as usize].mon.mon.moves[row as usize]
            .expect("the list holds only moves the mon knows");
        if is_move_hm(forgotten) {
            return self.text(Phase::HmCantDelete, script(pokered_symbols::HMCantDeleteText));
        }
        ctx.world.text.strings.insert(TextBuffer::NameBuffer, forgotten.name());
        // `OneTwoAndText` ends in a `text_asm` that plays `SFX_SWAP` and carries on with `PoofText`,
        // which runs on into `ForgotAndText`.
        let mut commands = script(pokered_symbols::OneTwoAndText);
        commands.extend(script(pokered_symbols::PoofText));
        self.phase = Phase::Forgetting(row);
        Transition::Push(Mode::TextBox(TextBox::script(commands).with_asm_sound(sounds::SFX_SWAP)))
    }
}

impl ModeUpdate for LearnMove {
    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        self.saved = Some(ctx.screen.ui.clone());
        let named = &ctx.world.party[self.slot as usize];
        let nick = named.nick.clone();
        ctx.world.text.strings.insert(TextBuffer::LearnMoveMonName, nick);
        ctx.world.text.strings.insert(TextBuffer::StringBuffer, self.learning.name());
        self.dont_abandon(ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        if self.phase != Phase::ChoosingMove {
            return Transition::Stay;
        }
        match self.input.update(ctx) {
            Some(keys) => self.chose_move(keys, ctx),
            None => Transition::Stay,
        }
    }

    fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        let chose_yes = outcome == Outcome::Chosen(0);
        match self.phase {
            Phase::TryingToLearn => self.yes_no(Phase::AskingToDelete),
            Phase::AskingToDelete if chose_yes =>
                self.text(Phase::WhichMoveToForget, script(pokered_symbols::WhichMoveToForgetText)),
            Phase::AskingToDelete => self.abandon(),
            Phase::WhichMoveToForget => self.show_moves(ctx),
            Phase::HmCantDelete =>
                self.text(Phase::WhichMoveToForget, script(pokered_symbols::WhichMoveToForgetText)),
            Phase::Forgetting(row) => self.learn(row, ctx),
            Phase::Learned => Transition::Pop(Outcome::Chosen(1)),
            Phase::AbandonLearning => self.yes_no(Phase::AskingToAbandon),
            Phase::AskingToAbandon if chose_yes =>
                self.text(Phase::DidNotLearn, script(pokered_symbols::DidNotLearnText)),
            Phase::AskingToAbandon => self.dont_abandon(ctx),
            Phase::DidNotLearn => Transition::Pop(Outcome::Chosen(0)),
            Phase::Starting | Phase::ChoosingMove => Transition::Stay,
        }
    }

    fn status(&self) -> Status {
        if self.phase == Phase::ChoosingMove && self.input.is_polling() {
            Status::Waiting(Decision::ForgetMove)
        } else {
            Status::Busy
        }
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use poke_core::species::PokemonSpecies;
    use crate::command::{Command, Reply};
    use crate::party::{Named, PartyMon};
    use crate::rng::GameRng;
    use crate::systems::add_mon::{new_party_mon, Origin};
    use crate::{Game, Input, Pacing};
    use super::*;
    use PokemonMoveName::*;

    fn pidgey(moves: [Option<PokemonMoveName>; 4]) -> Named<PartyMon> {
        let mut mon = new_party_mon(PokemonSpecies::Pidgey, 20, 1, &Origin::Trainer, &mut GameRng::tape(vec![]));
        mon.mon.moves = moves;
        mon.mon.pp = [0xC5; 4];
        Named { mon, ot: encode("RED").unwrap(), nick: encode("BIRD").unwrap() }
    }

    fn game(moves: [Option<PokemonMoveName>; 4], learning: PokemonMoveName) -> Game {
        let world = World { party: vec![pidgey(moves)], ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Instant);
        game.push(Mode::LearnMove(LearnMove::new(0, learning)));
        game
    }

    const FULL: [Option<PokemonMoveName>; 4] = [Some(Gust), Some(SandAttack), Some(Cut), Some(QuickAttack)];

    /// Answers every text box until `decision` is up, or until nothing is left if it is `None`.
    fn until(game: &mut Game, decision: Option<Decision>) {
        for _ in 0..2000 {
            match (game.status(), &decision) {
                (Status::Waiting(now), Some(wanted)) if now == *wanted => return,
                (Status::Idle, None) => return,
                (Status::Waiting(Decision::Text), _) => {
                    game.frame(Input::Buttons(Joypad::A));
                }
                _ => {}
            }
            game.frame(Input::None);
        }
        panic!("never reached {decision:?}: {:?}", game.status());
    }

    fn press(game: &mut Game, button: Joypad) {
        game.frame(Input::Buttons(button));
        game.frame(Input::None);
    }

    fn moves(game: &Game) -> ([Option<PokemonMoveName>; 4], [u8; 4]) {
        let mon = &game.world().party[0].mon.mon;
        (mon.moves, mon.pp)
    }

    #[test]
    fn a_free_slot_takes_the_move_with_its_full_pp() {
        let mut game = game([Some(Gust), None, None, None], QuickAttack);
        until(&mut game, None);
        assert_eq!(moves(&game), ([Some(Gust), Some(QuickAttack), None, None], [0xC5, 30, 0xC5, 0xC5]));
    }

    #[test]
    fn yes_then_a_move_forgets_it_and_the_pp_ups_go_with_it() {
        let mut game = game(FULL, WingAttack);
        until(&mut game, Some(Decision::TwoOption));
        press(&mut game, Joypad::A);
        until(&mut game, Some(Decision::ForgetMove));
        assert_eq!(game.ui().row(8)[6..10], encode("GUST").unwrap()[..], "single spaced, one a row");
        assert_eq!(game.ui().row(9)[6..12], encode("SAND-A").unwrap()[..]);
        press(&mut game, Joypad::DOWN);
        until(&mut game, Some(Decision::ForgetMove));
        assert_eq!(game.ui().get(5, 9), 0xED, "the cursor moves a row, not two");
        press(&mut game, Joypad::A);
        until(&mut game, None);
        assert_eq!(moves(&game), ([Some(Gust), Some(WingAttack), Some(Cut), Some(QuickAttack)], [0xC5, 35, 0xC5, 0xC5]));
    }

    /// `OneTwoAndText`'s `text_asm`: `SFX_SWAP` between `1, 2 and...` and ` Poof!`, which goes on
    /// the same line.
    #[test]
    fn the_swap_sound_plays_between_one_two_and_and_poof() {
        let world = World { party: vec![pidgey(FULL)], ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::LearnMove(LearnMove::new(0, WingAttack)));
        until(&mut game, Some(Decision::TwoOption));
        press(&mut game, Joypad::A);
        until(&mut game, Some(Decision::ForgetMove));
        press(&mut game, Joypad::A);
        let swapping = |game: &Game| (4..8).any(|channel| game.audio().channel_sound_id(channel) == sounds::SFX_SWAP.0);
        let line = |game: &Game| game.ui().row(14)[1..18].to_vec();
        for _ in 0..400 {
            if swapping(&game) {
                break;
            }
            game.frame(Input::None);
        }
        assert!(swapping(&game), "SFX_SWAP never played");
        let one_two = encode("1, 2 and...").unwrap();
        assert_eq!(line(&game)[..one_two.len()], one_two[..], "after the first half");
        assert_eq!(line(&game)[one_two.len()], UiSurface::BLANK, "and before the second");
        for _ in 0..100 {
            game.frame(Input::None);
        }
        let poof = encode("1, 2 and... Poof!").unwrap();
        assert_eq!(line(&game)[..poof.len()], poof[..], "Poof! goes on where the text stopped");
    }

    #[test]
    fn an_hm_cannot_be_forgotten_and_the_list_comes_back() {
        let mut game = game(FULL, WingAttack);
        until(&mut game, Some(Decision::TwoOption));
        press(&mut game, Joypad::A);
        until(&mut game, Some(Decision::ForgetMove));
        press(&mut game, Joypad::DOWN);
        until(&mut game, Some(Decision::ForgetMove));
        press(&mut game, Joypad::DOWN);
        until(&mut game, Some(Decision::ForgetMove));
        press(&mut game, Joypad::A);
        until(&mut game, Some(Decision::ForgetMove));
        assert_eq!(moves(&game).0, FULL, "CUT is still known");
        assert_eq!(game.ui().get(5, 8), 0xED, "and the cursor starts at the top again");
    }

    #[test]
    fn no_then_yes_abandons_and_nothing_changes() {
        let mut game = game(FULL, WingAttack);
        until(&mut game, Some(Decision::TwoOption));
        press(&mut game, Joypad::B);
        for _ in 0..20 {
            game.frame(Input::None);
        }
        until(&mut game, Some(Decision::TwoOption));
        press(&mut game, Joypad::A);
        until(&mut game, None);
        assert_eq!(moves(&game).0, FULL);
    }

    /// NO to abandoning is the whole question again, from `TryingToLearnText`.
    #[test]
    fn no_to_abandoning_asks_the_whole_question_again() {
        let mut game = game(FULL, WingAttack);
        until(&mut game, Some(Decision::TwoOption));
        press(&mut game, Joypad::B);
        for _ in 0..20 {
            game.frame(Input::None);
        }
        until(&mut game, Some(Decision::TwoOption));
        press(&mut game, Joypad::B);
        for _ in 0..20 {
            game.frame(Input::None);
        }
        until(&mut game, Some(Decision::TwoOption));
        press(&mut game, Joypad::A);
        until(&mut game, Some(Decision::ForgetMove));
    }

    #[test]
    fn b_on_the_list_asks_to_abandon_and_puts_the_screen_back() {
        let mut game = game(FULL, WingAttack);
        until(&mut game, Some(Decision::TwoOption));
        press(&mut game, Joypad::A);
        until(&mut game, Some(Decision::ForgetMove));
        press(&mut game, Joypad::B);
        assert_eq!(game.ui().get(4, 7), UiSurface::BLANK, "the list's box is gone");
        until(&mut game, Some(Decision::TwoOption));
    }

    #[test]
    fn a_command_picks_the_move_to_forget() {
        let mut game = game(FULL, WingAttack);
        until(&mut game, Some(Decision::TwoOption));
        game.frame(Input::Command(Command::ChooseOption(0)));
        until(&mut game, Some(Decision::ForgetMove));
        assert!(matches!(game.frame(Input::Command(Command::ChooseOption(4))).reply, Some(Reply::Refused(_))));
        assert_eq!(game.frame(Input::Command(Command::ChooseOption(3))).reply, Some(Reply::Accepted));
        until(&mut game, None);
        assert_eq!(moves(&game).0[3], Some(WingAttack));
    }

    #[test]
    fn a_level_offers_the_move_it_brings() {
        let world = World { party: vec![pidgey([Some(Gust), None, None, None])], ..World::default() };
        assert_eq!(LearnMove::from_level_up(&world, 0, 12).map(|learn| learn.learning), Some(QuickAttack));
        assert_eq!(LearnMove::from_level_up(&world, 0, 13), None);
    }

    #[test]
    fn a_save_mid_question_resumes_identically() {
        let mut whole = game(FULL, WingAttack);
        until(&mut whole, Some(Decision::TwoOption));
        press(&mut whole, Joypad::A);
        for _ in 0..3 {
            whole.frame(Input::None);
        }
        let mut restored = Game::load(&whole.save(), Pacing::Instant).unwrap();
        for frame in 0..120 {
            let input = || if frame % 7 == 6 { Input::Buttons(Joypad::A) } else { Input::None };
            let (a, b) = (whole.frame(input()), restored.frame(input()));
            assert_eq!((whole.ui(), a.status, &whole.world().party), (restored.ui(), b.status, &restored.world().party), "frame {frame}");
        }
    }
}
