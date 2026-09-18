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

/// A toggleable object a new game starts with shown, which a script somewhere else would hide.
fn hide(world: &mut World, toggle: u16) {
    let index = toggle as usize;
    world.location.hidden_objects[index / 8] |= 1 << (index % 8);
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

/// Articuno fights whoever walks up to it, and the floor's own script takes the current back over
/// once the battle is over.
#[test]
fn articuno_fights_the_player_who_walks_up_to_it() {
    use poke_core::symbols::pokered_toggles::TOGGLE_ARTICUNO;
    let mut game = game(Map::SeafoamIslandsB4F, 6, 2, SpriteFacing::Up, 5, |world| {
        one_shot(world);
        // Mewtwo's own first move is Swift, which an Articuno at 50 outlasts.
        world.party[0].mon.mon.moves[0] = Some(poke_core::move_name::PokemonMoveName::Psychic);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    play_until(&mut game, 60_000, |game| game.world().events.is_set(EVENT_BEAT_ARTICUNO) && free(game));
    assert!(game.world().location.is_hidden(TOGGLE_ARTICUNO as u8));
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

/// The second floor's switch presses the same button as the ground floor's and moves three walls of
/// its own, only the first of which it shuts.
#[test]
fn the_mansion_second_floor_switch_moves_its_own_three_walls() {
    let mut game = game(Map::PokemonMansion2F, 2, 12, SpriteFacing::Up, 5, |_| {});
    play_until(&mut game, 600, free);
    press_a(&mut game);
    play_answering(&mut game, 8000, &mut vec![0], |game| game.world().events.is_set(EVENT_MANSION_SWITCH_ON) && free(game));
    assert_eq!(block_at(&game, 4, 2), 0x5F);
    assert_eq!(block_at(&game, 9, 4), 0x0E);
    assert_eq!(block_at(&game, 3, 11), 0x0E);
}

/// The top floor's switch moves two walls, one open and one shut.
#[test]
fn the_mansion_top_floor_switch_moves_two_walls() {
    let mut game = game(Map::PokemonMansion3F, 10, 6, SpriteFacing::Up, 5, |_| {});
    play_until(&mut game, 600, free);
    press_a(&mut game);
    play_answering(&mut game, 8000, &mut vec![0], |game| game.world().events.is_set(EVENT_MANSION_SWITCH_ON) && free(game));
    assert_eq!(block_at(&game, 7, 2), 0x5F);
    assert_eq!(block_at(&game, 7, 5), 0x0E);
}

/// The top floor's three holes are not all the same drop: the rightmost lands on the second floor
/// and the other two fall all the way to the ground floor.
#[test]
fn a_hole_in_the_mansions_top_floor_drops_the_player_onto_the_floor_it_names() {
    for (x, below) in [(17, Map::PokemonMansion1F), (19, Map::PokemonMansion2F)] {
        let mut game = game(Map::PokemonMansion3F, x, 13, SpriteFacing::Down, 5, |_| {});
        play_until(&mut game, 600, free);
        command(&mut game, Command::Step(Direction::Down));
        play_until(&mut game, 5000, |game| game.world().location.map == below && free(game));
    }
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

/// Whether `words` are on the first line of the text box.
fn on_screen(game: &Game, words: &str) -> bool {
    let words = encode(&format!("{words}@")).unwrap();
    let words = &words[..words.len() - 1];
    game.screen().ui.row(14).windows(words.len()).any(|window| window == words)
}

/// Waits until the sprite with `text_id` is the one the player faces, it being free to wander.
fn wait_to_face(game: &mut Game, text_id: u8) {
    wait_to_face_within(game, text_id, 6000);
}

/// The same for a sprite that wanders in every direction, and so takes longer to come back.
fn wait_to_face_within(game: &mut Game, text_id: u8, limit: u32) {
    for _ in 0..limit {
        if let Some(Mode::Overworld(overworld)) = game.modes().last()
            && overworld.in_front_text(game.world()) == text_id
            && free(game)
        {
            return;
        }
        game.frame(Input::None);
    }
    panic!("text {text_id} never came in front of the player");
}

/// Mom only sends the player to Oak until there is a starter, and heals the party after.
#[test]
fn mom_heals_the_party_only_once_the_player_has_a_starter() {
    let talk = |got_starter: bool| {
        let mut game = game(Map::RedsHouse1F, 4, 4, SpriteFacing::Right, 3, |world| {
            world.party[0].mon.mon.hp = 1;
            world.scripts.got_starter = got_starter;
        });
        play_until(&mut game, 600, free);
        command(&mut game, Command::Interact);
        play_until(&mut game, 5000, free);
        game.world().party[0].mon.mon.hp
    };
    assert_eq!(talk(false), 1, "not yet");
    let fresh = game(Map::RedsHouse1F, 4, 4, SpriteFacing::Right, 3, |_| {});
    assert_eq!(talk(true), fresh.world().party[0].mon.stats[0], "healed");
}

/// The TV shows its film only to a player standing in front of it.
#[test]
fn the_tv_in_reds_house_shows_its_film_only_from_the_front() {
    let watch = |x: u8, y: u8, facing: SpriteFacing| {
        let mut game = game(Map::RedsHouse1F, x, y, facing, 3, |_| {});
        play_until(&mut game, 600, free);
        command(&mut game, Command::Interact);
        play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::Text));
        let film = on_screen(&game, "There's a movie");
        let wrong_side = on_screen(&game, "Oops, wrong side.");
        play_until(&mut game, 2000, free);
        (film, wrong_side)
    };
    assert_eq!(watch(3, 2, SpriteFacing::Up), (true, false));
    assert_eq!(watch(4, 1, SpriteFacing::Left), (false, true));
}

/// Daisy has the Town Map for the player only once Oak has given out the Pokédex, and it leaves the
/// table as she hands it over.
#[test]
fn daisy_hands_over_the_town_map_once_the_pokedex_is_in() {
    use poke_core::symbols::pokered_toggles::TOGGLE_TOWN_MAP;
    let mut before = game(Map::BluesHouse, 2, 4, SpriteFacing::Up, 3, |_| {});
    play_until(&mut before, 600, free);
    assert!(before.world().events.is_set(EVENT_ENTERED_BLUES_HOUSE));
    command(&mut before, Command::Interact);
    play_until(&mut before, 3000, free);
    assert_eq!(before.world().bag.quantity_of(ItemId::TownMap), 0, "not before the Pokédex");

    let mut game = game(Map::BluesHouse, 2, 4, SpriteFacing::Up, 3, |world| world.events.set(EVENT_GOT_POKEDEX));
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 5000, |game| game.world().events.is_set(EVENT_GOT_TOWN_MAP) && free(game));
    assert_eq!(game.world().bag.quantity_of(ItemId::TownMap), 1);
    assert!(game.world().location.is_hidden(TOGGLE_TOWN_MAP as u8));
    command(&mut game, Command::Interact);
    play_until(&mut game, 3000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::TownMap), 1, "she has only the one");
}

/// Speary and the Pewter Nidoran each cry after their words, and the box closes on a press.
#[test]
fn speary_and_the_pewter_nidoran_cry_after_their_words() {
    use poke_core::symbols::pokered_map_scripts::{TEXT_PEWTERNIDORANHOUSE_NIDORAN, TEXT_VIRIDIANNICKNAMEHOUSE_SPEAROW};
    for (map, x, y, facing, text_id) in [
        (Map::ViridianNicknameHouse, 4, 4, SpriteFacing::Down, TEXT_VIRIDIANNICKNAMEHOUSE_SPEAROW),
        (Map::PewterNidoranHouse, 5, 5, SpriteFacing::Left, TEXT_PEWTERNIDORANHOUSE_NIDORAN),
    ] {
        let mut game = game(map, x, y, facing, 3, |_| {});
        play_until(&mut game, 600, free);
        // Speary paces left and right along the row below.
        wait_to_face(&mut game, text_id);
        command(&mut game, Command::Interact);
        let mut cried = false;
        play_until(&mut game, 5000, |game| {
            cried |= (4..8).any(|channel| game.audio().channel_sound_id(channel) != 0);
            free(game)
        });
        assert!(cried, "{map:?}");
    }
}

/// Oak's aide on Route 2 hands over HM05 to a player with ten Pokémon owned, and explains Flash
/// ever after.
#[test]
fn the_route_2_aide_hands_over_hm05_for_ten_owned() {
    let ask = |owned: [u8; 2]| {
        let mut game = game(Map::Route2Gate, 2, 4, SpriteFacing::Left, 3, |world| {
            world.pokedex.owned[..2].copy_from_slice(&owned);
        });
        play_until(&mut game, 600, free);
        command(&mut game, Command::Interact);
        play_answering(&mut game, 8000, &mut vec![0], free);
        game
    };
    let short = ask([0xFF, 0x01]);
    assert_eq!(short.world().bag.quantity_of(ItemId::Hm05Flash), 0, "nine is not enough");
    assert!(!short.world().events.is_set(EVENT_GOT_HM05));

    let mut enough = ask([0xFF, 0x03]);
    assert_eq!(enough.world().bag.quantity_of(ItemId::Hm05Flash), 1);
    assert!(enough.world().events.is_set(EVENT_GOT_HM05));
    command(&mut enough, Command::Interact);
    play_answering(&mut enough, 8000, &mut vec![0], free);
    assert_eq!(enough.world().bag.quantity_of(ItemId::Hm05Flash), 1, "once");
}

/// The Route 2 trade house's boy trades his Mr. Mime for an Abra.
#[test]
fn the_route_2_trade_house_boy_trades_a_mr_mime_for_an_abra() {
    let mut game = game(Map::Route2TradeHouse, 4, 2, SpriteFacing::Up, 3, |world| {
        world.party.push(mon(PokemonSpecies::Abra, 5));
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    for _ in 0..8000 {
        if free(&game) && game.world().in_game_trades != 0 {
            break;
        }
        let input = match game.status() {
            Status::Waiting(Decision::Text) => Input::Command(Command::Advance),
            Status::Waiting(Decision::TwoOption) => Input::Command(Command::ChooseOption(0)),
            Status::Waiting(Decision::PartyMenu) => Input::Command(Command::ChooseOption(1)),
            _ => Input::None,
        };
        game.frame(input);
    }
    assert_eq!(game.world().party[1].mon.mon.species, PokemonSpecies::MrMime);
}

/// Whichever way the player came in, the doors of the cave's Route 2 end lead back to Route 2.
#[test]
fn digletts_cave_holds_its_route_2_door_to_route_2() {
    let mut game = game(Map::DiglettsCaveRoute2, 3, 5, SpriteFacing::Down, 3, |world| {
        world.location.last_map = Map::Route11;
    });
    play_until(&mut game, 600, free);
    assert_eq!(game.world().location.last_map, Map::Route2);
}

/// The Route 22 gate's guard walks a player without the Boulder Badge back a square, and lets one
/// with it through for good; the gate's two halves open onto two different routes.
#[test]
fn the_route_22_gate_guard_turns_back_a_player_without_the_boulder_badge() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROUTE22GATE_DEFAULT, SCRIPT_ROUTE22GATE_NOOP,
        SCRIPT_ROUTE22GATE_PLAYER_MOVING};
    let step_up = |badge: bool| {
        let mut game = game(Map::Route22Gate, 4, 3, SpriteFacing::Up, 3, |world| {
            if badge {
                world.badges = 1;
            }
        });
        play_until(&mut game, 600, free);
        assert_eq!(game.world().location.last_map, Map::Route23);
        command(&mut game, Command::Step(Direction::Up));
        game
    };
    let mut turned_back = step_up(false);
    play_until(&mut turned_back, 5000, |game| {
        game.world().scripts.maps.route22_gate.cur_script == SCRIPT_ROUTE22GATE_PLAYER_MOVING
    });
    play_until(&mut turned_back, 5000, |game| {
        free(game) && game.world().scripts.maps.route22_gate.cur_script == SCRIPT_ROUTE22GATE_DEFAULT
    });
    assert_eq!((turned_back.world().location.x, turned_back.world().location.y), (4, 3), "walked back down");

    let mut through = step_up(true);
    play_until(&mut through, 5000, |game| {
        free(game) && game.world().scripts.maps.route22_gate.cur_script == SCRIPT_ROUTE22GATE_NOOP
    });
    assert_eq!((through.world().location.x, through.world().location.y), (4, 2));

    let mut south = game(Map::Route22Gate, 4, 5, SpriteFacing::Up, 3, |_| {});
    play_until(&mut south, 600, free);
    assert_eq!(south.world().location.last_map, Map::Route22);
}

/// The Pewter Mart's shoppers talk in a box of their own, the map drawing none for them.
#[test]
fn the_pewter_mart_shoppers_talk() {
    use poke_core::symbols::pokered_map_scripts::TEXT_PEWTERMART_SUPER_NERD;
    let mut game = game(Map::PewterMart, 5, 6, SpriteFacing::Up, 3, |_| {});
    play_until(&mut game, 600, free);
    wait_to_face(&mut game, TEXT_PEWTERMART_SUPER_NERD);
    command(&mut game, Command::Interact);
    play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::Text));
    assert_eq!(game.screen().ui.get(0, 12), 0x79, "the box's corner");
    play_until(&mut game, 2000, free);
}

