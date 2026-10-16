//! The turning points of a run, for whatever tells its story afterwards. A `timeline` event marks
//! every change of map, badges, party or Pokédex; a [`Moment`] is a line of `moments.jsonl` naming a
//! save state in `moments/`, taken at a badge, a boss battle, a new species, an evolution, a
//! black-out, the Hall of Fame, or after [`LULL_SECONDS`] of play without any of those.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use poke_core::map::Map;

use crate::pokemon::GameState;
use crate::pokemon::badge::Badge;
use crate::pokemon::observe::{self, Readout};

use super::files;

/// In-game time after which a stretch with no moment gets one anyway.
pub const LULL_SECONDS: u32 = 5 * 3600;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Member {
    pub nickname: String,
    pub species: String,
    pub dex: u8,
    pub level: u8,
}

/// A trainer class worth a moment, by the name the story would use.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Boss {
    pub name: String,
    pub role: &'static str,
    pub trainer_class: u8,
}

/// `class` (`wTrainerClass`), fought on `map` with the rival called `rival`, if it is a boss.
pub fn boss(class: u8, map: Map, rival: &str) -> Option<Boss> {
    let (name, role) = match class {
        0x19 | 0x2A => (rival, "rival"),
        // RIVAL3 is fought nowhere but the Champion's room.
        0x2B => (rival, "champion"),
        0x1D if map == Map::ViridianGym => ("GIOVANNI", "gym leader"),
        0x1D => ("GIOVANNI", "Team Rocket's boss"),
        0x22 => ("BROCK", "gym leader"),
        0x23 => ("MISTY", "gym leader"),
        0x24 => ("LT.SURGE", "gym leader"),
        0x25 => ("ERIKA", "gym leader"),
        0x26 => ("KOGA", "gym leader"),
        0x27 => ("BLAINE", "gym leader"),
        0x28 => ("SABRINA", "gym leader"),
        0x2C => ("LORELEI", "Elite Four"),
        0x21 => ("BRUNO", "Elite Four"),
        0x2E => ("AGATHA", "Elite Four"),
        0x2F => ("LANCE", "Elite Four"),
        _ => return None,
    };
    Some(Boss { name: name.to_string(), role, trainer_class: class })
}

/// What the game holds that a story turns on: the body of a `timeline` event.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Landmarks {
    pub map: String,
    pub x: u8,
    pub y: u8,
    pub badges: Vec<String>,
    pub party: Vec<Member>,
    pub owned: usize,
    pub seen: usize,
    pub money: u32,
    pub playtime: String,
    pub playtime_seconds: u32,
    /// Dex number and name, by dex number.
    #[serde(skip)]
    owned_dex: Vec<(u8, &'static str)>,
    #[serde(skip)]
    boss: Option<Boss>,
}

impl Landmarks {
    pub fn of(state: &GameState, readout: &(impl Readout + ?Sized)) -> Self {
        let mut owned_dex: Vec<(u8, &'static str)> = state.pokedex_owned.species().iter()
            .map(|species| (species.metadata().pokedex_number, species.metadata().name))
            .collect();
        owned_dex.sort_unstable();
        Self {
            map: format!("{}", state.map.map),
            x: state.map.player_position.x,
            y: state.map.player_position.y,
            badges: Badge::ORDER
                .iter()
                .filter(|badge| state.badges.contains(**badge))
                .map(|badge| format!("{badge}").to_uppercase())
                .collect(),
            party: state.pokemon.iter()
                .map(|mon| Member {
                    nickname: mon.nickname.to_default_string(),
                    species: mon.species.metadata().name.to_string(),
                    dex: mon.species.metadata().pokedex_number,
                    level: mon.level,
                })
                .collect(),
            owned: owned_dex.len(),
            seen: state.pokedex_seen.species().len(),
            money: state.money,
            playtime: observe::playtime(readout),
            playtime_seconds: observe::playtime_seconds(readout),
            owned_dex,
            boss: state.battle.as_ref().and_then(|battle| {
                boss(battle.trainer_class, state.map.map, &state.rival_name.to_default_string())
            }),
        }
    }

    /// Whether a timeline would draw these two the same: position, money and the clock aside.
    fn same_story(&self, other: &Self) -> bool {
        self.same_places_and_people(other) && self.boss == other.boss
    }

    fn same_places_and_people(&self, other: &Self) -> bool {
        self.map == other.map && self.badges == other.badges && self.party == other.party
            && self.owned == other.owned && self.seen == other.seen
    }
}

/// A moment found, before it has a number, a save state or a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub kind: &'static str,
    pub title: String,
}

