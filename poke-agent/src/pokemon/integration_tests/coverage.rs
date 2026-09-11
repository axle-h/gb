use std::collections::BTreeMap;

use crate::pokemon::agent::{AgentEvent, OverworldActionAbortedReason};

/// How many times an id may be blocked by the game speaking before the repetition is the finding.
pub const REPEAT_IS_A_DEFECT: usize = 10;

/// What became of one action id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Never reached: seen in a menu, never chosen.
    Unreached,
    /// The walk arrived, or the conversation happened.
    Completed,
    /// The game stopped the player to say something, and here is what it said.
    Blocked { times: usize, message: Option<String> },
    /// The menu offered a row the agent could not then execute.
    Defect { reason: String, at: Option<gb::geometry::Point8> },
    /// The action was taken and the agent never said what became of it.
    Silent,
}

impl Verdict {
    /// Whether this verdict is a defect *proper*: the agent could not do what the menu offered.
    pub fn is_defect(&self) -> bool {
        match self {
            Self::Defect { .. } => true,
            Self::Blocked { times, .. } => *times >= REPEAT_IS_A_DEFECT,
            Self::Unreached | Self::Completed | Self::Silent => false,
        }
    }

    /// Whether this verdict makes the coverage tier red. A defect, or a silence.
    pub fn fails_the_walk(&self) -> bool {
        self.is_defect() || matches!(self, Self::Silent)
    }
}

/// One id's row in the table.
#[derive(Debug, Clone)]
pub struct Entry {
    /// `{map}:{x},{y}:{kind}`, as `OverworldAction::id` makes it.
    pub id: String,
    /// How many times the id was chosen.
    pub attempts: usize,
    pub verdict: Verdict,
    /// Every abort reason this id has produced, and how often, kept after the id completes.
    pub aborts: BTreeMap<String, usize>,
}

/// The table, plus the two failures that belong to no id.
#[derive(Debug, Default)]
pub struct CoverageLog {
    entries: BTreeMap<String, Entry>,
    /// The id whose walk is in flight, if any.
    open: Option<String>,
    pub interleaved: Vec<(String, String)>,
    /// Every watchdog firing, with the agent state it fired in; always a defect.
    pub watchdog: Vec<String>,
    /// Ids whose verdict became a hard [`Verdict::Defect`] since the last call to
    /// [`Self::take_new_defects`].
    new_defects: Vec<String>,
    /// The last thing the game said, so a `Blocked` verdict can quote it.
    last_blocked: Option<String>,
}

