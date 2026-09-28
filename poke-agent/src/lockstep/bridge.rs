//! The recreation re-made from wherever the cartridge stands: its `World` and its overworld, read
//! out of WRAM and SRAM. Everything a save keeps and every map's script state is carried, so a
//! difference after an action is the recreation's rather than this bridge's.

use gb::game_boy::GameBoy;
use gb::ram::ROM;
use poke_core::map::Map;
use poke_core::move_name::PokemonMoveName;
use poke_core::species::PokemonSpecies;
use poke_core::sprite::SpriteFacing;
use poke_core::map_objects::Warp;
use pokered::mode::Mode;
use pokered::modes::overworld::{Overworld, Standing};
use pokered::party::{BoxMon, Named, Pokedex};
use pokered::rng::GameRng;
use pokered::scripts::MapStates;
use pokered::systems::hall_of_fame::{HallOfFameMon, HOF_TEAM_CAPACITY};
use pokered::systems::inventory::Inventory;
use pokered::systems::overworld::location::Ahead;
use pokered::systems::overworld::sprites::{SpriteState, Sprites};
use pokered::systems::overworld::Location;
use pokered::systems::play_time::PlayTime;
use pokered::systems::stats::Dvs;
use pokered::world::{BattleStyle, TextSpeed, World};
use pokered::{Game, Pacing};
use crate::pokemon::item::ItemId;
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointerRead};

const NAME_LENGTH: usize = 11;
const TERMINATOR: u8 = 0x50;
const MONS_PER_BOX: usize = 20;
const NUM_BOXES: usize = 12;
const BOX_STRUCT: usize = 33;
/// `wBoxDataStart` to `wBoxDataEnd`, which is also one `sBox`.
const BOX_DATA: usize = 1 + MONS_PER_BOX + 1 + MONS_PER_BOX * (BOX_STRUCT + 2 * NAME_LENGTH);
/// `sBox1` and `sBox7`, as offsets into `dump_sram`'s banks laid end to end.
const S_BOX1: usize = 2 * 0x2000;
const S_BOX7: usize = 3 * 0x2000;
/// `sHallOfFame`'s offset into the first SRAM bank, and `HOF_MON`.
const S_HALL_OF_FAME: usize = 0x598;
const HOF_MON: usize = 16;
const HOF_TEAM: usize = 6 * HOF_MON;
const SPRITE_STATE_BYTES: u16 = 16;
/// `wOverworldMap`'s border, in blocks, round every side.
const MAP_BORDER: usize = 3;

// The flags `World` keeps as fields, by the byte that holds them.
const BIT_STRENGTH_ACTIVE: u8 = 0; // wStatusFlags1
const BIT_GOT_OLD_ROD: u8 = 3;
const BIT_GOT_GOOD_ROD: u8 = 4;
const BIT_GOT_SUPER_ROD: u8 = 5;
const BIT_GAVE_SAFFRON_GUARDS_DRINK: u8 = 6;
const BIT_GOT_LAPRAS: u8 = 0; // wStatusFlags4
const BIT_USED_POKECENTER: u8 = 2;
const BIT_GOT_STARTER: u8 = 3;
const BIT_NO_BATTLES: u8 = 4;
const BIT_NO_TEXT_DELAY: u8 = 6; // wStatusFlags5
const BIT_GAME_TIMER_COUNTING: u8 = 0; // wStatusFlags6
const BIT_ALWAYS_ON_BIKE: u8 = 5;
const BIT_STARTED_ELITE_4: u8 = 1; // wElite4Flags
const BIT_FAST_TEXT_DELAY: u8 = 0; // wLetterPrintingDelayFlags
const BIT_HAS_CHANGED_BOXES: u8 = 7; // wCurrentBoxNum

fn name(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().copied().take(NAME_LENGTH).take_while(|&byte| byte != TERMINATOR).collect()
}

