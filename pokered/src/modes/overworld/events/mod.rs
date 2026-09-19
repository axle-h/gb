//! `engine/events` on the script runtime: what `DisplayTextID` dispatches to (the nurse, the vending
//! machine, the prize vendor), what an A press finds before a sprite or a sign (hidden objects,
//! bookshelves, card key doors), poison on the step, and the routines a map's code calls (trades, the
//! day care, Oak's aides and the rest, in `api.rs`).
//!
//! Each routine is a run of labels, as a map's are: a label does what the cartridge does up to the
//! next thing that takes frames and names the label after it.

pub mod api;
mod day_care;
mod hidden;
mod pokecenter;
mod poison;
mod trades;
mod vending;

use poke_core::symbols::DmgPointer;
use serde::{Deserialize, Serialize};
use crate::gfx::mon_icons::clear_sprites;
use crate::mode::{Mode, Outcome};
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::systems::events::hidden_events::HiddenEvent;
use crate::systems::overworld::sprites;
use super::script::{text_at, Block, Flow, Script, Then};

pub use hidden::predef_text;
pub(super) use poison::poison_step_takes_frames;

/// What an A press found in front of the player before any sprite or sign.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Hidden {
    Event(HiddenEvent),
    /// A bookshelf's text predef.
    Bookshelf(u8),
    CardKeyDoor,
}

/// What the events keep between their frames.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRuntime {
    pub hidden: Option<Hidden>,
    /// `wHiddenEventIndex`, which counts up from the map's entry.
    pub hidden_event_index: u8,
    /// `wHiddenItemOrCoinsIndex`.
    pub hidden_item_or_coins_index: u8,
    /// `rOBP1` as the healing machine found it.
    pub saved_obp1: u8,
    /// `wAudioSavedROMBank`, as the healing machine leaves it.
    pub saved_audio_bank: Option<crate::audio::data::AudioBank>,
    /// `hVendingMachinePrice`.
    pub vending_price: [u8; 3],
    /// `wWhichTrade` and `wInGameTradeTextPointerTableIndex`.
    pub trade: Option<u8>,
    pub trade_text: Option<crate::systems::events::tables::TradeText>,
    /// The mon the day care man is working with, and the level it went in at.
    pub day_care_start_level: u8,
    /// The mon paid for: out of the day care, as `wDayCareInUse` is cleared, and not yet in the party.
    #[serde(default)]
    pub day_care_collected: Option<crate::party::Named<crate::party::BoxMon>>,
    /// `hOaksAideResult`.
    pub oaks_aide_result: Option<crate::systems::events::tables::OaksAideResult>,
    /// `wCardKeyDoorY` and `wCardKeyDoorX`, in blocks: the door opened last.
    pub card_key_door: (u8, u8),
    /// `wOpponentAfterWrongAnswer`.
    pub opponent_after_wrong_answer: u8,
    /// `hGymGateIndex` and `hGymGateAnswer`.
    pub gym_gate: (u8, u8),
    /// A menu loop's `wMenuItemOffset` and `wCurrentMenuItem`, kept round the loop.
    pub menu: (u8, u8),
    /// `hOaksAideRequirement` and `hOaksAideRewardItem`.
    pub oaks_aide: (u8, u8),
    /// The Hall of Fame's rating.
    pub dex_rating_text: Option<DmgPointer>,
    /// `wFilteredBagItems`.
    pub filtered_bag_items: Vec<poke_core::item::ItemId>,
    /// `wWhichPrize`.
    pub prize: u8,
    /// `wItemList` and `wElevatorWarpMaps`.
    pub elevator: (Vec<poke_core::item::ItemId>, Vec<(u8, poke_core::map::Map)>),
    /// `BIT_CUR_MAP_USED_ELEVATOR`.
    pub used_elevator: bool,
    /// `wListScrollOffset`, which the floor menu pushes and pops.
    pub saved_list_scroll: u8,
    /// `hSCY` before the elevator shakes.
    pub shake_scy: u8,
    /// `wLuckySlotHiddenEventIndex`, counting from one.
    #[serde(default)]
    pub lucky_slot_machine: u8,
}