impl CoverageLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold in one event. Call it for every event the agent drains, under any driver.
    pub fn observe(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::StartedOverworldAction { id, .. } => {
                // A start while one is open means the previous action ended silently.
                if let Some(already) = self.open.clone() {
                    self.interleaved.push((already.clone(), id.clone()));
                    let entry = self.entry(&already);
                    if entry.verdict == Verdict::Unreached {
                        entry.verdict = Verdict::Silent;
                    }
                }
                self.entry(id).attempts += 1;
                self.open = Some(id.clone());
                self.last_blocked = None;
            }
            AgentEvent::OverworldActionCompleted { .. }
            | AgentEvent::OverworldInteractionCompleted { .. } => {
                if let Some(id) = self.open.take() {
                    self.entry(&id).verdict = Verdict::Completed;
                }
            }
            AgentEvent::OverworldPickupFailed { .. } => {
                if let Some(id) = self.open.take() {
                    let message = self.last_blocked.clone();
                    self.block(&id, message);
                }
            }
            AgentEvent::OverworldActionAborted { reason, at, .. } => {
                let Some(id) = self.open.take() else { return };
                self.entry(&id).aborts.entry(reason.to_string()).and_modify(|n| *n += 1).or_insert(1);
                match reason {
                    OverworldActionAbortedReason::Battle
                    | OverworldActionAbortedReason::NamingScreen => {}
                    // A pace that ran its whole budget with nothing turning up is done, not failed.
                    OverworldActionAbortedReason::NothingAppeared => {
                        self.entry(&id).verdict = Verdict::Completed;
                    }
                    // A wedged Strength floor is the world saying no, not the agent failing.
                    OverworldActionAbortedReason::PuzzleUnsolvable => {
                        let message = Some(reason.to_string());
                        self.block(&id, message);
                    }
                    OverworldActionAbortedReason::Textbox | OverworldActionAbortedReason::Script => {
                        let message = self.last_blocked.clone();
                        self.block(&id, message);
                        // Re-opened so the quote that follows lands on this id.
                        self.open = Some(id);
                    }
                    other => {
                        self.entry(&id).verdict =
                            Verdict::Defect { reason: other.to_string(), at: *at };
                        self.new_defects.push(id);
                    }
                }
            }
            AgentEvent::TextBox { message } => {
                self.last_blocked = Some(message.clone());
                // Attach it to the id that was just stopped and has no quote yet.
                if let Some(id) = self.open.clone() {
                    if let Some(entry) = self.entries.get_mut(&id) {
                        if let Verdict::Blocked { message: quote @ None, .. } = &mut entry.verdict {
                            *quote = Some(message.clone());
                        }
                    }
                }
            }
            AgentEvent::WatchdogFired { agent_state, stuck_for } => {
                self.watchdog.push(format!("{agent_state} for {stuck_for:?}"));
            }
            _ => {}
        }
    }

    fn entry(&mut self, id: &str) -> &mut Entry {
        self.entries.entry(id.to_string()).or_insert_with(|| Entry {
            id: id.to_string(),
            attempts: 0,
            verdict: Verdict::Unreached,
            aborts: BTreeMap::new(),
        })
    }

    /// Mark an id blocked, keeping the count and never overwriting a quote with a missing one.
    fn block(&mut self, id: &str, message: Option<String>) {
        let entry = self.entry(id);
        let (times, kept) = match &entry.verdict {
            Verdict::Blocked { times, message } => (*times + 1, message.clone()),
            _ => (1, None),
        };
        entry.verdict = Verdict::Blocked { times, message: message.or(kept) };
    }

    /// Ids that became a defect since the last call, clearing the list, so the fixture can save the
    /// failing square while the emulator is still on it.
    pub fn take_new_defects(&mut self) -> Vec<String> {
        std::mem::take(&mut self.new_defects)
    }

    /// Note an id the menu offered but nothing chose, so the table says what was not done.
    pub fn offered(&mut self, id: &str) {
        self.entry(id);
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.get(id)
    }

    pub fn completed(&self) -> usize {
        self.entries.values().filter(|entry| entry.verdict == Verdict::Completed).count()
    }

    /// How many distinct maps the run reached, which says whether a driver explores or diffuses.
    pub fn maps_touched(&self) -> usize {
        self.entries
            .keys()
            .filter_map(|id| id.split(':').next())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }

    /// Every defect id and every watchdog firing, one line each; silences are in
    /// [`Self::failures`].
    pub fn defects(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .entries
            .values()
            .filter(|entry| entry.verdict.is_defect())
            .map(|entry| match &entry.verdict {
                Verdict::Defect { reason, at } => match at {
                    Some(at) => format!("{}: {reason} (standing at ({}, {}))", entry.id, at.x, at.y),
                    None => format!("{}: {reason}", entry.id),
                },
                Verdict::Blocked { times, message } => format!(
                    "{}: stopped {times} times by the same thing{}",
                    entry.id,
                    message.as_deref().map(|m| format!(" — {m:?}")).unwrap_or_default(),
                ),
                _ => unreachable!("filtered on is_defect"),
            })
            .collect();
        // The watchdog belongs to no id: no decision point was reached, so nothing was in flight.
        out.extend(self.watchdog.iter().map(|where_| format!("the watchdog fired: {where_}")));
        out
    }

    pub fn report(&self) -> String {
        let mut out = String::new();
        out.push_str("id\tattempts\tverdict\tdetail\n");
        for entry in self.entries.values() {
            let (verdict, detail) = match &entry.verdict {
                Verdict::Unreached => ("unreached", String::new()),
                Verdict::Completed => ("completed", String::new()),
                Verdict::Blocked { times, message } => (
                    "blocked",
                    format!("{times}x {}", message.as_deref().unwrap_or("(nothing was quoted)")),
                ),
                Verdict::Defect { reason, at } => (
                    "defect",
                    match at {
                        Some(at) => format!("{reason}, standing at ({}, {})", at.x, at.y),
                        None => reason.clone(),
                    },
                ),
                Verdict::Silent => ("silent", "chosen; no outcome was ever reported".to_string()),
            };
            let aborts: Vec<String> =
                entry.aborts.iter().map(|(reason, n)| format!("{n}x {reason}")).collect();
            out.push_str(&format!(
                "{}\t{}\t{verdict}\t{detail}{}\n",
                entry.id,
                entry.attempts,
                match aborts.is_empty() {
                    true => String::new(),
                    false => format!(" [{}]", aborts.join("; ")),
                },
            ));
        }
        out
    }

    /// Everything that makes the tier red: [`Self::defects`], and then every silence.
    pub fn failures(&self) -> Vec<String> {
        let mut out = self.defects();
        out.extend(
            self.entries
                .values()
                .filter(|entry| entry.verdict == Verdict::Silent)
                .map(|entry| format!(
                    "{}: chosen {} time(s) and the agent never said what became of any of them",
                    entry.id, entry.attempts,
                )),
        );
        out
    }

    /// How many ids were chosen and never reported an outcome. See [`Verdict::Silent`].
    pub fn silent(&self) -> usize {
        self.entries.values().filter(|entry| entry.verdict == Verdict::Silent).count()
    }

    #[cfg(feature = "slow-tests")]
    /// The kinds of action that went silent, and how many of each.
    pub fn silent_kinds(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for entry in self.entries.values().filter(|entry| entry.verdict == Verdict::Silent) {
            let kind = entry.id.rsplit(':').next().unwrap_or("?").to_string();
            *out.entry(kind).or_insert(0) += 1;
        }
        out
    }

    #[cfg(feature = "slow-tests")]
    pub fn unreached_kinds(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for entry in self.entries.values().filter(|entry| entry.verdict == Verdict::Unreached) {
            *out.entry(kind_of(&entry.id)).or_insert(0) += 1;
        }
        out
    }

    /// One line for a test to print: how much of the world this driver touched.
    pub fn summary(&self) -> String {
        format!(
            "{} ids across {} maps: {} completed, {} blocked, {} silent, {} defects",
            self.len(),
            self.maps_touched(),
            self.completed(),
            self.entries
                .values()
                .filter(|entry| matches!(entry.verdict, Verdict::Blocked { .. }))
                .count(),
            self.silent(),
            self.defects().len(),
        )
    }

    #[cfg(feature = "slow-tests")]
    /// Write the table under `target/test-artifacts/coverage/`, returning the path or why it could
    /// not be written.
    pub fn write_report(&self, name: &str) -> Result<std::path::PathBuf, String> {
        let dir = std::path::Path::new("target/test-artifacts/coverage");
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("{name}.tsv"));
        std::fs::write(&path, self.report()).map_err(|e| e.to_string())?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::tile::MetaTile;

    /// Every regional start stands where its row says, on a game the cartridge finished.
    #[test]
    fn every_coverage_start_stands_where_it_says_on_a_finished_game() {
        use crate::pokemon::integration_tests::fixture::TestFixture;

        let mut names: Vec<&str> = COVERAGE_STARTS.iter().map(|start| start.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), COVERAGE_STARTS.len(), "two starts share a name: {names:?}");
        assert_eq!(
            COVERAGE_STARTS.first().map(|start| start.name),
            Some("entry"),
            "`entry` is the default start, and the table leads with it",
        );

        let mut maps = std::collections::BTreeSet::new();
        for start in COVERAGE_STARTS {
            let mut fixture =
                TestFixture::new(start.state, std::time::Duration::from_mins(1), vec![]);
            let state = fixture.game_state();
            assert_eq!(
                state.map.map, start.map,
                "the {} start stands on {:?}, not the {:?} its row claims",
                start.name, state.map.map, start.map,
            );
            match start.before_the_credits {
                None => assert!(
                    state.hall_of_fame_teams > 0,
                    "the {} start is not a finished game, so the event gates only a finished game \
                     opens are shut. If that is deliberate, `before_the_credits` is where the reason goes",
                    start.name,
                ),
                Some(why) => assert!(
                    state.hall_of_fame_teams == 0,
                    "the {} start says it is before the credits ({why}), but this save has \
                     finished the game",
                    start.name,
                ),
            }
            assert!(
                maps.insert((state.map.map, start.before_the_credits.is_some())),
                "two starts stand on {:?} in the same game, so one of them is a wasted sweep",
                state.map.map,
            );
        }
    }

    fn started(id: &str) -> AgentEvent {
        AgentEvent::StartedOverworldAction { destination: MetaTile::Grass, id: id.to_string() }
    }

    #[test]
    fn the_rom_cross_check_finds_a_door_that_was_never_offered() {
        let gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        let mmu = gb.core().mmu();

        // Oak's lab was offered and nothing else was.
        let offered: std::collections::BTreeSet<String> =
            ["PalletTown:12,12:Warp".to_string()].into_iter().collect();
        let report = rom_cross_check(mmu, &offered);

        assert!(report.contains("1 maps the walk entered"), "{report}");
        assert!(report.contains("(5, 6) → RedsHouse1F"), "{report}");
        assert!(report.contains("(13, 6) → BluesHouse"), "{report}");
        assert!(!report.contains("→ OaksLab"), "the offered door must not be reported: {report}");

        assert!(!report.contains("ViridianCity"), "{report}");

        // Nothing offered: the scope is empty and the report says so rather than listing the game.
        let nothing = rom_cross_check(mmu, &std::collections::BTreeSet::new());
        assert!(nothing.contains("0 maps the walk entered"), "{nothing}");
        assert!(nothing.contains("nothing to report"), "{nothing}");
    }

    fn aborted(reason: OverworldActionAbortedReason) -> AgentEvent {
        AgentEvent::OverworldActionAborted { destination: MetaTile::Grass, reason, at: None }
    }

    const TREE: &str = "Route2:5,8:CutTree";
    const GATE: &str = "Route22Gate:2,4:Warp";

    /// The four verdicts, each from the events that produce it.
    #[test]
    fn every_abort_reason_lands_on_the_verdict_it_deserves() {
        let mut log = CoverageLog::new();

        log.observe(&started(TREE));
        log.observe(&AgentEvent::OverworldActionCompleted { destination: MetaTile::Grass });
        assert_eq!(log.get(TREE).unwrap().verdict, Verdict::Completed);

        // A battle leaves the id retryable rather than judged.
        let id = "Route1:3,3:Grass";
        log.observe(&started(id));
        log.observe(&aborted(OverworldActionAbortedReason::Battle));
        assert_eq!(log.get(id).unwrap().verdict, Verdict::Unreached, "a battle is not a verdict");
        log.observe(&started(id));
        log.observe(&AgentEvent::OverworldActionCompleted { destination: MetaTile::Grass });
        assert_eq!(log.get(id).unwrap().verdict, Verdict::Completed);
        assert_eq!(log.get(id).unwrap().attempts, 2, "both attempts are counted");

        // A guard is `Blocked`, and his quote arrives after the abort.
        log.observe(&started(GATE));
        log.observe(&aborted(OverworldActionAbortedReason::Textbox));
        log.observe(&AgentEvent::TextBox {
            message: "You don't have the BOULDERBADGE yet!".to_string(),
        });
        let Verdict::Blocked { times, message } = log.get(GATE).unwrap().verdict.clone() else {
            panic!("a guard is a block: {:?}", log.get(GATE).unwrap().verdict);
        };
        assert_eq!(times, 1);
        assert_eq!(message.as_deref(), Some("You don't have the BOULDERBADGE yet!"));
        assert!(!log.get(GATE).unwrap().verdict.is_defect(), "being told something once is not a bug");

        // Every reason the agent could not do what the menu offered is a defect outright.
        for reason in [
            OverworldActionAbortedReason::Unknown,
            OverworldActionAbortedReason::DidNotArrive,
            OverworldActionAbortedReason::NoAdjacentGrass,
            OverworldActionAbortedReason::NoRoute(MetaTile::Grass),
            OverworldActionAbortedReason::WrongMap(crate::pokemon::map::Map::PalletTown),
        ] {
            let mut log = CoverageLog::new();
            log.observe(&started("Route3:1,1:Warp"));
            log.observe(&aborted(reason));
            assert!(
                log.get("Route3:1,1:Warp").unwrap().verdict.is_defect(),
                "{reason} has to be a defect: the menu offered a row the agent could not execute",
            );
        }
    }

    /// Being stopped by the same thing over and over is itself the defect.
    #[test]
    fn being_stopped_by_the_same_thing_over_and_over_is_the_defect() {
        let mut log = CoverageLog::new();
        for attempt in 1..=REPEAT_IS_A_DEFECT {
            log.observe(&started(GATE));
            log.observe(&aborted(OverworldActionAbortedReason::Script));
            let blocked = matches!(log.get(GATE).unwrap().verdict, Verdict::Blocked { times, .. } if times == attempt);
            assert!(blocked, "attempt {attempt}: {:?}", log.get(GATE).unwrap().verdict);
        }
        assert!(log.get(GATE).unwrap().verdict.is_defect(), "{REPEAT_IS_A_DEFECT} is too many");
        assert_eq!(log.defects().len(), 1, "{:?}", log.defects());
        assert!(log.defects()[0].contains(GATE));
    }

    #[test]
    fn a_row_the_agent_never_said_what_became_of_fails_the_walk() {
        let mut log = CoverageLog::new();
        log.observe(&started("Route18:39,13:Grass"));
        // Nothing closes it: the next decision opens on top of it.
        log.observe(&started("Route18:39,14:Grass"));
        log.observe(&aborted(OverworldActionAbortedReason::NothingAppeared));

        let quiet = log.get("Route18:39,13:Grass").unwrap();
        assert_eq!(quiet.verdict, Verdict::Silent);
        assert!(quiet.verdict.fails_the_walk(), "a silence has to make the tier red");
        assert!(!quiet.verdict.is_defect(), "and it is still not a defect: nothing failed to execute");

        assert!(log.defects().is_empty(), "{:?}", log.defects());
        assert_eq!(log.failures().len(), 1, "{:?}", log.failures());
        assert!(log.failures()[0].contains("Route18:39,13:Grass"), "{:?}", log.failures());
        assert!(log.summary().contains("1 silent, 0 defects"), "{}", log.summary());
    }

    /// The watchdog belongs to no id and is always a defect.
    #[test]
    fn a_watchdog_firing_fails_a_coverage_run_on_its_own() {
        let mut log = CoverageLog::new();
        log.observe(&AgentEvent::WatchdogFired {
            agent_state: "OverworldMovement".to_string(),
            stuck_for: std::time::Duration::from_secs(300),
        });
        assert_eq!(log.defects().len(), 1);
        assert!(log.defects()[0].contains("the watchdog fired"), "{:?}", log.defects());
        assert_eq!(log.len(), 0, "a watchdog firing belongs to no id");
    }

    /// The report has a row per id, and the summary counts what is in it.
    #[test]
    fn the_report_says_what_was_touched_and_what_was_not() {
        let mut log = CoverageLog::new();
        log.observe(&started(TREE));
        log.observe(&AgentEvent::OverworldActionCompleted { destination: MetaTile::Grass });
        log.offered("Route2:9,9:Warp");

        assert_eq!(log.len(), 2);
        assert_eq!(log.completed(), 1);
        assert_eq!(log.maps_touched(), 1);
        let report = log.report();
        assert!(report.contains(TREE) && report.contains("completed"), "{report}");
        assert!(report.contains("Route2:9,9:Warp") && report.contains("unreached"), "{report}");
        assert!(log.summary().contains("2 ids across 1 maps"), "{}", log.summary());
    }
}