/// What one sample said.
#[derive(Debug, Default)]
pub struct Observed {
    /// The sample, when it is the first or the story moved.
    pub timeline: Option<Landmarks>,
    pub moments: Vec<Found>,
}

/// Turns heartbeat samples into timeline changes and moments.
#[derive(Debug, Default)]
pub struct MomentTracker {
    /// The last sample, whose story is believed only once the next one tells the same: a screen
    /// transition reads RAM the game is halfway through rewriting.
    candidate: Option<Landmarks>,
    accepted: Option<Landmarks>,
    /// Play time at the last moment; `None` until a moment or the first sample sets it.
    last_moment_at: Option<u32>,
}

impl MomentTracker {
    /// A tracker for a run whose last moment, if any, was at `last_moment_at` seconds of play.
    pub fn resuming(last_moment_at: Option<u32>) -> Self {
        Self { last_moment_at, ..Self::default() }
    }

    pub fn observe(&mut self, sample: Landmarks) -> Observed {
        let agreed = self.candidate.as_ref().is_some_and(|candidate| candidate.same_story(&sample));
        self.candidate = Some(sample.clone());
        if !agreed {
            return Observed::default();
        }
        let Some(previous) = self.accepted.replace(sample.clone()) else {
            self.last_moment_at.get_or_insert(sample.playtime_seconds);
            return Observed { timeline: Some(sample), moments: Vec::new() };
        };

        let mut moments = Vec::new();
        for badge in sample.badges.iter().filter(|badge| !previous.badges.contains(badge)) {
            moments.push(Found { kind: "badge", title: format!("earned the {badge}") });
        }
        if let Some(boss) = sample.boss.as_ref().filter(|boss| previous.boss.as_ref() != Some(boss)) {
            moments.push(Found { kind: "boss_battle", title: format!("challenged {}, {}", boss.name, the(boss.role)) });
        }
        let evolved = evolution(&previous.party, &sample.party);
        if let Some((from, to)) = &evolved {
            moments.push(Found { kind: "evolution", title: format!("{} evolved into {}", from.nickname, to.species) });
        }
        for (dex, name) in sample.owned_dex.iter().filter(|owned| previous.owned_dex.binary_search(owned).is_err()) {
            if evolved.as_ref().is_some_and(|(_, to)| to.dex == *dex) {
                continue;
            }
            moments.push(Found { kind: "new_species", title: format!("registered {name} in the Pokédex") });
        }
        if moments.is_empty()
            && self.last_moment_at.is_some_and(|at| sample.playtime_seconds >= at.saturating_add(LULL_SECONDS))
        {
            moments.push(Found { kind: "lull", title: format!("{} hours of play with nothing to mark them", LULL_SECONDS / 3600) });
        }

        if !moments.is_empty() {
            self.last_moment_at = Some(sample.playtime_seconds);
        }
        let timeline = (!previous.same_places_and_people(&sample)).then(|| sample.clone());
        Observed { timeline, moments }
    }

    /// A moment this tracker did not find was taken at `playtime_seconds`, so the lull counts from there.
    pub fn taken_at(&mut self, playtime_seconds: u32) {
        self.last_moment_at = Some(playtime_seconds);
    }

    /// The last sample believed, for a moment the tracker did not find itself.
    pub fn latest(&self) -> Option<&Landmarks> {
        self.accepted.as_ref()
    }
}

fn the(role: &str) -> String {
    match role {
        "Elite Four" => "of the Elite Four".to_string(),
        "Team Rocket's boss" => role.to_string(),
        _ => format!("the {role}"),
    }
}

/// The one slot that changed species with everything else in place and no level lost.
fn evolution<'a>(before: &'a [Member], after: &'a [Member]) -> Option<(&'a Member, &'a Member)> {
    if before.len() != after.len() {
        return None;
    }
    let mut changed = before.iter().zip(after).filter(|(was, now)| was != now);
    let (was, now) = changed.next()?;
    (changed.next().is_none() && was.dex != now.dex && now.level >= was.level).then_some((was, now))
}

