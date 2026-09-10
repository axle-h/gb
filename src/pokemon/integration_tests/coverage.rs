//! **C3** — what a run actually touched, and whether the agent could carry it out.
//!
//! `docs/coverage-plan.md` §5. This module is the *oracle* half: a [`CoverageLog`] that consumes the
//! agent's own event stream and gives every action id it sees one of four verdicts. The frontier
//! brain that goes looking for unvisited ids is built on top of it.
//!
//! ⚠️ **Over [`AgentEvent`], not over the policy**, and that is what makes it worth having. It works
//! under *any* driver — the scripted playthrough, a random soak, the LLM harness, a leg test — so
//! wiring it into the tests that already exist costs nothing and says how much of the world they
//! already touch. That number is what decides how much an exhaustive walk is really worth.
//!
//! ## The verdicts
//!
//! | Reason | Verdict |
//! |---|---|
//! | completed, interaction completed | `Completed` |
//! | `Battle`, `NamingScreen` | expected. The id is left open to be retried |
//! | `Textbox`, `Script` | `Blocked` — expected **once**; a repeat is the signal |
//! | `Unknown`, `DidNotArrive`, `NoRoute`, `WrongMap`, `NoAdjacentGrass`, `CastRefused`, `CastNeverFinished` | `Defect` |
//! | `WatchdogFired` | `Defect`, always |
//! | `NothingAppeared` | `Completed` — the pace ran its budget; the empty roll is the game, not a fault |
//! | a start with no terminal event before the next one | `Silent` — chosen, outcome never reported |
//!
//! ⭐ **`Defect` and `Silent` both make the tier red** ([`Verdict::fails_the_walk`],
//! [`CoverageLog::failures`]); the counts stay apart because they are different faults, and only
//! `Defect` is a row the agent could not carry out.
//!
//! ⚠️ **A repeat of `Textbox`/`Script` is the signal, not the first one.** Being stopped is how this
//! game says almost everything — a guard, a locked door, an errand — so the first one is the game
//! working. The deployed run of 2026-09-02 aborted on `the way into Route2` **143 times** because
//! the Viridian old man blocks the north exit until Oak's Parcel is delivered; one of those is a
//! fact about the world and a hundred and forty-three is a run that has stopped learning.
//! [`Verdict::Blocked`] therefore carries the count, and [`CoverageLog::defects`] reports a blocked
//! id that was retried past [`REPEAT_IS_A_DEFECT`] as one.

use std::collections::BTreeMap;

use crate::pokemon::agent::{AgentEvent, OverworldActionAbortedReason};

/// How many times an id may be blocked by the game speaking before the repetition is itself the
/// finding.
///
/// ⚠️ **It was 3, and 3 is wrong — measured.** The reasoning for 3 was "the second attempt is
/// already a model that did not read the answer" and it ignores the honest case: a gate is worth
/// **one** try per pass, because the thing that opens it may have happened since. C3's walk from
/// Pallet Town re-tried Pewter's east exit — Brock's gym guide, who blocks it until the Boulder
/// Badge — once on each of three sweeps of the map, and was called a defect for diligence.
///
/// Ten, because it has to sit clearly above "once per pass over a map the walk keeps coming back
/// to" and clearly below the number that made this a rule: the deployed run of 2026-09-02 aborted
/// on the way into Route 2 **143 times**.
pub const REPEAT_IS_A_DEFECT: usize = 10;

/// What became of one action id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Never reached: seen in a menu, never chosen. Only a frontier walk can produce this; a log
    /// over events alone never sees an id it was not asked to take.
    Unreached,
    /// The walk arrived, or the conversation happened.
    Completed,
    /// The game stopped the player to say something, and here is what it said. Expected once.
    Blocked { times: usize, message: Option<String> },
    /// The menu offered a row the agent could not then execute.
    ///
    /// ⚠️ **`at` is the square the walk was standing on when it gave up**, off
    /// `AgentEvent::OverworldActionAborted`'s own `at`, and it is here for the reason that field
    /// exists at all: a reason on its own is not something anyone can act on. A `NoRoute` names a
    /// row `MetaTileMap::actions()` minted and the walk then could not re-derive, and the *only*
    /// thing that changes between those two moments is where the player is standing. Without it the
    /// next person to read this table has to reproduce a ninety-second walk to learn one coordinate.
    Defect { reason: String, at: Option<crate::geometry::Point8> },
    /// ⭐ **The action was taken and the agent never said what became of it.** A
    /// `StartedOverworldAction` with no terminal event before the next one.
    ///
    /// ⚠️ **This is a gap in the *event stream*, not in the agent's behaviour, and C3 found it on
    /// its first walk from Pallet Town** — 66 of 307 chosen ids, and **every single one** of them a
    /// `Grass` (52) or a `CutTree` (14).
    ///
    /// ✅ **Both are closed** (2026-09-07), and they were two different mechanisms with one shape.
    /// Reaching tall grass hands over to `AgentState::PacingForEncounters`, which used to leave by
    /// three doors and report through none of them: a battle fell through `assert_battle_state`'s
    /// `_` arm, which emits `BattleStarted` and no abort; the budget expiring emitted a `TextBox`
    /// the agent had made up; and a map change emitted nothing at all. All three now end the action
    /// they belong to, the budget one through
    /// [`OverworldActionAbortedReason::NothingAppeared`], which is scored a completion above. A cut
    /// ended in a made-up `TextBox` too and now ends in `OverworldActionCompleted { Cut }`.
    ///
    /// ✅ **And the fishing row and the trainer who notices you in tall grass are closed too**
    /// (2026-09-09, `docs/coverage-plan.md` step 1), which were the whole of the 2026-09-09
    /// baseline's 41 silences. A cast refused, wedged or sent to a shore it could not reach ended in
    /// a `TextBox` the agent made up and nothing else; and a pace whose action was taken away from
    /// outside — a trainer's walk-up commits as `GameMode::Script` — reported through neither the
    /// script door nor the text-box one, because both knew only about `OverworldMovement`. See
    /// `postgame::fishing::tick` and `AgentState::open_overworld_action`.
    ///
    /// ⚠️ **A boulder push is no longer one of these, and the note that said so is retired.** The
    /// shove is still invisible — it runs as `GameMode::Script` and the driver's state is taken away
    /// before it can report — but the row is a `BoulderGoal` now and the *goal* completes, so a
    /// `PushBoulder*` id scores `Completed`: 52 of them across six sweeps of 2026-09-09, none
    /// silent. That is why the assertion below needs no exemption for it.
    ///
    /// ⭐ **It fails the walk** ([`Verdict::fails_the_walk`]), which it did not until step 1 closed
    /// the last family. Nothing goes wrong in the *game* when an action goes quiet — what is missing
    /// is the sentence — but the sentence is the whole product here: a model that chose a row and is
    /// told nothing about it is the failure mode this tier exists to find, and a silence nobody
    /// fails on is a silence nobody reads (`docs/coverage-plan.md` §7.2.7).
    Silent,
}

impl Verdict {
    /// Whether this verdict is a defect *proper*: the agent could not do what the menu offered.
    ///
    /// ⚠️ **Not the same question as [`Self::fails_the_walk`]**, and keeping them apart is what
    /// keeps [`CoverageLog::summary`]'s counts from double-counting a silence as a defect as well.
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
    /// `{map}:{x},{y}:{kind}` — [`OverworldAction::id`](crate::pokemon::actions::OverworldAction::id).
    pub id: String,
    /// How many times the id was chosen.
    pub attempts: usize,
    pub verdict: Verdict,
    /// Every abort reason this id has produced, and how often. Kept even once the id completes: an
    /// id that took nine tries and then worked is a different thing from one that worked first time.
    pub aborts: BTreeMap<String, usize>,
}

