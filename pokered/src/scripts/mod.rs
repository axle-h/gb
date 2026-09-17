//! Map scripts: each map's `<Map>_Script`, run every overworld pass from `RunMapScript`, and the
//! `text_asm` entries of its `<Map>_TextPointers`, as plain Rust in `scripts/<map>.rs`.
//!
//! A map module is five items, each named for what it recreates:
//!
//! - `pub struct State`: the map's `w<Map>CurScript` and any variable of its own that outlives a
//!   pass. It is saved, so it derives serde and `Default`.
//! - `pub enum Label`: every place the map's code resumes after something that takes frames, named
//!   after the source's labels. A variant may carry what the code had worked out before the wait.
//! - `pub fn script(rt: &mut Script) -> Flow`: `<Map>_Script`.
//! - `pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow>`: the entries of the text pointer
//!   table that run code, `None` for the rest, which print from the cartridge as they are.
//! - `pub fn resume(rt: &mut Script, label: Label) -> Flow`: the code after each label.
//!
//! The cartridge's code blocks: `call DisplayTextID` returns once the text is closed, frames later.
//! Here a routine returns a [`Flow`] saying what it waits on and the label to carry on from, and the
//! overworld keeps the labels still to return to on a stack of [`Code`]s. Nothing in it is a
//! closure, so a save in the middle of a cutscene resumes in the middle of it.
//!
//! What the code calls is on [`Script`], by the source's names: `display_text_id`, `print_text`,
//! `yes_no_choice`, `give_item`, `give_pokemon`, `hide_object` and `show_object`, `move_sprite`,
//! `set_sprite_facing_direction_and_delay`, `emotion_bubble`, `find_path_to_player`,
//! `start_npc_movement_script`, `joy_ignore`, the events, `play_music` and `play_sound` with the
//! rival's alternate starts, `wait_for_sound_to_finish`, `delay_frames`. A trainer map's script is
//! `execute_cur_map_script_in_table` into `trainer_script`, and its trainers' texts `talk_to_trainer`;
//! `viridian_forest.rs` is the one to copy. A cutscene is `pallet_town.rs`, a gift `route1.rs`.
//!
//! A map that is not registered below runs `EnableAutoTextBoxDrawing` as its script, which is how
//! nearly every map script in the cartridge begins, and prints only its plain texts. `TEXT_*` and
//! `SCRIPT_*` constants come from `poke_core::symbols::pokered_map_scripts`, local labels from
//! `pokered_local_labels` (`Route1Youngster1Text.GotPotionText` is
//! `pokered_local_labels::Route1Youngster1Text::GotPotionText`).
//!
//! To add a map: write `scripts/<map>.rs`, add its `mod` line and one line to `maps!`.

pub mod agathas_room;
pub mod bills_house;
pub mod brunos_room;
pub mod celadon_city;
pub mod celadon_gym;
pub mod champions_room;
pub mod celadon_diner;
pub mod celadon_mansion_1f;
pub mod celadon_mansion_3f;
pub mod celadon_mansion_roof_house;
pub mod celadon_mart_3f;
pub mod celadon_mart_roof;
pub mod cerulean_city;
pub mod cinnabar_gym;
pub mod cinnabar_island;
pub mod cinnabar_lab_fossil_room;
pub mod cerulean_gym;
pub mod daycare;
pub mod fuchsia_city;
pub mod fuchsia_gym;
pub mod oaks_lab;
pub mod game_corner;
pub mod hall_of_fame;
pub mod indigo_plateau_lobby;
pub mod loreleis_room;
pub mod lances_room;
pub mod lavender_cubone_house;
pub mod lavender_mart;
pub mod lavender_town;
pub mod mr_fujis_house;
pub mod name_raters_house;
pub mod pokemon_mansion_1f;
pub mod pokemon_mansion_b1f;
pub mod pokemon_tower_2f;
pub mod pokemon_tower_3f;
pub mod pokemon_tower_4f;
pub mod pokemon_tower_5f;
pub mod pokemon_tower_6f;
pub mod pokemon_tower_7f;
pub mod rock_tunnel_1f;
pub mod rock_tunnel_b1f;
pub mod route10;
pub mod route8;
pub mod route9;
pub mod mt_moon_1f;
pub mod mt_moon_b2f;
pub mod pallet_town;
pub mod pewter_city;
pub mod pewter_gym;
pub mod reds_house_2f;
pub mod rocket_hideout_b1f;
pub mod rocket_hideout_b2f;
pub mod rocket_hideout_b3f;
pub mod rocket_hideout_b4f;
pub mod route1;
pub mod route24;
pub mod route25;
pub mod route4;
pub mod safari_zone_gate;
pub mod safari_zone_secret_house;
pub mod saffron_gym;
pub mod route5_gate;
pub mod route6;
pub mod route6_gate;
pub mod route7_gate;
pub mod route8_gate;
pub mod route11;
pub mod route16_gate_1f;
pub mod route16_gate_2f;
pub mod route2;
pub mod route22;
pub mod route3;
pub mod route18_gate_1f;
pub mod seafoam_islands_b3f;
pub mod silph_co_1f;
pub mod silph_co_2f;
pub mod silph_co_3f;
pub mod silph_co_11f;
pub mod ss_anne_2f;
pub mod ss_anne_captains_room;
pub mod seafoam_islands_b4f;
pub mod underground_path_route5;
pub mod victory_road_1f;
pub mod victory_road_2f;
pub mod victory_road_3f;
pub mod vermilion_city;
pub mod vermilion_gym;
pub mod vermilion_trade_house;
pub mod viridian_city;
pub mod viridian_forest;
pub mod viridian_gym;
pub mod viridian_mart;
pub mod wardens_house;

