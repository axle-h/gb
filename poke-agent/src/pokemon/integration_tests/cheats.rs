
use crate::pokemon::badge::Badge;
use crate::pokemon::item::ItemId;
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::party::PokemonParty;
use crate::pokemon::pokemon::Pokemon;
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::symbols::{DmgPointerRead, pokered_symbols};
use crate::pokemon::{GameState, PokemonApi};

/// What the god party's fighter knows.
const FIGHTER_MOVES: [PokemonMoveName; 4] = [
    PokemonMoveName::Psychic,
    PokemonMoveName::Blizzard,
    PokemonMoveName::Thunderbolt,
    PokemonMoveName::Earthquake,
];

/// The four HMs the action menu gates rows on: a cuttable tree, a water crossing, a boulder push,
/// and a dark floor.
const TERRAIN_MOVES: [PokemonMoveName; 4] = [
    PokemonMoveName::Cut,
    PokemonMoveName::Surf,
    PokemonMoveName::Strength,
    PokemonMoveName::Flash,
];

const FLIGHT_MOVES: [PokemonMoveName; 4] = [
    PokemonMoveName::Fly,
    PokemonMoveName::WingAttack,
    PokemonMoveName::QuickAttack,
    PokemonMoveName::SandAttack,
];

/// The key items a coverage walk needs in the bag before the *overworld* is fully offered to it.
pub const COVERAGE_KEY_ITEMS: [ItemId; 14] = [
    ItemId::OldRod, ItemId::GoodRod, ItemId::SuperRod,
    ItemId::Bicycle, ItemId::PokeFlute, ItemId::SilphScope,
    ItemId::CardKey, ItemId::LiftKey, ItemId::SecretKey, ItemId::SSTicket,
    ItemId::Itemfinder, ItemId::CoinCase, ItemId::GoldTeeth, ItemId::TownMap,
];

/// The sidecar. Built once, [`Self::apply`]-ed between ticks, and idempotent — every field is a
/// *state to hold the game in* rather than an action to take, so it can be re-applied fifty times
/// a second and only write when the game has drifted off it.
#[derive(Debug, Clone)]
pub struct Cheats {
    /// Badges to hold the player at. `None` leaves `wObtainedBadges` alone.
    pub badges: Option<Badge>,
    /// Install the god party once the game has produced a party of its own.
    pub god_party: bool,
    /// Top the party up to full HP and PP whenever it is safe to.
    pub keep_healthy: bool,
    /// Whether the god party has already been installed. Public so a test can assert on it.
    pub installed: bool,
    /// How many times a top-up actually wrote something, for a test that wants to prove the gate
    /// below is doing work rather than never being reached.
    pub top_ups: u32,
    pub refused_in_battle: u32,
    /// Key items to hold in the bag, and how much money to hold. `None` leaves the bag alone.
    pub key_items: Option<u32>,
    /// Whether the bag has been stocked yet.
    pub stocked: bool,
    /// Key items that would not fit, by name. Reported rather than fatal: the bag holds twenty
    /// *kinds* and a finished save arrives nearly full, so a walk that cannot be handed a Bicycle
    /// is a walk that cannot reach Cycling Road — a coverage gap worth printing, not a reason to
    /// fail before the run has taken a single step.
    pub bag_was_full: Vec<crate::pokemon::item::ItemId>,
    /// What was dropped from the bag to make room for [`COVERAGE_KEY_ITEMS`], as raw ids.
    pub bag_was_shed: Vec<u8>,
}

impl Default for Cheats {
    fn default() -> Self {
        Self {
            badges: Some(Badge::all()),
            god_party: true,
            keep_healthy: true,
            installed: false,
            top_ups: 0,
            refused_in_battle: 0,
            key_items: None,
            stocked: false,
            bag_was_full: Vec::new(),
            bag_was_shed: Vec::new(),
        }
    }
}

impl Cheats {

    /// Stock the bag with [`COVERAGE_KEY_ITEMS`] and `money`, once.
    pub fn with_key_items(mut self, money: u32) -> Self {
        self.key_items = Some(money);
        self
    }