/// A `box_struct`, with the nickname and OT beside it.
pub(super) fn box_mon(b: &[u8], ot: &[u8], nick: &[u8]) -> Named<BoxMon> {
    let word = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
    Named {
        mon: BoxMon {
            species: PokemonSpecies::from_repr(b[0]).expect("a species"),
            hp: word(1),
            box_level: b[3],
            status: b[4],
            types: [b[5], b[6]],
            catch_rate: b[7],
            moves: [0, 1, 2, 3].map(|i| PokemonMoveName::from_repr(b[8 + i])),
            ot_id: word(12),
            exp: u32::from_be_bytes([0, b[14], b[15], b[16]]),
            stat_exp: [0, 1, 2, 3, 4].map(|i| word(17 + 2 * i)),
            dvs: Dvs([b[27], b[28]]),
            pp: [b[29], b[30], b[31], b[32]],
        },
        ot: name(ot),
        nick: name(nick),
    }
}

/// One box as `wBoxDataStart` and every `sBox` lay it out: the count, the species list, the mons,
/// then the OTs and the nicknames.
fn box_data(data: &[u8]) -> Vec<Named<BoxMon>> {
    let count = (data[0] as usize).min(MONS_PER_BOX);
    let mons = 1 + MONS_PER_BOX + 1;
    let ots = mons + MONS_PER_BOX * BOX_STRUCT;
    let nicks = ots + MONS_PER_BOX * NAME_LENGTH;
    (0..count).map(|i| box_mon(
        &data[mons + i * BOX_STRUCT..][..BOX_STRUCT],
        &data[ots + i * NAME_LENGTH..][..NAME_LENGTH],
        &data[nicks + i * NAME_LENGTH..][..NAME_LENGTH],
    )).collect()
}

/// `sHallOfFame`'s team `index`, as far as its `$FF`.
pub(super) fn sram_team(sram: &[u8], index: usize) -> Vec<HallOfFameMon> {
    let team = &sram[S_HALL_OF_FAME + index * HOF_TEAM..S_HALL_OF_FAME + (index + 1) * HOF_TEAM];
    team.chunks_exact(HOF_MON).take_while(|entry| entry[0] != 0xFF).map(|entry| HallOfFameMon {
        species: PokemonSpecies::from_repr(entry[0]).expect("a species"),
        level: entry[1],
        nick: name(&entry[2..13]),
    }).collect()
}

macro_rules! cur_scripts {
    ($($module:ident => $symbol:ident),* $(,)?) => {
        /// Every map's `w<Map>CurScript`, one to one with the recreation's map modules that keep one.
        fn cur_scripts(gb: &GameBoy, maps: &mut MapStates) {
            let mmu = gb.core().mmu();
            $(maps.$module.cur_script = mmu.read_pointer(&sym::$symbol);)*
        }
    };
}