/// Every place an event carries on from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    // `CheckForHiddenEventOrBookshelfOrCardKeyDoor`.
    HiddenEventOrBookshelf,
    HiddenItemFound,
    HiddenItemBagFull,
    HiddenItemSound,
    CardKeyOpened { x: u8, y: u8 },
    IndigoPlateauStatues,
    TownMap,
    TownMapClosed,

    // `DisplayPokemonCenterDialogue_` and `AnimateHealingMachine`.
    PokemonCenter,
    NurseWelcomed,
    NurseAsk,
    NurseAnswered,
    NurseTurns,
    HealingMachineMusicStopped,
    HealingMachineBall(u8),
    HealingMachineFlash(u8),
    HealingMachineJingleOver,
    HealingMachineDone,
    NurseBows,
    NurseFarewell,
    NurseDone,

    // `VendingMachineMenu`.
    VendingMachine,
    VendingMachineMenu,
    VendingMachineChosen,
    VendingMachineDeliver(u8),
    VendingMachineRattle(u8),
    VendingMachinePaid,

    // `ApplyOutOfBattlePoisonDamage`.
    ApplyOutOfBattlePoisonDamage,
    PoisonMon(u8),
    PoisonFlashed,
    PoisonBlackedOut,

    // `SafariZoneGameOver`.
    SafariZoneGameOver,
    SafariGameOverDone,
    SafariGameOverText,
    SafariGameOverTextDone,

    // `DoInGameTradeDialogue`.
    InGameTrade(u8),
    InGameTradeAsk,
    InGameTradeAnswered,
    InGameTradeChoseMon,
    InGameTradeConnected(u8),
    InGameTradeText,

    // `DaycareGentlemanText`.
    DayCare,
    DayCareAsk,
    DayCareAnswered,
    DayCareWhichMon,
    DayCareChoseMon,
    DayCareTakes(u8),
    DayCareTaken,
    DayCareInUse,
    DayCareGrown,
    DayCareOwe,
    DayCarePay,
    DayCarePurchaseSound,
    DayCareHeresYourMon,
    DayCareReturned,
    DayCareDone(DmgPointer),

    // `api.rs`.
    MonPopup(api::Picture),
    MonPopupStage(u8),
    MonPopupDone,
    PredefAfterPopup { id: u8, bank: u8 },
    Binoculars,
    CellSeparator(u8),
    CellSeparatorSound(u8),
    BillsHouseInitiated,
    BillsHouseInitiatedSwitch,
    BillsHouseInitiatedDone,
    BillsListLoop,
    BillsListMenu,
    BillsListChosen,
    BillsListDexShown,
    DisplayPokedex(poke_core::species::PokemonSpecies),
    DisplayPokedexShown(poke_core::species::PokemonSpecies),
    StarterDex(u8),
    StarterDexShown,
    QuizQuestion,
    QuizAsk,
    QuizAnswered,
    QuizCorrectText,
    QuizCorrectSound,
    QuizUnlock,
    QuizWrongSound,
    QuizWrongText,
    QuizWrongDone,
    NotebookTurnPage(u8),
    NotebookAsk(u8),
    NotebookAnswered(u8),
    NotebookLastPage,
    BlackboardLoop,
    BlackboardMenu,
    BlackboardChosen,
    LinkCableHelpLoop,
    LinkCableHelpMenu,
    LinkCableHelpChosen,
    TrashSound(u8),
    TrashSoundPlay(u8),
    OaksAide,
    OaksAideAsk,
    OaksAideAnswered,
    OaksAideGive,
    DexRating,
    DexRatingText,
    DexRatingSfx,
    DexRatingSfxPlay,
    DexRatingDone,
    CinnabarLab,
    CinnabarLabChosen,
    CinnabarLabAsk,
    CinnabarLabAnswered,
    CinnabarLabTaken,
    CinnabarLabDone,
    PrizeMenu,
    PrizeMenuShown,
    PrizeMenuMenu,
    PrizeMenuChosen,
    PrizeAsk,
    PrizeAnswered,
    PrizeGiven,
    PrizeGivenWaited,
    ElevatorFloorMenu,
    ElevatorFloorList,
    ElevatorChosen,
    ShakeElevator,
    ShakeElevatorStep(u8),
    ShakeElevatorChime,
    ShakeElevatorDone,
    Diploma,
    DiplomaDone,
    SlotsAsk(u8),
    SlotsAnswered(u8),
    SlotsBubbled(u8),
    SlotsLeft,
    SlotsDone,
}

