//! `EvolutionAfterBattle` over `next_evolution` and `evolve`, with `EvolveMon`'s animation between
//! them and `LearnMoveFromLevelUp` handing off to `LearnMove` after.
//!
//! For every party mon whose flag is set, the old species' entries are walked in order. One that
//! fires prints `IsEvolvingText`, waits 50 frames, clears the top twelve rows and runs `EvolveMon`:
//! the old picture and its cry, 80 frames of the Safari Zone's music, then eight rounds of watching
//! for B over 16, 14, ... 2 frames, each followed by one more flip of the picture to the new species
//! and back than the last, and a final flip to the new one. B stops it unless an item forced it
//! (`wForceEvolution`), and a stopped mon's remaining entries are not looked at. A finished one
//! prints `EvolvedText` and `IntoText`, plays `SFX_GET_ITEM_2`, waits 40 frames, clears the screen,
//! and only then changes: see `evolve`. The walk then carries on through the *old* species' entries
//! after the one that fired.
//!
//! `cur_item` is `wCurItem`, which is `wCurPartySpecies`: an item entry compares against it, and
//! `LearnMoveFromLevelUp` leaves the new species there, which the next mon then sees.
//!
//! Entry points: [`Evolution::after_battle`] for the end of a battle, [`Evolution::try_evolving`]
//! for `TryEvolvingMon` (Rare Candy, stones). It answers `Outcome::Chosen(1)` when anything evolved
//! (`wEvolutionOccurred`); `PlayDefaultMusic` after an evolution outside a battle is the caller's.
//!
//! The screen goes black and back only on the SGB, through `SET_PAL_POKEMON_WHOLE_SCREEN`: nothing
//! here touches `rBGP`, so a DMG shows no whiteout.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::{pokered_symbols, DmgPointer};
use poke_core::text_script::{decode, TextBuffer, TextCommand};
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::gfx::sgb::{determine_palette_id_out_of_battle, PaletteCommand};
use crate::gfx::tiles::V_CHARS2;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::learn_move::LearnMove;
use crate::modes::place_string::{ch, PlaceString, FIRST_LINE};
use crate::modes::text_box::TextBox;
use crate::systems::evos_moves::{evolve, next_evolution};
use crate::systems::pokedex::front_pic_tiles;

/// `wEvoMonTileOffset`: the old picture's tiles plus this are the new picture's, in `vBackPic`.
const BACK_PIC_OFFSET: u8 = 0x31;
/// The picture's corner, `hlcoord 7, 2`.
const PIC_X: usize = 7;
const PIC_Y: usize = 2;
const PIC_TILES: usize = 7;
/// `lb bc, $1, $10`: one flip in the first round, and 16 frames to press B before it.
const FIRST_WINDOW: u8 = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evolution {
    /// `wCanEvolveFlags`, a bit a party slot.
    can_evolve: u8,
    cur_item: u8,
    force: bool,
    /// `wIsInBattle`, which `EndOfBattle` has not cleared yet when it calls this.
    in_battle: bool,
    /// `wWhichPokemon`, and the old species' entry to look at next.
    slot: u8,
    entry: usize,
    /// `wEvoOldSpecies`, set once a mon, and `wEvoNewSpecies`.
    old_species: Option<PokemonSpecies>,
    new_species: Option<PokemonSpecies>,
    occurred: bool,
    phase: Phase,
    printer: Option<PlaceString>,
    /// `hTileAnimations`, pushed on the way in.
    tile_animations: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// The next mon, or the next entry of this one, is looked at on the next update.
    Walking,
    IsEvolving,
    Pausing(u8),
    /// `PlayCry`'s wait on the old species' cry.
    OldCry,
    Music(u8),
    /// `Evolution_CheckForCancel`: `flips` is `b`, `window` is `c` and `left` the frames still to
    /// poll in this window.
    Watching { flips: u8, window: u8, left: u8 },
    /// `Evolution_BackAndForthAnim`'s `Delay3` after flip `done` of `2 * flips`.
    Flipping { flips: u8, window: u8, done: u8, frames: u8 },
    /// The last flip's `Delay3`.
    Settling(u8),
    /// `.done`'s `PlayCry`, of the new species or of the old one when stopped.
    Cry { cancelled: bool },
    Stopped,
    Evolved,
    Into,
    /// `PlaySoundWaitForCurrent` and then `WaitForSoundToFinish`.
    GetItem { playing: bool },
    AfterGetItem(u8),
    Learning,
    Done,
}

