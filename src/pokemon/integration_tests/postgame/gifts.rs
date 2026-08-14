//! Tests for workstream `gifts` — see `docs/postgame-coverage-plan.md` §6-G and
//! [`crate::pokemon::postgame::gifts`].
//!
//! Rooted on **B's** output rather than `postgame-phase0.bin`: the gifts are in Cinnabar, Pewter,
//! Saffron and Celadon, so Fly is the difference between one step and a cross-Kanto walk — the same
//! reasoning C's and F's rows give.

#[allow(unused_imports)]
use super::super::*;

/// Workstream B's output (§9): Fuchsia City, Fly on Articuno, the Bicycle in the bag (16/20), party
/// Venusaur / Articuno / Vaporeon / Slowpoke, ¥41,209, dex 7 owned / 112 seen.
const FLY_BIKE: &[u8] = include_bytes!("../../data/postgame-fly-bike.bin");

/// **Task G1** — revive the **Helix Fossil** into an **Omanyte** at the Cinnabar Lab.
///
/// The agent has carried that fossil since Mt Moon and has never handed it over. The mechanic is a
/// two-visit one — the scientist takes it, then wants you to "go for a walk", and the walk is
/// literally `CinnabarIsland_Script` running once (`scripts/CinnabarIsland.asm:6` resets
/// `EVENT_LAB_STILL_REVIVING_FOSSIL`), so the leg leaves the building and comes back.
///
/// Assertions are on both ends of the trade: the fossil is a key item, so its *disappearance* proves
/// the bespoke fossil-choice menu was driven rather than cancelled, and the Omanyte proves the second
/// visit landed. ~1 min of game time.
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

/// G1's output: Cinnabar Island, Omanyte lv30 in the party (5), the Helix Fossil spent.
const OMANYTE: &[u8] = include_bytes!("../../data/postgame-omanyte.bin");

/// **Task G2** — the **Old Amber** out of the Pewter Museum, revived into an **Aerodactyl**.
///
/// The one species in the game behind a *building* the agent has never opened. Two observables in
/// one leg, checked in order: the amber lands in the bag (the museum half), then the party gains an
/// Aerodactyl (the lab half, which is G1's two-visit mechanic again with the other fossil).
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

/// G2's output: Cinnabar Island, party **6** (Venusaur / Articuno / Vaporeon / Slowpoke / Omanyte /
/// Aerodactyl), both fossils spent, dex 9 owned.
const AERODACTYL: &[u8] = include_bytes!("../../data/postgame-aerodactyl.bin");

/// **Task G3** — the **Lapras** the rescued Silph employee has been holding all along.
///
/// Deliberately run with a **full party**, which makes this the other half of the gift path: with no
/// room, `_GivePokemon` takes the `SendNewMonToBox` branch instead of `AddPartyMon`. F proved the
/// party branch by driving the naming screen for a prize Abra; this drives the branch F only read.
///
/// ❗ F's §11 entry says the box branch "skips the naming entirely". It does not —
/// `SendNewMonToBox` ends with its own `predef AskName` (`engine/items/item_effects.asm:2731-2733`),
/// so **both** branches name, and the nickname assertion below is what pins that: a default name
/// would mean the screen never ran and the agent got lucky.
///
/// ⚠️ **It has since become the guard for a second thing, and it is the only test in the suite that
/// can be.** The nickname prompt is a yes/no, and it arrives moments after the Silph Co lift has
/// left `wTextBoxID` reading `ListMenuBox` — so this is the one place where a menu-shape test that
/// believes a *lingering* id, on a rule that runs at every text box, answers B and silently declines
/// the nickname. That is exactly what the first draft of `MENU_HANDOVER_TICKS` did. If this fails
/// with two identical `LAPRAS` strings, look at `MenuEvidence` before anything else.
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