cur_scripts! {
    agathas_room => wAgathasRoomCurScript,
    bills_house => wBillsHouseCurScript,
    brunos_room => wBrunosRoomCurScript,
    celadon_gym => wCeladonGymCurScript,
    champions_room => wChampionsRoomCurScript,
    cerulean_city => wCeruleanCityCurScript,
    cinnabar_gym => wCinnabarGymCurScript,
    cinnabar_island => wCinnabarIslandCurScript,
    cerulean_gym => wCeruleanGymCurScript,
    fuchsia_gym => wFuchsiaGymCurScript,
    oaks_lab => wOaksLabCurScript,
    game_corner => wGameCornerCurScript,
    hall_of_fame => wHallOfFameCurScript,
    loreleis_room => wLoreleisRoomCurScript,
    lances_room => wLancesRoomCurScript,
    pokemon_mansion_1f => wPokemonMansion1FCurScript,
    pokemon_mansion_b1f => wPokemonMansionB1FCurScript,
    pokemon_tower_2f => wPokemonTower2FCurScript,
    pokemon_tower_3f => wPokemonTower3FCurScript,
    pokemon_tower_4f => wPokemonTower4FCurScript,
    pokemon_tower_5f => wPokemonTower5FCurScript,
    pokemon_tower_6f => wPokemonTower6FCurScript,
    pokemon_tower_7f => wPokemonTower7FCurScript,
    rock_tunnel_1f => wRockTunnel1FCurScript,
    rock_tunnel_b1f => wRockTunnelB1FCurScript,
    route8 => wRoute8CurScript,
    route9 => wRoute9CurScript,
    route10 => wRoute10CurScript,
    mt_moon_1f => wMtMoon1FCurScript,
    mt_moon_b2f => wMtMoonB2FCurScript,
    pallet_town => wPalletTownCurScript,
    pewter_city => wPewterCityCurScript,
    pewter_gym => wPewterGymCurScript,
    reds_house_2f => wRedsHouse2FCurScript,
    rocket_hideout_b1f => wRocketHideoutB1FCurScript,
    rocket_hideout_b2f => wRocketHideoutB2FCurScript,
    rocket_hideout_b3f => wRocketHideoutB3FCurScript,
    rocket_hideout_b4f => wRocketHideoutB4FCurScript,
    route24 => wRoute24CurScript,
    route25 => wRoute25CurScript,
    route4 => wRoute4CurScript,
    safari_zone_gate => wSafariZoneGateCurScript,
    saffron_gym => wSaffronGymCurScript,
    route5_gate => wRoute5GateCurScript,
    route6 => wRoute6CurScript,
    route6_gate => wRoute6GateCurScript,
    route7_gate => wRoute7GateCurScript,
    route8_gate => wRoute8GateCurScript,
    route11 => wRoute11CurScript,
    route16_gate_1f => wRoute16Gate1FCurScript,
    route22 => wRoute22CurScript,
    route3 => wRoute3CurScript,
    route18_gate_1f => wRoute18Gate1FCurScript,
    seafoam_islands_b3f => wSeafoamIslandsB3FCurScript,
    silph_co_2f => wSilphCo2FCurScript,
    silph_co_3f => wSilphCo3FCurScript,
    silph_co_4f => wSilphCo4FCurScript,
    silph_co_5f => wSilphCo5FCurScript,
    silph_co_6f => wSilphCo6FCurScript,
    silph_co_11f => wSilphCo11FCurScript,
    ss_anne_2f => wSSAnne2FCurScript,
    seafoam_islands_b4f => wSeafoamIslandsB4FCurScript,
    victory_road_1f => wVictoryRoad1FCurScript,
    victory_road_2f => wVictoryRoad2FCurScript,
    victory_road_3f => wVictoryRoad3FCurScript,
    vermilion_city => wVermilionCityCurScript,
    vermilion_gym => wVermilionGymCurScript,
    viridian_city => wViridianCityCurScript,
    viridian_forest => wViridianForestCurScript,
    viridian_gym => wViridianGymCurScript,
    viridian_mart => wViridianMartCurScript,
    blues_house => wBluesHouseCurScript,
    route22_gate => wRoute22GateCurScript,
    ss_anne_bow => wSSAnneBowCurScript,
    ss_anne_1f_rooms => wSSAnne1FRoomsCurScript,
    ss_anne_b1f_rooms => wSSAnneB1FRoomsCurScript,
    ss_anne_2f_rooms => wSSAnne2FRoomsCurScript,
    route12 => wRoute12CurScript,
    route13 => wRoute13CurScript,
    route14 => wRoute14CurScript,
    route15 => wRoute15CurScript,
    route16 => wRoute16CurScript,
    route17 => wRoute17CurScript,
    route18 => wRoute18CurScript,
    route19 => wRoute19CurScript,
    route20 => wRoute20CurScript,
    route21 => wRoute21CurScript,
    fighting_dojo => wFightingDojoCurScript,
    silph_co_7f => wSilphCo7FCurScript,
    silph_co_8f => wSilphCo8FCurScript,
    silph_co_9f => wSilphCo9FCurScript,
    silph_co_10f => wSilphCo10FCurScript,
    pokemon_mansion_2f => wPokemonMansion2FCurScript,
    pokemon_mansion_3f => wPokemonMansion3FCurScript,
    museum_1f => wMuseum1FCurScript,
    power_plant => wPowerPlantCurScript,
    cerulean_cave_b1f => wCeruleanCaveB1FCurScript,
    route23 => wRoute23CurScript,
}