#[cfg(feature = "slow-tests")]
/// A brain that takes every row it is offered, once, and then goes looking for more.
pub struct ExploringBrain {
    /// Every id ever offered, and how many times it has been chosen.
    seen: std::collections::BTreeMap<String, usize>,
    /// How many turns this brain has spent on each map, so leaving prefers somewhere new.
    maps: std::collections::BTreeMap<String, usize>,
    /// Times a door from one map to another has been taken, keyed `"{here}->{there}"`.
    exits: std::collections::BTreeMap<String, usize>,
    /// Turns since an id was first seen; the walk has settled once this reaches the patience.
    pub barren: usize,
    pub turns: usize,
    /// Set once the walk has played the game to its end, a terminus rather than a fault.
    pub reached_the_end: bool,
    /// Set the first time a turn is asked on `IndigoPlateauLobby`, the square the driver rewinds
    /// to.
    pub reached_the_lobby: bool,
    /// How many times the walk has been rewound past the credits.
    pub rewound: usize,
    /// Rows the walk must never choose again, whatever the frontier thinks of them.
    barred: std::collections::BTreeSet<String>,
    /// The map the last turn was asked on, for the `[walk]` heartbeat that tells slow from wedged.
    pub here: String,
    /// PC operations the walk has tried, by map.
    pc_ops_tried: std::collections::BTreeSet<(String, &'static str)>,
    /// Floors each lift has been ridden to, which no menu row leads to either.
    rides: std::collections::BTreeSet<(String, crate::pokemon::map::Map)>,
    /// Consecutive turns the menu has offered nothing at all, and the flies spent escaping it.
    pub rowless_turns: usize,
    /// Consecutive turns on which the menu had rows and the brain chose nothing: every row visited
    /// and no exit among them.
    pub stalled_turns: usize,
    pub stalled_worst: usize,
    pub boxed_in_at: Vec<String>,
}

#[cfg(feature = "slow-tests")]
/// The PC operations the walk exercises, in order, as (`move`, `op`, extra arguments).
const PC_OPS: [(&str, &str, &str); 4] = [
    ("pc_items", "deposit", r#""item":"PokeBall","quantity":1"#),
    ("pc_items", "withdraw", r#""item":"PokeBall","quantity":1"#),
    ("pc_pokemon", "deposit", r#""slot":5"#),
    ("pc_pokemon", "change_box", r#""box":2"#),
];

#[cfg(feature = "slow-tests")]
/// Consecutive rowless turns before the walk gives up on where it is standing and flies out.
const BOXED_IN_PATIENCE: usize = 20;

#[cfg(feature = "slow-tests")]
/// A row that leaves the map.
fn is_a_way_out(id: &str) -> bool {
    matches!(id.rsplit(':').next(), Some("Warp" | "Connection" | "ConnectionWater"))
}

#[cfg(feature = "slow-tests")]
impl Default for ExploringBrain {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "slow-tests")]
impl ExploringBrain {
    pub fn new() -> Self {
        Self {
            seen: std::collections::BTreeMap::new(),
            maps: std::collections::BTreeMap::new(),
            exits: std::collections::BTreeMap::new(),
            barren: 0,
            reached_the_end: false,
            reached_the_lobby: false,
            rewound: 0,
            barred: std::collections::BTreeSet::new(),
            here: String::new(),
            turns: 0,
            pc_ops_tried: std::collections::BTreeSet::new(),
            rides: std::collections::BTreeSet::new(),
            rowless_turns: 0,
            stalled_turns: 0,
            stalled_worst: 0,
            boxed_in_at: Vec::new(),
        }
    }

    pub fn carry_on_after_the_credits(&mut self, door: &str) {
        self.reached_the_end = false;
        self.rewound += 1;
        self.barred.insert(door.to_string());
        // The rewind puts the player back before the gauntlet, so the barren count starts again.
        self.barren = 0;
        self.stalled_turns = 0;
    }

    /// How many distinct PC operations the walk has taken, across all maps.
    pub fn pc_ops(&self) -> usize {
        self.pc_ops_tried.len()
    }

    /// Every id ever offered, so the log can mark the ones never chosen `Unreached`.
    pub fn offered_ids(&self) -> Vec<String> {
        self.seen.keys().cloned().collect()
    }

    pub fn discovered(&self) -> usize {
        self.seen.len()
    }

    pub fn visited(&self) -> usize {
        self.seen.values().filter(|taken| **taken > 0).count()
    }

    /// How many maps the walk has stood on.
    pub fn maps(&self) -> usize {
        self.maps.len()
    }

    pub fn map_turns(&self) -> &std::collections::BTreeMap<String, usize> {
        &self.maps
    }

    /// Whether the fixpoint has been reached: `barren` turns in a row with nothing new.
    pub fn settled(&self, patience: usize) -> bool {
        self.barren >= patience
    }

    /// How promising a way out is, low is better.
    fn promise_of(&self, description: &str) -> u8 {
        let known = self.maps_named(description);
        if known.is_empty() {
            return 0;
        }
        let has_work = known.iter().any(|map| {
            self.seen
                .iter()
                .any(|(id, taken)| *taken == 0 && id.starts_with(&format!("{map}:")))
        });
        match has_work {
            true => 1,
            false => 2,
        }
    }

    /// Every word in a row's prose that names a map this walk has an id for: all the brain may know
    /// of where a door goes.
    fn maps_named<'a>(&self, description: &'a str) -> Vec<&'a str> {
        description
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|word| self.seen.keys().any(|id| id.starts_with(&format!("{word}:"))))
            .collect()
    }

    /// The key into [`Self::exits`] for a way out of the current map: from here to where its prose
    /// says.
    fn crossing(&self, description: &str) -> Option<String> {
        let there = self.maps_named(description).first().map(|m| m.to_string())?;
        Some(format!("{}->{there}", self.here))
    }
}

#[cfg(feature = "slow-tests")]
impl crate::pokemon::integration_tests::llm_harness::Brain for ExploringBrain {
    fn respond(
        &mut self,
        request: &crate::pokemon::integration_tests::llm_harness::TurnRequest,
    ) -> crate::pokemon::integration_tests::llm_harness::Reply {
        use crate::pokemon::integration_tests::llm_harness::{Call, Reply};

        if request.is_summary() {
            return Reply::Content("Walking every action in the world, once each.".into());
        }
        if request.is_battle() {
            let id = request
                .menu_ids()
                .into_iter()
                .find(|id| id.starts_with("fight:"))
                .unwrap_or_else(|| "run".to_string());
            return Reply::call("choose_battle_action", serde_json::json!({ "id": id }));
        }
        if request.is_stuck() {
            return Reply::call(
                "press_buttons",
                serde_json::json!({ "buttons": ["a"], "why": "the agent reached no decision point" }),
            );
        }
        if !request.has_tool("choose_action") {
            // A nickname, a mart, a move to forget: answer with the game's default.
            for name in ["set_nickname", "forget_move", "buy_item"] {
                if request.has_tool(name) {
                    return Reply::call(name, serde_json::json!({}));
                }
            }
            return Reply::Calls(vec![Call::wait(1)]);
        }

        self.turns += 1;
        if let Some(map) = request.location() {
            self.here = map.clone();
            *self.maps.entry(map).or_insert(0) += 1;
        }

        // The PC, which no menu row leads to.
        if let Some(map) = request.location()
            && request.has_tool("use_field_move")
            && (map.ends_with("Pokecenter") || map == "RedsHouse2F" || map == "CeladonHotel")
        {
            if let Some((field_move, op, extra)) = PC_OPS.iter()
                .find(|(field_move, op, _)| {
                    !self.pc_ops_tried.contains(&(map.clone(), *field_move))
                        || !self.pc_ops_tried.contains(&(map.clone(), *op))
                })
                .filter(|(_, op, _)| self.pc_ops_tried.insert((map.clone(), *op)))
            {
                let arguments: serde_json::Value = serde_json::from_str(&format!(
                    r#"{{"move":"{field_move}","op":"{op}",{extra},"summary":"exercise the PC"}}"#
                )).expect("the arguments are valid JSON");
                return Reply::call("use_field_move", arguments);
            }
        }

        // A lift, once to each floor: on a floor only a lift reaches, it is the one way on.
        if let Some(here) = request.location()
            && request.has_tool("use_field_move")
            && let Some(lift) = <crate::pokemon::map::Map as strum::IntoEnumIterator>::iter().find(|map| format!("{map:?}") == here)
            && let Some((_, floors)) = crate::pokemon::tile_map::elevator_for(lift)
            && let Some(&floor) = floors.iter().find(|&&floor| !self.rides.contains(&(here.clone(), floor)))
        {
            self.rides.insert((here, floor));
            let arguments = serde_json::json!({
                "move": "elevator", "map": format!("{floor:?}"), "summary": "ride the lift",
            });
            return Reply::call("use_field_move", arguments);
        }

        // Reaching the Hall of Fame ends the walk, because it ends the game.
        if request.location().as_deref() == Some("HallOfFame") {
            self.reached_the_end = true;
        }
        if request.location().as_deref() == Some("IndigoPlateauLobby") {
            self.reached_the_lobby = true;
        }
        // Barred rows are dropped before anything counts them: each was chosen once and rewound out
        // of.
        let rows: Vec<(String, String)> = request.menu_rows().into_iter()
            .filter(|(id, _)| !self.barred.contains(id))
            .collect();
        // Counted before anything is chosen, so an empty menu is observed rather than inferred.
        if rows.is_empty() {
            self.rowless_turns += 1;
        }
        let mut anything_new = false;
        for (id, _) in &rows {
            if !self.seen.contains_key(id) {
                self.seen.insert(id.clone(), 0);
                anything_new = true;
            }
        }
        self.barren = match anything_new {
            true => 0,
            false => self.barren + 1,
        };

        // Prefer the first unvisited row that stays on the map, so a map is exhausted before it is
        // left; then the way out most likely to lead to work.
        let unvisited = |id: &String| self.seen.get(id).copied() == Some(0);

        // A row the world puts back, which the walk must be willing to take again.
        let re_takeable = |id: &String| !matches!(id.rsplit(':').next(), Some("Grass" | "Empty"));
        let chosen = rows
            .iter()
            .map(|(id, _)| id)
            .find(|id| unvisited(id) && !is_a_way_out(id))
            .cloned()
            .or_else(|| {
                rows.iter()
                    // Exits are never `Grass` or `Empty`, so this keeps every way out and every row
                    // that comes back.
                    .filter(|(id, _)| re_takeable(id))
                    // Times already taken first, the promise of where it goes second.
                    .min_by_key(|(id, description)| {
                        let id_times = self.seen.get(id).copied().unwrap_or(0);
                        let exit = is_a_way_out(id);
                        let map_times = match exit {
                            true => self.crossing(description)
                                .and_then(|crossing| self.exits.get(&crossing).copied())
                                .unwrap_or(id_times),
                            false => id_times,
                        };
                        // A row that is not a way out scores worse than any exit it ties with.
                        let promise = match exit { true => self.promise_of(description), false => 3 };
                        let new_map_worth_a_try = exit && promise == 0 && id_times < 2;
                        (!new_map_worth_a_try, map_times, promise, id_times)
                    })
                    .map(|(id, _)| id.clone())
            });

        // Nothing above: take the least-taken row again rather than wait.
        let chosen = chosen.or_else(|| {
            rows.iter()
                .min_by_key(|(id, _)| self.seen.get(id).copied().unwrap_or(0))
                .map(|(id, _)| id.clone())
        });

        match chosen {
            Some(id) => {
                self.stalled_turns = 0;
                *self.seen.entry(id.clone()).or_insert(0) += 1;
                // …and the same crossing against the map it leads to, whichever branch chose it.
                if is_a_way_out(&id)
                    && let Some(description) = rows.iter().find(|(i, _)| *i == id).map(|(_, d)| d)
                    && let Some(crossing) = self.crossing(description)
                {
                    *self.exits.entry(crossing).or_insert(0) += 1;
                }
                // `resume_after_battle` everywhere except the two rows that exist to start a
                // battle.
                let resume = !matches!(id.rsplit(':').next(), Some("Grass" | "Empty"));
                Reply::call(
                    "choose_action",
                    serde_json::json!({ "id": id, "resume_after_battle": resume }),
                )
            }
            None => {
                self.stalled_turns += 1;
                self.stalled_worst = self.stalled_worst.max(self.stalled_turns);
                if self.stalled_turns == BOXED_IN_PATIENCE {
                    let situation = request.messages.last().map(|m| m.text.as_str()).unwrap_or("");
                    let head: Vec<&str> = situation
                        .lines()
                        .filter(|l| l.starts_with("Location:") || l.starts_with("Blocked here:"))
                        .collect();
                    self.boxed_in_at.push(format!(
                        "{} | rows={} all-visited, exits={} :: {}",
                        head.join(" | "),
                        rows.len(),
                        rows.iter().filter(|(id, _)| is_a_way_out(id)).count(),
                        rows.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>().join(", "),
                    ));
                }
                Reply::Calls(vec![Call::wait(20)])
            }
        }
    }
}

pub struct Start {
    /// What `GB_COVERAGE_START` names it by.
    pub name: &'static str,
    /// The committed fixture, never written by a walk: exploration is destructive.
    pub state: &'static [u8],
    /// Where it stands, asserted by
    /// [`every_coverage_start_stands_where_it_says_on_a_finished_game`].
    pub map: crate::pokemon::map::Map,
    /// Set only for a start taken before the credits, with the reason.
    pub before_the_credits: Option<&'static str>,
    /// Whether the walk must earn its own way: `Cheats::story`, so the story's gates are live,
    /// rather than every key item and badge.
    pub story: bool,
}

/// The regional starts, one per region the `entry` walk never reaches, plus `entry` itself.
pub const COVERAGE_STARTS: &[Start] = &[
    Start {
        name: "entry",
        state: include_bytes!("../data/postgame-entry.bin"),
        map: crate::pokemon::map::Map::ViridianPokecenter,
        before_the_credits: None,
        story: false,
    },
    Start {
        name: "cerulean",
        state: include_bytes!("../data/postgame-daycare.bin"),
        map: crate::pokemon::map::Map::Route5,
        before_the_credits: None,
        story: false,
    },
    Start {
        name: "vermilion",
        state: include_bytes!("../data/postgame-farfetchd.bin"),
        map: crate::pokemon::map::Map::VermilionCity,
        before_the_credits: None,
        story: false,
    },
    Start {
        name: "lavender",
        state: include_bytes!("../data/postgame-sweep-lavender.bin"),
        map: crate::pokemon::map::Map::Route10,
        before_the_credits: None,
        story: false,
    },
    Start {
        name: "celadon",
        state: include_bytes!("../data/postgame-game-corner.bin"),
        map: crate::pokemon::map::Map::CeladonCity,
        before_the_credits: None,
        story: false,
    },
    Start {
        name: "saffron",
        state: include_bytes!("../data/postgame-silph-floors.bin"),
        map: crate::pokemon::map::Map::SaffronCity,
        before_the_credits: None,
        story: false,
    },
    Start {
        name: "fuchsia",
        state: include_bytes!("../data/postgame-safari.bin"),
        map: crate::pokemon::map::Map::FuchsiaCity,
        before_the_credits: None,
        story: false,
    },
    Start {
        name: "cinnabar",
        state: include_bytes!("../data/postgame-seel.bin"),
        map: crate::pokemon::map::Map::CinnabarIsland,
        before_the_credits: None,
        story: false,
    },
    // The only start that is not a finished game.
    Start {
        name: "ssanne",
        state: include_bytes!("../data/at-vermilion.bin"),
        map: crate::pokemon::map::Map::VermilionCity,
        before_the_credits: Some(
            "the S.S. Anne has not sailed yet, and `EVENT_SS_ANNE_LEFT` is what makes its ten \
             rooms and VermilionDock unreachable from every finished game"),
        story: false,
    },
    // The only start whose job is to remove a coin flip.
    Start {
        name: "ssanneship",
        state: include_bytes!("../data/on-the-ss-anne.bin"),
        map: crate::pokemon::map::Map::SSAnne1F,
        before_the_credits: Some(
            "it is standing on the S.S. Anne, which sails before the third badge and never comes \
             back, so no finished save can be cut here at all"),
        story: false,
    },
    // Mid-story: what a finished game has already opened is shut here, and the walk earns its way.
    Start {
        name: "rocktunnel",
        state: include_bytes!("../data/back-in-cerulean.bin"),
        map: crate::pokemon::map::Map::CeruleanCity,
        before_the_credits: Some("three badges, before Rock Tunnel, with every later gate still shut"),
        story: true,
    },
    Start {
        name: "hideout",
        state: include_bytes!("../data/at-rocket-hideout.bin"),
        map: crate::pokemon::map::Map::RocketHideoutB1F,
        before_the_credits: Some("four badges, with the Rocket Hideout and Silph Co still held"),
        story: true,
    },
    Start {
        name: "tower",
        state: include_bytes!("../data/post-silph-scope.bin"),
        map: crate::pokemon::map::Map::RocketHideoutB4F,
        before_the_credits: Some("the Silph Scope in the bag and Pokémon Tower still haunted"),
        story: true,
    },
    Start {
        name: "seafoam",
        state: include_bytes!("../data/post-marsh-badge.bin"),
        map: crate::pokemon::map::Map::SaffronGym,
        before_the_credits: Some("six badges, before Seafoam and Cinnabar"),
        story: true,
    },
];

/// What one walk came back with, so several can be summed without keeping their logs.
#[cfg(feature = "slow-tests")]
struct WalkOutcome {
    name: &'static str,
    ids: std::collections::BTreeSet<String>,
    unreached: std::collections::BTreeSet<String>,
    /// PC operations this walk took.
    pc_ops: usize,
    maps: std::collections::BTreeSet<String>,
    /// Everything that makes this walk red: defects and silences.
    failures: Vec<String>,
    turns: usize,
    game_time: std::time::Duration,
    wall: std::time::Duration,
    /// Why the walk stopped, so a multi-region summary says which regions settled.
    stopped: String,
}

#[test]
#[cfg(feature = "slow-tests")]
fn coverage_walk_of_the_finished_game() {
    // Game-minutes per walk, from `GB_COVERAGE_MINUTES`.
    let minutes: u64 = std::env::var("GB_COVERAGE_MINUTES").ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(90);

    let patience: usize = std::env::var("GB_COVERAGE_PATIENCE").ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or((60 * minutes.max(90) / 90) as usize);

    let wall_secs: u64 = std::env::var("GB_COVERAGE_WALL_SECS").ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60 + minutes * 3);

