//! `OaksLab_Script`: the rival, the three Poké Balls on the table, the first battle, the Pokédex and
//! the parcel.

use poke_core::item::ItemId;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::*;
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_symbols::{OAKSLAB_BULBASAUR_POKE_BALL, OAKSLAB_CHARMANDER_POKE_BALL, OAKSLAB_OAK1,
    OAKSLAB_OAK2, OAKSLAB_RIVAL, OAKSLAB_SQUIRTLE_POKE_BALL};
use poke_core::symbols::pokered_toggles::*;
use poke_core::trainer_headers::OPP_ID_OFFSET;
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::input::Joypad;
use crate::modes::overworld::movement::{NPC_MOVEMENT_DOWN, NPC_MOVEMENT_LEFT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_UP};
use crate::modes::overworld::script::SpritePosition;
use crate::systems::pokedex::index_to_pokedex;
use crate::systems::overworld::sprites::{SPRITE_FACING_DOWN, SPRITE_FACING_LEFT, SPRITE_FACING_RIGHT, SPRITE_FACING_UP};
use super::{text_at, Flow, Script};

/// `PLAYER_DIR_UP`.
const PLAYER_DIR_UP: u8 = 8;
/// The end of a `MoveSprite` list.
const END: u8 = 0xFF;
/// `OPP_RIVAL1`.
const OPP_RIVAL1: u8 = OPP_ID_OFFSET + 25;
/// `STARTER1`, `STARTER2` and `STARTER3`.
const STARTER1: PokemonSpecies = PokemonSpecies::Charmander;
const STARTER2: PokemonSpecies = PokemonSpecies::Squirtle;
const STARTER3: PokemonSpecies = PokemonSpecies::Bulbasaur;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wOaksLabCurScript`.
    pub cur_script: u8,
    /// `wPlayerStarter`.
    pub player_starter: u8,
    /// `wRivalStarterTemp` and `wRivalStarterBallSpriteIndex`: what the rival will take once the
    /// player has chosen, and which ball he walks up to.
    pub rival_starter_temp: u8,
    pub rival_starter_ball: u8,
    /// `wSavedNPCMovementDirections2Index`: how many squares the rival walked up to Oak, which is
    /// how many he walks back down again.
    pub saved_steps: u8,
    /// The rival's place, saved across the battle by `GetSpritePosition1`.
    pub rival_at: Option<SpritePosition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    ChooseMonSpeechFedUp,
    ChooseMonSpeechChooseMon,
    ChooseMonSpeechWhatAboutMe,
    ChooseMonSpeechWhatAboutMeText,
    ChooseMonSpeechBePatient,
    ChooseMonSpeechBePatientText,
    ChooseMonSpeechDone,
    DontGoAwayRivalFaces,
    DontGoAwayText,
    DontGoAwayWalkBack,
    ForcedToWalkBackDelay,
    RivalChoosesText,
    RivalChoosesHideBall,
    RivalChoosesFaceUp,
    RivalChoosesReceived,
    RivalChoosesDone,
    ChallengesPlayerText,
    RivalStartsExitText,
    PokedexRivalSpeaks,
    PokedexRequest,
    PokedexInvention,
    PokedexGot,
    PokedexHideBoth,
    PokedexThatWasMyDream,
    PokedexRivalFacesRight,
    PokedexLeaveItToMe,
    PokedexRivalLeaves,
    RivalArrivesMoves,
    /// `OaksLabOak1Text`: the rating after `.HowIsYourPokedexComingText`, and the parcel handed over.
    Oak1DexRating,
    Oak1DexRatingDone,
    Oak1DeliverParcel,
    /// `OaksLabShowPokeBallPokemonScript`: the dex page, then the offer and its answer.
    BallDexShown,
    BallChoiceAnswered,
    BallReceivedMon,
    BallAddMon,
    BallAdded,
    BallNamed,
}

/// The three balls, as (text id, sprite slot, the species in it, the toggle that hides it).
const BALLS: [(u8, u8, PokemonSpecies, u16); 3] = [
    (TEXT_OAKSLAB_CHARMANDER_POKE_BALL, OAKSLAB_CHARMANDER_POKE_BALL, STARTER1, TOGGLE_STARTER_BALL_1),
    (TEXT_OAKSLAB_SQUIRTLE_POKE_BALL, OAKSLAB_SQUIRTLE_POKE_BALL, STARTER2, TOGGLE_STARTER_BALL_2),
    (TEXT_OAKSLAB_BULBASAUR_POKE_BALL, OAKSLAB_BULBASAUR_POKE_BALL, STARTER3, TOGGLE_STARTER_BALL_3),
];

/// The ball whose sprite is `slot`.
fn ball(slot: u8) -> (u8, u8, PokemonSpecies, u16) {
    BALLS.into_iter().find(|&(_, sprite, ..)| sprite == slot).expect("a starter's ball")
}

pub fn script(rt: &mut Script) -> Flow {
    if rt.check_event(EVENT_PALLET_AFTER_GETTING_POKEBALLS_2) {
        rt.set_text_pointers(sym::OaksLab_TextPointers2);
    }
    rt.disable_auto_text_box_drawing();
    rt.set_do_not_wait_for_button_press(false);
    match rt.maps().oaks_lab.cur_script {
        SCRIPT_OAKSLAB_DEFAULT => default_script(rt),
        SCRIPT_OAKSLAB_OAK_ENTERS_LAB => {
            rt.move_sprite(OAKSLAB_OAK2, &[NPC_MOVEMENT_UP, NPC_MOVEMENT_UP, NPC_MOVEMENT_UP, END]);
            rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_TOGGLE_OAKS;
            Flow::Return
        }
        SCRIPT_OAKSLAB_TOGGLE_OAKS => {
            if rt.npc_moving() {
                return Flow::Return;
            }
            rt.hide_object(TOGGLE_OAKS_LAB_OAK_2);
            rt.show_object(TOGGLE_OAKS_LAB_OAK_1);
            rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_PLAYER_ENTERS_LAB;
            Flow::Return
        }
        SCRIPT_OAKSLAB_PLAYER_ENTERS_LAB => player_enters_lab(rt),
        SCRIPT_OAKSLAB_FOLLOWED_OAK => followed_oak(rt),
        SCRIPT_OAKSLAB_OAK_CHOOSE_MON_SPEECH => {
            rt.joy_ignore(Joypad::SELECT | Joypad::START | PAD_CTRL_PAD);
            rt.display_text_id(TEXT_OAKSLAB_RIVAL_FED_UP_WITH_WAITING).then(Label::ChooseMonSpeechFedUp)
        }
        SCRIPT_OAKSLAB_PLAYER_DONT_GO_AWAY_SCRIPT => dont_go_away(rt),
        SCRIPT_OAKSLAB_PLAYER_FORCED_TO_WALK_BACK_SCRIPT => {
            if rt.simulated_joypad_states_index() != 0 {
                return Flow::Return;
            }
            rt.delay3().then(Label::ForcedToWalkBackDelay)
        }
        SCRIPT_OAKSLAB_CHOSE_STARTER_SCRIPT => chose_starter(rt),
        SCRIPT_OAKSLAB_RIVAL_CHOOSES_STARTER => {
            if rt.npc_moving() {
                return Flow::Return;
            }
            rt.joy_ignore(Joypad::SELECT | Joypad::START | PAD_CTRL_PAD);
            rt.set_sprite_facing_direction_and_delay(OAKSLAB_RIVAL, SPRITE_FACING_UP).then(Label::RivalChoosesText)
        }
        SCRIPT_OAKSLAB_RIVAL_CHALLENGES_PLAYER => challenges_player(rt),
        SCRIPT_OAKSLAB_RIVAL_START_BATTLE => rival_start_battle(rt),
        SCRIPT_OAKSLAB_RIVAL_END_BATTLE => rival_end_battle(rt),
        SCRIPT_OAKSLAB_RIVAL_STARTS_EXIT => {
            rt.delay_frames(20).then(Label::RivalStartsExitText)
        }
        SCRIPT_OAKSLAB_PLAYER_WATCH_RIVAL_EXIT => watch_rival_exit(rt),
        SCRIPT_OAKSLAB_RIVAL_ARRIVES_AT_OAKS_REQUEST => rival_arrives(rt),
        SCRIPT_OAKSLAB_OAK_GIVES_POKEDEX => {
            if rt.npc_moving() {
                return Flow::Return;
            }
            rt.enable_auto_text_box_drawing();
            rt.joy_ignore(Joypad::SELECT | Joypad::START | PAD_CTRL_PAD);
            rival_face_up_oak_face_down(rt);
            rt.play_default_music().then(Label::PokedexRivalSpeaks)
        }
        SCRIPT_OAKSLAB_RIVAL_LEAVES_WITH_POKEDEX => rival_leaves_with_pokedex(rt),
        _ => Flow::Return,
    }
}

/// `OaksLabDefaultScript`: Oak walks in behind the player, who has just followed him from town.
fn default_script(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_OAK_APPEARED_IN_PALLET) || rt.npc_movement_script_running() {
        return Flow::Return;
    }
    rt.show_object(TOGGLE_OAKS_LAB_OAK_2);
    rt.set_no_battles(false);
    rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_OAK_ENTERS_LAB;
    Flow::Return
}

/// `OaksLabPlayerEntersLabScript`: the player is walked eight squares up to the table.
fn player_enters_lab(rt: &mut Script) -> Flow {
    rt.simulate_joypad_rle(sym::PlayerEntryMovementRLE);
    rt.set_sprite_facing(OAKSLAB_RIVAL, SPRITE_FACING_DOWN);
    rt.set_sprite_facing(OAKSLAB_OAK1, SPRITE_FACING_DOWN);
    rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_FOLLOWED_OAK;
    Flow::Return
}

/// `OaksLabFollowedOakScript`.
fn followed_oak(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.set_event(EVENT_FOLLOWED_OAK_INTO_LAB);
    rt.set_event(EVENT_FOLLOWED_OAK_INTO_LAB_2);
    rt.set_sprite_facing(OAKSLAB_RIVAL, SPRITE_FACING_UP);
    rt.update_sprites();
    rt.set_no_map_music(false);
    rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_OAK_CHOOSE_MON_SPEECH;
    rt.play_default_music().ret()
}

/// `OaksLabPlayerDontGoAwayScript`: a player walking back out of the lab is walked up again.
fn dont_go_away(rt: &mut Script) -> Flow {
    if rt.y() != 6 {
        return Flow::Return;
    }
    rt.set_sprite_facing_direction_and_delay(OAKSLAB_OAK1, SPRITE_FACING_DOWN).then(Label::DontGoAwayRivalFaces)
}

/// `OaksLabChoseStarterScript`: the rival crosses to the ball the player left him.
fn chose_starter(rt: &mut Script) -> Flow {
    let below_table = rt.y() == 4;
    let path: Vec<u8> = match PokemonSpecies::from_repr(rt.maps().oaks_lab.player_starter) {
        // `.MiddleBallMovement1` and `2`.
        Some(STARTER1) if below_table => vec![NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_RIGHT,
            NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_UP, END],
        Some(STARTER1) => vec![NPC_MOVEMENT_DOWN, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, END],
        // `.RightBallMovement1` and `2`.
        Some(STARTER2) if below_table => vec![NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_RIGHT,
            NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_UP, END],
        Some(STARTER2) => vec![NPC_MOVEMENT_DOWN, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT,
            NPC_MOVEMENT_RIGHT, END],
        // `.LeftBallMovement1`, and `2` from inside it: a player standing right of the table puts the
        // rival off screen, so he is put back on it before he walks the one square left of the list.
        _ if rt.x() == 9 => {
            rt.set_sprite_position(OAKSLAB_RIVAL, SpritePosition { screen_y: 0x4C, screen_x: 0, map_y: 8, map_x: 9 });
            vec![NPC_MOVEMENT_RIGHT, END]
        }
        _ => vec![NPC_MOVEMENT_DOWN, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, END],
    };
    rt.move_sprite(OAKSLAB_RIVAL, &path);
    rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_RIVAL_CHOOSES_STARTER;
    Flow::Return
}

/// `OaksLabRivalChallengesPlayerScript`.
fn challenges_player(rt: &mut Script) -> Flow {
    if rt.y() != 6 {
        return Flow::Return;
    }
    rt.set_sprite_facing(OAKSLAB_RIVAL, SPRITE_FACING_DOWN);
    rt.set_player_moving_direction(PLAYER_DIR_UP);
    rt.play_music(sounds::MUSIC_MEET_RIVAL);
    rt.display_text_id(TEXT_OAKSLAB_RIVAL_ILL_TAKE_YOU_ON).then(Label::ChallengesPlayerText)
}

/// `OaksLabRivalStartBattleScript`: the rival's team follows from the starter he took.
fn rival_start_battle(rt: &mut Script) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    let trainer_no = match PokemonSpecies::from_repr(rt.globals().rival_starter) {
        Some(STARTER2) => 1,
        Some(STARTER3) => 2,
        _ => 3,
    };
    let at = rt.sprite_position(OAKSLAB_RIVAL);
    rt.maps().oaks_lab.rival_at = Some(at);
    rt.start_trainer_battle(OPP_RIVAL1, trainer_no, sym::OaksLabRivalIPickedTheWrongPokemonText);
    rt.joy_ignore(Joypad::empty());
    rt.set_player_moving_direction(PLAYER_DIR_UP);
    rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_RIVAL_END_BATTLE;
    Flow::Return
}

/// `OaksLabRivalEndBattleScript`: the rival is put back where the battle took him from.
fn rival_end_battle(rt: &mut Script) -> Flow {
    rt.joy_ignore(PAD_CTRL_PAD);
    rt.set_player_moving_direction(PLAYER_DIR_UP);
    rt.update_sprites();
    if let Some(at) = rt.maps().oaks_lab.rival_at.take() {
        rt.set_sprite_position(OAKSLAB_RIVAL, at);
    }
    rt.set_sprite_facing(OAKSLAB_RIVAL, SPRITE_FACING_DOWN);
    rt.heal_party();
    rt.set_event(EVENT_BATTLED_RIVAL_IN_OAKS_LAB);
    rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_RIVAL_STARTS_EXIT;
    Flow::Return
}

/// `OaksLabPlayerWatchRivalExitScript`: the player keeps facing the rival as he goes.
fn watch_rival_exit(rt: &mut Script) -> Flow {
    if !rt.npc_moving() {
        rt.hide_object(TOGGLE_OAKS_LAB_RIVAL);
        rt.joy_ignore(Joypad::empty());
        rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_NOOP;
        return rt.play_default_music().ret();
    }
    match rt.npc_steps_left() {
        5 if rt.x() == 4 => rt.set_player_facing(SPRITE_FACING_RIGHT),
        5 => rt.set_player_facing(SPRITE_FACING_LEFT),
        4 => rt.set_player_facing(SPRITE_FACING_DOWN),
        _ => {}
    }
    Flow::Return
}

/// `OaksLabRivalArrivesAtOaksRequestScript`: the rival comes back in once the parcel is handed over.
fn rival_arrives(rt: &mut Script) -> Flow {
    rt.clear_joy_held();
    rt.enable_auto_text_box_drawing();
    rt.play_sound(SoundId::STOP_ALL_MUSIC);
    rt.music_rival_alternate_start();
    rt.display_text_id(TEXT_OAKSLAB_RIVAL_GRAMPS).then(Label::RivalArrivesMoves)
}

/// `OaksLabCalcRivalMovementScript`: where the rival appears and how far he walks up, both of which
/// follow from where the player is standing round Oak.
fn calc_rival_movement(rt: &mut Script) -> u8 {
    let (steps, screen_x, map_y) = match (rt.y(), rt.x()) {
        (3, _) => (4, 0x30, 11),
        (1, _) => (2, 0x30, 9),
        (_, 4) => (3, 0x40, 10),
        _ => (3, 0x20, 10),
    };
    rt.set_sprite_position(OAKSLAB_RIVAL, SpritePosition { screen_y: 0x7C, screen_x, map_y, map_x: 8 });
    steps
}

/// `OaksLabRivalFaceUpOakFaceDownScript`.
fn rival_face_up_oak_face_down(rt: &mut Script) {
    rt.set_sprite_facing(OAKSLAB_RIVAL, SPRITE_FACING_UP);
    rt.set_sprite_facing(OAKSLAB_OAK2, SPRITE_FACING_DOWN);
}

/// `OaksLabRivalLeavesWithPokedexScript`: the rival is gone, and Route 22 is waiting for him.
fn rival_leaves_with_pokedex(rt: &mut Script) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    rt.hide_object(TOGGLE_OAKS_LAB_RIVAL);
    rt.set_event(EVENT_1ST_ROUTE22_RIVAL_BATTLE);
    rt.reset_event(EVENT_2ND_ROUTE22_RIVAL_BATTLE);
    rt.set_event(EVENT_ROUTE22_RIVAL_WANTS_BATTLE);
    rt.show_object(TOGGLE_ROUTE_22_RIVAL_1);
    rt.maps().pallet_town.cur_script = SCRIPT_PALLETTOWN_DAISY;
    rt.joy_ignore(Joypad::empty());
    rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_NOOP;
    rt.play_default_music().ret()
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_OAKSLAB_RIVAL => rival_text(rt),
        TEXT_OAKSLAB_CHARMANDER_POKE_BALL | TEXT_OAKSLAB_SQUIRTLE_POKE_BALL | TEXT_OAKSLAB_BULBASAUR_POKE_BALL => {
            poke_ball_text(rt, text_id)
        }
        TEXT_OAKSLAB_OAK1 => oak1_text(rt),
        TEXT_OAKSLAB_POKEDEX1 | TEXT_OAKSLAB_POKEDEX2 => {
            rt.print_text(text_at(local::OaksLabPokedexText::Text)).ret()
        }
        TEXT_OAKSLAB_GIRL => rt.print_text(text_at(local::OaksLabGirlText::Text)).ret(),
        TEXT_OAKSLAB_SCIENTIST1 | TEXT_OAKSLAB_SCIENTIST2 => {
            rt.print_text(text_at(local::OaksLabScientistText::Text)).ret()
        }
        TEXT_OAKSLAB_OAK_DONT_GO_AWAY_YET => rt.print_text(text_at(local::OaksLabOakDontGoAwayYetText::Text)).ret(),
        TEXT_OAKSLAB_RIVAL_ILL_TAKE_THIS_ONE => {
            rt.print_text(text_at(local::OaksLabRivalIllTakeThisOneText::Text)).ret()
        }
        TEXT_OAKSLAB_RIVAL_RECEIVED_MON => rt.print_text(text_at(local::OaksLabRivalReceivedMonText::Text)).ret(),
        TEXT_OAKSLAB_RIVAL_ILL_TAKE_YOU_ON => rt.print_text(text_at(local::OaksLabRivalIllTakeYouOnText::Text)).ret(),
        TEXT_OAKSLAB_RIVAL_SMELL_YOU_LATER => rt.print_text(text_at(local::OaksLabRivalSmellYouLaterText::Text)).ret(),
        TEXT_OAKSLAB_RIVAL_FED_UP_WITH_WAITING => {
            rt.print_text(text_at(local::OaksLabRivalFedUpWithWaitingText::Text)).ret()
        }
        TEXT_OAKSLAB_OAK_CHOOSE_MON => rt.print_text(text_at(local::OaksLabOakChooseMonText::Text)).ret(),
        TEXT_OAKSLAB_RIVAL_WHAT_ABOUT_ME => rt.print_text(text_at(local::OaksLabRivalWhatAboutMeText::Text)).ret(),
        TEXT_OAKSLAB_OAK_BE_PATIENT => rt.print_text(text_at(local::OaksLabOakBePatientText::Text)).ret(),
        _ => return None,
    })
}

/// `OaksLabRivalText`.
fn rival_text(rt: &mut Script) -> Flow {
    let words = if !rt.check_event(EVENT_FOLLOWED_OAK_INTO_LAB_2) {
        local::OaksLabRivalText::GrampsIsntAroundText
    } else if !rt.check_event(EVENT_GOT_STARTER) {
        local::OaksLabRivalText::GoAheadAndChooseText
    } else {
        local::OaksLabRivalText::MyPokemonLooksStrongerText
    };
    rt.print_text(text_at(words)).ret()
}

/// `OaksLabCharmanderPokeBallText` and its two siblings into `OaksLabSelectedPokeBallScript`: each
/// ball names the one to its right as what the rival will take, so choosing leaves him the next.
fn poke_ball_text(rt: &mut Script, text_id: u8) -> Flow {
    let index = BALLS.iter().position(|&(id, ..)| id == text_id).expect("a ball's text");
    let (_, slot, species, _) = BALLS[index];
    let (_, next_slot, next_species, _) = BALLS[(index + 1) % BALLS.len()];
    let state = &mut rt.maps().oaks_lab;
    state.rival_starter_temp = next_species as u8;
    state.rival_starter_ball = next_slot;
    rt.set_sprite_index(slot);
    if rt.check_event(EVENT_GOT_STARTER) {
        // `OaksLabLastMonScript`: the ball the rival left behind is only ever looked at.
        rt.set_sprite_facing(OAKSLAB_OAK1, SPRITE_FACING_DOWN);
        return rt.print_text(text_at(sym::OaksLabLastMonText)).ret();
    }
    if !rt.check_event(EVENT_OAK_ASKED_TO_CHOOSE_MON) {
        return rt.print_text(text_at(sym::OaksLabThoseArePokeBallsText)).ret();
    }
    // `OaksLabShowPokeBallPokemonScript`.
    rt.set_sprite_facing(OAKSLAB_OAK1, SPRITE_FACING_DOWN);
    rt.set_sprite_facing(OAKSLAB_RIVAL, SPRITE_FACING_RIGHT);
    rt.starter_dex(index_to_pokedex(species as u8)).then(Label::BallDexShown)
}

/// `OaksLabOak1Text`: everything Oak says between the choice and the Poké Balls.
fn oak1_text(rt: &mut Script) -> Flow {
    let rated = rt.check_event(EVENT_PALLET_AFTER_GETTING_POKEBALLS)
        || (rt.pokedex_owned() >= 2 && rt.check_event(EVENT_GOT_POKEDEX));
    if rated {
        rt.set_do_not_wait_for_button_press(true);
        return rt.print_text(text_at(local::OaksLabOak1Text::HowIsYourPokedexComingText)).then(Label::Oak1DexRating);
    }
    if rt.is_item_in_bag(ItemId::PokeBall) {
        return rt.print_text(text_at(local::OaksLabOak1Text::ComeSeeMeSometimesText)).ret();
    }
    if rt.check_event(EVENT_BEAT_ROUTE22_RIVAL_1ST_BATTLE) {
        if rt.check_and_set_event(EVENT_GOT_POKEBALLS_FROM_OAK) {
            return rt.print_text(text_at(local::OaksLabOak1Text::ComeSeeMeSometimesText)).ret();
        }
        rt.give_item(ItemId::PokeBall, 5);
        return rt.print_text(text_at(local::OaksLabOak1Text::GivePokeballsText)).ret();
    }
    if rt.check_event(EVENT_GOT_POKEDEX) {
        return rt.print_text(text_at(local::OaksLabOak1Text::PokemonAroundTheWorldText)).ret();
    }
    if rt.check_event(EVENT_BATTLED_RIVAL_IN_OAKS_LAB) {
        if !rt.is_item_in_bag(ItemId::OaksParcel) {
            return rt.print_text(text_at(local::OaksLabOak1Text::RaiseYourYoungPokemonText)).ret();
        }
        return rt.print_text(text_at(local::OaksLabOak1Text::DeliverParcelText)).then(Label::Oak1DeliverParcel);
    }
    let words = match rt.check_event(EVENT_GOT_STARTER) {
        true => local::OaksLabOak1Text::YourPokemonCanFightText,
        false => local::OaksLabOak1Text::WhichPokemonDoYouWantText,
    };
    rt.print_text(text_at(words)).ret()
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::ChooseMonSpeechFedUp => rt.delay3().then(Label::ChooseMonSpeechChooseMon),
        Label::ChooseMonSpeechChooseMon => {
            rt.display_text_id(TEXT_OAKSLAB_OAK_CHOOSE_MON).then(Label::ChooseMonSpeechWhatAboutMe)
        }
        Label::ChooseMonSpeechWhatAboutMe => {
            rt.delay3().then(Label::ChooseMonSpeechWhatAboutMeText)
        }
        Label::ChooseMonSpeechWhatAboutMeText => {
            rt.display_text_id(TEXT_OAKSLAB_RIVAL_WHAT_ABOUT_ME).then(Label::ChooseMonSpeechBePatient)
        }
        Label::ChooseMonSpeechBePatient => rt.delay3().then(Label::ChooseMonSpeechBePatientText),
        Label::ChooseMonSpeechBePatientText => {
            rt.display_text_id(TEXT_OAKSLAB_OAK_BE_PATIENT).then(Label::ChooseMonSpeechDone)
        }
        Label::ChooseMonSpeechDone => {
            rt.set_event(EVENT_OAK_ASKED_TO_CHOOSE_MON);
            rt.joy_ignore(Joypad::empty());
            rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_PLAYER_DONT_GO_AWAY_SCRIPT;
            Flow::Return
        }

        Label::DontGoAwayRivalFaces => {
            rt.set_sprite_facing_direction_and_delay(OAKSLAB_RIVAL, SPRITE_FACING_DOWN).then(Label::DontGoAwayText)
        }
        Label::DontGoAwayText => {
            rt.update_sprites();
            rt.display_text_id(TEXT_OAKSLAB_OAK_DONT_GO_AWAY_YET).then(Label::DontGoAwayWalkBack)
        }
        Label::DontGoAwayWalkBack => {
            rt.simulate_joypad_presses(vec![Joypad::UP]);
            rt.set_player_moving_direction(PLAYER_DIR_UP);
            rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_PLAYER_FORCED_TO_WALK_BACK_SCRIPT;
            Flow::Return
        }
        Label::ForcedToWalkBackDelay => {
            rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_PLAYER_DONT_GO_AWAY_SCRIPT;
            Flow::Return
        }

        Label::RivalChoosesText => {
            rt.display_text_id(TEXT_OAKSLAB_RIVAL_ILL_TAKE_THIS_ONE).then(Label::RivalChoosesHideBall)
        }
        Label::RivalChoosesHideBall => {
            let (.., toggle) = ball(rt.maps().oaks_lab.rival_starter_ball);
            rt.hide_object(toggle);
            rt.delay3().then(Label::RivalChoosesFaceUp)
        }
        Label::RivalChoosesFaceUp => {
            let species = rt.maps().oaks_lab.rival_starter_temp;
            rt.globals().rival_starter = species;
            if let Some(species) = PokemonSpecies::from_repr(species) {
                rt.get_mon_name(species);
            }
            rt.set_sprite_facing_direction_and_delay(OAKSLAB_RIVAL, SPRITE_FACING_UP).then(Label::RivalChoosesReceived)
        }
        Label::RivalChoosesReceived => {
            rt.display_text_id(TEXT_OAKSLAB_RIVAL_RECEIVED_MON).then(Label::RivalChoosesDone)
        }
        Label::RivalChoosesDone => {
            rt.set_event(EVENT_GOT_STARTER);
            rt.joy_ignore(Joypad::empty());
            rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_RIVAL_CHALLENGES_PLAYER;
            Flow::Return
        }

        Label::ChallengesPlayerText => {
            let path = rt.find_path_to_player(OAKSLAB_RIVAL, true, -1);
            rt.move_sprite(OAKSLAB_RIVAL, &path);
            rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_RIVAL_START_BATTLE;
            Flow::Return
        }

        Label::RivalStartsExitText => {
            rt.display_text_id(TEXT_OAKSLAB_RIVAL_SMELL_YOU_LATER).then(Label::PokedexRivalLeaves)
        }
        Label::PokedexRivalLeaves => {
            rt.music_rival_alternate_start();
            // `.RivalExitMovement`, whose leading `NPC_CHANGE_FACING` the code below it overwrites
            // with the sidestep that takes the rival out of the player's column.
            let first = if rt.x() == 4 { NPC_MOVEMENT_RIGHT } else { NPC_MOVEMENT_LEFT };
            rt.move_sprite(OAKSLAB_RIVAL, &[first, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN,
                NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, END]);
            rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_PLAYER_WATCH_RIVAL_EXIT;
            Flow::Return
        }

        Label::RivalArrivesMoves => {
            let steps = calc_rival_movement(rt);
            rt.maps().oaks_lab.saved_steps = steps;
            rt.show_object(TOGGLE_OAKS_LAB_RIVAL);
            let mut path = vec![NPC_MOVEMENT_UP; steps as usize];
            path.push(END);
            rt.move_sprite(OAKSLAB_RIVAL, &path);
            rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_OAK_GIVES_POKEDEX;
            Flow::Return
        }

        Label::PokedexRivalSpeaks => {
            rt.display_text_id(TEXT_OAKSLAB_RIVAL_WHAT_DID_YOU_CALL_ME_FOR).then(Label::PokedexRequest)
        }
        Label::PokedexRequest => {
            rival_face_up_oak_face_down(rt);
            rt.display_text_id(TEXT_OAKSLAB_OAK_I_HAVE_A_REQUEST).then(Label::PokedexInvention)
        }
        Label::PokedexInvention => {
            rival_face_up_oak_face_down(rt);
            rt.display_text_id(TEXT_OAKSLAB_OAK_MY_INVENTION_POKEDEX).then(Label::PokedexGot)
        }
        Label::PokedexGot => rt.display_text_id(TEXT_OAKSLAB_OAK_GOT_POKEDEX).then(Label::PokedexHideBoth),
        Label::PokedexHideBoth => {
            rt.hide_object(TOGGLE_POKEDEX_1);
            rt.hide_object(TOGGLE_POKEDEX_2);
            rival_face_up_oak_face_down(rt);
            rt.display_text_id(TEXT_OAKSLAB_OAK_THAT_WAS_MY_DREAM).then(Label::PokedexThatWasMyDream)
        }
        Label::PokedexThatWasMyDream => {
            rt.set_sprite_facing_direction_and_delay(OAKSLAB_RIVAL, SPRITE_FACING_RIGHT)
                .then(Label::PokedexRivalFacesRight)
        }
        Label::PokedexRivalFacesRight => {
            rt.display_text_id(TEXT_OAKSLAB_RIVAL_LEAVE_IT_ALL_TO_ME).then(Label::PokedexLeaveItToMe)
        }
        Label::PokedexLeaveItToMe => {
            rt.set_event(EVENT_GOT_POKEDEX);
            rt.set_event(EVENT_OAK_GOT_PARCEL);
            rt.hide_object(TOGGLE_LYING_OLD_MAN);
            rt.show_object(TOGGLE_OLD_MAN);
            let steps = rt.maps().oaks_lab.saved_steps;
            rt.play_sound(SoundId::STOP_ALL_MUSIC);
            rt.music_rival_alternate_start();
            let mut path = vec![NPC_MOVEMENT_DOWN; steps as usize];
            path.push(END);
            rt.move_sprite(OAKSLAB_RIVAL, &path);
            rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_RIVAL_LEAVES_WITH_POKEDEX;
            Flow::Return
        }

        Label::Oak1DexRating => rt.display_dex_rating().then(Label::Oak1DexRatingDone),
        Label::Oak1DexRatingDone => Flow::Return,
        Label::Oak1DeliverParcel => {
            rt.remove_item(ItemId::OaksParcel, 1);
            rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_RIVAL_ARRIVES_AT_OAKS_REQUEST;
            Flow::Return
        }

        Label::BallDexShown => {
            let (.., species, _) = ball(rt.sprite_index());
            let words = match species {
                STARTER1 => local::OaksLabYouWantCharmanderText::Text,
                STARTER2 => local::OaksLabYouWantSquirtleText::Text,
                _ => local::OaksLabYouWantBulbasaurText::Text,
            };
            rt.print_text(text_at(words)).then(Label::BallChoiceAnswered)
        }
        // `OaksLabMonChoiceMenu`'s yes/no.
        Label::BallChoiceAnswered => {
            rt.set_do_not_wait_for_button_press(true);
            rt.yes_no_choice().then(Label::BallReceivedMon)
        }
        Label::BallReceivedMon => {
            if !rt.chose_yes() {
                return Flow::Return;
            }
            let (.., species, toggle) = ball(rt.sprite_index());
            rt.maps().oaks_lab.player_starter = species as u8;
            rt.get_mon_name(species);
            rt.hide_object(toggle);
            rt.set_do_not_wait_for_button_press(true);
            rt.print_text(text_at(sym::OaksLabMonEnergeticText)).then(Label::BallAddMon)
        }
        Label::BallAddMon => rt.print_text(text_at(sym::OaksLabReceivedMonText)).then(Label::BallAdded),
        Label::BallAdded => {
            let (.., species, _) = ball(rt.sprite_index());
            rt.add_party_mon(species, 5).then(Label::BallNamed)
        }
        // The pad is only locked once the mon is in, since `AskName` reads it.
        Label::BallNamed => {
            rt.globals().got_starter = true;
            rt.joy_ignore(Joypad::SELECT | Joypad::START | PAD_CTRL_PAD);
            rt.maps().oaks_lab.cur_script = SCRIPT_OAKSLAB_CHOSE_STARTER_SCRIPT;
            Flow::Return
        }
    }
}
