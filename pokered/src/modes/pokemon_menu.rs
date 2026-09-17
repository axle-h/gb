//! `StartMenu_Pokemon`: the party menu the start menu's `POKéMON` row opens, the submenu a chosen
//! mon opens over it, and the screens its rows lead to.
//!
//! `STATS` runs both status screen pages and then opens the party menu again from the top, cleared
//! and redrawn. `SWITCH` marks the mon and reopens the list over itself; with a party of one it too
//! starts again from the top. B on the submenu goes back to the list without clearing it, and
//! `CANCEL` or B on the list goes back to the start menu in the same frame: the delays of
//! `GBPalWhiteOutWithDelay3` and `RestoreScreenTilesAndReloadTilePatterns` are loading.
//!
//! SURF is carried out here (`.surf`): the Soul Badge, `IsSurfingAllowed` and `ItemUseSurfboard`,
//! whose texts print over the party menu. A refusal goes back to the list; getting on, getting off,
//! or finding no place to get off answers with the move and closes the start menu, and the overworld
//! takes the step forward.
//!
//! CUT, STRENGTH and FLASH are carried out here too, as far as the party menu's own screen goes:
//! the badge, the refusals and the texts. What is left of each goes to the overworld under the
//! closing start menu, as [`UsedFieldMove`] for CUT's animation and FLASH's palette, and as
//! `Location::strength_active` for STRENGTH, which is its whole effect.
//!
//! Any other field move answers with the move, from [`PokemonMenu::field_move`], and pops back to
//! the start menu where the cartridge would carry it out.

use poke_core::map_header::MapHeader;
use poke_core::move_name::PokemonMoveName;
use poke_core::symbols::{pokered_local_labels as local, pokered_symbols, DmgPointer};
use poke_core::text_script::{decode, far_text, TextBuffer};
use serde::{Deserialize, Serialize};
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::field_move_menu::{FieldMoveChoice, FieldMoveMenu};
use crate::modes::overworld::bike_surf::item_use_surfboard;
use crate::modes::overworld::escape::{arm_escape_warp, escape_rope_allowed};
use crate::modes::use_item::UseItem;
use crate::modes::text_box::TextBox;
use crate::systems::overworld::bike_surf::surfing_refusal;
use crate::systems::overworld::cut::cut_tile;
use crate::systems::overworld::location::UsedFieldMove;
use crate::modes::party_menu::{PartyMenu, PartyMenuType};
use crate::modes::status_screen::StatusScreen;
use crate::modes::town_map::TownMap;
use crate::systems::field_moves::{field_moves, FieldMoves};
use crate::systems::overworld::collision::is_outside;

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
    /// `ChooseFlyDestination`.
    TownMap,
    /// `PlayDefaultMusic`'s `WaitForSoundToFinish` inside `ItemUseSurfboard`, then its song and text.
    Music { text: Option<DmgPointer>, result: bool },
    /// A field move's text, and whether the start menu closes after it rather than the list coming back.
    Text { result: bool },
    /// `UsedStrengthText`'s `text_asm`: the cry, `PlayCry`'s wait for it, its `Delay3`, then
    /// `CanMoveBouldersText`.
    StrengthCry,
    StrengthSound,
    StrengthBeat(u8),
    /// `.canTeleport`'s text, then its `DelayFrames 60` before the start menu closes.
    Teleport,
    TeleportDelay(u8),
    /// `.softboiled`'s `UseItem`, after which the party list comes back whatever happened.
    Softboiled,
}

/// `.canTeleport`'s `DelayFrames 60`.
const TELEPORT_FRAMES: u8 = 60;

