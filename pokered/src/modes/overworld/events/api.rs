//! What a map's code calls, and the hidden objects and text predefs only one map reaches: Oak's aides,
//! the Pokédex rating, the fossils, the prize vendor, the elevators, the diploma, Bill's PC, the
//! Cinnabar quiz, the Viridian school, the Vermilion trash cans and the pictures in a box.

use poke_core::item::{self, ItemId};
use poke_core::map::Map;
use poke_core::mon_gfx::pic_shades;
use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::species::PokemonSpecies;
use poke_core::sprite::SpriteFacing;
use poke_core::symbols::pokered_events::*;
use poke_core::symbols::pokered_map_scripts::{TEXT_GAMECORNERPRIZEROOM_PRIZE_VENDOR_1,
    TEXT_POKEMONMANSION1F_SWITCH, TEXT_POKEMONMANSION2F_SWITCH, TEXT_POKEMONMANSION3F_SWITCH,
    TEXT_POKEMONMANSIONB1F_SWITCH};
use poke_core::symbols::pokered_local_labels::GiveFossilToCinnabarLab as lab;
use poke_core::symbols::{pokered_symbols as sym, DmgPointer};
use poke_core::text_script::{TextBuffer, TextNumber};
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::gfx::layers::Object;
use crate::gfx::mon_icons::clear_sprites;
use crate::gfx::text_boxes::TextBoxId;
use crate::gfx::tiles::{V_CHARS0, V_CHARS1, V_CHARS2};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::input::Joypad;
use crate::mode::{Mode, Outcome};
use crate::modes::cursor_menu::CursorMenu;
use crate::modes::list_menu::ListMenu;
use crate::modes::pokedex::PokedexMenu;
use crate::modes::slots::SlotMachine;
use crate::rng::Rng;
use crate::systems::events::hidden_events::HiddenEvent;
use crate::systems::events::tables::{self, OaksAideResult, PrizeWindow};
use crate::systems::math::sub_bcd;
use crate::systems::pokedex::{count_set_bits, index_to_pokedex, pic_tiles};
use crate::systems::print_num::{print_bcd, BcdFormat};
use crate::systems::slots;
use super::super::script::{Block, Flow, Routine, Script, Then};
use super::hidden::{predef_in, print_without_box};
use super::{after_yes_no, place_rom_string, print, restore_screen_tiles_and_reload_tile_patterns, save_screen_tiles_to_buffer2,
    yes_no, Label};

/// A picture `DisplayMonFrontSpriteInBox` shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Picture {
    Mon(PokemonSpecies),
    /// `FOSSIL_KABUTOPS` and `FOSSIL_AERODACTYL`, which `GetMonHeader` gives pictures of their own.
    FossilKabutops,
    FossilAerodactyl,
}

impl Script<'_, '_> {
    /// `DoInGameTradeDialogue` for `TRADE_FOR_*`.
    pub fn do_in_game_trade_dialogue(&mut self, which: u8) -> Then {
        Then::call(Label::InGameTrade(which))
    }

    /// `DaycareGentlemanText`.
    pub fn day_care_gentleman(&mut self) -> Then {
        Then::call(Label::DayCare)
    }

    /// `OaksAideScript` with `hOaksAideRequirement` and `hOaksAideRewardItem`, the item's name put
    /// where the text reads it as the map's code does first. [`Script::oaks_aide_result`] reads the end.
    pub fn oaks_aide(&mut self, requirement: u8, reward: ItemId) -> Then {
        let text = &mut self.ctx.world.text;
        text.numbers.insert(TextNumber::OaksAideRequirement, requirement as u32);
        text.strings.insert(TextBuffer::OaksAideRewardItemName, item::name(reward));
        self.ow.rt.events.oaks_aide = (requirement, reward as u8);
        Then::call(Label::OaksAide)
    }

    /// `hOaksAideResult`.
    pub fn oaks_aide_result(&self) -> Option<OaksAideResult> {
        self.ow.rt.events.oaks_aide_result
    }

    /// `DisplayDexRating`. With `EVENT_HALL_OF_FAME_DEX_RATING` set it only works the rating out for
    /// the Hall of Fame, in [`Script::dex_rating_text`] and the two counts.
    pub fn display_dex_rating(&mut self) -> Then {
        Then::call(Label::DexRating)
    }

    /// `wDexRatingText`'s source, as the Hall of Fame's rating left it.
    pub fn dex_rating_text(&self) -> Option<DmgPointer> {
        self.ow.rt.events.dex_rating_text
    }

    /// `StarterDex`: the data page of `dex` with the three starters and Ivysaur owned for its length.
    pub fn starter_dex(&mut self, dex: u8) -> Then {
        Then::call(Label::StarterDex(dex))
    }

    /// `DisplayPokedex`: a species' data page from outside the dex, and the species marked seen.
    pub fn display_pokedex(&mut self, species: PokemonSpecies) -> Then {
        Then::call(Label::DisplayPokedex(species))
    }

    /// `GiveFossilToCinnabarLab` over `wFilteredBagItems`, the fossils in the bag.
    pub fn give_fossil_to_cinnabar_lab(&mut self, fossils: Vec<ItemId>) -> Then {
        self.ow.rt.events.filtered_bag_items = fossils;
        Then::call(Label::CinnabarLab)
    }

    /// `LoadFossilItemAndMonName`, for `wFossilItem` and `wFossilMon`.
    pub fn load_fossil_item_and_mon_name(&mut self) {
        if let Some((fossil, mon)) = self.ctx.world.fossil {
            let strings = &mut self.ctx.world.text.strings;
            strings.insert(TextBuffer::StringBuffer, mon.name());
            strings.insert(TextBuffer::NameBuffer, item::name(fossil));
        }
    }

    /// `wFossilMon`: what the lab is reviving, if anything has been handed over.
    pub fn fossil_mon(&self) -> Option<PokemonSpecies> {
        self.ctx.world.fossil.map(|(_, mon)| mon)
    }

    /// `RemoveGuardDrink`.
    pub fn remove_guard_drink(&mut self) -> Option<ItemId> {
        tables::remove_guard_drink(&mut self.ctx.world.bag)
    }

    /// `PewterGuys` with `wWhichPewterGuy`: the presses that line the player up behind a guide, put on
    /// the simulated presses the map's code has started.
    pub fn pewter_guys(&mut self, which: u8) {
        let (x, y) = (self.x(), self.y());
        let mut presses: Vec<u8> = self.ow.simulated.iter().map(|press| press.bits()).collect();
        presses.truncate(self.ow.simulated_index as usize);
        tables::pewter_guys(which, x, y, &mut presses);
        self.ow.simulated_index = presses.len() as u8;
        self.ow.simulated = presses.into_iter().map(Joypad::from_bits_truncate).collect();
    }