    // An unknown name fails rather than falling back to the default.
    let wanted = std::env::var("GB_COVERAGE_START").unwrap_or_else(|_| "entry".to_string());
    let starts: Vec<&Start> = match wanted.as_str() {
        "all" => COVERAGE_STARTS.iter().collect(),
        name => vec![COVERAGE_STARTS
            .iter()
            .find(|start| start.name == name)
            .unwrap_or_else(|| panic!(
                "GB_COVERAGE_START={name:?} names no start; the table holds {}",
                COVERAGE_STARTS.iter().map(|s| s.name).collect::<Vec<_>>().join(", ")))],
    };

    let outcomes: Vec<WalkOutcome> = starts
        .iter()
        .map(|start| walk_from(start, minutes, patience, wall_secs))
        .collect();

    if outcomes.len() > 1 {
        let maps: std::collections::BTreeSet<&String> =
            outcomes.iter().flat_map(|o| o.maps.iter()).collect();
        let ids: std::collections::BTreeSet<&String> =
            outcomes.iter().flat_map(|o| o.ids.iter()).collect();
        let rows: Vec<String> = outcomes
            .iter()
            .map(|o| format!(
                "  {:<10} {:>4} maps {:>5} ids {:>6} turns  {:>5.0}s wall  {}",
                o.name, o.maps.len(), o.ids.len(), o.turns, o.wall.as_secs_f64(), o.stopped))
            .collect();
        let only: Vec<String> = outcomes
            .iter()
            .map(|o| {
                let mine = o.maps.iter().filter(|map| {
                    !outcomes.iter().any(|other| other.name != o.name && other.maps.contains(*map))
                }).count();
                format!("{}:{mine}", o.name)
            })
            .collect();
        println!(
            "\n════ the regional sweep ════\n{}\n\
             union      {} maps, {} ids, over {} walks\n\
             only here  {}\n\
             cost       {:?} of game time, {:?} of wall clock in total\n\
             {}\n",
            rows.join("\n"),
            maps.len(),
            ids.len(),
            outcomes.len(),
            only.join(" "),
            outcomes.iter().map(|o| o.game_time).sum::<std::time::Duration>(),
            outcomes.iter().map(|o| o.wall).sum::<std::time::Duration>(),
            unreached_report(&maps),
        );

        let taken: std::collections::BTreeSet<&String> = outcomes
            .iter()
            .flat_map(|o| o.ids.difference(&o.unreached))
            .collect();
        let never: std::collections::BTreeSet<&String> = outcomes
            .iter()
            .flat_map(|o| o.unreached.iter())
            .filter(|id| !taken.contains(id))
            .collect();
        let mut by_kind: std::collections::BTreeMap<String, usize> = Default::default();
        for id in &never { *by_kind.entry(kind_of(id)).or_insert(0) += 1 }
        let mut ranked: Vec<(String, usize)> = by_kind.into_iter().collect();
        ranked.sort_by_key(|(kind, n)| (std::cmp::Reverse(*n), kind.clone()));
        println!(
            "unreached  {} of {} ids were offered somewhere and chosen nowhere ({:.0}%)\n\
             by kind    {}\n",
            never.len(), ids.len(),
            100.0 * never.len() as f64 / ids.len().max(1) as f64,
            ranked.iter().map(|(kind, n)| format!("{kind}:{n}")).collect::<Vec<_>>().join(" "));

        let (report, kind_failures) =
            kind_cross_check(&ids, outcomes.iter().map(|o| o.pc_ops).sum());
        println!("{report}");
        match minutes >= COVERAGE_BUDGET_MINUTES {
            true => assert!(kind_failures.is_empty(),
                "the sweep was never offered {} kind(s) of row the game has:\n  {}",
                kind_failures.len(), kind_failures.join("\n  ")),
            false => println!(
                "           not asserted: {minutes} game-minutes per walk is below the \
                 {COVERAGE_BUDGET_MINUTES} a coverage sweep spends, and a short walk cannot be \
                 expected to have seen every kind"),
        }
    }