    /// Applied by the driver between ticks. Never from `pick_*`.
    pub fn apply(&mut self, api: &mut PokemonApi<'_>, state: &GameState) {
        if let Some(badges) = self.badges {
            if state.badges != badges {
                api.debug_set_badges(badges);
            }
        }

        if !self.party_writes_are_safe(api, state) {
            self.refused_in_battle += 1;
            return;
        }

        if self.god_party && !self.installed && state.pokemon.len() > 0 {
            match api.debug_set_party(&god_party(state)) {
                Ok(()) => self.installed = true,
                Err(why) => panic!("could not install the god party: {why}"),
            }
        }

        // The bag, once the game has one to write into.
        if let Some(money) = self.key_items
            && !self.stocked
            && state.pokemon.len() > 0
        {
            self.bag_was_shed = api.debug_keep_only_items(&COVERAGE_KEY_ITEMS);

            // Only what is missing, and a full bag is still not a panic.
            for item in COVERAGE_KEY_ITEMS {
                if state.bag.iter().any(|held| held.id == item) {
                    continue;
                }
                if api.debug_give_item(item, 1).is_err() {
                    self.bag_was_full.push(item);
                }
            }
            api.debug_set_money(money);
            self.stocked = true;
        }

        if self.keep_healthy && self.needs_a_top_up(state) {
            api.debug_heal_party().expect("the party can be healed");
            api.debug_restore_pp().expect("PP can be restored");
            self.top_ups += 1;
        }
    }

    /// Whether it is safe to write the party struct: no battle, and the black-out window closed.
    fn party_writes_are_safe(&self, api: &PokemonApi<'_>, state: &GameState) -> bool {
        state.battle.is_none()
            && api.mmu().read_pointer(&pokered_symbols::wIsInBattle)
                != crate::pokemon::battle::LOST_BATTLE
    }

    /// Whether anything in the party is short of full HP or PP.
    fn needs_a_top_up(&self, state: &GameState) -> bool {
        state.pokemon.iter().any(|member| {
            member.current_hp < member.stats.hp
                || member.status != crate::pokemon::status::PokemonStatus::None
                || member
                    .moves
                    .iter()
                    .flatten()
                    .any(|battle_move| battle_move.pp < battle_move.name.metadata().pp)
        })
    }
}

