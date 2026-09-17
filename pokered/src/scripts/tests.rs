//! The exemplar maps played end to end through the game's own loop, by commands.

use poke_core::charmap::encode;
use poke_core::item::ItemId;
use poke_core::map::Map;
use poke_core::species::PokemonSpecies;
use poke_core::sprite::SpriteFacing;
use poke_core::symbols::pokered_events::*;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_CHAMPIONSROOM_PLAYER_ENTERS, SCRIPT_CHAMPIONSROOM_RIVAL_DEFEATED};
use poke_core::symbols::pokered_toggles::TOGGLE_PALLET_TOWN_OAK;
use crate::command::{Command, Decision, Reply};
use crate::mode::{Mode, Status};
use crate::modes::overworld::Overworld;
use crate::party::Named;
use crate::rng::GameRng;
use crate::systems::add_mon::{new_party_mon, Origin};
use crate::systems::overworld::{Direction, Location};
use crate::world::World;
use crate::{Game, Input, Pacing};

fn mon(species: PokemonSpecies, level: u8) -> Named<crate::party::PartyMon> {
    Named {
        mon: new_party_mon(species, level, 1, &Origin::Trainer, &mut GameRng::tape(vec![])),
        ot: encode("RED").unwrap(),
        nick: encode("MON").unwrap(),
    }
}

fn game(map: Map, x: u8, y: u8, facing: SpriteFacing, seed: u64, setup: impl FnOnce(&mut World)) -> Game {
    let mut world = World { player_name: encode("RED").unwrap(), ..World::default() };
    world.location = Location { map, x, y, facing, last_map: Map::PalletTown, ..Location::default() };
    world.party = vec![mon(PokemonSpecies::Pidgey, 40)];
    world.events.set(EVENT_FOLLOWED_OAK_INTO_LAB);
    setup(&mut world);
    let mut game = Game::new(world, GameRng::seeded(seed), Pacing::Faithful);
    game.push(Mode::Overworld(Overworld::new()));
    game
}

/// Answers whatever is asked, a text with A and a battle with its first move, until `done`.
fn play_until(game: &mut Game, limit: u32, mut done: impl FnMut(&Game) -> bool) {
    for _ in 0..limit {
        if done(game) {
            return;
        }
        let input = match game.status() {
            Status::Waiting(Decision::Text) => Input::Command(Command::Advance),
            Status::Waiting(Decision::BattleMenu | Decision::BattleMoves) => Input::Command(Command::Fight(0)),
            Status::Waiting(Decision::TwoOption) => Input::Command(Command::ChooseOption(1)),
            _ => Input::None,
        };
        game.frame(input);
    }
    panic!("never done: {:?} on {:?}", game.status(), game.modes().last().map(std::mem::discriminant));
}

fn free(game: &Game) -> bool {
    game.status() == Status::Waiting(Decision::Overworld)
}

/// A party that settles a scripted battle in a turn rather than grinding it out or being slept
/// through: these tests are about what the map does either side of a battle, not the battle.
fn one_shot(world: &mut World) {
    world.party = vec![mon(PokemonSpecies::Mewtwo, 70)];
}

/// A toggleable object a new game starts with hidden, which a script somewhere else would show.
fn show(world: &mut World, toggle: u16) {
    let index = toggle as usize;
    world.location.hidden_objects[index / 8] &= !(1 << (index % 8));
}

fn command(game: &mut Game, command: Command) {
    for _ in 0..200 {
        match game.frame(Input::Command(command.clone())).reply {
            Some(Reply::Accepted) => return,
            _ => game.frame(Input::None),
        };
    }
    panic!("{command:?} was never accepted");
}

#[test]
fn oak_stops_the_player_at_the_grass_and_leads_them_into_his_lab() {
    let mut game = game(Map::PalletTown, 10, 2, SpriteFacing::Up, 3, |world| world.events.clear(EVENT_FOLLOWED_OAK_INTO_LAB));
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 3000, |game| game.world().location.map == Map::OaksLab);
    // The movement script ends on the lab's first pass, taking Oak out of Pallet Town.
    play_until(&mut game, 100, |game| game.world().location.is_hidden(TOGGLE_PALLET_TOWN_OAK as u8));
    let world = game.world();
    assert!(world.events.is_set(EVENT_OAK_APPEARED_IN_PALLET));
    assert_eq!((world.location.x, world.location.y), (5, 11), "the lab's door");
    assert_eq!(world.scripts.maps.pallet_town.cur_script, poke_core::symbols::pokered_map_scripts::SCRIPT_PALLETTOWN_PLAYER_FOLLOWS_OAK);
}

#[test]
fn the_route_1_clerk_hands_over_a_potion() {
    let mut game = game(Map::Route1, 5, 26, SpriteFacing::Up, 1, |world| world.party.clear());
    play_until(&mut game, 600, free);
    // The clerk wanders up and down his column: wait for him to stand right above the player.
    for _ in 0..3000 {
        if let Some(Mode::Overworld(overworld)) = game.modes().last()
            && overworld.in_front_text(game.world()) == 1
            && free(&game)
        {
            break;
        }
        game.frame(Input::None);
    }
    command(&mut game, Command::Interact);
    play_until(&mut game, 2000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::Potion), 1);
    assert!(game.world().events.is_set(EVENT_GOT_POTION_SAMPLE));
}

#[test]
fn a_bug_catcher_sees_the_player_walks_up_and_fights() {
    // Bug Catcher 0 stands at (30, 33) facing left and sees four squares.
    let mut game = game(Map::ViridianForest, 25, 33, SpriteFacing::Right, 5, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Right));
    play_until(&mut game, 20_000, |game| game.world().events.is_set(EVENT_BEAT_VIRIDIAN_FOREST_TRAINER_0) && free(game));
    assert_eq!((game.world().location.x, game.world().location.y), (26, 33));
    assert_eq!(game.world().scripts.maps.viridian_forest.cur_script, 0);
    assert_ne!(game.world().money, [0; 3], "the prize");
}

#[test]
fn a_wild_mon_jumps_out_of_the_grass_and_the_walk_goes_on_after() {
    let mut game = game(Map::Route1, 13, 31, SpriteFacing::Up, 11, |_| {});
    play_until(&mut game, 600, free);
    let mut fought = false;
    for step in 0..400 {
        let direction = if step % 2 == 0 { Direction::Up } else { Direction::Down };
        command(&mut game, Command::Step(direction));
        play_until(&mut game, 20_000, |game| {
            fought |= game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_)));
            free(game)
        });
        if fought {
            break;
        }
    }
    assert!(fought, "no encounter in 400 steps of grass");
    assert_eq!(game.world().location.map, Map::Route1);
}

#[test]
fn a_party_that_faints_blacks_out_to_the_last_town_with_half_the_money() {
    let mut game = game(Map::Route1, 13, 31, SpriteFacing::Up, 11, |world| {
        world.party = vec![mon(PokemonSpecies::Magikarp, 2)];
        world.party[0].mon.mon.hp = 1;
        world.money = [0x00, 0x30, 0x01];
        world.location.last_blackout_map = Map::ViridianCity;
    });
    play_until(&mut game, 600, free);
    for step in 0..400 {
        let direction = if step % 2 == 0 { Direction::Up } else { Direction::Down };
        command(&mut game, Command::Step(direction));
        play_until(&mut game, 20_000, |game| free(game));
        if game.world().location.map != Map::Route1 {
            break;
        }
    }
    let world = game.world();
    assert_eq!((world.location.map, world.location.x, world.location.y), (Map::ViridianCity, 23, 26));
    assert_eq!(world.money, [0x00, 0x15, 0x00]);
    assert_eq!(world.party[0].mon.mon.hp, world.party[0].mon.stats[0], "healed");
}

#[test]
fn a_save_in_the_middle_of_oak_s_walk_resumes_in_the_middle_of_it() {
    let mut game = game(Map::PalletTown, 10, 2, SpriteFacing::Up, 3, |world| world.events.clear(EVENT_FOLLOWED_OAK_INTO_LAB));
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 3000, |game| game.world().scripts.maps.pallet_town.cur_script == 3);
    let mut loaded = Game::load(&game.save(), Pacing::Faithful).unwrap();
    for _ in 0..600 {
        let input = || if game.status() == Status::Waiting(Decision::Text) { Input::Command(Command::Advance) } else { Input::None };
        let (a, b) = (input(), if loaded.status() == Status::Waiting(Decision::Text) { Input::Command(Command::Advance) } else { Input::None });
        game.frame(a);
        loaded.frame(b);
    }
    assert_eq!(loaded.save(), game.save());
}

#[test]
fn a_mart_clerk_across_the_counter_opens_the_mart_with_its_stock() {
    let mut game = game(Map::PewterMart, 2, 5, SpriteFacing::Left, 1, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 600, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Pokemart(_))));
    play_until(&mut game, 600, |game| game.status() == Status::Waiting(Decision::BuySellQuit));
    command(&mut game, Command::ChooseOption(2));
    play_until(&mut game, 2000, free);
    assert!(game.ui().cover(1, 14).is_none(), "the box is down and the player free");
}

#[test]
fn the_route_1_clerk_says_something_else_the_second_time() {
    let mut game = game(Map::Route1, 5, 26, SpriteFacing::Up, 1, |world| {
        world.party.clear();
        world.events.set(EVENT_GOT_POTION_SAMPLE);
    });
    play_until(&mut game, 600, free);
    for _ in 0..3000 {
        if let Some(Mode::Overworld(overworld)) = game.modes().last()
            && overworld.in_front_text(game.world()) == 1
            && free(&game)
        {
            break;
        }
        game.frame(Input::None);
    }
    command(&mut game, Command::Interact);
    play_until(&mut game, 2000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::Potion), 0);
}


#[test]
fn the_viridian_clerk_stops_the_player_hands_over_the_parcel_and_later_opens_the_mart() {
    let mut game = game(Map::ViridianMart, 3, 7, SpriteFacing::Up, 2, |world| world.events.clear(EVENT_GOT_OAKS_PARCEL));
    play_until(&mut game, 3000, |game| game.world().events.is_set(EVENT_GOT_OAKS_PARCEL) && free(game));
    let world = game.world();
    assert_eq!(world.bag.quantity_of(ItemId::OaksParcel), 1);
    assert_eq!((world.location.x, world.location.y), (2, 5), "walked left and up to the counter");

    // Delivered, the same clerk is a mart: the map's second text table.
    let mut game = game_after_delivery();
    command(&mut game, Command::Interact);
    play_until(&mut game, 600, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Pokemart(_))));
}

fn game_after_delivery() -> Game {
    let mut game = game(Map::ViridianMart, 2, 5, SpriteFacing::Left, 2, |world| {
        world.events.set(EVENT_GOT_OAKS_PARCEL);
        world.events.set(EVENT_OAK_GOT_PARCEL);
        world.scripts.maps.viridian_mart.cur_script = poke_core::symbols::pokered_map_scripts::SCRIPT_VIRIDIANMART_NOOP;
    });
    play_until(&mut game, 600, free);
    game
}

#[test]
fn a_blackout_fades_the_music_to_silence_before_it_warps() {
    let mut game = game(Map::Route1, 13, 31, SpriteFacing::Up, 11, |world| {
        world.party = vec![mon(PokemonSpecies::Magikarp, 2)];
        world.party[0].mon.mon.hp = 1;
        world.location.last_blackout_map = Map::ViridianCity;
    });
    play_until(&mut game, 600, free);
    let mut faded = false;
    for step in 0..400 {
        command(&mut game, Command::Step(if step % 2 == 0 { Direction::Up } else { Direction::Down }));
        play_until(&mut game, 20_000, |game| {
            faded |= game.audio().fading_out();
            free(game)
        });
        if game.world().location.map != Map::Route1 {
            break;
        }
    }
    assert!(faded, "StopMusic's fade ran");
    assert_eq!(game.world().location.map, Map::ViridianCity);
}

/// `ViridianGymArrowMovement5`, `db PAD_DOWN, 2`, under (16, 10).
#[test]
fn an_arrow_tile_slides_the_player_along_it_until_its_presses_run_out() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_VIRIDIANGYM_DEFAULT;
    let mut game = game(Map::ViridianGym, 16, 10, SpriteFacing::Down, 7, |_| {});
    play_until(&mut game, 3000, |game| game.world().location.y == 12 && free(game));
    assert_eq!(game.world().location.x, 16, "an arrow only ever sends the player one way at a time");
    assert_eq!(game.world().scripts.maps.viridian_gym.cur_script, SCRIPT_VIRIDIANGYM_DEFAULT);
}

/// The guard stops anyone carrying a bicycle on any of the four squares in front of his counter, and
/// the number of squares between them and it is how many steps he walks them up.
#[test]
fn the_cycling_road_s_gate_takes_the_bike_back_and_walks_the_player_to_the_counter() {
    use crate::systems::overworld::location::BIKING;
    let mut game = game(Map::Route16Gate1F, 4, 10, SpriteFacing::Up, 9, |world| {
        world.bag.add(ItemId::Bicycle, 1);
        world.location.always_on_bike = true;
        world.location.walk_bike_surf = BIKING;
    });
    assert!(game.world().location.always_on_bike);
    play_until(&mut game, 8000, |game| {
        (game.world().location.x, game.world().location.y) == (5, 7) && free(game)
    });
    assert!(!game.world().location.always_on_bike, "the gate is the only place the road gives the bike back");
    assert_eq!(game.world().scripts.maps.route16_gate_1f.cur_script, 0);
}

