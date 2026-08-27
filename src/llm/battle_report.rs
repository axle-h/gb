//! What happened in a battle nobody was asked about.
//!
//! A scripted battle costs no requests, which is the whole point of it — and means the model would
//! otherwise learn nothing at all about a fight it just had. This is the feedback loop that makes
//! scripting safe to leave armed: one report, folded into the next turn's situation, saying what the
//! script did, what the game said back, and what it cost.
//!
//! ⚠️ **The numbers are inferred and the prose is the cartridge's, because there is no third
//! option.** This codebase has no per-turn battle outcome event: `BattleActionStarted` is the
//! *intent*, emitted the moment the policy commits, and the enemy's action is never an event at
//! all. So damage is a diff of the HP seen at consecutive decisions, and everything else —
//! "It's super effective!", "SPARKY fainted!", "SPARKY gained 56 EXP!", the prize money — is read
//! off the game's own message boxes, which `PokemonTextReader` was already collecting.
//!
//! ⚠️ **A turn's damage is only known at the *next* decision**, so a turn is held open until then
//! and closed by whatever comes after it: the next decision, or the end of the battle. That is why
//! [`BattleReport::finish`] takes a final state rather than being a plain `render`.
//!
//! ⚠️ **The report is rendered into the situation rather than appended as a message of its own.**
//! The plan is a message because it is re-read every turn and wants the prefix cache
//! ([`crate::llm::worker::Worker::sync_plan`]); a battle report is read once, by the turn straight
//! after it, and appending it separately would put a permanent extra message in a history that is
//! already compacted for length. The situation is fresh tokens either way.

use crate::pokemon::GameState;
use crate::pokemon::battle::{BattleAction, BattleType};

/// How much of one report the model is shown. A long trainer battle is elided in the middle rather
/// than truncated at the end: the turns worth reading are the first few, where the script's plan is
/// visible, and the last few, where it went wrong.
pub const MAX_TURNS_SHOWN: usize = 8;
/// How much of one message box is quoted.
///
/// ⚠️ **Head *and* tail, because the outcome is always at the end.** The first draft kept the head
/// alone, on the argument that only the first sentence of each box is the point — which is true of
/// the middle of a battle and false of the sentence that ends one. A blackout arrives as
/// `"… Ember fainted! AI is out of useable POKéMON! AI blacked out!"`, 131 bytes, and a head-only
/// cut landed on `"AI is out of useable POKéMON! A…"`: the two words saying what had happened were
/// the two the truncation removed. The deployed run of 2026-08-27 lost to Misty six times and was
/// never once told so.
const MAX_QUOTE: usize = 120;
/// How much of [`MAX_QUOTE`] is spent on the end of the box rather than the beginning.
const QUOTE_TAIL: usize = 44;
/// How many battles may queue up before the model is next asked anything. Deliberately small: past
/// this the reports are describing a stretch of play the model can do nothing about, and the count
/// says more than the detail.
pub const MAX_QUEUED: usize = 3;

/// One side of the battle at the moment a decision was taken.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Side {
    name: String,
    hp: u16,
    max: u16,
}

impl Side {
    /// `RATTATA 21 → 4`, or nothing at all when the number did not move. ⚠️ **Silence when nothing
    /// happened is the point**: a status move, a miss and a failed run all leave both HP bars where
    /// they were, and printing `34 → 34` twice a turn would bury the turns that did something.
    fn delta(&self, then: &Side) -> Option<String> {
        if self.hp == then.hp && self.max == then.max {
            return None;
        }
        // ⚠️ **A changed maximum is a level-up, and reporting only the current HP makes it read as
        // healing.** Winning a battle takes a Pokémon from `30/54` to `33/57`, and `Celina 30 → 33`
        // in a column of damage lines says it was healed for 3.
        if self.max != then.max {
            return Some(format!("{} {}/{} → {}/{}", self.name, then.hp, then.max, self.hp, self.max));
        }
        Some(format!("{} {} → {}", self.name, then.hp, self.hp))
    }
}

/// One decision, held open until the next one tells us what it did.
#[derive(Debug, Clone)]
struct Turn {
    number: u32,
    /// What the script chose, as a verb phrase.
    intent: String,
    /// Both sides as they stood when it was chosen.
    me: Side,
    foe: Side,
    /// What the game said afterwards, and what the script printed before choosing.
    said: Vec<String>,
    prints: Vec<String>,
}