/// `wObtainedBadges`: the bits the four field moves built here are refused without.
const BOULDER_BADGE: u8 = 1 << 0;
const CASCADE_BADGE: u8 = 1 << 1;
const THUNDER_BADGE: u8 = 1 << 2;
const RAINBOW_BADGE: u8 = 1 << 3;
const SOUL_BADGE: u8 = 1 << 4;
/// `UsedStrengthText`'s `Delay3` after the cry.
const CRY_BEAT: u8 = 3;

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
                // `.choseStats`: the mon icons come down before the status screen goes up.
                crate::gfx::mon_icons::clear_sprites(&mut ctx.screen.sprites);
                self.phase = Phase::StatusScreen;
                let mon = ctx.world.party[self.slot as usize].clone();
                Transition::Push(Mode::StatusScreen(StatusScreen::new(mon)))
            }
            FieldMoveChoice::Move(field_move) => {
                self.field_move = Some(field_move);
                match field_move {
                    PokemonMoveName::Fly => self.fly(ctx),
                    PokemonMoveName::Surf => self.surf(ctx),
                    PokemonMoveName::Cut => self.cut(ctx),
                    PokemonMoveName::Strength => self.strength(ctx),
                    PokemonMoveName::Flash => self.flash(ctx),
                    PokemonMoveName::Dig => self.dig(ctx),
                    PokemonMoveName::Teleport => self.teleport(ctx),
                    PokemonMoveName::Softboiled => self.softboiled(ctx),
                    _ => Transition::Pop(Outcome::Chosen(field_move as u8)),
                }
            }
        }
    }

    /// `_NewBadgeRequiredText`, which every badge check shares.
    fn new_badge_required(&mut self) -> Transition {
        self.text(far_text("_NewBadgeRequiredText").expect("the text is in the cartridge"), false)
    }

    /// The chosen mon's nickname in `wStringBuffer`, as `GetPartyMonName` leaves it for a text.
    fn name_the_mon(&self, ctx: &mut Ctx) {
        let nick = ctx.world.party[self.slot as usize].nick.clone();
        ctx.world.text.strings.insert(TextBuffer::NameBuffer, nick);
    }

    /// `.cut` and the part of `UsedCut` that happens over the party menu: the tile check and its
    /// refusal. What can be cut is named for the overworld, which does the rest once the start menu
    /// has closed.
    fn cut(&mut self, ctx: &mut Ctx) -> Transition {
        if ctx.world.badges & CASCADE_BADGE == 0 {
            return self.new_badge_required();
        }
        let tileset = MapHeader::read(ctx.world.location.map).expect("the player stands on a map with a header").tileset;
        let Some(tile) = cut_tile(tileset, ctx.world.location.ahead.tile) else {
            return self.text(decode(local::UsedCut::NothingToCutText).expect("the text decodes"), false);
        };
        self.name_the_mon(ctx);
        ctx.world.location.used_field_move = Some(UsedFieldMove::Cut(tile));
        Transition::Pop(Outcome::Chosen(PokemonMoveName::Cut as u8))
    }

    /// `.fly`: the badge, `CheckIfInOutsideMap`, then the town map to choose a town on.
    fn fly(&mut self, ctx: &mut Ctx) -> Transition {
        if ctx.world.badges & THUNDER_BADGE == 0 {
            return self.new_badge_required();
        }
        let tileset = MapHeader::read(ctx.world.location.map).expect("the player stands on a map with a header").tileset;
        if !is_outside(tileset) {
            self.name_the_mon(ctx);
            return self.text(far_text("_CannotFlyHereText").expect("the text is in the cartridge"), false);
        }
        self.phase = Phase::TownMap;
        Transition::Push(Mode::TownMap(TownMap::fly()))
    }

    /// `.dig`, which is `ItemUseEscapeRope` with `wPseudoItemID` set, so no rope is spent and the
    /// tileset is the whole of the test: there is no badge check.
    fn dig(&mut self, ctx: &mut Ctx) -> Transition {
        let tileset = MapHeader::read(ctx.world.location.map).expect("the player stands on a map with a header").tileset;
        if !escape_rope_allowed(ctx.world.location.map, tileset) {
            return self.text(far_text("_ItemUseNotTimeText").expect("the text is in the cartridge"), false);
        }
        arm_escape_warp(ctx);
        Transition::Pop(Outcome::Chosen(PokemonMoveName::Dig as u8))
    }

    /// `.teleport`: `CheckIfInOutsideMap`, its text, then sixty frames before the start menu closes.
    /// There is no badge check here either.
    fn teleport(&mut self, ctx: &mut Ctx) -> Transition {
        let tileset = MapHeader::read(ctx.world.location.map).expect("the player stands on a map with a header").tileset;
        if !is_outside(tileset) {
            self.name_the_mon(ctx);
            return self.text(far_text("_CannotUseTeleportNowText").expect("the text is in the cartridge"), false);
        }
        self.phase = Phase::Teleport;
        Transition::Push(Mode::TextBox(TextBox::script(far_text("_WarpToLastPokemonCenterText").expect("the text is in the cartridge"))))
    }

    /// `.softboiled`: a fifth of the chosen mon's max HP is what it can give, and it has to have
    /// more than that left. `ItemUseMedicine` does the rest as a Potion of that size.
    fn softboiled(&mut self, ctx: &mut Ctx) -> Transition {
        let mon = &ctx.world.party[self.slot as usize].mon;
        if mon.stats[0] / 5 >= mon.mon.hp {
            return self.text(far_text("_NotHealthyEnoughText").expect("the text is in the cartridge"), false);
        }
        self.phase = Phase::Softboiled;
        Transition::Push(Mode::UseItem(UseItem::softboiled(self.slot)))
    }

    /// `.strength` into `PrintStrengthText`. The flag is the whole of it: the overworld's
    /// `TryPushingBoulder` reads it every pass.
    fn strength(&mut self, ctx: &mut Ctx) -> Transition {
        if ctx.world.badges & RAINBOW_BADGE == 0 {
            return self.new_badge_required();
        }
        ctx.world.location.strength_active = true;
        // `.strength` calls no `GetPartyMonName`, so both its texts read whichever name
        // `RedrawPartyMenu_` left in `wNameBuffer`: the last mon drawn, not the one chosen.
        self.phase = Phase::StrengthCry;
        Transition::Push(Mode::TextBox(TextBox::script(decode(pokered_symbols::UsedStrengthText).expect("the text decodes"))))
    }

    /// `.flash`. `wMapPalOffset` is the overworld's, so lighting the map is left to it.
    fn flash(&mut self, ctx: &mut Ctx) -> Transition {
        if ctx.world.badges & BOULDER_BADGE == 0 {
            return self.new_badge_required();
        }
        ctx.world.location.used_field_move = Some(UsedFieldMove::Flash);
        self.text(decode(local::StartMenu_Pokemon::flashLightsAreaText).expect("the text decodes"), true)
    }
}