fn script(at: DmgPointer) -> Vec<TextCommand> {
    decode(at).expect("the evolution texts are in the cartridge")
}

impl Evolution {
    /// `EvolutionAfterBattle`, with `wForceEvolution` cleared as `EndOfBattle` clears it.
    pub fn after_battle(can_evolve: u8, cur_item: u8) -> Self {
        Self::new(can_evolve, cur_item, false, true)
    }

    /// `TryEvolvingMon`: one party slot. A stone passes itself as `cur_item` and `force`; a Rare
    /// Candy passes whatever `wCurItem` holds, which `LearnMoveFromLevelUp` has just set to the
    /// mon's species, and no `force`.
    pub fn try_evolving(slot: u8, cur_item: u8, force: bool) -> Self {
        Self::new(1 << slot, cur_item, force, false)
    }

    fn new(can_evolve: u8, cur_item: u8, force: bool, in_battle: bool) -> Self {
        Self {
            can_evolve, cur_item, force, in_battle, slot: 0, entry: 0, old_species: None, new_species: None,
            occurred: false, phase: Phase::Walking, printer: None, tile_animations: 0,
        }
    }

    /// `wEvolutionOccurred`.
    pub fn occurred(&self) -> bool {
        self.occurred
    }

    /// Whether a B pressed now would stop the evolution: from the 80 frames of music on, and never
    /// when an item forced it. A press in the music is an edge the first watch sees.
    pub fn can_cancel(&self) -> bool {
        !self.force && matches!(self.phase, Phase::Music(_) | Phase::Watching { .. } | Phase::Flipping { .. })
    }

    /// Whether `Evolution_CheckForCancel` is reading the pad, which is where a lockstep agrees to
    /// press B.
    pub fn is_watching(&self) -> bool {
        matches!(self.phase, Phase::Watching { .. })
    }

    fn text(&mut self, phase: Phase, commands: Vec<TextCommand>) -> Transition {
        self.phase = phase;
        Transition::Push(Mode::TextBox(TextBox::script(commands)))
    }

    fn species(&self) -> (PokemonSpecies, PokemonSpecies) {
        (self.old_species.expect("an evolution is under way"), self.new_species.expect("an evolution is under way"))
    }

    /// `Evolution_PartyMonLoop` and `.evoEntryLoop`, up to the next evolution that fires.
    fn walk(&mut self, ctx: &mut Ctx) -> Transition {
        while (self.slot as usize) < ctx.world.party.len() {
            if self.can_evolve & 1 << self.slot != 0 {
                let party_mon = &ctx.world.party[self.slot as usize].mon;
                let old = *self.old_species.get_or_insert(party_mon.mon.species);
                if let Some((at, into)) = next_evolution(old, party_mon.level, self.cur_item, self.force, self.entry) {
                    self.entry = at + 1;
                    self.new_species = Some(into);
                    self.occurred = true;
                    let nick = ctx.world.party[self.slot as usize].nick.clone();
                    ctx.world.text.strings.insert(TextBuffer::StringBuffer, nick);
                    return self.text(Phase::IsEvolving, script(pokered_symbols::IsEvolvingText));
                }
            }
            self.next_mon();
        }
        ctx.screen.tiles.animation.kind = self.tile_animations;
        self.phase = Phase::Done;
        Transition::Pop(Outcome::Chosen(self.occurred as u8))
    }