    /// `DisplayElevatorFloorMenu` over `wItemList` (the floors) and `wElevatorWarpMaps` (for each, the
    /// warp and map its doors lead to). A floor chosen rewrites the map's first two warps and sets
    /// [`Script::used_elevator`].
    pub fn display_elevator_floor_menu(&mut self, floors: Vec<ItemId>, warps: Vec<(u8, Map)>) -> Then {
        self.ow.rt.events.elevator = (floors, warps);
        Then::call(Label::ElevatorFloorMenu)
    }

    /// `bit BIT_CUR_MAP_USED_ELEVATOR` then `res`.
    pub fn check_and_reset_used_elevator(&mut self) -> bool {
        std::mem::take(&mut self.ow.rt.events.used_elevator)
    }

    /// `ShakeElevator`.
    pub fn shake_elevator(&mut self) -> Then {
        Then::call(Label::ShakeElevator)
    }

    /// `DisplayDiploma`.
    pub fn display_diploma(&mut self) -> Then {
        Then::call(Label::Diploma)
    }

    /// `UpdateCinnabarGymGateTileBlocks_`.
    pub fn update_cinnabar_gym_gate_tile_blocks(&mut self) {
        update_cinnabar_gym_gate_tile_blocks(self);
    }

    /// `wOpponentAfterWrongAnswer`: the quiz's trainer who fights a wrong answer, or 0.
    pub fn opponent_after_wrong_answer(&self) -> u8 {
        self.ow.rt.events.opponent_after_wrong_answer
    }

    /// `ld [wOpponentAfterWrongAnswer], a`, which the gym's own script clears once he has fought.
    pub fn set_opponent_after_wrong_answer(&mut self, opponent: u8) {
        self.ow.rt.events.opponent_after_wrong_answer = opponent;
    }

    /// `wCardKeyDoorX` and `wCardKeyDoorY`, in blocks.
    pub fn card_key_door(&self) -> (u8, u8) {
        self.ow.rt.events.card_key_door
    }

    /// `HealParty`.
    pub fn heal_party(&mut self) {
        crate::systems::events::heal_party::heal_party(&mut self.ctx.world.party);
    }

    /// `GameCornerSelectLuckySlotMachine`: which of the twelve machines pays better, rolled once a
    /// load of the Game Corner. A machine asks for it as well, so a map whose script does not call
    /// it still gets its roll.
    pub fn game_corner_select_lucky_slot_machine(&mut self) {
        if self.check_and_reset_cur_map_loaded(2) {
            self.ow.rt.events.lucky_slot_machine = slots::lucky_slot_machine(self.ctx.rng);
        }
    }
}

/// A hidden event whose function `hidden.rs` does not recreate.
pub(super) fn hidden_event(s: &mut Script, event: HiddenEvent) -> Flow {
    let f = event.function;
    let up = s.ow.sprites[0].facing == SpriteFacing::Up as u8;
    if f == sym::PrintCinnabarQuiz {
        if !up {
            return Flow::Return;
        }
        s.enable_auto_text_box_drawing();
        return predef_in(f, 0x31).ret();
    }
    if f == sym::BillsHousePC {
        s.enable_auto_text_box_drawing();
        if !up {
            return Flow::Return;
        }
        if s.check_event(EVENT_LEFT_BILLS_HOUSE_AFTER_HELPING) {
            s.set_do_not_wait_for_button_press(true);
            return predef_in(f, 0x2F).ret();
        }
        if !s.check_event(EVENT_USED_CELL_SEPARATOR_ON_BILL) && s.check_event(EVENT_BILL_SAID_USE_CELL_SEPARATOR) {
            s.set_do_not_wait_for_button_press(true);
            return predef_in(f, 0x2E).then(Label::CellSeparator(0));
        }
        return predef_in(f, 0x2D).ret();
    }
    if f == sym::AerodactylFossil || f == sym::KabutopsFossil {
        let (picture, text) = if f == sym::AerodactylFossil { (Picture::FossilAerodactyl, 0x09) } else { (Picture::FossilKabutops, 0x0B) };
        return Then::call(Label::MonPopup(picture)).then(Label::PredefAfterPopup { id: text, bank: bank(f) });
    }
    if f == sym::Route15GateLeftBinoculars {
        if !up {
            return Flow::Return;
        }
        s.enable_auto_text_box_drawing();
        return predef_in(f, 0x0A).then(Label::Binoculars);
    }
    if f == sym::GymTrashScript {
        return gym_trash(s, event);
    }
    if f == sym::StartSlotMachine {
        return start_slot_machine(s, event);
    }
    // A switch is only a switch from below it; the wall it moves is its map's own business.
    let switches = [(sym::Mansion1Script_Switches, TEXT_POKEMONMANSION1F_SWITCH),
        (sym::Mansion2Script_Switches, TEXT_POKEMONMANSION2F_SWITCH),
        (sym::Mansion3Script_Switches, TEXT_POKEMONMANSION3F_SWITCH),
        (sym::Mansion4Script_Switches, TEXT_POKEMONMANSIONB1F_SWITCH)];
    if let Some(&(_, text)) = switches.iter().find(|&&(at, _)| at == f) {
        if !up {
            return Flow::Return;
        }
        s.clear_joy_held();
        return s.display_text_id(text).ret();
    }
    // The Cable Club's Game Boys are a non-goal.
    Flow::Return
}

fn bank(at: DmgPointer) -> u8 {
    match at.bank {
        poke_core::symbols::DmgBank::ROM { bank } => bank,
        _ => unreachable!("code is in ROM"),
    }
}

