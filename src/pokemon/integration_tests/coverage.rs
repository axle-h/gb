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
//! | `Unknown`, `DidNotArrive`, `NoRoute`, `WrongMap`, `NoAdjacentGrass` | `Defect` |
//! | `WatchdogFired` | `Defect`, always |
//! | a start with no terminal event before the next one | `Silent` — chosen, outcome never reported |
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
    Defect { reason: String },
    /// ⭐ **The action was taken and the agent never said what became of it.** A
    /// `StartedOverworldAction` with no terminal event before the next one.
    ///
    /// ⚠️ **This is a gap in the *event stream*, not in the agent's behaviour, and C3 found it on
    /// its first walk from Pallet Town** — 66 of 307 chosen ids, and **every single one** of them a
    /// `Grass` (52) or a `CutTree` (14). The cause is in `agent.rs`: reaching tall grass hands over
    /// to
    /// `AgentState::PacingForEncounters` without an event, a cave wander (`MetaTile::Empty`) does
    /// the same, and pacing then ends either at a battle — whose `assert_battle_state` arm for a
    /// non-`OverworldMovement` state emits only `BattleStarted`, with no abort — or at its own
    /// budget, which emits a `TextBox`. So "walk in grass" is an action the model is offered and is
    /// never told the outcome of.
    ///
    /// ⚠️ **Deliberately not a defect yet.** Nothing went wrong in the game; what is missing is the
    /// sentence. Failing the walk on it would make C3 permanently red for a reason C3 cannot fix,
    /// and the number is more useful reported than fatal. Closing the gap is an `AgentEvent` change
    /// and belongs with the rest of the prose the model reads.
    Silent,
}