/// The table, plus the two failures that belong to no id.
#[derive(Debug, Default)]
pub struct CoverageLog {
    entries: BTreeMap<String, Entry>,
    /// The id whose walk is in flight, if any. `AgentEvent::StartedOverworldAction` opens it and the
    /// next terminal overworld event closes it.
    ///
    /// ⚠️ **Pairing is positional because only the *start* carries an id.** The terminal events
    /// carry a `MetaTile` and, for an abort, where the walk stopped — neither of which identifies
    /// the row that was chosen. The agent only ever has one walk in flight, which is what makes this
    /// sound — so a start arriving while one is open is not two walks, it is one that ended without
    /// saying so, and the open id gets [`Verdict::Silent`]. The pair is kept in
    /// [`Self::interleaved`] as `(what was open, what started)` because the *shape* of that list is
    /// what names the gap: on the first walk from Pallet Town every pair's first half was a `Grass`,
    /// a `CutTree` or a warp.
    open: Option<String>,
    pub interleaved: Vec<(String, String)>,
    /// Every watchdog firing, with the agent state it fired in. ⚠️ **Always a defect**: in a healthy
    /// run the agent never goes a whole timeout without reaching a decision point of any kind.
    pub watchdog: Vec<String>,
    /// Ids whose verdict became a hard [`Verdict::Defect`] since the last call to
    /// [`Self::take_new_defects`].
    ///
    /// ⚠️ **This exists so the *save state* can be taken where the defect happened**, which
    /// `docs/coverage-plan.md` §5.5 asks for and nothing else can supply: exploration is destructive
    /// and one-shot, so by the end of a ninety-second walk the world has moved on and the failing
    /// square cannot be revisited. A driver drains events every tick, so this is the one moment the
    /// emulator is still standing where it went wrong.
    new_defects: Vec<String>,
    /// The last thing the game said, so a `Blocked` verdict can quote it.
    ///
    /// ⚠️ **The message arrives *after* the abort**, not before: Pokémon Red turns the player back
    /// by printing a message and then running a script that steps them backwards, so the walk is
    /// abandoned first and the text reader is drained afterwards. The quote is therefore attached to
    /// the id that was open when the box closed, which is why `open` is cleared lazily.
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
                // ⚠️ **A start while one is open means the previous action ended without saying
                // so.** The agent only ever has one walk in flight, so this is not two at once: it
                // is one that went quiet. See [`Verdict::Silent`].
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
            // ⚠️ **A pickup that failed is not a defect and not a success.** Every ball on the floor
            // is a sprite: the agent walks to it, the game opens a text box, and the only difference
            // between the pickup working and the bag being full is a sentence. It is `Blocked` — the
            // game said no — and a *repeat* of it is the finding, exactly as for a guard.
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
                    // The game asked a different question. The action is untouched and will be
                    // re-issued; leaving the verdict alone is what makes it retryable.
                    OverworldActionAbortedReason::Battle
                    | OverworldActionAbortedReason::NamingScreen => {}
                    // ⭐ **The pace ran its whole budget and nothing turned up, which is the action
                    // done rather than the action failed.** It is an abort only because the walk is
                    // over and the policy has to be asked again. Scoring it `Completed` is what
                    // makes "walk in the grass" gradeable at all: before `NothingAppeared` existed
                    // the agent said nothing here, and 52 `Grass` ids on C3's first walk came back
                    // `Silent`.
                    OverworldActionAbortedReason::NothingAppeared => {
                        self.entry(&id).verdict = Verdict::Completed;
                    }
                    // ⭐ **A Strength floor that has been wedged is the *world* saying no, not the
                    // agent failing.** The row was legal when it was offered and the layout has
                    // since moved into one the floor cannot be solved from — usually by a shove of
                    // this run's own. That is the same shape as a guard turning the player back, so
                    // it is `Blocked` and not a defect, and it keeps its teeth: `REPEAT_IS_A_DEFECT`
                    // still fires if the walk keeps choosing a row that keeps being impossible.
                    // ⚠️ The line the agent gives here already names the cure (leave the floor and
                    // come back), so this is a verdict the reader can act on rather than a silence.
                    OverworldActionAbortedReason::PuzzleUnsolvable => {
                        let message = Some(reason.to_string());
                        self.block(&id, message);
                    }
                    // The game spoke. Expected once; the message usually arrives just after this.
                    OverworldActionAbortedReason::Textbox | OverworldActionAbortedReason::Script => {
                        let message = self.last_blocked.clone();
                        self.block(&id, message);
                        // Re-opened so the quote that follows lands on this id — see `last_blocked`.
                        self.open = Some(id);
                    }
                    // The menu offered a row the agent could not then execute. Nothing about the
                    // world explains these; they are the agent's own failure to do what it offered.
                    other => {
                        self.entry(&id).verdict =
                            Verdict::Defect { reason: other.to_string(), at: *at };
                        self.new_defects.push(id);
                    }
                }
            }
            AgentEvent::TextBox { message } => {
                self.last_blocked = Some(message.clone());
                // Attach it to whatever is blocked and quoteless, which is the id that was just
                // stopped. Nothing else is waiting for a quote.
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

    /// Mark an id blocked, keeping the count across repeats and never overwriting a quote with a
    /// missing one.
    fn block(&mut self, id: &str, message: Option<String>) {
        let entry = self.entry(id);
        let (times, kept) = match &entry.verdict {
            Verdict::Blocked { times, message } => (*times + 1, message.clone()),
            _ => (1, None),
        };
        entry.verdict = Verdict::Blocked { times, message: message.or(kept) };
    }

    /// Ids that became a defect since this was last called, and clear the list. Called by the
    /// fixture after every drain so a failing square can be saved while the emulator is still on it.
    pub fn take_new_defects(&mut self) -> Vec<String> {
        std::mem::take(&mut self.new_defects)
    }

    /// Note an id the menu offered but nothing chose. A frontier walk calls this for the rows it
    /// leaves behind, so the table says what was *not* done as well as what was.
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

    /// How many distinct maps the run reached at all. The one figure that says whether a driver is
    /// exploring or diffusing.
    pub fn maps_touched(&self) -> usize {
        self.entries
            .keys()
            .filter_map(|id| id.split(':').next())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }

    /// Every id the agent could not do what it offered on, as one line each, plus every watchdog
    /// firing. ⚠️ **Not the whole of what makes a run red** — see [`Self::failures`].
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
        // ⚠️ Always a defect, and it belongs to no id — the agent reached no decision point at all,
        // so nothing was in flight to blame.
        out.extend(self.watchdog.iter().map(|where_| format!("the watchdog fired: {where_}")));
        out
    }

    /// The whole table, as the file a coverage run writes whether it passed or failed.
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
    ///
    /// ⭐ **The silences are here rather than in `defects` because they are a different fault and
    /// the summary counts them separately.** A defect is the agent unable to do what it offered; a
    /// silence is the agent doing it and never saying so. Both fail the walk — see
    /// [`Verdict::Silent`] — and this is the list the test asserts on.
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

    /// The kinds of action that went silent, and how many of each — which is what names the gap.
    pub fn silent_kinds(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for entry in self.entries.values().filter(|entry| entry.verdict == Verdict::Silent) {
            let kind = entry.id.rsplit(':').next().unwrap_or("?").to_string();
            *out.entry(kind).or_insert(0) += 1;
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

    /// Drop the table under `target/test-artifacts/coverage/`, where the rest of the suite puts its
    /// failure artifacts. Returns the path, or the reason it could not be written — which is never
    /// worth failing a test over.
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

    /// ⭐ **Every regional start stands where its row says, on a game the cartridge finished.**
    ///
    /// A [`Start`] contributes exactly one thing — the square the walk begins on — so a fixture
    /// regenerated onto a different map turns a regional sweep into a duplicate of another one, and
    /// nothing else in the suite would notice: the walk would run, settle, and report a perfectly
    /// healthy number for somewhere it had already been. This is the check, and it is in the
    /// **default** tier because the sweep it protects costs minutes and is behind a feature.
    ///
    /// ⚠️ **`hall_of_fame_teams` is the part that is not cosmetic.** §5.2.5: badges are a byte a
    /// cheat can write and the gates that actually shut Kanto read *event flags*, which only the
    /// cartridge's own scripts set. A start whose game was never finished walls its walk in at
    /// Pewter exactly as the Pallet Town sweeps were, and would look like a bad region rather than
    /// a bad fixture.
    #[test]
    fn every_coverage_start_stands_where_it_says_on_a_finished_game() {
        use crate::pokemon::integration_tests::fixture::TestFixture;

        let mut names: Vec<&str> = COVERAGE_STARTS.iter().map(|start| start.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), COVERAGE_STARTS.len(), "two starts share a name: {names:?}");
        assert_eq!(
            COVERAGE_STARTS.first().map(|start| start.name),
            Some("phase0"),
            "`phase0` is the default and every figure in the plan was taken from it",
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
                    "the {} start is not a finished game, so the event gates §5.2.5 is about are \
                     shut. If that is deliberate, `before_the_credits` is where the reason goes",
                    start.name,
                ),
                // ⚠️ **And the exception has to *be* one.** A `before_the_credits` start that turns
                // out to have finished the game after all is a fixture that was regenerated further
                // along the chain, which would silently make it a duplicate of an ordinary start.
                Some(why) => assert!(
                    state.hall_of_fame_teams == 0,
                    "the {} start says it is before the credits ({why}), but this save has \
                     finished the game",
                    start.name,
                ),
            }
            // ⚠️ **Keyed on the game as well as the map.** Two starts on `VermilionCity` are a
            // wasted sweep when they are the same game and are the whole point when one of them is
            // standing beside a ship the other cannot see.
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

    /// ⭐ **§5.3's cross-check, pinned without a sweep.** The report itself only ever runs at the
    /// end of a walk that costs minutes, so the thing that would rot — the arithmetic that turns a
    /// ROM warp entry into the id `actions()` would mint for it — is exercised here instead. Pallet
    /// Town is the case to hold it to: three warps in the header, one connection strip on its north
    /// edge, and every coordinate checkable by hand against `pokered/data/maps/objects/PalletTown.asm`.
    #[test]
    fn the_rom_cross_check_finds_a_door_that_was_never_offered() {
        let gb = crate::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        let mmu = gb.core().mmu();

        // Oak's lab was offered and nothing else was. ⚠️ The id has to be the one `actions()` would
        // mint, which is **not** the coordinate in the ROM's own table: Pallet Town has a north
        // connection, so every square is shifted one row down by the strip
        // (`MapMetadata::dimensions`). Oak's lab sits at (12, 11) in the header and (12, 12) on the
        // grid — see `map_metadata`'s own Pallet Town warp assertions for the other two. Getting
        // this shift wrong is the one way this report can lie, which is why the test states it.
        let offered: std::collections::BTreeSet<String> =
            ["PalletTown:12,12:Warp".to_string()].into_iter().collect();
        let report = rom_cross_check(mmu, &offered);

        assert!(report.contains("1 maps the walk entered"), "{report}");
        // Red's house and the rival's house are both real doors that this run never saw.
        assert!(report.contains("(5, 6) → RedsHouse1F"), "{report}");
        assert!(report.contains("(13, 6) → BluesHouse"), "{report}");
        // And the one that *was* offered is not in the list.
        assert!(!report.contains("→ OaksLab"), "the offered door must not be reported: {report}");

        // ⚠️ **A map the walk never entered is not in the report at all** — that is W2's ceiling
        // rather than a missing row, and mixing the two would bury the finding under 200 maps.
        assert!(!report.contains("ViridianCity"), "{report}");

        // Nothing was offered anywhere: the scope is empty and the report says so rather than
        // listing the whole game.
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

        // Completed.
        log.observe(&started(TREE));
        log.observe(&AgentEvent::OverworldActionCompleted { destination: MetaTile::Grass });
        assert_eq!(log.get(TREE).unwrap().verdict, Verdict::Completed);

        // A battle is expected and leaves the id retryable rather than judged.
        let id = "Route1:3,3:Grass";
        log.observe(&started(id));
        log.observe(&aborted(OverworldActionAbortedReason::Battle));
        assert_eq!(log.get(id).unwrap().verdict, Verdict::Unreached, "a battle is not a verdict");
        log.observe(&started(id));
        log.observe(&AgentEvent::OverworldActionCompleted { destination: MetaTile::Grass });
        assert_eq!(log.get(id).unwrap().verdict, Verdict::Completed);
        assert_eq!(log.get(id).unwrap().attempts, 2, "both attempts are counted");

        // A guard is `Blocked`, and what he said is quoted — which arrives *after* the abort.
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

        // Every reason that says the agent could not do what the menu offered is a defect outright.
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

    /// ⚠️ **The repeat is the signal.** One guard is the game; a hundred and forty-three is a run
    /// that has stopped learning, which is what the deployed run of 2026-09-02 did on the way into
    /// Route 2.
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

    /// ⭐ **A row taken and never reported makes the tier red**, as of `docs/coverage-plan.md`
    /// step 1. It is not a `defect` — nothing about the world went wrong, the agent just never said
    /// what became of the decision — so the two counts stay apart and only [`CoverageLog::failures`]
    /// carries both.
    #[test]
    fn a_row_the_agent_never_said_what_became_of_fails_the_walk() {
        let mut log = CoverageLog::new();
        log.observe(&started("Route18:39,13:Grass"));
        // Nothing closes it: the next decision opens on top of it, which is the whole tell.
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

    /// The report is a table with a row per id, and the summary counts what is in it.
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

// ── C3 §5.1: the frontier ────────────────────────────────────────────────────────────────────────

/// A brain that takes every row it is offered, once, and then goes looking for more.
///
/// `docs/coverage-plan.md` §5.1:
///
/// 1. every row in the menu has an id;
/// 2. choose the first unvisited id on this map;
/// 3. when the map has no unvisited rows, go to a map that has one;
/// 4. stop when a full pass over the reachable world discovers no id not already seen.
///
/// ⚠️ **The frontier is over ids, and ids are discovered by standing there.** A map is not finished
/// the first time its rows are exhausted: a row can appear later because a flag changed, a Pokémon
/// learnt a field move, or an item entered the bag. Hence the fixpoint in (4) — a count of turns
/// since the last new id — rather than a per-map done flag.
///
/// ⚠️ **It reads the rendered menu and nothing else**, like every other brain here. That is what
/// makes a row it never sees a finding about `llm::prompt` rather than about this file.
///
/// ⚠️ **Exploration is destructive and mostly one-shot.** A sprite talked to is often gone, an item
/// picked up is gone, a trainer beaten does not rebattle. An id is therefore visited once and its
/// verdict is final; there is no re-running a single id without re-running the walk.
pub struct ExploringBrain {
    /// Every id ever offered, and **how many times** it has been chosen. `0` is offered-never-taken.
    ///
    /// ⚠️ **A count rather than a flag, and the count is load-bearing.** With a flag, an exit that
    /// is *blocked* — a guard, a locked door — stays the best-scoring row on its map for ever,
    /// because being turned back never changes anything the brain can see. The first walk from
    /// Pallet Town took `PewterCity:40,18:Connection` **59 times**: Brock's gym guide stops you at
    /// the east exit until you have the Boulder Badge, and the walk had no way to prefer anything
    /// else. That is the same shape as the deployed run's 143 aborts on the way into Route 2 — and
    /// the oracle caught it, which is the oracle working. Ordering exits by how often they have
    /// already been taken makes a blocked one lose to every alternative after a try or two.
    seen: std::collections::BTreeMap<String, usize>,
    /// How many turns this brain has spent on each map, so leaving prefers somewhere new.
    maps: std::collections::BTreeMap<String, usize>,
    /// ⭐ **Times a door from one map to another has been taken**, keyed `"{here}->{there}"` — the
    /// same tally as [`Self::seen`] one level up, keyed by where a door goes rather than by which
    /// door it is.
    ///
    /// Route 16 has eight squares facing its gate's doors, so `Route16` and `Route16Gate1F` carry
    /// eight warp ids between them and every one works. Counted by id, each crossing makes the door
    /// just used score one more than its siblings, so the walk goes through all four and comes back
    /// through all four; counted by crossing they are two options — into the gate and out of it —
    /// and one trip settles the group. The id count stays as the *last* key, so every door is still
    /// taken eventually and coverage is unharmed: one option taken eight times, not eight options.
    ///
    /// ⚠️ **This is right on its own terms and it is *not* what freed Route 16, which is worth
    /// knowing before trusting it too far.** `docs/coverage-plan.md` step 4 predicted it would, and
    /// measured, the counters alternated exactly as intended while the walk ping-ponged just as
    /// hard: `lavender` finished with two ids chosen **1 593 times each** on a menu of three rows.
    /// What had sealed that map was a **cut tree that regrows** and a frontier that would not cut it
    /// twice — see `re_takeable` in `respond`. Both changes are kept; only one of them mattered.
    ///
    /// ⚰️ **Keyed by the destination alone first, and that lost 22 maps.** A global tally makes a
    /// *hub* repellent: Celadon City is entered from Route 16, Route 7, the mart, the gym and the
    /// Game Corner, so by the time a walk is inside Celadon Mart every door leading back out to the
    /// city has been "taken" a dozen times while the doors between the mart's own floors have not.
    /// The walk was pushed away from the only way out and cycled the building: measured
    /// 2026-09-09, five of the eight regions spent **4 400 to 5 000 turns** in
    /// `CeladonMart{2..5}F`/`Elevator`/`Roof`, the union fell from **159 maps to 137**, and Route
    /// 16's gate — which the change did fix — was simply replaced by a bigger trap. An option is a
    /// decision available *here*, so the key has to say where "here" is.
    exits: std::collections::BTreeMap<String, usize>,
    /// Turns since an id was seen for the first time. The fixpoint of (4).
    pub barren: usize,
    pub turns: usize,
    /// Set once the walk has played the game to its end, which is a terminus rather than a fault.
    /// See the note where it is set.
    pub reached_the_end: bool,
    /// The map the last turn was asked on, for the progress heartbeat. Nothing reads it but the
    /// `[walk]` line, and that line is the only way to tell a slow sweep from a wedged one.
    pub here: String,
    /// ⭐ **PC operations the walk has tried, by map.** They are not in the action menu and cannot
    /// be: `llm::tools` withholds `MetaTile::Pc` on purpose, because every PC operation is a
    /// `use_field_move` that walks to the PC itself, and offering a row that only walks there would
    /// leave the agent holding an open storage menu with nothing chosen. "One way in, not two."
    ///
    /// So a walk that only ever calls `choose_action` can never touch the boxes or PC item storage —
    /// which is exactly what C3's walks did, and why the table had no PC coverage at all. This
    /// drives the other way in.
    pc_ops_tried: std::collections::BTreeSet<(String, &'static str)>,
    /// ⭐ **Consecutive turns the menu has offered nothing at all**, and the flies spent escaping it.
    ///
    /// A 48-game-hour sweep spent **279 009 of its 279 422 turns on `VictoryRoad1F`** and issued 468
    /// overworld actions in the whole run: the walk went into the boulder puzzle, ended up somewhere
    /// `actions()` offers no row from, and answered "nothing to do" for ever. Nothing else can see
    /// that — the agent is not stuck (it reaches a decision point every turn and asks), so the
    /// watchdog never fires, and a menu with no rows in it is not a defect against any id.
    /// Turns on which the menu carried **no rows at all**.
    pub rowless_turns: usize,
    /// ⭐ Consecutive turns on which the menu had rows and the brain still chose nothing — every row
    /// already visited and **no exit among them**. This, not an empty menu, is what the arithmetic
    /// of the 48-hour sweep points at: 279 422 turns at `wait(20)` (0.4 s of game time each) is
    /// almost exactly the 172 800 s the run lasted.
    pub stalled_turns: usize,
    pub stalled_worst: usize,
    /// Where the menu went empty, one entry per episode, with the `Location:` line it was standing
    /// on. ⚠️ **The square is the whole claim**: "boxed in" is otherwise an inference from the brain
    /// having done nothing, and I have twice been wrong reasoning that way instead of measuring.
    pub boxed_in_at: Vec<String>,
}

/// The PC operations the walk exercises, in the order it tries them, as
/// (`move`, `op`, extra arguments).
///
/// ⚠️ **Deposit before withdraw, and an item the walk is certain to be holding.** A withdrawal of
/// something never deposited is refused by `PcBoxOp::blocked_by` before a button is pressed, which
/// is the tool working and tells the walk nothing. The god party is six strong so a deposit is
/// always legal; `blocked_by` refuses depositing the *last* Pokémon, never the sixth.
const PC_OPS: [(&str, &str, &str); 4] = [
    ("pc_items", "deposit", r#""item":"PokeBall","quantity":1"#),
    ("pc_items", "withdraw", r#""item":"PokeBall","quantity":1"#),
    ("pc_pokemon", "deposit", r#""slot":5"#),
    ("pc_pokemon", "change_box", r#""box":2"#),
];

/// Consecutive rowless turns before the walk gives up on where it is standing and flies out.
/// Generous: a map change settles over a few turns and a menu is briefly empty while it does.
const BOXED_IN_PATIENCE: usize = 20;

/// A row that leaves the map. Matched on the id's kind, which is the one part of a row that is a key
/// rather than prose.
fn is_a_way_out(id: &str) -> bool {
    matches!(id.rsplit(':').next(), Some("Warp" | "Connection" | "ConnectionWater"))
}

impl Default for ExploringBrain {
    fn default() -> Self {
        Self::new()
    }
}

impl ExploringBrain {
    pub fn new() -> Self {
        Self {
            seen: std::collections::BTreeMap::new(),
            maps: std::collections::BTreeMap::new(),
            exits: std::collections::BTreeMap::new(),
            barren: 0,
            reached_the_end: false,
            here: String::new(),
            turns: 0,
            pc_ops_tried: std::collections::BTreeSet::new(),
            rowless_turns: 0,
            stalled_turns: 0,
            stalled_worst: 0,
            boxed_in_at: Vec::new(),
        }
    }

    /// How many distinct PC operations the walk has taken, across all maps.
    pub fn pc_ops(&self) -> usize {
        self.pc_ops_tried.len()
    }

    /// Every id this brain was ever offered, so the run can tell the log about the ones it never
    /// chose — which is what makes `Unreached` a verdict rather than an absence.
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

    /// Turns spent on each map, so a settled walk can say what it was cycling over.
    pub fn map_turns(&self) -> &std::collections::BTreeMap<String, usize> {
        &self.maps
    }

    /// Whether the fixpoint has been reached: `barren` turns in a row with nothing new.
    pub fn settled(&self, patience: usize) -> bool {
        self.barren >= patience
    }

    /// How promising a way out is, low is better. §5.1's step 3 — "go to the nearest map that has an
    /// unvisited row" — as far as it can be answered **from strings alone**.
    ///
    /// ⚠️ **The destination is read out of the row's prose, and a name the walk has never seen is
    /// the *best* answer.** A row says "go to Route2" or "take the warp to ViridianMart"; the walk
    /// knows which maps it has ids for, because an id is `{map}:{x},{y}:{kind}`. So:
    ///
    /// - **0** — no word in the description names a map this walk has any id for. Somewhere new.
    /// - **1** — it names a map with rows still unvisited.
    /// - **2** — it names a map whose rows are all done.
    ///
    /// ⚠️ **It knows no map table and must not.** A hard-coded list of the 248 map names would be
    /// the brain reaching past the rendered situation for something a model does not have, and the
    /// property that makes every finding here worth reading is that it cannot.
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

    /// Every word in a row's prose that names a map this walk has an id for — the one thing the
    /// brain is allowed to know about where a door goes, and the same scan
    /// [`Self::promise_of`] grades.
    ///
    /// ⚠️ **Strings, and no map table.** A hard-coded list of the 248 names would be the brain
    /// reaching past the rendered situation for something a model does not have.
    fn maps_named<'a>(&self, description: &'a str) -> Vec<&'a str> {
        description
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|word| self.seen.keys().any(|id| id.starts_with(&format!("{word}:"))))
            .collect()
    }

    /// The key into [`Self::exits`] for a way out of the map the walk is standing on: where it
    /// goes, as far as its prose says, against where it goes *from*. `None` for a door into
    /// somewhere this walk has never had an id — which is [`Self::promise_of`]'s `0`, the case the
    /// ordering already prioritises on its own terms.
    fn crossing(&self, description: &str) -> Option<String> {
        let there = self.maps_named(description).first().map(|m| m.to_string())?;
        Some(format!("{}->{there}", self.here))
    }
}

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
            // A nickname, a mart, a move to forget: answer with the game's own default and carry on.
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

        // ⭐ **The PC, which no menu row leads to.** Tried once per operation per map that has one,
        // before the ordinary frontier choice, so a Pokémon Centre is not left the moment its rows
        // are exhausted. `use_field_move` walks to the PC itself, so this needs no routing and no
        // row — but it does need the brain to know a PC is there, and the only string that says so
        // is the map's name. Every Centre is `…Pokecenter`; `RedsHouse2F` is the player's own.
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

        // ⭐ **Reaching the Hall of Fame ends the walk, because it ends the *game*.**
        //
        // With a god party the walk beat the Elite Four a second time, and the Champion's room
        // does not offer a door to choose: the cartridge force-walks the player in. pokered then
        // increments `wNumHoFTeams`, plays the parade, saves, and **soft-resets to the title
        // screen** — and `PokemonAgent` has no `GameMode` for a title screen, so it goes on reading
        // stale map RAM (`HallOfFame` at (4, 2)) and offering warps for a player who is no longer
        // in the world. `host.rs` catches that byte in the product and starts a new run; a bare
        // agent has nothing.
        //
        // ⚠️ **Filtering the rows out was tried first and was strictly worse.** It could not stop
        // the walk arriving (nothing was chosen to get there), so all it did was leave the brain
        // with nothing to pick: the sweep reported **zero defects** and spent 20 059 of its 20 538
        // turns at the title screen. A terminus has to be *reported*, not made unreachable — the
        // agent gap was a real finding and is recorded in `docs/coverage-plan.md` §7.1.
        if request.location().as_deref() == Some("HallOfFame") {
            self.reached_the_end = true;
        }
        let rows = request.menu_rows();
        // Counted before anything is chosen, so "the menu was empty" is a fact rather than an
        // inference from the brain having done nothing.
        if rows.is_empty() {
            self.rowless_turns += 1;
        }
        // ⚠️ **Insert only if absent.** A row is re-offered on every turn the player stands near it,
        // and overwriting would reset an id already chosen back to unchosen — an infinite loop on
        // the first row of every map, which is exactly the shape the agent exists to avoid.
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

        // (2) the first unvisited row that does not leave the map, so a map is exhausted before it
        // is left; then (3) the way out that most likely leads somewhere with work left.
        let unvisited = |id: &String| self.seen.get(id).copied() == Some(0);

        // ⭐ **A row the world puts back, and the walk has to be willing to take again.**
        //
        // Route 16's east half is cut in two by a tree, and the way out to Celadon City is on the
        // far side of it. The walk cut it, crossed, marked `Route16:34,10:CutTree` done — and cut
        // trees **regrow when a map reloads** (`PokemonAgent`'s `cut_tiles.clear()` says so). Every
        // time it came back the tree was standing and the row was "visited", so the only rows left
        // were the gate and the Fly House. Measured on 2026-09-09: `lavender` spent **6 266 of its
        // 6 427 turns** in that three-map pocket and `Route16:40,10:Connection` — the way out, one
        // tree away — finished the run **offered and never once chosen**. Five of the eight regions
        // did the same, 80% to 97.5% of every turn the sweep took.
        //
        // ⚠️ **This is the Pokémon Mansion statue again**, and the fallback written for that only
        // fires when the menu has *no exit at all* (see below). Route 16 has three, so it never
        // fired; the walk had somewhere to go and went there fifteen hundred times.
        //
        // ⚠️ **Not `Grass` or `Empty`.** Those are a request for an *encounter*, and a second pace
        // discovers nothing while costing a 60 s budget — the same distinction `resume` makes below
        // and for the same reason: the grind is the right answer for a model playing the game and
        // the wrong one for a walk whose whole job is breadth.
        let re_takeable = |id: &String| !matches!(id.rsplit(':').next(), Some("Grass" | "Empty"));
        let chosen = rows
            .iter()
            .map(|(id, _)| id)
            .find(|id| unvisited(id) && !is_a_way_out(id))
            .cloned()
            .or_else(|| {
                rows.iter()
                    // Exits are never `Grass` or `Empty`, so the one test covers both halves:
                    // every way out, plus every other row the world might have put back.
                    .filter(|(id, _)| re_takeable(id))
                    // ⚠️ **How often it has already been taken comes *first*, and the promise of
                    // where it goes second.** The other order looks obviously right and loops for
                    // ever: an exit into a map the walk has never reached scores best on promise,
                    // and being turned back at it does not change anything the brain can see — so
                    // Brock's gym guide, who blocks Pewter's east exit until the Boulder Badge, held
                    // the walk on `PewterCity:40,18:Connection` for **59 attempts**. With the count
                    // first, every exit is taken once before any is taken twice, and promise decides
                    // the order within a pass, which is what it is for.
                    // ⚠️ **The count is uncapped, and capping it is measured to be worse.** The
                    // obvious refinement is `min(times, 1)` so that `promise` decides once every
                    // exit has been taken once — it sounds strictly better and it took the walk
                    // from **41 maps to 21**. Promise-leading bounces between two adjacent maps
                    // that both still have unvisited rows (both score 1) and never pushes outward;
                    // the plain round-robin diffuses, which is slower per map and reaches far more
                    // of them. Tried and reverted 2026-09-07.
                    // ⭐ **A door into a map nobody has been through beats one that only leads
                    // somewhere known — but only for its first two tries.**
                    //
                    // Count-first diffuses, which is why it wins in general (see above), but it
                    // has no way out of a *cluster*: the walk of 2026-09-08 spent its whole budget
                    // inside Victory Road because every exit from every floor leads to another
                    // floor of Victory Road, so `promise` was equal everywhere and the round-robin
                    // simply cycled the ladders. It settled at **30 of 248 maps**, the whole of it
                    // north-west Kanto, having never pushed through Mt Moon to Cerulean.
                    //
                    // ⚠️ **Two tries and then it rejoins the pool**, which is the half that keeps
                    // this from being the promise-first ordering that was tried and reverted. A
                    // blocked door leads somewhere unseen for ever and nothing about being turned
                    // back changes what the brain can see, so unconditional priority is how
                    // Brock's gym guide held the walk on `PewterCity:40,18:Connection` for 59
                    // attempts. Priority that expires cannot do that, and it still cannot help an
                    // exit whose promise is merely "there is work left there" (score 1), which is
                    // the case the `min(times, 1)` experiment lost 20 maps on.
                    // ⭐ **Taken-ness is counted per *crossing* — this map to that one — rather
                    // than per id.** See [`Self::exits`]. The id count stays as the last key: it
                    // decides *which* of a group of doors to take when the group's turn comes
                    // round, so all eight of Route 16's are still taken and the frontier loses
                    // nothing.
                    //
                    // ⚠️ **A door into a map with no name the walk knows falls back to its id
                    // count**, which is what it had before. That is the `promise == 0` case, and it
                    // is already the one the two-try priority above is for; once the door has been
                    // through once the walk has ids on the other side and the name resolves.
                    .min_by_key(|(id, description)| {
                        let id_times = self.seen.get(id).copied().unwrap_or(0);
                        let exit = is_a_way_out(id);
                        let map_times = match exit {
                            true => self.crossing(description)
                                .and_then(|crossing| self.exits.get(&crossing).copied())
                                .unwrap_or(id_times),
                            false => id_times,
                        };
                        // ⚠️ **A row that is not a way out scores *worse* than any exit that ties
                        // with it, and that is the line that keeps this from being the
                        // promise-first ordering that lost 20 maps.** `promise_of` reads map names
                        // out of the prose, so a `CutTree` or a person names nothing and would come
                        // back `0` — the best score there is. Ranking it below `2` means a
                        // re-takeable row is chosen only when it has been taken **strictly fewer**
                        // times than every way out, which on a map passed through once is never.
                        let promise = match exit { true => self.promise_of(description), false => 3 };
                        let new_map_worth_a_try = exit && promise == 0 && id_times < 2;
                        (!new_map_worth_a_try, map_times, promise, id_times)
                    })
                    .map(|(id, _)| id.clone())
            });

        // ⭐ **Nothing at all above: take the least-taken row again rather than wait.** With the
        // widened ordering this is now only reached when every row is a `Grass` or an `Empty` that
        // has already been paced, which is the one case the arm above declines to answer.**
        //
        // ⚠️ **A once-only frontier cannot solve a puzzle whose pieces toggle**, and that is not a
        // hypothetical: a 15-game-hour walk spent **80 636 consecutive turns** on
        // `PokemonMansionB1F` at (27, 10) with four rows on the menu, every one of them already
        // visited and **no exit among them** — one of the four being `Statue1`. The Mansion statues
        // toggle a shared barrier, so the walk had pressed one, sealed its own way out, and then
        // refused to press it again because it had "done" that row. The same shape burned 279 009
        // turns on `VictoryRoad1F`, which is why that floor looked like the problem and was not.
        //
        // A model would simply choose it again, so the walk does too. Coverage is unharmed: the
        // frontier is over ids *offered*, and an id chosen twice is still one id.
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
                // The fallback below can pick an exit too, and a tally that missed those would
                // undercount exactly the rows this is for.
                if is_a_way_out(&id)
                    && let Some(description) = rows.iter().find(|(i, _)| *i == id).map(|(_, d)| d)
                    && let Some(crossing) = self.crossing(description)
                {
                    *self.exits.entry(crossing).or_insert(0) += 1;
                }
                // ⚠️ `resume_after_battle` everywhere **except** the two rows that exist to *start*
                // a battle, and that exception is measured. A wild encounter says nothing about a
                // walk across a route, so resuming one is a whole turn saved. But a `Grass` or an
                // `Empty` row is a request for an encounter, so resuming it means fighting the next
                // one too: `MAX_BATTLE_RESUMES` of them per id, which is a grind. That grind is the
                // right answer for a model playing the game and the wrong one for a walk whose
                // whole job is breadth. It cost this walk 12 maps and 94 ids the first time the
                // agent started reporting a pace that ended in a battle at all: 28 maps and 352 ids
                // became 16 and 258 in the same 90 game-minutes, because until then the abort was
                // never emitted and every resume was silently dropped as `Dropped::Unreported`.
                let resume = !matches!(id.rsplit(':').next(), Some("Grass" | "Empty"));
                Reply::call(
                    "choose_action",
                    serde_json::json!({ "id": id, "resume_after_battle": resume }),
                )
            }
            // ⭐ **Boxed in: no row at all.** Waiting is all this branch can do, and waiting is what
            // burned a 48-hour sweep: 279 009 turns on one map, because a walk with no row to choose
            // has no way to leave the square it is standing on either.
            //
            // ⚠️ **Fly is not the escape, and assuming it was is an error worth leaving written
            // down.** Gen 1 refuses Fly anywhere but outdoors, and every map this has happened on is
            // a cave — so the one move that looks like a way out of a dungeon is the one the
            // cartridge will not allow there. `Map::is_overworld` is the same fact.
            //
            // So the turn ends the only way it can, and what the walk does instead is *record* it:
            // `boxed_in` is counted above and the run dumps a save state the first time it passes
            // `BOXED_IN_PATIENCE`, because "the menu was empty" is a claim that needs the square it
            // was empty on.
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

/// ⭐ **W2's direction 1 — where a walk starts is a knob, because no single walk can reach Kanto.**
///
/// Every sweep from `postgame-phase0.bin` settles in north-west Kanto plus Victory Road: Cerulean,
/// Vermilion, Lavender, Celadon, Fuchsia, Saffron and Cinnabar are never entered at all. Two
/// different things do that, and `docs/coverage-plan.md` §7.2 records conflating them.
///
/// ⭐ **The walk stops because it *wins*.** Seven of eight identical walks halt at exactly 38 maps
/// on [`ExploringBrain::reached_the_end`]: the god party's way out of Viridian is Route 22 → Route
/// 23 → Victory Road → the Indigo Plateau, so it meets the Elite Four before it meets Cerulean, and
/// beating them ends the cartridge. No budget moves that, because there is no world left to walk.
///
/// ⚠️ **And separately, east is shut.** Mt Moon B1F's regions are entry-dependent, so three of its
/// eight warps are never in the menu to be chosen, and the brain's frontier cannot steer toward a
/// map it has no id for — a walk that misses the Plateau still does not get past Route 4.
///
/// So the walk is given somewhere else to start. Each of these is a **finished game** — the credits
/// have rolled, so every gate is open because the cartridge opened it, which is the whole reason
/// §5.2.5 abandoned Pallet Town and §1.2 rules out writing the event flags by hand. The one
/// exception is argued on [`Self::before_the_credits`], and it has to be argued there: nothing else
/// in this file admits one.
///
/// ⚠️ **The party and the bag do not come from the fixture and must not be read into this table.**
/// `Cheats::default()` installs the god party (Cut/Surf/Strength/Flash and Fly) and
/// `with_key_items` stocks the bag, so what a start contributes is one thing only: *where the
/// player is standing*. That is what makes the table cheap — any postgame fixture would do, and
/// these are chosen for their map and nothing else.
///
/// ⚠️ **`phase0` is first and is the default, and its name is the fixture's rather than its
/// region's.** Every measurement in the plan is "the phase0 walk", and renaming it here would make
/// a baseline taken before this change impossible to line up against one taken after it.
pub struct Start {
    /// What `GB_COVERAGE_START` names it by.
    pub name: &'static str,
    /// The committed fixture. ⚠️ Never a state the walk writes: exploration is destructive, so a
    /// start is a file in the tree that a sweep can be re-run against, not a checkpoint.
    pub state: &'static [u8],
    /// Where it stands, asserted by [`every_coverage_start_stands_where_it_says_on_a_finished_game`]
    /// in the **default** tier. The map is the entire content of a start, so a fixture regenerated
    /// onto a different square is a regional sweep quietly becoming a duplicate of another one.
    pub map: crate::pokemon::map::Map,
    /// ⚠️ **Set only for a start taken *before* the credits, with the reason it has to be.**
    ///
    /// Every other start is a finished game on purpose (see this type's own note), and the rule is
    /// load-bearing: a save whose scripts have not run walls its walk in behind gates that read
    /// event flags, and every stall found in one is a false positive. This field is the one
    /// admitted exception and it exists because of a place no finished game can ever stand.
    ///
    /// ⭐ **The S.S. Anne sails, and it takes eleven maps with it.** `EVENT_SS_ANNE_LEFT` is set the
    /// moment the captain hands over HM01 — before the third badge — and
    /// `VermilionCityLeftSSAnneCallbackScript` then shuts the dock for the rest of the game. So
    /// `SSAnne1F`, `1FRooms`, `2F`, `2FRooms`, `3F`, `B1F`, `B1FRooms`, `Bow`, `CaptainsRoom`,
    /// `Kitchen` and `VermilionDock` are unreachable from *every* finished save, and
    /// `docs/coverage-plan.md` §2.1 was wrong to file them under "the bag was full and refused the
    /// S.S. Ticket": the ticket now fits in every start's bag and the cluster did not move.
    pub before_the_credits: Option<&'static str>,
}

/// The regional starts, one per region the `phase0` walk never reaches, plus `phase0` itself.
///
/// ⚠️ **Cerulean has no city fixture and Route 5 is the stand-in.** Nothing in the postgame chain
/// ends inside Cerulean City; Route 5 runs south out of it to Saffron's north gate, so the walk
/// starts one connection from Cerulean and one gate from Saffron.
pub const COVERAGE_STARTS: &[Start] = &[
    Start {
        name: "phase0",
        state: include_bytes!("../data/postgame-phase0.bin"),
        map: crate::pokemon::map::Map::ViridianPokecenter,
        before_the_credits: None,
    },
    Start {
        name: "cerulean",
        state: include_bytes!("../data/postgame-daycare.bin"),
        map: crate::pokemon::map::Map::Route5,
        before_the_credits: None,
    },
    Start {
        name: "vermilion",
        state: include_bytes!("../data/postgame-farfetchd.bin"),
        map: crate::pokemon::map::Map::VermilionCity,
        before_the_credits: None,
    },
    Start {
        name: "lavender",
        state: include_bytes!("../data/postgame-sweep-lavender.bin"),
        map: crate::pokemon::map::Map::Route10,
        before_the_credits: None,
    },
    Start {
        name: "celadon",
        state: include_bytes!("../data/postgame-game-corner.bin"),
        map: crate::pokemon::map::Map::CeladonCity,
        before_the_credits: None,
    },
    Start {
        name: "saffron",
        state: include_bytes!("../data/postgame-silph-floors.bin"),
        map: crate::pokemon::map::Map::SaffronCity,
        before_the_credits: None,
    },
    Start {
        name: "fuchsia",
        state: include_bytes!("../data/postgame-safari.bin"),
        map: crate::pokemon::map::Map::FuchsiaCity,
        before_the_credits: None,
    },
    Start {
        name: "cinnabar",
        state: include_bytes!("../data/postgame-seel.bin"),
        map: crate::pokemon::map::Map::CinnabarIsland,
        before_the_credits: None,
    },
    // ⭐ **The ninth, and the only one that is not a finished game.** See
    // [`Start::before_the_credits`] for the whole argument: the ship sails before the third badge
    // and never comes back, so eleven maps are unreachable from every save the other eight are cut
    // from. `at-vermilion.bin` is the playthrough's own state one leg before it boards, S.S. Ticket
    // already in the bag.
    Start {
        name: "ssanne",
        state: include_bytes!("../data/at-vermilion.bin"),
        map: crate::pokemon::map::Map::VermilionCity,
        before_the_credits: Some(
            "the S.S. Anne has not sailed yet, and `EVENT_SS_ANNE_LEFT` is what makes its ten \
             rooms and VermilionDock unreachable from every finished game"),
    },
    // ⭐ **The tenth, and the only start whose whole job is to remove a coin flip.** `ssanne` above
    // stands in Vermilion City one warp from the dock, and the frontier's least-taken exit out of
    // Vermilion goes *north*: its first walk of 2026-09-10 never took `VermilionCity:18,32:Warp`
    // at all and spent a whole 6-hour budget without boarding, while its second found the ship and
    // came back with eleven maps no sweep in this plan's history had entered. A start already
    // aboard makes those eleven a certainty and hands the budget to the ship instead of to the way
    // there. `regen_on_the_ss_anne_fixture` cuts it, from the same `at-vermilion.bin` its sibling
    // uses.
    //
    // ⚠️ **Both are kept, and the pair is not a duplicate.** `ssanne` earns its place twice over —
    // §2.1 — because a start that has *not* beaten the Elite Four is never funnelled into Victory
    // Road, so it is also the only walk that reaches Pokémon Mansion, Cinnabar's buildings, Pallet
    // Town's interiors and Viridian Mart. That half is about the south-west and has nothing to do
    // with the ship.
    Start {
        name: "ssanneship",
        state: include_bytes!("../data/on-the-ss-anne.bin"),
        map: crate::pokemon::map::Map::SSAnne1F,
        before_the_credits: Some(
            "it is standing on the S.S. Anne, which sails before the third badge and never comes \
             back, so no finished save can be cut here at all"),
    },
];

/// What one walk came back with, so a run of several can be summed without keeping eight logs alive.
///
/// ⚠️ **`maps` and `ids` are sets rather than counts, because the union is the whole point.** Eight
/// walks that each reach 30 maps have reached somewhere between 30 and 240 of them, and only the
/// sets say which.
#[cfg(feature = "coverage-tests")]
struct WalkOutcome {
    name: &'static str,
    ids: std::collections::BTreeSet<String>,
    maps: std::collections::BTreeSet<String>,
    /// Everything that makes this walk red — [`CoverageLog::failures`], so defects *and* silences.
    failures: Vec<String>,
    turns: usize,
    game_time: std::time::Duration,
    wall: std::time::Duration,
    /// The line the walk printed about why it stopped, kept so the summary of a multi-region sweep
    /// can say which regions settled and which were cut off.
    stopped: String,
}

/// **C3's walk.** Every reachable action from a starting save, taken once, with a verdict on each.
///
/// ⚠️ **Its first deliverable is a number, not a pass**: ids discovered per game-minute, and the
/// shape of the curve (§5.6). The budget follows the measurement, so this prints its own and the
/// caller decides whether to buy more.
///
/// ⚠️ **It fails on any `defect` and on any `silent`**, which is the whole point of the verdict
/// oracle: a row the menu offered and the agent could not then execute, a watchdog firing, or a row
/// it took and never said what became of. A `blocked` is not a failure until it repeats — being
/// stopped is how this game says almost everything.
///
/// ⭐ **The silence half is new (2026-09-09) and it is the last item of `docs/coverage-plan.md`
/// step 1**, deliberately turned on only once all three families the baseline found had been fixed:
/// a `Fish` row that surfed onto the water it was sent to cast into, a `Grass` pace whose action was
/// taken away by a trainer's walk-up, and a `PushBoulder*` row that turned out not to be silent at
/// all any more. Turning it on before those would have made the tier red for something the step had
/// not fixed.
///
/// ⭐ **`GB_COVERAGE_START` picks where it starts** — a name from [`COVERAGE_STARTS`], or `all` for
/// one walk per region and the union of what they reached. See [`Start`] for why one walk is not
/// enough. The default is `phase0`, which is the sweep every number in the plan was taken from.
#[test]
#[cfg(feature = "coverage-tests")]
fn coverage_walk_of_the_finished_game() {
    /// How much game time **each** walk may spend, in game-minutes, from `GB_COVERAGE_MINUTES`.
    ///
    /// ⚠️ **A bound, not a target, and it is `min`'d against the fixture's own cap deliberately.**
    /// §9's last risk is that the fixpoint keeps discovering rows and the walk never terminates —
    /// the answer is to *cap the passes and report a non-empty frontier as a result* rather than to
    /// hang. The fixture's cycle budget is a panic; this is a stop.
    ///
    /// ⚠️ **The default is a *smoke* budget, not a coverage one.** 90 game-minutes reaches 28 maps
    /// of 248 — the Pallet/Viridian/Pewter corner — and stops with the frontier wide open. Reaching
    /// the rest of the world is a matter of game time and nothing else: the emulator runs at ~56x,
    /// so an hour of wall clock buys about 56 game-hours. Set `GB_COVERAGE_MINUTES` for a real
    /// sweep; the committed default stays small so the tier is runnable.
    ///
    /// ⚠️ **It is per walk, not per run.** `GB_COVERAGE_START=all` spends it eight times over.
    let minutes: u64 = std::env::var("GB_COVERAGE_MINUTES").ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(90);

    // Turns with nothing new before the frontier is called settled. Generous: a walk that has just
    // crossed into a new building spends several turns on rows it has already seen.
    //
    // ⚠️ **It scales with the budget.** On a long walk the frontier goes quiet for a while whenever
    // the run is crossing a region it has already swept to reach one it has not, and calling that
    // "settled" stops the walk exactly where it was about to be most useful.
    // ⚠️ **Patience, not the budget, is what stopped the sweeps.** The 24-game-hour walk settled
    // after 960 barren turns having spent 2.6 of its 24 hours: it was re-treading known maps, and
    // the round-robin needs a long time to work its way outward through a world this size.
    // `GB_COVERAGE_PATIENCE` decouples the two so a sweep can be told to run to its budget.
    let patience: usize = std::env::var("GB_COVERAGE_PATIENCE").ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or((60 * minutes.max(90) / 90) as usize);

    // How long **each** walk may take in wall clock, as opposed to game time. It was
    // `60 + minutes * 3` inline, which quietly assumes the agent manages about 20x real time; a
    // sweep that runs slower than that is cut off having covered a fraction of what it was asked
    // for, and before this printed a reason it looked exactly like a sweep that had finished.
    let wall_secs: u64 = std::env::var("GB_COVERAGE_WALL_SECS").ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60 + minutes * 3);

    // ⚠️ **An unknown name is a failure rather than a fallback to the default.** A typo that
    // silently walked `phase0` would report the baseline under another region's name, which is the
    // one way this knob can produce a number that is wrong rather than merely disappointing.
    let wanted = std::env::var("GB_COVERAGE_START").unwrap_or_else(|_| "phase0".to_string());
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

    // ⭐ **The union is W2's number**, and it is only meaningful when more than one walk ran — a
    // single-region run prints its own figures above and this would just repeat them.
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
        // How much each region added that no other did. ⚠️ **Overlap is the thing to read here**:
        // two starts whose walks reach the same places are one start and a wasted five minutes, and
        // the table is what says which of these to keep.
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
            "\n════ C3: the regional sweep ════\n{}\n\
             union      ⭐ {} maps of 248, {} ids, over {} walks\n\
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
    }

    // ⚠️ **Every region walks before any assertion, and that is deliberate.** A defect in the first
    // region would otherwise take the other seven's numbers with it, and those numbers are the
    // deliverable; a sweep is minutes long and re-running it to see the rest is not a trade worth
    // making. The failure below still names the region it came from.
    let failures: Vec<String> = outcomes
        .iter()
        .flat_map(|o| o.failures.iter().map(|d| format!("[{}] {d}", o.name)))
        .collect();
    assert!(failures.is_empty(), "the walk found {} failures:\n  {}",
            failures.len(), failures.join("\n  "));
    let discovered: usize = outcomes.iter().map(|o| o.ids.len()).sum();
    assert!(discovered > 10, "only {discovered} ids were ever offered; the walk did not happen");
}

/// ⭐ **The maps no walk entered, which is the other half of the union and the input to
/// `docs/coverage-plan.md`'s step 6.**
///
/// A sweep that prints "186 maps of 248" says nothing about the 62, and the 62 are the only thing
/// left to act on: a gate doing its job and a walk that never arrived look identical from a count.
/// Before this the list was assembled by hand — `Map::iter()` diffed against a shell union of the
/// tsvs — which is exactly the sort of arithmetic that gets done once and then quoted for a week
/// after it stopped being true.
///
/// ⚠️ **Two thirds of what is "missing" is not missing.** 44 of the 248 are the ROM's `UnusedMap*`
/// padding — map numbers with no header, which `Map::iter()` yields because the enum is the byte —
/// and `Colosseum` and `TradeCenter` are the link-cable rooms, which need a second Game Boy. They
/// are counted and then set aside, so the list that is left is the one worth reading.
fn unreached_report(entered: &std::collections::BTreeSet<&String>) -> String {
    use crate::pokemon::map::Map;
    use strum::IntoEnumIterator;
    let (mut padding, mut cable, mut real) = (0usize, 0usize, Vec::new());
    for map in Map::iter() {
        let name = format!("{map:?}");
        if entered.contains(&name) {
            continue;
        }
        match map {
            _ if name.starts_with("UnusedMap") => padding += 1,
            Map::Colosseum | Map::TradeCenter => cable += 1,
            _ => real.push(name),
        }
    }
    real.sort();
    // Wrapped rather than one per line: this is a list to scan for a cluster — the S.S. Anne's nine
    // rooms, Rocket Hideout's four floors — and a column of sixty names hides one.
    let mut lines: Vec<String> = vec![format!(
        "unreached  ⭐ {} real maps, plus {padding} UnusedMap* and {cable} link-cable rooms",
        real.len())];
    for chunk in real.chunks(6) {
        lines.push(format!("           {}", chunk.join(" ")));
    }
    lines.join("\n")
}

/// One walk, from one [`Start`]. Everything above it is knobs and arithmetic; this is the walk that
/// every number in `docs/coverage-plan.md` §2 came out of.
#[cfg(feature = "coverage-tests")]
fn walk_from(start: &Start, minutes: u64, patience: usize, wall_secs: u64) -> WalkOutcome {
    use crate::pokemon::integration_tests::cheats::Cheats;
    use crate::pokemon::integration_tests::llm_harness::LlmRun;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    let budget = Duration::from_mins(minutes);

    /// A handle on the brain, since the endpoint owns it.
    #[derive(Clone)]
    struct Shared(Arc<Mutex<ExploringBrain>>);

    impl crate::pokemon::integration_tests::llm_harness::Brain for Shared {
        fn respond(
            &mut self,
            request: &crate::pokemon::integration_tests::llm_harness::TurnRequest,
        ) -> crate::pokemon::integration_tests::llm_harness::Reply {
            use crate::pokemon::integration_tests::llm_harness::Brain;
            self.0.lock().expect("not poisoned").respond(request)
        }
    }

    let brain = Shared(Arc::new(Mutex::new(ExploringBrain::new())));
    // ⭐ **A *finished* game, which is what §5.1 asked for all along** — "from C2's finished save".
    // Starting from Pallet Town walls the walk in at Pewter: the east exit is held by the Youngster
    // who drags you to Brock ("BROCK's looking for new challengers! Follow me!") and Brock's own
    // guide refuses ("you're still light years from facing BROCK!"), because both read the *event
    // flag* for having beaten him and not the badge byte `debug_set_badges` writes. So Mt Moon,
    // Cerulean, Vermilion, Celadon, Lavender, Fuchsia, Saffron, Silph and Victory Road were all
    // unreachable, and the walk settled at 33 maps of 248 having spent 2 of its 12 game-hours.
    //
    // ⚠️ **And the fix is not to write that flag.** §1.2: setting `wEventFlags` desynchronises
    // scripts from map objects and every stall found in such a save is a false positive. Every
    // entry in `COVERAGE_STARTS` is a game the cartridge itself played to the credits.
    let mut run = LlmRun::builder(start.state)
        .named("coverage-walk")
        // Twice `BUDGET`, so the walk always stops on its own bound rather than on the fixture's
        // panic. The two are different failures and only one of them is a result.
        .game_time(budget * 2)
        .with_coverage()
        .start(Box::new(brain.clone()));
    // ⭐ **The bag is what makes the *overworld* fully offered.** Without a rod there is no
    // `MetaTile::Fish` row anywhere in the game, and without the Silph Scope, Card Key, Lift Key,
    // Poké Flute, Bicycle, Secret Key and S.S. Ticket whole regions are shut. `actions()` is right
    // to withhold those rows; a walk meant to reach everywhere has to be given the items, exactly as
    // it is given the badges. See `cheats::COVERAGE_KEY_ITEMS`.
    run.with_cheats(Cheats::default().with_key_items(999_999));

    /// Wall-clock seconds between progress lines. ⭐ **A walk with no heartbeat is indistinguishable
    /// from a hung one**, and the 24-hour sweep of 2026-09-07 was left running for two and a half
    /// hours before anyone could tell it was livelocked on one Victory Road boulder at a rate of one
    /// action per minute. The line is cheap and it is the difference between noticing in 30 seconds
    /// and noticing in two hours.
    const BEAT_SECS: u64 = 30;

    let started = std::time::Instant::now();
    let mut spent_the_budget = false;
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
            // ⚠️ **`turns/min` is the number that spots a livelock**, not the id counts: a walk
            // wedged on one action still discovers rows every time the menu is rebuilt, and still
            // burns game time. A healthy walk does hundreds of turns a minute; the wedged one did
            // one.
            let per_min = (turns - beat_turns) as u64 * 60 / BEAT_SECS;
            let warn = if turns - beat_turns <= 2 { "  ⚠️ NOT MOVING" } else { "" };
            println!("[walk:{name}] {wall:>5.0}s wall {game:>6.0}s game ({rate:>4.1}x) | {turns} turns \
                      (+{per_min}/min) | {visited} chosen (+{new_ids}) | {maps} maps | on {here}{warn}",
                wall = wall.as_secs_f64(), game = game.as_secs_f64(),
                rate = game.as_secs_f64() / wall.as_secs_f64().max(0.001),
                new_ids = visited - beat_visited);
            (beat_turns, beat_visited) = (turns, visited);
        }
        let brain = brain.0.lock().expect("not poisoned");
        spent_the_budget || brain.reached_the_end || brain.settled(patience)
    }) && !spent_the_budget;
    let elapsed = started.elapsed();

    let (discovered, visited, maps, turns) = {
        let brain = brain.0.lock().expect("not poisoned");
        (brain.discovered(), brain.visited(), brain.maps(), brain.turns)
    };
    // Everything the menu offered and the walk never chose is `Unreached` rather than absent.
    let offered = brain.0.lock().expect("not poisoned").offered_ids();
    // Where the turns actually went. A walk that has stopped finding anything is usually cycling
    // over a handful of maps, and this is what says which.
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
    // ⚠️ **The one cheat that can silently fail, said out loud.** Gen 1's bag holds twenty *kinds*
    // and a finished save arrives nearly full, so `Cheats` reports rather than panics when a key
    // item will not fit — and its own comment calls that "a coverage gap worth printing", which
    // nothing printed. A walk with no Bicycle cannot reach Cycling Road, and it took a probe to
    // rule that out as the reason five regions could not leave Route 16.
    let refused: Vec<String> = run.cheats.as_ref()
        .map(|c| c.bag_was_full.iter().map(|i| format!("{i:?}")).collect())
        .unwrap_or_default();
    // ⭐ **And what it took out to make them fit.** Step 6's first job was that every one of the
    // eight starts arrived with all twenty of Gen 1's bag kinds used and refused between one and
    // nine key items; `debug_keep_only_items` sheds what a cheated walk cannot use, and this says
    // what went, because a walk that turns out to have needed one of them has to be able to see it.
    let shed: Vec<String> = run.cheats.as_ref()
        .map(|c| c.bag_was_shed.iter().map(|id| match crate::pokemon::item::ItemId::from_repr(*id) {
            Some(named) => format!("{named:?}"),
            None => format!("${id:02x}"),
        }).collect())
        .unwrap_or_default();
    let bag = match refused.is_empty() {
        true => format!("every key item fit the bag; {} shed to make room: {}",
            shed.len(), shed.join(", ")),
        false => format!("⚠️ the bag was full and refused {} — whatever they gate is unreachable: {} \
                          (⚠️ and {} were shed to make room, so this is not the junk: {})",
            refused.len(), refused.join(", "), shed.len(), shed.join(", ")),
    };
    {
        let log = run.fixture().coverage.as_mut().expect("coverage was asked for");
        for id in &offered {
            log.offered(id);
        }
    }
    let log = run.coverage().expect("coverage was asked for");
    // ⚠️ **The report is named after the start**, so a regional sweep does not overwrite itself
    // eight times. The plan's older figures quote `walk-of-the-finished-game.tsv`, which is this
    // file under its previous name and is `walk-phase0.tsv` now.
    let written = log.write_report(&format!("walk-{name}"));

    // ⚠️ **Three ways to stop and they are not interchangeable.** This used to print "stopped
    // on the {budget} budget" for every unsettled walk, including the ones that had run out of
    // *wall clock* having spent a tenth of their game-time budget — which reads as "the sweep
    // finished, the world is just big" when it means "the sweep was cut off and you are
    // looking at a fraction of it". The 24-hour walk of 2026-09-07 reported exactly that after
    // reaching 8 365 s of 86 400, and the two hours spent believing it are the reason this
    // string is now computed rather than assumed.
    let stopped = match (settled, spent_the_budget) {
        _ if brain.0.lock().expect("not poisoned").reached_the_end => format!(
            "⭐ the walk played the game to the **Hall of Fame** and stopped there, which is a \
             terminus rather than a fault: the cartridge saves and soft-resets to the title \
             screen, and there is no world left to walk. {:.0}% of the {budget:?} game-time \
             budget was spent getting there",
            100.0 * game_time.as_secs_f64() / budget.as_secs_f64()),
        (true, _) => format!("{patience} turns with nothing new"),
        (false, true) => format!("stopped on the {budget:?} game-time budget with the frontier still open"),
        (false, false) => format!(
            "⚠️ CUT OFF after {elapsed:?} of WALL CLOCK with only {:.0}% of the {budget:?} \
             game-time budget spent — raise GB_COVERAGE_WALL_SECS, or find out what is running \
             this slowly",
            100.0 * game_time.as_secs_f64() / budget.as_secs_f64()),
    };

    println!(
        "\n════ C3: a walk of the finished game, from {name} ({:?}) ════\n\
         frontier   {discovered} ids offered, {visited} chosen, across {maps} maps in {turns} turns\n\
         cost       {game_time:?} of game time, {elapsed:?} of wall clock\n\
         rate       {:.1} ids discovered per game-minute\n\
         settled    {settled} ({stopped})\n\
         verdicts   {}\n\
         silent     {:?}\n\
         busiest    {busiest}\n\
         stuck      {stalled_worst} consecutive turns choosing nothing; {rowless} turn(s) had no rows at all\n\
         cheats     {bag}\n\
         where      {boxed_at}\n\
         table      {written:?}\n",
        start.map,
        discovered as f64 / (game_time.as_secs_f64() / 60.0).max(0.001),
        log.summary(),
        log.silent_kinds(),
        busiest = busiest,
        stalled_worst = stalled_worst,
        rowless = rowless,
        boxed_at = boxed_at.join("\n            "),
        bag = bag,
    );

    // Both taken as owned values here, so the borrow of the run's log ends before the cross-check
    // below reaches back into the same run for its MMU.
    let ids: std::collections::BTreeSet<String> =
        log.entries().map(|entry| entry.id.clone()).collect();
    let failures = log.failures();

    // §5.3, and it is printed rather than asserted on purpose: the ROM's tables are a cross-check,
    // not the universe (§0.1). ⚠️ Over the ids this run was offered, so it reports on the maps this
    // walk reached and no others.
    println!("{}", rom_cross_check(run.fixture().gb.core().mmu(), &ids));

    // ⚠️ **Maps come off the ids rather than off the brain's own tally**, so that the union across
    // regions is the same arithmetic as `CoverageLog::maps_touched` and the two can be compared. An
    // id is `{map}:…` and `actions()` only ever mints rows for the map the player is standing on,
    // so an id's prefix is a map the walk stood on.
    let maps = ids.iter().filter_map(|id| id.split(':').next().map(str::to_string)).collect();
    WalkOutcome {
        name,
        ids,
        maps,
        failures,
        turns,
        game_time,
        wall: elapsed,
        stopped,
    }
}