/// A text predef that runs code, not one `hidden.rs` handles.
pub(super) fn predef_text(s: &mut Script, at: DmgPointer) -> Option<Flow> {
    Some(if at == sym::CinnabarGymQuiz {
        let argument = match s.ow.rt.events.hidden {
            Some(super::Hidden::Event(event)) => event.argument,
            _ => 0,
        };
        s.ow.rt.events.opponent_after_wrong_answer = 0;
        s.ow.rt.events.gym_gate = (argument & 0xF, argument >> 4);
        print(sym::CinnabarGymQuizIntroText).then(Label::QuizQuestion)
    } else if at == sym::ViridianSchoolNotebook {
        print(sym::ViridianSchoolNotebookText1).then(Label::NotebookTurnPage(1))
    } else if at == sym::ViridianSchoolBlackboard {
        s.ow.rt.saved_screen = Some(s.ctx.screen.ui.clone());
        s.ow.rt.events.menu = (0, 0);
        s.ctx.menu.last_item = 0;
        print(sym::ViridianSchoolBlackboardText1).then(Label::BlackboardLoop)
    } else if at == sym::LinkCableHelp {
        s.ow.rt.saved_screen = Some(s.ctx.screen.ui.clone());
        s.ow.rt.events.menu = (0, 0);
        s.ctx.menu.last_item = 0;
        print(sym::LinkCableHelpText1).then(Label::LinkCableHelpLoop)
    } else if at == sym::BillsHousePokemonList {
        s.ow.rt.saved_screen = Some(s.ctx.screen.ui.clone());
        s.ow.rt.events.menu = (0, 0);
        s.ctx.menu.last_item = 0;
        print(sym::BillsHousePokemonListText1).then(Label::BillsListLoop)
    } else if at == sym::BillsHouseInitiatedText {
        print_without_box(at).then(Label::BillsHouseInitiated)
    } else if at == sym::VermilionGymTrashSuccessText1 {
        print_without_box(at).then(Label::TrashSound(sounds::SFX_SWITCH.0))
    } else if at == sym::VermilionGymTrashSuccessText3 {
        print_without_box(at).then(Label::TrashSound(sounds::SFX_GO_INSIDE.0))
    } else if at == sym::VermilionGymTrashFailText {
        print_without_box(at).then(Label::TrashSound(sounds::SFX_DENIED.0))
    } else {
        return None;
    })
}