/// Jigglypuff sings the song through, turning round on the spot until it ends, and the box closes
/// by itself after.
#[test]
fn jigglypuff_turns_round_for_as_long_as_it_sings() {
    let mut game = game(Map::PewterPokecenter, 1, 4, SpriteFacing::Up, 3, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    let mut pictures = std::collections::BTreeSet::new();
    let mut frames = 0;
    for _ in 0..3000 {
        if free(&game) && frames > 100 {
            break;
        }
        if let Some(Mode::Overworld(overworld)) = game.modes().iter().find(|mode| matches!(mode, Mode::Overworld(_))) {
            pictures.insert(overworld.sprites()[3].image_index);
        }
        assert_ne!(game.status(), Status::Waiting(Decision::Text), "the box waits for nothing");
        game.frame(Input::None);
        frames += 1;
    }
    assert!(free(&game));
    assert!(pictures.len() >= 4, "{pictures:x?}");
    assert!(frames > 32 + 48 + 24 * 4, "{frames}");
}

/// The Cerulean trade house's gambler trades his Jynx for a Poliwhirl.
#[test]
fn the_cerulean_trade_house_gambler_trades_a_jynx_for_a_poliwhirl() {
    let mut game = game(Map::CeruleanTradeHouse, 1, 3, SpriteFacing::Up, 3, |world| {
        world.party.push(mon(PokemonSpecies::Poliwhirl, 20));
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    for _ in 0..8000 {
        if free(&game) && game.world().in_game_trades != 0 {
            break;
        }
        let input = match game.status() {
            Status::Waiting(Decision::Text) => Input::Command(Command::Advance),
            Status::Waiting(Decision::TwoOption) => Input::Command(Command::ChooseOption(0)),
            Status::Waiting(Decision::PartyMenu) => Input::Command(Command::ChooseOption(1)),
            _ => Input::None,
        };
        game.frame(input);
    }
    assert_eq!(game.world().party[1].mon.mon.species, PokemonSpecies::Jynx);
}

/// Talks to what is in front and waits for the first box, returning whether `words` open it.
fn first_words(game: &mut Game, words: &str) -> bool {
    command(game, Command::Interact);
    play_until(game, 2000, |game| game.status() == Status::Waiting(Decision::Text));
    let said = on_screen(game, words);
    play_until(game, 4000, free);
    said
}

/// The trashed house's fishing guru has given up on his TM once the player holds Dig, except that
/// the count is masked by the ROM bank, so eight of them read as none.
#[test]
fn the_cerulean_fishing_guru_gives_up_on_his_tm_once_the_player_has_dig() {
    let guru = |dig: u8| {
        let mut game = game(Map::CeruleanTrashedHouse, 2, 2, SpriteFacing::Up, 3, |world| {
            if dig != 0 {
                world.bag.add(ItemId::Tm28Dig, dig);
            }
        });
        play_until(&mut game, 600, free);
        game
    };
    assert!(first_words(&mut guru(0), "Those miserable"));
    assert!(first_words(&mut guru(1), "I figure what's"));
    assert!(first_words(&mut guru(8), "Those miserable"));
}

/// The badge man explains a badge picked off the list and asks again with the cursor where it was,
/// until the list is backed out of.
#[test]
fn the_cerulean_badge_man_explains_badges_until_the_list_is_cancelled() {
    let mut game = game(Map::CeruleanBadgeHouse, 5, 4, SpriteFacing::Up, 3, |world| world.badges = 1);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 4000, |game| game.status() == Status::Waiting(Decision::List));
    command(&mut game, Command::ChooseListEntry(2));
    play_until(&mut game, 4000, |game| game.status() == Status::Waiting(Decision::Text) && on_screen(game, "The SPEED of all"));
    play_until(&mut game, 4000, |game| game.status() == Status::Waiting(Decision::List));
    assert_eq!(game.menu().list_scroll + game.menu().chosen_item, 2, "the list reopens where it was left");
    command(&mut game, Command::CancelList);
    play_until(&mut game, 4000, free);
    assert_eq!(game.menu().list_scroll, 0);
}

/// The bike shop's clerk swaps the voucher for a Bicycle, once.
#[test]
fn the_bike_shop_clerk_swaps_the_voucher_for_a_bicycle() {
    let mut game = game(Map::BikeShop, 6, 3, SpriteFacing::Up, 3, |world| {
        world.bag.add(ItemId::BikeVoucher, 1);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 4000, free);
    let world = game.world();
    assert_eq!(world.bag.quantity_of(ItemId::Bicycle), 1);
    assert_eq!(world.bag.quantity_of(ItemId::BikeVoucher), 0);
    assert!(world.events.is_set(EVENT_GOT_BICYCLE));
    assert!(first_words(&mut game, "How do you like"));
}

/// Without a voucher the clerk offers the Bicycle off a menu; backing out with B skips resetting the
/// text delay flag, so the text prints instantly from then on.
#[test]
fn backing_out_of_the_bike_shop_menu_leaves_text_printing_at_once() {
    let offer = |cancel: Command| {
        let mut game = game(Map::BikeShop, 6, 3, SpriteFacing::Up, 3, |_| {});
        play_until(&mut game, 600, free);
        command(&mut game, Command::Interact);
        play_until(&mut game, 4000, |game| game.status() == Status::Waiting(Decision::CursorMenu));
        command(&mut game, cancel);
        play_until(&mut game, 4000, free);
        game
    };
    assert!(offer(Command::CancelOption).world().no_text_delay);
    assert!(!offer(Command::ChooseOption(1)).world().no_text_delay);
    assert_eq!(offer(Command::ChooseOption(0)).world().bag.quantity_of(ItemId::Bicycle), 0, "a million is too much");
}

/// The Mt. Moon Magikarp salesman takes ¥500 for his Magikarp once, and turns away a player without.
#[test]
fn the_magikarp_salesman_sells_one_magikarp_for_500() {
    use poke_core::symbols::pokered_map_scripts::TEXT_MTMOONPOKECENTER_MAGIKARP_SALESMAN;
    let buy = |money: [u8; 3]| {
        let mut game = game(Map::MtMoonPokecenter, 10, 7, SpriteFacing::Up, 3, |world| world.money = money);
        play_until(&mut game, 600, free);
        wait_to_face(&mut game, TEXT_MTMOONPOKECENTER_MAGIKARP_SALESMAN);
        command(&mut game, Command::Interact);
        play_answering(&mut game, 8000, &mut vec![0, 1], |game| {
            free(game) && !game.modes().iter().any(|mode| !matches!(mode, Mode::Overworld(_)))
        });
        game
    };
    let bought = buy([0x00, 0x06, 0x00]);
    let world = bought.world();
    assert_eq!(world.party.len(), 2);
    assert_eq!(world.party[1].mon.mon.species, PokemonSpecies::Magikarp);
    assert_eq!(world.money, [0x00, 0x01, 0x00]);
    assert!(world.events.is_set(EVENT_BOUGHT_MAGIKARP));

    let broke = buy([0x00, 0x04, 0x99]);
    assert_eq!(broke.world().party.len(), 1);
    assert_eq!(broke.world().money, [0x00, 0x04, 0x99]);
    assert!(!broke.world().events.is_set(EVENT_BOUGHT_MAGIKARP));
}

/// The bike shop's youngster admires the player's Bicycle once there is one.
#[test]
fn the_bike_shop_youngster_admires_the_bicycle_once_it_is_had() {
    let youngster = |got_bike: bool| {
        let mut game = game(Map::BikeShop, 1, 2, SpriteFacing::Down, 3, |world| {
            if got_bike {
                world.events.set(EVENT_GOT_BICYCLE);
            }
        });
        play_until(&mut game, 600, free);
        game
    };
    assert!(first_words(&mut youngster(false), "These BIKEs are"));
    assert!(first_words(&mut youngster(true), "Wow. Your BIKE is"));
}

/// The Vermilion Pidgey and the Fan Club's Pikachu and Seel each cry after their words.
#[test]
fn the_vermilion_pidgey_and_the_fan_club_pets_cry_after_their_words() {
    use poke_core::symbols::pokered_map_scripts::{TEXT_POKEMONFANCLUB_PIKACHU, TEXT_POKEMONFANCLUB_SEEL,
        TEXT_VERMILIONPIDGEYHOUSE_PIDGEY};
    for (map, x, y, facing, text_id) in [
        (Map::VermilionPidgeyHouse, 3, 4, SpriteFacing::Down, TEXT_VERMILIONPIDGEYHOUSE_PIDGEY),
        (Map::PokemonFanClub, 5, 4, SpriteFacing::Right, TEXT_POKEMONFANCLUB_PIKACHU),
        (Map::PokemonFanClub, 2, 4, SpriteFacing::Left, TEXT_POKEMONFANCLUB_SEEL),
    ] {
        let mut game = game(map, x, y, facing, 3, |_| {});
        play_until(&mut game, 600, free);
        // The Pidgey paces left and right along the row below.
        wait_to_face(&mut game, text_id);
        command(&mut game, Command::Interact);
        let mut cried = false;
        play_until(&mut game, 5000, |game| {
            cried |= (4..8).any(|channel| game.audio().channel_sound_id(channel) != 0);
            free(game)
        });
        assert!(cried, "{map:?} {text_id}");
    }
}

/// The Fishing Guru gives the Old Rod only to a player who says they like to fish, and only once.
#[test]
fn the_fishing_guru_gives_the_old_rod_to_a_player_who_likes_to_fish() {
    let mut game = game(Map::VermilionOldRodHouse, 3, 4, SpriteFacing::Left, 3, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_answering(&mut game, 8000, &mut vec![1], free);
    assert_eq!(game.world().bag.quantity_of(ItemId::OldRod), 0, "not to a player who said no");
    assert!(!game.world().scripts.got_old_rod);

    command(&mut game, Command::Interact);
    play_answering(&mut game, 8000, &mut vec![0], free);
    assert_eq!(game.world().bag.quantity_of(ItemId::OldRod), 1);
    assert!(game.world().scripts.got_old_rod);

    command(&mut game, Command::Interact);
    play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::Text));
    assert!(on_screen(&game, "Hello there,"));
    play_answering(&mut game, 8000, &mut vec![0], free);
    assert_eq!(game.world().bag.quantity_of(ItemId::OldRod), 1, "once");
}

/// Hearing the chairman's story out earns a Bike Voucher, once, and a player already holding a
/// Bicycle is not told it.
#[test]
fn the_fan_club_chairman_gives_a_bike_voucher_for_hearing_his_story() {
    let talk = |game: &mut Game, answer: u8| {
        command(game, Command::Interact);
        play_answering(game, 20_000, &mut vec![answer], free);
    };
    let mut game = game(Map::PokemonFanClub, 3, 2, SpriteFacing::Up, 3, |_| {});
    play_until(&mut game, 600, free);
    talk(&mut game, 1);
    assert_eq!(game.world().bag.quantity_of(ItemId::BikeVoucher), 0, "not without the story");
    talk(&mut game, 0);
    assert_eq!(game.world().bag.quantity_of(ItemId::BikeVoucher), 1);
    assert!(game.world().events.is_set(EVENT_GOT_BIKE_VOUCHER));
    talk(&mut game, 0);
    assert_eq!(game.world().bag.quantity_of(ItemId::BikeVoucher), 1, "once");

    let mut cyclist = self::game(Map::PokemonFanClub, 3, 2, SpriteFacing::Up, 3, |world| {
        world.bag.add(ItemId::Bicycle, 1);
    });
    play_until(&mut cyclist, 600, free);
    command(&mut cyclist, Command::Interact);
    play_until(&mut cyclist, 2000, |game| game.status() == Status::Waiting(Decision::Text));
    assert!(on_screen(&cyclist, "Hello, "), "the chairman has nothing left for a cyclist");
}

/// Each fan's plain boast arms the other's comeback, which is said once.
#[test]
fn the_fan_club_fans_out_boast_each_other() {
    let mut game = game(Map::PokemonFanClub, 5, 3, SpriteFacing::Right, 3, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::Text));
    assert!(on_screen(&game, "Won't you admire"));
    assert!(!game.world().events.is_set(EVENT_SEEL_FAN_BOAST), "the event is set once the text is closed");
    play_answering(&mut game, 3000, &mut vec![0], free);
    assert!(game.world().events.is_set(EVENT_SEEL_FAN_BOAST));

    let mut game = self::game(Map::PokemonFanClub, 2, 3, SpriteFacing::Left, 3, |world| {
        world.events.set(EVENT_SEEL_FAN_BOAST);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::Text));
    assert!(on_screen(&game, "Oh dear!"));
    play_answering(&mut game, 3000, &mut vec![0], free);
    assert!(!game.world().events.is_set(EVENT_SEEL_FAN_BOAST));
    assert!(!game.world().events.is_set(EVENT_PIKACHU_FAN_BOAST), "the comeback arms nothing");
}

/// Coming down the gangway with HM01, the S.S. Anne sails off: the ship's lines scroll away under a
/// drifting puff of smoke, its rows are left as water, and the player is walked off the dock.
#[test]
fn the_ss_anne_sails_off_as_the_player_leaves_it_with_hm01() {
    let mut game = game(Map::SSAnne1F, 26, 1, SpriteFacing::Up, 3, |world| {
        world.events.set(EVENT_GOT_HM01);
        world.location.last_map = Map::VermilionCity;
    });
    play_until(&mut game, 600, free);
    lean(&mut game, crate::input::Joypad::UP, 2000, |game| game.world().location.map == Map::VermilionDock);
    play_until(&mut game, 2000, |game| game.world().events.is_set(EVENT_SS_ANNE_LEFT));
    assert_eq!(game.world().location.map, Map::VermilionDock);

    let (mut frames, mut scrolled, mut smoke) = (0, 0, false);
    play_until(&mut game, 2000, |game| game.screen().background.is_some());
    play_until(&mut game, 2000, |game| {
        let screen = game.screen();
        if let Some(lines) = &screen.effects.line_scx {
            assert_eq!((lines[79], lines[128]), (0, 0), "only the ship's lines scroll");
            scrolled = scrolled.max(lines[80]);
        }
        smoke |= screen.sprites.get(4).is_some_and(|object| object.tile == 0xFC && object.y == 100);
        frames += 1;
        screen.background.is_none()
    });
    assert_eq!(scrolled, 127);
    assert!(smoke);
    assert!((8 * 16 * 8..8 * 16 * 8 + 130).contains(&frames), "{frames}");

    let map = &game.screen().map;
    for row in 10..16 {
        for column in 0..20 {
            assert_eq!(map.tile_at(map.camera.0 + column * 8, map.camera.1 + row * 8), Some(0x14), "({column}, {row})");
        }
    }
    play_until(&mut game, 2000, |game| game.world().location.map == Map::VermilionCity && free(game));
    assert!(game.world().events.is_set(EVENT_STARTED_WALKING_OUT_OF_DOCK));
}

/// A Sailor on the Bow fights when he is spoken to, and says something else once he is beaten.
#[test]
fn a_ss_anne_sailor_fights_when_he_is_spoken_to() {
    use poke_core::symbols::pokered_map_scripts::TEXT_SSANNEBOW_SAILOR2;
    let mut game = game(Map::SSAnneBow, 3, 4, SpriteFacing::Right, 5, one_shot);
    play_until(&mut game, 600, free);
    wait_to_face(&mut game, TEXT_SSANNEBOW_SAILOR2);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    play_until(&mut game, 20_000, |game| game.world().events.is_set(EVENT_BEAT_SS_ANNE_5_TRAINER_0) && free(game));
    assert_eq!(game.world().scripts.maps.ss_anne_bow.cur_script, 0);
    assert_ne!(game.world().money, [0; 3], "the prize");
}

/// The Wigglytuff and the Machoke in the cabins each cry after their words.
#[test]
fn the_ss_anne_wigglytuff_and_machoke_cry_after_their_words() {
    use poke_core::symbols::pokered_map_scripts::{TEXT_SSANNE1FROOMS_WIGGLYTUFF, TEXT_SSANNEB1FROOMS_MACHOKE};
    for (map, x, y, facing, text_id) in [
        (Map::SSAnne1FRooms, 4, 11, SpriteFacing::Left, TEXT_SSANNE1FROOMS_WIGGLYTUFF),
        (Map::SSAnneB1FRooms, 11, 13, SpriteFacing::Up, TEXT_SSANNEB1FROOMS_MACHOKE),
    ] {
        let mut game = game(map, x, y, facing, 3, |_| {});
        play_until(&mut game, 600, free);
        wait_to_face(&mut game, text_id);
        command(&mut game, Command::Interact);
        let mut cried = false;
        play_until(&mut game, 5000, |game| {
            cried |= (4..8).any(|channel| game.audio().channel_sound_id(channel) != 0);
            free(game)
        });
        assert!(cried, "{map:?}");
    }
}

/// The second-class cabins draw no box for a text, so the gentleman who saw the Snorlax prints his
/// words in one of his own, and the Pokédex page he opens marks it seen.
#[test]
fn the_ss_anne_gentleman_shows_the_snorlax_he_saw() {
    use poke_core::symbols::pokered_map_scripts::TEXT_SSANNE2FROOMS_GENTLEMAN3;
    let mut game = game(Map::SSAnne2FRooms, 1, 3, SpriteFacing::Up, 5, |_| {});
    play_until(&mut game, 600, free);
    wait_to_face(&mut game, TEXT_SSANNE2FROOMS_GENTLEMAN3);
    command(&mut game, Command::Interact);
    play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::Text));
    assert_eq!(game.screen().ui.get(0, 12), 0x79, "the box's corner");
    play_answering(&mut game, 20_000, &mut vec![0], |game| game.world().pokedex.is_seen(PokemonSpecies::Snorlax));
    play_until(&mut game, 5000, free);
}