pub(super) fn sprites(gb: &GameBoy) -> Sprites {
    let mmu = gb.core().mmu();
    std::array::from_fn(|slot| {
        let at = slot as u16 * SPRITE_STATE_BYTES;
        let data1 = mmu.read_slice(sym::wSpriteStateData1.address + at, 16);
        let data2 = mmu.read_slice(sym::wSpriteStateData2.address + at, 16);
        let map_data = if slot == 0 { [0, 0] } else {
            let entry = sym::wMapSpriteData.address + (slot as u16 - 1) * 2;
            [mmu.read(entry), mmu.read(entry + 1)]
        };
        SpriteState::from_bytes(&data1, &data2, map_data)
    })
}

pub(super) fn standing(gb: &GameBoy) -> Standing {
    let mmu = gb.core().mmu();
    Standing {
        player_direction: mmu.read_pointer(&sym::wPlayerDirection),
        moving_direction: mmu.read_pointer(&sym::wPlayerMovingDirection),
        last_stop_direction: mmu.read_pointer(&sym::wPlayerLastStopDirection),
        check_for_180_degree_turn: mmu.read_pointer(&sym::wCheckFor180DegreeTurn),
        standing_on_warp: mmu.read_pointer(&sym::wMovementFlags) & 1 << 2 != 0,
        destination_warp: mmu.read_pointer(&sym::wDestinationWarpID),
    }
}

pub(super) fn location(gb: &GameBoy) -> Location {
    let mmu = gb.core().mmu();
    Location {
        map: Map::from_repr(mmu.read_pointer(&sym::wCurMap)).unwrap(),
        x: mmu.read_pointer(&sym::wXCoord),
        y: mmu.read_pointer(&sym::wYCoord),
        facing: SpriteFacing::from_repr(mmu.read(sym::wSpriteStateData1.address + 9)).unwrap(),
        last_map: Map::from_repr(mmu.read_pointer(&sym::wLastMap)).unwrap(),
        walk_bike_surf: mmu.read_pointer(&sym::wWalkBikeSurfState),
        hidden_objects: mmu.read_slice(sym::wToggleableObjectFlags.address, 32),
        towns_visited: mmu.read_u16_le(sym::wTownVisitedFlag.address),
        last_blackout_map: Map::from_repr(mmu.read_pointer(&sym::wLastBlackoutMap)).unwrap(),
        repel_steps: mmu.read_pointer(&sym::wRepelRemainingSteps),
        always_on_bike: mmu.read_pointer(&sym::wStatusFlags6) & 1 << BIT_ALWAYS_ON_BIKE != 0,
        ahead: Ahead {
            tile: mmu.read_pointer(&sym::wTileInFrontOfPlayer),
            standing_on: mmu.read_pointer(&sym::wTilePlayerStandingOn),
            sprite: false,
        },
        strength_active: mmu.read_pointer(&sym::wStatusFlags1) & 1 << BIT_STRENGTH_ACTIVE != 0,
        used_field_move: None,
        fly_warp: None,
        escape_warp: false,
    }
}