/// One line of `moments.jsonl`, and the body of the `moment` event that announces it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Moment {
    /// From 1, in the order taken.
    pub n: u32,
    pub kind: String,
    pub title: String,
    /// Relative to the run directory.
    pub state: String,
    pub map: String,
    pub x: u8,
    pub y: u8,
    pub playtime: String,
    pub playtime_seconds: u32,
    pub badges: usize,
    pub party: Vec<Member>,
}

impl Moment {
    pub fn new(n: u32, found: &Found, landmarks: &Landmarks, extension: &str) -> Self {
        Self {
            n,
            kind: found.kind.to_string(),
            title: found.title.clone(),
            state: format!("{}/{n:04}-{}.{extension}", files::MOMENTS, found.kind),
            map: landmarks.map.clone(),
            x: landmarks.x,
            y: landmarks.y,
            playtime: landmarks.playtime.clone(),
            playtime_seconds: landmarks.playtime_seconds,
            badges: landmarks.badges.len(),
            party: landmarks.party.clone(),
        }
    }
}

/// The last line of `run_dir`'s `moments.jsonl`, which numbers the next and times the lull.
pub fn last(run_dir: &Path) -> Option<Moment> {
    let file = std::fs::File::open(run_dir.join(files::MOMENTS_LOG)).ok()?;
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Moment>(&line).ok())
        .last()
}