/// The head cook announces the main course, which is one of three and rolled every time he is asked.
#[test]
fn the_ss_anne_head_cook_announces_one_of_three_main_courses() {
    use poke_core::symbols::pokered_map_scripts::TEXT_SSANNEKITCHEN_COOK7;
    let dishes: std::collections::BTreeSet<Vec<u8>> = (1..12)
        .map(|seed| {
            let mut game = game(Map::SSAnneKitchen, 11, 12, SpriteFacing::Down, seed, |_| {});
            play_until(&mut game, 600, free);
            wait_to_face(&mut game, TEXT_SSANNEKITCHEN_COOK7);
            command(&mut game, Command::Interact);
            // The first line of every box the cook puts up: his announcement, then the dish itself.
            let mut lines: Vec<Vec<u8>> = Vec::new();
            play_until(&mut game, 5000, |game| {
                if game.status() == Status::Waiting(Decision::Text) {
                    let line: Vec<u8> = (1..18).map(|column| game.screen().ui.get(column, 14)).collect();
                    if lines.last() != Some(&line) {
                        lines.push(line);
                    }
                }
                free(game)
            });
            assert!(lines.len() >= 2, "the announcement and the dish: {lines:?}");
            lines.pop().expect("a dish")
        })
        .collect();
    assert!(dishes.len() >= 2, "the dish is rolled: {dishes:?}");
}

/// An item ball in the second-class cabins draws its own box, the map having turned the automatic
/// one off.
#[test]
fn a_ss_anne_2f_rooms_item_ball_still_hands_its_item_over() {
    use poke_core::symbols::pokered_toggles::TOGGLE_SS_ANNE_2F_ROOMS_ITEM_1;
    let mut game = game(Map::SSAnne2FRooms, 11, 1, SpriteFacing::Right, 5, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 5000, |game| game.world().bag.quantity_of(ItemId::MaxEther) == 1 && free(game));
    assert!(game.world().location.is_hidden(TOGGLE_SS_ANNE_2F_ROOMS_ITEM_1 as u8), "the ball is off the map");
}

/// Whichever way the player came in, the doors of the cave's Route 11 end lead back to Route 11.
#[test]
fn digletts_cave_holds_its_route_11_door_to_route_11() {
    let mut game = game(Map::DiglettsCaveRoute11, 3, 5, SpriteFacing::Down, 3, |world| {
        world.location.last_map = Map::Route2;
    });
    play_until(&mut game, 600, free);
    assert_eq!(game.world().location.last_map, Map::Route11);
}

