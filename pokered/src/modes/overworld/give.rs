//! `GivePokemon`: `_GivePokemon`'s `SetPokedexOwnedFlag`, then `_AddPartyMon` with the nickname
//! `AskName` offers, or with a full party `SendNewMonToBox`, which offers one too. A full box refuses.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_symbols;
use poke_core::text_script::TextBuffer;
use crate::mode::{Ctx, Mode, Outcome};
use crate::modes::naming_screen::{NamingScreen, NamingScreenType};
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::party::{BoxMon, Named, MONS_PER_BOX, NUM_STATS};
use crate::systems::battle::BattleMon;
use crate::systems::experience::calc_experience;
use crate::systems::battle::enemy::load_enemy_mon_data;
use crate::systems::battle::{Battle, BattleKind};
use crate::systems::add_mon::{add_party_mon, new_party_mon, Origin};
use super::script::{text_at, Block, Flow, Routine, Then};
use super::Overworld;

/// `wBoxCount` at `MONS_PER_BOX`.
fn box_is_full(ctx: &Ctx) -> bool {
    ctx.world.boxes.get(ctx.world.current_box as usize).is_some_and(|mons| mons.len() >= MONS_PER_BOX)
}

/// `SendNewMonToBox` past its `AskName`: the mon made for the box first in the current box, at its
/// level's experience with no stat experience.
fn send_new_mon_to_box(ctx: &mut Ctx, enemy: &BattleMon, ot: Vec<u8>, nick: Vec<u8>) {
    let growth = poke_core::base_stats::BaseStats::of(enemy.species).growth_rate;
    let mon = BoxMon {
        species: enemy.species,
        hp: enemy.hp,
        box_level: enemy.level,
        status: enemy.status,
        types: enemy.types,
        catch_rate: enemy.catch_rate,
        moves: enemy.moves,
        ot_id: ctx.world.player_id,
        exp: calc_experience(growth, enemy.level),
        stat_exp: [0; NUM_STATS],
        dvs: enemy.dvs,
        pp: enemy.pp,
    };
    let current = ctx.world.current_box as usize;
    if ctx.world.boxes.len() <= current {
        ctx.world.boxes.resize(current + 1, Vec::new());
    }
    ctx.world.boxes[current].insert(0, Named { mon, ot, nick });
}

/// `PARTY_LENGTH`.
const PARTY_LENGTH: usize = 6;

impl Overworld {
    /// `_GivePokemon` up to its `PrintText`: the species marked owned and named in `wNameBuffer`.
    pub(super) fn give_pokemon(&mut self, ctx: &mut Ctx, species: PokemonSpecies, level: u8) -> Flow {
        self.rt.no_auto_text_box = false;
        self.rt.do_not_wait = false;
        self.rt.added_to_party = false;
        self.rt.gave_pokemon = true;
        if ctx.world.party.len() >= PARTY_LENGTH {
            if box_is_full(ctx) {
                self.rt.gave_pokemon = false;
                return Then::block(Block::PrintText(text_at(pokered_symbols::BoxIsFullText))).ret();
            }
            // `LoadEnemyMonData` draws the DVs before anything is printed.
            let lead = ctx.world.party[0].mon.clone();
            let mut battle = Battle::new(BattleKind::Wild, &lead, Vec::new());
            load_enemy_mon_data(&mut battle, &mut ctx.world.pokedex, species, level, 0, ctx.rng);
            self.rt.box_mon = Some(battle.enemy.mon);
        }
        ctx.world.pokedex.set_owned(species);
        ctx.world.text.strings.insert(TextBuffer::NameBuffer, species.name());
        Then::block(Block::PrintText(text_at(pokered_symbols::GotMonText))).then(Routine::GivePokemonAskName(species, level))
    }

    /// `AskName`'s question.
    pub(super) fn give_pokemon_ask_name(&mut self, ctx: &mut Ctx, species: PokemonSpecies, level: u8) -> Flow {
        self.rt.saved_screen = Some(ctx.screen.ui.clone());
        ctx.world.text.strings.insert(TextBuffer::NameBuffer, species.name());
        Then::block(Block::PrintText(text_at(pokered_symbols::DoYouWantToNicknameText)))
            .then(Routine::GivePokemonYesNo(species, level))
    }

    pub(super) fn give_pokemon_yes_no(&mut self, species: PokemonSpecies, level: u8) -> Flow {
        let menu = TwoOptionMenu::new(TwoOptionMenuId::YesNo, (14, 7), false);
        Then::block(Block::Mode(Box::new(Mode::TwoOptionMenu(menu)))).then(Routine::GivePokemonAnswered(species, level))
    }