    // Every region walks before any assertion.
    let failures: Vec<String> = outcomes
        .iter()
        .flat_map(|o| o.failures.iter().map(|d| format!("[{}] {d}", o.name)))
        .collect();
    assert!(failures.is_empty(), "the walk found {} failures:\n  {}",
            failures.len(), failures.join("\n  "));
    let discovered: usize = outcomes.iter().map(|o| o.ids.len()).sum();
    assert!(discovered > 10, "only {discovered} ids were ever offered; the walk did not happen");
}

#[cfg(feature = "slow-tests")]
fn unreached_report(entered: &std::collections::BTreeSet<&String>) -> String {
    use crate::pokemon::map::Map;
    use strum::IntoEnumIterator;
    let (mut padding, mut cable, mut duplicates, mut real) = (0usize, 0usize, 0usize, Vec::new());
    let mut reachable = 0usize;
    for map in Map::iter() {
        let name = format!("{map:?}");
        let bucket = classify(map, &name);
        if bucket == MapBucket::Reachable { reachable += 1 }
        if entered.contains(&name) {
            continue;
        }
        match bucket {
            MapBucket::Padding => padding += 1,
            MapBucket::LinkCable => cable += 1,
            MapBucket::Duplicate => duplicates += 1,
            MapBucket::Reachable => real.push(name),
        }
    }
    real.sort();
    // Wrapped rather than one per line, to scan for a cluster.
    let mut lines: Vec<String> = vec![format!(
        "unreached  {} of {reachable} reachable maps, plus {padding} UnusedMap*, \
         {cable} link-cable rooms and {duplicates} unreachable duplicates",
        real.len())];
    for chunk in real.chunks(6) {
        lines.push(format!("           {}", chunk.join(" ")));
    }
    lines.join("\n")
}