/// The party a cheated run plays with: a fighter that cannot lose and the field moves that make
/// the whole map reachable, with whatever the game itself produced kept behind them.
pub fn god_party(state: &GameState) -> PokemonParty {
    let (name, id) = (state.name.clone(), state.player_id);
    let mut party = PokemonParty::default();
    let mut push = |species, nickname: &str, moves| {
        // A party is six; three named members plus up to three carried over cannot overflow it,
        // and the carried-over loop below stops at the cap anyway.
        let _ = party.push(Pokemon::maxed(species, nickname, moves, name.clone(), id));
    };
    push(PokemonSpecies::Mewtwo, "MEWTWO", FIGHTER_MOVES);
    push(PokemonSpecies::Lapras, "TERRAIN", TERRAIN_MOVES);
    push(PokemonSpecies::Pidgeot, "FLIGHT", FLIGHT_MOVES);
    // What the game produced, kept.
    for member in state.pokemon.iter() {
        if party.push(member.clone()).is_err() {
            break;
        }
    }
    party
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::integration_tests::TestFixture;
    use std::time::Duration;

    /// Every new primitive, written and read back through the ordinary `GameState` path — not
    /// through the symbol it wrote, which would only prove the write landed where the write went.
    #[test]
    fn the_debug_primitives_are_visible_to_everything_that_reads_the_game() {
        let mut fixture = TestFixture::with_policy(
            crate::pokemon::integration_tests::PALLET_TOWN_STATE,
            Duration::from_secs(10),
            Box::new(crate::pokemon::policy::RandomPolicy::seeded(0)),
        );

        fixture.api().debug_set_badges(Badge::all());
        assert_eq!(fixture.game_state().badges, Badge::all(), "the badges did not reach `GameState`");

        // A party to work on.
        let state = fixture.game_state();
        fixture.api().debug_set_party(&god_party(&state)).expect("a party can be installed");
        let installed = fixture.game_state();
        // The three named members lead, and whatever the fixture was already carrying follows
        // them — kept deliberately, as the evidence that the story ran rather than being skipped.
        assert!(installed.pokemon.len() >= 3, "the god party is at least its three named members");
        assert_eq!(installed.pokemon[0].species, PokemonSpecies::Mewtwo);
        assert_eq!(installed.pokemon.len(), 3 + state.pokemon.len().min(3), "the old party was not carried");

        // The point of the three-member party: every field move the action menu gates a row on,
        // plus the one that travels.
        for wanted in TERRAIN_MOVES.iter().chain(FLIGHT_MOVES.iter().take(1)) {
            assert!(
                installed.pokemon.iter().any(|member| member
                    .moves
                    .iter()
                    .flatten()
                    .any(|battle_move| battle_move.name == *wanted)),
                "nothing in the god party knows {wanted}",
            );
        }

        // Hurt it, spend its PP, and put both back.
        {
            let mut party = installed.pokemon.clone();
            party[0].current_hp = 1;
            party[0].status = crate::pokemon::status::PokemonStatus::Poisoned;
            party[0].moves[0] = Some(crate::pokemon::move_name::PokemonMove {
                name: FIGHTER_MOVES[0],
                pp: 0,
            });
            fixture.api().debug_set_party(&party).expect("a party can be installed");
        }
        let hurt = fixture.game_state();
        assert_eq!(hurt.pokemon[0].current_hp, 1, "the damage did not land");

        fixture.api().debug_heal_party().expect("healing works");
        fixture.api().debug_restore_pp().expect("PP restore works");
        let healed = fixture.game_state();
        assert_eq!(healed.pokemon[0].current_hp, healed.pokemon[0].stats.hp, "not healed");
        assert_eq!(healed.pokemon[0].status, crate::pokemon::status::PokemonStatus::None);
        assert_eq!(
            healed.pokemon[0].moves[0].expect("slot 0").pp,
            FIGHTER_MOVES[0].metadata().pp,
            "PP was not restored",
        );

        // And a move can be put into a slot by name.
        fixture.api().debug_teach_move(0, 3, PokemonMoveName::Fly).expect("teaching works");
        let taught = fixture.game_state();
        assert_eq!(taught.pokemon[0].moves[3].expect("slot 3").name, PokemonMoveName::Fly);
        // A slot or a member that is not there is an error rather than a silent no-op: a caller
        // that believed a field move was available and was wrong has no way to find out
        // otherwise.
        assert!(fixture.api().debug_teach_move(9, 0, PokemonMoveName::Cut).is_err());
        assert!(fixture.api().debug_teach_move(0, 9, PokemonMoveName::Cut).is_err());
    }

    #[test]
    fn every_coverage_start_can_be_handed_all_of_the_key_items() {
        use crate::pokemon::integration_tests::coverage::COVERAGE_STARTS;

        for start in COVERAGE_STARTS {
            let mut fixture = TestFixture::with_policy(
                start.state,
                Duration::from_secs(10),
                Box::new(crate::pokemon::policy::RandomPolicy::seeded(0)),
            );
            let mut cheats = Cheats::default().with_key_items(999_999);
            let state = fixture.game_state();
            // The gate `Cheats::apply` waits on: a fresh save has no bag until Oak's script has
            // run, so a start with no party would silently never be stocked at all.
            assert!(state.pokemon.len() > 0, "{}: a start has to have a party", start.name);
            cheats.apply(&mut fixture.api(), &state);
            assert!(cheats.stocked, "{}: the bag was never stocked", start.name);

            assert!(
                cheats.bag_was_full.is_empty(),
                "{}: the bag refused {:?}; it shed {:?} to make room",
                start.name, cheats.bag_was_full, cheats.bag_was_shed,
            );
            let bag = fixture.game_state().bag;
            for item in COVERAGE_KEY_ITEMS {
                assert!(
                    bag.iter().any(|held| held.id == item),
                    "{}: {item:?} is not in the bag after stocking", start.name,
                );
            }
            // Room left for what the walk finds on the floor.
            let used = bag.iter().count();
            assert!(
                used < crate::pokemon::bag::Bag::MAX_ITEMS,
                "{}: the bag came out full at {used} kinds, so no pickup can land", start.name,
            );
        }
    }

    /// The gate in [`Cheats::apply`] is what keeps a cheated run honest, so it gets its own test:
    /// it must refuse to touch the party during a battle and during the black-out window, and it
    /// must still write the badges, which have neither constraint.
    #[test]
    fn the_sidecar_will_not_write_the_party_during_a_battle() {
        use gb::ram::RAM;

        let mut fixture = TestFixture::with_policy(
            crate::pokemon::integration_tests::BATTLE_STATE,
            Duration::from_secs(10),
            Box::new(crate::pokemon::policy::RandomPolicy::seeded(0)),
        );
        // Step until the battle is actually readable — the fixture is captured a moment before
        // it.
        let mut ticks = 0;
        while fixture.try_game_state().map_or(true, |state| state.battle.is_none()) {
            fixture.step();
            ticks += 1;
            assert!(ticks < 2_000, "the battle fixture never entered a battle");
        }

        let mut cheats = Cheats::default();
        let state = fixture.game_state();
        assert!(state.battle.is_some(), "the fixture has to be in a battle for this to mean anything");
        let before = state.pokemon.clone();
        cheats.apply(&mut fixture.api(), &state);

        assert!(!cheats.installed, "the god party was installed during a battle");
        assert_eq!(cheats.top_ups, 0, "the party was topped up during a battle");
        assert_eq!(cheats.refused_in_battle, 1, "the refusal was not counted");
        assert_eq!(fixture.game_state().pokemon, before, "the party was written during a battle");
        // The badges have neither constraint and are the proof the sidecar ran at all.
        assert_eq!(fixture.game_state().badges, Badge::all(), "the badges were not written");

        // The black-out window: no battle, and still not a moment to write a party.
        fixture.gb.core_mut().mmu_mut().write(
            pokered_symbols::wIsInBattle.address,
            crate::pokemon::battle::LOST_BATTLE,
        );
        let state = fixture.game_state();
        assert!(state.battle.is_none(), "`$ff` must read as no battle");
        cheats.apply(&mut fixture.api(), &state);
        assert!(!cheats.installed, "the god party was installed inside the black-out window");
        assert_eq!(cheats.refused_in_battle, 2, "the black-out refusal was not counted");
    }

    /// And in the overworld it does all three, once, and then stops writing.
    #[test]
    fn the_sidecar_installs_the_party_once_and_then_holds_it() {
        let mut fixture = TestFixture::with_policy(
            crate::pokemon::integration_tests::ROUTE1_STATE,
            Duration::from_secs(10),
            Box::new(crate::pokemon::policy::RandomPolicy::seeded(0)),
        );
        let mut cheats = Cheats::default();
        for _ in 0..8 {
            let state = fixture.game_state();
            cheats.apply(&mut fixture.api(), &state);
        }
        assert!(cheats.installed, "the god party was never installed: {cheats:?}");
        assert_eq!(cheats.refused_in_battle, 0, "nothing here is a battle");
        let state = fixture.game_state();
        assert_eq!(state.pokemon[0].species, PokemonSpecies::Mewtwo);
        assert_eq!(state.badges, Badge::all());
        // Idempotent: a party already at full HP and PP needs no further write, so the count
        // settles.
        let settled = cheats.top_ups;
        for _ in 0..8 {
            let state = fixture.game_state();
            cheats.apply(&mut fixture.api(), &state);
        }
        assert_eq!(cheats.top_ups, settled, "the sidecar keeps rewriting a party that is already full");
    }
}