/// A battle being written up as it is fought.
#[derive(Debug, Clone)]
pub struct BattleReport {
    kind: BattleType,
    /// Who it started against, kept because the foe can change mid-battle in a trainer fight.
    opener: String,
    closed: Vec<(Turn, Option<String>, Option<String>)>,
    open: Option<Turn>,
    /// Turns the script handed back with `battle.ask()`, which cost a request and are worth
    /// counting separately from the ones that did not.
    asked: u32,
    /// Which party slot was out when the battle opened, so the closing line can find our Pokémon in
    /// the party once `wBattleMon` is gone.
    my_slot: usize,
    /// Both sides as they last stood, for the closing line. `None` until the battle ends, and the
    /// foe is `None` when the battle was already over by the time anything could be read.
    ///
    /// ⚠️ **Read rather than inferred, and it says nothing it cannot see.** "You won" is not a fact
    /// this module has: a battle ends on a faint, a capture, a successful run and a trainer running
    /// out of Pokémon, and three of those look identical from here. The HP of both sides at the end
    /// is a fact, and the cartridge's own sentences quoted above it say the rest.
    ending: Option<(Side, Option<Side>)>,
    /// The cartridge said the player blacked out. See [`is_blackout`] for why this is the only
    /// witness, and [`Self::finish`] for what it stops the report claiming.
    blacked_out: bool,
    /// Where `LlmPolicy::events` stood when this battle began.
    ///
    /// ⚠️ **This is what stops the report being said twice.** The agent's own events are folded into
    /// the next turn under `### Since your last decision`, so without it a scripted battle's whole
    /// message-box stream would appear there *and* here, in two shapes, in the same request.
    pub events_mark: usize,
}

impl BattleReport {
    /// Open a report for the battle `state` is in, if it is in one.
    pub fn open(state: &GameState, events_mark: usize) -> Option<Self> {
        let battle = state.battle.as_ref()?;
        let me = side(state, true)?;
        let foe = side(state, false)?;
        Some(Self {
            kind: battle.battle_type,
            opener: format!(
                "{} Lv{} against {} Lv{}",
                foe.name, battle.enemy.level, me.name, battle.player.level,
            ),
            closed: Vec::new(),
            open: None,
            asked: 0,
            my_slot: battle.active_party_slot as usize,
            ending: None,
            blacked_out: false,
            events_mark,
        })
    }

    /// The script chose something. Whatever was open is closed against `state` first.
    pub fn decided(&mut self, state: &GameState, action: &BattleAction, prints: Vec<String>) {
        self.close_in_battle(state);
        self.open = Some(Turn {
            number: self.closed.len() as u32 + self.asked + 1,
            intent: intent(action),
            me: side(state, true).unwrap_or_else(unknown),
            foe: side(state, false).unwrap_or_else(unknown),
            said: Vec::new(),
            prints,
        });
    }

    /// The script handed this turn back — `battle.ask()`, or a failure. The model is about to be
    /// asked, so the turn itself needs no line here; only the count does.
    pub fn handed_back(&mut self, state: &GameState) {
        self.close_in_battle(state);
        self.asked += 1;
    }

    /// Something the game said. Attributed to the turn that is open, or to the last one closed when
    /// the battle is ending.
    pub fn said(&mut self, message: &str) {
        let message = message.trim();
        if message.is_empty() {
            return;
        }
        // ⚠️ **Tested before the truncation, not after.** The two words are at the very end of a
        // box that is longer than `MAX_QUOTE`, which is the whole reason this flag exists.
        self.blacked_out |= is_blackout(message);
        let quoted = truncated(message, MAX_QUOTE);
        match self.open.as_mut() {
            Some(turn) => turn.said.push(quoted),
            None => match self.closed.last_mut() {
                Some((turn, ..)) => turn.said.push(quoted),
                None => {}
            },
        }
    }