/// `SeafoamIslandsB4FMoveObjectScript`, which `CheckForceBikeOrSurf` arms as the player lands in the
/// water at (4, 14). The list is pressed from its end, so it is one up, three right and three up,
/// the last of them onto the stairs' landing.
#[test]
fn seafoam_s_current_carries_a_surfing_player_round_to_the_stairs_and_puts_them_on_their_feet() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_SEAFOAMISLANDSB4F_MOVE_OBJECT;
    use crate::systems::overworld::location::{SURFING, WALKING};
    let mut game = game(Map::SeafoamIslandsB4F, 4, 14, SpriteFacing::Down, 13, |world| {
        world.location.walk_bike_surf = SURFING;
        world.events.set(EVENT_SEAFOAM4_BOULDER1_DOWN_HOLE);
        world.events.set(EVENT_SEAFOAM4_BOULDER2_DOWN_HOLE);
        world.scripts.maps.seafoam_islands_b4f.cur_script = SCRIPT_SEAFOAMISLANDSB4F_MOVE_OBJECT;
    });
    play_until(&mut game, 8000, |game| {
        (game.world().location.x, game.world().location.y) == (7, 10) && free(game)
    });
    assert_eq!(game.world().location.walk_bike_surf, WALKING, "the press before the last one is the step ashore");
    assert_eq!(game.world().scripts.maps.seafoam_islands_b4f.cur_script, 0);
}

/// Like [`play_until`], with a yes/no answered by the next of `answers` (0 for the first option),
/// the last of them standing for every one after it, and a Pokédex page read and left.
fn play_answering(game: &mut Game, limit: u32, answers: &mut Vec<u8>, mut done: impl FnMut(&Game) -> bool) {
    for _ in 0..limit {
        if done(game) {
            return;
        }
        if game.status() == Status::Waiting(Decision::TwoOption) {
            let answer = answers[0];
            let accepted = matches!(game.frame(Input::Command(Command::ChooseOption(answer))).reply, Some(Reply::Accepted));
            if accepted && answers.len() > 1 {
                answers.remove(0);
            }
            continue;
        }
        let input = match game.status() {
            Status::Waiting(Decision::Text) => Input::Command(Command::Advance),
            Status::Waiting(Decision::PokedexData) => Input::Command(Command::CloseDex),
            _ => Input::None,
        };
        game.frame(input);
    }
    panic!("never done: {:?} on {:?}", game.status(), game.modes().last().map(std::mem::discriminant));
}

/// Oak's Lab from the door to the starter: Oak walks in behind the player, the player is walked up
/// to the table, the speech plays, and the ball the player leaves is the one the rival takes.
#[test]
fn oaks_lab_walks_the_player_to_the_table_and_hands_out_the_starters() {
    let mut game = game(Map::OaksLab, 5, 11, SpriteFacing::Up, 3, |world| {
        world.events.clear(EVENT_FOLLOWED_OAK_INTO_LAB);
        world.events.set(EVENT_OAK_APPEARED_IN_PALLET);
    });
    play_until(&mut game, 6000, |game| game.world().events.is_set(EVENT_OAK_ASKED_TO_CHOOSE_MON) && free(game));
    assert_eq!((game.world().location.x, game.world().location.y), (5, 3), "walked the eight squares up");
    assert!(game.world().events.is_set(EVENT_FOLLOWED_OAK_INTO_LAB));

    // The Charmander ball is the square to the right; the rival is left the one beside it.
    command(&mut game, Command::Face(Direction::Right));
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_answering(&mut game, 20_000, &mut vec![0, 1], |game| game.world().party.len() == 2);
    assert_eq!(game.world().party[1].mon.mon.species, PokemonSpecies::Charmander);
    assert!(game.world().scripts.got_starter);
    play_answering(&mut game, 40_000, &mut vec![1], |game| game.world().events.is_set(EVENT_GOT_STARTER) && free(game));
    assert_eq!(game.world().scripts.rival_starter, PokemonSpecies::Squirtle as u8);
    assert!(game.world().location.is_hidden(poke_core::symbols::pokered_toggles::TOGGLE_STARTER_BALL_1 as u8));
    assert!(game.world().location.is_hidden(poke_core::symbols::pokered_toggles::TOGGLE_STARTER_BALL_2 as u8));
}

/// The battle over, the rival is put back where it took him from, the party is healed and he leaves.
#[test]
fn the_rival_is_healed_over_and_walks_out_of_oaks_lab_once_the_battle_is_done() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_OAKSLAB_NOOP, SCRIPT_OAKSLAB_RIVAL_END_BATTLE};
    let mut game = game(Map::OaksLab, 5, 6, SpriteFacing::Up, 3, |world| {
        world.party[0].mon.mon.hp = 1;
        world.events.set(EVENT_GOT_STARTER);
        world.scripts.maps.oaks_lab.cur_script = SCRIPT_OAKSLAB_RIVAL_END_BATTLE;
    });
    play_until(&mut game, 20_000, |game| game.world().scripts.maps.oaks_lab.cur_script == SCRIPT_OAKSLAB_NOOP);
    let world = game.world();
    assert!(world.events.is_set(EVENT_BATTLED_RIVAL_IN_OAKS_LAB));
    assert_eq!(world.party[0].mon.mon.hp, world.party[0].mon.stats[0], "HealParty");
    assert!(world.location.is_hidden(poke_core::symbols::pokered_toggles::TOGGLE_OAKS_LAB_RIVAL as u8));
}

/// The parcel is what brings the rival back, and the Pokédex is what he is brought back for.
#[test]
fn oak_takes_the_parcel_and_gives_out_the_pokedex_when_the_rival_has_come_back() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_OAKSLAB_NOOP;
    let mut game = game(Map::OaksLab, 5, 3, SpriteFacing::Up, 3, |world| {
        world.bag.add(ItemId::OaksParcel, 1);
        world.events.set(EVENT_FOLLOWED_OAK_INTO_LAB_2);
        world.events.set(EVENT_GOT_STARTER);
        world.events.set(EVENT_BATTLED_RIVAL_IN_OAKS_LAB);
        world.scripts.maps.oaks_lab.cur_script = SCRIPT_OAKSLAB_NOOP;
        show(world, poke_core::symbols::pokered_toggles::TOGGLE_OAKS_LAB_OAK_1);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 40_000, |game| game.world().events.is_set(EVENT_GOT_POKEDEX) && free(game));
    let world = game.world();
    assert_eq!(world.bag.quantity_of(ItemId::OaksParcel), 0, "handed over");
    assert!(world.events.is_set(EVENT_OAK_GOT_PARCEL));
    assert!(world.events.is_set(EVENT_ROUTE22_RIVAL_WANTS_BATTLE), "Route 22 is armed for him");
    assert_eq!(world.scripts.maps.pallet_town.cur_script, poke_core::symbols::pokered_map_scripts::SCRIPT_PALLETTOWN_DAISY);
}

/// The gym's door is shut until every other badge is in, and whoever walks up to it is walked back.
#[test]
fn viridian_s_gym_turns_the_player_away_from_its_door() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_VIRIDIANCITY_DEFAULT;
    let mut game = game(Map::ViridianCity, 32, 9, SpriteFacing::Up, 4, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 6000, |game| {
        game.world().location.y > 8 && free(game)
            && game.world().scripts.maps.viridian_city.cur_script == SCRIPT_VIRIDIANCITY_DEFAULT
    });
    assert_eq!(game.world().location.x, 32, "walked straight back down");
    assert!(!game.world().events.is_set(EVENT_VIRIDIAN_GYM_OPEN));
}

/// The old man's lesson is a battle he fights himself, and his words follow it.
#[test]
fn the_viridian_old_man_shows_the_player_how_to_catch_a_weedle() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_VIRIDIANCITY_DEFAULT,
        SCRIPT_VIRIDIANCITY_OLD_MAN_START_CATCH_TRAINING};
    let mut game = game(Map::ViridianCity, 21, 10, SpriteFacing::Up, 6, |world| {
        world.scripts.maps.viridian_city.cur_script = SCRIPT_VIRIDIANCITY_OLD_MAN_START_CATCH_TRAINING;
    });
    play_until(&mut game, 3000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    play_until(&mut game, 40_000, |game| {
        game.world().scripts.maps.viridian_city.cur_script == SCRIPT_VIRIDIANCITY_DEFAULT && free(game)
    });
}

/// The rival is waiting on the way to the League: he walks up, says his piece and the battle is on.
#[test]
fn the_route_22_rival_walks_up_and_starts_the_battle() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_ROUTE22_RIVAL1_AFTER_BATTLE;
    let mut game = game(Map::Route22, 30, 4, SpriteFacing::Left, 8, |world| {
        world.events.set(EVENT_ROUTE22_RIVAL_WANTS_BATTLE);
        world.events.set(EVENT_1ST_ROUTE22_RIVAL_BATTLE);
        world.scripts.rival_starter = PokemonSpecies::Squirtle as u8;
        show(world, poke_core::symbols::pokered_toggles::TOGGLE_ROUTE_22_RIVAL_1);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Left));
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    assert_eq!(game.world().scripts.maps.route22.cur_script, SCRIPT_ROUTE22_RIVAL1_AFTER_BATTLE);
    assert_eq!((game.world().location.x, game.world().location.y), (29, 4), "he stops the player where he stands");
}

/// Beaten, he walks off south and takes the route's script back to its first entry with him.
#[test]
fn the_route_22_rival_leaves_the_route_behind_him_once_he_is_beaten() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROUTE22_DEFAULT, SCRIPT_ROUTE22_RIVAL1_AFTER_BATTLE};
    let mut game = game(Map::Route22, 29, 4, SpriteFacing::Left, 8, |world| {
        world.events.set(EVENT_ROUTE22_RIVAL_WANTS_BATTLE);
        world.events.set(EVENT_1ST_ROUTE22_RIVAL_BATTLE);
        world.scripts.maps.route22.cur_script = SCRIPT_ROUTE22_RIVAL1_AFTER_BATTLE;
        world.scripts.maps.route22.coord_index = 1;
        show(world, poke_core::symbols::pokered_toggles::TOGGLE_ROUTE_22_RIVAL_1);
    });
    play_until(&mut game, 40_000, |game| {
        game.world().scripts.maps.route22.cur_script == SCRIPT_ROUTE22_DEFAULT && free(game)
    });
    let world = game.world();
    assert!(world.events.is_set(EVENT_BEAT_ROUTE22_RIVAL_1ST_BATTLE));
    assert!(!world.events.is_set(EVENT_ROUTE22_RIVAL_WANTS_BATTLE), "he is done with the route");
    assert!(world.location.is_hidden(poke_core::symbols::pokered_toggles::TOGGLE_ROUTE_22_RIVAL_1 as u8));
}

/// Giovanni's badge is what opens Route 22 for the second rival battle.
#[test]
fn giovanni_hands_over_the_earth_badge_tm27_and_the_second_route_22_battle() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_VIRIDIANGYM_DEFAULT, SCRIPT_VIRIDIANGYM_GIOVANNI_POST_BATTLE};
    let mut game = game(Map::ViridianGym, 16, 12, SpriteFacing::Down, 7, |world| {
        world.scripts.maps.viridian_gym.cur_script = SCRIPT_VIRIDIANGYM_GIOVANNI_POST_BATTLE;
    });
    play_until(&mut game, 20_000, |game| {
        game.world().scripts.maps.viridian_gym.cur_script == SCRIPT_VIRIDIANGYM_DEFAULT && free(game)
    });
    let world = game.world();
    assert_eq!(world.badges & 0x80, 0x80, "the Earth Badge");
    assert_eq!(world.bag.quantity_of(ItemId::Tm27Fissure), 1);
    assert!(world.events.is_set(EVENT_BEAT_VIRIDIAN_GYM_GIOVANNI) && world.events.is_set(EVENT_GOT_TM27));
    assert!(world.events.is_set(EVENT_2ND_ROUTE22_RIVAL_BATTLE) && world.events.is_set(EVENT_ROUTE22_RIVAL_WANTS_BATTLE));
    assert!(world.events.is_set(EVENT_BEAT_VIRIDIAN_GYM_TRAINER_7), "and nobody is left to stop the way out");
}

/// The gym guide takes over the pad: the player is walked across town and he walks off and back.
#[test]
fn pewter_s_gym_guide_walks_the_player_to_the_gym_and_puts_himself_back() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_PEWTERCITY_DEFAULT, SCRIPT_PEWTERCITY_YOUNGSTER_SHOWS_PLAYER_GYM};
    let mut game = game(Map::PewterCity, 35, 17, SpriteFacing::Down, 5, |world| {
        world.events.clear(EVENT_BEAT_BROCK);
    });
    play_until(&mut game, 20_000, |game| {
        game.world().scripts.maps.pewter_city.cur_script == SCRIPT_PEWTERCITY_YOUNGSTER_SHOWS_PLAYER_GYM
    });
    play_until(&mut game, 40_000, |game| {
        game.world().scripts.maps.pewter_city.cur_script == SCRIPT_PEWTERCITY_DEFAULT && free(game)
    });
    assert_ne!((game.world().location.x, game.world().location.y), (35, 17), "led away from the way out");
    assert!(!game.world().location.is_hidden(poke_core::symbols::pokered_toggles::TOGGLE_GYM_GUY as u8),
        "he is back where he started");
}



