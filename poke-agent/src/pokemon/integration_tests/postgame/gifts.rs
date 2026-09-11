use super::super::*;

const FLY_BIKE: &[u8] = include_bytes!("../../data/postgame-fly-bike.bin");

/// Revive the Helix Fossil into an Omanyte at the Cinnabar Lab.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_revive_the_helix_fossil() {
    let mut fixture = TestFixture::new(FLY_BIKE, Duration::from_mins(20), PolicyStep::fossil_revival_steps());

    let before = fixture.game_state();
    assert!(before.bag.iter().any(|i| i.id == ItemId::HelixFossil), "entry fixture has no Helix Fossil");
    assert!(!before.pokedex_owned.contains(&PokemonSpecies::Omanyte), "entry fixture already owns an Omanyte");
    let party_before = before.pokemon.len();

    let state = fixture.run_leg(|s| s.pokemon.iter().any(|p| p.species == PokemonSpecies::Omanyte));

    assert!(!state.bag.iter().any(|i| i.id == ItemId::HelixFossil), "the fossil should have been handed over");
    assert_eq!(state.pokemon.len(), party_before + 1);
    assert!(state.pokedex_owned.contains(&PokemonSpecies::Omanyte));
    assert_eq!(state.map.map, Map::CinnabarIsland, "the leg ends outdoors so the next Fly is allowed");
    let omanyte = state.pokemon.iter().find(|p| p.species == PokemonSpecies::Omanyte).unwrap();
    println!("Omanyte lv{} · party {} · dex owned {}", omanyte.level, state.pokemon.len(), state.pokedex_owned.species().len());

    fixture.save_state_named("src/pokemon/data/postgame-omanyte.bin").unwrap();
}

/// `can_revive_the_helix_fossil`'s output: Cinnabar Island, Omanyte lv30 in the party.
const OMANYTE: &[u8] = include_bytes!("../../data/postgame-omanyte.bin");

/// The Old Amber from the Pewter Museum, revived into an Aerodactyl.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_the_old_amber_and_revive_it() {
    let mut fixture = TestFixture::new(OMANYTE, Duration::from_mins(30), PolicyStep::old_amber_steps());

    let before = fixture.game_state();
    assert!(!before.pokedex_owned.contains(&PokemonSpecies::Aerodactyl), "entry fixture already owns an Aerodactyl");
    let party_before = before.pokemon.len();

    let with_amber = fixture.run_until(|s| s.bag.iter().any(|i| i.id == ItemId::OldAmber));
    println!("Old Amber in the bag at {:?} — bag {} entries", with_amber.map.map, with_amber.bag.len());

    let state = fixture.run_leg(|s| s.pokemon.iter().any(|p| p.species == PokemonSpecies::Aerodactyl));

    assert!(!state.bag.iter().any(|i| i.id == ItemId::OldAmber), "the amber should have been handed over");
    assert_eq!(state.pokemon.len(), party_before + 1);
    assert_eq!(state.map.map, Map::CinnabarIsland, "the leg ends outdoors so the next Fly is allowed");
    let aerodactyl = state.pokemon.iter().find(|p| p.species == PokemonSpecies::Aerodactyl).unwrap();
    println!("Aerodactyl lv{} · party {} · dex owned {}", aerodactyl.level, state.pokemon.len(),
        state.pokedex_owned.species().len());

    fixture.save_state_named("src/pokemon/data/postgame-aerodactyl.bin").unwrap();
}

/// `can_get_the_old_amber_and_revive_it`'s output: Cinnabar Island, a party of six.
const AERODACTYL: &[u8] = include_bytes!("../../data/postgame-aerodactyl.bin");