#[cfg(test)]
mod tests;

use poke_core::map::Map;
use serde::{Deserialize, Serialize};
pub use crate::modes::overworld::script::{far, text_at, Flow, Routine, Script, Then};

macro_rules! maps {
    ($($map:ident => $module:ident),* $(,)?) => {
        /// A place to carry on from: one of the runtime's own routines or a map's label.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum Code {
            Runtime(Routine),
            Events(crate::modes::overworld::events::Label),
            $($map($module::Label),)*
        }

        $(impl From<$module::Label> for Code {
            fn from(label: $module::Label) -> Self {
                Code::$map(label)
            }
        })*

        /// Every registered map's saved state.
        #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
        pub struct MapStates {
            $(#[serde(default)] pub $module: $module::State,)*
        }

        /// `RunMapScript`'s jump through `wCurMapScriptPtr`.
        pub(crate) fn script(map: Map, rt: &mut Script) -> Flow {
            match map {
                $(Map::$map => $module::script(rt),)*
                _ => {
                    rt.enable_auto_text_box_drawing();
                    Flow::Return
                }
            }
        }

        /// The code behind a text id, when the map's text pointer is `text_asm`.
        pub(crate) fn text(map: Map, rt: &mut Script, text_id: u8) -> Option<Flow> {
            match map {
                $(Map::$map => $module::text(rt, text_id),)*
                _ => None,
            }
        }

        pub(crate) fn resume(code: Code, rt: &mut Script) -> Flow {
            match code {
                Code::Runtime(routine) => rt.routine(routine),
                Code::Events(label) => crate::modes::overworld::events::resume(rt, label),
                $(Code::$map(label) => $module::resume(rt, label),)*
            }
        }
    };
}