pub(super) fn resume(s: &mut Script, label: Label) -> Flow {
    use Label::*;
    match label {
        MonPopup(picture) => mon_popup(s, picture),
        MonPopupStage(stage) => mon_popup_stage(s, stage),
        MonPopupDone => {
            if let Some(saved) = s.ow.rt.saved_screen.take() {
                s.ctx.screen.ui = saved;
            }
            Flow::Return
        }
        PredefAfterPopup { id, bank } => {
            s.enable_auto_text_box_drawing();
            Then::call(Routine::PrintPredefTextId { id, bank }).ret()
        }
        Binoculars => {
            s.ctx.audio.play_cry(PokemonSpecies::Articuno as u8);
            s.wait_for_sound_to_finish().then(MonPopup(Picture::Mon(PokemonSpecies::Articuno)))
        }
        CellSeparator(step) => cell_separator(s, step),
        CellSeparatorSound(step) => {
            s.play_sound(CELL_SEPARATOR[step as usize].1);
            s.wait_for_sound_to_finish().then(CellSeparator(step + 1))
        }
        BillsHouseInitiated => {
            s.ctx.audio.play_new_sound(SoundId::STOP_ALL_MUSIC);
            s.delay_frames(16).then(BillsHouseInitiatedSwitch)
        }
        BillsHouseInitiatedSwitch => {
            s.play_sound(sounds::SFX_SWITCH);
            s.wait_for_sound_to_finish().then(BillsHouseInitiatedDone)
        }
        BillsHouseInitiatedDone => s.delay_frames(60).ret(),
        BillsListLoop => bills_list_loop(s),
        BillsListMenu => {
            s.ctx.world.no_text_delay = true;
            save_screen_tiles_to_buffer2(s);
            menu(s, 4, (1, 2), BillsListChosen)
        }
        BillsListChosen => bills_list_chosen(s),
        BillsListDexShown => {
            if let Some(saved) = s.ow.rt.saved_screen2.clone() {
                s.ctx.screen.ui = saved;
            }
            Flow::Jump(BillsListLoop.into())
        }
        DisplayPokedex(species) => display_pokedex(s, species),
        DisplayPokedexShown(species) => {
            s.ctx.world.no_text_delay = false;
            s.ctx.screen.tiles.load_text_box_tiles();
            s.ctx.screen.tiles.load_tileset(s.ow.view.tileset);
            s.ctx.world.pokedex.set_seen(species);
            s.set_do_not_wait_for_button_press(true);
            Flow::Return
        }
        StarterDex(dex) => {
            s.ctx.world.pokedex.owned[0] = 1 << (index_to_pokedex(PokemonSpecies::Bulbasaur as u8) - 1)
                | 1 << (index_to_pokedex(PokemonSpecies::Ivysaur as u8) - 1)
                | 1 << (index_to_pokedex(PokemonSpecies::Charmander as u8) - 1)
                | 1 << (index_to_pokedex(PokemonSpecies::Squirtle as u8) - 1);
            let page = PokedexMenu::data_page(dex);
            Then::block(Block::Mode(Box::new(Mode::Pokedex(page)))).then(StarterDexShown)
        }
        StarterDexShown => {
            s.ctx.world.pokedex.owned[0] = 0;
            Flow::Return
        }

        QuizQuestion => {
            let (index, _) = s.ow.rt.events.gym_gate;
            let questions = sym::CinnabarQuizQuestions;
            let pointer = rom_slice(questions + 2 * (index as u16).wrapping_sub(1));
            let question = DmgPointer { bank: questions.bank, address: u16::from_le_bytes([pointer[0], pointer[1]]) };
            print(question).then(QuizAsk)
        }
        QuizAsk => {
            s.set_do_not_wait_for_button_press(true);
            yes_no(s).then(QuizAnswered)
        }
        QuizAnswered => quiz_answered(s),
        QuizCorrectText => {
            let (index, _) = s.ow.rt.events.gym_gate;
            if s.check_event(EVENT_CINNABAR_GYM_GATE0_UNLOCKED + index as u16) {
                return Flow::Jump(QuizUnlock.into());
            }
            s.wait_for_sound_to_finish().then(QuizCorrectSound)
        }
        QuizCorrectSound => {
            s.play_sound(sounds::SFX_GO_INSIDE);
            s.wait_for_sound_to_finish().then(QuizUnlock)
        }
        QuizUnlock => {
            let (index, _) = s.ow.rt.events.gym_gate;
            s.set_event(EVENT_CINNABAR_GYM_GATE0_UNLOCKED + index as u16);
            update_cinnabar_gym_gate_tile_blocks(s);
            Flow::Return
        }
        QuizWrongSound => {
            s.play_sound(sounds::SFX_DENIED);
            s.wait_for_sound_to_finish().then(QuizWrongText)
        }
        QuizWrongText => print(sym::CinnabarGymQuizIncorrectText).then(QuizWrongDone),
        QuizWrongDone => {
            let (index, _) = s.ow.rt.events.gym_gate;
            if !s.check_event(EVENT_BEAT_CINNABAR_GYM_TRAINER_0 + index as u16) {
                s.ow.rt.events.opponent_after_wrong_answer = index + 2;
            }
            Flow::Return
        }

        NotebookTurnPage(page) => print(sym::TurnPageText).then(NotebookAsk(page)),
        NotebookAsk(page) => yes_no(s).then(NotebookAnswered(page)),
        NotebookAnswered(page) => {
            if !after_yes_no(s) {
                return Flow::Return;
            }
            match page {
                1 => print(sym::ViridianSchoolNotebookText2).then(NotebookTurnPage(2)),
                2 => print(sym::ViridianSchoolNotebookText3).then(NotebookTurnPage(3)),
                _ => print(sym::ViridianSchoolNotebookText4).then(NotebookLastPage),
            }
        }
        NotebookLastPage => print(sym::ViridianSchoolNotebookText5).ret(),

        BlackboardLoop => {
            s.ctx.world.no_text_delay = true;
            let ui = &mut s.ctx.screen.ui;
            ui.text_box_border(0, 0, 10, 6);
            place_rom_string(ui, 1, 2, sym::StatusAilmentText1);
            place_rom_string(ui, 6, 2, sym::StatusAilmentText2);
            print(sym::ViridianSchoolBlackboardText2).then(BlackboardMenu)
        }
        BlackboardMenu => {
            let (offset, current) = s.ow.rt.events.menu;
            let left = if offset == 0 { 1 } else { 6 };
            let menu = CursorMenu::new(current, 2, (left, 2)).watching(Joypad::LEFT | Joypad::RIGHT);
            Then::block(Block::Mode(Box::new(Mode::CursorMenu(menu)))).then(BlackboardChosen)
        }
        BlackboardChosen => blackboard_chosen(s),

        LinkCableHelpLoop => {
            s.ctx.world.no_text_delay = true;
            let ui = &mut s.ctx.screen.ui;
            ui.text_box_border(0, 0, 13, 8);
            place_rom_string(ui, 2, 2, sym::HowToLinkText);
            print(sym::LinkCableHelpText2).then(LinkCableHelpMenu)
        }
        LinkCableHelpMenu => menu(s, 3, (1, 2), LinkCableHelpChosen),
        LinkCableHelpChosen => {
            let row = match s.ow.rt.outcome {
                Some(Outcome::Chosen(row)) if row != 3 => row,
                _ => return leave_menu(s),
            };
            s.ow.rt.events.menu.1 = row;
            s.ctx.world.no_text_delay = false;
            let pointer = rom_slice(sym::LinkCableInfoTexts + 2 * row as u16);
            let text = DmgPointer { bank: sym::LinkCableInfoTexts.bank, address: u16::from_le_bytes([pointer[0], pointer[1]]) };
            print(text).then(LinkCableHelpLoop)
        }

        TrashSound(sound) => s.wait_for_sound_to_finish().then(TrashSoundPlay(sound)),
        TrashSoundPlay(sound) => {
            s.play_sound(SoundId(sound));
            s.wait_for_sound_to_finish().ret()
        }

        OaksAide => print(sym::OaksAideHiText).then(OaksAideAsk),
        OaksAideAsk => yes_no(s).then(OaksAideAnswered),
        OaksAideAnswered => {
            if !after_yes_no(s) {
                s.ow.rt.events.oaks_aide_result = Some(OaksAideResult::Refused);
                return print(sym::OaksAideComeBackText).ret();
            }
            let owned = count_set_bits(&s.ctx.world.pokedex.owned);
            s.ctx.world.text.numbers.insert(TextNumber::OaksAideNumMonsOwned, owned as u32);
            let (requirement, _) = s.ow.rt.events.oaks_aide;
            if requirement > owned {
                s.ow.rt.events.oaks_aide_result = Some(OaksAideResult::NotEnoughMons);
                return print(sym::OaksAideUhOhText).ret();
            }
            print(sym::OaksAideHereYouGoText).then(OaksAideGive)
        }
        OaksAideGive => {
            let (_, reward) = s.ow.rt.events.oaks_aide;
            let reward = ItemId::from_repr(reward).expect("an aide's reward is an item");
            if !s.give_item(reward, 1) {
                s.ow.rt.events.oaks_aide_result = Some(OaksAideResult::BagFull);
                return print(sym::OaksAideNoRoomText).ret();
            }
            s.ow.rt.events.oaks_aide_result = Some(OaksAideResult::GotItem);
            print(sym::OaksAideGotItemText).ret()
        }

        DexRating => dex_rating(s),
        DexRatingText => {
            let owned = count_set_bits(&s.ctx.world.pokedex.owned);
            print(tables::dex_rating_text(owned)).then(DexRatingSfx)
        }
        DexRatingSfx => s.wait_for_sound_to_finish().then(DexRatingSfxPlay),
        DexRatingSfxPlay => {
            s.ctx.audio.play_new_sound(SoundId::STOP_ALL_MUSIC);
            let owned = count_set_bits(&s.ctx.world.pokedex.owned);
            s.play_music(tables::dex_rating_sound(owned));
            s.play_default_music().then(DexRatingDone)
        }
        DexRatingDone => Then::block(Block::TextScrollButton).ret(),

        CinnabarLab => cinnabar_lab(s),
        CinnabarLabChosen => cinnabar_lab_chosen(s),
        CinnabarLabAsk => yes_no(s).then(CinnabarLabAnswered),
        CinnabarLabAnswered => {
            if !after_yes_no(s) {
                return print(lab::ComeAgainText).ret();
            }
            print(lab::ScientistTakesFossilText).then(CinnabarLabTaken)
        }
        CinnabarLabTaken => {
            if let Some((fossil, _)) = s.ctx.world.fossil
                && let Some(slot) = s.ctx.world.bag.items.iter().position(|item| item.id == fossil)
            {
                s.ctx.world.bag.remove(slot, 1);
            }
            print(lab::GoForAWalkText).then(CinnabarLabDone)
        }
        CinnabarLabDone => {
            s.set_event(EVENT_GAVE_FOSSIL_TO_LAB);
            s.set_event(EVENT_LAB_STILL_REVIVING_FOSSIL);
            Flow::Return
        }

        PrizeMenu => prize_menu(s),
        PrizeMenuShown => prize_menu_shown(s),
        PrizeMenuChosen => prize_menu_chosen(s),
        PrizeAsk => yes_no(s).then(PrizeAnswered),
        PrizeAnswered => prize_answered(s),
        PrizeGiven => {
            if s.added_to_party() {
                return Flow::Jump(PrizeGivenWaited.into());
            }
            Then::block(Block::TextScrollButton).then(PrizeGivenWaited)
        }
        PrizeGivenWaited => {
            if !s.ow.rt.gave_pokemon {
                return prize_menu_done(s);
            }
            subtract_coins(s)
        }

        ElevatorFloorMenu => print(sym::WhichFloorText).then(ElevatorFloorList),
        ElevatorFloorList => {
            s.ow.rt.events.saved_list_scroll = s.ctx.menu.list_scroll;
            let floors = s.ow.rt.events.elevator.0.clone();
            Then::block(Block::Mode(Box::new(Mode::ListMenu(ListMenu::special(floors))))).then(ElevatorChosen)
        }
        ElevatorChosen => elevator_chosen(s),
        ShakeElevator => shake_elevator(s),
        ShakeElevatorStep(n) => shake_elevator_step(s, n),
        ShakeElevatorChime => {
            if s.ctx.audio.channel_sound_id(4) == sounds::SFX_SAFARI_ZONE_PA.0 {
                return Then::block(Block::ChannelPlaying { channel: 4, id: sounds::SFX_SAFARI_ZONE_PA.0 }).then(ShakeElevatorDone);
            }
            Flow::Jump(ShakeElevatorDone.into())
        }
        ShakeElevatorDone => {
            s.update_sprites();
            s.play_default_music().ret()
        }

        SlotsAsk(chance) => yes_no(s).then(SlotsAnswered(chance)),
        SlotsAnswered(chance) => {
            if !after_yes_no(s) {
                return Flow::Jump(SlotsDone.into());
            }
            s.emotion_bubble(0, SMILE_BUBBLE).then(SlotsBubbled(chance))
        }
        SlotsBubbled(chance) => {
            // `wUpdateSpritesEnabled` of `$ff`: the wheels are objects, so nothing may redraw the
            // map's sprites over them.
            s.ow.rt.sprites_frozen = true;
            Then::block(Block::Mode(Box::new(Mode::SlotMachine(SlotMachine::new(chance))))).then(SlotsLeft)
        }
        SlotsLeft => {
            restore_screen_tiles_and_reload_tile_patterns(s);
            Flow::Jump(SlotsDone.into())
        }
        SlotsDone => {
            if let Some(saved) = s.ow.rt.saved_screen2.take() {
                s.ctx.screen.ui = saved;
            }
            Then::call(Routine::CloseTextDisplay).ret()
        }

        Diploma => diploma(s),
        DiplomaDone => {
            s.ctx.world.no_text_delay = false;
            restore_screen_tiles_and_reload_tile_patterns(s);
            s.ctx.screen.effects = Default::default();
            Flow::Return
        }
        _ => unreachable!("{label:?} is not the API's"),
    }
}