/// Diagnostic for **G3**: what does each of Silph 7F's three arrival points actually reach?
///
/// 7F has six warps — the lift at (18,0), stairs to 6F/8F, and three teleport pads — and the floor is
/// cut into pockets by them, exactly like Victory Road 2F was for D. Kept `#[ignore]`d in the tree
/// because "which pocket is the Lapras worker in" is the only question this leg has and it is worth
/// being able to re-ask it.
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_silph_7f_pockets() {
    let dump = |fixture: &mut TestFixture, label: &str| {
        for _ in 0..50 { fixture.step(); }
        let state = fixture.game_state();
        println!("== {label}: {} @ {}", state.map.map, state.map.player_position);
        for action in state.map.actions() {
            println!("   {:?} @ {} ({} steps)", action.tile, action.destination, action.route.len());
        }
    };

    // 1. The lift door at (18,0) — what `lapras_steps` tried first.
    let mut fixture = TestFixture::new(AERODACTYL, Duration::from_mins(60), vec![
        PolicyStep::Fly { to: Map::SaffronCity },
        PolicyStep::enter(Map::SilphCo1F),
        PolicyStep::enter(Map::SilphCoElevator),
        PolicyStep::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 6 },
    ]);
    fixture.run_until(|s| s.map.map == Map::SilphCo7F);
    dump(&mut fixture, "arrived by lift");

    // 2. The rival pocket, reached down 3F's (11,11) pad — the route `silph_giovanni_steps` threads.
    let mut fixture = TestFixture::new(AERODACTYL, Duration::from_mins(60), vec![
        PolicyStep::Fly { to: Map::SaffronCity },
        PolicyStep::enter(Map::SilphCo1F),
        PolicyStep::enter(Map::SilphCoElevator),
        PolicyStep::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 2 },
        PolicyStep::EnterMap { to_map: Map::SilphCo7F, to_position: Some(Point8 { x: 5, y: 3 }) },
    ]);
    fixture.run_until(|s| s.map.map == Map::SilphCo7F);
    dump(&mut fixture, "arrived by the 3F pad");
}

/// G3's output: Saffron City, party 6, **Lapras in box 1**, dex 10 owned.
const LAPRAS: &[u8] = include_bytes!("../../data/postgame-lapras.bin");

/// **Task G4** — the **Fighting Dojo**: beat the Karate Master, take a **Hitmonlee**.
///
/// The dojo is a whole room of the game the agent has never entered, and the reward is the plan's
/// only *choice* — the two Poké Balls at the back are mutually exclusive, so Hitmonchan is gone for
/// this cartridge the moment this leg passes. The counterpart to G3: a slot is banked first so the
/// gift takes the `AddPartyMon` branch and the party count is what moves.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_beat_the_karate_master_and_take_a_hitmonlee() {
    /// Omanyte, G1's trophy — dex-registered, so banking it costs nothing.
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

/// G4's output: Saffron City, party 6 with Hitmonlee, box 1 holding Lapras + Omanyte, bag 15/20,
/// dex 11 owned.
const HITMONLEE: &[u8] = include_bytes!("../../data/postgame-hitmonlee.bin");

/// Diagnostic for **G7**: what does each Silph floor's *lift landing* reach?
///
/// 7F taught this workstream that a Silph floor is not one room (see [`probe_silph_7f_pockets`]), so
/// the same question has to be asked of every floor before writing a pickup route: are the item balls
/// in the pocket the elevator opens onto, or behind a teleport pad? Menu index = floor − 1.
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_silph_item_floors() {
    for (floor, index) in [("2F", 1u8), ("4F", 3), ("6F", 5), ("8F", 7), ("10F", 9)] {
        let mut fixture = TestFixture::new(HITMONLEE, Duration::from_mins(60), vec![
            PolicyStep::Fly { to: Map::SaffronCity },
            PolicyStep::enter(Map::SilphCo1F),
            PolicyStep::enter(Map::SilphCoElevator),
            PolicyStep::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: index },
        ]);
        fixture.run_until(|s| s.map.map != Map::SilphCoElevator && s.map.map != Map::SilphCo1F
            && s.map.map != Map::SaffronCity);
        for _ in 0..50 { fixture.step(); }
        let state = fixture.game_state();
        println!("== {floor} (menu index {index}): {} @ {}", state.map.map, state.map.player_position);
        for action in state.map.actions() {
            println!("   {:?} @ {} ({} steps)", action.tile, action.destination, action.route.len());
        }
    }
}