/// The Lapras the rescued Silph employee holds goes to the box when the party is full.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn a_full_party_sends_the_silph_lapras_to_the_box() {
    let mut fixture = TestFixture::new(AERODACTYL, Duration::from_mins(30), PolicyStep::lapras_steps());

    let before = fixture.game_state();
    assert_eq!(before.pokemon.len(), 6, "this leg is about the *full* party branch");
    assert!(before.boxed_pokemon.is_empty(), "entry fixture's box 1 should be empty");
    assert!(!before.pokedex_owned.contains(&PokemonSpecies::Lapras));

    let state = fixture.run_leg(|s| !s.boxed_pokemon.is_empty());

    assert_eq!(state.pokemon.len(), 6, "the party should be untouched — the gift went to the box");
    assert_eq!(state.boxed_pokemon.len(), 1);
    assert_eq!(state.boxed_pokemon[0].species, PokemonSpecies::Lapras);
    assert!(state.pokedex_owned.contains(&PokemonSpecies::Lapras));
    assert_ne!(format!("{}", state.boxed_pokemon[0].nickname), "LAPRAS",
        "the box branch runs its own naming screen too — a default name means it was never driven");
    assert_eq!(state.map.map, Map::SaffronCity, "the leg ends outdoors so the next Fly is allowed");
    println!("Lapras \"{}\" lv{} in box 1 · party {} · dex owned {}", state.boxed_pokemon[0].nickname,
        state.boxed_pokemon[0].level,
        state.pokemon.len(), state.pokedex_owned.species().len());

    fixture.save_state_named("src/pokemon/data/postgame-lapras.bin").unwrap();
}

/// `a_full_party_sends_the_silph_lapras_to_the_box`'s output: Saffron City, Lapras in box 1.
const LAPRAS: &[u8] = include_bytes!("../../data/postgame-lapras.bin");

/// The Fighting Dojo: beat the Karate Master and take a Hitmonlee.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_beat_the_karate_master_and_take_a_hitmonlee() {
    /// Omanyte, dex-registered, so banking it costs nothing.
    const BANK_SLOT: u8 = 4;

    let mut fixture = TestFixture::new(LAPRAS, Duration::from_mins(45),
        PolicyStep::hitmonlee_steps(BANK_SLOT));

    let before = fixture.game_state();
    assert_eq!(before.pokemon[BANK_SLOT as usize].species, PokemonSpecies::Omanyte, "wrong mon banked");
    assert!(!before.pokedex_owned.contains(&PokemonSpecies::Hitmonlee));

    let state = fixture.run_leg(|s| s.pokemon.iter().any(|p| p.species == PokemonSpecies::Hitmonlee));

    assert_eq!(state.pokemon.len(), 6, "5 after banking Omanyte, 6 with Hitmonlee");
    assert_eq!(state.boxed_pokemon.len(), 2, "Lapras plus the banked Omanyte");
    assert!(state.pokedex_owned.contains(&PokemonSpecies::Hitmonlee));
    assert_eq!(state.map.map, Map::SaffronCity, "the leg ends outdoors so the next Fly is allowed");
    let hitmonlee = state.pokemon.iter().find(|p| p.species == PokemonSpecies::Hitmonlee).unwrap();
    println!("Hitmonlee \"{}\" lv{} · party {} · box {} · dex owned {}", hitmonlee.nickname,
        hitmonlee.level, state.pokemon.len(), state.boxed_pokemon.len(),
        state.pokedex_owned.species().len());

    fixture.save_state_named("src/pokemon/data/postgame-hitmonlee.bin").unwrap();
}

/// `can_beat_the_karate_master_and_take_a_hitmonlee`'s output: Saffron, Hitmonlee in the party.
const HITMONLEE: &[u8] = include_bytes!("../../data/postgame-hitmonlee.bin");

/// The five Silph floors the main quest skips, and everything left on them.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_clear_the_skipped_silph_floors() {
    const ALL: u8 = u8::MAX;
    /// Six entries nothing needs again: the S.S. Ticket, Lift Key, Silph Scope and three stacks.
    const BANK: &[(ItemId, u8)] = &[(ItemId::SSTicket, 1), (ItemId::LiftKey, 1), (ItemId::SilphScope, 1),
                                    (ItemId::GreatBall, ALL), (ItemId::Revive, ALL), (ItemId::FullRestore, ALL)];

    let mut fixture = TestFixture::new(HITMONLEE, Duration::from_mins(60),
        PolicyStep::silph_floors_steps(BANK));

    let before = fixture.game_state();
    let bag_before = before.bag.iter().count();
    for (item, _) in BANK {
        assert!(before.bag.iter().any(|i| i.id == *item), "entry fixture is missing {item:?}");
    }

    // 2F and 8F have nothing to pick up, so arrival is the observable.
    fixture.run_until(|s| s.map.map == Map::SilphCo2F);
    println!("reached SilphCo2F");

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::Carbos));

    for item in [ItemId::FullHeal, ItemId::MaxRevive, ItemId::EscapeRope, ItemId::HpUp,
                 ItemId::XAccuracy, ItemId::Calcium, ItemId::RareCandy, ItemId::Carbos] {
        assert!(state.bag.iter().any(|i| i.id == item), "{item:?} was never picked up");
    }
    for (item, _) in BANK {
        assert!(!state.bag.iter().any(|i| i.id == *item), "{item:?} should have been banked");
    }
    assert_eq!(state.map.map, Map::SaffronCity, "the leg ends outdoors so the next Fly is allowed");
    println!("bag {} → {} entries · {} banked", bag_before, state.bag.iter().count(), BANK.len());

    fixture.save_state_named("src/pokemon/data/postgame-silph-floors.bin").unwrap();
}