impl PokemonMenu {
    /// `.surf`.
    fn surf(&mut self, ctx: &mut Ctx) -> Transition {
        self.name_the_mon(ctx);
        if ctx.world.badges & SOUL_BADGE == 0 {
            return self.new_badge_required();
        }
        if let Some(text) = surfing_refusal(&ctx.world.location, &ctx.world.events) {
            return self.text(decode(text).expect("the text decodes"), false);
        }
        let used = item_use_surfboard(ctx);
        if used.music {
            self.phase = Phase::Music { text: used.text, result: used.result };
            return self.update(ctx);
        }
        self.after_music(used.text, used.result)
    }

    fn text(&mut self, text: Vec<poke_core::text_script::TextCommand>, result: bool) -> Transition {
        self.phase = Phase::Text { result };
        Transition::Push(Mode::TextBox(TextBox::script(text)))
    }

    fn after_music(&mut self, text: Option<DmgPointer>, result: bool) -> Transition {
        match text {
            Some(text) => self.text(decode(text).expect("the text decodes"), result),
            None => self.after_text(result),
        }
    }

    /// `.goBackToMap` once the move's text is over, or `.loop` where it was refused.
    fn after_text(&mut self, result: bool) -> Transition {
        if let Some(used) = self.field_move.filter(|_| result) {
            return Transition::Pop(Outcome::Chosen(used as u8));
        }
        self.field_move = None;
        self.back_to_the_list()
    }

    /// `.loop`: `GoBackToPartyMenu`.
    fn back_to_the_list(&mut self) -> Transition {
        self.phase = Phase::PartyMenu;
        Transition::Push(Mode::PartyMenu(PartyMenu::again(PartyMenuType::Normal)))
    }
}