    fn next_mon(&mut self) {
        self.slot += 1;
        self.entry = 0;
        self.old_species = None;
    }

    /// `Evolution_LoadPic` for `species` at the picture's corner.
    fn load_pic(ctx: &mut Ctx, species: PokemonSpecies, first_tile: usize) {
        ctx.screen.tiles.load(V_CHARS2 + first_tile, &front_pic_tiles(species, true).concat());
        for column in 0..PIC_TILES {
            for row in 0..PIC_TILES {
                ctx.screen.ui.set(PIC_X + PIC_TILES - 1 - column, PIC_Y + row, (column * PIC_TILES + row) as u8);
            }
        }
    }

    /// `Evolution_ChangeMonPic`: every tile of the picture moved by `offset`, which is the other
    /// species' picture.
    fn change_mon_pic(ctx: &mut Ctx, offset: u8) {
        for row in PIC_Y..PIC_Y + PIC_TILES {
            for column in PIC_X..PIC_X + PIC_TILES {
                let tile = ctx.screen.ui.get(column, row);
                ctx.screen.ui.set(column, row, tile.wrapping_add(offset));
            }
        }
    }

    fn whole_screen_palette(ctx: &mut Ctx, species: PokemonSpecies, black: bool) {
        let mon = determine_palette_id_out_of_battle(species as u8);
        ctx.screen.sgb.run(&PaletteCommand::PokemonWholeScreen { mon, black });
    }

    /// `EvolveMon` up to the old species' cry: the rows cleared, `SFX_TINK`, both pictures loaded and
    /// the old one shown. The `Delay3` after the sound is loading and not modelled.
    fn start_evolve_mon(&mut self, ctx: &mut Ctx) {
        ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, 12, UiSurface::BLANK);
        crate::gfx::mon_icons::clear_sprites(&mut ctx.screen.sprites);
        ctx.audio.end_low_health_alarm();
        ctx.audio.play_sound(SoundId::STOP_ALL_MUSIC);
        ctx.audio.play_sound(sounds::SFX_TINK);
        let (old, new) = self.species();
        ctx.screen.tiles.animation.kind = 0;
        Self::whole_screen_palette(ctx, old, false);
        Self::load_pic(ctx, new, BACK_PIC_OFFSET as usize);
        Self::load_pic(ctx, old, 0);
        ctx.audio.play_cry(old as u8);
        self.phase = Phase::OldCry;
    }

    /// `Evolution_CheckForCancel`'s frame: its `DelayFrame` has passed, so the pad is read.
    fn watch(&mut self, ctx: &mut Ctx, flips: u8, window: u8, left: u8) -> Transition {
        let pressed = ctx.pad.low_sensitivity(ctx.frame_counter).contains(Joypad::B);
        if pressed && !self.force {
            return self.finish_evolve_mon(ctx, true);
        }
        if left > 1 {
            self.phase = Phase::Watching { flips, window, left: left - 1 };
            return Transition::Stay;
        }
        Self::change_mon_pic(ctx, BACK_PIC_OFFSET);
        self.phase = Phase::Flipping { flips, window, done: 1, frames: 3 };
        Transition::Stay
    }

    fn flipped(&mut self, ctx: &mut Ctx, flips: u8, window: u8, done: u8) {
        if done < 2 * flips {
            let offset = if done % 2 == 0 { BACK_PIC_OFFSET } else { BACK_PIC_OFFSET.wrapping_neg() };
            Self::change_mon_pic(ctx, offset);
            self.phase = Phase::Flipping { flips, window, done: done + 1, frames: 3 };
        } else if window > 2 {
            self.phase = Phase::Watching { flips: flips + 1, window: window - 2, left: window - 2 };
        } else {
            Self::change_mon_pic(ctx, BACK_PIC_OFFSET);
            self.phase = Phase::Settling(3);
        }
    }

    /// `EvolveMon`'s `.done`.
    fn finish_evolve_mon(&mut self, ctx: &mut Ctx, cancelled: bool) -> Transition {
        let (old, new) = self.species();
        let shown = if cancelled { old } else { new };
        ctx.audio.play_sound(SoundId::STOP_ALL_MUSIC);
        ctx.audio.play_cry(shown as u8);
        Self::whole_screen_palette(ctx, shown, false);
        self.phase = Phase::Cry { cancelled };
        Transition::Stay
    }

    fn reload_tileset(ctx: &mut Ctx) {
        if let Some(tileset) = ctx.screen.map.tileset {
            ctx.screen.tiles.load_tileset(tileset);
        }
    }

    /// Everything after the screen clears: the mon changes, it may learn a move, and the walk
    /// carries on through the old species' entries.
    fn change(&mut self, ctx: &mut Ctx) -> Transition {
        let (_, new) = self.species();
        let named = &mut ctx.world.party[self.slot as usize];
        let learning = evolve(named, new, &mut ctx.world.pokedex);
        self.cur_item = new as u8;
        if !self.in_battle {
            Self::reload_tileset(ctx);
        }
        match learning {
            Some(learning) => {
                self.phase = Phase::Learning;
                Transition::Push(Mode::LearnMove(LearnMove::new(self.slot, learning)))
            }
            None => self.walk(ctx),
        }
    }
}