/// `HandleMenuInput` over rows already drawn, with `wCurrentMenuItem` where the loop left it.
fn menu(s: &mut Script, max: u8, top: (u8, u8), then: Label) -> Flow {
    let current = s.ow.rt.events.menu.1;
    Then::block(Block::Mode(Box::new(Mode::CursorMenu(CursorMenu::new(current, max, top))))).then(then)
}

/// A menu's `.exit`: `LoadScreenTilesFromBuffer1`, and the text ends.
fn leave_menu(s: &mut Script) -> Flow {
    s.ctx.world.no_text_delay = false;
    if let Some(saved) = s.ow.rt.saved_screen.take() {
        s.ctx.screen.ui = saved;
    }
    Flow::Return
}

// ---- DisplayMonFrontSpriteInBox ----

/// `hStartTileID`: the picture is loaded at `vChars1` tile `$31` and named from `$80`.
const POPUP_BASE: u8 = 0x80;
/// `AnimateSendingOutMon`'s ball, out of battle.
const POPUP_BALL: u8 = 0x4C;
const POPUP_AT: (usize, usize) = (10, 11);

fn mon_popup(s: &mut Script, picture: Picture) -> Flow {
    s.ow.rt.saved_screen = Some(s.ctx.screen.ui.clone());
    TextBoxId::MonSpritePopup.draw(&mut s.ctx.screen.ui);
    s.update_sprites();
    let shades = match picture {
        Picture::Mon(species) => poke_core::mon_gfx::front_pic_shades(species),
        Picture::FossilKabutops => pic_shades(rom_slice(sym::FossilKabutopsPic)),
        Picture::FossilAerodactyl => pic_shades(rom_slice(sym::FossilAerodactylPic)),
    };
    s.ctx.screen.tiles.load(V_CHARS1 + 0x31, &pic_tiles(&shades, false).concat());
    s.ctx.screen.ui.set(POPUP_AT.0, POPUP_AT.1, POPUP_BALL);
    s.delay_frames(3).then(Label::MonPopupStage(1))
}

fn mon_popup_stage(s: &mut Script, stage: u8) -> Flow {
    let (x, y) = POPUP_AT;
    let ui = &mut s.ctx.screen.ui;
    let copy = |ui: &mut UiSurface, x: usize, y: usize, size: usize, table: DmgPointer| {
        let ids = rom_slice(table);
        for row in 0..size {
            for column in 0..size {
                ui.set(x + column, y + row, ids[row * size + column].wrapping_add(POPUP_BASE));
            }
        }
    };
    match stage {
        1 => {
            copy(ui, x - 1, y - 2, 3, sym::DownscaledMonTiles_3x3);
            s.delay_frames(4).then(Label::MonPopupStage(2))
        }
        2 => {
            copy(ui, x - 2, y - 4, 5, sym::DownscaledMonTiles_5x5);
            s.delay_frames(5).then(Label::MonPopupStage(3))
        }
        _ => {
            crate::modes::battle::hud::place_pic(ui, x - 3, y - 6, POPUP_BASE + 0x31);
            Then::block(Block::TextScrollButton).then(Label::MonPopupDone)
        }
    }
}

// ---- Bill's house ----

/// `BillsHousePC`'s `.doCellSeparator`: a wait and a sound each, then the music back.
const CELL_SEPARATOR: [(u8, SoundId); 4] =
    [(32, sounds::SFX_TINK), (80, sounds::SFX_SHRINK), (48, sounds::SFX_TINK), (32, sounds::SFX_GET_ITEM_1)];

fn cell_separator(s: &mut Script, step: u8) -> Flow {
    if let Some(&(frames, _)) = CELL_SEPARATOR.get(step as usize) {
        return s.delay_frames(frames).then(Label::CellSeparatorSound(step));
    }
    s.set_event(EVENT_USED_CELL_SEPARATOR_ON_BILL);
    s.play_default_music().ret()
}

fn bills_list_loop(s: &mut Script) -> Flow {
    s.ctx.world.no_text_delay = true;
    let ui = &mut s.ctx.screen.ui;
    ui.text_box_border(0, 0, 9, 10);
    place_rom_string(ui, 2, 2, sym::BillsMonListText);
    print(sym::BillsHousePokemonListText2).then(Label::BillsListMenu)
}

fn bills_list_chosen(s: &mut Script) -> Flow {
    let row = match s.ow.rt.outcome {
        Some(Outcome::Chosen(row)) => row,
        _ => 4,
    };
    s.ow.rt.events.menu.1 = row;
    let species = PokemonSpecies::from_repr((PokemonSpecies::Eevee as u8).wrapping_add(row));
    match species {
        Some(species @ (PokemonSpecies::Eevee | PokemonSpecies::Flareon | PokemonSpecies::Jolteon | PokemonSpecies::Vaporeon)) =>
            Then::call(Label::DisplayPokedex(species)).then(Label::BillsListDexShown),
        _ => {
            s.ctx.world.no_text_delay = false;
            if let Some(saved) = s.ow.rt.saved_screen2.clone() {
                s.ctx.screen.ui = saved;
            }
            Flow::Return
        }
    }
}