impl ModeUpdate for PokemonMenu {
    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        if ctx.world.party.is_empty() {
            return Transition::Pop(Outcome::Done);
        }
        self.from_the_top()
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Music { .. } if !ctx.audio.sound_finished() => Transition::Stay,
            Phase::Music { text, result } => {
                crate::modes::overworld::play_default_music(ctx);
                self.after_music(text, result)
            }
            Phase::StrengthSound if !ctx.audio.sound_finished() => Transition::Stay,
            Phase::StrengthSound => {
                self.phase = Phase::StrengthBeat(CRY_BEAT);
                Transition::Stay
            }
            Phase::StrengthBeat(1) => {
                self.phase = Phase::Text { result: true };
                Transition::Push(Mode::TextBox(TextBox::script(decode(pokered_symbols::CanMoveBouldersText).expect("the text decodes"))))
            }
            Phase::StrengthBeat(frames) => {
                self.phase = Phase::StrengthBeat(frames - 1);
                Transition::Stay
            }
            Phase::TeleportDelay(1) => Transition::Pop(Outcome::Chosen(PokemonMoveName::Teleport as u8)),
            Phase::TeleportDelay(frames) => {
                self.phase = Phase::TeleportDelay(frames - 1);
                Transition::Stay
            }
            _ => Transition::Stay,
        }
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
            (Phase::FieldMoveMenu, _) => self.back_to_the_list(),
            (Phase::Text { result }, _) => self.after_text(result),
            // `.goBackToMap` if a town was chosen, and otherwise `StartMenu_Pokemon` from the top.
            (Phase::TownMap, _) if ctx.world.location.fly_warp.is_some() =>
                Transition::Pop(Outcome::Chosen(PokemonMoveName::Fly as u8)),
            (Phase::TownMap, _) => self.from_the_top(),
            // `UsedStrengthText`'s `text_asm`: the cry the cartridge plays is whatever
            // `wCurPartySpecies` was last left as, and the chosen mon's is the one that reads right.
            (Phase::StrengthCry, _) => {
                let species = ctx.world.party[self.slot as usize].mon.mon.species;
                ctx.audio.play_cry(species as u8);
                self.phase = Phase::StrengthSound;
                Transition::Stay
            }
            // The bits are set after the text, not before it.
            (Phase::Teleport, _) => {
                arm_escape_warp(ctx);
                self.phase = Phase::TeleportDelay(TELEPORT_FRAMES);
                Transition::Stay
            }
            (Phase::Softboiled, _) => self.back_to_the_list(),
            (Phase::Music { .. } | Phase::StrengthSound | Phase::StrengthBeat(_) | Phase::TeleportDelay(_), _) =>
                Transition::Stay,
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

    /// One mon knowing `mv` alone, on `map`, with the party menu open over nothing else.
    fn with_move(mv: PokemonMoveName, map: poke_core::map::Map, edit: impl FnOnce(&mut World)) -> Game {
        let mut party = two();
        party[0].mon.mon.moves[0] = Some(mv);
        let mut world = World { party, player_name: encode("RED").unwrap(), ..World::default() };
        world.location.map = map;
        world.location.last_blackout_map = poke_core::map::Map::ViridianCity;
        edit(&mut world);
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::PokemonMenu(PokemonMenu::new()));
        game
    }

    /// The first mon's only field move, chosen.
    fn choose_the_move(game: &mut Game) {
        until(game, Decision::PartyMenu);
        press(game, Joypad::A);
        until(game, Decision::FieldMoveMenu);
        press(game, Joypad::A);
    }

    #[test]
    fn teleport_indoors_says_so_and_leaves_the_list_up() {
        let mut game = with_move(PokemonMoveName::Teleport, poke_core::map::Map::RedsHouse2F, |_| {});
        choose_the_move(&mut game);
        until(&mut game, Decision::Text);
        assert!(text_row(&game, 14).starts_with(&encode("BIRD can").unwrap()), "the mon is named in the refusal");
        press(&mut game, Joypad::A);
        until(&mut game, Decision::PartyMenu);
        assert!(!game.world().location.escape_warp);
    }

    #[test]
    fn teleport_outdoors_arms_the_escape_warp_after_its_text_and_closes_the_menu() {
        let mut game = with_move(PokemonMoveName::Teleport, poke_core::map::Map::PalletTown, |_| {});
        choose_the_move(&mut game);
        // `_WarpToLastPokemonCenterText` ends in `done`, so nothing waits for a press here.
        assert!(!game.world().location.escape_warp, "the bits are set after the text, not before it");
        for _ in 0..400 {
            game.frame(Input::None);
            if game.modes().is_empty() {
                break;
            }
        }
        assert!(game.modes().is_empty(), "the start menu closes");
        assert!(game.world().location.escape_warp);
        assert_eq!(game.world().location.fly_warp, Some(poke_core::map::Map::ViridianCity));
    }

    #[test]
    fn dig_needs_a_tileset_it_works_on_and_no_badge() {
        let mut outside = with_move(PokemonMoveName::Dig, poke_core::map::Map::PalletTown, |_| {});
        choose_the_move(&mut outside);
        until(&mut outside, Decision::Text);
        assert!(text_row(&outside, 14).starts_with(&encode("OAK: RED").unwrap()), "not the time for it out in the open");

        let mut cave = with_move(PokemonMoveName::Dig, poke_core::map::Map::MtMoon1F, |_| {});
        choose_the_move(&mut cave);
        for _ in 0..200 {
            cave.frame(Input::None);
            if cave.modes().is_empty() {
                break;
            }
        }
        assert!(cave.modes().is_empty(), "no badge is asked for");
        assert!(cave.world().location.escape_warp);
        assert_eq!(cave.world().location.fly_warp, Some(poke_core::map::Map::ViridianCity));
    }

    #[test]
    fn softboiled_refuses_a_giver_with_a_fifth_of_its_max_hp_or_less() {
        let mut game = with_move(PokemonMoveName::Softboiled, poke_core::map::Map::PalletTown, |world| {
            world.party[0].mon.mon.hp = world.party[0].mon.stats[0] / 5;
        });
        choose_the_move(&mut game);
        until(&mut game, Decision::Text);
        assert_eq!(text_row(&game, 14), encode("Not healthy").unwrap());
        press(&mut game, Joypad::A);
        until(&mut game, Decision::PartyMenu);
    }

    #[test]
    fn softboiled_pays_a_fifth_of_the_giver_s_max_hp_into_another_mon() {
        let mut game = with_move(PokemonMoveName::Softboiled, poke_core::map::Map::PalletTown, |world| {
            world.party[1].mon.mon.hp = 1;
        });
        let share = game.world().party[0].mon.stats[0] / 5;
        let giver_hp = game.world().party[0].mon.mon.hp;
        choose_the_move(&mut game);
        // The giver cannot be its own target: choosing it asks again.
        until(&mut game, Decision::PartyMenu);
        press(&mut game, Joypad::A);
        until(&mut game, Decision::PartyMenu);
        press(&mut game, Joypad::DOWN);
        press(&mut game, Joypad::A);
        for _ in 0..900 {
            game.frame(Input::None);
            if game.status() == Status::Waiting(Decision::Text) {
                break;
            }
        }
        assert_eq!(game.world().party[0].mon.mon.hp, giver_hp - share, "the giver pays");
        assert_eq!(game.world().party[1].mon.mon.hp, 1 + share, "and the other mon is healed by it");
    }

    /// A mon with SURF alone on the list, the player on Pallet Town's shore facing the pond.
    fn surfer(badges: u8, ahead: crate::systems::overworld::location::Ahead, state: u8) -> Game {
        let mut party = two();
        party[0].mon.mon.moves[0] = Some(PokemonMoveName::Surf);
        let mut world = World { party, player_name: encode("RED").unwrap(), badges, ..World::default() };
        world.location.map = poke_core::map::Map::PalletTown;
        world.location.ahead = ahead;
        world.location.walk_bike_surf = state;
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::PokemonMenu(PokemonMenu::new()));
        until(&mut game, Decision::PartyMenu);
        command(&mut game, Command::ChooseOption(0));
        until(&mut game, Decision::FieldMoveMenu);
        game.frame(Input::Command(Command::ChooseOption(0)));
        game
    }

    const WATER: u8 = 0x14;
    /// Passable ground on the overworld tileset, and not a shore.
    const LAND: u8 = 0x2C;

    fn text_row(game: &Game, y: usize) -> Vec<u8> {
        let row = game.ui().row(y)[1..18].to_vec();
        let end = row.iter().rposition(|&tile| tile != crate::gfx::ui::UiSurface::BLANK).map_or(0, |i| i + 1);
        row[..end].to_vec()
    }

    #[test]
    fn surf_without_the_soul_badge_is_refused_and_the_list_comes_back() {
        use crate::systems::overworld::location::{Ahead, WALKING};
        let mut game = surfer(0, Ahead { tile: WATER, standing_on: LAND, sprite: false }, WALKING);
        until(&mut game, Decision::Text);
        assert_eq!(text_row(&game, 14), encode("No! A new BADGE").unwrap());
        command(&mut game, Command::Advance);
        until(&mut game, Decision::PartyMenu);
    }

    #[test]
    fn surf_onto_the_water_ahead_answers_for_the_overworld() {
        use crate::systems::overworld::location::{Ahead, SURFING, WALKING};
        let mut game = surfer(0xFF, Ahead { tile: WATER, standing_on: LAND, sprite: false }, WALKING);
        until(&mut game, Decision::Text);
        assert_eq!(text_row(&game, 14), encode("RED got on").unwrap());
        assert_eq!(text_row(&game, 16)[..5], encode("BIRD!").unwrap()[..]);
        assert_eq!(game.world().location.walk_bike_surf, SURFING);
        command(&mut game, Command::Advance);
        assert!(game.modes().is_empty());
    }

    #[test]
    fn surf_on_land_and_off_the_water_into_a_wall_are_both_refused() {
        use crate::systems::overworld::location::{Ahead, SURFING, WALKING};
        let mut game = surfer(0xFF, Ahead { tile: LAND, standing_on: LAND, sprite: false }, WALKING);
        until(&mut game, Decision::Text);
        assert_eq!(text_row(&game, 14), encode("No SURFing on").unwrap());
        command(&mut game, Command::Advance);
        until(&mut game, Decision::PartyMenu);

        let mut game = surfer(0xFF, Ahead { tile: LAND, standing_on: WATER, sprite: true }, SURFING);
        until(&mut game, Decision::Text);
        assert_eq!(text_row(&game, 14), encode("There's no place").unwrap());
        command(&mut game, Command::Advance);
        assert!(game.modes().is_empty(), "the menus close all the same");
        assert_eq!(game.world().location.walk_bike_surf, SURFING);
    }

    /// `.strength` calls no `GetPartyMonName`, so its texts read whatever `RedrawPartyMenu_`'s loop
    /// left in `wNameBuffer`: the last mon on the list, not the one chosen.
    #[test]
    fn strength_names_the_last_mon_on_the_list_rather_than_the_one_chosen() {
        let mut party = two();
        party.push(mon(PokemonSpecies::Ivysaur, "IVY"));
        party[0].mon.mon.moves[0] = Some(PokemonMoveName::Strength);
        let world = World { party, player_name: encode("RED").unwrap(), badges: 0xFF, ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::PokemonMenu(PokemonMenu::new()));
        until(&mut game, Decision::PartyMenu);
        command(&mut game, Command::ChooseOption(0));
        until(&mut game, Decision::FieldMoveMenu);
        game.frame(Input::Command(Command::ChooseOption(0)));
        // The first text is not held at a `▼`; the second is.
        let mut used = Vec::new();
        for _ in 0..600 {
            if game.status() == Status::Waiting(Decision::Text) {
                break;
            }
            game.frame(Input::None);
            let row = text_row(&game, 14);
            if row.starts_with(&encode("IVY used").unwrap()) {
                used = row;
            }
        }
        assert_eq!(used, encode("IVY used").unwrap());
        assert_eq!(text_row(&game, 14), encode("IVY can").unwrap());
        assert!(game.world().location.strength_active);
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