/// The Super Nerd guarding the fossils speaks up the moment the player steps in front of him, and
/// the battle is the only way past.
#[test]
fn mt_moon_s_super_nerd_speaks_up_and_fights_for_both_fossils() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_MTMOONB2F_DEFEATED_SUPER_NERD;
    let mut game = game(Map::MtMoonB2F, 13, 9, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    assert_eq!((game.world().location.x, game.world().location.y), (13, 8), "he stops the player in front of him");
    assert_eq!(game.world().scripts.maps.mt_moon_b2f.cur_script, SCRIPT_MTMOONB2F_DEFEATED_SUPER_NERD);
}

/// Beaten, he gives the fossils up and the floor goes back to being a floor.
#[test]
fn the_super_nerd_beaten_gives_the_fossils_up() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_MTMOONB2F_DEFAULT, SCRIPT_MTMOONB2F_DEFEATED_SUPER_NERD};
    let mut game = game(Map::MtMoonB2F, 13, 8, SpriteFacing::Left, 5, |world| {
        world.scripts.maps.mt_moon_b2f.cur_script = SCRIPT_MTMOONB2F_DEFEATED_SUPER_NERD;
    });
    play_until(&mut game, 20_000, |game| {
        game.world().scripts.maps.mt_moon_b2f.cur_script == SCRIPT_MTMOONB2F_DEFAULT && free(game)
    });
    assert!(game.world().events.is_set(EVENT_BEAT_MT_MOON_EXIT_SUPER_NERD));
}

/// Beaten, he offers a choice of two: the one the player leaves is the one he steps over and takes.
#[test]
fn the_fossil_the_player_leaves_is_the_one_the_super_nerd_takes() {
    use poke_core::symbols::pokered_toggles::{TOGGLE_MT_MOON_B2F_FOSSIL_1, TOGGLE_MT_MOON_B2F_FOSSIL_2};
    let mut game = game(Map::MtMoonB2F, 12, 7, SpriteFacing::Up, 5, |world| {
        world.events.set(EVENT_BEAT_MT_MOON_EXIT_SUPER_NERD);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_answering(&mut game, 40_000, &mut vec![0], |game| {
        game.world().location.is_hidden(TOGGLE_MT_MOON_B2F_FOSSIL_2 as u8) && free(game)
    });
    let world = game.world();
    assert_eq!(world.bag.quantity_of(ItemId::DomeFossil), 1);
    assert!(world.events.is_set(EVENT_GOT_DOME_FOSSIL));
    assert!(world.location.is_hidden(TOGGLE_MT_MOON_B2F_FOSSIL_1 as u8), "the one the player took");
    assert_eq!(world.scripts.maps.mt_moon_b2f.cur_script, 0);
}

/// Mt. Moon's first floor is a plain trainer map: the Hiker at (5, 6) sees two squares down.
#[test]
fn a_mt_moon_hiker_sees_the_player_and_fights() {
    let mut game = game(Map::MtMoon1F, 5, 9, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 40_000, |game| {
        game.world().events.is_set(EVENT_BEAT_MT_MOON_1_TRAINER_0) && free(game)
    });
    assert_eq!(game.world().scripts.maps.mt_moon_1f.cur_script, 0);
}

/// The Nugget Bridge pays out to whoever steps onto the square at the top of it, and the prize is
/// the offer of a job that comes with it.
#[test]
fn the_nugget_bridge_pays_out_and_the_man_paying_turns_out_to_be_a_rocket() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_ROUTE24_DEFAULT;
    let mut game = game(Map::Route24, 10, 16, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    assert_eq!(game.world().bag.quantity_of(ItemId::Nugget), 1, "paid before the fight");
    assert!(game.world().events.is_set(EVENT_GOT_NUGGET));
    play_until(&mut game, 60_000, |game| game.world().events.is_set(EVENT_BEAT_ROUTE24_ROCKET) && free(game));
    assert_eq!(game.world().scripts.maps.route24.cur_script, SCRIPT_ROUTE24_DEFAULT);
}

/// The rival drops onto the bridge north of Cerulean and the battle is on where the player stands.
#[test]
fn cerulean_s_rival_drops_onto_the_bridge_and_starts_the_battle() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_CERULEANCITY_RIVAL_DEFEATED;
    let mut game = game(Map::CeruleanCity, 20, 7, SpriteFacing::Up, 8, |world| {
        one_shot(world);
        world.scripts.rival_starter = PokemonSpecies::Squirtle as u8;
        world.events.set(EVENT_BEAT_CERULEAN_ROCKET_THIEF);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    assert_eq!((game.world().location.x, game.world().location.y), (20, 6));
    assert_eq!(game.world().scripts.maps.cerulean_city.cur_script, SCRIPT_CERULEANCITY_RIVAL_DEFEATED);
}

/// Beaten, he walks round the player and off south, and the map has no rival on it again.
#[test]
fn the_cerulean_rival_walks_off_south_once_he_is_beaten() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_CERULEANCITY_DEFAULT, SCRIPT_CERULEANCITY_RIVAL_DEFEATED};
    use poke_core::symbols::pokered_toggles::TOGGLE_CERULEAN_RIVAL;
    let mut game = game(Map::CeruleanCity, 20, 6, SpriteFacing::Up, 8, |world| {
        world.events.set(EVENT_BEAT_CERULEAN_ROCKET_THIEF);
        world.scripts.maps.cerulean_city.cur_script = SCRIPT_CERULEANCITY_RIVAL_DEFEATED;
        show(world, TOGGLE_CERULEAN_RIVAL);
    });
    play_until(&mut game, 40_000, |game| {
        game.world().scripts.maps.cerulean_city.cur_script == SCRIPT_CERULEANCITY_DEFAULT && free(game)
    });
    assert!(game.world().events.is_set(EVENT_BEAT_CERULEAN_RIVAL));
    assert!(game.world().location.is_hidden(TOGGLE_CERULEAN_RIVAL as u8), "he is off the map again");
}

/// The thief who robbed the house hands TM28 back, and the hole he came out of is a guard again.
#[test]
fn the_cerulean_thief_hands_back_tm28_and_a_guard_takes_his_place() {
    use poke_core::symbols::pokered_toggles::{TOGGLE_CERULEAN_GUARD_1, TOGGLE_CERULEAN_GUARD_2,
        TOGGLE_CERULEAN_ROCKET};
    let mut game = game(Map::CeruleanCity, 30, 7, SpriteFacing::Down, 8, |world| {
        world.events.set(EVENT_BEAT_CERULEAN_ROCKET_THIEF);
        show(world, TOGGLE_CERULEAN_ROCKET);
        show(world, TOGGLE_CERULEAN_GUARD_2);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 40_000, |game| {
        game.world().location.is_hidden(TOGGLE_CERULEAN_ROCKET as u8) && free(game)
    });
    let world = game.world();
    assert_eq!(world.bag.quantity_of(ItemId::Tm28Dig), 1);
    assert!(!world.location.is_hidden(TOGGLE_CERULEAN_GUARD_1 as u8));
    assert!(world.location.is_hidden(TOGGLE_CERULEAN_GUARD_2 as u8));
}

/// Bill as a Pokémon walks himself into the machine, and a no is only asked again.
#[test]
fn bill_walks_himself_into_the_cell_separator_whatever_the_player_answers() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_BILLSHOUSE_BILL_EXITS_MACHINE;
    use poke_core::symbols::pokered_toggles::TOGGLE_BILL_POKEMON;
    let mut game = game(Map::BillsHouse, 6, 6, SpriteFacing::Up, 3, |world| {
        show(world, TOGGLE_BILL_POKEMON);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_answering(&mut game, 40_000, &mut vec![1], |game| {
        game.world().scripts.maps.bills_house.cur_script == SCRIPT_BILLSHOUSE_BILL_EXITS_MACHINE
    });
    let world = game.world();
    assert!(world.location.is_hidden(TOGGLE_BILL_POKEMON as u8), "he is inside the machine");
    assert!(world.events.is_set(EVENT_BILL_SAID_USE_CELL_SEPARATOR));
}

/// The PC is what puts him back together: the script only waits for the event and walks him out.
#[test]
fn bill_steps_out_of_the_machine_once_the_cell_separator_has_run() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_BILLSHOUSE_BILL_EXITS_MACHINE, SCRIPT_BILLSHOUSE_DEFAULT};
    use poke_core::symbols::pokered_toggles::TOGGLE_BILL_1;
    let mut game = game(Map::BillsHouse, 6, 6, SpriteFacing::Up, 3, |world| {
        world.events.set(EVENT_BILL_SAID_USE_CELL_SEPARATOR);
        world.events.set(EVENT_USED_CELL_SEPARATOR_ON_BILL);
        world.scripts.maps.bills_house.cur_script = SCRIPT_BILLSHOUSE_BILL_EXITS_MACHINE;
    });
    play_until(&mut game, 40_000, |game| {
        game.world().scripts.maps.bills_house.cur_script == SCRIPT_BILLSHOUSE_DEFAULT && free(game)
    });
    let world = game.world();
    assert!(!world.location.is_hidden(TOGGLE_BILL_1 as u8));
    assert!(world.events.is_set(EVENT_MET_BILL) && world.events.is_set(EVENT_MET_BILL_2));
}

/// Bill pays in the one ticket he has, and the guard who blocks Vermilion's dock is swapped for the
/// one who does not.
#[test]
fn bill_hands_over_the_ss_ticket_and_opens_the_dock() {
    use poke_core::symbols::pokered_toggles::{TOGGLE_BILL_1, TOGGLE_CERULEAN_GUARD_1, TOGGLE_CERULEAN_GUARD_2};
    let mut game = game(Map::BillsHouse, 4, 5, SpriteFacing::Up, 3, |world| {
        world.events.set(EVENT_MET_BILL);
        world.events.set(EVENT_MET_BILL_2);
        show(world, TOGGLE_BILL_1);
        show(world, TOGGLE_CERULEAN_GUARD_2);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 40_000, |game| game.world().events.is_set(EVENT_GOT_SS_TICKET) && free(game));
    let world = game.world();
    assert_eq!(world.bag.quantity_of(ItemId::SSTicket), 1);
    assert!(!world.location.is_hidden(TOGGLE_CERULEAN_GUARD_1 as u8));
    assert!(world.location.is_hidden(TOGGLE_CERULEAN_GUARD_2 as u8));
}

/// Route 25 carries Bill's own sprites: the pass after it loads puts on the map what the player did
/// in his house a moment ago.
#[test]
fn route_25_swaps_bills_sprites_over_as_the_player_leaves_his_house() {
    use poke_core::symbols::pokered_toggles::{TOGGLE_BILL_1, TOGGLE_BILL_2, TOGGLE_NUGGET_BRIDGE_GUY};
    let mut game = game(Map::BillsHouse, 2, 6, SpriteFacing::Down, 4, |world| {
        world.location.last_map = Map::Route25;
        world.events.set(EVENT_MET_BILL_2);
        world.events.set(EVENT_GOT_SS_TICKET);
        show(world, TOGGLE_BILL_1);
        show(world, TOGGLE_NUGGET_BRIDGE_GUY);
    });
    play_until(&mut game, 600, free);
    // The first step is onto the doormat and the second through it.
    command(&mut game, Command::Step(Direction::Down));
    play_until(&mut game, 8000, free);
    command(&mut game, Command::Step(Direction::Down));
    play_until(&mut game, 20_000, |game| {
        game.world().location.map == Map::Route25 && game.world().events.is_set(EVENT_LEFT_BILLS_HOUSE_AFTER_HELPING)
    });
    let world = game.world();
    assert!(world.location.is_hidden(TOGGLE_NUGGET_BRIDGE_GUY as u8));
    assert!(world.location.is_hidden(TOGGLE_BILL_1 as u8));
    assert!(!world.location.is_hidden(TOGGLE_BILL_2 as u8), "Bill is out on the route now");
}

/// Route 4's one trainer stands on the ledge above Cerulean and sees three squares right.
#[test]
fn the_route_4_lass_sees_the_player_along_the_ledge() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_ROUTE4_END_BATTLE;
    let mut game = game(Map::Route4, 65, 3, SpriteFacing::Left, 5, one_shot);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    assert_eq!(game.world().scripts.maps.route4.cur_script, SCRIPT_ROUTE4_END_BATTLE, "the battle is the map's");
}


/// Brock's badge takes the guide off the map and puts the rival back on Route 22 for later, since
/// the first battle there is only offered to a player who has not been to Pewter yet.
#[test]
fn brocks_boulder_badge_takes_the_gym_guide_and_the_route_22_rival_away() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_PEWTERGYM_BROCK_POST_BATTLE, SCRIPT_PEWTERGYM_DEFAULT};
    use poke_core::symbols::pokered_toggles::{TOGGLE_GYM_GUY, TOGGLE_ROUTE_22_RIVAL_1};
    let mut game = game(Map::PewterGym, 4, 2, SpriteFacing::Up, 5, |world| {
        world.events.set(EVENT_1ST_ROUTE22_RIVAL_BATTLE);
        world.events.set(EVENT_ROUTE22_RIVAL_WANTS_BATTLE);
        world.scripts.maps.pewter_gym.cur_script = SCRIPT_PEWTERGYM_BROCK_POST_BATTLE;
    });
    play_until(&mut game, 20_000, |game| {
        game.world().scripts.maps.pewter_gym.cur_script == SCRIPT_PEWTERGYM_DEFAULT && free(game)
    });
    let world = game.world();
    assert_eq!(world.badges & 0x01, 0x01, "the Boulder Badge");
    assert_eq!(world.bag.quantity_of(ItemId::Tm34Bide), 1);
    assert!(world.events.is_set(EVENT_BEAT_BROCK) && world.events.is_set(EVENT_GOT_TM34));
    assert!(world.events.is_set(EVENT_BEAT_PEWTER_GYM_TRAINER_0), "nobody is left to stop the way out");
    assert!(!world.events.is_set(EVENT_1ST_ROUTE22_RIVAL_BATTLE));
    assert!(!world.events.is_set(EVENT_ROUTE22_RIVAL_WANTS_BATTLE));
    assert!(world.location.is_hidden(TOGGLE_GYM_GUY as u8));
    assert!(world.location.is_hidden(TOGGLE_ROUTE_22_RIVAL_1 as u8));
}