/// Oak's aide upstairs of the Route 11 gate hands over the Itemfinder for thirty owned, and
/// explains it ever after.
#[test]
fn the_route_11_gate_aide_hands_over_the_itemfinder_for_thirty_owned() {
    let ask = |owned: [u8; 4]| {
        let mut game = game(Map::Route11Gate2F, 3, 6, SpriteFacing::Left, 3, |world| {
            world.pokedex.owned[..4].copy_from_slice(&owned);
        });
        play_until(&mut game, 600, free);
        command(&mut game, Command::Interact);
        play_answering(&mut game, 8000, &mut vec![0], free);
        game
    };
    let short = ask([0xFF, 0xFF, 0xFF, 0x1F]);
    assert_eq!(short.world().bag.quantity_of(ItemId::Itemfinder), 0, "twenty-nine is not enough");
    assert!(!short.world().events.is_set(EVENT_GOT_ITEMFINDER));

    let mut enough = ask([0xFF, 0xFF, 0xFF, 0x3F]);
    assert_eq!(enough.world().bag.quantity_of(ItemId::Itemfinder), 1);
    assert!(enough.world().events.is_set(EVENT_GOT_ITEMFINDER));
    command(&mut enough, Command::Interact);
    play_answering(&mut enough, 8000, &mut vec![0], free);
    assert_eq!(enough.world().bag.quantity_of(ItemId::Itemfinder), 1, "once");
}

/// The left binoculars of the Route 11 gate look out over Route 12, so what they show depends on
/// whether the Snorlax is still lying across it.
#[test]
fn the_route_11_gate_binoculars_show_the_snorlax_until_it_is_beaten() {
    let look = |beaten: bool| {
        let mut game = game(Map::Route11Gate2F, 1, 3, SpriteFacing::Up, 6, |world| {
            if beaten {
                world.events.set(EVENT_BEAT_ROUTE12_SNORLAX);
            }
        });
        play_until(&mut game, 600, free);
        command(&mut game, Command::Interact);
        play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::Text));
        game.frame(Input::Command(Command::Advance));
        play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::Text));
        let said = on_screen(&game, if beaten { "It's a beautiful" } else { "A big" });
        play_until(&mut game, 4000, free);
        said
    };
    assert!(look(false), "a road with a Snorlax on it");
    assert!(look(true), "a road without one");
}

/// The girl upstairs of the Route 12 gate hands over TM39 once, then talks about Swift.
#[test]
fn the_route_12_gate_girl_hands_over_tm39_once() {
    let mut game = game(Map::Route12Gate2F, 2, 4, SpriteFacing::Right, 3, |_| {});
    play_until(&mut game, 600, free);
    wait_to_face(&mut game, poke_core::symbols::pokered_map_scripts::TEXT_ROUTE12GATE2F_BRUNETTE_GIRL);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::Tm39Swift), 1);
    assert!(game.world().events.is_set(EVENT_GOT_TM39));

    wait_to_face(&mut game, poke_core::symbols::pokered_map_scripts::TEXT_ROUTE12GATE2F_BRUNETTE_GIRL);
    assert!(first_words(&mut game, "TM39 is a move"));
    assert_eq!(game.world().bag.quantity_of(ItemId::Tm39Swift), 1, "once");
}

/// The Fishing Guru's brother gives the Super Rod only to a player who says they like to fish, and
/// only once.
#[test]
fn the_super_rod_guru_gives_his_rod_to_a_player_who_likes_to_fish() {
    let mut game = game(Map::Route12SuperRodHouse, 3, 4, SpriteFacing::Left, 3, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_answering(&mut game, 8000, &mut vec![1], free);
    assert_eq!(game.world().bag.quantity_of(ItemId::SuperRod), 0, "not to a player who said no");
    assert!(!game.world().scripts.got_super_rod);

    command(&mut game, Command::Interact);
    play_answering(&mut game, 8000, &mut vec![0], free);
    assert_eq!(game.world().bag.quantity_of(ItemId::SuperRod), 1);
    assert!(game.world().scripts.got_super_rod);

    assert!(first_words(&mut game, "Hello there,"));
    assert_eq!(game.world().bag.quantity_of(ItemId::SuperRod), 1, "once");
}

/// A Route 12 fisherman spots the player on the road beside his pier.
#[test]
fn a_route_12_fisherman_sees_the_player_on_the_road() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_ROUTE12_END_BATTLE;
    let mut game = game(Map::Route12, 10, 53, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 40_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    assert_eq!(game.world().scripts.maps.route12.cur_script, SCRIPT_ROUTE12_END_BATTLE);
}

/// The Poké Flute played beside the Snorlax on Route 12 wakes it into a battle, and beating it
/// takes it off the road for good.
#[test]
fn the_poke_flute_wakes_the_route_12_snorlax() {
    use poke_core::symbols::pokered_toggles::TOGGLE_ROUTE_12_SNORLAX;
    use crate::modes::start_menu::StartMenuEntry;
    let mut game = game(Map::Route12, 10, 61, SpriteFacing::Down, 3, |world| {
        one_shot(world);
        world.bag.add(ItemId::PokeFlute, 1);
    });
    play_until(&mut game, 600, free);
    assert!(!game.world().location.is_hidden(TOGGLE_ROUTE_12_SNORLAX as u8), "it is lying there");

    command(&mut game, Command::OpenStartMenu);
    command(&mut game, Command::ChooseStartMenuEntry(StartMenuEntry::Item));
    command(&mut game, Command::ChooseListEntry(0));
    command(&mut game, Command::ChooseOption(0));
    play_until(&mut game, 30_000, |game| game.world().events.is_set(EVENT_BEAT_ROUTE12_SNORLAX));
    play_until(&mut game, 4000, free);
    assert!(game.world().location.is_hidden(TOGGLE_ROUTE_12_SNORLAX as u8), "the road is clear");
    assert_eq!(game.world().bag.quantity_of(ItemId::PokeFlute), 1, "the flute is not spent");
}

// ---- Routes 13 to 15, and the gate between the last two ----

/// The three roads out to Fuchsia are plain trainer maps: a trainer whose line the player is
/// standing in engages on the map's own passes, with no step taken.
#[test]
fn a_trainer_on_each_of_routes_13_to_15_sees_the_player_standing_in_its_line() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROUTE13_END_BATTLE, SCRIPT_ROUTE14_END_BATTLE,
        SCRIPT_ROUTE15_END_BATTLE};
    for (map, x, y, end_battle) in [
        (Map::Route13, 10, 6, SCRIPT_ROUTE13_END_BATTLE),
        (Map::Route14, 6, 47, SCRIPT_ROUTE14_END_BATTLE),
        (Map::Route15, 31, 11, SCRIPT_ROUTE15_END_BATTLE),
    ] {
        let mut game = game(map, x, y, SpriteFacing::Down, 5, one_shot);
        play_until(&mut game, 40_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
        let scripts = &game.world().scripts.maps;
        let cur_script = match map {
            Map::Route13 => scripts.route13.cur_script,
            Map::Route14 => scripts.route14.cur_script,
            _ => scripts.route15.cur_script,
        };
        assert_eq!(cur_script, end_battle, "{map:?}");
    }
}

/// Oak's aide upstairs of the Route 15 gate hands over the EXP.ALL for fifty owned, and explains it
/// ever after.
#[test]
fn the_route_15_gate_aide_hands_over_the_exp_all_for_fifty_owned() {
    let ask = |owned: [u8; 7]| {
        let mut game = game(Map::Route15Gate2F, 4, 3, SpriteFacing::Up, 3, |world| {
            world.pokedex.owned[..7].copy_from_slice(&owned);
        });
        play_until(&mut game, 600, free);
        command(&mut game, Command::Interact);
        play_answering(&mut game, 8000, &mut vec![0], free);
        game
    };
    let short = ask([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01]);
    assert_eq!(short.world().bag.quantity_of(ItemId::ExpAll), 0, "forty-nine is not enough");
    assert!(!short.world().events.is_set(EVENT_GOT_EXP_ALL));

    let mut enough = ask([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x03]);
    assert_eq!(enough.world().bag.quantity_of(ItemId::ExpAll), 1);
    assert!(enough.world().events.is_set(EVENT_GOT_EXP_ALL));
    assert!(first_words(&mut enough, "EXP.ALL gives"));
    assert_eq!(enough.world().bag.quantity_of(ItemId::ExpAll), 1, "once");
}

/// The binoculars beside him show the island out at sea, the map drawing the box itself.
#[test]
fn the_route_15_gate_binoculars_print_to_a_player_facing_up() {
    let mut game = game(Map::Route15Gate2F, 6, 3, SpriteFacing::Up, 6, |_| {});
    play_until(&mut game, 600, free);
    assert!(first_words(&mut game, "Looked into the"));
}

// ---- Routes 16 to 18, the Cycling Road ----

/// A rider on each of the three Cycling Road maps engages a player standing in its line, on the
/// map's own passes and with no step taken.
#[test]
fn a_trainer_on_each_of_routes_16_to_18_sees_the_player_standing_in_its_line() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROUTE16_END_BATTLE, SCRIPT_ROUTE17_END_BATTLE,
        SCRIPT_ROUTE18_END_BATTLE};
    for (map, x, y, end_battle) in [
        (Map::Route16, 15, 12, SCRIPT_ROUTE16_END_BATTLE),
        (Map::Route17, 10, 19, SCRIPT_ROUTE17_END_BATTLE),
        (Map::Route18, 38, 11, SCRIPT_ROUTE18_END_BATTLE),
    ] {
        let mut game = game(map, x, y, SpriteFacing::Down, 5, one_shot);
        play_until(&mut game, 40_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
        let scripts = &game.world().scripts.maps;
        let cur_script = match map {
            Map::Route16 => scripts.route16.cur_script,
            Map::Route17 => scripts.route17.cur_script,
            _ => scripts.route18.cur_script,
        };
        assert_eq!(cur_script, end_battle, "{map:?}");
    }
}

/// The Poké Flute played beside the Snorlax on Route 16 wakes it into a battle, and beating it
/// takes it off the road for good.
#[test]
fn the_poke_flute_wakes_the_route_16_snorlax() {
    use poke_core::symbols::pokered_toggles::TOGGLE_ROUTE_16_SNORLAX;
    use crate::modes::start_menu::StartMenuEntry;
    let mut game = game(Map::Route16, 27, 10, SpriteFacing::Left, 3, |world| {
        one_shot(world);
        world.bag.add(ItemId::PokeFlute, 1);
    });
    play_until(&mut game, 600, free);
    assert!(!game.world().location.is_hidden(TOGGLE_ROUTE_16_SNORLAX as u8), "it is lying there");

    command(&mut game, Command::OpenStartMenu);
    command(&mut game, Command::ChooseStartMenuEntry(StartMenuEntry::Item));
    command(&mut game, Command::ChooseListEntry(0));
    command(&mut game, Command::ChooseOption(0));
    play_until(&mut game, 30_000, |game| game.world().events.is_set(EVENT_BEAT_ROUTE16_SNORLAX));
    play_until(&mut game, 4000, free);
    assert!(game.world().location.is_hidden(TOGGLE_ROUTE_16_SNORLAX as u8), "the road is clear");
}

/// The girl hiding behind the Cycling Road hands over HM02 once, and explains Fly ever after.
#[test]
fn the_route_16_fly_house_girl_hands_over_hm02_once() {
    let mut game = game(Map::Route16FlyHouse, 3, 3, SpriteFacing::Left, 3, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::Hm02Fly), 1);
    assert!(game.world().events.is_set(EVENT_GOT_HM02));

    assert!(first_words(&mut game, "HM02 is FLY."));
    assert_eq!(game.world().bag.quantity_of(ItemId::Hm02Fly), 1, "once");
}