/// **§5.3 — the ROM's own tables, as a cross-check rather than as the universe.**
///
/// The plan settled early (§0.1) that `read_warp_events`, `header.connections()` and
/// `map.sprites()` do **not** define what the walk should have covered: what the model may choose is
/// what `MetaTileMap::actions` mints, and a row correctly withheld — a tree with no Cut, a boulder
/// with no Strength, an item already in the bag — is the agent working rather than a gap. So this
/// asserts nothing. It answers one question the frontier cannot ask itself: *is there something in
/// the ROM that never once appeared as a row?* That is either a gate doing its job, or a decision
/// the model is never offered and nobody can see — and the second is invisible today.
///
/// ⚠️ **Only over maps the walk actually entered.** A warp on a map never visited is W2's ceiling,
/// not a missing row, and mixing the two would bury the finding under two hundred maps of "never
/// went there".
///
/// ⚠️ **A missing row is classified where it can be, because the raw list is mostly noise.**
/// `actions()` emits one warp row per unique *destination*, so the Mansion's four bottom exits are
/// one door and three of them are correctly never rows; a warp square with no walkable sub-tile is
/// dropped on purpose (`meta_tiles_base`); and every item ball on a finished save is `hidden`,
/// having already been picked up. Those three are named and set aside, and what is left over is the
/// part worth reading.
///
/// ⚠️ **Sprite ids are matched exactly, which they could not have been before 2026-09-08.** A sprite
/// row is keyed on `map + name` now ([`OverworldAction::id`](crate::pokemon::actions::OverworldAction::id)),
/// so "did this object ever appear" is one lookup. While the id carried the player's approach
/// square there was no id to look up: the same object appeared under up to eleven of them.
pub fn rom_cross_check(
    mmu: &crate::mmu::MMU,
    offered: &std::collections::BTreeSet<String>,
) -> String {
    use crate::pokemon::map::Map;
    use crate::pokemon::map_metadata::{MapMetadataCache, MapMetadataReader};
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
            // A handful of maps are drawn from RAM rather than from the ROM's block list
            // (`map_uses_runtime_blocks`). Named rather than skipped silently: an absence in this
            // report has to mean "nothing missing", never "not looked at".
            Err(why) => { unreadable.push(format!("{map}: {why}")); continue }
        };
        let dims = metadata.dimensions();
        let was_offered = |id: &str| offered.contains(id);
        let mut said: Vec<String> = Vec::new();

        // Where a warp *would* be minted: the ROM's square, shifted by this map's connection strips,
        // exactly as `meta_tiles_base` places it.
        let square = |warp: &crate::pokemon::tile::WarpEvent| {
            (warp.position.x as usize + dims.west_extra, warp.position.y as usize + dims.north_extra)
        };
        for warp in &metadata.warp_events {
            warps += 1;
            let (mx, my) = square(warp);
            if was_offered(&format!("{map}:{mx},{my}:Warp")) { continue }
            warps_missing += 1;
            // The destination is what `actions()` dedupes on, so a sibling that leads to the same
            // place and *was* offered is the reason this one is not a row.
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
                    "({mx}, {my}) → {}: ⚠️ **on the grid, no sibling, and never a row**",
                    warp.destination_map)),
            }
        }

        for sprite in map.sprites() {
            objects += 1;
            let id = format!("{map}:{}", MetaTile::Sprite(sprite.name).id_kind());
            if was_offered(&id) { continue }
            match sprite.hidden_object_id {
                // A toggleable object: an item ball already in the bag, or something a script has
                // not put on the map yet. On the finished save the walk starts from, most of the
                // game's item balls are in this state and none of them is a finding.
                Some(_) => gated_missing += 1,
                // ⚠️ **A boulder is a sprite and does get a talk row** — `VictoryRoad1F:Boulder1` is
                // one, minted by `actions()`'s sprite scan like any other — so it belongs in this
                // check rather than out of it. It is counted apart because facing a boulder is not
                // the decision anyone came for: the same rock is offered as a `PushBoulder*` goal,
                // and a walk that took the goal and never faced the rock has missed nothing.
                None if sprite.name.starts_with("Boulder") => boulders_missing += 1,
                None => { npcs_missing += 1;
                    said.push(format!("{:?}: ⚠️ **a person on this map who was never a row**", sprite.name)) }
            }
        }

        // Connections are counted rather than matched: a `Connection` id names the crossing tile
        // and not the map it leads to, so which neighbour a row was for cannot be recovered from
        // the id alone. ⚠️ The count going the *other* way is a finding of its own —
        // `actions()` mints one crossing per adjacent map and picks the nearest, so the coordinate
        // moves with the player exactly as a sprite's used to (§5.2.8). Route 3 carried seven
        // Connection ids for three neighbours on the sweep of 2026-09-08.
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
                               coordinate is moving with the player (§5.2.8)"));
        }

        if !said.is_empty() {
            lines.push(format!("  {map}\n    {}", said.join("\n    ")));
        }
    }

    format!(
        "\n──── §5.3: the ROM's tables against what was offered ────\n\
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