/// `_DisplayPokedex`. `ReloadMapData` and the ten frames after it are loading.
fn display_pokedex(s: &mut Script, species: PokemonSpecies) -> Flow {
    s.ctx.world.no_text_delay = true;
    let page = PokedexMenu::data_page(index_to_pokedex(species as u8));
    Then::block(Block::Mode(Box::new(Mode::Pokedex(page)))).then(Label::DisplayPokedexShown(species))
}

// ---- The Cinnabar quiz ----

fn quiz_answered(s: &mut Script) -> Flow {
    after_yes_no(s);
    let (_, answer) = s.ow.rt.events.gym_gate;
    let chosen = match s.ow.rt.outcome {
        Some(Outcome::Chosen(row)) => row,
        _ => 1,
    };
    if chosen == answer {
        s.ow.rt.cur_map_loaded[0] = true;
        return print(sym::CinnabarGymQuizCorrectText).then(Label::QuizCorrectText);
    }
    s.wait_for_sound_to_finish().then(Label::QuizWrongSound)
}

/// `UpdateCinnabarGymGateTileBlocks_`: each of the six gates open or shut by its event.
fn update_cinnabar_gym_gate_tile_blocks(s: &mut Script) {
    for gate in (1..=6u16).rev() {
        let row = rom_slice(sym::CinnabarGymGateCoords + 4 * (gate - 1));
        let open = s.check_event(EVENT_CINNABAR_GYM_GATE0_UNLOCKED + gate);
        s.replace_tile_block(row[0], row[1], if open { 0x0E } else { row[2] });
    }
}

// ---- The Viridian school's blackboard ----

fn blackboard_chosen(s: &mut Script) -> Flow {
    let (offset, _) = s.ow.rt.events.menu;
    let (row, keys) = match s.ow.rt.outcome {
        Some(Outcome::Chosen(chosen)) => (chosen & CursorMenu::ROW, chosen & !CursorMenu::ROW),
        _ => return leave_menu(s),
    };
    s.ow.rt.events.menu.1 = row;
    if keys & CursorMenu::PRESSED_RIGHT != 0 {
        s.ow.rt.events.menu.0 = 3;
        return Flow::Jump(Label::BlackboardLoop.into());
    }
    if keys & CursorMenu::PRESSED_LEFT != 0 {
        s.ow.rt.events.menu.0 = 0;
        return Flow::Jump(Label::BlackboardLoop.into());
    }
    let status = row + offset;
    if status == 5 {
        return leave_menu(s);
    }
    s.ctx.world.no_text_delay = false;
    let pointer = rom_slice(sym::ViridianBlackboardStatusPointers + 2 * status as u16);
    let text = DmgPointer { bank: sym::ViridianBlackboardStatusPointers.bank, address: u16::from_le_bytes([pointer[0], pointer[1]]) };
    print(text).then(Label::BlackboardLoop)
}

// ---- The Vermilion Gym's trash cans ----

/// `GymTrashScript`: the first lock in the can the gym chose, the second in one next to it, and a
/// wrong can after the first shuts it again somewhere new.
fn gym_trash(s: &mut Script, event: HiddenEvent) -> Flow {
    s.enable_auto_text_box_drawing();
    let f = event.function;
    let can = event.argument;
    if s.check_event(EVENT_2ND_LOCK_OPENED) {
        return predef_in(f, 0x26).ret();
    }
    let [first, second] = s.ctx.world.scripts.trash_cans;
    if s.check_event(EVENT_1ST_LOCK_OPENED) {
        if can == second {
            s.set_event(EVENT_2ND_LOCK_OPENED);
            s.ow.rt.cur_map_loaded[1] = true;
            return predef_in(f, 0x3D).ret();
        }
        s.reset_event(EVENT_1ST_LOCK_OPENED);
        s.ctx.world.scripts.trash_cans[0] = s.ctx.rng.random() & 0x0E;
        return predef_in(f, 0x3E).ret();
    }
    if can != first {
        return predef_in(f, 0x26).ret();
    }
    s.set_event(EVENT_1ST_LOCK_OPENED);
    let entry = sym::GymTrashCans + 5 * can as u16;
    let mask = rom_slice(entry)[0];
    // A mask with no bit in common with the random byte gives `$ff`, and the neighbour is read from
    // past the end of the table.
    let offset = (mask & s.ctx.rng.random().rotate_left(4)).wrapping_sub(1);
    s.ctx.world.scripts.trash_cans[1] = rom_slice(entry + 1 + offset as u16)[0] & 0x0F;
    predef_in(f, 0x3B).ret()
}

// ---- The Pokédex rating ----

fn dex_rating(s: &mut Script) -> Flow {
    let seen = count_set_bits(&s.ctx.world.pokedex.seen);
    let owned = count_set_bits(&s.ctx.world.pokedex.owned);
    let numbers = &mut s.ctx.world.text.numbers;
    numbers.insert(TextNumber::DexRatingNumMonsSeenH, seen as u32);
    numbers.insert(TextNumber::DexRatingNumMonsOwnedH, owned as u32);
    if s.check_event(EVENT_HALL_OF_FAME_DEX_RATING) {
        s.reset_event(EVENT_HALL_OF_FAME_DEX_RATING);
        let numbers = &mut s.ctx.world.text.numbers;
        numbers.insert(TextNumber::DexRatingNumMonsSeen, seen as u32);
        numbers.insert(TextNumber::DexRatingNumMonsOwned, owned as u32);
        s.ow.rt.events.dex_rating_text = Some(tables::dex_rating_text(owned));
        return Flow::Return;
    }
    print(sym::DexCompletionText).then(Label::DexRatingText)
}

// ---- The Cinnabar lab's fossils ----

fn cinnabar_lab(s: &mut Script) -> Flow {
    s.ctx.world.no_text_delay = true;
    let fossils = s.ow.rt.events.filtered_bag_items.clone();
    let count = fossils.len() as u8;
    let ui = &mut s.ctx.screen.ui;
    ui.text_box_border(0, 0, 13, 2 * count as usize);
    s.update_sprites();
    for (i, &fossil) in fossils.iter().enumerate() {
        s.ctx.screen.ui.place(2, 2 + 2 * i, &item::name(fossil));
    }
    s.ctx.world.no_text_delay = false;
    let menu = CursorMenu::new(0, count.wrapping_sub(1), (1, 2));
    Then::block(Block::Mode(Box::new(Mode::CursorMenu(menu)))).then(Label::CinnabarLabChosen)
}