    /// The battle is over. `state` is the last one seen, which is where the closing HP comes from.
    /// The battle is over. `state` is the last one seen, which is where the closing HP comes from.
    ///
    /// ⚠️ **A state with no battle in it is the *ordinary* case, not a failure.** `service_tools`
    /// runs only at decision points, so a wild Pokémon knocked out in one turn is never observed
    /// again while `wIsInBattle` is set — there is no second decision to observe it at. What
    /// survives is the party, so our own side is still exact and the enemy's simply is not reported.
    /// Guessing it from the last decision would print the HP it *started* the turn on as though it
    /// were the HP it ended on.
    pub fn finish(mut self, state: Option<&GameState>) -> String {
        match state {
            // ⚠️ **First, above the in-battle arm, because a state that still holds a battle after a
            // blackout is a *different* battle** — the trainer waiting on the other side of the
            // Centre, or a wild encounter on the walk back — and its HP bars are nothing to do with
            // this one.
            //
            // ⚠️ **A blackout heals the party before this can read it, so the party is not evidence
            // about the battle any more — it is evidence about the recovery.** The arm below reads
            // our side out of `state.pokemon`, which after
            // `ResetStatusAndHalveMoneyOnBlackout` is every Pokémon at full HP. That turned the
            // losing turn's delta into `Ember 12 → 76` and closed six consecutive defeats with
            // "Ended with Ember on 76/76 HP", on a run whose only account of a scripted battle is
            // this report. No number at all is the honest answer; `render` says what happened
            // instead, in the cartridge's own terms.
            _ if self.blacked_out => self.close(None, None),
            Some(state) if state.battle.is_some() => {
                self.close_in_battle(state);
                if let (Some(me), Some(foe)) = (side(state, true), side(state, false)) {
                    self.ending = Some((me, Some(foe)));
                }
            }
            Some(state) => match party_side(state, self.my_slot) {
                Some(me) => {
                    self.close(Some(&me), None);
                    self.ending = Some((me, None));
                }
                None => self.close(None, None),
            },
            None => self.close(None, None),
        }
        self.render()
    }

    /// How many decisions this battle took, which is the figure the whole feature is about.
    pub fn decisions(&self) -> usize {
        self.closed.len() + usize::from(self.open.is_some()) + self.asked as usize
    }

    /// Close the open turn against whichever sides can still be read.
    ///
    /// ⚠️ **Compared by name as well as by number.** A switch or a trainer sending out their next
    /// Pokémon replaces one side entirely, and reporting `RATTATA 21 → 30` when the 30 belongs to a
    /// PIDGEY is worse than reporting nothing.
    fn close(&mut self, me: Option<&Side>, foe: Option<&Side>) {
        let Some(turn) = self.open.take() else { return };
        let my_delta = me.filter(|now| now.name == turn.me.name).and_then(|now| now.delta(&turn.me));
        let foe_delta = foe.filter(|now| now.name == turn.foe.name).and_then(|now| now.delta(&turn.foe));
        self.closed.push((turn, my_delta, foe_delta));
    }

    /// [`Self::close`] against a state that still has a battle in it.
    fn close_in_battle(&mut self, state: &GameState) {
        let (me, foe) = (side(state, true), side(state, false));
        self.close(me.as_ref(), foe.as_ref());
    }