/// A gym leader is fought from his own text rather than by being walked up to, and the script he
/// arms is what hands the badge over afterwards.
#[test]
fn talking_to_brock_starts_the_battle_and_arms_the_badge_script() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_PEWTERGYM_BROCK_POST_BATTLE;
    let mut game = game(Map::PewterGym, 4, 2, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    assert_eq!(game.world().scripts.maps.pewter_gym.cur_script, SCRIPT_PEWTERGYM_BROCK_POST_BATTLE);
}

/// The guide's offer is a yes/no that changes nothing but which line he says first.
#[test]
fn the_pewter_gym_guide_gives_his_advice_either_way() {
    let mut game = game(Map::PewterGym, 7, 11, SpriteFacing::Up, 5, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_answering(&mut game, 20_000, &mut vec![1], free);
    assert_eq!(game.world().badges, 0, "he hands out nothing");
}

#[test]
fn mistys_cascade_badge_comes_with_tm11_and_marks_her_two_trainers_beaten() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_CERULEANGYM_DEFAULT, SCRIPT_CERULEANGYM_MISTY_POST_BATTLE};
    let mut game = game(Map::CeruleanGym, 4, 3, SpriteFacing::Up, 5, |world| {
        world.scripts.maps.cerulean_gym.cur_script = SCRIPT_CERULEANGYM_MISTY_POST_BATTLE;
    });
    play_until(&mut game, 20_000, |game| {
        game.world().scripts.maps.cerulean_gym.cur_script == SCRIPT_CERULEANGYM_DEFAULT && free(game)
    });
    let world = game.world();
    assert_eq!(world.badges & 0x02, 0x02, "the Cascade Badge");
    assert_eq!(world.bag.quantity_of(ItemId::Tm11Bubblebeam), 1);
    assert!(world.events.is_set(EVENT_BEAT_MISTY) && world.events.is_set(EVENT_GOT_TM11));
    assert!(world.events.is_set(EVENT_BEAT_CERULEAN_GYM_TRAINER_0));
    assert!(world.events.is_set(EVENT_BEAT_CERULEAN_GYM_TRAINER_1));
}

#[test]
fn lt_surges_thunder_badge_comes_with_tm24_and_marks_his_three_trainers_beaten() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_VERMILIONGYM_DEFAULT,
        SCRIPT_VERMILIONGYM_LT_SURGE_AFTER_BATTLE};
    let mut game = game(Map::VermilionGym, 5, 2, SpriteFacing::Up, 5, |world| {
        world.events.set(EVENT_2ND_LOCK_OPENED);
        world.scripts.maps.vermilion_gym.cur_script = SCRIPT_VERMILIONGYM_LT_SURGE_AFTER_BATTLE;
    });
    play_until(&mut game, 20_000, |game| {
        game.world().scripts.maps.vermilion_gym.cur_script == SCRIPT_VERMILIONGYM_DEFAULT && free(game)
    });
    let world = game.world();
    assert_eq!(world.badges & 0x04, 0x04, "the Thunder Badge");
    assert_eq!(world.bag.quantity_of(ItemId::Tm24Thunderbolt), 1);
    assert!(world.events.is_set(EVENT_BEAT_LT_SURGE) && world.events.is_set(EVENT_GOT_TM24));
    assert!(world.events.is_set(EVENT_BEAT_VERMILION_GYM_TRAINER_2));
}

/// Walking into a gym is what loads its leader's and its city's names, which is all its statue
/// prints, and what shuts the Vermilion doors behind the two rubbish-bin switches.
fn walk_into_the_vermilion_gym(second_lock: bool) -> Game {
    let mut game = game(Map::VermilionCity, 12, 18, SpriteFacing::Down, 5, |world| {
        // Nobody in the gym is left to look up as the player walks past.
        world.events.set(EVENT_BEAT_VERMILION_GYM_TRAINER_0);
        world.events.set(EVENT_BEAT_VERMILION_GYM_TRAINER_1);
        world.events.set(EVENT_BEAT_VERMILION_GYM_TRAINER_2);
        if second_lock {
            world.events.set(EVENT_1ST_LOCK_OPENED);
            world.events.set(EVENT_2ND_LOCK_OPENED);
        }
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Down));
    play_until(&mut game, 20_000, |game| game.world().location.map == Map::VermilionGym && free(game));
    game
}

#[test]
fn a_gym_loads_its_leader_and_city_name_as_it_is_walked_into() {
    use poke_core::text_script::TextBuffer;
    let game = walk_into_the_vermilion_gym(false);
    assert_eq!(game.world().text.string(TextBuffer::GymCityName), encode("VERMILION CITY").unwrap());
    assert_eq!(game.world().text.string(TextBuffer::GymLeaderName), encode("LT.SURGE").unwrap());
}

/// The gym's second switch swaps the double door behind the bins for floor, on the pass after it.
#[test]
fn the_vermilion_gym_s_doors_are_shut_until_the_second_switch_is_found() {
    /// `ReplaceTileBlock`'s block (2, 2), past the map view's three blocks of border.
    fn door_block(game: &Game) -> u8 {
        let map = &game.screen().map;
        map.blocks[map.blocks_wide * 5 + 5]
    }
    assert_eq!(door_block(&walk_into_the_vermilion_gym(false)), 0x24, "the double door");
    assert_eq!(door_block(&walk_into_the_vermilion_gym(true)), 0x05, "clear floor");
}

/// The sailor on the pier checks for a ticket by standing in the way: whoever steps onto his square
/// without one is walked straight back off it.
#[test]
fn vermilion_s_sailor_turns_a_ticketless_player_back_off_the_pier() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_VERMILIONCITY_DEFAULT;
    let step_onto_the_pier = |ticket: bool| {
        let mut game = game(Map::VermilionCity, 18, 29, SpriteFacing::Down, 5, |world| {
            if ticket {
                world.bag.add(ItemId::SSTicket, 1);
            }
        });
        play_until(&mut game, 600, free);
        command(&mut game, Command::Step(Direction::Down));
        play_until(&mut game, 20_000, |game| {
            free(game) && game.world().scripts.maps.vermilion_city.cur_script == SCRIPT_VERMILIONCITY_DEFAULT
        });
        (game.world().location.x, game.world().location.y)
    };
    assert_eq!(step_onto_the_pier(false), (18, 29), "walked back up");
    assert_eq!(step_onto_the_pier(true), (18, 30), "the ticket is the way past");
}

/// The Machop's line is interrupted by its own cry, which the rest of the line waits out.
#[test]
fn the_vermilion_machop_cries_in_the_middle_of_being_scolded() {
    let mut game = game(Map::VermilionCity, 28, 9, SpriteFacing::Right, 5, |_| {});
    play_until(&mut game, 600, free);
    // It paces up and down its column; wait for it to come back level with the player.
    for _ in 0..6000 {
        if let Some(Mode::Overworld(overworld)) = game.modes().last()
            && overworld.in_front_text(game.world()) == 5
            && free(&game)
        {
            break;
        }
        game.frame(Input::None);
    }
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, free);
}

/// The rival is waiting outside the captain's cabin: he walks down the corridor, fights, and the
/// corridor is empty behind him.
#[test]
fn the_ss_anne_rival_comes_down_the_corridor_and_starts_the_battle() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_SSANNE2F_RIVAL_AFTER_BATTLE;
    let mut game = game(Map::SSAnne2F, 36, 9, SpriteFacing::Up, 8, |world| {
        one_shot(world);
        world.scripts.rival_starter = PokemonSpecies::Squirtle as u8;
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 40_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    assert_eq!((game.world().location.x, game.world().location.y), (36, 8));
    assert_eq!(game.world().scripts.maps.ss_anne_2f.cur_script, SCRIPT_SSANNE2F_RIVAL_AFTER_BATTLE);
}

#[test]
fn the_ss_anne_rival_walks_off_down_the_corridor_once_he_is_beaten() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_SSANNE2F_NOOP, SCRIPT_SSANNE2F_RIVAL_AFTER_BATTLE};
    use poke_core::symbols::pokered_toggles::TOGGLE_SS_ANNE_2F_RIVAL;
    let mut game = game(Map::SSAnne2F, 36, 8, SpriteFacing::Up, 8, |world| {
        world.scripts.maps.ss_anne_2f.cur_script = SCRIPT_SSANNE2F_RIVAL_AFTER_BATTLE;
        show(world, TOGGLE_SS_ANNE_2F_RIVAL);
    });
    play_until(&mut game, 40_000, |game| {
        game.world().scripts.maps.ss_anne_2f.cur_script == SCRIPT_SSANNE2F_NOOP && free(game)
    });
    assert!(game.world().location.is_hidden(TOGGLE_SS_ANNE_2F_RIVAL as u8), "he is off the ship");
}

/// The captain's back is rubbed once, and what it is worth is HM01.
#[test]
fn the_seasick_captain_hands_over_hm01_for_a_rub_of_his_back() {
    let mut game = game(Map::SSAnneCaptainsRoom, 4, 3, SpriteFacing::Up, 5, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 40_000, |game| game.world().events.is_set(EVENT_GOT_HM01) && free(game));
    let world = game.world();
    assert_eq!(world.bag.quantity_of(ItemId::Hm01Cut), 1);
    assert!(world.events.is_set(EVENT_RUBBED_CAPTAINS_BACK));

    // Spoken to again he only says he is better, and hands out nothing.
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::Hm01Cut), 1);
}

/// The Saffron gate guard is thirsty, and anyone with nothing to give him is walked back off the
/// square in front of his counter. One drink opens all four gates.
#[test]
fn the_saffron_gate_guard_walks_a_thirsty_player_back_and_a_drink_opens_the_way() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_ROUTE5GATE_DEFAULT;
    let step_south = |drink: bool| {
        let mut game = game(Map::Route5Gate, 3, 2, SpriteFacing::Down, 5, |world| {
            if drink {
                world.bag.add(ItemId::FreshWater, 1);
            }
        });
        play_until(&mut game, 600, free);
        command(&mut game, Command::Step(Direction::Down));
        play_until(&mut game, 20_000, |game| {
            free(game) && game.world().scripts.maps.route5_gate.cur_script == SCRIPT_ROUTE5GATE_DEFAULT
        });
        game
    };
    let turned_back = step_south(false);
    assert_eq!((turned_back.world().location.x, turned_back.world().location.y), (3, 2), "walked back up");
    assert!(!turned_back.world().scripts.gave_saffron_guards_drink);

    let paid = step_south(true);
    assert_eq!((paid.world().location.x, paid.world().location.y), (3, 3));
    assert!(paid.world().scripts.gave_saffron_guards_drink);
    assert_eq!(paid.world().bag.quantity_of(ItemId::FreshWater), 0, "he drinks it");
}

/// The Route 6 gate asks for the same drink walking the other way, and turns the player back down.
#[test]
fn the_route_6_gate_turns_a_thirsty_player_back_south() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_ROUTE6GATE_DEFAULT;
    let mut game = game(Map::Route6Gate, 3, 3, SpriteFacing::Up, 5, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 20_000, |game| {
        free(game) && game.world().scripts.maps.route6_gate.cur_script == SCRIPT_ROUTE6GATE_DEFAULT
    });
    assert_eq!((game.world().location.x, game.world().location.y), (3, 3), "walked back down");
}

/// Every Saffron gate's text table points at the one `SaffronGateGuardText`, so talking to the guard
/// in any of them presses UP and arms the Route 5 gate's script, wherever the player is standing.
#[test]
fn talking_to_the_route_6_gate_guard_runs_the_route_5_gate_s_code() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_ROUTE5GATE_PLAYER_MOVING;
    let mut game = game(Map::Route6Gate, 5, 2, SpriteFacing::Right, 5, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| {
        game.world().scripts.maps.route5_gate.cur_script == SCRIPT_ROUTE5GATE_PLAYER_MOVING
    });
    assert_eq!(game.world().scripts.maps.route6_gate.cur_script, 0, "its own script was never armed");
}