    /// `AskName` after the yes/no: the naming screen for a yes.
    pub(super) fn give_pokemon_answered(&mut self, ctx: &mut Ctx, species: PokemonSpecies, level: u8) -> Flow {
        ctx.world.text.strings.insert(TextBuffer::StringBuffer, Vec::new());
        if self.rt.outcome != Some(Outcome::Chosen(0)) {
            return Flow::Jump(Routine::GivePokemonNamed(species, level).into());
        }
        let screen = NamingScreen::new(NamingScreenType::Mon, Some(species));
        Then::block(Block::Mode(Box::new(Mode::NamingScreen(screen)))).then(Routine::GivePokemonNamed(species, level))
    }

    /// The rest of `_AddPartyMon`: the name, or the species' own for none, and the mon made.
    pub(super) fn give_pokemon_named(&mut self, ctx: &mut Ctx, species: PokemonSpecies, level: u8) -> Flow {
        if let Some(screen) = self.rt.saved_screen.take() {
            ctx.screen.ui = screen;
        }
        let typed = ctx.world.text.string(TextBuffer::StringBuffer);
        let nick = if typed.is_empty() { species.name() } else { typed };
        if let Some(enemy) = self.rt.box_mon.take() {
            send_new_mon_to_box(ctx, &enemy, ctx.world.player_name.clone(), nick.clone());
            ctx.world.text.strings.insert(TextBuffer::BoxMonNicks, nick);
            let current = ctx.world.current_box;
            let number = if current < 9 { vec![0xF7 + current] } else { vec![0xF7, 0xF6 + current - 9] };
            ctx.world.text.strings.insert(TextBuffer::StringBuffer, number);
            return Then::block(Block::PrintText(text_at(pokered_symbols::SentToBoxText))).ret();
        }
        let mon = new_party_mon(species, level, ctx.world.player_id, &Origin::Given, ctx.rng);
        let named = Named { mon, ot: ctx.world.player_name.clone(), nick };
        let pokedex = &mut ctx.world.pokedex;
        add_party_mon(&mut ctx.world.party, named, Some(pokedex));
        self.rt.do_not_wait = true;
        self.rt.added_to_party = true;
        Flow::Return
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use poke_core::map::Map;
    use crate::command::{Command, Decision};
    use crate::mode::Status;
    use crate::rng::GameRng;
    use crate::systems::overworld::Location;
    use crate::world::World;
    use crate::{Game, Input, Pacing};
    use super::super::{Phase, Routine};
    use super::*;

    /// A game in Pallet Town partway into a script that gives a mon.
    fn giving(species: PokemonSpecies, answer: u8) -> Game {
        let world = World {
            player_name: encode("RED").unwrap(),
            location: Location { map: Map::PalletTown, x: 5, y: 8, ..Location::default() },
            ..World::default()
        };
        let mut game = Game::new(world, GameRng::seeded(1), Pacing::Faithful);
        let mut overworld = Overworld::new();
        overworld.phase = Phase::Script;
        overworld.rt.stack = vec![Routine::AfterDisplayDialogue.into(), Routine::GivePokemon(species, 5).into()];
        game.push(Mode::Overworld(overworld));
        for _ in 0..3000 {
            let input = match game.status() {
                Status::Waiting(Decision::Text) => Input::Command(Command::Advance),
                Status::Waiting(Decision::TwoOption) => Input::Command(Command::ChooseOption(answer)),
                Status::Waiting(Decision::NamingScreen) => Input::Command(Command::EnterName(encode("BUD").unwrap())),
                Status::Waiting(Decision::Overworld) => break,
                _ => Input::None,
            };
            game.frame(input);
        }
        game
    }

    #[test]
    fn a_given_mon_joins_the_party_under_its_species_name_when_the_player_declines_a_nickname() {
        let game = giving(PokemonSpecies::Eevee, 1);
        let party = &game.world().party;
        assert_eq!(party.len(), 1);
        assert_eq!((party[0].mon.mon.species, party[0].mon.level), (PokemonSpecies::Eevee, 5));
        assert_eq!(party[0].nick, PokemonSpecies::Eevee.name());
        assert!(game.world().pokedex.is_owned(PokemonSpecies::Eevee));
    }

    #[test]
    fn a_given_mon_takes_the_nickname_typed() {
        let game = giving(PokemonSpecies::Eevee, 0);
        assert_eq!(game.world().party[0].nick, encode("BUD").unwrap());
    }
}