    fn render(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push_str("### Battle report\n\n");
        // ⚠️ **Facts, in the order they happened, and no sentence about how little it cost.** The
        // first draft opened "A wild battle you did not have to answer" and counted the decisions
        // "you were not asked about" — which is the report congratulating itself for existing, on
        // every battle, for the length of the run. What the model needs is what happened; the
        // saving is real whether or not the prose points at it.
        out.push_str(&format!(
            "{} battle. {}.\n",
            match self.kind {
                BattleType::Wild => "Wild",
                BattleType::Trainer => "Trainer",
                BattleType::Safari => "Safari",
            },
            self.opener,
        ));
        let decisions = self.decisions();
        out.push_str(&format!(
            "{decisions} turn{}{}.\n\n",
            match decisions == 1 { true => "", false => "s" },
            match self.asked {
                0 => String::new(),
                asked => format!(", {asked} of them answered by you"),
            },
        ));

        // First few and last few. ⚠️ The middle is where a long battle repeats itself; the ends are
        // where the script's plan and its consequences are.
        let shown: Vec<usize> = match self.closed.len() > MAX_TURNS_SHOWN {
            false => (0..self.closed.len()).collect(),
            true => {
                let head = MAX_TURNS_SHOWN / 2;
                let tail = self.closed.len() - (MAX_TURNS_SHOWN - head);
                (0..head).chain(tail..self.closed.len()).collect()
            }
        };
        let mut last = None;
        for index in shown {
            if last.is_some_and(|last| index > last + 1) {
                out.push_str(&format!("… {} more turns like these\n", index - last.unwrap() - 1));
            }
            last = Some(index);
            let (turn, my_delta, foe_delta) = &self.closed[index];
            out.push_str(&format!("{}. {}", turn.number, turn.intent));
            for delta in [foe_delta, my_delta].into_iter().flatten() {
                out.push_str(&format!(". {delta}"));
            }
            out.push_str(".\n");
            for line in &turn.said {
                out.push_str(&format!("   \"{line}\"\n"));
            }
            for line in &turn.prints {
                out.push_str(&format!("   your script said: {line}\n"));
            }
        }

        // How it stood at the end. ⚠️ **Not "you won"** — see `ending`: a faint, a capture and a
        // successful run are indistinguishable from here, and a report that guessed would be wrong
        // on every Poké Ball the script ever threw.
        if let Some((me, foe)) = self.ending.as_ref() {
            match foe {
                Some(foe) => out.push_str(&format!("\nEnded with {} and {}.\n", standing(foe), standing(me))),
                // ⚠️ The enemy's HP goes with the battle; ours is in the party. Reporting the half
                // that is still readable beats inventing the other.
                None => out.push_str(&format!("\nEnded with {}.\n", standing(me))),
            }
        }
        // ⚠️ **This is not the verdict the ⚠️ on `ending` refuses to guess.** That one is about
        // reading a *result* out of the HP, where a faint, a capture and a successful run are
        // indistinguishable. A blackout is none of those: the cartridge said it out loud, and what
        // follows from it is fixed — `ResetStatusAndHalveMoneyOnBlackout` halves the money, and
        // `SetLastBlackoutMap` (written only when a heal was *accepted*) decides where the player
        // wakes up. Without this the situation around the report reads as an ordinary walk out of
        // the building, at full HP, with the money quietly halved and nothing pointing at it.
        if self.blacked_out {
            out.push_str(
                "\n**You lost. Your last Pokémon fainted, so you blacked out.** The game has taken \
                 you back to the Pokémon Center you last accepted a heal at and half your money is \
                 gone. Your party is at full HP now because blacking out healed it, not because the \
                 battle went well.\n",
            );
        }
        out.push('\n');
        out
    }
}

/// One side of the battle now, named the way the model already knows it.
fn side(state: &GameState, mine: bool) -> Option<Side> {
    let battle = state.battle.as_ref()?;
    let summary = match mine {
        true => &battle.player,
        false => &battle.enemy,
    };
    let name = match mine {
        // The nickname, because that is what the model chose and what every other line calls it.
        true => state
            .pokemon
            .get(battle.active_party_slot as usize)
            .map(|mon| mon.nickname.to_default_string())
            .unwrap_or_else(|| summary.species.to_string()),
        // The species, because a wild Pokémon has no name the player can see.
        false => summary.species.to_string(),
    };
    Some(Side { name, hp: summary.current_hp, max: summary.stats.hp })
}

/// `RATTATA fainted` or `SPARKY on 44/48 HP`.
fn standing(side: &Side) -> String {
    match side.hp {
        0 => format!("{} fainted", side.name),
        hp => format!("{} on {hp}/{} HP", side.name, side.max),
    }
}

/// Our active Pokémon read out of the **party**, which is where its HP still is once the battle has
/// gone and taken `wBattleMon` with it.
fn party_side(state: &GameState, slot: usize) -> Option<Side> {
    let mon = state.pokemon.get(slot)?;
    Some(Side { name: mon.nickname.to_default_string(), hp: mon.current_hp, max: mon.stats.hp })
}

fn unknown() -> Side {
    Side { name: String::new(), hp: 0, max: 0 }
}