/// Write `state` where `moment` says and then its line, which therefore never names a missing file.
/// `seq` is the transcript's `moment` event.
pub fn record(run_dir: &Path, moment: &Moment, seq: u64, state: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(run_dir.join(files::MOMENTS))
        .map_err(|e| format!("could not create the moments directory: {e}"))?;
    let path = run_dir.join(&moment.state);
    std::fs::write(&path, state).map_err(|e| format!("could not write {}: {e}", path.display()))?;

    let mut line = serde_json::to_value(moment).map_err(|e| format!("could not encode a moment: {e}"))?;
    line["seq"] = seq.into();
    let log = run_dir.join(files::MOMENTS_LOG);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(|e| format!("could not open {}: {e}", log.display()))?;
    writeln!(file, "{line}").map_err(|e| format!("could not append to {}: {e}", log.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(nickname: &str, species: &str, dex: u8, level: u8) -> Member {
        Member { nickname: nickname.into(), species: species.into(), dex, level }
    }

    fn landmarks(playtime_seconds: u32) -> Landmarks {
        Landmarks {
            map: "Pallet Town".into(),
            x: 5,
            y: 6,
            badges: Vec::new(),
            party: vec![member("SQUIRTLE", "Squirtle", 7, 15)],
            owned: 1,
            seen: 1,
            money: 3000,
            playtime: String::new(),
            playtime_seconds,
            owned_dex: vec![(7, "Squirtle")],
            boss: None,
        }
    }

    /// Feed `sample` twice, as two agreeing heartbeats, and keep what either said.
    fn settle(tracker: &mut MomentTracker, sample: &Landmarks) -> Observed {
        let first = tracker.observe(sample.clone());
        let mut second = tracker.observe(sample.clone());
        second.timeline = second.timeline.or(first.timeline);
        second.moments.splice(0..0, first.moments);
        second
    }

    fn kinds(observed: &Observed) -> Vec<&'static str> {
        observed.moments.iter().map(|found| found.kind).collect()
    }

    #[test]
    fn the_first_settled_sample_is_the_timeline_baseline_and_no_moment() {
        let mut tracker = MomentTracker::default();
        let observed = settle(&mut tracker, &landmarks(60));
        assert!(observed.timeline.is_some());
        assert!(observed.moments.is_empty());
    }

    #[test]
    fn a_sample_the_next_one_disagrees_with_is_never_believed() {
        let mut tracker = MomentTracker::default();
        settle(&mut tracker, &landmarks(60));
        let mut glitch = landmarks(61);
        glitch.badges = vec!["BOULDERBADGE".into()];
        let observed = tracker.observe(glitch);
        assert!(observed.moments.is_empty() && observed.timeline.is_none());
        let observed = settle(&mut tracker, &landmarks(62));
        assert!(observed.moments.is_empty() && observed.timeline.is_none());
    }

    #[test]
    fn a_badge_a_boss_and_a_new_species_are_moments() {
        let mut tracker = MomentTracker::default();
        settle(&mut tracker, &landmarks(60));

        let mut gym = landmarks(70);
        gym.map = "Pewter Gym".into();
        gym.boss = boss(0x22, Map::PewterGym, "BLUE");
        let observed = settle(&mut tracker, &gym);
        assert_eq!(kinds(&observed), ["boss_battle"]);
        assert_eq!(observed.moments[0].title, "challenged BROCK, the gym leader");
        assert!(observed.timeline.is_some(), "a new map");

        // The same battle, a turn later, is not a second moment.
        let mut later = gym.clone();
        later.money = 1;
        assert!(settle(&mut tracker, &later).moments.is_empty());

        let mut won = gym.clone();
        won.boss = None;
        won.badges = vec!["BOULDERBADGE".into()];
        won.party.push(member("PIDGEY", "Pidgey", 16, 3));
        won.owned_dex = vec![(7, "Squirtle"), (16, "Pidgey")];
        won.owned = 2;
        let observed = settle(&mut tracker, &won);
        assert_eq!(kinds(&observed), ["badge", "new_species"]);
        assert_eq!(observed.moments[0].title, "earned the BOULDERBADGE");
    }

    #[test]
    fn an_evolution_is_one_moment_even_when_it_registers_a_species() {
        let mut tracker = MomentTracker::default();
        settle(&mut tracker, &landmarks(60));
        let mut evolved = landmarks(70);
        evolved.party = vec![member("WARTORTLE", "Wartortle", 8, 16)];
        evolved.owned_dex = vec![(7, "Squirtle"), (8, "Wartortle")];
        let observed = settle(&mut tracker, &evolved);
        assert_eq!(kinds(&observed), ["evolution"]);
        assert_eq!(observed.moments[0].title, "SQUIRTLE evolved into Wartortle");
    }

    #[test]
    fn walking_is_believed_at_once_and_draws_no_timeline() {
        let mut tracker = MomentTracker::default();
        settle(&mut tracker, &landmarks(60));
        let mut step = landmarks(61);
        step.x = 6;
        let observed = tracker.observe(step);
        assert!(observed.timeline.is_none() && observed.moments.is_empty());
        assert_eq!(tracker.latest().map(|latest| latest.x), Some(6));
    }

    #[test]
    fn a_swap_is_not_an_evolution() {
        let before = [member("A", "Pidgey", 16, 5), member("B", "Rattata", 19, 5)];
        let after = [member("B", "Rattata", 19, 5), member("A", "Pidgey", 16, 5)];
        assert_eq!(evolution(&before, &after), None);
    }

    #[test]
    fn a_long_stretch_without_a_moment_is_one() {
        let mut tracker = MomentTracker::resuming(Some(100));
        settle(&mut tracker, &landmarks(100 + LULL_SECONDS - 1));
        let mut moved = landmarks(100 + LULL_SECONDS);
        moved.x = 9;
        let observed = settle(&mut tracker, &moved);
        assert_eq!(kinds(&observed), ["lull"]);
        moved.x = 10;
        assert!(settle(&mut tracker, &moved).moments.is_empty());
    }

    #[test]
    fn a_moment_is_written_before_the_line_that_names_it_and_numbers_the_next() {
        let dir = std::env::temp_dir().join(format!("gb-moments-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let found = Found { kind: "badge", title: "earned the BOULDERBADGE".into() };
        let moment = Moment::new(7, &found, &landmarks(3600), "pkrd");
        assert_eq!(moment.state, "moments/0007-badge.pkrd");
        record(&dir, &moment, 42, b"state").unwrap();

        assert_eq!(std::fs::read(dir.join(&moment.state)).unwrap(), b"state");
        let last = last(&dir).expect("one line");
        assert_eq!((last.n, last.playtime_seconds), (7, 3600));
        let line = std::fs::read_to_string(dir.join(files::MOMENTS_LOG)).unwrap();
        assert!(line.contains("\"seq\":42"), "{line}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