/// `can_clear_the_skipped_silph_floors`'s output: Saffron City, the ten Silph items taken.
const SILPH_FLOORS: &[u8] = include_bytes!("../../data/postgame-silph-floors.bin");

/// The two Saffron TM gifts, one of which has to be bought.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_collect_the_saffron_tm_gifts() {
    /// At 19/20 there is no room for the doll and two TMs, so two pickups nothing wants go back.
    const BANK: &[(ItemId, u8)] = &[(ItemId::HpUp, u8::MAX), (ItemId::XAccuracy, u8::MAX)];

    let mut fixture = TestFixture::new(SILPH_FLOORS, Duration::from_mins(45),
        PolicyStep::saffron_tm_gifts_steps(BANK));

    let before = fixture.game_state();
    let money_before = before.money;
    assert!(!before.bag.iter().any(|i| i.id == ItemId::PokeDoll), "entry fixture already has a Poké Doll");

    let with_doll = fixture.run_until(|s| s.bag.iter().any(|i| i.id == ItemId::PokeDoll));
    assert_eq!(money_before - with_doll.money, 1_000, "the Poké Doll is ¥1000");

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::Tm31Mimic));

    assert!(state.bag.iter().any(|i| i.id == ItemId::Tm29Psychic), "Mr. Psychic's TM29 never arrived");
    assert!(!state.bag.iter().any(|i| i.id == ItemId::PokeDoll), "the Copycat should have taken the doll");
    assert_eq!(state.map.map, Map::SaffronCity, "the leg ends outdoors so the next Fly is allowed");
    println!("TM29 + TM31 in the bag ({} entries) · ¥{} → ¥{}", state.bag.iter().count(),
        money_before, state.money);

    fixture.save_state_named("src/pokemon/data/postgame-gifts.bin").unwrap();
}

/// `can_collect_the_saffron_tm_gifts`'s output: Saffron City with TM29 and TM31.
const GIFTS: &[u8] = include_bytes!("../../data/postgame-gifts.bin");

/// Leave a Pokémon at the Day Care.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_leave_a_pokemon_at_the_day_care() {
    /// Hitmonlee, the one party member with no HM move, which the gentleman checks.
    const HM_FREE_SLOT: u8 = 5;
    let mut fixture = TestFixture::new(GIFTS, Duration::from_mins(45),
        PolicyStep::daycare_steps(HM_FREE_SLOT));

    let before = fixture.game_state();
    assert_eq!(before.pokemon.len(), 6);
    assert_eq!(before.pokemon[HM_FREE_SLOT as usize].species, PokemonSpecies::Hitmonlee);
    let money_before = before.money;
    let nicknames: Vec<String> = before.pokemon.iter()
        .map(|p| format!("{:?} \"{}\"", p.species, p.nickname)).collect();
    println!("party in: {}", nicknames.join(", "));

    let deposited = fixture.run_until(|s| s.pokemon.len() == 5);
    assert!(!deposited.pokemon.iter().any(|p| p.species == PokemonSpecies::Hitmonlee),
        "the deposited mon should have left the party");
    println!("deposited — party {} at {:?}", deposited.pokemon.len(), deposited.map.map);

    let state = fixture.run_leg(|s| s.pokemon.len() == 6);

    assert!(state.pokemon.iter().any(|p| p.species == PokemonSpecies::Hitmonlee), "it never came back");
    assert_eq!(money_before - state.money, 100, "¥100 × (levels grown + 1), and nothing grew");
    // Handing over slot 0 promotes the one behind it, so the Cut holder leads again.
    assert_eq!(state.pokemon[0].species, PokemonSpecies::Venusaur, "the Cut holder must lead again");
    assert_eq!(state.pokemon[5].species, PokemonSpecies::Hitmonlee, "a collected mon is appended");
    assert_eq!(state.map.map, Map::Route5, "the leg ends outdoors so the next Fly is allowed");
    println!("collected · ¥{money_before} → ¥{} · party {}", state.money, state.pokemon.len());

    fixture.save_state_named("src/pokemon/data/postgame-daycare.bin").unwrap();
}

