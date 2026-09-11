//! What happened in a battle nobody was asked about.

use crate::pokemon::GameState;
use crate::pokemon::battle::{BattleAction, BattleType};

/// Turns one report shows; a longer battle keeps its first and last and elides the middle.
pub const MAX_TURNS_SHOWN: usize = 8;
/// How much of one message box is quoted.
const MAX_QUOTE: usize = 120;
/// How much of [`MAX_QUOTE`] is spent on the end of the box rather than the beginning.
const QUOTE_TAIL: usize = 44;
/// Battles that may queue before the model is next asked; past this the count says enough.
pub const MAX_QUEUED: usize = 3;

/// One side of the battle at the moment a decision was taken.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Side {
    name: String,
    hp: u16,
    max: u16,
}

impl Side {
    /// `RATTATA 21 → 4`, or nothing at all when the number did not move.
    fn delta(&self, then: &Side) -> Option<String> {
        if self.hp == then.hp && self.max == then.max {
            return None;
        }
        // A changed maximum is a level-up, which the current HP alone would show as healing.
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
    /// Turns handed back with `battle.ask()`, counted apart because the model paid for them.
    asked: u32,
    /// Closed turns the model has been shown, so a hand-back reports only the rest.
    told: usize,
    /// The party slot out at the start, to find our Pokémon once `wBattleMon` is gone.
    my_slot: usize,
    /// Both sides as they last stood, for the closing line.
    ending: Option<(Side, Option<Side>)>,
    /// The cartridge said the player blacked out.
    blacked_out: bool,
    /// Where `LlmPolicy::events` stood when this battle began.
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
            told: 0,
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

    /// The script handed this turn back; answers the turns it took since the model last chose,
    /// without which the model sees its own last decision silently replaced.
    #[must_use]
    pub fn handed_back(&mut self, state: &GameState) -> Option<String> {
        self.close_in_battle(state);
        self.asked += 1;
        let since = std::mem::replace(&mut self.told, self.closed.len());
        if since >= self.closed.len() {
            return None;
        }
        Some(format!(
            "Your battle script took {} while you were not being asked:\n{}",
            match self.closed.len() - since {
                1 => "the turn before this one".to_string(),
                turns => format!("these {turns} turns"),
            },
            self.turns_from(since),
        ))
    }

    /// Something the game said, for the open turn or, as the battle ends, the last closed one.
    pub fn said(&mut self, message: &str) {
        let message = message.trim();
        if message.is_empty() {
            return;
        }
        // Tested before the truncation, which could cut the words.
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

    /// The battle is over; `state` is the last one seen, for the closing HP.
    pub fn finish(mut self, state: Option<&GameState>) -> String {
        match state {
            // Above the in-battle arm: a battle in `state` after a blackout is a different one,
            // whose HP has nothing to do with this.
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

    /// How many decisions this battle took.
    pub fn decisions(&self) -> usize {
        self.closed.len() + usize::from(self.open.is_some()) + self.asked as usize
    }

    /// Close the open turn against whichever sides can still be read.
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

    /// The closed turns from `from` on, as the lines the model reads them in.
    fn turns_from(&self, from: usize) -> String {
        let mut out = String::with_capacity(256);
        let shown: Vec<usize> = match self.closed.len() - from > MAX_TURNS_SHOWN {
            false => (from..self.closed.len()).collect(),
            true => {
                let head = MAX_TURNS_SHOWN / 2;
                let tail = self.closed.len() - (MAX_TURNS_SHOWN - head);
                (from..from + head).chain(tail..self.closed.len()).collect()
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
        out
    }

    fn render(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push_str("### Battle report\n\n");
        // Facts, in order, and no sentence about how little it cost.
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

        out.push_str(&self.turns_from(0));

        if let Some((me, foe)) = self.ending.as_ref() {
            match foe {
                Some(foe) => out.push_str(&format!("\nEnded with {} and {}.\n", standing(foe), standing(me))),
                // The enemy's HP goes with the battle; ours is in the party.
                None => out.push_str(&format!("\nEnded with {}.\n", standing(me))),
            }
        }
        // The one verdict stated: the HP read afterwards is a healed party.
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

/// Our active Pokémon read out of the party, where its HP is once `wBattleMon` has gone.
fn party_side(state: &GameState, slot: usize) -> Option<Side> {
    let mon = state.pokemon.get(slot)?;
    Some(Side { name: mon.nickname.to_default_string(), hp: mon.current_hp, max: mon.stats.hp })
}

fn unknown() -> Side {
    Side { name: String::new(), hp: 0, max: 0 }
}

/// What the script did, as a verb phrase.
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
    // Only a `limit` below `QUOTE_TAIL` gets here with overlapping ends.
    match head.len() + tail.len() >= text.len() {
        true => text.to_string(),
        false => format!("{head}…{tail}"),
    }
}

/// The cartridge's own sentence, because `wBattleResult` is zeroed before anything here runs.
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
        assert!(rendered.contains("1 turn."), "the turn count is stated plainly: {rendered}");
        assert!(!rendered.contains("did not have to"), "and nothing congratulates itself: {rendered}");
        assert!(rendered.contains("Ended with"), "and it says how it stood at the end: {rendered}");
    }

    #[test]
    fn a_turn_that_changed_no_hp_prints_no_numbers() {
        let start = state();
        let mut report = BattleReport::open(&start, 0).unwrap();
        report.decided(&start, &ember(), Vec::new());
        let rendered = report.finish(Some(&state()));
        assert!(rendered.contains("used Ember"), "{rendered}");
        assert!(!rendered.contains('→'), "nothing moved, so no arrow: {rendered}");
    }

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

    /// The guard on why `LlmPolicy::last_battle_state` exists.
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

        // And a faint is said as a faint, because that one is visible in the HP.
        let mut beaten = BattleReport::open(&start, 0).unwrap();
        beaten.decided(&start, &ember(), Vec::new());
        assert!(beaten.finish(Some(&hurt(state(), 20, 0))).contains("fainted"));
    }

    /// A blackout is the one ending whose numbers are gone by the time anything reads them.
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
        // Read out of the rendered string, never the source.
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

    #[test]
    fn a_hand_back_says_what_the_script_did_since_the_model_last_chose() {
        let start = state();
        let mut report = BattleReport::open(&start, 0).unwrap();

        // Turn 1 is handed back.
        assert_eq!(report.handed_back(&start), None, "the first hand-back has no history behind it");

        // The model chose, and then the script took turn 2 for itself.
        report.decided(&hurt(state(), 30, 18), &ember(), Vec::new());
        report.said("Enemy RATTATA fainted!");

        let account = report.handed_back(&hurt(state(), 30, 0)).expect("turn 2 is unaccounted for");
        assert!(account.contains("used Ember"), "what it chose: {account}");
        assert!(account.contains("18 → 0"), "and what that did: {account}");
        assert!(account.contains("Enemy RATTATA fainted!"), "and what the game said: {account}");

        // Not a second time.
        assert_eq!(report.handed_back(&start), None, "already shown, so nothing is repeated");
    }

    #[test]
    fn the_account_and_the_report_describe_a_turn_the_same_way() {
        let start = state();
        let mut report = BattleReport::open(&start, 0).unwrap();
        report.decided(&start, &ember(), Vec::new());
        let account = report.handed_back(&hurt(state(), 30, 7)).expect("one turn to account for");
        let line = account.lines().find(|line| line.starts_with("1. ")).expect("the turn's line");
        assert!(
            report.finish(Some(&start)).contains(line),
            "the finished report carries the same line: {line:?}",
        );
    }

    #[test]
    fn the_turns_the_model_answered_are_counted_separately() {
        let start = state();
        let mut report = BattleReport::open(&start, 0).unwrap();
        report.decided(&start, &ember(), Vec::new());
        let _ = report.handed_back(&start);
        report.decided(&start, &ember(), Vec::new());
        let rendered = report.finish(Some(&start));
        assert!(rendered.contains("3 turns, 1 of them answered by you"), "{rendered}");
    }

    /// By the end of a battle no turn is open to attach the exp and money to.
    #[test]
    fn what_the_game_says_after_the_last_turn_is_still_reported() {
        let start = state();
        let mut report = BattleReport::open(&start, 0).unwrap();
        report.decided(&start, &ember(), Vec::new());
        report.said("Enemy RATTATA fainted!");
        let _ = report.handed_back(&start);
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