impl ModeUpdate for Evolution {
    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        self.tile_animations = ctx.screen.tiles.animation.kind;
        self.walk(ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Pausing(frames) if frames > 1 => self.phase = Phase::Pausing(frames - 1),
            Phase::Pausing(_) => self.start_evolve_mon(ctx),
            Phase::OldCry if ctx.audio.sound_finished() => {
                ctx.audio.play_music(sounds::MUSIC_SAFARI_ZONE);
                self.phase = Phase::Music(80);
            }
            Phase::Music(frames) if frames > 1 => self.phase = Phase::Music(frames - 1),
            Phase::Music(_) => {
                let (old, _) = self.species();
                Self::whole_screen_palette(ctx, old, true);
                self.phase = Phase::Watching { flips: 1, window: FIRST_WINDOW, left: FIRST_WINDOW };
            }
            Phase::Watching { flips, window, left } => return self.watch(ctx, flips, window, left),
            Phase::Flipping { flips, window, done, frames } if frames > 1 =>
                self.phase = Phase::Flipping { flips, window, done, frames: frames - 1 },
            Phase::Flipping { flips, window, done, .. } => self.flipped(ctx, flips, window, done),
            Phase::Settling(frames) if frames > 1 => self.phase = Phase::Settling(frames - 1),
            Phase::Settling(_) => return self.finish_evolve_mon(ctx, false),
            Phase::Cry { cancelled } if ctx.audio.sound_finished() => {
                return if cancelled {
                    self.text(Phase::Stopped, script(pokered_symbols::StoppedEvolvingText))
                } else {
                    self.text(Phase::Evolved, script(pokered_symbols::EvolvedText))
                };
            }
            Phase::Into => {
                let mut printer = self.printer.take().expect("IntoText is printing");
                let mut answered = 0;
                if printer.update(ctx, &mut answered).is_none() {
                    self.printer = Some(printer);
                } else {
                    self.phase = Phase::GetItem { playing: false };
                    return self.update(ctx);
                }
            }
            Phase::GetItem { playing: false } if ctx.audio.sound_finished() => {
                ctx.audio.play_sound(sounds::SFX_GET_ITEM_2);
                self.phase = Phase::GetItem { playing: true };
            }
            Phase::GetItem { playing: true } if ctx.audio.sound_finished() => self.phase = Phase::AfterGetItem(40),
            Phase::AfterGetItem(frames) if frames > 1 => self.phase = Phase::AfterGetItem(frames - 1),
            // `ClearScreen`, whose `Delay3` is loading and not modelled.
            Phase::AfterGetItem(_) => {
                ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
                return self.change(ctx);
            }
            Phase::Walking => return self.walk(ctx),
            Phase::OldCry | Phase::Cry { .. } | Phase::GetItem { .. } | Phase::IsEvolving | Phase::Stopped
            | Phase::Evolved | Phase::Learning | Phase::Done => {}
        }
        Transition::Stay
    }

    fn resume(&mut self, _outcome: Outcome, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::IsEvolving => self.phase = Phase::Pausing(50),
            Phase::Stopped => {
                ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
                Self::reload_tileset(ctx);
                self.next_mon();
                return self.walk(ctx);
            }
            // `IntoText` is printed into the box `EvolvedText` left, without drawing another.
            Phase::Evolved => {
                let (_, new) = self.species();
                let mut text = vec![ch::LINE];
                text.extend(poke_core::charmap::encode("into ").expect("IntoText encodes"));
                text.extend(new.name());
                text.extend([poke_core::charmap::encode("!").expect("IntoText encodes")[0], ch::DONE]);
                ctx.world.text.strings.insert(TextBuffer::NameBuffer, new.name());
                self.printer = Some(PlaceString::call(text, FIRST_LINE));
                self.phase = Phase::Into;
                return self.update(ctx);
            }
            Phase::Learning => return self.walk(ctx),
            _ => {}
        }
        Transition::Stay
    }

    fn status(&self) -> Status {
        Status::Busy
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use poke_core::item::ItemId;
    use poke_core::move_name::PokemonMoveName;
    use crate::command::{Command, Decision, Reply};
    use crate::party::{Named, PartyMon};
    use crate::rng::GameRng;
    use crate::systems::add_mon::{new_party_mon, Origin};
    use crate::world::World;
    use crate::{Game, Input, Pacing};
    use super::*;
    use PokemonSpecies::*;

    fn mon(species: PokemonSpecies, level: u8, nick: &str) -> Named<PartyMon> {
        let mon = new_party_mon(species, level, 1, &Origin::Trainer, &mut GameRng::tape(vec![]));
        Named { mon, ot: encode("RED").unwrap(), nick: encode(nick).unwrap() }
    }

    fn game(party: Vec<Named<PartyMon>>, evolution: Evolution) -> Game {
        let world = World { party, ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::Evolution(evolution));
        game
    }

    /// Runs to the end, answering every text and holding `hold` down from the frame `at` on.
    fn run(game: &mut Game, hold: Option<(u32, Joypad)>) -> u32 {
        for frame in 0..5000 {
            if game.modes().is_empty() {
                return frame;
            }
            let input = match (game.status(), hold) {
                (Status::Waiting(Decision::Text), _) if frame % 2 == 0 => Input::Buttons(Joypad::A),
                (Status::Waiting(Decision::TwoOption), _) => Input::Buttons(Joypad::B),
                (_, Some((at, button))) if frame >= at => Input::Buttons(button),
                _ => Input::None,
            };
            game.frame(input);
        }
        panic!("the evolution never finished: {:?}", game.modes().last());
    }

    #[test]
    fn a_mon_at_its_level_evolves_and_is_renamed_and_owned() {
        let mut game = game(vec![mon(Charmander, 16, "CHARMANDER")], Evolution::try_evolving(0, 0, false));
        run(&mut game, None);
        let evolved = &game.world().party[0];
        assert_eq!(evolved.mon.mon.species, Charmeleon);
        assert_eq!(evolved.nick, Charmeleon.name());
        assert!(game.world().pokedex.is_owned(Charmeleon));
    }

    #[test]
    fn a_flag_that_is_not_set_is_passed_over() {
        let party = vec![mon(Charmander, 16, "A"), mon(Bulbasaur, 16, "B")];
        let mut game = game(party, Evolution::after_battle(0b10, 0));
        run(&mut game, None);
        assert_eq!(game.world().party[0].mon.mon.species, Charmander);
        assert_eq!(game.world().party[1].mon.mon.species, Ivysaur);
    }

    #[test]
    fn b_stops_it_and_nothing_changes() {
        let mut game = game(vec![mon(Charmander, 16, "CHAR")], Evolution::try_evolving(0, 0, false));
        run(&mut game, Some((200, Joypad::B)));
        assert_eq!(game.world().party[0].mon.mon.species, Charmander);
        assert_eq!(game.world().party[0].nick, encode("CHAR").unwrap());
    }

    #[test]
    fn a_stone_cannot_be_stopped() {
        let stone = ItemId::FireStone as u8;
        let mut game = game(vec![mon(Growlithe, 5, "DOG")], Evolution::try_evolving(0, stone, true));
        run(&mut game, Some((200, Joypad::B)));
        assert_eq!(game.world().party[0].mon.mon.species, Arcanine);
    }

    /// The picture flips 1 + 2 + ... + 8 times each way over the watching windows, then once more.
    #[test]
    fn the_animation_takes_the_windows_and_the_flips_it_should() {
        let mut game = game(vec![mon(Charmander, 16, "CHAR")], Evolution::try_evolving(0, 0, false));
        while !matches!(game.modes().last(), Some(Mode::Evolution(evolution)) if matches!(evolution.phase, Phase::Watching { .. })) {
            game.frame(Input::None);
        }
        let start = game.frames();
        while matches!(game.modes().last(), Some(Mode::Evolution(evolution))
            if matches!(evolution.phase, Phase::Watching { .. } | Phase::Flipping { .. } | Phase::Settling(_))) {
            game.frame(Input::None);
        }
        let windows: u64 = (1..=8).map(|round| 18 - 2 * round).sum();
        let flips: u64 = (1..=8).map(|round| 2 * round * 3).sum::<u64>() + 3;
        assert_eq!(game.frames() - start, windows + flips);
        assert_eq!(game.ui().get(PIC_X + 6, PIC_Y), BACK_PIC_OFFSET, "the new picture's first tile");
    }

    #[test]
    fn a_move_the_new_level_brings_is_offered() {
        // Ivysaur learns Razor Leaf at 30; a Bulbasaur already that level evolves into it.
        let mut bulbasaur = mon(Bulbasaur, 30, "B");
        bulbasaur.mon.mon.moves = [Some(PokemonMoveName::Tackle), None, None, None];
        let mut game = game(vec![bulbasaur], Evolution::try_evolving(0, 0, false));
        run(&mut game, None);
        assert!(game.world().party[0].mon.mon.moves.contains(&Some(PokemonMoveName::RazorLeaf)));
    }

    #[test]
    fn cancel_evolution_is_a_command_while_it_can_be_stopped() {
        let mut game = game(vec![mon(Charmander, 16, "CHAR")], Evolution::try_evolving(0, 0, false));
        assert!(matches!(game.frame(Input::Command(Command::CancelEvolution)).reply, Some(Reply::Refused(_))));
        while !matches!(game.modes().last(), Some(Mode::Evolution(evolution)) if evolution.can_cancel()) {
            game.frame(Input::None);
        }
        assert_eq!(game.frame(Input::Command(Command::CancelEvolution)).reply, Some(Reply::Accepted));
        run(&mut game, None);
        assert_eq!(game.world().party[0].mon.mon.species, Charmander);
    }

    #[test]
    fn a_save_mid_animation_resumes_identically() {
        let mut whole = game(vec![mon(Charmander, 16, "CHAR")], Evolution::try_evolving(0, 0, false));
        for _ in 0..150 {
            whole.frame(Input::None);
        }
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..400 {
            let input = || if frame % 50 == 49 { Input::Buttons(Joypad::A) } else { Input::None };
            let (a, b) = (whole.frame(input()), restored.frame(input()));
            assert_eq!((whole.ui(), a.audio, &whole.world().party), (restored.ui(), b.audio, &restored.world().party), "frame {frame}");
        }
    }
}