/// What the script did, as a verb phrase.
///
/// ⚠️ **Not `BattleAction`'s `Display`, which is a *menu row*** — `FIGHT Ember PP 24` is what the
/// option looks like before it is chosen, and reads as nonsense in an account of what happened. It
/// also carries an em dash on the switch row (`PKMN Squirtle Lv12 — 292/292 HP`), which is not
/// allowed anywhere a model or a viewer reads.
///
/// ⚠️ **Shared with `battle_script`'s validation table** rather than written twice: what a script
/// *would* do on a made-up battle and what it *did* on a real one are the same sentence, and a
/// model that reads one phrasing when it installs a script and another when it gets the report has
/// two vocabularies to keep straight for no reason.
pub(crate) fn intent(action: &BattleAction) -> String {
    match action {
        BattleAction::Fight { battle_move, .. } => format!("used {}", battle_move.name),
        BattleAction::UseItem { item, .. } => format!("used a {}", item.id),
        BattleAction::SwitchPokemon { pokemon, .. } => format!("sent out {}", pokemon.species),
        BattleAction::Run => "tried to run".to_string(),
        BattleAction::SafariBall => "threw a Safari Ball".to_string(),
        BattleAction::SafariBait => "threw bait".to_string(),
        BattleAction::SafariRock => "threw a rock".to_string(),
    }
}

/// The first `limit - QUOTE_TAIL` bytes and the last `QUOTE_TAIL`, with the middle elided.
///
/// ⚠️ **Byte budgets over a `&str`, so both ends are taken a `char` at a time.** The cartridge's
/// prose carries `é` (`POKéMON`, in most of the sentences that end a battle) and `¥`, so slicing on
/// a byte index panics on exactly the boxes this exists to keep.
fn truncated(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let head: String = text
        .chars()
        .scan(0usize, |used, c| {
            *used += c.len_utf8();
            (*used <= limit.saturating_sub(QUOTE_TAIL)).then_some(c)
        })
        .collect();
    let tail: String = text
        .chars()
        .rev()
        .scan(0usize, |used, c| {
            *used += c.len_utf8();
            (*used <= QUOTE_TAIL).then_some(c)
        })
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    // Overlapping ends mean the whole thing fits after all, which the length test above already
    // ruled out — but a `limit` smaller than `QUOTE_TAIL` would reach here, so say it rather than
    // printing the same words twice.
    match head.len() + tail.len() >= text.len() {
        true => text.to_string(),
        false => format!("{head}…{tail}"),
    }
}