/// The whole `World`, from WRAM and, for the boxes other than the open one and the Hall of Fame,
/// from SRAM, which the cartridge writes those to as it changes them.
pub(super) fn world(gb: &GameBoy) -> World {
    let mmu = gb.core().mmu();
    let flag = |pointer, bit: u8| mmu.read_pointer(pointer) & 1 << bit != 0;
    let names = |at: u16| name(&mmu.read_slice(at, NAME_LENGTH));
    let items = |count: &_, at: u16| (0..mmu.read_pointer(count) as u16)
        .map(|i| poke_core::bag::BagItem::new(ItemId::from_repr(mmu.read(at + 2 * i)).expect("an item"),
                                              mmu.read(at + 2 * i + 1)))
        .collect();
    let sram = gb.dump_sram();

    let mut world = World {
        player_name: names(sym::wPlayerName.address),
        rival_name: names(sym::wRivalName.address),
        party: super::status_screen::the_party(gb),
        pokedex: Pokedex {
            owned: mmu.read_slice(sym::wPokedexOwned.address, 19).try_into().unwrap(),
            seen: mmu.read_slice(sym::wPokedexSeen.address, 19).try_into().unwrap(),
        },
        bag: Inventory::bag(items(&sym::wNumBagItems, sym::wBagItems.address)),
        money: mmu.read_slice(sym::wPlayerMoney.address, 3).try_into().unwrap(),
        badges: mmu.read_pointer(&sym::wObtainedBadges),
        play_time: PlayTime {
            hours: mmu.read_pointer(&sym::wPlayTimeHours),
            maxed: mmu.read_pointer(&sym::wPlayTimeMaxed) != 0,
            minutes: mmu.read_pointer(&sym::wPlayTimeMinutes),
            seconds: mmu.read_pointer(&sym::wPlayTimeSeconds),
            frames: mmu.read_pointer(&sym::wPlayTimeFrames),
            counting: flag(&sym::wStatusFlags6, BIT_GAME_TIMER_COUNTING),
        },
        player_id: mmu.read_u16_be(sym::wPlayerID.address),
        current_box: mmu.read_pointer(&sym::wCurrentBoxNum) & !(1 << BIT_HAS_CHANGED_BOXES),
        safari_balls: mmu.read_pointer(&sym::wNumSafariBalls),
        pc_items: Inventory::pc(items(&sym::wNumBoxItems, sym::wBoxItems.address)),
        hall_of_fame_teams: mmu.read_pointer(&sym::wNumHoFTeams),
        one_frame_letter_delay: !flag(&sym::wLetterPrintingDelayFlags, BIT_FAST_TEXT_DELAY),
        coins: mmu.read_slice(sym::wPlayerCoins.address, 2).try_into().unwrap(),
        hidden_items: mmu.read_slice(sym::wObtainedHiddenItemsFlags.address, 14).try_into().unwrap(),
        hidden_coins: mmu.read_slice(sym::wObtainedHiddenCoinsFlags.address, 2).try_into().unwrap(),
        in_game_trades: mmu.read_u16_le(sym::wCompletedInGameTradeFlags.address),
        used_pokecenter: flag(&sym::wStatusFlags4, BIT_USED_POKECENTER),
        safari_steps: mmu.read_u16_be(sym::wSafariSteps.address),
        no_text_delay: flag(&sym::wStatusFlags5, BIT_NO_TEXT_DELAY),
        location: location(gb),
        ..World::default()
    };

    let options = mmu.read_pointer(&sym::wOptions);
    world.options.text_speed = match options & 0xF {
        1 => TextSpeed::Fast,
        5 => TextSpeed::Slow,
        _ => TextSpeed::Medium,
    };
    world.options.battle_animation = options & 1 << 7 == 0;
    world.options.battle_style = if options & 1 << 6 != 0 { BattleStyle::Set } else { BattleStyle::Shift };

    let events = mmu.read_slice(sym::wEventFlags.address, pokered::world::NUM_EVENTS / 8);
    for event in 0..pokered::world::NUM_EVENTS {
        if events[event / 8] & 1 << (event % 8) != 0 {
            world.events.set(event as u16);
        }
    }

    let open_box = box_data(&mmu.read_slice(sym::wBoxDataStart.address, BOX_DATA));
    world.boxes = if flag(&sym::wCurrentBoxNum, BIT_HAS_CHANGED_BOXES) {
        (0..NUM_BOXES).map(|i| if i == world.current_box as usize { open_box.clone() } else {
            let at = if i < 6 { S_BOX1 + i * BOX_DATA } else { S_BOX7 + (i - 6) * BOX_DATA };
            box_data(&sram[at..at + BOX_DATA])
        }).collect()
    } else {
        // SRAM's boxes are not initialised until the first change of box, and until then the open
        // box is the only one.
        let mut boxes = vec![Vec::new(); world.current_box as usize + 1];
        boxes[world.current_box as usize] = open_box;
        boxes
    };
    world.hall_of_fame = (0..(world.hall_of_fame_teams as usize).min(HOF_TEAM_CAPACITY))
        .map(|i| sram_team(&sram, i)).collect();

    if mmu.read_pointer(&sym::wDayCareInUse) != 0 {
        world.day_care = Some(box_mon(
            &mmu.read_slice(sym::wDayCareMon.address, BOX_STRUCT),
            &mmu.read_slice(sym::wDayCareMonOT.address, NAME_LENGTH),
            &mmu.read_slice(sym::wDayCareMonName.address, NAME_LENGTH),
        ));
    }
    let fossil = mmu.read_pointer(&sym::wFossilItem);
    if fossil != 0 {
        world.fossil = Some((ItemId::from_repr(fossil).expect("a fossil"),
                             PokemonSpecies::from_repr(mmu.read_pointer(&sym::wFossilMon)).expect("a species")));
    }

    let scripts = &mut world.scripts;
    scripts.cur_map_script = mmu.read_pointer(&sym::wCurMapScript);
    scripts.rival_starter = mmu.read_pointer(&sym::wRivalStarter);
    scripts.got_starter = flag(&sym::wStatusFlags4, BIT_GOT_STARTER);
    scripts.gave_saffron_guards_drink = flag(&sym::wStatusFlags1, BIT_GAVE_SAFFRON_GUARDS_DRINK);
    scripts.got_lapras = flag(&sym::wStatusFlags4, BIT_GOT_LAPRAS);
    scripts.got_old_rod = flag(&sym::wStatusFlags1, BIT_GOT_OLD_ROD);
    scripts.got_good_rod = flag(&sym::wStatusFlags1, BIT_GOT_GOOD_ROD);
    scripts.got_super_rod = flag(&sym::wStatusFlags1, BIT_GOT_SUPER_ROD);
    scripts.trash_cans = [mmu.read_pointer(&sym::wFirstLockTrashCanIndex), mmu.read_pointer(&sym::wSecondLockTrashCanIndex)];
    scripts.started_elite_4 = flag(&sym::wElite4Flags, BIT_STARTED_ELITE_4);

    let maps = &mut scripts.maps;
    cur_scripts(gb, maps);
    // One byte under three names: which one it is depends on the map that set it.
    let saved_coord_index = mmu.read_pointer(&sym::wSavedCoordIndex);
    maps.pallet_town.oak_walked_to_player = saved_coord_index != 0;
    maps.safari_zone_gate.next_script = saved_coord_index;
    maps.route22.coord_index = saved_coord_index;
    maps.fighting_dojo.saved_coord_index = saved_coord_index;
    maps.silph_co_7f.saved_coord_index = saved_coord_index;
    maps.silph_co_11f.saved_coord_index = saved_coord_index;
    maps.cinnabar_gym.trainer_header_flag_bit = mmu.read_pointer(&sym::wTrainerHeaderFlagBit);
    let oaks_lab = &mut maps.oaks_lab;
    oaks_lab.player_starter = mmu.read_pointer(&sym::wPlayerStarter);
    oaks_lab.rival_starter_temp = mmu.read_pointer(&sym::wRivalStarterTemp);
    oaks_lab.rival_starter_ball = mmu.read_pointer(&sym::wRivalStarterBallSpriteIndex);
    oaks_lab.saved_steps = mmu.read_pointer(&sym::wSavedNPCMovementDirections2Index);
    world
}