/// The Fearow beside her cries after its words.
#[test]
fn the_route_16_fly_house_fearow_cries_after_its_words() {
    use poke_core::symbols::pokered_map_scripts::TEXT_ROUTE16FLYHOUSE_FEAROW;
    let mut game = game(Map::Route16FlyHouse, 6, 5, SpriteFacing::Up, 3, |_| {});
    play_until(&mut game, 600, free);
    // The Fearow wanders the room, so wait for it to come back to its perch.
    wait_to_face(&mut game, TEXT_ROUTE16FLYHOUSE_FEAROW);
    command(&mut game, Command::Interact);
    let mut cried = false;
    play_until(&mut game, 5000, |game| {
        cried |= (4..8).any(|channel| game.audio().channel_sound_id(channel) != 0);
        free(game)
    });
    assert!(cried);
}

/// The youngster upstairs of the Route 18 gate trades his Lickitung for a Slowbro.
#[test]
fn the_route_18_gate_youngster_trades_a_lickitung_for_a_slowbro() {
    let mut game = game(Map::Route18Gate2F, 4, 3, SpriteFacing::Up, 3, |world| {
        world.party.push(mon(PokemonSpecies::Slowbro, 30));
    });
    play_until(&mut game, 600, free);
    wait_to_face(&mut game, poke_core::symbols::pokered_map_scripts::TEXT_ROUTE18GATE2F_YOUNGSTER);
    command(&mut game, Command::Interact);
    for _ in 0..8000 {
        if free(&game) && game.world().in_game_trades != 0 {
            break;
        }
        let input = match game.status() {
            Status::Waiting(Decision::Text) => Input::Command(Command::Advance),
            Status::Waiting(Decision::TwoOption) => Input::Command(Command::ChooseOption(0)),
            Status::Waiting(Decision::PartyMenu) => Input::Command(Command::ChooseOption(1)),
            _ => Input::None,
        };
        game.frame(input);
    }
    assert_eq!(game.world().party[1].mon.mon.species, PokemonSpecies::Lickitung);
}

/// The binoculars beside him look west over Pallet Town, to a player facing up.
#[test]
fn the_route_18_gate_binoculars_print_to_a_player_facing_up() {
    let mut game = game(Map::Route18Gate2F, 1, 3, SpriteFacing::Up, 6, |_| {});
    play_until(&mut game, 600, free);
    assert!(first_words(&mut game, "Looked into the"));
}

// ---- The sea routes and the Seafoam Islands ----

/// The three sea roads are plain trainer maps: a swimmer whose line the player is standing in
/// engages on the map's own passes, with no step taken.
#[test]
fn a_trainer_on_each_of_the_sea_routes_sees_the_player_standing_in_its_line() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROUTE19_END_BATTLE, SCRIPT_ROUTE20_END_BATTLE,
        SCRIPT_ROUTE21_END_BATTLE};
    for (map, x, y, end_battle) in [
        (Map::Route19, 7, 7, SCRIPT_ROUTE19_END_BATTLE),
        (Map::Route20, 45, 11, SCRIPT_ROUTE20_END_BATTLE),
        (Map::Route21, 10, 30, SCRIPT_ROUTE21_END_BATTLE),
    ] {
        let mut game = game(map, x, y, SpriteFacing::Down, 5, one_shot);
        play_until(&mut game, 40_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
        let scripts = &game.world().scripts.maps;
        let cur_script = match map {
            Map::Route19 => scripts.route19.cur_script,
            Map::Route20 => scripts.route20.cur_script,
            _ => scripts.route21.cur_script,
        };
        assert_eq!(cur_script, end_battle, "{map:?}");
    }
}

/// The Seafoam hole at (17, 6) takes a player who walks onto it down to B1F, and standing on the
/// island's top floor at all is what tells Route 20 to put the boulders back.
#[test]
fn a_seafoam_hole_drops_the_player_onto_the_floor_below() {
    let mut game = game(Map::SeafoamIslands1F, 17, 6, SpriteFacing::Down, 5, |_| {});
    play_until(&mut game, 8000, |game| game.world().location.map == Map::SeafoamIslandsB1F);
    play_until(&mut game, 4000, free);
    assert!(game.world().events.is_set(EVENT_IN_SEAFOAM_ISLANDS));
}

/// B2F's first boulder stands one square west of its hole: pushed in, it is hidden here and shown
/// in B3F's water, where it is what slows the current at the foot of B3F's steps. It is only on the
/// floor at all because it came down from B1F, so the test has to put it there.
#[test]
fn a_boulder_pushed_down_a_seafoam_hole_turns_up_in_the_water_below() {
    use poke_core::symbols::pokered_toggles::{TOGGLE_SEAFOAM_ISLANDS_B2F_BOULDER_1,
        TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_3};
    let mut game = game(Map::SeafoamIslandsB2F, 17, 6, SpriteFacing::Right, 5, |world| {
        strength(world);
        show(world, TOGGLE_SEAFOAM_ISLANDS_B2F_BOULDER_1);
    });
    play_until(&mut game, 600, free);
    use_strength(&mut game);
    lean(&mut game, crate::input::Joypad::RIGHT, 2000,
        |game| game.world().events.is_set(EVENT_SEAFOAM3_BOULDER1_DOWN_HOLE));
    let location = &game.world().location;
    assert!(location.is_hidden(TOGGLE_SEAFOAM_ISLANDS_B2F_BOULDER_1 as u8), "the boulder has gone down");
    assert!(!location.is_hidden(TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_3 as u8), "and is lying in B3F's water");

    lean(&mut game, crate::input::Joypad::RIGHT, 4000, |game| game.world().location.map == Map::SeafoamIslandsB3F);
    play_until(&mut game, 4000, free);
    assert_eq!(game.world().location.map, Map::SeafoamIslandsB3F, "the player fell down after it");
}

/// `Route20BoulderScript`: the first pass on Route 20 after the islands puts every boulder back
/// where it started. The events stay set, so the currents stay slowed and the puzzle cannot be
/// undone by pushing them down again.
#[test]
fn route_20_puts_the_seafoam_boulders_back_where_they_started() {
    use poke_core::map_objects::initial_toggleable_object_flags;
    use poke_core::symbols::pokered_toggles::*;
    let mut game = game(Map::Route20, 5, 5, SpriteFacing::Down, 5, |world| {
        for event in [EVENT_IN_SEAFOAM_ISLANDS, EVENT_SEAFOAM3_BOULDER1_DOWN_HOLE, EVENT_SEAFOAM3_BOULDER2_DOWN_HOLE,
            EVENT_SEAFOAM4_BOULDER1_DOWN_HOLE, EVENT_SEAFOAM4_BOULDER2_DOWN_HOLE] {
            world.events.set(event);
        }
        for toggle in [TOGGLE_SEAFOAM_ISLANDS_1F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_1F_BOULDER_2,
            TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_2] {
            hide(world, toggle);
        }
        for toggle in [TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_3, TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_4,
            TOGGLE_SEAFOAM_ISLANDS_B4F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B4F_BOULDER_2] {
            show(world, toggle);
        }
    });
    play_until(&mut game, 4000, free);
    let world = game.world();
    assert!(!world.events.is_set(EVENT_IN_SEAFOAM_ISLANDS), "the pass reads the event and clears it");
    assert_eq!(world.location.hidden_objects, initial_toggleable_object_flags(), "every boulder is back");
    assert!(world.events.is_set(EVENT_SEAFOAM3_BOULDER1_DOWN_HOLE), "and the currents stay slowed");
}

// ---- Saffron's dojo and its houses ----

/// The Karate Master answers a player who steps up beside him and fights them on the spot.
#[test]
fn the_karate_master_fights_whoever_steps_up_beside_him() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_FIGHTINGDOJO_KARATE_MASTER_POST_BATTLE;
    let mut game = game(Map::FightingDojo, 4, 4, SpriteFacing::Up, 5, |world| {
        // His four students are beaten, so nobody engages the player on the way up to him.
        for event in EVENT_BEAT_FIGHTING_DOJO_TRAINER_0..=EVENT_BEAT_FIGHTING_DOJO_TRAINER_3 {
            world.events.set(event);
        }
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    let state = &game.world().scripts.maps.fighting_dojo;
    assert_eq!(state.cur_script, SCRIPT_FIGHTINGDOJO_KARATE_MASTER_POST_BATTLE);
    assert_eq!(state.saved_coord_index, 1, "walked up to, not talked to");
}

/// Beaten, he marks his four students beaten too, so nobody left standing stops the walk to the
/// Poké Balls behind him.
#[test]
fn the_beaten_karate_master_marks_his_students_beaten_as_well() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_FIGHTINGDOJO_DEFAULT,
        SCRIPT_FIGHTINGDOJO_KARATE_MASTER_POST_BATTLE};
    let mut game = game(Map::FightingDojo, 4, 3, SpriteFacing::Right, 5, |world| {
        world.scripts.maps.fighting_dojo.cur_script = SCRIPT_FIGHTINGDOJO_KARATE_MASTER_POST_BATTLE;
        world.scripts.maps.fighting_dojo.saved_coord_index = 1;
    });
    play_until(&mut game, 20_000, |game| {
        game.world().scripts.maps.fighting_dojo.cur_script == SCRIPT_FIGHTINGDOJO_DEFAULT && free(game)
    });
    let world = game.world();
    assert!(world.events.is_set(EVENT_BEAT_KARATE_MASTER));
    assert!(world.events.is_set(EVENT_BEAT_FIGHTING_DOJO_TRAINER_0));
    assert!(world.events.is_set(EVENT_BEAT_FIGHTING_DOJO_TRAINER_3));
    assert!(!world.events.is_set(EVENT_DEFEATED_FIGHTING_DOJO), "not until a ball is taken");
}

/// One of the dojo's two Poké Balls, and only one: the mon comes at level 30 and the other ball has
/// nothing but "Better not get greedy" for whoever comes back for it.
#[test]
fn the_dojo_gives_up_one_of_its_two_fighting_mon() {
    use poke_core::symbols::pokered_toggles::{TOGGLE_FIGHTING_DOJO_GIFT_1, TOGGLE_FIGHTING_DOJO_GIFT_2};
    let beaten = |world: &mut World| {
        for event in EVENT_BEAT_KARATE_MASTER..=EVENT_BEAT_FIGHTING_DOJO_TRAINER_3 {
            world.events.set(event);
        }
    };
    let mut dojo = game(Map::FightingDojo, 4, 2, SpriteFacing::Up, 5, beaten);
    play_until(&mut dojo, 600, free);

    // NO leaves the ball where it is.
    command(&mut dojo, Command::Interact);
    play_answering(&mut dojo, 20_000, &mut vec![1], free);
    assert_eq!(dojo.world().party.len(), 1);
    assert!(!dojo.world().location.is_hidden(TOGGLE_FIGHTING_DOJO_GIFT_1 as u8));

    command(&mut dojo, Command::Interact);
    play_answering(&mut dojo, 20_000, &mut vec![0, 1], |game| game.world().party.len() == 2 && free(game));
    play_until(&mut dojo, 8000, free);
    let world = dojo.world();
    assert_eq!(world.party[1].mon.mon.species, PokemonSpecies::Hitmonlee);
    assert_eq!(world.party[1].mon.level, 30);
    assert!(world.location.is_hidden(TOGGLE_FIGHTING_DOJO_GIFT_1 as u8), "the ball is gone");
    assert!(world.events.is_set(EVENT_GOT_HITMONLEE) && world.events.is_set(EVENT_DEFEATED_FIGHTING_DOJO));
    assert!(world.pokedex.is_seen(PokemonSpecies::Hitmonlee), "the dex page it was offered on");

    let mut greedy = game(Map::FightingDojo, 5, 2, SpriteFacing::Up, 5, |world| {
        beaten(world);
        world.events.set(EVENT_GOT_HITMONLEE);
        world.events.set(EVENT_DEFEATED_FIGHTING_DOJO);
    });
    play_until(&mut greedy, 600, free);
    assert!(first_words(&mut greedy, "Better not get"));
    assert_eq!(greedy.world().party.len(), 1);
    assert!(!greedy.world().location.is_hidden(TOGGLE_FIGHTING_DOJO_GIFT_2 as u8), "still there");
}