/// The cartridge saying the player has just blacked out.
///
/// ⚠️ **Matched on the game's own sentence rather than read out of RAM, because the byte that says
/// so is cleared by the blackout itself.** `wBattleResult` is set to `LOSE` by `HandlePlayerBlackOut`
/// and then zeroed — back to *win* — three instructions into
/// `ResetStatusAndHalveMoneyOnBlackout` (`engine/events/black_out.asm:3-4`), which runs before
/// anything here gets to look. The party is fully healed by the same routine and the money is
/// halved, so every number a report could read has already moved. The sentence is the only witness
/// left, and it is the one the model would be reading anyway.
pub fn is_blackout(message: &str) -> bool {
    message.contains("blacked out")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::move_name::PokemonMoveName;

    /// The scenarios `battle_script` validates against are exactly the shapes needed here.
    fn state() -> GameState {
        crate::llm::battle_script::test_scenario()
    }

    fn hurt(mut state: GameState, mine: u16, theirs: u16) -> GameState {
        if let Some(battle) = state.battle.as_mut() {
            battle.player.current_hp = mine;
            battle.enemy.current_hp = theirs;
        }
        state
    }

    fn ember() -> BattleAction {
        BattleAction::Fight {
            slot: 1,
            battle_move: crate::pokemon::move_name::PokemonMove::with_max_pp(PokemonMoveName::Ember),
        }
    }

    /// The whole shape: an intent, the damage it did, and what the game said, in one line each.
    #[test]
    fn a_report_says_what_happened_and_what_it_cost() {
        let start = state();
        let (my_hp, foe_hp) = {
            let battle = start.battle.as_ref().unwrap();
            (battle.player.current_hp, battle.enemy.current_hp)
        };

        let mut report = BattleReport::open(&start, 0).expect("a battle to report on");
        report.decided(&start, &ember(), vec!["going for the burn".to_string()]);
        report.said("ENEMY RATTATA used TACKLE!");
        let rendered = report.finish(Some(&hurt(state(), my_hp - 5, foe_hp - 17)));

        assert!(rendered.contains("used Ember"), "{rendered}");
        assert!(rendered.contains(&format!("{foe_hp} → {}", foe_hp - 17)), "the foe's damage: {rendered}");
        assert!(rendered.contains(&format!("{my_hp} → {}", my_hp - 5)), "and ours: {rendered}");
        assert!(rendered.contains("ENEMY RATTATA used TACKLE!"), "the cartridge's own words: {rendered}");
        assert!(rendered.contains("going for the burn"), "and the script's: {rendered}");
        // ⚠️ Facts only: the count, not a sentence about how little the turn cost. See `render`.
        assert!(rendered.contains("1 turn."), "the turn count is stated plainly: {rendered}");
        assert!(!rendered.contains("did not have to"), "and nothing congratulates itself: {rendered}");
        assert!(rendered.contains("Ended with"), "and it says how it stood at the end: {rendered}");
    }

    /// ⚠️ A turn where nothing moved says nothing about HP. Growl, a miss and a failed run all look
    /// like this, and `34 → 34` twice a turn would bury the turns that mattered.
    #[test]
    fn a_turn_that_changed_no_hp_prints_no_numbers() {
        let start = state();
        let mut report = BattleReport::open(&start, 0).unwrap();
        report.decided(&start, &ember(), Vec::new());
        let rendered = report.finish(Some(&state()));
        assert!(rendered.contains("used Ember"), "{rendered}");
        assert!(!rendered.contains('→'), "nothing moved, so no arrow: {rendered}");
    }

    /// ⚠️ A switch replaces a side outright, and attributing the new Pokémon's HP to the old one is
    /// the bug this guards. `close` compares the name as well as the number.
    #[test]
    fn a_side_that_was_replaced_is_not_reported_as_damaged() {
        let start = state();
        let mut report = BattleReport::open(&start, 0).unwrap();
        report.decided(&start, &ember(), Vec::new());

        // The same battle with a different Pokémon out: a big HP difference that is not damage.
        let mut swapped = state();
        if let Some(battle) = swapped.battle.as_mut() {
            battle.active_party_slot = 1;
            battle.player = swapped.pokemon.get(1).expect("a bench member").summary();
        }
        let rendered = report.finish(Some(&swapped));
        assert!(!rendered.contains('→'), "a replaced side reports no delta: {rendered}");
    }

    /// ⚠️ **The guard on why `LlmPolicy::last_battle_state` exists.** A report closed against a
    /// state with no battle in it has nothing to diff, so the turn that actually *won* the fight —
    /// the most interesting line in the whole report — is the one reported without a number.
    /// `BattleEnded` can easily arrive on a tick whose state has already cleared, which is why the
    /// policy keeps the last in-battle one rather than reaching for `self.state`.
    #[test]
    fn closing_against_a_finished_battle_loses_the_damage() {
        let start = state();
        let foe_hp = start.battle.as_ref().unwrap().enemy.current_hp;

        let mut over = state();
        over.battle = None;
        let mut lost = BattleReport::open(&start, 0).unwrap();
        lost.decided(&start, &ember(), Vec::new());
        assert!(!lost.finish(Some(&over)).contains('→'), "there is nothing to diff against");

        // The same battle closed against the last state that still had one keeps its numbers.
        let mut kept = BattleReport::open(&start, 0).unwrap();
        kept.decided(&start, &ember(), Vec::new());
        let rendered = kept.finish(Some(&hurt(state(), 20, foe_hp - 9)));
        assert!(rendered.contains(&format!("{foe_hp} → {}", foe_hp - 9)), "{rendered}");
    }

    /// ⚠️ **A battle ends and the enemy is gone with it.** `service_tools` runs only at decision
    /// points, so a wild Pokémon knocked out in one turn is never observed again while the battle is
    /// live — there is no second decision to observe it at. What survives is the party, so our side
    /// stays exact and the enemy is simply not reported. The alternative, closing against the last
    /// *decision's* state, printed the HP the foe started the turn on as though it were the HP it
    /// ended on.
    #[test]
    fn a_report_closed_after_the_battle_reads_our_side_out_of_the_party() {
        let start = state();
        let my_max = start.battle.as_ref().unwrap().player.stats.hp;

        let mut over = state();
        over.battle = None;
        if let Some(mon) = over.pokemon.get_mut(0) {
            mon.current_hp = my_max - 9;
        }

        let mut report = BattleReport::open(&start, 0).unwrap();
        report.decided(&start, &ember(), Vec::new());
        let rendered = report.finish(Some(&over));

        assert!(rendered.contains(&format!("on {}/{my_max} HP", my_max - 9)), "our HP is exact: {rendered}");
        assert!(rendered.contains(&format!("{my_max} → {}", my_max - 9)), "and so is the turn's: {rendered}");
        assert!(!rendered.contains("Rattata on"), "the enemy is not reported at all: {rendered}");
    }

    /// ⚠️ **A changed maximum is a level-up, not healing.** Winning takes a Pokémon from `30/54` to
    /// `33/57`, and `Celina 30 → 33` in a column of damage lines says it was healed for 3. Found by
    /// reading `probe_scripted_battles`' output rather than by a test.
    #[test]
    fn a_level_up_is_not_reported_as_healing() {
        let start = state();
        let (hp, max) = {
            let battle = start.battle.as_ref().unwrap();
            (battle.player.current_hp, battle.player.stats.hp)
        };

        let mut grown = state();
        grown.battle = None;
        if let Some(mon) = grown.pokemon.get_mut(0) {
            mon.current_hp = hp + 3;
            mon.stats.hp = max + 3;
        }

        let mut report = BattleReport::open(&start, 0).unwrap();
        report.decided(&start, &ember(), Vec::new());
        let rendered = report.finish(Some(&grown));
        assert!(
            rendered.contains(&format!("{hp}/{max} → {}/{}", hp + 3, max + 3)),
            "both numbers have to move or it reads as a heal: {rendered}",
        );
    }

    /// A long trainer battle is elided in the middle, not cut off at the end: the turns that matter
    /// are the opening plan and whatever it ran into.
    #[test]
    fn a_long_battle_is_elided_rather_than_truncated() {
        let start = state();
        let mut report = BattleReport::open(&start, 0).unwrap();
        for _ in 0..30 {
            report.decided(&start, &ember(), Vec::new());
        }
        let rendered = report.finish(Some(&start));

        assert!(rendered.contains("more turns like these"), "the middle is elided: {rendered}");
        assert!(rendered.contains("30 turns"), "the count is still honest: {rendered}");
        assert!(rendered.contains("1. used Ember"), "the first turn survives: {rendered}");
        assert!(rendered.contains("30. used Ember"), "and so does the last: {rendered}");
        assert!(rendered.len() < 2_000, "and it stays affordable at {} bytes", rendered.len());
    }

    /// ⚠️ **The closing line reports HP, never a result.** A faint, a capture and a successful run
    /// all end a battle and are indistinguishable from the HP alone, so a report that said "you
    /// won" would be wrong on every Poké Ball the script ever threw. The cartridge's own sentences,
    /// quoted above it, are what say which happened.
    #[test]
    fn the_ending_states_the_hp_rather_than_claiming_a_result() {
        let start = state();
        let foe_hp = start.battle.as_ref().unwrap().enemy.current_hp;

        // A wild Pokémon caught: it ends the battle in perfect health, and nobody won anything.
        let mut caught = BattleReport::open(&start, 0).unwrap();
        caught.decided(&start, &ember(), Vec::new());
        caught.said("All right! RATTATA was caught!");
        let rendered = caught.finish(Some(&state()));
        assert!(rendered.contains(&format!("on {foe_hp}/")), "the foe's HP is reported: {rendered}");
        for verdict in ["won", "lost", "defeated", "victory"] {
            assert!(!rendered.contains(verdict), "it must not claim `{verdict}`: {rendered}");
        }
        assert!(rendered.contains("was caught"), "what happened is the cartridge's line: {rendered}");

        // And a faint is said as a faint, because that one *is* visible in the HP.
        let mut beaten = BattleReport::open(&start, 0).unwrap();
        beaten.decided(&start, &ember(), Vec::new());
        assert!(beaten.finish(Some(&hurt(state(), 20, 0))).contains("fainted"));
    }

    /// ⚠️ **A blackout is the one ending whose numbers are gone by the time anything reads them.**
    /// `ResetStatusAndHalveMoneyOnBlackout` heals the whole party, so the state the report is
    /// finished against says full HP — and the deployed run of 2026-08-27 was therefore told, six
    /// times in a row, that a battle it had lost to Misty "ended with Ember on 76/76 HP", with the
    /// losing turn's delta reading as a heal. Both halves are asserted: no invented number, and the
    /// words that say what happened.
    #[test]
    fn a_blackout_is_reported_rather_than_read_off_a_healed_party() {
        let start = state();
        let mut report = BattleReport::open(&start, 0).unwrap();
        report.decided(&hurt(state(), 12, 40), &ember(), Vec::new());
        // One box, exactly as the cartridge sends it, and longer than `MAX_QUOTE`.
        let final_box = "Ember used SCRATCH! Enemy STARMIE used WATER GUN! It's super effective! \
                         Ember fainted! AI is out of useable POKéMON! AI blacked out!";
        assert!(final_box.len() > MAX_QUOTE, "the case only exists because the box is long");
        report.said(final_box);

        // Finished against the state the game leaves behind: no battle, party healed to full.
        let mut after = state();
        after.battle = None;
        let rendered = report.finish(Some(&after));

        assert!(rendered.contains("blacked out"), "the two words survive the quote: {rendered}");
        assert!(rendered.contains("You lost."), "and the report says so in its own line: {rendered}");
        // ⚠️ **Read out of the rendered string, never the source.** A multi-line Rust literal
        // without a trailing `\\` carries the whole indent into the prose; this repo has shipped
        // that twice.
        let sentence = rendered.lines().find(|l| l.contains("You lost.")).expect("asserted above");
        assert!(
            !sentence.contains("  "),
            "no continuation whitespace in the prose the model reads: {sentence:?}",
        );
        assert!(
            !rendered.contains("Ended with"),
            "a healed party is not how the battle ended: {rendered}",
        );
        assert!(
            !rendered.contains("12 → "),
            "and the heal must not be attributed to the losing turn: {rendered}",
        );
    }

    /// ⚠️ **The head-only truncation cut the answer off every box that had one.** What ends a battle
    /// is always the last sentence, so the quote keeps both ends.
    #[test]
    fn a_long_quote_keeps_the_sentence_that_ends_the_battle() {
        let start = state();
        let mut report = BattleReport::open(&start, 0).unwrap();
        report.decided(&start, &ember(), Vec::new());
        report.said(
            "Ember used SCRATCH! MISTY used X DEFEND on STARYU! Enemy STARYU's DEFENSE rose! \
             Enemy STARYU fainted! Ember gained 408 EXP. Points!",
        );
        let rendered = report.finish(Some(&state()));
        assert!(rendered.contains("Ember used SCRATCH!"), "the head is still there: {rendered}");
        assert!(rendered.contains("gained 408 EXP"), "and so is the tail: {rendered}");
        assert!(rendered.contains('…'), "with the middle elided: {rendered}");
    }

    /// `battle.ask()` and a disarm both land here, and both are worth counting apart: they are the
    /// turns the model *did* pay for.
    #[test]
    fn the_turns_the_model_answered_are_counted_separately() {
        let start = state();
        let mut report = BattleReport::open(&start, 0).unwrap();
        report.decided(&start, &ember(), Vec::new());
        report.handed_back(&start);
        report.decided(&start, &ember(), Vec::new());
        let rendered = report.finish(Some(&start));
        assert!(rendered.contains("3 turns, 1 of them answered by you"), "{rendered}");
    }

    /// The end of a battle is the one moment the model most wants quoted — the exp, the money, the
    /// faint — and by then no turn is open to attach it to.
    #[test]
    fn what_the_game_says_after_the_last_turn_is_still_reported() {
        let start = state();
        let mut report = BattleReport::open(&start, 0).unwrap();
        report.decided(&start, &ember(), Vec::new());
        report.said("Enemy RATTATA fainted!");
        report.handed_back(&start);
        report.said("SPARKY gained 56 EXP. Points!");
        let rendered = report.finish(Some(&start));
        assert!(rendered.contains("fainted"), "{rendered}");
        assert!(rendered.contains("56 EXP"), "the reward is the feedback: {rendered}");
    }

    #[test]
    fn a_very_long_message_box_is_quoted_rather_than_carried() {
        let start = state();
        let mut report = BattleReport::open(&start, 0).unwrap();
        report.decided(&start, &ember(), Vec::new());
        report.said(&"WORDS ".repeat(200));
        let rendered = report.finish(Some(&start));
        assert!(rendered.contains('…'), "it is marked as cut: {rendered}");
        assert!(rendered.len() < 600, "and actually cut, at {} bytes", rendered.len());
    }
}