/// The overworld the cartridge is standing in, its sprites where they are and its blocks as the
/// map's load code left them.
pub(super) fn overworld(gb: &GameBoy) -> Overworld {
    let mmu = gb.core().mmu();
    let stride = mmu.read_pointer(&sym::wCurMapWidth) as usize + 2 * MAP_BORDER;
    let blocks = stride * (mmu.read_pointer(&sym::wCurMapHeight) as usize + 2 * MAP_BORDER);
    Overworld::standing(sprites(gb), mmu.read_pointer(&sym::wNumSprites), standing(gb))
        .with_battle_flags(
            mmu.read_pointer(&sym::wStatusFlags4) & 1 << BIT_NO_BATTLES != 0,
            mmu.read_pointer(&sym::wStatusFlags2) & 1 != 0,
            mmu.read_pointer(&sym::wNumberOfNoRandomBattleStepsLeft),
        )
        .with_step_counter(mmu.read_pointer(&sym::wStepCounter))
        .with_blocks(mmu.read_slice(sym::wOverworldMap.address, blocks))
        .with_warps(mmu.read_slice(sym::wWarpEntries.address, 4 * mmu.read_pointer(&sym::wNumberOfWarps) as usize)
            .chunks(4).map(|entry| Warp { y: entry[0], x: entry[1], destination_warp: entry[2], destination_map: entry[3] })
            .collect())
}