/// The Copycat swaps TM31 for a Poké Doll, takes the doll, and has nothing to offer a player
/// without one.
#[test]
fn the_copycat_swaps_tm31_for_a_poke_doll() {
    use poke_core::symbols::pokered_map_scripts::TEXT_COPYCATSHOUSE2F_COPYCAT;
    let mut empty_handed = game(Map::CopycatsHouse2F, 4, 4, SpriteFacing::Up, 3, |_| {});
    play_until(&mut empty_handed, 600, free);
    wait_to_face_within(&mut empty_handed, TEXT_COPYCATSHOUSE2F_COPYCAT, 30_000);
    command(&mut empty_handed, Command::Interact);
    play_until(&mut empty_handed, 8000, free);
    assert_eq!(empty_handed.world().bag.quantity_of(ItemId::Tm31Mimic), 0, "nothing without a doll");

    let mut game = game(Map::CopycatsHouse2F, 4, 4, SpriteFacing::Up, 3, |world| {
        world.bag.add(ItemId::PokeDoll, 1);
    });
    play_until(&mut game, 600, free);
    wait_to_face_within(&mut game, TEXT_COPYCATSHOUSE2F_COPYCAT, 30_000);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::Tm31Mimic), 1);
    assert_eq!(game.world().bag.quantity_of(ItemId::PokeDoll), 0, "she keeps the doll");
    assert!(game.world().events.is_set(EVENT_GOT_TM31));

    wait_to_face_within(&mut game, TEXT_COPYCATSHOUSE2F_COPYCAT, 30_000);
    assert!(first_words(&mut game, "RED: Hi!"));
    assert_eq!(game.world().bag.quantity_of(ItemId::Tm31Mimic), 1, "once");
}

/// Her PC faces the wall, so it answers a player facing it and nobody else.
#[test]
fn the_copycats_pc_keeps_its_secrets_from_a_player_beside_it() {
    let pc = |x, y, facing| {
        let mut game = game(Map::CopycatsHouse2F, x, y, facing, 3, |_| {});
        play_until(&mut game, 600, free);
        game
    };
    assert!(first_words(&mut pc(0, 2, SpriteFacing::Up), "..."), "read from below");
    assert!(first_words(&mut pc(1, 1, SpriteFacing::Left), "Huh? Can't see!"), "read from beside it");
}

/// Mr Psychic hands over TM29 once, and explains it ever after.
#[test]
fn mr_psychic_hands_over_tm29_once() {
    let mut game = game(Map::MrPsychicsHouse, 4, 3, SpriteFacing::Right, 3, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::Tm29Psychic), 1);
    assert!(game.world().events.is_set(EVENT_GOT_TM29));

    assert!(first_words(&mut game, "TM29 is PSYCHIC!"));
    assert_eq!(game.world().bag.quantity_of(ItemId::Tm29Psychic), 1, "once");
}

/// The Copycat's Chansey and the Pidgey two doors along cry after their words.
#[test]
fn the_copycats_chansey_and_the_saffron_pidgey_cry_after_their_words() {
    use poke_core::symbols::pokered_map_scripts::{TEXT_COPYCATSHOUSE1F_CHANSEY, TEXT_SAFFRONPIDGEYHOUSE_PIDGEY};
    for (map, x, y, facing, text_id) in [
        (Map::CopycatsHouse1F, 2, 4, SpriteFacing::Left, TEXT_COPYCATSHOUSE1F_CHANSEY),
        (Map::SaffronPidgeyHouse, 1, 4, SpriteFacing::Left, TEXT_SAFFRONPIDGEYHOUSE_PIDGEY),
    ] {
        let mut game = game(map, x, y, facing, 3, |_| {});
        play_until(&mut game, 600, free);
        // Both pace up and down their column, so wait for the one in the column beside the player.
        wait_to_face(&mut game, text_id);
        command(&mut game, Command::Interact);
        let mut cried = false;
        play_until(&mut game, 5000, |game| {
            cried |= (4..8).any(|channel| game.audio().channel_sound_id(channel) != 0);
            free(game)
        });
        assert!(cried, "{map:?}");
    }
}


/// Every Silph Co floor draws its card key doors shut again on every load, so a door opened on one
/// visit is the only one that stays open. The stairwell between the fourth and fifth floors loads
/// each of them in turn, and each floor has its own block for a shut door.
#[test]
fn the_silph_co_floors_draw_their_card_key_doors_shut_on_every_load() {
    const FOURTH_FLOOR_DOOR: u8 = 0x54;
    const FIFTH_FLOOR_DOOR: u8 = 0x5F;
    let mut game = game(Map::SilphCo4F, 26, 1, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 3000, |game| game.world().location.map == Map::SilphCo5F && free(game));
    assert_eq!(block_at(&game, 3, 2), FIFTH_FLOOR_DOOR);
    assert_eq!(block_at(&game, 3, 6), FIFTH_FLOOR_DOOR);
    assert_eq!(block_at(&game, 7, 5), FIFTH_FLOOR_DOOR, "the fifth floor has three of them");
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 3000, |game| game.world().location.map == Map::SilphCo4F && free(game));
    assert_eq!(block_at(&game, 2, 6), FOURTH_FLOOR_DOOR);
    assert_eq!(block_at(&game, 6, 4), FOURTH_FLOOR_DOOR);
}

/// A gate answers to an event of its own, in the order the floor lists them: the fifth floor's third
/// gate is the third event's.
#[test]
fn a_silph_co_gate_its_own_event_remembers_stays_open() {
    const CLOSED_DOOR: u8 = 0x5F;
    let mut game = game(Map::SilphCo4F, 26, 1, SpriteFacing::Up, 5, |world| {
        one_shot(world);
        world.events.set(EVENT_SILPH_CO_5_UNLOCKED_DOOR3);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 3000, |game| game.world().location.map == Map::SilphCo5F && free(game));
    assert_ne!(block_at(&game, 7, 5), CLOSED_DOOR, "the gate its event remembers");
    assert_eq!(block_at(&game, 3, 2), CLOSED_DOOR, "the other two are still shut");
    assert_eq!(block_at(&game, 3, 6), CLOSED_DOOR);
}

/// The sixth floor has one gate and one event for it.
#[test]
fn the_silph_co_6f_card_key_door_is_drawn_shut_on_every_load() {
    const CLOSED_DOOR: u8 = 0x5F;
    let mut game = game(Map::SilphCo5F, 24, 1, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 3000, |game| game.world().location.map == Map::SilphCo6F && free(game));
    assert_eq!(block_at(&game, 2, 6), CLOSED_DOOR);
}

/// The seventh floor has three gates in one block, and only the events keep one open.
#[test]
fn the_silph_co_7f_card_key_doors_are_drawn_shut_until_their_events_are_set() {
    const CLOSED_DOOR: u8 = 0x54;
    let mut shut = game(Map::SilphCo8F, 14, 1, SpriteFacing::Up, 5, one_shot);
    play_until(&mut shut, 600, free);
    command(&mut shut, Command::Step(Direction::Up));
    play_until(&mut shut, 3000, |game| game.world().location.map == Map::SilphCo7F && free(game));
    assert_eq!(block_at(&shut, 5, 3), CLOSED_DOOR);
    assert_eq!(block_at(&shut, 10, 2), CLOSED_DOOR);
    assert_eq!(block_at(&shut, 10, 6), CLOSED_DOOR);

    let mut opened = game(Map::SilphCo8F, 14, 1, SpriteFacing::Up, 5, |world| {
        one_shot(world);
        world.events.set(EVENT_SILPH_CO_7_UNLOCKED_DOOR2);
    });
    play_until(&mut opened, 600, free);
    command(&mut opened, Command::Step(Direction::Up));
    play_until(&mut opened, 3000, |game| game.world().location.map == Map::SilphCo7F && free(game));
    assert_ne!(block_at(&opened, 10, 2), CLOSED_DOOR, "the gate its event remembers");
    assert_eq!(block_at(&opened, 5, 3), CLOSED_DOOR, "the other two are still shut");
    assert_eq!(block_at(&opened, 10, 6), CLOSED_DOOR);
}

/// The eighth and tenth floors have one gate each, in the block their own floor is tiled with.
#[test]
fn the_silph_co_8f_and_10f_card_key_doors_are_drawn_shut_on_every_load() {
    let mut eighth = game(Map::SilphCo7F, 16, 1, SpriteFacing::Up, 5, one_shot);
    play_until(&mut eighth, 600, free);
    command(&mut eighth, Command::Step(Direction::Up));
    play_until(&mut eighth, 3000, |game| game.world().location.map == Map::SilphCo8F && free(game));
    assert_eq!(block_at(&eighth, 3, 4), 0x5F);

    let mut tenth = game(Map::SilphCo9F, 14, 1, SpriteFacing::Up, 5, one_shot);
    play_until(&mut tenth, 600, free);
    command(&mut tenth, Command::Step(Direction::Up));
    play_until(&mut tenth, 3000, |game| game.world().location.map == Map::SilphCo10F && free(game));
    assert_eq!(block_at(&tenth, 5, 4), 0x54);
}

/// The ninth floor's four gates are not all tiled alike: the two in the middle of the floor are drawn
/// shut with the other floors' block and the two on its edges with their own.
#[test]
fn the_silph_co_9f_card_key_doors_are_drawn_shut_in_two_blocks() {
    let mut game = game(Map::SilphCo8F, 16, 1, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 3000, |game| game.world().location.map == Map::SilphCo9F && free(game));
    assert_eq!(block_at(&game, 1, 4), 0x5F);
    assert_eq!(block_at(&game, 9, 2), 0x54);
    assert_eq!(block_at(&game, 9, 5), 0x54);
    assert_eq!(block_at(&game, 5, 6), 0x5F);
}

/// The worker on the seventh floor parts with his Lapras once, and has something else to say after.
#[test]
fn the_silph_co_7f_worker_hands_over_his_lapras_once() {
    let mut game = game(Map::SilphCo7F, 1, 6, SpriteFacing::Up, 5, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, |game| game.world().scripts.got_lapras && free(game));
    assert_eq!(game.world().party.len(), 2);
    assert_eq!(game.world().party[1].mon.mon.species, PokemonSpecies::Lapras);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, free);
    assert_eq!(game.world().party.len(), 2, "he has only the one");
}

/// The rival is waiting in the middle of the seventh floor: standing on either of the two squares in
/// front of him walks him up and the battle is on.
#[test]
fn the_silph_co_7f_rival_walks_up_and_starts_the_battle() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_SILPHCO7F_RIVAL_AFTER_BATTLE;
    let mut game = game(Map::SilphCo7F, 3, 2, SpriteFacing::Down, 5, |world| {
        world.scripts.rival_starter = PokemonSpecies::Squirtle as u8;
    });
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    let state = &game.world().scripts.maps.silph_co_7f;
    assert_eq!(state.cur_script, SCRIPT_SILPHCO7F_RIVAL_AFTER_BATTLE);
    assert_eq!(state.saved_coord_index, 1, "the upper of the two squares");
}