fn cinnabar_lab_chosen(s: &mut Script) -> Flow {
    let Some(Outcome::Chosen(row)) = s.ow.rt.outcome else {
        return print(lab::ComeAgainText).ret();
    };
    let fossil = s.ow.rt.events.filtered_bag_items[row as usize];
    let mon = match fossil {
        ItemId::DomeFossil => PokemonSpecies::Kabuto,
        ItemId::HelixFossil => PokemonSpecies::Omanyte,
        _ => PokemonSpecies::Aerodactyl,
    };
    s.ctx.world.fossil = Some((fossil, mon));
    s.load_fossil_item_and_mon_name();
    print(lab::ScientistSeesFossilText).then(Label::CinnabarLabAsk)
}

// ---- The Game Corner's slot machines ----

/// `SMILE_BUBBLE`.
const SMILE_BUBBLE: u8 = 2;

/// `StartSlotMachine`. A machine the map marks broken only has a line to say; the rest go through
/// `AbleToPlaySlotsCheck`, which wants the player stood beside the machine rather than facing it:
/// `wSpritePlayerStateData1ImageIndex and $8` passes only left and right.
fn start_slot_machine(s: &mut Script, event: HiddenEvent) -> Flow {
    const OUT_OF_ORDER: u8 = 0xFD;
    const OUT_TO_LUNCH: u8 = 0xFE;
    const SOMEONES_KEYS: u8 = 0xFF;
    if let Some(text) = match event.argument {
        OUT_OF_ORDER => Some(0x28),
        OUT_TO_LUNCH => Some(0x29),
        SOMEONES_KEYS => Some(0x2A),
        _ => None,
    } {
        s.enable_auto_text_box_drawing();
        return predef_in(event.function, text).ret();
    }
    s.game_corner_select_lucky_slot_machine();
    if s.ow.sprites[0].facing & SpriteFacing::Left as u8 == 0 {
        return Flow::Return;
    }
    // `AbleToPlaySlotsCheck` is farcalled, so its two texts are read from its bank rather than
    // `StartSlotMachine`'s.
    if s.ctx.world.bag.quantity_of(ItemId::CoinCase) == 0 {
        s.enable_auto_text_box_drawing();
        return predef_in(sym::AbleToPlaySlotsCheck, 0x33).ret();
    }
    if s.ctx.world.coins == [0, 0] {
        s.enable_auto_text_box_drawing();
        return predef_in(sym::AbleToPlaySlotsCheck, 0x32).ret();
    }
    let chance = slots::seven_and_bar_mode_chance(s.ow.rt.events.lucky_slot_machine, s.ow.rt.events.hidden_event_index);
    save_screen_tiles_to_buffer2(s);
    print(sym::PlaySlotMachineText).then(Label::SlotsAsk(chance))
}

// ---- The Game Corner's prizes ----

fn prize_window(s: &Script) -> u8 {
    s.ow.rt.sprite_index.wrapping_sub(TEXT_GAMECORNERPRIZEROOM_PRIZE_VENDOR_1)
}

fn prize_menu(s: &mut Script) -> Flow {
    if s.ctx.world.bag.quantity_of(ItemId::CoinCase) == 0 {
        return print(sym::RequireCoinCaseText).ret();
    }
    s.ctx.world.no_text_delay = true;
    print(sym::ExchangeCoinsForPrizesText).then(Label::PrizeMenuShown)
}

/// `PrintPrizePrice`: the purse in its box.
fn print_prize_price(s: &mut Script) {
    let coins = s.ctx.world.coins;
    let ui = &mut s.ctx.screen.ui;
    ui.text_box_border(11, 0, 7, 1);
    s.update_sprites();
    let ui = &mut s.ctx.screen.ui;
    ui.place(12, 0, &poke_core::charmap::encode("COIN").unwrap());
    ui.place(13, 1, &[UiSurface::BLANK; 6]);
    print_bcd(ui, SCREEN_TILES_X + 13, &coins, BcdFormat { skip_leading_zeroes: true, left_align: false, money_sign: false });
}

fn prize_menu_shown(s: &mut Script) -> Flow {
    s.ctx.menu.last_item = 0;
    print_prize_price(s);
    let window = PrizeWindow::of(prize_window(s));
    let ui = &mut s.ctx.screen.ui;
    ui.text_box_border(0, 2, 16, 8);
    for (i, &prize) in window.prizes.iter().enumerate() {
        let name = if prize_window(s) == 2 {
            ItemId::from_repr(prize).map(item::name).unwrap_or_default()
        } else {
            PokemonSpecies::from_repr(prize).map(PokemonSpecies::name).unwrap_or_default()
        };
        s.ctx.screen.ui.place(2, 4 + 2 * i, &name);
    }
    let ui = &mut s.ctx.screen.ui;
    place_rom_string(ui, 2, 10, sym::NoThanksText);
    for (i, price) in window.prices.iter().enumerate() {
        print_bcd(ui, (5 + 2 * i) * SCREEN_TILES_X + 13, price, BcdFormat { skip_leading_zeroes: true, left_align: false, money_sign: false });
    }
    s.update_sprites();
    print(sym::WhichPrizeText).then(Label::PrizeMenuMenu)
}

pub(super) fn prize_menu_menu(s: &mut Script) -> Flow {
    let menu = CursorMenu::new(0, 3, (1, 4));
    let _ = s;
    Then::block(Block::Mode(Box::new(Mode::CursorMenu(menu)))).then(Label::PrizeMenuChosen)
}

fn prize_menu_done(s: &mut Script) -> Flow {
    s.ctx.world.no_text_delay = false;
    Flow::Return
}

fn prize_menu_chosen(s: &mut Script) -> Flow {
    let row = match s.ow.rt.outcome {
        Some(Outcome::Chosen(row)) if row != 3 => row,
        _ => return prize_menu_done(s),
    };
    s.ow.rt.events.prize = row;
    let prize = PrizeWindow::of(prize_window(s)).prizes[row as usize];
    let name = if prize_window(s) == 2 {
        ItemId::from_repr(prize).map(item::name).unwrap_or_default()
    } else {
        PokemonSpecies::from_repr(prize).map(PokemonSpecies::name).unwrap_or_default()
    };
    s.ctx.world.text.strings.insert(TextBuffer::NameBuffer, name);
    print(sym::SoYouWantPrizeText).then(Label::PrizeAsk)
}

fn prize_price(s: &Script) -> [u8; 2] {
    PrizeWindow::of(prize_window(s)).prices[s.ow.rt.events.prize as usize]
}