/// The recreation re-made from a cartridge that has just polled in the overworld.
pub(super) fn game(gb: &GameBoy, rng: GameRng) -> Game {
    let mut game = Game::new(world(gb), rng, Pacing::Faithful);
    game.push(Mode::Overworld(overworld(gb)));
    game
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::native::NativeGame;
    use crate::pokemon::{PokemonApi, PokemonApiTrait};
    use super::super::game_state::{compared, differences};
    use super::super::overworld::Cartridge;

    /// Long enough for a fixture saved mid-walk to finish the step and poll; one saved in a battle,
    /// a text or a menu never polls without a press, and is not what this compares.
    const BUDGET: u32 = 300;

    #[test]
    fn every_fixture_standing_in_the_overworld_is_re_made_as_the_same_game_state() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pokemon/data");
        let mut states: Vec<_> = std::fs::read_dir(&dir).unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|e| e == "bin"))
            .collect();
        states.sort();
        let (mut bridged, mut skipped, mut failed) = (0, Vec::new(), Vec::new());
        for path in &states {
            let what = path.file_name().unwrap().to_string_lossy().into_owned();
            let mut cartridge = Cartridge::from_state(&std::fs::read(path).unwrap());
            if !(0..BUDGET).any(|_| cartridge.frame()) {
                skipped.push(what);
                continue;
            }
            let theirs = compared(&PokemonApi::new(&mut cartridge.gb).game_state().unwrap());
            let native = NativeGame::new(game(&cartridge.gb, GameRng::seeded(0))).unwrap();
            let ours = compared(&native.game_state().unwrap());
            bridged += 1;
            if ours != theirs {
                failed.push(format!("{what}:\n{}", differences(&ours, &theirs).join("\n")));
            }
        }
        println!("bridged {bridged} of {}; skipped {skipped:?}", states.len());
        assert!(failed.is_empty(), "{} of {bridged} differ:\n{}", failed.len(), failed.join("\n"));
        assert!(bridged >= states.len() * 3 / 4, "only {bridged} of {} polled: {skipped:?}", states.len());
    }
}