/// Route 6 is a plain trainer map: its table is the one every trainer map runs.
#[test]
fn a_route_6_trainer_talked_to_fights_on_the_spot() {
    let mut game = game(Map::Route6, 10, 22, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
}

/// Route 11's first Gambler looks three squares down the path.
#[test]
fn a_route_11_gambler_sees_the_player_coming_up_the_path() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_ROUTE11_END_BATTLE;
    let mut game = game(Map::Route11, 10, 18, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 40_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    assert_eq!(game.world().scripts.maps.route11.cur_script, SCRIPT_ROUTE11_END_BATTLE);
}

// ---- Lavender Town, the tower, and the roads between ----

/// The little girl's question is the only code in the whole town, and a no gets its own answer.
#[test]
fn the_lavender_little_girl_takes_either_answer_about_ghosts() {
    for answer in [0, 1] {
        let mut game = game(Map::LavenderTown, 15, 10, SpriteFacing::Up, 6, |_| {});
        play_until(&mut game, 600, free);
        // She wanders her own square, so wait until she is the one being faced.
        for _ in 0..3000 {
            if let Some(Mode::Overworld(overworld)) = game.modes().last()
                && overworld.in_front_text(game.world()) == 1
                && free(&game)
            {
                break;
            }
            game.frame(Input::None);
        }
        command(&mut game, Command::Interact);
        play_answering(&mut game, 4000, &mut vec![answer], free);
    }
}

/// Mr Fuji hands the Poké Flute over once and asks after it every time after that.
#[test]
fn mr_fuji_gives_the_poke_flute_once_he_is_home() {
    use poke_core::symbols::pokered_toggles::TOGGLE_MR_FUJIS_HOUSE_MR_FUJI;
    let mut game = game(Map::MrFujisHouse, 3, 2, SpriteFacing::Up, 6, |world| {
        world.events.set(EVENT_RESCUED_MR_FUJI);
        show(world, TOGGLE_MR_FUJIS_HOUSE_MR_FUJI);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, |game| game.world().events.is_set(EVENT_GOT_POKE_FLUTE) && free(game));
    assert_eq!(game.world().bag.quantity_of(ItemId::PokeFlute), 1);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::PokeFlute), 1, "he has only the one");
}

/// The rater renames a mon the player caught himself, and refuses one caught by anybody else.
#[test]
fn the_name_rater_renames_only_the_players_own_mon() {
    let name = encode("FRED").unwrap();
    let mut game = game(Map::NameRatersHouse, 6, 3, SpriteFacing::Left, 6, |world| world.player_id = 1);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 4000, |game| game.status() == Status::Waiting(Decision::TwoOption));
    command(&mut game, Command::ChooseOption(0));
    play_until(&mut game, 4000, |game| game.status() == Status::Waiting(Decision::PartyMenu));
    command(&mut game, Command::ChooseOption(0));
    play_until(&mut game, 4000, |game| game.status() == Status::Waiting(Decision::TwoOption));
    command(&mut game, Command::ChooseOption(0));
    play_until(&mut game, 8000, |game| game.status() == Status::Waiting(Decision::NamingScreen));
    command(&mut game, Command::EnterName(name.clone()));
    play_until(&mut game, 20_000, free);
    assert_eq!(game.world().party[0].nick, name);

}

/// `NameRatersHouseCheckMonOTScript`: the OT name and the OT id both have to be the player's, so a
/// traded mon is praised and left alone.
#[test]
fn the_name_rater_refuses_a_traded_mon() {
    let mut game = game(Map::NameRatersHouse, 6, 3, SpriteFacing::Left, 6, |world| {
        world.player_id = 1;
        world.party[0].ot = encode("BLUE").unwrap();
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 4000, |game| game.status() == Status::Waiting(Decision::TwoOption));
    command(&mut game, Command::ChooseOption(0));
    play_until(&mut game, 4000, |game| game.status() == Status::Waiting(Decision::PartyMenu));
    command(&mut game, Command::ChooseOption(0));
    play_until(&mut game, 8000, free);
    assert_eq!(game.world().party[0].nick, encode("MON").unwrap(), "a traded mon keeps its name");
}

/// The rival is waiting on the second floor and starts the battle from either square beside him.
#[test]
fn the_tower_rival_starts_his_battle_from_the_square_east_of_him() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_POKEMONTOWER2F_DEFEATED_RIVAL;
    let mut game = game(Map::PokemonTower2F, 15, 5, SpriteFacing::Left, 6, |world| {
        world.scripts.rival_starter = PokemonSpecies::Squirtle as u8;
        one_shot(world);
    });
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    let world = game.world();
    assert_eq!(world.scripts.maps.pokemon_tower_2f.cur_script, SCRIPT_POKEMONTOWER2F_DEFEATED_RIVAL);
    assert!(world.events.is_set(EVENT_POKEMON_TOWER_RIVAL_ON_LEFT), "he is west of the player here");
}

/// Beaten, he walks round the player and out, and takes the floor's script back to its first entry.
#[test]
fn the_tower_rival_walks_out_once_he_is_beaten() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_POKEMONTOWER2F_DEFAULT, SCRIPT_POKEMONTOWER2F_DEFEATED_RIVAL};
    use poke_core::symbols::pokered_toggles::TOGGLE_POKEMON_TOWER_2F_RIVAL;
    let mut game = game(Map::PokemonTower2F, 15, 5, SpriteFacing::Left, 6, |world| {
        world.events.set(EVENT_POKEMON_TOWER_RIVAL_ON_LEFT);
        world.scripts.maps.pokemon_tower_2f.cur_script = SCRIPT_POKEMONTOWER2F_DEFEATED_RIVAL;
    });
    play_until(&mut game, 40_000, |game| {
        game.world().scripts.maps.pokemon_tower_2f.cur_script == SCRIPT_POKEMONTOWER2F_DEFAULT && free(game)
    });
    let world = game.world();
    assert!(world.events.is_set(EVENT_BEAT_POKEMON_TOWER_RIVAL));
    assert!(world.location.is_hidden(TOGGLE_POKEMON_TOWER_2F_RIVAL as u8));
}

/// The purified zone heals the party where it stands and stops the floor's wild battles while the
/// player is on it; stepping off arms it again.
#[test]
fn the_purified_zone_heals_the_party_and_arms_itself_again_when_it_is_left() {
    let mut game = game(Map::PokemonTower5F, 10, 8, SpriteFacing::Down, 6, |world| {
        world.party[0].mon.mon.hp = 1;
    });
    play_until(&mut game, 20_000, |game| game.world().events.is_set(EVENT_IN_PURIFIED_ZONE) && free(game));
    let full = game.world().party[0].mon.stats[0];
    assert_eq!(game.world().party[0].mon.mon.hp, full, "HealParty");
    command(&mut game, Command::Step(Direction::Left));
    play_until(&mut game, 20_000, |game| !game.world().events.is_set(EVENT_IN_PURIFIED_ZONE) && free(game));
}

/// The ghost on the sixth floor is a wild Marowak, and beating it takes it off the stairs for good.
#[test]
fn the_ghost_marowak_fights_and_leaves_the_stairs_when_it_is_beaten() {
    let mut game = game(Map::PokemonTower6F, 10, 16, SpriteFacing::Up, 6, |world| {
        world.bag.add(ItemId::SilphScope, 1);
        one_shot(world);
    });
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    play_until(&mut game, 60_000, |game| game.world().events.is_set(EVENT_BEAT_GHOST_MAROWAK) && free(game));
    assert_eq!(game.world().scripts.maps.pokemon_tower_6f.cur_script, 0);
}

/// A Rocket on the top floor walks out of the room when he is beaten rather than standing where he
/// lost, which is what the floor's own end-of-battle routine is for.
#[test]
fn a_beaten_rocket_leaves_the_top_of_the_tower() {
    use poke_core::symbols::pokered_toggles::TOGGLE_POKEMON_TOWER_7F_ROCKET_1;
    let mut game = game(Map::PokemonTower7F, 10, 11, SpriteFacing::Left, 6, one_shot);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    play_until(&mut game, 80_000, |game| {
        game.world().location.is_hidden(TOGGLE_POKEMON_TOWER_7F_ROCKET_1 as u8) && free(game)
    });
    assert_ne!(game.world().money, [0; 3], "the prize");
    assert_eq!(game.world().scripts.maps.pokemon_tower_7f.cur_script, 0);
}

/// Mr Fuji thanks the player and walks them home, which is a warp the script asks for rather than
/// one under the player's feet.
#[test]
fn mr_fuji_takes_the_player_home_from_the_top_of_the_tower() {
    use poke_core::symbols::pokered_toggles::{TOGGLE_MR_FUJIS_HOUSE_MR_FUJI, TOGGLE_POKEMON_TOWER_7F_MR_FUJI};
    let mut game = game(Map::PokemonTower7F, 10, 4, SpriteFacing::Up, 6, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 40_000, |game| game.world().location.map == Map::MrFujisHouse && free(game));
    let world = game.world();
    assert!(world.events.is_set(EVENT_RESCUED_MR_FUJI) && world.events.is_set(EVENT_RESCUED_MR_FUJI_2));
    assert!(!world.location.is_hidden(TOGGLE_MR_FUJIS_HOUSE_MR_FUJI as u8), "he is home");
    assert!(world.location.is_hidden(TOGGLE_POKEMON_TOWER_7F_MR_FUJI as u8));
    assert_eq!(world.location.last_map, Map::LavenderTown, "the house is left by its own door");
}

/// Rock Tunnel is a plain trainer map, as Routes 8, 9 and 10 and the floor below it are.
#[test]
fn a_rock_tunnel_hiker_talked_to_fights_on_the_spot() {
    let mut game = game(Map::RockTunnel1F, 7, 6, SpriteFacing::Up, 6, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
}


/// The old man by the Celadon store hands TM41 over once, and explains it every time after.
#[test]
fn the_celadon_gramps_hands_over_tm41_and_then_talks_about_it() {
    let mut game = game(Map::CeladonCity, 22, 17, SpriteFacing::Up, 6, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, |game| game.world().events.is_set(EVENT_GOT_TM41) && free(game));
    assert_eq!(game.world().bag.quantity_of(ItemId::Tm41Softboiled), 1);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::Tm41Softboiled), 1, "he has only the one");
}

#[test]
fn erikas_rainbow_badge_comes_with_tm21_and_marks_her_seven_trainers_beaten() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_CELADONGYM_DEFAULT, SCRIPT_CELADONGYM_ERIKA_POST_BATTLE};
    let mut game = game(Map::CeladonGym, 4, 4, SpriteFacing::Up, 5, |world| {
        world.scripts.maps.celadon_gym.cur_script = SCRIPT_CELADONGYM_ERIKA_POST_BATTLE;
    });
    play_until(&mut game, 20_000, |game| {
        game.world().scripts.maps.celadon_gym.cur_script == SCRIPT_CELADONGYM_DEFAULT && free(game)
    });
    let world = game.world();
    assert_eq!(world.badges & 0x08, 0x08, "the Rainbow Badge");
    assert_eq!(world.bag.quantity_of(ItemId::Tm21MegaDrain), 1);
    assert!(world.events.is_set(EVENT_BEAT_ERIKA) && world.events.is_set(EVENT_GOT_TM21));
    assert!(world.events.is_set(EVENT_BEAT_CELADON_GYM_TRAINER_0));
    assert!(world.events.is_set(EVENT_BEAT_CELADON_GYM_TRAINER_6));
}

/// Talking to Erika arms the post-battle script and starts her battle where the player stands.
#[test]
fn talking_to_erika_starts_the_battle_and_arms_the_badge_script() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_CELADONGYM_ERIKA_POST_BATTLE;
    let mut game = game(Map::CeladonGym, 4, 4, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    assert_eq!(game.world().scripts.maps.celadon_gym.cur_script, SCRIPT_CELADONGYM_ERIKA_POST_BATTLE);
}

/// `ReplaceTileBlock`'s block `(x, y)`, past the map view's three blocks of border.
fn block_at(game: &Game, x: usize, y: usize) -> u8 {
    let map = &game.screen().map;
    map.blocks[map.blocks_wide * (y + 3) + x + 3]
}

/// The Rocket in front of the poster fights and walks off east, the wall goes back up behind him,
/// and the switch under the poster opens it for good. The map is marked loaded again when he has
/// gone, which is what runs the callback that puts the wall back.
#[test]
fn the_game_corner_rocket_leaves_the_poster_and_its_switch_behind_him() {
    use poke_core::symbols::pokered_toggles::TOGGLE_GAME_CORNER_ROCKET;
    let mut game = game(Map::GameCorner, 9, 6, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    play_until(&mut game, 80_000, |game| {
        game.world().location.is_hidden(TOGGLE_GAME_CORNER_ROCKET as u8) && free(game)
    });
    assert_eq!(game.world().scripts.maps.game_corner.cur_script, 0);
    assert_eq!(block_at(&game, 8, 2), 0x2A, "walled up again");
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 4000, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, |game| game.world().events.is_set(EVENT_FOUND_ROCKET_HIDEOUT) && free(game));
    assert_eq!(block_at(&game, 8, 2), 0x43, "the staircase down");
}

/// The Fishing Guru hands out ten coins once, and only into a coin case.
#[test]
fn the_game_corner_fishing_guru_hands_out_ten_coins_once() {
    let with_case = |world: &mut World| {
        world.bag.add(ItemId::CoinCase, 1);
    };
    let mut cased = game(Map::GameCorner, 6, 11, SpriteFacing::Left, 5, with_case);
    play_until(&mut cased, 600, free);
    command(&mut cased, Command::Interact);
    play_until(&mut cased, 8000, |game| game.world().events.is_set(EVENT_GOT_10_COINS) && free(game));
    assert_eq!(cased.world().coins, [0x00, 0x10]);
    command(&mut cased, Command::Interact);
    play_until(&mut cased, 8000, free);
    assert_eq!(cased.world().coins, [0x00, 0x10], "he pays out the once");

    let mut caseless = game(Map::GameCorner, 6, 11, SpriteFacing::Left, 5, |_| {});
    play_until(&mut caseless, 600, free);
    command(&mut caseless, Command::Interact);
    play_until(&mut caseless, 8000, free);
    assert_eq!(caseless.world().coins, [0, 0], "and nowhere to put them without the case");
    assert!(!caseless.world().events.is_set(EVENT_GOT_10_COINS));
}