/// **Task G7** — the five Silph floors the main quest skips, and everything left on them.
///
/// Ten item balls against **five** free bag slots, which is what makes this more than a walk: Phase
/// 0's item PC is used first to bank six dead entries, three of them **key items** (the S.S. Ticket,
/// Lift Key and Silph Scope are all spent). That composition — `deposit_item` feeding `CollectItem`
/// — is the only thing in this workstream that needed the bag to be managed at all, and it is the
/// scenario §2 calls the plan's founding blocker.
///
/// 2F and 8F carry no items, so they are asserted by *arrival*: they are two of the 96 never-visited
/// maps and riding to them is the coverage. ~4 min of game time, most of it Rocket trainers.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_clear_the_skipped_silph_floors() {
    /// Six entries nothing needs again: the S.S. Anne has sailed, the Rocket Hideout lift and the
    /// Pokémon Tower are done, and the party out-levels anything on these floors by forty. The
    /// The quantity must be the **whole stack** — a bag *slot* is freed only when its last unit
    /// leaves, and freeing slots is the only thing this leg cares about. `ALL` overshoots on purpose:
    /// `ItemPcState::new` clamps to what is actually held, so the leg does not have to know or track
    /// the counts (and it did not — nine Great Balls had become eight; see §11).
    const ALL: u8 = u8::MAX;
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

/// G7's output: Saffron City, bag 19/20 with the ten Silph items, PC storage holding the six banked
/// entries, party healed.
const SILPH_FLOORS: &[u8] = include_bytes!("../../data/postgame-silph-floors.bin");

/// **Task G8a** — the two Saffron TM gifts, one of which has to be *bought*.
///
/// `MrPsychicsHouse` hands over TM29 for nothing. The Copycat wants a **Poké Doll**, and her script
/// is the trap: with no doll it prints one text box and ends, identical from the outside to a
/// successful conversation. So the leg buys the doll at `CeladonMart4F` first and asserts **TM31**,
/// which cannot arrive any other way. Two more never-visited maps (`CopycatsHouse1F/2F`,
/// `MrPsychicsHouse`) fall out of it. ~3 min of game time.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_collect_the_saffron_tm_gifts() {
    /// Two of G7's own pickups go straight back into storage: at 19/20 there is not room for the
    /// doll *and* two TMs, and these are the two nothing else in the plan wants.
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

/// G8a's output: Saffron City with TM29 + TM31, ¥44,384.
const GIFTS: &[u8] = include_bytes!("../../data/postgame-gifts.bin");

/// **Task G8b** — the **Day Care**, the last unexercised mechanic in G.
///
/// Deposit a mon, collect it, pay. The two things that make it work without a party-menu driver are
/// in [`PolicyStep::daycare_steps`]: the lead is what an A-mash deposits (so the HM-carrying Venusaur
/// has to step aside first, or the gentleman refuses), and a *second* conversation would collect the
/// mon straight back — which is why this is the one leg in the file that queues a single `Interact`.
///
/// Money is the assertion that matters: the party count returning to six proves the mon came back,
/// but only the ¥100 proves it came back through the counter rather than never having left.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_leave_a_pokemon_at_the_day_care() {
    /// Hitmonlee — the one party member with no HM move, which is what the gentleman checks.
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
    // Handing over slot 0 promotes the mon behind it, so the Cut holder leads again for free.
    assert_eq!(state.pokemon[0].species, PokemonSpecies::Venusaur, "the Cut holder must lead again");
    assert_eq!(state.pokemon[5].species, PokemonSpecies::Hitmonlee, "a collected mon is appended");
    assert_eq!(state.map.map, Map::Route5, "the leg ends outdoors so the next Fly is allowed");
    println!("collected · ¥{money_before} → ¥{} · party {}", state.money, state.pokemon.len());

    fixture.save_state_named("src/pokemon/data/postgame-daycare.bin").unwrap();
}