#[cfg(feature = "slow-tests")]
use super::completion::{classify, MapBucket};

#[cfg(feature = "slow-tests")]
/// Game-minutes per walk below which a sweep is a smoke run and the union checks only print.
const COVERAGE_BUDGET_MINUTES: u64 = 180;

#[cfg(feature = "slow-tests")]
/// What an id ends in: [`MetaTile::id_kind`], the name a family of rows shares.
pub fn kind_of(id: &str) -> String {
    match id.split(':').count() {
        0 | 1 | 2 => "Sprite".to_string(),
        _ => id.rsplit(':').next().unwrap_or("?").to_string(),
    }
}

#[cfg(feature = "slow-tests")]
fn kind_cross_check(offered: &std::collections::BTreeSet<&String>, pc_ops: usize)
    -> (String, Vec<String>) {
    use crate::pokemon::tile::{HiddenObject, JumpDirection, MetaTile};
    use crate::pokemon::postgame::fishing::Rod;
    use crate::pokemon::map::Map;
    use gb::geometry::Point8;

    /// Whether `MetaTileMap::actions` can put a `MetaTile` on the menu, and if not, why.
    enum Expect {
        /// `actions()` mints this, so a sweep of all Kanto must have been offered one.
        Row,
        /// Terrain, not a decision: it classifies a square and is never a row of its own.
        Terrain,
        /// A row this harness is expected not to see, with the reason.
        Absent(&'static str),
    }

    let here = Point8 { x: 0, y: 0 };
    let every = [
        MetaTile::Empty, MetaTile::Obstacle, MetaTile::Water, MetaTile::Counter,
        MetaTile::Jump(JumpDirection::South),
        MetaTile::Sprite("Youngster"),
        MetaTile::Warp { to_map: Map::PalletTown, to_position: here },
        MetaTile::Connection { to_map: Map::PalletTown, to_position: here },
        MetaTile::ConnectionWater(Map::PalletTown),
        MetaTile::CutTree, MetaTile::Cut { at: here },
        MetaTile::BoulderGoal { boulder: here, at: here, hole: false },
        MetaTile::BoulderGoal { boulder: here, at: here, hole: true },
        MetaTile::Pc, MetaTile::Grass, MetaTile::Fish { rod: Rod::Old },
        MetaTile::Switch { object: HiddenObject::TrashCan, ordinal: 1 },
        MetaTile::Switch { object: HiddenObject::VendingMachine, ordinal: 1 },
        MetaTile::Switch { object: HiddenObject::Poster, ordinal: 1 },
        MetaTile::Switch { object: HiddenObject::Statue, ordinal: 1 },
        MetaTile::Switch { object: HiddenObject::CellSeparator, ordinal: 1 },
    ];
    let expectation = |tile: &MetaTile| match tile {
        MetaTile::Empty | MetaTile::Obstacle | MetaTile::Counter | MetaTile::Jump(_) => Expect::Terrain,
        // Water is crossed rather than chosen: its rows are `ConnectionWater`, a `Fish` square, or
        // a walk that mounts Surf.
        MetaTile::Water => Expect::Terrain,
        // Withheld from the action menu, and covered by the PC operations.
        MetaTile::Pc => Expect::Absent(
            "withheld from the action menu on purpose (llm::tools); the walk drives it through \
             use_field_move instead, and pc ops are counted separately"),
        // Bill's cell separator is offered only while pressing it does something
        // (`MetaTileMap::bill_cell_separator`), and every start is past Bill.
        MetaTile::Switch { object: HiddenObject::CellSeparator, .. } => Expect::Absent(
            "only offered before Bill has been turned back into a person, which every coverage \
             start is long past"),
        _ => Expect::Row,
    };

    let sprite_seen = offered.iter().any(|id| id.split(':').count() == 2);
    let mut lines: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    let (mut covered, mut expected) = (0usize, 0usize);
    // `CutTree` is two variants — the terrain and the whole cut — and one kind.
    let mut said_already: std::collections::BTreeSet<String> = Default::default();
    for tile in &every {
        let kind = tile.id_kind().to_string();
        if !said_already.insert(kind.clone()) { continue }
        let seen = match tile {
            MetaTile::Sprite(_) => sprite_seen,
            _ => offered.iter().any(|id| kind_of(id) == kind
                // A `Switch` id is the object plus an ordinal, so the family is a prefix.
                || (matches!(tile, MetaTile::Switch { .. })
                    && kind_of(id).trim_end_matches(|c: char| c.is_ascii_digit()) == kind.trim_end_matches(|c: char| c.is_ascii_digit()))),
        };
        match expectation(tile) {
            Expect::Terrain => {}
            Expect::Absent(why) => lines.push(format!(
                "  {kind:<24} {} — expected: {why}",
                if seen { "offered (and it was not expected to be)" } else { "never offered" })),
            Expect::Row => {
                expected += 1;
                if seen { covered += 1; continue }
                failures.push(format!(
                    "no row of kind {kind:?} was offered anywhere in the sweep"));
                lines.push(format!("  {kind:<24} ⛔ never offered, and it is not on the allow-list"));
            }
        }
    }
    // The `Pc` allow-list entry holds only while the PC operations run, so their count is checked.
    if pc_ops == 0 {
        failures.push("no PC operation was taken anywhere in the sweep, and `MetaTile::Pc` is \
                       allow-listed on the grounds that the walk covers it that way instead".into());
    }
    let report = format!(
        "\n──── every kind of row, against what was offered ────\n\
         kinds      {covered} of {expected} offered somewhere in the sweep; {pc_ops} PC operations\n\
         {}\n",
        if lines.is_empty() { "  every kind was offered".to_string() } else { lines.join("\n") });
    (report, failures)
}

/// One walk, from one [`Start`].
#[cfg(feature = "slow-tests")]
fn walk_from(start: &Start, minutes: u64, patience: usize, wall_secs: u64) -> WalkOutcome {
    use crate::pokemon::integration_tests::cheats::Cheats;
    use crate::pokemon::integration_tests::llm_harness::LlmRun;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    let budget = Duration::from_mins(minutes);

    #[derive(Clone)]
    struct Shared(Arc<Mutex<ExploringBrain>>);

    impl crate::pokemon::integration_tests::llm_harness::Brain for Shared {
        fn respond(
            &mut self,
            request: &crate::pokemon::integration_tests::llm_harness::TurnRequest,
        ) -> crate::pokemon::integration_tests::llm_harness::Reply {
            self.0.lock().expect("not poisoned").respond(request)
        }
    }

    let brain = Shared(Arc::new(Mutex::new(ExploringBrain::new())));
    let mut run = LlmRun::builder(start.state)
        .named("coverage-walk")
        // Twice the budget, so the walk stops on its own bound rather than the fixture's panic.
        .game_time(budget * 2)
        .with_coverage()
        .start(Box::new(brain.clone()));
    // The bag is what makes the overworld fully offered, except where the story is the point.
    run.with_cheats(match start.story {
        true => Cheats::story(999_999),
        false => Cheats::default().with_key_items(999_999),
    });

    /// Wall-clock seconds between progress lines.
    const BEAT_SECS: u64 = 30;

    let started = std::time::Instant::now();
    let mut spent_the_budget = false;
    /// The door into Lorelei's room, which a rewound walk must never take again.
    const ELITE_FOUR_DOOR: &str = "IndigoPlateauLobby:8,0:Warp";
    const MAX_REWINDS: usize = 1;
    let mut checkpointed = false;
    let mut beat = started;
    let mut beat_turns = 0usize;
    let mut beat_visited = 0usize;
    let name = start.name;
    let settled = run.tick_until(Duration::from_secs(wall_secs), |run| {
        spent_the_budget |= run.fixture().total_cycles.to_duration() >= budget;
        if beat.elapsed().as_secs() >= BEAT_SECS {
            beat = std::time::Instant::now();
            let (turns, visited, maps, here) = {
                let brain = brain.0.lock().expect("not poisoned");
                (brain.turns, brain.visited(), brain.maps(), brain.here.clone())
            };
            let game = run.fixture().total_cycles.to_duration();
            let wall = started.elapsed();
            // `turns/min` spots a livelock: a wedged walk still discovers rows and burns game time.
            let per_min = (turns - beat_turns) as u64 * 60 / BEAT_SECS;
            let warn = if turns - beat_turns <= 2 { "  NOT MOVING" } else { "" };
            println!("[walk:{name}] {wall:>5.0}s wall {game:>6.0}s game ({rate:>4.1}x) | {turns} turns \
                      (+{per_min}/min) | {visited} chosen (+{new_ids}) | {maps} maps | on {here}{warn}",
                wall = wall.as_secs_f64(), game = game.as_secs_f64(),
                rate = game.as_secs_f64() / wall.as_secs_f64().max(0.001),
                new_ids = visited - beat_visited);
            (beat_turns, beat_visited) = (turns, visited);
        }
        let (at_the_lobby, won, rewound) = {
            let brain = brain.0.lock().expect("not poisoned");
            (brain.reached_the_lobby, brain.reached_the_end, brain.rewound)
        };
        if at_the_lobby && !checkpointed {
            checkpointed = true;
            run.checkpoint();
            println!("[walk:{name}] checkpointed at the Indigo Plateau lobby; the credits are a \
                      rewind from here rather than the end of the walk");
        }
        if won && checkpointed && rewound < MAX_REWINDS {
            let game = run.fixture().total_cycles.to_duration();
            println!("[walk:{name}] the Hall of Fame, at {:.0}% of the {budget:?} budget — \
                      rewinding to the lobby and spending the rest of it walking",
                     100.0 * game.as_secs_f64() / budget.as_secs_f64());
            run.restart_from_last_checkpoint();
            brain.0.lock().expect("not poisoned").carry_on_after_the_credits(ELITE_FOUR_DOOR);
            return false;
        }
        let brain = brain.0.lock().expect("not poisoned");
        spent_the_budget || brain.reached_the_end || brain.settled(patience)
    }) && !spent_the_budget;
    let elapsed = started.elapsed();

    let (discovered, visited, maps, turns, pc_ops) = {
        let brain = brain.0.lock().expect("not poisoned");
        (brain.discovered(), brain.visited(), brain.maps(), brain.turns, brain.pc_ops())
    };
    let offered = brain.0.lock().expect("not poisoned").offered_ids();
    let (stalled_worst, rowless, boxed_at) = {
        let brain = brain.0.lock().expect("not poisoned");
        (brain.stalled_worst, brain.rowless_turns, brain.boxed_in_at.clone())
    };
    let busiest = {
        let brain = brain.0.lock().expect("not poisoned");
        let mut by_turns: Vec<(String, usize)> =
            brain.map_turns().iter().map(|(m, n)| (m.clone(), *n)).collect();
        by_turns.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        by_turns.truncate(8);
        by_turns.iter().map(|(m, n)| format!("{m}:{n}")).collect::<Vec<_>>().join(" ")
    };
    let game_time = run.fixture().total_cycles.to_duration();
    // The one cheat that can silently fail.
    let refused: Vec<String> = run.cheats.as_ref()
        .map(|c| c.bag_was_full.iter().map(|i| format!("{i:?}")).collect())
        .unwrap_or_default();
    let shed: Vec<String> = run.cheats.as_ref()
        .map(|c| c.bag_was_shed.iter().map(|id| match crate::pokemon::item::ItemId::from_repr(*id) {
            Some(named) => format!("{named:?}"),
            None => format!("${id:02x}"),
        }).collect())
        .unwrap_or_default();
    let bag = match refused.is_empty() {
        true => format!("every key item fit the bag; {} shed to make room: {}",
            shed.len(), shed.join(", ")),
        false => format!("the bag was full and refused {} — whatever they gate is unreachable: {} \
                          (and {} were shed to make room, so this is not the junk: {})",
            refused.len(), refused.join(", "), shed.len(), shed.join(", ")),
    };
    {
        let log = run.fixture().coverage.as_mut().expect("coverage was asked for");
        for id in &offered {
            log.offered(id);
        }
    }
    let log = run.coverage().expect("coverage was asked for");
    // Named after the start, so regional sweeps do not overwrite each other.
    let written = log.write_report(&format!("walk-{name}"));

    // Three ways to stop, and they are not interchangeable.
    let stopped = match (settled, spent_the_budget) {
        _ if brain.0.lock().expect("not poisoned").reached_the_end => format!(
            "the walk played the game to the Hall of Fame and stopped there, which is a \
             terminus rather than a fault: the cartridge saves and soft-resets to the title \
             screen, and there is no world left to walk. {:.0}% of the {budget:?} game-time \
             budget was spent getting there",
            100.0 * game_time.as_secs_f64() / budget.as_secs_f64()),
        (true, _) => format!("{patience} turns with nothing new"),
        (false, true) => format!("stopped on the {budget:?} game-time budget with the frontier still open"),
        (false, false) => format!(
            "CUT OFF after {elapsed:?} of WALL CLOCK with only {:.0}% of the {budget:?} \
             game-time budget spent — raise GB_COVERAGE_WALL_SECS, or find out what is running \
             this slowly",
            100.0 * game_time.as_secs_f64() / budget.as_secs_f64()),
    };

    println!(
        "\n════ a walk of the finished game, from {name} ({:?}) ════\n\
         frontier   {discovered} ids offered, {visited} chosen, across {maps} maps in {turns} turns\n\
         cost       {game_time:?} of game time, {elapsed:?} of wall clock\n\
         rate       {:.1} ids discovered per game-minute\n\
         settled    {settled} ({stopped})\n\
         verdicts   {}\n\
         unreached  {} offered and never chosen, by kind: {:?}\n\
         silent     {:?}\n\
         busiest    {busiest}\n\
         stuck      {stalled_worst} consecutive turns choosing nothing; {rowless} turn(s) had no rows at all\n\
         cheats     {bag}\n\
         where      {boxed_at}\n\
         table      {written:?}\n",
        start.map,
        discovered as f64 / (game_time.as_secs_f64() / 60.0).max(0.001),
        log.summary(),
        log.entries().filter(|e| e.verdict == Verdict::Unreached).count(),
        log.unreached_kinds(),
        log.silent_kinds(),
        busiest = busiest,
        stalled_worst = stalled_worst,
        rowless = rowless,
        boxed_at = boxed_at.join("\n            "),
        bag = bag,
    );

    // Owned, so the borrow of the run's log ends before the cross-check borrows its MMU.
    let ids: std::collections::BTreeSet<String> =
        log.entries().map(|entry| entry.id.clone()).collect();
    let unreached: std::collections::BTreeSet<String> = log.entries()
        .filter(|entry| entry.verdict == Verdict::Unreached)
        .map(|entry| entry.id.clone())
        .collect();
    let failures = log.failures();

    println!("{}", rom_cross_check(run.fixture().gb.core().mmu(), &ids));

    // Maps come off the ids, so the union is the same arithmetic as `CoverageLog::maps_touched`.
    let maps = ids.iter().filter_map(|id| id.split(':').next().map(str::to_string)).collect();
    WalkOutcome {
        name,
        ids,
        unreached,
        pc_ops,
        maps,
        failures,
        turns,
        game_time,
        wall: elapsed,
        stopped,
    }
}

pub fn rom_cross_check(
    mmu: &gb::mmu::MMU,
    offered: &std::collections::BTreeSet<String>,
) -> String {
    use crate::pokemon::map::Map;
    use crate::pokemon::map_metadata::MapMetadataCache;
    use crate::pokemon::tile::MetaTile;
    use strum::IntoEnumIterator;

    let by_name: std::collections::HashMap<String, Map> =
        Map::iter().map(|m| (m.to_string(), m)).collect();
    let visited: std::collections::BTreeSet<Map> = offered
        .iter()
        .filter_map(|id| id.split(':').next())
        .filter_map(|name| by_name.get(name).copied())
        .collect();

    let cache = MapMetadataCache::default();
    let mut lines: Vec<String> = Vec::new();
    let (mut warps, mut warps_missing, mut same_door, mut impassable) = (0, 0, 0, 0);
    let (mut objects, mut npcs_missing, mut gated_missing, mut boulders_missing) = (0, 0, 0, 0);
    let mut unreadable: Vec<String> = Vec::new();

    for map in &visited {
        let map = *map;
        let metadata = match cache.read_map(mmu, map) {
            Ok(metadata) => metadata,
            // A handful of maps are drawn from RAM rather than the ROM (`map_uses_runtime_blocks`).
            Err(why) => { unreadable.push(format!("{map}: {why}")); continue }
        };
        let dims = metadata.dimensions();
        let was_offered = |id: &str| offered.contains(id);
        let mut said: Vec<String> = Vec::new();

        // Where a warp would be minted: the ROM's square shifted by connection strips, as
        // `meta_tiles_base` places it.
        let square = |warp: &crate::pokemon::tile::WarpEvent| {
            (warp.position.x as usize + dims.west_extra, warp.position.y as usize + dims.north_extra)
        };
        for warp in &metadata.warp_events {
            warps += 1;
            let (mx, my) = square(warp);
            if was_offered(&format!("{map}:{mx},{my}:Warp")) { continue }
            warps_missing += 1;
            // `actions()` dedupes on destination, so an offered sibling to the same place explains
            // this one.
            let sibling = metadata.warp_events.iter()
                .filter(|other| other.destination_map == warp.destination_map
                             && other.destination_position == warp.destination_position)
                .map(square)
                .find(|(ox, oy)| (*ox, *oy) != (mx, my) && was_offered(&format!("{map}:{ox},{oy}:Warp")));
            let on_grid = metadata.meta_tiles_base
                .get(mx + my * dims.full_width())
                .copied()
                .unwrap_or(MetaTile::Obstacle);
            match (sibling, on_grid) {
                (Some((ox, oy)), _) => { same_door += 1;
                    said.push(format!("({mx}, {my}) → {}: the same door as ({ox}, {oy}), which was offered",
                                      warp.destination_map)) }
                (None, tile) if !matches!(tile, MetaTile::Warp { .. }) => { impassable += 1;
                    said.push(format!("({mx}, {my}) → {}: no walkable sub-tile, so it is not on the grid at all",
                                      warp.destination_map)) }
                (None, _) => said.push(format!(
                    "({mx}, {my}) → {}: on the grid, no sibling, and never a row",
                    warp.destination_map)),
            }
        }

        for sprite in map.sprites() {
            objects += 1;
            let id = format!("{map}:{}", MetaTile::Sprite(sprite.name).id_kind());
            if was_offered(&id) { continue }
            match sprite.hidden_object_id {
                // A toggleable object: an item ball already taken, or something a script has not
                // placed yet.
                Some(_) => gated_missing += 1,
                // A boulder is a sprite and gets a talk row (`VictoryRoad1F:Boulder1`), so it
                // belongs in this check.
                None if sprite.name.starts_with("Boulder") => boulders_missing += 1,
                None => { npcs_missing += 1;
                    said.push(format!("{:?}: a person on this map who was never a row", sprite.name)) }
            }
        }

        // Connections are counted rather than matched: a `Connection` id names the crossing tile,
        // not the neighbour.
        let neighbours = [&metadata.map_header.north_connection, &metadata.map_header.south_connection,
                          &metadata.map_header.east_connection,  &metadata.map_header.west_connection]
            .iter().filter(|c| c.is_some()).count();
        let prefix = format!("{map}:");
        let rows = offered.iter()
            .filter(|id| id.starts_with(&prefix) && id.rsplit(':').next() == Some("Connection"))
            .count();
        if neighbours > 0 && rows < neighbours {
            said.push(format!("connections: {rows} row(s) for {neighbours} neighbour(s) in the header"));
        } else if rows > neighbours {
            said.push(format!("connections: {rows} ids for {neighbours} neighbour(s) — the crossing \
                               coordinate is moving with the player"));
        }

        if !said.is_empty() {
            lines.push(format!("  {map}\n    {}", said.join("\n    ")));
        }
    }

    format!(
        "\n──── the ROM's tables against what was offered ────\n\
         scope      {} maps the walk entered, of {} in the game\n\
         warps      {warps} in those headers; {warps_missing} never a row \
         ({same_door} the same door as one that was, {impassable} not on the grid)\n\
         objects    {objects} in those sprite tables; {npcs_missing} people never a row \
         ({gated_missing} toggleable objects and {boulders_missing} boulders also, both expected)\n\
         unreadable {}\n\
         {}\n",
        visited.len(), Map::iter().count(),
        if unreadable.is_empty() { "none".to_string() } else { unreadable.join("; ") },
        if lines.is_empty() { "  nothing to report".to_string() } else { lines.join("\n") },
    )
}