/// The door on the hideout's first floor is walled up until the Rocket in front of it is beaten,
/// and the callback that draws it runs on the map re-entered after any battle on the floor.
#[test]
fn the_rocket_hideout_door_is_walled_up_until_its_guard_is_beaten() {
    let mut game = game(Map::RocketHideoutB1F, 25, 8, SpriteFacing::Right, 5, one_shot);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    play_until(&mut game, 80_000, |game| {
        game.world().events.is_set(EVENT_BEAT_ROCKET_HIDEOUT_1_TRAINER_0) && free(game)
    });
    assert_eq!(block_at(&game, 12, 8), 0x54, "the door, since its own guard is still standing");
}

/// Giovanni goes behind a fade and the Silph Scope is left on the square he was standing on.
#[test]
fn giovanni_leaves_the_silph_scope_behind_him_in_the_hideout() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROCKETHIDEOUTB4F_BEAT_GIOVANNI,
        SCRIPT_ROCKETHIDEOUTB4F_DEFAULT};
    use poke_core::symbols::pokered_toggles::{TOGGLE_ROCKET_HIDEOUT_B4F_GIOVANNI, TOGGLE_ROCKET_HIDEOUT_B4F_ITEM_4};
    let mut game = game(Map::RocketHideoutB4F, 25, 4, SpriteFacing::Up, 5, |world| {
        world.scripts.maps.rocket_hideout_b4f.cur_script = SCRIPT_ROCKETHIDEOUTB4F_BEAT_GIOVANNI;
    });
    play_until(&mut game, 20_000, |game| {
        game.world().scripts.maps.rocket_hideout_b4f.cur_script == SCRIPT_ROCKETHIDEOUTB4F_DEFAULT && free(game)
    });
    let world = game.world();
    assert!(world.events.is_set(EVENT_BEAT_ROCKET_HIDEOUT_GIOVANNI));
    assert!(world.location.is_hidden(TOGGLE_ROCKET_HIDEOUT_B4F_GIOVANNI as u8));
    assert!(!world.location.is_hidden(TOGGLE_ROCKET_HIDEOUT_B4F_ITEM_4 as u8), "the Silph Scope");
}

/// Talking to Giovanni in the hideout arms the script the badge and the scope hang off.
#[test]
fn talking_to_the_hideout_giovanni_starts_the_battle() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_ROCKETHIDEOUTB4F_BEAT_GIOVANNI;
    let mut game = game(Map::RocketHideoutB4F, 25, 4, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    assert_eq!(game.world().scripts.maps.rocket_hideout_b4f.cur_script, SCRIPT_ROCKETHIDEOUTB4F_BEAT_GIOVANNI);
}

/// The girl on the roof takes whichever drink is picked off a menu of the ones in the bag.
#[test]
fn the_thirsty_girl_trades_a_tm_for_each_drink() {
    let mut game = game(Map::CeladonMartRoof, 5, 6, SpriteFacing::Up, 6, |world| {
        world.bag.add(ItemId::SodaPop, 1);
        world.bag.add(ItemId::FreshWater, 1);
    });
    play_until(&mut game, 600, free);
    // She wanders her own square, so wait until she is the one being faced.
    for _ in 0..3000 {
        if let Some(Mode::Overworld(overworld)) = game.modes().last()
            && overworld.in_front_text(game.world()) == 2
            && free(&game)
        {
            break;
        }
        game.frame(Input::None);
    }
    command(&mut game, Command::Interact);
    play_until(&mut game, 4000, |game| game.status() == Status::Waiting(Decision::TwoOption));
    command(&mut game, Command::ChooseOption(0));
    play_until(&mut game, 4000, |game| game.status() == Status::Waiting(Decision::CursorMenu));
    // The menu lists the drinks in the bag in the cartridge's own order, so Fresh Water is first.
    command(&mut game, Command::ChooseOption(1));
    play_until(&mut game, 8000, |game| game.world().events.is_set(EVENT_GOT_TM48) && free(game));
    let world = game.world();
    assert_eq!(world.bag.quantity_of(ItemId::Tm48RockSlide), 1);
    assert_eq!(world.bag.quantity_of(ItemId::SodaPop), 0, "drunk");
    assert_eq!(world.bag.quantity_of(ItemId::FreshWater), 1, "the other one is still in the bag");
}

/// The Eevee stays in its ball until there is room in the party for it.
#[test]
fn the_celadon_eevee_is_left_where_it_is_when_the_party_is_full() {
    use poke_core::symbols::pokered_toggles::TOGGLE_CELADON_MANSION_EEVEE_GIFT;
    let full = |world: &mut World| world.party = vec![mon(PokemonSpecies::Pidgey, 5); 6];
    let mut packed = game(Map::CeladonMansionRoofHouse, 4, 4, SpriteFacing::Up, 6, full);
    play_until(&mut packed, 600, free);
    command(&mut packed, Command::Interact);
    play_until(&mut packed, 8000, free);
    assert_eq!(packed.world().party.len(), 6);
    assert!(!packed.world().location.is_hidden(TOGGLE_CELADON_MANSION_EEVEE_GIFT as u8), "still there");

    let mut room = game(Map::CeladonMansionRoofHouse, 4, 4, SpriteFacing::Up, 6, |_| {});
    play_until(&mut room, 600, free);
    command(&mut room, Command::Interact);
    play_until(&mut room, 8000, |game| game.world().party.len() == 2 && free(game));
    assert_eq!(room.world().party[1].mon.mon.species, PokemonSpecies::Eevee);
    assert!(room.world().location.is_hidden(TOGGLE_CELADON_MANSION_EEVEE_GIFT as u8));
}

/// The binoculars upstairs of the Route 16 gate answer a player who is facing them.
#[test]
fn the_route_16_gate_binoculars_print_to_a_player_facing_up() {
    let mut game = game(Map::Route16Gate2F, 1, 3, SpriteFacing::Up, 6, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::Text));
    play_until(&mut game, 4000, free);
}

/// The man at the back of the diner hands over the Coin Case once and talks about it after.
#[test]
fn the_celadon_diner_hands_over_the_coin_case_once() {
    let mut game = game(Map::CeladonDiner, 0, 2, SpriteFacing::Up, 6, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, |game| game.world().events.is_set(EVENT_GOT_COIN_CASE) && free(game));
    assert_eq!(game.world().bag.quantity_of(ItemId::CoinCase), 1);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::CoinCase), 1, "he has only the one");
}

/// The clerk on the toy floor hands over TM18.
#[test]
fn the_celadon_mart_toy_floor_clerk_hands_over_tm18() {
    let mut game = game(Map::CeladonMart3F, 16, 6, SpriteFacing::Up, 6, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, |game| game.world().events.is_set(EVENT_GOT_TM18) && free(game));
    assert_eq!(game.world().bag.quantity_of(ItemId::Tm18Counter), 1);
}

/// The Silph Co receptionist is only put behind her desk once Giovanni has been beaten, and the
/// event that remembers it means the script only shows her the once.
#[test]
fn the_silph_co_receptionist_comes_back_once_giovanni_is_beaten() {
    use poke_core::symbols::pokered_toggles::TOGGLE_SILPH_CO_1F_RECEPTIONIST;
    let mut occupied = game(Map::SilphCo1F, 4, 5, SpriteFacing::Up, 5, |_| {});
    play_until(&mut occupied, 600, free);
    assert!(occupied.world().location.is_hidden(TOGGLE_SILPH_CO_1F_RECEPTIONIST as u8), "away while the Rockets hold it");
    assert!(!occupied.world().events.is_set(EVENT_SILPH_CO_RECEPTIONIST_AT_DESK));

    let mut cleared = game(Map::SilphCo1F, 4, 5, SpriteFacing::Up, 5, |world| {
        world.events.set(EVENT_BEAT_SILPH_CO_GIOVANNI);
    });
    play_until(&mut cleared, 600, free);
    assert!(!cleared.world().location.is_hidden(TOGGLE_SILPH_CO_1F_RECEPTIONIST as u8));
    assert!(cleared.world().events.is_set(EVENT_SILPH_CO_RECEPTIONIST_AT_DESK));
}

/// The second floor's two card key doors are drawn shut again on every load, and only the events
/// keep one open. The map is reloaded by the battle with the scientist on the floor.
#[test]
fn the_silph_co_2f_card_key_doors_are_drawn_shut_until_their_events_are_set() {
    const CLOSED_DOOR: u8 = 0x54;
    let mut shut = game(Map::SilphCo2F, 5, 13, SpriteFacing::Up, 5, one_shot);
    play_until(&mut shut, 600, free);
    command(&mut shut, Command::Interact);
    play_until(&mut shut, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    play_until(&mut shut, 80_000, |game| {
        game.world().events.is_set(EVENT_BEAT_SILPH_CO_2F_TRAINER_0) && free(game)
    });
    assert_eq!(block_at(&shut, 2, 2), CLOSED_DOOR);
    assert_eq!(block_at(&shut, 2, 5), CLOSED_DOOR);

    let mut opened = game(Map::SilphCo2F, 5, 13, SpriteFacing::Up, 5, |world| {
        one_shot(world);
        world.events.set(EVENT_SILPH_CO_2_UNLOCKED_DOOR1);
    });
    play_until(&mut opened, 600, free);
    command(&mut opened, Command::Interact);
    play_until(&mut opened, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    play_until(&mut opened, 80_000, |game| {
        game.world().events.is_set(EVENT_BEAT_SILPH_CO_2F_TRAINER_0) && free(game)
    });
    assert_ne!(block_at(&opened, 2, 2), CLOSED_DOOR, "the door its event remembers");
    assert_eq!(block_at(&opened, 2, 5), CLOSED_DOOR, "the other one is still shut");
}

/// The Silph worker upstairs hands over TM36, and only the once.
#[test]
fn the_silph_co_2f_worker_hands_over_tm36_once() {
    let mut game = game(Map::SilphCo2F, 10, 2, SpriteFacing::Up, 5, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, |game| game.world().events.is_set(EVENT_GOT_TM36) && free(game));
    assert_eq!(game.world().bag.quantity_of(ItemId::Tm36Selfdestruct), 1);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::Tm36Selfdestruct), 1, "she has only the one");
}

/// Stepping in front of Giovanni on the eleventh floor walks him down to the square below and
/// starts his battle where the player stands.
#[test]
fn giovanni_walks_down_the_eleventh_floor_and_starts_his_battle() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_SILPHCO11F_GIOVANNI_AFTER_BATTLE;
    let mut game = game(Map::SilphCo11F, 7, 12, SpriteFacing::Left, 5, one_shot);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    let state = &game.world().scripts.maps.silph_co_11f;
    assert_eq!(state.cur_script, SCRIPT_SILPHCO11F_GIOVANNI_AFTER_BATTLE);
    assert_eq!(state.saved_coord_index, 2, "the square east of where he stops");
}

/// Beaten, he takes every Rocket in the building and in Saffron's streets away with him, and the
/// townspeople who were kept out take their place.
#[test]
fn giovanni_loses_silph_co_and_takes_team_rocket_out_of_saffron() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_SILPHCO11F_DEFAULT,
        SCRIPT_SILPHCO11F_GIOVANNI_AFTER_BATTLE};
    use poke_core::symbols::pokered_toggles::{TOGGLE_SAFFRON_CITY_1, TOGGLE_SAFFRON_CITY_8, TOGGLE_SILPH_CO_2F_2};
    let mut game = game(Map::SilphCo11F, 7, 12, SpriteFacing::Left, 5, |world| {
        show(world, TOGGLE_SAFFRON_CITY_1);
        world.scripts.maps.silph_co_11f.cur_script = SCRIPT_SILPHCO11F_GIOVANNI_AFTER_BATTLE;
        world.scripts.maps.silph_co_11f.saved_coord_index = 2;
    });
    play_until(&mut game, 20_000, |game| {
        game.world().events.is_set(EVENT_BEAT_SILPH_CO_GIOVANNI) && free(game)
    });
    let world = game.world();
    assert_eq!(world.scripts.maps.silph_co_11f.cur_script, SCRIPT_SILPHCO11F_DEFAULT);
    assert!(world.location.is_hidden(TOGGLE_SAFFRON_CITY_1 as u8), "the Rocket barring a Saffron street");
    assert!(world.location.is_hidden(TOGGLE_SILPH_CO_2F_2 as u8), "and the ones downstairs");
    assert!(!world.location.is_hidden(TOGGLE_SAFFRON_CITY_8 as u8), "the townspeople take their place");
}

/// The president hands over the Master Ball, and only the once.
#[test]
fn the_silph_president_hands_over_the_master_ball_once() {
    let mut game = game(Map::SilphCo11F, 7, 6, SpriteFacing::Up, 5, |world| {
        world.events.set(EVENT_BEAT_SILPH_CO_GIOVANNI);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, |game| game.world().events.is_set(EVENT_GOT_MASTER_BALL) && free(game));
    assert_eq!(game.world().bag.quantity_of(ItemId::MasterBall), 1);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::MasterBall), 1, "he has only the one");
}