/// Beaten, he wishes the player luck and leaves the building by the stairwell beside him, taking the
/// floor's script back to its first entry with him.
#[test]
fn the_silph_co_7f_rival_leaves_the_building_once_he_is_beaten() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_SILPHCO7F_DEFAULT, SCRIPT_SILPHCO7F_RIVAL_AFTER_BATTLE};
    use poke_core::symbols::pokered_toggles::TOGGLE_SILPH_CO_7F_RIVAL;
    let mut game = game(Map::SilphCo7F, 3, 2, SpriteFacing::Down, 5, |world| {
        world.scripts.maps.silph_co_7f.cur_script = SCRIPT_SILPHCO7F_RIVAL_AFTER_BATTLE;
        world.scripts.maps.silph_co_7f.saved_coord_index = 1;
    });
    play_until(&mut game, 40_000, |game| {
        game.world().scripts.maps.silph_co_7f.cur_script == SCRIPT_SILPHCO7F_DEFAULT && free(game)
    });
    let world = game.world();
    assert!(world.events.is_set(EVENT_BEAT_SILPH_CO_RIVAL));
    assert!(world.location.is_hidden(TOGGLE_SILPH_CO_7F_RIVAL as u8));
}

/// The nurse on the ninth floor heals the party for as long as Team Rocket hold the building, and
/// only thanks the player once they are gone.
#[test]
fn the_silph_co_9f_nurse_heals_the_party_until_giovanni_is_beaten() {
    let hurt = |world: &mut World| world.party[0].mon.mon.hp = 1;
    let mut held = game(Map::SilphCo9F, 3, 15, SpriteFacing::Up, 5, hurt);
    play_until(&mut held, 600, free);
    command(&mut held, Command::Interact);
    play_until(&mut held, 8000, free);
    let party = &held.world().party[0].mon;
    assert_eq!(party.mon.hp, party.stats[0], "HealParty");

    let mut freed = game(Map::SilphCo9F, 3, 15, SpriteFacing::Up, 5, |world| {
        hurt(world);
        world.events.set(EVENT_BEAT_SILPH_CO_GIOVANNI);
    });
    play_until(&mut freed, 600, free);
    command(&mut freed, Command::Interact);
    play_until(&mut freed, 8000, free);
    assert_eq!(freed.world().party[0].mon.mon.hp, 1, "the free heal goes with Team Rocket");
}

/// The elevator's own two warps lead nowhere: its load-time script aims both of them back at the
/// warp the player stepped in through, so leaving without choosing a floor goes back out again.
#[test]
fn the_silph_co_elevator_doors_lead_back_out_until_a_floor_is_chosen() {
    let mut game = game(Map::SilphCo4F, 20, 1, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 3000, |game| game.world().location.map == Map::SilphCoElevator && free(game));
    command(&mut game, Command::Step(Direction::Down));
    play_until(&mut game, 3000, |game| game.world().location.map == Map::SilphCo4F && free(game));
    assert_eq!((game.world().location.x, game.world().location.y), (20, 1), "back at the elevator's door");
}

/// A floor chosen at the panel aims both doors at it, and the elevator shakes on its way there.
/// The panel is read from the corner opposite the doors.
#[test]
fn the_silph_co_elevator_takes_the_player_to_the_floor_chosen() {
    let mut game = game(Map::SilphCo4F, 20, 1, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 3000, |game| game.world().location.map == Map::SilphCoElevator && free(game));
    for step in [Direction::Up, Direction::Up, Direction::Right, Direction::Right] {
        command(&mut game, Command::Step(step));
        play_until(&mut game, 200, free);
    }
    command(&mut game, Command::Face(Direction::Up));
    play_until(&mut game, 200, free);
    assert_eq!((game.world().location.x, game.world().location.y), (3, 1), "below the panel");
    command(&mut game, Command::Interact);
    play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::List));
    // The sixth row is the sixth floor.
    command(&mut game, Command::ChooseListEntry(5));
    play_until(&mut game, 20_000, free);
    for step in [Direction::Left, Direction::Down, Direction::Down] {
        command(&mut game, Command::Step(step));
        play_until(&mut game, 200, free);
    }
    command(&mut game, Command::Step(Direction::Down));
    play_until(&mut game, 3000, |game| game.world().location.map != Map::SilphCoElevator && free(game));
    assert_eq!(game.world().location.map, Map::SilphCo6F);
}

/// The mart's lift is the Silph one's shape over five buttons: the floor chosen is where both its
/// doors then lead, and the roof is not on the panel at all.
#[test]
fn the_celadon_mart_elevator_takes_the_player_to_the_floor_chosen() {
    let mut game = game(Map::CeladonMart3F, 1, 2, SpriteFacing::Up, 5, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 3000, |game| game.world().location.map == Map::CeladonMartElevator && free(game));
    for step in [Direction::Up, Direction::Up, Direction::Right, Direction::Right] {
        command(&mut game, Command::Step(step));
        play_until(&mut game, 200, free);
    }
    command(&mut game, Command::Face(Direction::Up));
    play_until(&mut game, 200, free);
    assert_eq!((game.world().location.x, game.world().location.y), (3, 1), "below the panel");
    command(&mut game, Command::Interact);
    play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::List));
    // The fifth row is the fifth floor, which the stairs alone cannot reach from here.
    command(&mut game, Command::ChooseListEntry(4));
    play_until(&mut game, 20_000, free);
    for step in [Direction::Left, Direction::Down, Direction::Down] {
        command(&mut game, Command::Step(step));
        play_until(&mut game, 200, free);
    }
    command(&mut game, Command::Step(Direction::Down));
    play_until(&mut game, 3000, |game| game.world().location.map != Map::CeladonMartElevator && free(game));
    assert_eq!(game.world().location.map, Map::CeladonMart5F);
}

/// The lift's doorway is warp carpet rather than a door tile, so a step onto it only stands on it and
/// the warp waits for a second press into the doorway, which is a collision no `Step` is accepted for.
fn enter_hideout_lift(game: &mut Game) {
    command(game, Command::Step(Direction::Down));
    play_until(game, 300, free);
    for _ in 0..60 {
        if game.world().location.map == Map::RocketHideoutElevator {
            break;
        }
        game.frame(Input::Buttons(crate::input::Joypad::DOWN));
    }
    play_until(game, 3000, |game| game.world().location.map == Map::RocketHideoutElevator && free(game));
}

/// The lift's own two warps lead to the floor it is parked on: its load-time script aims both of them
/// back at the doorway walked in through, so leaving without choosing a floor goes back out again.
#[test]
fn the_rocket_hideout_lift_doors_lead_back_out_until_a_floor_is_chosen() {
    let mut game = game(Map::RocketHideoutB1F, 24, 18, SpriteFacing::Down, 5, one_shot);
    play_until(&mut game, 600, free);
    enter_hideout_lift(&mut game);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 3000, |game| game.world().location.map == Map::RocketHideoutB1F && free(game));
    assert_eq!((game.world().location.x, game.world().location.y), (24, 19), "back at the lift's doorway");
}

/// Without the Lift Key the panel says so and the floor list never opens. The panel is read from the
/// square the doors let out onto.
#[test]
fn the_rocket_hideout_elevator_panel_needs_the_lift_key() {
    let mut game = game(Map::RocketHideoutElevator, 2, 1, SpriteFacing::Left, 5, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    let mut said_so = false;
    for _ in 0..3000 {
        assert_ne!(game.status(), Status::Waiting(Decision::List), "the panel stayed shut");
        if game.status() == Status::Waiting(Decision::Text) {
            said_so = true;
            game.frame(Input::Command(Command::Advance));
            continue;
        }
        if said_so && free(&game) {
            break;
        }
        game.frame(Input::None);
    }
    assert!(said_so, "the panel appears to need a key");
}

/// With the key the panel opens onto the three floors it stops at: B3F, where the key is guarded, has
/// no button. The key is not spent choosing a floor.
#[test]
fn the_rocket_hideout_elevator_takes_the_player_to_the_floor_chosen() {
    let mut game = game(Map::RocketHideoutElevator, 2, 1, SpriteFacing::Left, 5, |world| {
        world.bag.add(ItemId::LiftKey, 1);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::List));
    // The second row is B2F.
    command(&mut game, Command::ChooseListEntry(1));
    play_until(&mut game, 20_000, free);
    command(&mut game, Command::Step(Direction::Down));
    play_until(&mut game, 600, free);
    command(&mut game, Command::Step(Direction::Up));
    play_until(&mut game, 3000, |game| game.world().location.map != Map::RocketHideoutElevator && free(game));
    assert_eq!(game.world().location.map, Map::RocketHideoutB2F);
    assert_eq!(game.world().bag.quantity_of(ItemId::LiftKey), 1);
}

// ---- Cinnabar's lab, Fuchsia's Good Rod, Pewter's museum and the Power Plant ----

/// The gramps in the lab's trade room swaps his Electrode for a Raichu.
#[test]
fn the_cinnabar_lab_gramps_trades_an_electrode_for_a_raichu() {
    let mut game = game(Map::CinnabarLabTradeRoom, 1, 5, SpriteFacing::Up, 3, |world| {
        world.party.push(mon(PokemonSpecies::Raichu, 20));
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    for _ in 0..8000 {
        if free(&game) && game.world().in_game_trades != 0 {
            break;
        }
        let input = match game.status() {
            Status::Waiting(Decision::Text) => Input::Command(Command::Advance),
            Status::Waiting(Decision::TwoOption) => Input::Command(Command::ChooseOption(0)),
            Status::Waiting(Decision::PartyMenu) => Input::Command(Command::ChooseOption(1)),
            _ => Input::None,
        };
        game.frame(input);
    }
    assert_eq!(game.world().party[1].mon.mon.species, PokemonSpecies::Electrode);
}

/// The metronome room's scientist hands over TM35 once, and explains the move after.
#[test]
fn the_cinnabar_lab_scientist_hands_over_tm35_once() {
    let mut game = game(Map::CinnabarLabMetronomeRoom, 7, 3, SpriteFacing::Up, 3, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::Tm35Metronome), 1);
    assert!(game.world().events.is_set(EVENT_GOT_TM35));

    command(&mut game, Command::Interact);
    play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::Text));
    assert!(on_screen(&game, "Tch-tch-tch!"));
    play_until(&mut game, 8000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::Tm35Metronome), 1, "once");
}

/// The Good Rod goes only to a player who says they like to fish, and only once.
#[test]
fn the_good_rod_guru_gives_his_rod_to_a_player_who_likes_to_fish() {
    let mut game = game(Map::FuchsiaGoodRodHouse, 4, 3, SpriteFacing::Right, 3, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_answering(&mut game, 8000, &mut vec![1], free);
    assert_eq!(game.world().bag.quantity_of(ItemId::GoodRod), 0, "not to a player who said no");
    assert!(!game.world().scripts.got_good_rod);

    command(&mut game, Command::Interact);
    play_answering(&mut game, 8000, &mut vec![0], free);
    assert_eq!(game.world().bag.quantity_of(ItemId::GoodRod), 1);
    assert!(game.world().scripts.got_good_rod);

    command(&mut game, Command::Interact);
    play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::Text));
    assert!(on_screen(&game, "Hello there,"));
    play_answering(&mut game, 8000, &mut vec![0], free);
    assert_eq!(game.world().bag.quantity_of(ItemId::GoodRod), 1, "once");
}