/// The Day Care boards no Pokémon nobody chose, every time.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn talking_to_the_day_care_does_not_board_a_pokemon_nobody_chose() {
    /// Hitmonlee, the one party member with no HM move.
    const HM_FREE_SLOT: u8 = 5;

    let mut steps = PolicyStep::daycare_steps(HM_FREE_SLOT);
    // Everything up to the shuffle, then the row a model would take instead of the `PartyScript`.
    steps.truncate(1 + steps.iter().position(|step| matches!(step, PolicyStep::MovePokemonToFront { .. }))
        .expect("daycare_steps arranges the party before it deposits"));
    steps.extend(std::iter::repeat_n(PolicyStep::Interact(MapSprite::DAYCARE_GENTLEMAN), 3));

    let mut fixture = TestFixture::new(GIFTS, Duration::from_mins(45), steps);
    let money_before = fixture.game_state().money;
    fixture.run_until(|s| s.map.map == Map::Daycare);
    let party_before: Vec<_> = fixture.run_until(|s| s.pokemon[0].species == PokemonSpecies::Hitmonlee)
        .pokemon.iter().map(|mon| mon.species).collect();

    fixture.step_until_exhausted();
    // Long enough for a deposit to show.
    for _ in 0..600 { fixture.step(); }

    let state = fixture.game_state();
    let party_after: Vec<_> = state.pokemon.iter().map(|mon| mon.species).collect();
    assert_eq!(party_after, party_before,
        "the gentleman was handed a Pokémon nobody chose: {party_before:?} → {party_after:?}");
    assert_eq!(state.money, money_before, "nothing should have been paid for");
    assert_eq!(state.map.map, Map::Daycare, "the leg ends where it was talking");
    println!("day care declined · party {party_after:?} · ¥{}", state.money);
}

/// `can_leave_a_pokemon_at_the_day_care`'s output: Route 5, Hitmonlee back from the Day Care.
const DAYCARE: &[u8] = include_bytes!("../../data/postgame-daycare.bin");

/// The Name Rater, and the last three never-visited rooms.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_rename_a_pokemon_and_visit_the_last_rooms() {
    /// Articuno, the one member not already called what the picker draws first.
    const RENAME_SLOT: u8 = 1;

    let mut fixture = TestFixture::new(DAYCARE, Duration::from_mins(45),
        PolicyStep::name_rater_and_rooms_steps(RENAME_SLOT));

    let before = fixture.game_state();
    let name_of = |s: &GameState, i: usize| format!("{}", s.pokemon[i].nickname);
    assert_eq!(before.pokemon[RENAME_SLOT as usize].species, PokemonSpecies::Articuno,
        "the fixture's slot 1 moved");
    let target_before = name_of(&before, RENAME_SLOT as usize);
    let lead_before = name_of(&before, 0);
    assert_eq!(before.pokemon.iter().filter(|p| format!("{}", p.nickname) == target_before).count(), 1,
        "the renamed mon's name must be unique in the party or the rename is unobservable");

    let renamed = fixture.run_until(|s| name_of(s, RENAME_SLOT as usize) != target_before);
    assert_eq!(renamed.pokemon[RENAME_SLOT as usize].species, PokemonSpecies::Articuno,
        "the wrong mon was renamed");
    assert_eq!(name_of(&renamed, 0), lead_before,
        "slot 0 was renamed — the cursor was never driven off its stale position");
    println!("slot {RENAME_SLOT}: \"{target_before}\" → \"{}\"", name_of(&renamed, RENAME_SLOT as usize));

    for room in [Map::ViridianSchoolHouse, Map::CeladonHotel, Map::CeladonChiefHouse] {
        let state = fixture.run_until(|s| s.map.map == room);
        println!("visited {:?} @ {}", room, state.map.player_position);
    }

    let state = fixture.run_leg(|s| s.map.map == Map::CeladonCity);
    assert_eq!(state.pokemon.len(), 6, "nothing here should change the party");
    fixture.save_state_named("src/pokemon/data/postgame-name-rater.bin").unwrap();
}