/// Sabrina's Marsh Badge comes with TM46 and marks her seven trainers beaten.
#[test]
fn sabrinas_marsh_badge_comes_with_tm46_and_marks_her_seven_trainers_beaten() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_SAFFRONGYM_DEFAULT, SCRIPT_SAFFRONGYM_SABRINA_POST_BATTLE};
    let mut game = game(Map::SaffronGym, 9, 9, SpriteFacing::Up, 5, |world| {
        world.scripts.maps.saffron_gym.cur_script = SCRIPT_SAFFRONGYM_SABRINA_POST_BATTLE;
    });
    play_until(&mut game, 20_000, |game| {
        game.world().scripts.maps.saffron_gym.cur_script == SCRIPT_SAFFRONGYM_DEFAULT && free(game)
    });
    let world = game.world();
    assert_eq!(world.badges & 0x20, 0x20, "the Marsh Badge");
    assert_eq!(world.bag.quantity_of(ItemId::Tm46Psywave), 1);
    assert!(world.events.is_set(EVENT_BEAT_SABRINA) && world.events.is_set(EVENT_GOT_TM46));
    assert!(world.events.is_set(EVENT_BEAT_SAFFRON_GYM_TRAINER_0));
    assert!(world.events.is_set(EVENT_BEAT_SAFFRON_GYM_TRAINER_6));
}

/// Talking to Sabrina arms the post-battle script and starts her battle where the player stands.
#[test]
fn talking_to_sabrina_starts_the_battle_and_arms_the_badge_script() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_SAFFRONGYM_SABRINA_POST_BATTLE;
    let mut game = game(Map::SaffronGym, 9, 9, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    assert_eq!(game.world().scripts.maps.saffron_gym.cur_script, SCRIPT_SAFFRONGYM_SABRINA_POST_BATTLE);
}

/// The third floor draws its own two card key doors shut, in its own tiling, and the worker there
/// talks either side of Giovanni being beaten.
#[test]
fn the_silph_co_3f_card_key_doors_are_drawn_shut_in_the_floors_own_block() {
    const CLOSED_DOOR: u8 = 0x5F;
    let mut game = game(Map::SilphCo3F, 7, 10, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    play_until(&mut game, 80_000, |game| {
        game.world().events.is_set(EVENT_BEAT_SILPH_CO_3F_TRAINER_1) && free(game)
    });
    assert_eq!(block_at(&game, 4, 4), CLOSED_DOOR);
    assert_eq!(block_at(&game, 8, 4), CLOSED_DOOR);
}

/// Koga's Soul Badge comes with TM06 and marks his six trainers beaten.
#[test]
fn kogas_soul_badge_comes_with_tm06_and_marks_his_six_trainers_beaten() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_FUCHSIAGYM_DEFAULT, SCRIPT_FUCHSIAGYM_KOGA_POST_BATTLE};
    let mut game = game(Map::FuchsiaGym, 4, 11, SpriteFacing::Up, 5, |world| {
        world.scripts.maps.fuchsia_gym.cur_script = SCRIPT_FUCHSIAGYM_KOGA_POST_BATTLE;
    });
    play_until(&mut game, 20_000, |game| {
        game.world().scripts.maps.fuchsia_gym.cur_script == SCRIPT_FUCHSIAGYM_DEFAULT && free(game)
    });
    let world = game.world();
    assert_eq!(world.badges & 0x10, 0x10, "the Soul Badge");
    assert_eq!(world.bag.quantity_of(ItemId::Tm06Toxic), 1);
    assert!(world.events.is_set(EVENT_BEAT_KOGA) && world.events.is_set(EVENT_GOT_TM06));
    assert!(world.events.is_set(EVENT_BEAT_FUCHSIA_GYM_TRAINER_0));
    assert!(world.events.is_set(EVENT_BEAT_FUCHSIA_GYM_TRAINER_5));
}

/// Talking to Koga arms the post-battle script and starts his battle where the player stands.
#[test]
fn talking_to_koga_starts_the_battle_and_arms_the_badge_script() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_FUCHSIAGYM_KOGA_POST_BATTLE;
    let mut game = game(Map::FuchsiaGym, 4, 11, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    assert_eq!(game.world().scripts.maps.fuchsia_gym.cur_script, SCRIPT_FUCHSIAGYM_KOGA_POST_BATTLE);
}

/// The warden takes the gold teeth out of the bag, hands over HM04, and explains it after.
#[test]
fn the_warden_swaps_the_gold_teeth_for_hm04() {
    let mut game = game(Map::WardensHouse, 2, 4, SpriteFacing::Up, 5, |world| {
        world.bag.add(ItemId::GoldTeeth, 1);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.world().events.is_set(EVENT_GOT_HM04) && free(game));
    let world = game.world();
    assert_eq!(world.bag.quantity_of(ItemId::Hm04Strength), 1);
    assert_eq!(world.bag.quantity_of(ItemId::GoldTeeth), 0, "the teeth are his again");
    assert!(world.events.is_set(EVENT_GAVE_GOLD_TEETH));
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::Hm04Strength), 1, "he has only the one");
}

/// Without the teeth the warden is unintelligible either way he is answered, and hands out nothing.
#[test]
fn the_warden_is_unintelligible_until_his_teeth_are_back() {
    let mut game = game(Map::WardensHouse, 2, 4, SpriteFacing::Up, 5, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_answering(&mut game, 20_000, &mut vec![1], free);
    let world = game.world();
    assert_eq!(world.bag.quantity_of(ItemId::Hm04Strength), 0);
    assert!(!world.events.is_set(EVENT_GAVE_GOLD_TEETH));
}

/// The gate walks the player to the counter, takes the fee and starts the game.
#[test]
fn the_safari_zone_gate_takes_500_for_thirty_balls_and_502_steps() {
    let mut game = game(Map::SafariZoneGate, 3, 2, SpriteFacing::Right, 5, |world| {
        world.money = [0x00, 0x10, 0x00];
    });
    play_until(&mut game, 600, |game| (game.world().location.x, game.world().location.y) == (4, 2));
    play_answering(&mut game, 20_000, &mut vec![0], |game| game.world().safari_balls == 30);
    let world = game.world();
    assert_eq!(world.safari_steps, 502, "two more than the 500 the sign outside promises");
    assert_eq!(world.money, [0x00, 0x05, 0x00]);
    assert!(world.events.is_set(EVENT_IN_SAFARI_ZONE));
}

/// A game that has run out ends at the gate: the balls go back and the zone lets the player go.
#[test]
fn the_safari_zone_gate_takes_the_balls_back_when_the_game_is_over() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_SAFARIZONEGATE_DEFAULT, SCRIPT_SAFARIZONEGATE_LEAVING_SAFARI};
    let mut game = game(Map::SafariZoneGate, 4, 1, SpriteFacing::Down, 5, |world| {
        world.scripts.maps.safari_zone_gate.cur_script = SCRIPT_SAFARIZONEGATE_LEAVING_SAFARI;
        world.events.set(EVENT_IN_SAFARI_ZONE);
        world.events.set(EVENT_SAFARI_GAME_OVER);
        world.safari_balls = 7;
        world.safari_steps = 200;
    });
    play_until(&mut game, 20_000, |game| {
        game.world().scripts.maps.safari_zone_gate.cur_script == SCRIPT_SAFARIZONEGATE_DEFAULT && free(game)
    });
    let world = game.world();
    assert_eq!(world.safari_balls, 0);
    assert!(!world.events.is_set(EVENT_IN_SAFARI_ZONE) && !world.events.is_set(EVENT_SAFARI_GAME_OVER));
    assert_eq!((world.location.x, world.location.y), (4, 4), "walked back down to the door");
}

/// The secret house at the far end of the zone hands over HM03, and only the once.
#[test]
fn the_safari_zone_secret_house_hands_over_hm03_once() {
    let mut game = game(Map::SafariZoneSecretHouse, 3, 4, SpriteFacing::Up, 5, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.world().events.is_set(EVENT_GOT_HM03) && free(game));
    assert_eq!(game.world().bag.quantity_of(ItemId::Hm03Surf), 1);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::Hm03Surf), 1, "he has only the one");
}

/// Fuchsia's fossil sign reads as undetermined until a fossil has been picked up, and then opens the
/// Pokédex page of what that fossil becomes.
#[test]
fn the_fuchsia_fossil_sign_names_whichever_fossil_was_left_behind() {
    let mut game = game(Map::FuchsiaCity, 7, 8, SpriteFacing::Up, 5, |world| {
        world.events.set(EVENT_GOT_DOME_FOSSIL);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_answering(&mut game, 20_000, &mut vec![0], |game| {
        game.world().pokedex.is_seen(PokemonSpecies::Omanyte)
    });
    assert!(!game.world().pokedex.is_seen(PokemonSpecies::Kabuto), "the helix fossil is still in the wall");
}

/// A hidden object has nothing in front of the player to talk to, so it takes a raw press.
fn press_a(game: &mut Game) {
    for _ in 0..4 {
        game.frame(Input::Buttons(crate::input::Joypad::A));
    }
}

/// The mansion's switch is only a switch from below it, and it toggles: one wall opens as three
/// shut, and pressing it again puts all four back.
#[test]
fn the_mansion_switch_opens_one_wall_and_shuts_three() {
    const GATE: u8 = 0x2D;
    const FLOOR: u8 = 0x0E;
    let mut game = game(Map::PokemonMansion1F, 2, 6, SpriteFacing::Up, 5, |_| {});
    play_until(&mut game, 600, free);
    press_a(&mut game);
    play_answering(&mut game, 8000, &mut vec![0], |game| game.world().events.is_set(EVENT_MANSION_SWITCH_ON) && free(game));
    assert_eq!(block_at(&game, 12, 6), GATE, "the wall the switch shuts");
    assert_eq!(block_at(&game, 8, 3), FLOOR);
    assert_eq!(block_at(&game, 10, 8), FLOOR);
    assert_eq!(block_at(&game, 13, 13), FLOOR);

    press_a(&mut game);
    play_answering(&mut game, 8000, &mut vec![0], |game| !game.world().events.is_set(EVENT_MANSION_SWITCH_ON) && free(game));
    assert_eq!(block_at(&game, 12, 6), FLOOR, "and back again");
    assert_eq!(block_at(&game, 8, 3), GATE);
}

/// The basement's switch prints the second floor's words and moves four walls of its own, two of
/// them into blocks that are not gates at all.
#[test]
fn the_mansion_basement_switch_moves_four_walls_of_its_own() {
    let mut game = game(Map::PokemonMansionB1F, 20, 4, SpriteFacing::Up, 5, |_| {});
    play_until(&mut game, 600, free);
    press_a(&mut game);
    play_answering(&mut game, 8000, &mut vec![0], |game| game.world().events.is_set(EVENT_MANSION_SWITCH_ON) && free(game));
    assert_eq!(block_at(&game, 13, 8), 0x2D);
    assert_eq!(block_at(&game, 6, 11), 0x5F);
    assert_eq!(block_at(&game, 4, 3), 0x0E, "the way to the Secret Key");
    assert_eq!(block_at(&game, 8, 8), 0x0E);
}

/// Blaine's Volcano Badge comes with TM38 and marks all seven of his gates won.
#[test]
fn blaines_volcano_badge_comes_with_tm38_and_marks_his_seven_trainers_beaten() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_CINNABARGYM_BLAINE_POST_BATTLE, SCRIPT_CINNABARGYM_DEFAULT};
    let mut game = game(Map::CinnabarGym, 3, 4, SpriteFacing::Up, 5, |world| {
        world.scripts.maps.cinnabar_gym.cur_script = SCRIPT_CINNABARGYM_BLAINE_POST_BATTLE;
    });
    play_until(&mut game, 20_000, |game| {
        game.world().scripts.maps.cinnabar_gym.cur_script == SCRIPT_CINNABARGYM_DEFAULT && free(game)
    });
    let world = game.world();
    assert_eq!(world.badges & 0x40, 0x40, "the Volcano Badge");
    assert_eq!(world.bag.quantity_of(ItemId::Tm38FireBlast), 1);
    assert!(world.events.is_set(EVENT_BEAT_BLAINE) && world.events.is_set(EVENT_GOT_TM38));
    assert!(world.events.is_set(EVENT_BEAT_CINNABAR_GYM_TRAINER_0));
    assert!(world.events.is_set(EVENT_BEAT_CINNABAR_GYM_TRAINER_6));
}

/// Talking to Blaine arms the post-battle script and starts his battle where the player stands.
#[test]
fn talking_to_blaine_starts_the_battle_and_arms_the_badge_script() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_CINNABARGYM_BLAINE_POST_BATTLE;
    let mut game = game(Map::CinnabarGym, 3, 4, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    assert_eq!(game.world().scripts.maps.cinnabar_gym.cur_script, SCRIPT_CINNABARGYM_BLAINE_POST_BATTLE);
}

/// Beating a Super Nerd opens the gate two below his text id, and the gym's own script draws it.
#[test]
fn beating_a_cinnabar_super_nerd_opens_his_gate() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_CINNABARGYM_DEFAULT;
    let mut game = game(Map::CinnabarGym, 17, 9, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    play_until(&mut game, 80_000, |game| {
        game.world().scripts.maps.cinnabar_gym.cur_script == SCRIPT_CINNABARGYM_DEFAULT
            && game.world().events.is_set(EVENT_BEAT_CINNABAR_GYM_TRAINER_1)
            && free(game)
    });
    assert!(game.world().events.is_set(EVENT_CINNABAR_GYM_GATE0_UNLOCKED + 1));
    assert_eq!(block_at(&game, 9, 3), 0x0E, "the gate his answer was guarding");
}