impl Verdict {
    /// Whether this verdict fails a coverage run.
    pub fn is_defect(&self) -> bool {
        match self {
            Self::Defect { .. } => true,
            Self::Blocked { times, .. } => *times >= REPEAT_IS_A_DEFECT,
            // See `Silent`'s own note for why it is reported rather than fatal.
            Self::Unreached | Self::Completed | Self::Silent => false,
        }
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
            AgentEvent::OverworldActionAborted { reason, .. } => {
                let Some(id) = self.open.take() else { return };
                self.entry(&id).aborts.entry(reason.to_string()).and_modify(|n| *n += 1).or_insert(1);
                match reason {
                    // The game asked a different question. The action is untouched and will be
                    // re-issued; leaving the verdict alone is what makes it retryable.
                    OverworldActionAbortedReason::Battle
                    | OverworldActionAbortedReason::NamingScreen => {}
                    // The game spoke. Expected once; the message usually arrives just after this.
                    OverworldActionAbortedReason::Textbox | OverworldActionAbortedReason::Script => {
                        let message = self.last_blocked.clone();
                        self.block(&id, message);
                        // Re-opened so the quote that follows lands on this id — see `last_blocked`.
                        self.open = Some(id);
                    }
                    // The menu offered a row the agent could not then execute. Nothing about the
                    // world explains these; they are the agent's own failure to do what it offered.
                    other => self.entry(&id).verdict = Verdict::Defect { reason: other.to_string() },
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

    /// Everything that fails a coverage run, as one line each. Empty means the run is clean.
    pub fn defects(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .entries
            .values()
            .filter(|entry| entry.verdict.is_defect())
            .map(|entry| match &entry.verdict {
                Verdict::Defect { reason } => format!("{}: {reason}", entry.id),
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
                Verdict::Defect { reason } => ("defect", reason.clone()),
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

    fn started(id: &str) -> AgentEvent {
        AgentEvent::StartedOverworldAction { destination: MetaTile::Grass, id: id.to_string() }
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
    /// Turns since an id was seen for the first time. The fixpoint of (4).
    pub barren: usize,
    pub turns: usize,
}

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
            barren: 0,
            turns: 0,
        }
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
        let known: Vec<&str> = description
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|word| self.seen.keys().any(|id| id.starts_with(&format!("{word}:"))))
            .collect();
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
            *self.maps.entry(map).or_insert(0) += 1;
        }

        let rows = request.menu_rows();
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
        let chosen = rows
            .iter()
            .map(|(id, _)| id)
            .find(|id| unvisited(id) && !is_a_way_out(id))
            .cloned()
            .or_else(|| {
                rows.iter()
                    .filter(|(id, _)| is_a_way_out(id))
                    // ⚠️ **How often it has already been taken comes *first*, and the promise of
                    // where it goes second.** The other order looks obviously right and loops for
                    // ever: an exit into a map the walk has never reached scores best on promise,
                    // and being turned back at it does not change anything the brain can see — so
                    // Brock's gym guide, who blocks Pewter's east exit until the Boulder Badge, held
                    // the walk on `PewterCity:40,18:Connection` for **59 attempts**. With the count
                    // first, every exit is taken once before any is taken twice, and promise decides
                    // the order within a pass, which is what it is for.
                    .min_by_key(|(id, description)| {
                        (self.seen.get(id).copied().unwrap_or(0), self.promise_of(description))
                    })
                    .map(|(id, _)| id.clone())
            });

        match chosen {
            Some(id) => {
                *self.seen.entry(id.clone()).or_insert(0) += 1;
                Reply::call(
                    "choose_action",
                    // ⚠️ `resume_after_battle`: a wild encounter says nothing about the walk, and
                    // without this every patch of grass costs a turn to re-issue one word for word.
                    serde_json::json!({ "id": id, "resume_after_battle": true }),
                )
            }
            // Boxed in: no row at all. The turn still has to end, and the situation says so.
            None => Reply::Calls(vec![Call::wait(20)]),
        }
    }
}

/// **C3's walk.** Every reachable action from a starting save, taken once, with a verdict on each.
///
/// ⚠️ **Its first deliverable is a number, not a pass**: ids discovered per game-minute, and the
/// shape of the curve (§5.6). The budget follows the measurement, so this prints its own and the
/// caller decides whether to buy more.
///
/// ⚠️ **It fails on any `defect`**, which is the whole point of the verdict oracle: a row the menu
/// offered and the agent could not then execute, or a watchdog firing. A `blocked` is not a failure
/// until it repeats — being stopped is how this game says almost everything.
#[test]
#[cfg(feature = "coverage-tests")]
fn coverage_walk_from_pallet_town() {
    use crate::pokemon::integration_tests::cheats::Cheats;
    use crate::pokemon::integration_tests::llm_harness::LlmRun;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// Turns with nothing new before the frontier is called settled. Generous: a walk that has just
    /// crossed into a new building spends several turns on rows it has already seen.
    const PATIENCE: usize = 60;
    /// How much game time this walk may spend. The measurement below says what that bought.
    ///
    /// ⚠️ **A bound, not a target, and it is `min`'d against the fixture's own cap deliberately.**
    /// §9's last risk is that the fixpoint keeps discovering rows and the walk never terminates —
    /// the answer is to *cap the passes and report a non-empty frontier as a result* rather than to
    /// hang. The fixture's cycle budget is a panic; this is a stop. The first frontier heuristic
    /// settled after 30 game-minutes because it could only bounce between two maps; the one that
    /// reads a row's prose for a destination was still discovering at 30 and hit the cap.
    const BUDGET: Duration = Duration::from_mins(90);

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
    let mut run = LlmRun::builder(crate::pokemon::integration_tests::PALLET_TOWN_STATE)
        .named("coverage-walk")
        // Twice `BUDGET`, so the walk always stops on its own bound rather than on the fixture's
        // panic. The two are different failures and only one of them is a result.
        .game_time(BUDGET * 2)
        .with_coverage()
        .start(Box::new(brain.clone()));
    run.with_cheats(Cheats::default());

    let started = std::time::Instant::now();
    let mut spent_the_budget = false;
    let settled = run.tick_until(Duration::from_secs(900), |run| {
        spent_the_budget |= run.fixture().total_cycles.to_duration() >= BUDGET;
        spent_the_budget || brain.0.lock().expect("not poisoned").settled(PATIENCE)
    }) && !spent_the_budget;
    let elapsed = started.elapsed();

    let (discovered, visited, maps, turns) = {
        let brain = brain.0.lock().expect("not poisoned");
        (brain.discovered(), brain.visited(), brain.maps(), brain.turns)
    };
    // Everything the menu offered and the walk never chose is `Unreached` rather than absent.
    let offered = brain.0.lock().expect("not poisoned").offered_ids();
    let game_time = run.fixture().total_cycles.to_duration();
    {
        let log = run.fixture().coverage.as_mut().expect("coverage was asked for");
        for id in &offered {
            log.offered(id);
        }
    }
    let log = run.coverage().expect("coverage was asked for");
    let written = log.write_report("walk-from-pallet-town");

    println!(
        "\n════ C3: a walk from Pallet Town ════\n\
         frontier   {discovered} ids offered, {visited} chosen, across {maps} maps in {turns} turns\n\
         cost       {game_time:?} of game time, {elapsed:?} of wall clock\n\
         rate       {:.1} ids discovered per game-minute\n\
         settled    {settled} ({})\n\
         verdicts   {}\n\
         silent     {:?}\n\
         table      {written:?}\n",
        discovered as f64 / (game_time.as_secs_f64() / 60.0).max(0.001),
        match settled {
            true => format!("{PATIENCE} turns with nothing new"),
            false => format!("stopped on the {BUDGET:?} budget with the frontier still open"),
        },
        log.summary(),
        log.silent_kinds(),
    );

    let defects = log.defects();
    assert!(defects.is_empty(), "the walk found {} defects:\n  {}", defects.len(), defects.join("\n  "));
    assert!(discovered > 10, "only {discovered} ids were ever offered; the walk did not happen");
}