pub fn resume(s: &mut Script, label: Label) -> Flow {
    use Label::*;
    match label {
        HiddenEventOrBookshelf => hidden::check(s),
        HiddenItemFound => hidden::hidden_item_found(s),
        HiddenItemBagFull => hidden::hidden_item_bag_full(s),
        HiddenItemSound => hidden::hidden_item_sound(s),
        CardKeyOpened { x, y } => hidden::card_key_opened(s, x, y),
        IndigoPlateauStatues => hidden::indigo_plateau_statues(s),
        TownMap => hidden::town_map(s),
        TownMapClosed => Then::call(super::script::Routine::CloseTextDisplay).ret(),

        PokemonCenter => pokecenter::pokemon_center(s),
        NurseWelcomed => pokecenter::welcomed(s),
        NurseAsk => pokecenter::ask(s),
        NurseAnswered => pokecenter::answered(s),
        NurseTurns => pokecenter::turns(s),
        HealingMachineMusicStopped => pokecenter::music_stopped(s),
        HealingMachineBall(n) => pokecenter::ball(s, n),
        HealingMachineFlash(n) => pokecenter::flash(s, n),
        HealingMachineJingleOver => pokecenter::jingle_over(s),
        HealingMachineDone => pokecenter::machine_done(s),
        NurseBows => pokecenter::bows(s),
        NurseFarewell => pokecenter::farewell(s),
        NurseDone => {
            s.update_sprites();
            Flow::Return
        }

        VendingMachine => vending::vending_machine(s),
        VendingMachineMenu => vending::menu(s),
        VendingMachineChosen => vending::chosen(s),
        VendingMachineDeliver(n) => vending::deliver(s, n),
        VendingMachineRattle(n) => vending::rattle(s, n),
        VendingMachinePaid => vending::paid(s),

        ApplyOutOfBattlePoisonDamage => poison::poison_mon(s, 0),
        PoisonMon(slot) => poison::poison_mon(s, slot),
        PoisonFlashed => poison::flashed(s),
        PoisonBlackedOut => poison::blacked_out(s),

        SafariZoneGameOver => poison::safari_zone_game_over(s),
        SafariGameOverDone => poison::safari_game_over_done(s),
        SafariGameOverText => poison::safari_game_over_text(s),
        SafariGameOverTextDone => poison::safari_game_over_text_done(s),

        InGameTrade(which) => trades::in_game_trade(s, which),
        InGameTradeAsk => trades::ask(s),
        InGameTradeAnswered => trades::answered(s),
        InGameTradeChoseMon => trades::chose_mon(s),
        InGameTradeConnected(slot) => trades::connected(s, slot),
        InGameTradeText => trades::print_text(s),

        DayCare => day_care::day_care(s),
        DayCareAsk => day_care::ask(s),
        DayCareAnswered => day_care::answered(s),
        DayCareWhichMon => day_care::which_mon(s),
        DayCareChoseMon => day_care::chose_mon(s),
        DayCareTakes(slot) => day_care::takes(s, slot),
        DayCareTaken => day_care::taken(s),
        DayCareInUse => day_care::in_use(s),
        DayCareGrown => day_care::grown(s),
        DayCareOwe => day_care::owe(s),
        DayCarePay => day_care::pay(s),
        DayCarePurchaseSound => day_care::purchase_sound(s),
        DayCareHeresYourMon => day_care::heres_your_mon(s),
        DayCareReturned => day_care::returned(s),
        DayCareDone(text) => print(text).ret(),

        PrizeMenuMenu => api::prize_menu_menu(s),
        label => api::resume(s, label),
    }
}

/// `PrintText`.
fn print(at: DmgPointer) -> Then {
    Then::block(Block::PrintText(text_at(at)))
}

/// `YesNoChoice`: `SaveScreenTilesToBuffer1`, the menu, and [`after_yes_no`] to put the screen back.
fn yes_no(s: &mut Script) -> Then {
    s.ow.rt.saved_screen = Some(s.ctx.screen.ui.clone());
    Then::block(Block::Mode(Box::new(Mode::TwoOptionMenu(TwoOptionMenu::new(TwoOptionMenuId::YesNo, (14, 7), false)))))
}

/// `LoadScreenTilesFromBuffer1` after a yes/no, and whether it was yes.
fn after_yes_no(s: &mut Script) -> bool {
    if let Some(saved) = s.ow.rt.saved_screen.take() {
        s.ctx.screen.ui = saved;
    }
    s.ow.rt.outcome == Some(Outcome::Chosen(0))
}

/// `SaveScreenTilesToBuffer2`.
fn save_screen_tiles_to_buffer2(s: &mut Script) {
    s.ow.rt.saved_screen2 = Some(s.ctx.screen.ui.clone());
}

/// `RestoreScreenTilesAndReloadTilePatterns`: the party menu's icons down, the map's sprites loaded
/// again, and the screen from `wTileMapBackup2`. Its `Delay3` is loading.
fn restore_screen_tiles_and_reload_tile_patterns(s: &mut Script) {
    clear_sprites(&mut s.ctx.screen.sprites);
    s.ow.rt.sprites_frozen = false;
    let location = &s.ctx.world.location;
    sprites::init_map_sprites(&mut s.ow.sprites, &mut s.ow.sprite_set, location.map, location.x, location.y,
        s.ow.num_sprites, false, &mut s.ctx.screen.tiles);
    if let Some(saved) = s.ow.rt.saved_screen2.clone() {
        s.ctx.screen.ui = saved;
    }
    s.ctx.screen.tiles.load_text_box_tiles();
    s.ctx.screen.tiles.load_tileset(s.ow.view.tileset);
}

/// `PlaceString` for a string in ROM with nothing to delay it: `<NEXT>` two rows down to the column it
/// started in, the ligatures spelled out.
fn place_rom_string(ui: &mut crate::gfx::ui::UiSurface, x: usize, y: usize, at: DmgPointer) {
    use crate::modes::place_string::{ch, ligature};
    let (mut column, mut row) = (x, y);
    for &byte in poke_core::rom_gfx::rom_slice(at).iter().take_while(|&&b| b != ch::TERMINATOR) {
        if byte == ch::NEXT {
            (column, row) = (x, row + 2);
            continue;
        }
        let tiles = ligature(byte).map_or_else(|| vec![byte], <[u8]>::to_vec);
        ui.place(column, row, &tiles);
        column += tiles.len();
    }
}

#[cfg(test)]
mod tests;