maps! {
    AgathasRoom => agathas_room,
    BillsHouse => bills_house,
    BrunosRoom => brunos_room,
    CeladonCity => celadon_city,
    CeladonGym => celadon_gym,
    ChampionsRoom => champions_room,
    CeladonDiner => celadon_diner,
    CeladonMansion1F => celadon_mansion_1f,
    CeladonMansion3F => celadon_mansion_3f,
    CeladonMansionRoofHouse => celadon_mansion_roof_house,
    CeladonMart3F => celadon_mart_3f,
    CeladonMartRoof => celadon_mart_roof,
    CeruleanCity => cerulean_city,
    CinnabarGym => cinnabar_gym,
    CinnabarIsland => cinnabar_island,
    CinnabarLabFossilRoom => cinnabar_lab_fossil_room,
    CeruleanGym => cerulean_gym,
    Daycare => daycare,
    FuchsiaCity => fuchsia_city,
    FuchsiaGym => fuchsia_gym,
    OaksLab => oaks_lab,
    GameCorner => game_corner,
    HallOfFame => hall_of_fame,
    IndigoPlateauLobby => indigo_plateau_lobby,
    LoreleisRoom => loreleis_room,
    LancesRoom => lances_room,
    LavenderCuboneHouse => lavender_cubone_house,
    LavenderMart => lavender_mart,
    LavenderTown => lavender_town,
    MrFujisHouse => mr_fujis_house,
    NameRatersHouse => name_raters_house,
    PokemonMansion1F => pokemon_mansion_1f,
    PokemonMansionB1F => pokemon_mansion_b1f,
    PokemonTower2F => pokemon_tower_2f,
    PokemonTower3F => pokemon_tower_3f,
    PokemonTower4F => pokemon_tower_4f,
    PokemonTower5F => pokemon_tower_5f,
    PokemonTower6F => pokemon_tower_6f,
    PokemonTower7F => pokemon_tower_7f,
    RockTunnel1F => rock_tunnel_1f,
    RockTunnelB1F => rock_tunnel_b1f,
    Route8 => route8,
    Route9 => route9,
    Route10 => route10,
    MtMoon1F => mt_moon_1f,
    MtMoonB2F => mt_moon_b2f,
    PalletTown => pallet_town,
    PewterCity => pewter_city,
    PewterGym => pewter_gym,
    RedsHouse2F => reds_house_2f,
    RocketHideoutB1F => rocket_hideout_b1f,
    RocketHideoutB2F => rocket_hideout_b2f,
    RocketHideoutB3F => rocket_hideout_b3f,
    RocketHideoutB4F => rocket_hideout_b4f,
    Route1 => route1,
    Route24 => route24,
    Route25 => route25,
    Route4 => route4,
    SafariZoneGate => safari_zone_gate,
    SafariZoneSecretHouse => safari_zone_secret_house,
    SaffronGym => saffron_gym,
    Route5Gate => route5_gate,
    Route6 => route6,
    Route6Gate => route6_gate,
    Route7Gate => route7_gate,
    Route8Gate => route8_gate,
    Route11 => route11,
    Route16Gate1F => route16_gate_1f,
    Route16Gate2F => route16_gate_2f,
    Route2 => route2,
    Route22 => route22,
    Route3 => route3,
    Route18Gate1F => route18_gate_1f,
    SeafoamIslandsB3F => seafoam_islands_b3f,
    SilphCo1F => silph_co_1f,
    SilphCo2F => silph_co_2f,
    SilphCo3F => silph_co_3f,
    SilphCo11F => silph_co_11f,
    SSAnne2F => ss_anne_2f,
    SSAnneCaptainsRoom => ss_anne_captains_room,
    SeafoamIslandsB4F => seafoam_islands_b4f,
    UndergroundPathRoute5 => underground_path_route5,
    VictoryRoad1F => victory_road_1f,
    VictoryRoad2F => victory_road_2f,
    VictoryRoad3F => victory_road_3f,
    VermilionCity => vermilion_city,
    VermilionGym => vermilion_gym,
    VermilionTradeHouse => vermilion_trade_house,
    ViridianCity => viridian_city,
    ViridianForest => viridian_forest,
    ViridianGym => viridian_gym,
    ViridianMart => viridian_mart,
    WardensHouse => wardens_house,
}

impl From<crate::modes::overworld::events::Label> for Code {
    fn from(label: crate::modes::overworld::events::Label) -> Self {
        Code::Events(label)
    }
}

impl From<Routine> for Code {
    fn from(routine: Routine) -> Self {
        Code::Runtime(routine)
    }
}

/// What the scripts keep that a save carries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptState {
    pub maps: MapStates,
    /// `wCurMapScript`: the index a trainer map's table runs, which its routines move on.
    #[serde(default)]
    pub cur_map_script: u8,
    /// `wRivalStarter`, which the rival's parties are chosen by.
    #[serde(default)]
    pub rival_starter: u8,
    /// `BIT_GOT_STARTER`, which Oak's lab sets and Red's house reads.
    #[serde(default)]
    pub got_starter: bool,
    /// `wStatusFlags1`'s `BIT_GAVE_SAFFRON_GUARDS_DRINK`: one drink opens all four Saffron gates.
    #[serde(default)]
    pub gave_saffron_guards_drink: bool,
    /// `wFirstLockTrashCanIndex` and `wSecondLockTrashCanIndex`: the Vermilion Gym's two locks.
    #[serde(default)]
    pub trash_cans: [u8; 2],
    /// `wElite4Flags`' `BIT_STARTED_ELITE_4`: set by walking into Lorelei's room, and what tells the
    /// lobby a challenge was left half fought.
    #[serde(default)]
    pub started_elite_4: bool,
}