/// The museum's doorway asks for the fee on its own, takes ¥50 from a player who agrees, and stops
/// asking once the ticket is bought.
#[test]
fn the_museum_doorway_takes_50_for_a_ticket_and_then_stops_asking() {
    use poke_core::symbols::pokered_map_scripts::SCRIPT_MUSEUM1F_NOOP;
    let mut game = game(Map::Museum1F, 10, 4, SpriteFacing::Up, 3, |world| world.money = [0x00, 0x01, 0x00]);
    play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::TwoOption));
    play_answering(&mut game, 8000, &mut vec![0], free);
    assert!(game.world().events.is_set(EVENT_BOUGHT_MUSEUM_TICKET));
    assert_eq!(game.world().money, [0x00, 0x00, 0x50]);
    assert_eq!(game.world().scripts.maps.museum_1f.cur_script, SCRIPT_MUSEUM1F_NOOP);
    // The noop script leaves the doorway alone from here on.
    play_until(&mut game, 600, free);
    assert!(free(&game));
}

/// A player who will not pay, or cannot, is told to come again and walked back out of the doorway.
#[test]
fn the_museum_doorway_walks_out_a_player_who_does_not_pay() {
    let turned_away = |money: [u8; 3], answer: u8| {
        let mut game = game(Map::Museum1F, 10, 4, SpriteFacing::Up, 3, |world| world.money = money);
        play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::TwoOption));
        play_answering(&mut game, 8000, &mut vec![answer], |game| free(game) && game.world().location.y != 4);
        game
    };
    let refused = turned_away([0x00, 0x01, 0x00], 1);
    assert!(!refused.world().events.is_set(EVENT_BOUGHT_MUSEUM_TICKET));
    assert_eq!(refused.world().money, [0x00, 0x01, 0x00]);
    assert_eq!((refused.world().location.x, refused.world().location.y), (10, 5));

    let broke = turned_away([0x00, 0x00, 0x49], 0);
    assert!(!broke.world().events.is_set(EVENT_BOUGHT_MUSEUM_TICKET));
    assert_eq!(broke.world().money, [0x00, 0x00, 0x49]);
}

/// Spoken to from behind his counter the scientist gives up on the fee and talks about amber
/// instead: yes earns the tip about the lab, no the definition.
#[test]
fn the_museum_counter_talks_about_amber_to_a_player_who_sneaked_round_it() {
    let asked = |answer: u8| {
        let mut game = game(Map::Museum1F, 13, 4, SpriteFacing::Left, 3, |_| {});
        play_until(&mut game, 600, free);
        command(&mut game, Command::Interact);
        play_until(&mut game, 4000, |game| game.status() == Status::Waiting(Decision::TwoOption));
        game.frame(Input::Command(Command::ChooseOption(answer)));
        play_until(&mut game, 4000, |game| game.status() == Status::Waiting(Decision::Text));
        game
    };
    assert!(on_screen(&asked(0), "There's a lab"));
    assert!(on_screen(&asked(1), "AMBER is fossil-"));
    assert!(!asked(0).world().events.is_set(EVENT_BOUGHT_MUSEUM_TICKET), "no fee taken back there");
}

/// The scientist at the back hands over the Old Amber once, and its display case goes with it.
#[test]
fn the_museum_scientist_hands_over_the_old_amber_once() {
    use poke_core::symbols::pokered_toggles::TOGGLE_OLD_AMBER;
    let mut game = game(Map::Museum1F, 15, 3, SpriteFacing::Up, 3, |_| {});
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 8000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::OldAmber), 1);
    assert!(game.world().events.is_set(EVENT_GOT_OLD_AMBER));
    assert!(game.world().location.is_hidden(TOGGLE_OLD_AMBER as u8));

    command(&mut game, Command::Interact);
    play_until(&mut game, 2000, |game| game.status() == Status::Waiting(Decision::Text));
    assert!(on_screen(&game, "Ssh! Get the OLD"));
    play_until(&mut game, 8000, free);
    assert_eq!(game.world().bag.quantity_of(ItemId::OldAmber), 1, "once");
}

/// A Power Plant item ball that is really a Voltorb fights the player who opens it, and is gone
/// from the floor once it is beaten.
#[test]
fn a_power_plant_item_ball_that_is_a_voltorb_fights_when_it_is_opened() {
    use poke_core::symbols::pokered_toggles::TOGGLE_VOLTORB_1;
    let mut game = game(Map::PowerPlant, 9, 21, SpriteFacing::Up, 5, one_shot);
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    play_until(&mut game, 60_000, |game| game.world().events.is_set(EVENT_BEAT_POWER_PLANT_VOLTORB_0) && free(game));
    assert_eq!(game.world().scripts.maps.power_plant.cur_script, 0);
    assert!(game.world().location.is_hidden(TOGGLE_VOLTORB_1 as u8));
}

/// Zapdos fights whoever walks up to it, and its perch is empty after.
#[test]
fn zapdos_fights_the_player_who_walks_up_to_it() {
    use poke_core::symbols::pokered_toggles::TOGGLE_ZAPDOS;
    let mut game = game(Map::PowerPlant, 4, 10, SpriteFacing::Up, 5, |world| {
        one_shot(world);
        // Mewtwo's own first move is Swift, which a Zapdos at 50 outlasts.
        world.party[0].mon.mon.moves[0] = Some(poke_core::move_name::PokemonMoveName::Psychic);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    play_until(&mut game, 60_000, |game| game.world().events.is_set(EVENT_BEAT_ZAPDOS) && free(game));
    assert!(game.world().location.is_hidden(TOGGLE_ZAPDOS as u8));
}

// ---- Route 23, the underground paths and Cerulean Cave ----

/// Each of Route 23's gates asks for one badge from the row its guard stands on: without it the
/// player is walked a square back south, with it the gate is passed for good.
#[test]
fn a_route_23_gate_turns_back_a_player_without_its_badge() {
    use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROUTE23_DEFAULT, SCRIPT_ROUTE23_PLAYER_MOVING};
    /// `BIT_CASCADEBADGE`, which the southernmost gate at y 136 asks for.
    const CASCADE: u8 = 1;
    let stand_on_the_row = |badge: bool| {
        let mut game = game(Map::Route23, 7, 136, SpriteFacing::Up, 3, |world| {
            if badge {
                world.badges = 1 << CASCADE;
            }
        });
        play_until(&mut game, 8000, |game| {
            game.world().scripts.maps.route23.cur_script != SCRIPT_ROUTE23_DEFAULT
                || game.world().events.is_set(EVENT_PASSED_CASCADEBADGE_CHECK)
        });
        game
    };

    let mut turned_back = stand_on_the_row(false);
    assert_eq!(turned_back.world().scripts.maps.route23.cur_script, SCRIPT_ROUTE23_PLAYER_MOVING);
    play_until(&mut turned_back, 8000, |game| game.world().location.y == 137 && free(game));
    assert!(!turned_back.world().events.is_set(EVENT_PASSED_CASCADEBADGE_CHECK), "the gate is not passed");
    assert_eq!(turned_back.world().scripts.maps.route23.cur_script, SCRIPT_ROUTE23_DEFAULT, "and asks again");

    let mut waved_through = stand_on_the_row(true);
    play_until(&mut waved_through, 8000, free);
    assert!(waved_through.world().events.is_set(EVENT_PASSED_CASCADEBADGE_CHECK));
    assert_eq!(waved_through.world().location.y, 136, "the player stays where they are");
}

/// `Route23SetVictoryRoadBoulders`: the one boulder Victory Road's top two floors share is put back
/// on 3F, and both floors' switches forgotten, whenever the player comes out onto the road.
#[test]
fn coming_out_onto_route_23_puts_victory_roads_boulder_back_on_3f() {
    use poke_core::symbols::pokered_toggles::{TOGGLE_VICTORY_ROAD_2F_BOULDER, TOGGLE_VICTORY_ROAD_3F_BOULDER};
    let switches = [EVENT_VICTORY_ROAD_2_BOULDER_ON_SWITCH1, EVENT_VICTORY_ROAD_2_BOULDER_ON_SWITCH2,
        EVENT_VICTORY_ROAD_3_BOULDER_ON_SWITCH1, EVENT_VICTORY_ROAD_3_BOULDER_ON_SWITCH2];
    let mut game = game(Map::VictoryRoad1F, 9, 16, SpriteFacing::Down, 3, |world| {
        world.location.last_map = Map::Route23;
        for event in switches {
            world.events.set(event);
        }
        hide(world, TOGGLE_VICTORY_ROAD_3F_BOULDER);
        show(world, TOGGLE_VICTORY_ROAD_2F_BOULDER);
    });
    play_until(&mut game, 600, free);
    // The cave mouth is on the map's bottom row, so the warp wants a second press into it.
    lean(&mut game, crate::input::Joypad::DOWN, 8000, |game| game.world().location.map == Map::Route23);
    play_until(&mut game, 8000, free);
    let world = game.world();
    assert!(switches.into_iter().all(|event| !world.events.is_set(event)), "no switch is still held");
    assert!(!world.location.is_hidden(TOGGLE_VICTORY_ROAD_3F_BOULDER as u8), "the boulder is back on 3F");
    assert!(world.location.is_hidden(TOGGLE_VICTORY_ROAD_2F_BOULDER as u8), "and gone from 2F");
}

/// Whichever end of the path the player came in by, each stairwell's door leads out to its own route.
#[test]
fn each_underground_path_stairwell_holds_its_door_to_its_own_route() {
    for (map, route) in [(Map::UndergroundPathRoute6, Map::Route6), (Map::UndergroundPathRoute7, Map::Route7),
        (Map::UndergroundPathRoute8, Map::Route8)]
    {
        let mut game = game(map, 3, 5, SpriteFacing::Down, 3, |world| {
            world.location.last_map = Map::PalletTown;
        });
        play_until(&mut game, 600, free);
        assert_eq!(game.world().location.last_map, route, "{map:?}");
    }
}

/// Mewtwo fights whoever walks up to it, and its cave is empty after.
#[test]
fn mewtwo_fights_the_player_who_walks_up_to_it() {
    use poke_core::symbols::pokered_toggles::TOGGLE_MEWTWO;
    let mut game = game(Map::CeruleanCaveB1F, 27, 14, SpriteFacing::Up, 5, |world| {
        // A wild Mewtwo has Amnesia and Recover and cannot be worn down, so this one is settled by a
        // one-hit-KO move from a party fast enough to land it.
        world.party = vec![mon(PokemonSpecies::Mewtwo, 100)];
        world.party[0].mon.mon.moves[0] = Some(poke_core::move_name::PokemonMoveName::HornDrill);
    });
    play_until(&mut game, 600, free);
    command(&mut game, Command::Interact);
    play_until(&mut game, 20_000, |game| game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))));
    play_until(&mut game, 200_000, |game| game.world().events.is_set(EVENT_BEAT_MEWTWO) && free(game));
    assert!(game.world().location.is_hidden(TOGGLE_MEWTWO as u8));
    assert_eq!(game.world().scripts.maps.cerulean_cave_b1f.cur_script, 0);
}