/// The lab hands the revived fossil over at level 30 and forgets it was ever holding one.
#[test]
fn the_cinnabar_lab_hands_over_the_revived_fossil() {
    let mut game = game(Map::CinnabarLabFossilRoom, 5, 3, SpriteFacing::Up, 5, |world| {
        world.fossil = Some((ItemId::DomeFossil, PokemonSpecies::Kabuto));
        world.events.set(EVENT_GAVE_FOSSIL_TO_LAB);
    });
    play_until(&mut game, 600, free);
    // The scientist paces left and right: wait for him to come back over his own square.
    for _ in 0..3000 {
        if let Some(Mode::Overworld(overworld)) = game.modes().last()
            && overworld.in_front_text(game.world()) == 1
            && free(&game)
        {
            break;
        }
        game.frame(Input::None);
    }
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, |game| game.world().party.len() == 2 && free(game));
    let world = game.world();
    assert_eq!(world.party[1].mon.mon.species, PokemonSpecies::Kabuto);
    assert_eq!(world.party[1].mon.level, 30);
    assert!(!world.events.is_set(EVENT_GAVE_FOSSIL_TO_LAB));
    assert!(!world.events.is_set(EVENT_LAB_HANDING_OVER_FOSSIL_MON));
}

/// Cinnabar Island is where the mansion's switch springs back and the lab's fossil finishes
/// reviving, and its gym door pushes anyone without the Secret Key back down.
#[test]
fn cinnabar_island_resets_the_mansion_switch_and_shuts_the_gym_door() {
    let mut locked = game(Map::CinnabarIsland, 18, 4, SpriteFacing::Up, 5, |world| {
        world.events.set(EVENT_MANSION_SWITCH_ON);
        world.events.set(EVENT_LAB_STILL_REVIVING_FOSSIL);
    });
    play_until(&mut locked, 8000, |game| game.world().location.y == 5 && free(game));
    let world = locked.world();
    assert!(!world.events.is_set(EVENT_MANSION_SWITCH_ON));
    assert!(!world.events.is_set(EVENT_LAB_STILL_REVIVING_FOSSIL));
    assert_eq!((world.location.x, world.location.y), (18, 5), "pushed back off the door");

    let mut keyed = game(Map::CinnabarIsland, 18, 4, SpriteFacing::Up, 5, |world| {
        world.bag.add(ItemId::SecretKey, 1);
    });
    play_until(&mut keyed, 600, free);
    for _ in 0..600 {
        keyed.frame(Input::None);
    }
    assert_eq!((keyed.world().location.x, keyed.world().location.y), (18, 4), "the key opens it");
}



/// A party that can shift a boulder: STRENGTH in the first slot and every badge that allows it.
fn strength(world: &mut World) {
    world.badges = 0xFF;
    let mut mon = new_party_mon(PokemonSpecies::Squirtle, 20, 1, &Origin::Trainer, &mut GameRng::tape(vec![]));
    mon.mon.moves[0] = Some(poke_core::move_name::PokemonMoveName::Strength);
    world.party = vec![Named { mon, ot: encode("RED").unwrap(), nick: encode("SQUIRT").unwrap() }];
}

/// START, POKéMON, the only mon, STRENGTH at the top of its field-move submenu, and its text.
fn use_strength(game: &mut Game) {
    command(game, Command::OpenStartMenu);
    command(game, Command::ChooseStartMenuEntry(crate::modes::start_menu::StartMenuEntry::Pokemon));
    command(game, Command::ChooseOption(0));
    command(game, Command::ChooseOption(0));
    play_until(game, 3000, free);
}

/// Leans on a direction until `done`: the first pass against a boulder only arms
/// `BIT_TRIED_PUSH_BOULDER`, so a push takes several.
fn lean(game: &mut Game, button: crate::input::Joypad, limit: u32, mut done: impl FnMut(&Game) -> bool) {
    for _ in 0..limit {
        if done(game) {
            return;
        }
        game.frame(Input::Buttons(button));
    }
    panic!("nothing moved: {:?}", game.status());
}

/// Victory Road 3F's fourth boulder stands one square west of the hole: pushed in, it is hidden
/// here and shown on the floor below, and the hole then takes the player down after it.
#[test]
fn a_boulder_pushed_down_victory_roads_hole_turns_up_on_the_floor_below() {
    use poke_core::symbols::pokered_toggles::{TOGGLE_VICTORY_ROAD_2F_BOULDER, TOGGLE_VICTORY_ROAD_3F_BOULDER};
    let mut game = game(Map::VictoryRoad3F, 21, 15, SpriteFacing::Right, 5, strength);
    play_until(&mut game, 600, free);
    use_strength(&mut game);
    lean(&mut game, crate::input::Joypad::RIGHT, 2000,
        |game| game.world().events.is_set(EVENT_VICTORY_ROAD_3_BOULDER_ON_SWITCH2));
    let location = &game.world().location;
    assert!(location.is_hidden(TOGGLE_VICTORY_ROAD_3F_BOULDER as u8), "the boulder has gone down");
    assert!(!location.is_hidden(TOGGLE_VICTORY_ROAD_2F_BOULDER as u8), "and is lying on 2F");

    lean(&mut game, crate::input::Joypad::RIGHT, 4000, |game| game.world().location.map == Map::VictoryRoad2F);
    play_until(&mut game, 4000, free);
    assert_eq!(game.world().location.map, Map::VictoryRoad2F, "the player fell down after it");
}

/// Victory Road's gates are blocks rather than objects, so nothing remembers them: each floor's
/// script draws its gate open again every time the map is loaded, which here is the player walking
/// in through the stairs between the two floors.
#[test]
fn victory_roads_gates_are_drawn_open_again_whenever_the_floor_is_loaded() {
    /// The block a switch held down opens its gate to.
    const OPEN: u8 = 0x1D;
    let mut first = game(Map::VictoryRoad2F, 1, 8, SpriteFacing::Left, 5, |world| {
        world.events.set(EVENT_VICTORY_ROAD_1_BOULDER_ON_SWITCH);
    });
    play_until(&mut first, 600, free);
    command(&mut first, Command::Step(Direction::Left));
    play_until(&mut first, 8000, |game| game.world().location.map == Map::VictoryRoad1F && free(game));
    assert_eq!(block_at(&first, 4, 6), OPEN, "the floor below's gate");

    let mut second = game(Map::VictoryRoad1F, 2, 1, SpriteFacing::Left, 5, |world| {
        world.events.set(EVENT_VICTORY_ROAD_2_BOULDER_ON_SWITCH1);
        world.events.set(EVENT_VICTORY_ROAD_2_BOULDER_ON_SWITCH2);
    });
    play_until(&mut second, 600, free);
    command(&mut second, Command::Step(Direction::Left));
    play_until(&mut second, 8000, |game| game.world().location.map == Map::VictoryRoad2F && free(game));
    assert_eq!((block_at(&second, 3, 4), block_at(&second, 11, 7)), (0x15, OPEN), "both of this floor's");
}

/// The door into Lorelei's room seals behind the player: walking in is six squares the room walks
/// for them, and it is what arms the flag the lobby reads.
#[test]
fn lorelei_walks_the_player_in_and_shuts_the_door_behind_them() {
    let mut game = game(Map::IndigoPlateauLobby, 8, 1, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 20_000, |game| {
        game.world().events.is_set(EVENT_AUTOWALKED_INTO_LORELEIS_ROOM) && free(game)
    });
    let world = game.world();
    assert_eq!(world.location.map, Map::LoreleisRoom);
    assert_eq!((world.location.x, world.location.y), (4, 5), "six squares up from the doorway");
    assert!(world.scripts.started_elite_4, "the lobby will forget an unfinished challenge");
    assert_eq!(block_at(&game, 2, 0), 0x24, "the way on is shut until she is beaten");
}

/// The lobby undoes a challenge that was started and not finished, and puts Victory Road's first
/// switch back up with it. A challenge nobody started leaves the rooms as they are.
#[test]
fn the_indigo_plateau_lobby_forgets_a_half_fought_elite_four() {
    fn arrive(started: bool) -> Game {
        let mut game = game(Map::IndigoPlateau, 9, 6, SpriteFacing::Up, 5, move |world| {
            world.events.set(EVENT_VICTORY_ROAD_1_BOULDER_ON_SWITCH);
            world.events.set(EVENT_BEAT_LORELEIS_ROOM_TRAINER_0);
            world.events.set(EVENT_AUTOWALKED_INTO_LORELEIS_ROOM);
            world.scripts.started_elite_4 = started;
        });
        play_until(&mut game, 600, free);
        command(&mut game, Command::Step(Direction::Up));
        play_until(&mut game, 20_000, |game| game.world().location.map == Map::IndigoPlateauLobby && free(game));
        game
    }

    let forgotten = arrive(true);
    let world = forgotten.world();
    assert!(!world.events.is_set(EVENT_VICTORY_ROAD_1_BOULDER_ON_SWITCH), "the switch springs back");
    assert!(!world.events.is_set(EVENT_BEAT_LORELEIS_ROOM_TRAINER_0));
    assert!(!world.events.is_set(EVENT_AUTOWALKED_INTO_LORELEIS_ROOM));
    assert!(!world.scripts.started_elite_4);

    let kept = arrive(false);
    let world = kept.world();
    assert!(!world.events.is_set(EVENT_VICTORY_ROAD_1_BOULDER_ON_SWITCH), "the switch springs back either way");
    assert!(world.events.is_set(EVENT_BEAT_LORELEIS_ROOM_TRAINER_0), "but a finished challenge stands");
}

/// The whole Elite Four gauntlet's doors: each room walks the player six squares in from its
/// doorway, and each opens the way on only once its own trainer is beaten. Lance's room is the odd
/// one, walking the player the length of a hallway and bricking the doorway up behind them.
#[test]
fn the_elite_four_rooms_walk_the_player_in_and_seal_themselves() {
    let mut game = game(Map::IndigoPlateauLobby, 8, 1, SpriteFacing::Up, 5, |world| {
        for event in [EVENT_BEAT_LORELEIS_ROOM_TRAINER_0, EVENT_BEAT_BRUNOS_ROOM_TRAINER_0,
            EVENT_BEAT_AGATHAS_ROOM_TRAINER_0] {
            world.events.set(event);
        }
    });
    play_until(&mut game, 600, free);

    fn climb_to(game: &mut Game, map: Map) {
        for _ in 0..8 {
            if game.world().location.map == map {
                return;
            }
            command(game, Command::Step(Direction::Up));
            play_until(game, 20_000, free);
        }
        panic!("never reached {map:?}");
    }

    for (map, walked, open) in [(Map::LoreleisRoom, EVENT_AUTOWALKED_INTO_LORELEIS_ROOM, 0x05),
        (Map::BrunosRoom, EVENT_AUTOWALKED_INTO_BRUNOS_ROOM, 0x05),
        (Map::AgathasRoom, EVENT_AUTOWALKED_INTO_AGATHAS_ROOM, 0x0E)] {
        climb_to(&mut game, map);
        play_until(&mut game, 20_000, |game| game.world().events.is_set(walked) && free(game));
        assert_eq!((game.world().location.x, game.world().location.y), (4, 5), "six squares up from {map:?}");
        assert_eq!(block_at(&game, 2, 0), open, "{map:?}'s trainer is beaten, so the way on is open");
    }

    climb_to(&mut game, Map::LancesRoom);
    play_until(&mut game, 40_000, |game| {
        game.world().events.is_set(EVENT_LANCES_ROOM_LOCK_DOOR) && free(game)
    });
    let world = game.world();
    assert_eq!((world.location.x, world.location.y), (6, 11), "the far end of the hallway");
    assert_eq!((block_at(&game, 2, 6), block_at(&game, 3, 6)), (0x72, 0x73), "the doorway is bricked up");
}

/// The champion's room, which Agatha's room arms: the rival walks the player in and the battle
/// starts without either of them being spoken to.
#[test]
fn the_rival_walks_the_player_into_the_champions_room_and_battles_them() {
    let mut game = game(Map::ChampionsRoom, 3, 7, SpriteFacing::Up, 5, |world| {
        world.scripts.maps.champions_room.cur_script = SCRIPT_CHAMPIONSROOM_PLAYER_ENTERS;
    });
    play_until(&mut game, 40_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    let world = game.world();
    assert_eq!((world.location.x, world.location.y), (4, 3), "stood in front of the rival");
    assert!(world.options.battle_animation, "the last battle is animated whatever was set");
}

/// Oak arrives once the rival is beaten, walks out, and the player follows him into the Hall of
/// Fame, where the ceremony that ends the game takes over.
#[test]
fn oak_walks_the_new_champion_to_the_hall_of_fame() {
    let mut game = game(Map::ChampionsRoom, 4, 3, SpriteFacing::Up, 5, |world| {
        world.events.set(EVENT_BEAT_CHAMPION_RIVAL);
        world.scripts.maps.champions_room.cur_script = SCRIPT_CHAMPIONSROOM_RIVAL_DEFEATED;
    });
    play_until(&mut game, 40_000, |game| game.world().location.map == Map::HallOfFame);
    play_until(&mut game, 40_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Movie(_))));
    assert_eq!(game.world().scripts.maps.loreleis_room.cur_script, 0, "the gauntlet can be fought again");
}