fn prize_answered(s: &mut Script) -> Flow {
    if !after_yes_no(s) {
        s.ctx.world.no_text_delay = false;
        return print(sym::OhFineThenText).ret();
    }
    if s.ctx.world.coins < prize_price(s) {
        s.ctx.world.no_text_delay = false;
        return print(sym::SorryNeedMoreCoinsText).ret();
    }
    let prize = PrizeWindow::of(prize_window(s)).prizes[s.ow.rt.events.prize as usize];
    if prize_window(s) == 2 {
        let tm = ItemId::from_repr(prize).expect("a TM");
        if !s.give_item(tm, 1) {
            s.ctx.world.no_text_delay = false;
            return print(sym::PrizeRoomBagIsFullText).ret();
        }
        return subtract_coins(s);
    }
    let species = PokemonSpecies::from_repr(prize).expect("a prize mon");
    let level = tables::prize_mon_level(species);
    s.give_pokemon(species, level).then(Label::PrizeGiven)
}

fn subtract_coins(s: &mut Script) -> Flow {
    let price = prize_price(s);
    sub_bcd(&mut s.ctx.world.coins, &price);
    print_prize_price(s);
    prize_menu_done(s)
}

// ---- The elevators ----

fn elevator_chosen(s: &mut Script) -> Flow {
    s.ctx.menu.list_scroll = s.ow.rt.events.saved_list_scroll;
    let Some(Outcome::Chosen(floor)) = s.ow.rt.outcome else { return Flow::Return };
    s.ow.rt.events.used_elevator = true;
    let (warp, map) = s.ow.rt.events.elevator.1[floor as usize];
    for door in s.ow.warps.iter_mut().take(2) {
        door.destination_warp = warp;
        door.destination_map = map as u8;
    }
    Flow::Return
}

/// `ShakeElevator`: the rows above and below the screen redrawn, which is loading, then a hundred
/// two-frame shakes with a bump each, and the PA's chime waited out.
fn shake_elevator(s: &mut Script) -> Flow {
    s.play_sound(SoundId::STOP_ALL_MUSIC);
    s.ow.rt.events.shake_scy = s.ctx.screen.effects.scy;
    Flow::Jump(Label::ShakeElevatorStep(0).into())
}

fn shake_elevator_step(s: &mut Script, n: u8) -> Flow {
    const SHAKES: u8 = 100;
    let scy = s.ow.rt.events.shake_scy;
    if n < SHAKES {
        let e = if n % 2 == 0 { 0xFF } else { 0x01 };
        s.ctx.screen.effects.scy = scy.wrapping_add(e);
        let bank = crate::audio::data::AudioBank::One;
        s.play_music(crate::audio::data::Sound { bank, id: sounds::SFX_COLLISION });
        return s.delay_frames(2).then(Label::ShakeElevatorStep(n + 1));
    }
    s.ctx.screen.effects.scy = scy;
    s.play_sound(SoundId::STOP_ALL_MUSIC);
    let poke_core::symbols::DmgBank::ROM { bank } = sym::SFX_Safari_Zone_PA.bank else { unreachable!() };
    let bank = crate::audio::data::AudioBank::from_rom_bank(bank).expect("an audio bank");
    s.play_music(crate::audio::data::Sound { bank, id: sounds::SFX_SAFARI_ZONE_PA });
    Flow::Jump(Label::ShakeElevatorChime.into())
}

// ---- The diploma ----

/// `DisplayDiploma`: a certificate over the whole screen with Red behind it, until a press.
fn diploma(s: &mut Script) -> Flow {
    const CIRCLE: u8 = 0x70;
    save_screen_tiles_to_buffer2(s);
    s.ctx.world.no_text_delay = true;
    s.ow.rt.sprites_frozen = true;
    let tiles = &mut s.ctx.screen.tiles;
    tiles.load(V_CHARS2 + CIRCLE as usize, &rom_slice(sym::CircleTile)[..TILE_BYTES]);
    let ui = &mut s.ctx.screen.ui;
    ui.fill(0, 0, SCREEN_TILES_X, crate::gfx::ui::SCREEN_TILES_Y, UiSurface::BLANK);
    cable_club_text_box_border(ui, 0, 0, 18, 16);
    let table = rom_slice(sym::DiplomaTextPointersAndCoords);
    for row in table.chunks(4).take(5) {
        let text = DmgPointer { bank: sym::DiplomaTextPointersAndCoords.bank, address: u16::from_le_bytes([row[0], row[1]]) };
        let at = u16::from_le_bytes([row[2], row[3]]) as usize - 0xC3A0;
        place_rom_string(ui, at % SCREEN_TILES_X, at / SCREEN_TILES_X, text);
    }
    let name = s.ctx.world.player_name.clone();
    ui.place(10, 4, &name);
    // `DrawPlayerCharacter`, moved 33 pixels right and behind the background.
    let red = &rom_slice(sym::PlayerCharacterTitleGraphics)[..(sym::PlayerCharacterTitleGraphicsEnd.address - sym::PlayerCharacterTitleGraphics.address) as usize];
    tiles_load(s, V_CHARS0, red);
    let objects = &mut s.ctx.screen.sprites;
    clear_sprites(objects);
    for row in 0..7u8 {
        for column in 0..5u8 {
            let at = (row * 5 + column) as usize;
            objects[at] = Object { y: 0x60 + 8 * row, x: 0x5A + 8 * column + 33, tile: row * 5 + column, attributes: Object::BEHIND_BG };
        }
    }
    let border = rom_slice(sym::TrainerInfoTextBoxTileGraphics);
    let count = (sym::TrainerInfoTextBoxTileGraphicsEnd.address - sym::TrainerInfoTextBoxTileGraphics.address) as usize;
    tiles_load(s, V_CHARS2 + 0x76, &border[..count]);
    s.ctx.screen.effects = Default::default();
    s.ctx.screen.effects.obp0 = 0x90;
    Then::block(Block::TextScrollButton).then(Label::DiplomaDone)
}

fn tiles_load(s: &mut Script, first: usize, bytes: &[u8]) {
    s.ctx.screen.tiles.load(first, bytes);
}

/// `CableClub_TextBoxBorder`, in the trainer card's tiles.
fn cable_club_text_box_border(ui: &mut UiSurface, x: usize, y: usize, width: usize, height: usize) {
    ui.set(x, y, 0x78);
    ui.fill(x + 1, y, width, 1, 0x79);
    ui.set(x + width + 1, y, 0x7A);
    for row in y + 1..=y + height {
        ui.set(x, row, 0x7B);
        ui.fill(x + 1, row, width, 1, UiSurface::BLANK);
        ui.set(x + width + 1, row, 0x77);
    }
    ui.set(x, y + height + 1, 0x7C);
    ui.fill(x + 1, y + height + 1, width, 1, 0x76);
    ui.set(x + width + 1, y + height + 1, 0x7D);
}