/// Diagnostic for **G8b**: Route 5's terraces. The Day Care door is at (10,21) and the walk in from
/// Cerulean lands at (18,1); `enter(Daycare)` from there does nothing at all.
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_route5_terraces() {
    let mut fixture = TestFixture::new(GIFTS, Duration::from_mins(60), vec![
        PolicyStep::Fly { to: Map::CeruleanCity },
        PolicyStep::enter(Map::CeruleanTrashedHouse),
        PolicyStep::enter_at(Map::CeruleanCity, 27, 9),
        PolicyStep::enter(Map::Route5),
    ]);
    fixture.run_until(|s| s.map.map == Map::Route5);
    for _ in 0..50 { fixture.step(); }
    let state = fixture.game_state();
    println!("== Route5 @ {}", state.map.player_position);
    for action in state.map.actions() {
        println!("   {:?} @ {} ({} steps)", action.tile, action.destination, action.route.len());
    }
    for y in 0..32u8 {
        let row: String = (0..20u8).map(|x| match state.map.tile_at_checked(Point8 { x, y }) {
            Some(MetaTile::Water) => '~',
            Some(MetaTile::Obstacle) => '#',
            Some(MetaTile::Empty) => '.',
            Some(MetaTile::Warp { .. }) => 'W',
            Some(MetaTile::Connection { .. }) => 'C',
            Some(MetaTile::Jump(_)) => 'J',
            Some(MetaTile::Sprite(_)) => 'S',
            Some(MetaTile::Grass) => 'g',
            Some(MetaTile::CutTree) => 'T',
            _ => '?',
        }).collect();
        println!("  y{y:>2} {row}");
    }
}

/// G8b's output: Route 5, party 6 with Hitmonlee back from the Day Care, ¥44,284.
const DAYCARE: &[u8] = include_bytes!("../../data/postgame-daycare.bin");

/// **Task G8c** — the **Name Rater**, and the last three never-visited rooms.
///
/// The Name Rater reuses G8b's [`PartyScript`] driver wholesale — same stale-cursor party menu, a
/// different completion test.
///
/// ⚠️ **Which mon to rename is the whole design of this test**, and the obvious choice is wrong.
/// `DeterministicPolicy` draws nicknames without replacement, but the picker is re-seeded per leg,
/// so every leg's *first* draw is the same name — and five of this party's six are already called
/// it, from the legs that caught them. Renaming one of those is invisible. **Articuno is the only
/// uniquely-named member** ("Leslee", from the main quest), so it is the only slot whose rename can
/// be observed at all.
///
/// That also makes the cursor assertion work: the entry fixture's `wCurrentMenuItem` is **0**, left
/// there by G8b's deposit, so a driver that failed to move it would rename **Venusaur** and leave
/// Articuno untouched — which is exactly what the two assertions below distinguish.
///
/// ⚠️ The rename runs through the ordinary naming screen, so `assert_naming_screen` takes the
/// agent's state away from the driver mid-conversation and never gives it back. That is fine and
/// worth knowing: the *effect* still lands, so this waits on the nickname, not on the driver.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_rename_a_pokemon_and_visit_the_last_rooms() {
    /// Articuno — the one party member not already called what the picker draws first.
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

    // The three text-only rooms, asserted by arrival: they are on §2's never-visited list.
    for room in [Map::ViridianSchoolHouse, Map::CeladonHotel, Map::CeladonChiefHouse] {
        let state = fixture.run_until(|s| s.map.map == room);
        println!("visited {:?} @ {}", room, state.map.player_position);
    }

    let state = fixture.run_leg(|s| s.map.map == Map::CeladonCity);
    assert_eq!(state.pokemon.len(), 6, "nothing here should change the party");
    fixture.save_state_named("src/pokemon/data/postgame-name-rater.bin").unwrap();
}
