//! The completion run: a fresh save played through the whole game by a scripted model, through
//! `LlmPolicy`, the worker and the wire, with every story gate live and the ledger asserted.
//!
//! The brain answers from the rendered turn only. What it may be handed is the god-mode boundary:
//! battle strength and money (`Cheats::story`), and Master Balls and Rare Candies where a step
//! asks for them. Everything else is earned through the menu.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::pokemon::integration_tests::cheats::Cheats;
use crate::pokemon::integration_tests::completion::{checklist, Entry, Ledger, Way};
use crate::pokemon::integration_tests::godmode::Intent;
use crate::pokemon::integration_tests::llm_harness::{Brain, Call, LlmRun, Reply, TurnRequest};
use crate::pokemon::item::ItemId;

/// Trainers are fought by the script; a wild battle is the brain's, because only it knows what the
/// run is hunting.
const SCRIPT: &str = r#"
if battle.kind == "wild" { battle.ask(); }
if battle.me.fainted {
    for mon in battle.party {
        if !mon.fainted && mon.slot != battle.me.slot { battle.switch_to(mon); }
    }
}
if battle.best_move != () { battle.fight(battle.best_move); }
for mon in battle.party {
    if !mon.fainted && mon.slot != battle.me.slot {
        for mv in mon.moves {
            if mv.usable && mv.damage > 0 { battle.switch_to(mon); }
        }
    }
}
battle.ask();
"#;

/// How a gift or a catch ends up in the party, for the nickname prompt that follows. A trade asks
/// for no name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Came { Caught, Given }

/// One thing the run does, resolved against the menu the turn was sent.
#[derive(Debug, Clone)]
pub enum Step {
    /// Walk into each map in turn, each a neighbour of the last.
    Go(&'static [&'static str]),
    /// The first row whose description contains this, once.
    Take(&'static str),
    /// The row whose id ends in `:{0}`, once: a person, an item ball, a PC.
    Talk(&'static str),
    /// [`Step::Talk`] to someone who hands over a Pokémon.
    Gift(&'static str),
    /// Every person and item ball on this map not already chosen here, once each, except these.
    Clear(&'static [&'static str]),
    /// The row whose description contains this, until the menu stops offering it.
    Repeat(&'static str),
    /// A `use_field_move`, by its JSON arguments.
    Field(&'static str),
    /// Answer the mart that is open with these orders, then leave.
    Buy(&'static [(&'static str, u32)]),
    /// Choose the row whose id ends in `:{row}` until a wild `species` is caught with `ball`;
    /// `*` is any species not caught yet.
    Hunt { species: &'static str, row: &'static str, ball: &'static str, way: Way },
    /// Choose the `row` and fight every wild battle, except from the species in `flee`, until a
    /// turn says `until`, then record `way`.
    Train { row: &'static str, until: &'static str, way: Way, flee: &'static [&'static str] },
    /// Walk to `map` over the transitions the brain has been offered so far.
    GoTo(&'static str),
    /// Take every person, item, tree and passage inside `maps` not yet taken, walking to the
    /// nearest part of them with anything left, until nothing is; `patience` caps the turns.
    Explore { maps: &'static [&'static str], patience: usize },
    /// Read the bag and toss every kind the run has no further use for.
    Tidy,
    /// Teach the machine `item` to the first party member of `species`, found in the turn's party.
    Teach { item: &'static str, species: &'static str },
    /// Talk to the in-game trader `npc`; the agent hands over what the trade wants.
    Trade(&'static str),
    /// Vermilion Gym's bins, searched from what the game says after each until the door opens.
    TrashCans,
    /// A `pc_pokemon` operation at the PC on this map, done once the driver reports it done.
    AtPc(Pc),
    /// End a turn without moving.
    Wait,
    /// In the Day Care: board the first party member of this species, or with `None` pay and
    /// collect; done once the party has shrunk or grown.
    DayCare(Option<&'static str>),
    /// Use a bag item on what the row whose id ends in `:{row}` stands at, as the Poké Flute is
    /// used on a sleeping Snorlax.
    UseItemOn { item: &'static str, row: &'static str },
    /// Talk to the Game Corner's coin clerk until the turn shows at least this many coins.
    Coins(u16),
    /// Buy this Pokémon prize; the nickname prompt that follows is the proof.
    Prize(&'static str),
    /// Use the stone `item` on the first party member of `species`; done once it has evolved.
    Evolve { item: &'static str, species: &'static str },
    /// From here on, throw a Master Ball at every wild species not caught yet, or stop.
    Collect(bool),
}

/// One `pc_pokemon` operation, on the first Pokémon of a species.
#[derive(Debug, Clone, Copy)]
pub enum Pc {
    Deposit(&'static str),
    Withdraw(&'static str),
    Release(&'static str),
    ChangeBox(u8),
}

impl Pc {
    fn way(self) -> Way {
        match self {
            Pc::Deposit(_) => Way::PcDeposit,
            Pc::Withdraw(_) => Way::PcWithdraw,
            Pc::Release(_) => Way::PcRelease,
            Pc::ChangeBox(_) => Way::PcChangeBox,
        }
    }
}

/// What a tidy keeps besides what cannot be tossed: the balls, the stones and the candy.
const KEEP: &[ItemId] = &[
    // The catches, a level evolution, Pikachu's evolution for the Raichu trade, and a drink for
    // Saffron's guards.
    ItemId::MasterBall, ItemId::RareCandy, ItemId::ThunderStone, ItemId::FreshWater,
];

/// One pocket of a map, as last offered: what is in it to take, and its passages out as
/// (where it leads, the row id, the map).
#[derive(Debug, Clone, Default)]
struct Pocket {
    map: String,
    things: Vec<String>,
    exits: Vec<(String, String, String)>,
}

/// A passage by where it leads, which holds still where its row id does not: a two-tile door's
/// id is whichever tile is nearer, and "the way you came in" comes and goes.
fn passage(description: &str) -> String {
    description.split([';', '.']).next().unwrap_or(description).trim().to_string()
}

/// The bin a trash-can row searches, from "it is at (x, y)" in its description.
fn bin_at(description: &str) -> Option<(u8, u8)> {
    let at = description.split("it is at (").nth(1)?;
    let (x, rest) = at.split_once(", ")?;
    let y = rest.split(')').next()?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

/// The party slot of the first member of `species`, from the turn's `### Party` lines.
fn party_slot_of(situation: &str, species: &str) -> Option<u8> {
    situation.split("### Party").nth(1)?.lines().skip(1)
        .take_while(|line| !line.trim().is_empty())
        .find_map(|line| {
            let (slot, rest) = line.split_once(". ")?;
            let name = rest.split(" Lv").next()?;
            let kind = name.rsplit(" the ").next()?;
            same_species(kind, species).then(|| slot.trim().parse().ok()).flatten()
        })
}

/// A species name as a turn spells it and as the code does, reduced to compare: `Farfetch'd` is
/// `Farfetchd`.
fn plain(species: &str) -> String {
    species.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase()
}

fn same_species(a: &str, b: &str) -> bool {
    plain(a) == plain(b)
}

/// The map an exit row leads to, from its description. A tree is a passage within its own map,
/// and one that grows back whenever the map is reloaded, so it is never a thing done once.
fn destination(id: &str, description: &str) -> Option<String> {
    if id.ends_with(":CutTree") {
        return id.split(':').next().map(str::to_string);
    }
    if !matches!(id.rsplit(':').next(), Some("Warp" | "Connection" | "ConnectionWater")) {
        return None;
    }
    ["warp to ", "walk into ", "surf into "].iter()
        .find_map(|lead| description.split(lead).nth(1))
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_alphanumeric()).next())
        .map(str::to_string)
}

/// The brain: the step list, and what it has seen and done.
pub struct CompletionBrain {
    steps: Vec<Step>,
    at: usize,
    armed: bool,
    /// Per map, the rows [`Step::Clear`] and [`Step::Talk`] have already chosen.
    chosen: std::collections::HashMap<String, HashSet<String>>,
    /// Consecutive turns the current step has found no row for.
    unresolved: usize,
    reissued: usize,
    /// Orders still to place at the mart that is open.
    orders: VecDeque<(&'static str, u32)>,
    /// The species a ball was thrown at, until the catch is seen or the battle is over.
    thrown: Option<String>,
    /// Every species the run has had in its party or caught, [`plain`].
    caught: HashSet<String>,
    /// [`Step::Collect`] is on, and the box has room.
    collecting: bool,
    box_full: bool,
    /// How often the current step has run from each species: what blocks the way is fought.
    ran: std::collections::HashMap<String, usize>,
    /// The step the run counts belong to.
    running_at: usize,
    /// How the next nickname prompt's Pokémon came.
    came: Came,
    /// Every nickname prompt alternates between a name and the species name.
    named: usize,
    /// The party the last overworld turn showed was full.
    party_was_full: bool,
    /// Every passage offered so far: map, then destination, then the row that goes there.
    graph: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    /// How often each passage has been taken, for [`Step::Explore`]'s least-travelled choice.
    travelled: std::collections::HashMap<String, usize>,
    /// Every row id ever offered, and the turns since a new one was.
    offered: HashSet<String>,
    barren: usize,
    /// The pocket the last overworld turn stood in.
    here: String,
    /// The last person or thing walked to: its map, row, name, and the step to go back to if
    /// the next turn says the walk was given up.
    last_walk: Option<(String, String, String, usize)>,
    /// A map is several pockets when walls split it; each is known by the passages it offers.
    pockets: std::collections::HashMap<String, Pocket>,
    /// Which pocket each passage out of a pocket came out in.
    pocket_edges: std::collections::HashMap<(String, String), String>,
    /// The pocket and passage the last turn left by.
    left_by: Option<(String, String)>,
    /// Turns the current [`Step::Explore`] has taken.
    exploring: usize,
    /// A tidy under way: `None` until the bag has been read, then what is left to toss.
    tidying: Option<Option<VecDeque<String>>>,
    /// The bins: where the first switch was found, the bins searched since, and the last one.
    bins: (Option<(u8, u8)>, HashSet<(u8, u8)>, Option<(u8, u8)>),
    /// The report the PC driver will make for the operation sent, once it is sent.
    pc_sent: Option<String>,
    /// A machine was just sent to be taught, so a move to forget is asked for it.
    teaching: bool,
    /// The party slot a machine was taught to, and the move, to find on the next overworld turn.
    taught: Option<(u8, String)>,
    /// The last row chosen, and how many turns running it has been.
    repeated: (String, usize),
    /// A [`Step::UseItemOn`] has been sent at least once.
    used_on: bool,
    /// A prize Pokémon was bought and its nickname prompt not yet seen.
    prize_pending: bool,
    /// The party slot a stone was used on.
    evolving: Option<u8>,
    /// The party's size when a Day Care call was sent.
    day_care_sent: Option<usize>,
    /// Pickups that failed, per map and row.
    pickups_failed: std::collections::HashMap<String, u8>,
    pub ledger: Arc<Mutex<Ledger>>,
    pub stuck: Arc<Mutex<Option<String>>>,
    pub turns: Arc<Mutex<usize>>,
}

impl CompletionBrain {
    /// Turns a step may find nothing before the run is called stuck.
    const PATIENCE: usize = 25;
    const MAX_REISSUES: usize = 30;

    pub fn new(steps: Vec<Step>, ledger: Arc<Mutex<Ledger>>) -> Self {
        Self {
            steps, at: 0, armed: false, chosen: Default::default(), unresolved: 0, reissued: 0,
            orders: VecDeque::new(), thrown: None, caught: HashSet::new(), collecting: false, box_full: false,
            ran: Default::default(), running_at: 0, came: Came::Given, named: 0, party_was_full: false,
            graph: Default::default(), travelled: Default::default(), offered: HashSet::new(), barren: 0,
            last_walk: None, here: String::new(), pockets: Default::default(), pocket_edges: Default::default(), left_by: None,
            exploring: 0, tidying: None, bins: (None, HashSet::new(), None), pc_sent: None, teaching: false, taught: None, pickups_failed: Default::default(), day_care_sent: None, used_on: false, prize_pending: false, evolving: None, repeated: (String::new(), 0), ledger,
            stuck: Arc::new(Mutex::new(None)), turns: Arc::new(Mutex::new(0)),
        }
    }

    pub fn finished(&self) -> bool {
        self.at >= self.steps.len()
    }

    fn saw(&self, way: Way) {
        self.ledger.lock().expect("not poisoned").saw(Entry::Way(way));
    }

    fn stuck(&self, why: String) {
        let mut stuck = self.stuck.lock().expect("not poisoned");
        if stuck.is_none() {
            *stuck = Some(why);
        }
    }

    /// The wild battle turn: throw at the hunted species, and run from everything else.
    fn battle(&mut self, request: &TurnRequest) -> Reply {
        let ids = request.menu_ids();
        let situation = request.situation();
        let foe = situation.lines().find_map(|line| line.strip_prefix("Enemy: "))
            .and_then(|rest| rest.split_whitespace().next()).unwrap_or("").to_string();
        let choose = |id: &str| Reply::call("choose_battle_action", serde_json::json!({ "id": id, "summary": "as planned" }));
        if situation.contains("BOX is full") {
            self.box_full = true;
        }
        let new = !self.caught.contains(&plain(&foe));
        let (hunted, ball) = match self.steps.get(self.at) {
            Some(Step::Hunt { species, ball, .. }) => ((*species == "*" && new) || same_species(&foe, species), *ball),
            _ => (false, "MasterBall"),
        };
        // One ball a battle, unless it is a Safari Ball: a Master Ball that did not catch was
        // refused, as the tower's ghost refuses everything, and throwing again refuses again.
        let safari = ids.iter().any(|id| id == "ball");
        if (hunted || (self.collecting && new && !self.box_full)) && (safari || self.thrown.is_none()) {
            let throw = if safari { "ball".to_string() } else { format!("item:{ball}") };
            if ids.contains(&throw) {
                self.thrown = Some(foe);
                self.came = Came::Caught;
                return choose(&throw);
            }
        }
        let training = match self.steps.get(self.at) {
            Some(Step::Train { flee, .. }) => !flee.iter().any(|species| foe.eq_ignore_ascii_case(species)),
            _ => false,
        };
        // Three goes: a wild Pokémon that keeps interrupting the same step is standing in the way,
        // as the tower's Marowak stands on the stairs, and running from it again changes nothing.
        if !training && ids.iter().any(|id| id == "run") {
            let ran = self.ran.entry(plain(&foe)).or_default();
            *ran += 1;
            if *ran <= 3 {
                return choose("run");
            }
        }
        match ids.iter().find(|id| id.starts_with("fight:")) {
            Some(id) => choose(id),
            None => Reply::Calls(vec![Call::wait(10)]),
        }
    }

    fn nickname(&mut self) -> Reply {
        // Every catch asks for a name, which makes the prompt the proof of the catch.
        if let Some(species) = self.thrown.take() {
            self.caught.insert(plain(&species));
            if self.party_was_full {
                self.saw(Way::CaughtToTheBox);
            }
            if let Some(Step::Hunt { way, species: hunted, .. }) = self.steps.get(self.at).cloned()
                && (hunted == "*" || same_species(&species, hunted))
            {
                self.saw(way);
                self.at += 1;
            }
        }
        if std::mem::take(&mut self.prize_pending) {
            self.saw(Way::GameCornerPrize);
            self.came = Came::Given;
        }
        let way = match self.came {
            Came::Caught => Way::NicknameAfterACatch,
            Came::Given => Way::NicknameAfterAGift,
        };
        self.saw(way);
        self.named += 1;
        if self.named % 2 == 1 {
            self.saw(Way::NicknameGiven);
            // The naming screen has letters and a little punctuation, and no digits.
            let name = ["ALPHA", "BRAVO", "DELTA", "ECHO", "GOLF", "HOTEL", "KILO", "LIMA", "OSCAR"][self.named / 2 % 9];
            Reply::call("set_nickname", serde_json::json!({ "name": name, "summary": "named" }))
        } else {
            self.saw(Way::NicknameDeclined);
            Reply::call("set_nickname", serde_json::json!({ "summary": "the species name will do" }))
        }
    }

    fn mart(&mut self) -> Reply {
        if let Some(Step::Buy(orders)) = self.steps.get(self.at).cloned() {
            self.orders = orders.iter().copied().collect();
            self.at += 1;
        }
        match self.orders.pop_front() {
            Some((item, quantity)) => {
                let then: Vec<serde_json::Value> = self.orders.drain(..)
                    .map(|(item, quantity)| serde_json::json!({ "item": item, "quantity": quantity }))
                    .collect();
                Reply::call("buy_item", serde_json::json!({ "item": item, "quantity": quantity, "then": then, "summary": "stocking up" }))
            }
            None => Reply::call("buy_item", serde_json::json!({ "summary": "nothing needed here" })),
        }
    }

    /// The first hop toward `target` over the passages seen, as a row offered now.
    fn route(&self, request: &TurnRequest, target: &str) -> Option<String> {
        let here = request.location()?;
        let mut came: std::collections::HashMap<String, String> = Default::default();
        let mut queue = VecDeque::from([here.clone()]);
        came.insert(here.clone(), String::new());
        while let Some(map) = queue.pop_front() {
            if map == target {
                // Walk back to the hop taken from `here`.
                let mut at = map;
                while came.get(&at).is_some_and(|prev| *prev != here) {
                    at = came[&at].clone();
                }
                return self.graph.get(&here)?.get(&at).cloned()
                    .filter(|id| request.menu_ids().contains(id));
            }
            for next in self.graph.get(&map).into_iter().flat_map(|exits| exits.keys()) {
                if !came.contains_key(next) {
                    came.insert(next.clone(), map.clone());
                    queue.push_back(next.clone());
                }
            }
        }
        None
    }

    /// The pocket this turn stands in, recorded with what it offers and linked to the one before.
    fn observe_pocket(&mut self, request: &TurnRequest) -> Option<String> {
        let map = request.location()?;
        let rows = request.menu_rows();
        let mut exits: Vec<(String, String, String)> = rows.iter()
            .filter_map(|(id, what)| destination(id, what).map(|to| (passage(what), id.clone(), to)))
            .collect();
        exits.sort();
        let key = format!("{map}|{}", exits.iter().map(|(way, ..)| way.as_str()).collect::<Vec<_>>().join(" ~ "));
        let things = rows.iter().map(|(id, _)| id.clone())
            .filter(|id| id.matches(':').count() == 1 && !id.contains("Boulder"))
            .collect();
        // A walk a battle interrupted comes back to the same pocket, which is no passage.
        if let Some(from) = self.left_by.take()
            && from.0 != key
        {
            self.pocket_edges.insert(from, key.clone());
        }
        self.pockets.insert(key.clone(), Pocket { map, things, exits });
        self.here = key.clone();
        Some(key)
    }

    /// Whether `pocket` has anything [`Step::Explore`] has not taken: a thing, or a way on.
    fn unfinished(&self, pocket: &Pocket, key: &str, maps: &[&str]) -> bool {
        let chosen = self.chosen.get(&pocket.map);
        pocket.things.iter().any(|id| !chosen.is_some_and(|c| c.contains(id)))
            || pocket.exits.iter().any(|(way, _, to)| maps.contains(&to.as_str())
                && !self.pocket_edges.contains_key(&(key.to_string(), way.clone())))
    }

    /// What [`Step::Explore`] takes next from `here`: something in this pocket, a passage never
    /// taken, or the first passage toward the nearest pocket with either.
    fn explore(&self, here: &str, maps: &[&str]) -> Option<String> {
        let pocket = self.pockets.get(here)?;
        let chosen = self.chosen.get(&pocket.map);
        let untaken = |id: &String| !chosen.is_some_and(|c| c.contains(id));
        if let Some(id) = pocket.things.iter().find(|id| untaken(id)) {
            return Some(id.clone());
        }
        let inside = |to: &String| maps.contains(&to.as_str());
        if let Some((_, id, _)) = pocket.exits.iter()
            .find(|(way, _, to)| inside(to) && !self.pocket_edges.contains_key(&(here.to_string(), way.clone())))
        {
            return Some(id.clone());
        }
        // Breadth-first over the passages taken, to the nearest pocket with work left.
        let mut first: std::collections::HashMap<String, String> = Default::default();
        let mut queue = VecDeque::from([here.to_string()]);
        let mut seen = HashSet::from([here.to_string()]);
        while let Some(at) = queue.pop_front() {
            let Some(p) = self.pockets.get(&at) else { continue };
            if at != here && self.unfinished(p, &at, maps) {
                return first.get(&at).cloned();
            }
            for (way, id, _) in p.exits.iter().filter(|(.., to)| inside(to)) {
                let Some(next) = self.pocket_edges.get(&(at.clone(), way.clone())) else { continue };
                if seen.insert(next.clone()) {
                    let hop = if at == here { id.clone() } else { first[&at].clone() };
                    first.insert(next.clone(), hop);
                    queue.push_back(next.clone());
                }
            }
        }
        None
    }

    /// The first hop from `here` toward the nearest pocket with an exit `wanted` picks, over the
    /// passages taken so far: for a map that is several pockets, where the map-level route cannot
    /// tell which door of a map leads on.
    fn toward(&self, here: &str, wanted: impl Fn(&str, &str) -> bool) -> Option<String> {
        let mut first: std::collections::HashMap<String, String> = Default::default();
        let mut queue = VecDeque::from([here.to_string()]);
        let mut seen = HashSet::from([here.to_string()]);
        while let Some(at) = queue.pop_front() {
            let Some(pocket) = self.pockets.get(&at) else { continue };
            if let Some((_, id, _)) = pocket.exits.iter().find(|(way, _, to)| wanted(way, to)) {
                return Some(if at == here { id.clone() } else { first[&at].clone() });
            }
            for (way, id, _) in &pocket.exits {
                let Some(next) = self.pocket_edges.get(&(at.clone(), way.clone())) else { continue };
                if seen.insert(next.clone()) {
                    let hop = if at == here { id.clone() } else { first[&at].clone() };
                    first.insert(next.clone(), hop);
                    queue.push_back(next.clone());
                }
            }
        }
        None
    }

    /// Choose `id`, remembering it against the map, and the pocket it leaves if it is a passage.
    fn choose(&mut self, request: &TurnRequest, id: String) -> Reply {
        let map = request.location().unwrap_or_default();
        let rows = request.menu_rows();
        if let Some((_, what)) = rows.iter().find(|(row, what)| *row == id && destination(row, what).is_some()) {
            self.left_by = Some((self.here.clone(), passage(what)));
        }
        // A person or a thing: what the event says if the walk is given up on the way.
        let name = request.menu_rows().iter().find(|(row, _)| *row == id)
            .and_then(|(_, what)| ["talk to ", "pick up the ", "pick up ", "examine the ", "examine ", "read the "].iter()
                .find_map(|lead| what.strip_prefix(lead)).map(|rest| rest.split(" (").next().unwrap_or(rest).to_string()));
        self.last_walk = name.map(|name| (map.clone(), id.clone(), name, self.at));
        *self.travelled.entry(format!("{map}|{id}")).or_default() += 1;
        // A row that never gets anywhere: what the agent could not do is the finding. Pacing
        // and fishing are chosen over and over on purpose.
        let hunting = ["Grass", "Pace", "PaceOnWater", "Fish"].iter().any(|kind| id.ends_with(&format!(":{kind}")))
            || matches!(self.steps.get(self.at), Some(Step::Coins(_)));
        if hunting {
            self.repeated = (String::new(), 0);
        } else if self.repeated.0 == format!("{map}|{id}") {
            self.repeated.1 += 1;
            if self.repeated.1 >= Self::MAX_REISSUES {
                self.stuck(format!("step {}: chose {id} {} turns running on {map}:\n{}",
                    self.at + 1, self.repeated.1, request.situation()));
            }
        } else {
            self.repeated = (format!("{map}|{id}"), 1);
        }
        self.chosen.entry(map).or_default().insert(id.clone());
        Reply::call("choose_action", serde_json::json!({ "id": id, "resume_after_battle": true, "summary": "as planned" }))
    }

    /// One turn of a tidy: read the bag, then toss what it holds that nothing needs.
    fn tidy(&mut self, request: &TurnRequest) -> Option<Reply> {
        match self.tidying.as_mut()? {
            None => {
                // The answer to the read just sent, never an older one: a second tidy built from
                // the first tidy's bag tosses what is no longer there and frees nothing.
                let bag = request.messages.last().filter(|m| m.role == "tool")
                    .and_then(|m| serde_json::from_str::<serde_json::Value>(&m.text).ok())
                    .filter(|value| value.get("slots_used").is_some() && value.get("items").is_some());
                // On its own, so the turn carries on with the answer rather than ending.
                let Some(bag) = bag else {
                    return Some(Reply::Calls(vec![Call::new("read_bag", serde_json::json!({}))]));
                };
                let tossable = bag["items"].as_array().into_iter().flatten()
                    .filter_map(|item| item["item"].as_str())
                    .filter(|name| {
                        let id = (1..=255u8).filter_map(ItemId::from_repr).find(|id| format!("{id:?}") == *name);
                        id.is_some_and(|id| !id.is_key_item() && !id.is_hm() && !KEEP.contains(&id))
                    })
                    .map(str::to_string)
                    .collect();
                self.tidying = Some(Some(tossable));
                self.tidy(request)
            }
            Some(left) => match left.pop_front() {
                Some(item) => Some(Reply::call("use_field_move",
                    serde_json::json!({ "move": "toss_item", "item": item, "summary": "making room" }))),
                None => { self.tidying = None; None }
            },
        }
    }

    /// One turn of a [`Step::Pc`] not yet reported done: read the box if the operation needs a
    /// box slot, then send it.
    fn pc(&mut self, request: &TurnRequest, op: Pc) -> Reply {
        let text = request.situation();
        if let Some(report) = &self.pc_sent {
            self.stuck(format!("step {} ({op:?}): no report of {report} done:\n{text}", self.at + 1));
            return Reply::Calls(vec![Call::wait(10)]);
        }
        use crate::pokemon::postgame::pc_box::PcBoxOp;
        let op_sent = match op {
            Pc::Deposit(species) => party_slot_of(text, species).map(|slot| PcBoxOp::Deposit { slot }),
            Pc::ChangeBox(n) => Some(PcBoxOp::ChangeBox { n: n - 1 }),
            Pc::Withdraw(species) | Pc::Release(species) => {
                // On its own, so the turn carries on with the answer rather than ending.
                let Some(boxed) = request.messages.last().filter(|m| m.role == "tool")
                    .and_then(|m| serde_json::from_str::<serde_json::Value>(&m.text).ok())
                    .filter(|value| value.get("open_box").is_some())
                else {
                    return Reply::Calls(vec![Call::new("read_pc", serde_json::json!({}))]);
                };
                boxed["pokemon"].as_array().into_iter().flatten()
                    .find(|mon| mon["species"].as_str().is_some_and(|s| same_species(s, species)))
                    .and_then(|mon| mon["box_slot"].as_u64())
                    .map(|box_slot| box_slot as u8)
                    .map(|box_slot| match op {
                        Pc::Withdraw(_) => PcBoxOp::Withdraw { box_slot },
                        _ => PcBoxOp::Release { box_slot },
                    })
            }
        };
        let Some(op_sent) = op_sent else {
            self.stuck(format!("step {} ({op:?}): no such Pokémon to move", self.at + 1));
            return Reply::Calls(vec![Call::wait(10)]);
        };
        let arguments = match op_sent {
            PcBoxOp::Deposit { slot } => serde_json::json!({ "op": "deposit", "slot": slot }),
            PcBoxOp::Withdraw { box_slot } => serde_json::json!({ "op": "withdraw", "box_slot": box_slot }),
            PcBoxOp::Release { box_slot } => serde_json::json!({ "op": "release", "box_slot": box_slot }),
            PcBoxOp::ChangeBox { n } => serde_json::json!({ "op": "change_box", "box": n + 1 }),
        };
        let mut arguments = arguments;
        arguments["move"] = serde_json::json!("pc_pokemon");
        arguments["summary"] = serde_json::json!("sorting the boxes");
        self.pc_sent = Some(format!("PC box: {op_sent:?}"));
        Reply::call("use_field_move", arguments)
    }

    fn overworld(&mut self, request: &TurnRequest) -> Reply {
        if self.running_at != self.at {
            self.running_at = self.at;
            self.ran.clear();
        }
        self.teaching = false;
        self.thrown = None;
        let text = request.situation().to_string();
        // A teach the game refused, or a machine never found, leaves the move unlearned.
        if let Some((slot, name)) = self.taught.take() {
            let knows = text.split("### Party").nth(1).and_then(|party| party.lines()
                .find(|line| line.starts_with(&format!("{slot}. "))))
                .is_some_and(|line| line.rsplit(" — ").next().unwrap_or("").replace(' ', "").contains(&name));
            if !knows {
                self.stuck(format!("step {}: party slot {slot} did not learn {name}:\n{text}", self.at));
                return Reply::Calls(vec![Call::wait(10)]);
            }
        }
        for line in text.split("### Party").nth(1).unwrap_or("").lines().skip(1).take_while(|line| !line.trim().is_empty()) {
            if let Some(kind) = line.split_once(". ").and_then(|(_, rest)| rest.split(" Lv").next()).and_then(|name| name.rsplit(" the ").next()) {
                self.caught.insert(plain(kind));
            }
        }
        // A pickup that did not happen: take it back, and make room if that was why. Twice at
        // most otherwise, since a starter's ball in Oak's lab is never picked up at all. The
        // verdict can land a turn late, so the row is found by the item's name.
        let map = request.location().unwrap_or_default();
        let full = text.contains("No more room");
        // A gift the bag had no room for: talk again once there is some.
        if let Some((map, id, _, _)) = self.last_walk.as_ref()
            && (text.contains("have any room for this") || text.contains("make room for this"))
        {
            if let Some(chosen) = self.chosen.get_mut(map) {
                chosen.remove(id);
            }
            self.tidying.get_or_insert(None);
        }
        let failed: Vec<String> = text.split("nothing was picked up: the ").skip(1)
            .filter_map(|rest| rest.split(" is still lying there").next())
            .filter_map(|name| request.menu_rows().into_iter()
                .find(|(_, what)| what.starts_with(&format!("pick up the {name}")) || what.starts_with(&format!("pick up {name}")))
                .map(|(id, _)| id))
            .collect();
        for id in failed {
            let tries = self.pickups_failed.entry(format!("{map}|{id}")).or_default();
            *tries += 1;
            if full || *tries <= 2 {
                if let Some(chosen) = self.chosen.get_mut(&map) {
                    chosen.remove(&id);
                }
            }
            if full {
                self.tidying.get_or_insert(None);
            }
        }
        if matches!(self.steps.get(self.at), Some(Step::Tidy)) {
            self.tidying.get_or_insert(None);
            self.at += 1;
        }
        if let Some(reply) = self.tidy(request) {
            return reply;
        }
        // A walk given up on the way did not happen: take it back, and the step with it.
        if let Some((map, id, name, step)) = self.last_walk.take()
            && text.contains(&format!("gave up on {name}"))
        {
            if let Some(chosen) = self.chosen.get_mut(&map) {
                chosen.remove(&id);
            }
            self.at = self.at.min(step);
        }
        // Six rows under the party heading: the next catch goes to the box.
        self.party_was_full = text.split("### Party").nth(1)
            .map(|party| party.lines().skip(1).take_while(|line| !line.trim().is_empty()).count() >= 6)
            .unwrap_or(false);
        let here = self.observe_pocket(request).unwrap_or_default();
        // What this turn offers: the passages, and whether anything is new.
        if let Some(map) = request.location() {
            let mut fresh = false;
            for (id, what) in request.menu_rows() {
                fresh |= self.offered.insert(id.clone());
                if let Some(to) = destination(&id, &what) {
                    self.graph.entry(map.clone()).or_default().insert(to, id);
                }
            }
            self.barren = if fresh { 0 } else { self.barren + 1 };
        }
        loop {
            let Some(step) = self.steps.get(self.at).cloned() else { return Reply::Calls(vec![Call::wait(50)]) };
            let rows = request.menu_rows();
            let map = request.location().unwrap_or_default();
            let chosen = self.chosen.get(&map).cloned().unwrap_or_default();
            let resolved: Option<String> = match &step {
                Step::Go(path) => {
                    let Some(next) = path.iter().position(|m| request.location().as_deref() == Some(*m))
                        .map(|here| here + 1).or(Some(0))
                        .and_then(|i| path.get(i).copied()) else { self.at += 1; continue };
                    Intent::Enter(next).resolve(request)
                }
                Step::Take(fragment) => Intent::Says(fragment).resolve(request)
                    .or_else(|| self.toward(&here, |way, _| way.contains(fragment))),
                Step::Talk(kind) | Step::Gift(kind) => Intent::Row(kind).resolve(request),
                Step::Repeat(fragment) => {
                    if Intent::Repeat(fragment).satisfied_by(request) { self.at += 1; self.reissued = 0; continue }
                    Intent::Repeat(fragment).resolve(request)
                }
                Step::Clear(skip) => {
                    let next = rows.iter().map(|(id, _)| id)
                        .filter(|id| id.matches(':').count() == 1)
                        .filter(|id| !id.contains("Boulder") && !skip.iter().any(|s| id.ends_with(&format!(":{s}"))))
                        .find(|id| !chosen.contains(*id)).cloned();
                    if next.is_none() { self.at += 1; continue }
                    next
                }
                Step::Hunt { row, .. } | Step::Train { row, .. } =>
                    rows.iter().map(|(id, _)| id).find(|id| id.ends_with(&format!(":{row}"))).cloned(),
                Step::Field(arguments) => {
                    self.at += 1;
                    let mut arguments: serde_json::Value = serde_json::from_str(arguments).expect("a step's JSON");
                    arguments["summary"] = serde_json::json!("as planned");
                    return Reply::call("use_field_move", arguments);
                }
                Step::Buy(_) => {
                    // The mart turn answers it; an overworld turn in between means the mart never opened.
                    self.unresolved += 1;
                    if self.unresolved > Self::PATIENCE {
                        self.stuck(format!("step {} ({step:?}): no mart opened on {map}", self.at + 1));
                    }
                    return Reply::Calls(vec![Call::wait(10)]);
                }
                Step::GoTo(target) => {
                    if request.location().as_deref() == Some(*target) { self.at += 1; continue }
                    Intent::Enter(target).resolve(request)
                        .or_else(|| self.toward(&here, |_, to| to == *target))
                        .or_else(|| self.route(request, target))
                        // Walled in with nothing known beyond: a way on within this map, such as a
                        // tree, not taken from here before.
                        .or_else(|| self.pockets.get(&here).and_then(|pocket| pocket.exits.iter()
                            .find(|(way, _, to)| *to == pocket.map && !self.pocket_edges.contains_key(&(here.clone(), way.clone())))
                            .map(|(_, id, _)| id.clone())))
                }
                Step::Explore { maps, patience } => {
                    self.exploring += 1;
                    match self.explore(&here, maps) {
                        Some(id) if self.exploring < *patience => Some(id),
                        _ => {
                            self.exploring = 0; self.at += 1; continue
                        }
                    }
                }
                Step::Wait => { self.at += 1; return Reply::Calls(vec![Call::wait(1)]) }
                Step::Collect(on) => { self.collecting = *on; self.at += 1; continue }
                Step::Coins(target) => {
                    let coins: u16 = text.split("Coins: ").nth(1)
                        .and_then(|rest| rest.split_whitespace().next())
                        .and_then(|n| n.parse().ok()).unwrap_or(0);
                    if coins >= *target { self.at += 1; continue }
                    rows.iter().map(|(id, _)| id).find(|id| id.ends_with(":Clerk1")).cloned()
                }
                Step::UseItemOn { item, row } => {
                    // Done when what the item was used on is no longer a row: a trainer's battle
                    // can interrupt the walk to it, and then nothing happened at all.
                    let at = rows.iter().find(|(id, _)| id.ends_with(&format!(":{row}")))
                        .and_then(|(_, what)| bin_at(what));
                    let Some((x, y)) = at else {
                        if self.used_on {
                            self.used_on = false;
                            self.at += 1;
                            continue;
                        }
                        self.unresolved += 1;
                        if self.unresolved >= Self::PATIENCE {
                            self.stuck(format!("step {} ({step:?}) had no {row} on {map}:\n{text}", self.at + 1));
                        }
                        return Reply::Calls(vec![Call::wait(20)]);
                    };
                    self.used_on = true;
                    self.unresolved += 1;
                    if self.unresolved >= Self::PATIENCE {
                        self.stuck(format!("step {} ({step:?}): the {row} on {map} is still there", self.at + 1));
                    }
                    return Reply::call("use_field_move", serde_json::json!({
                        "move": "use_item", "item": item, "target": { "x": x, "y": y }, "summary": "waking it" }));
                }
                Step::Prize(name) => {
                    self.at += 1;
                    self.prize_pending = true;
                    return Reply::call("use_field_move", serde_json::json!({ "move": "prize", "item": name, "summary": "a prize" }));
                }
                Step::Evolve { item, species } => {
                    if let Some(slot) = self.evolving {
                        let still = text.split("### Party").nth(1).and_then(|party| party.lines()
                            .find(|line| line.starts_with(&format!("{slot}. "))))
                            .is_some_and(|line| line.split(" Lv").next().and_then(|name| name.rsplit(" the ").next())
                                .is_some_and(|kind| same_species(kind, species)));
                        if still {
                            self.stuck(format!("step {}: the {species} in slot {slot} did not evolve", self.at + 1));
                            return Reply::Calls(vec![Call::wait(10)]);
                        }
                        self.evolving = None;
                        self.saw(Way::EvolvedByStone);
                        self.at += 1;
                        continue;
                    }
                    let Some(slot) = party_slot_of(&text, species) else {
                        self.stuck(format!("step {}: no {species} in the party to evolve", self.at + 1));
                        return Reply::Calls(vec![Call::wait(10)]);
                    };
                    self.evolving = Some(slot);
                    return Reply::call("use_field_move", serde_json::json!({
                        "move": "evolve", "item": item, "slot": slot, "summary": "evolving" }));
                }
                Step::DayCare(board) => {
                    let party = text.split("### Party").nth(1).map_or(0, |party| party.lines().skip(1)
                        .take_while(|line| !line.trim().is_empty()).count());
                    if let Some(before) = self.day_care_sent {
                        let done = match board { Some(_) => party < before, None => party > before };
                        if !done {
                            self.stuck(format!("step {} ({step:?}): the party is still {party}", self.at + 1));
                            return Reply::Calls(vec![Call::wait(10)]);
                        }
                        self.day_care_sent = None;
                        if board.is_none() {
                            self.saw(Way::DayCareWithdrawn);
                        }
                        self.at += 1;
                        continue;
                    }
                    let arguments = match board {
                        Some(species) => match party_slot_of(&text, species) {
                            Some(slot) => serde_json::json!({ "move": "day_care", "op": "deposit", "slot": slot, "summary": "boarding" }),
                            None => {
                                self.stuck(format!("step {}: no {species} in the party to board", self.at + 1));
                                return Reply::Calls(vec![Call::wait(10)]);
                            }
                        },
                        None => serde_json::json!({ "move": "day_care", "op": "withdraw", "summary": "collecting" }),
                    };
                    self.day_care_sent = Some(party);
                    return Reply::call("use_field_move", arguments);
                }
                Step::Tidy => { self.at += 1; continue }
                Step::Teach { item, species } => {
                    let Some(slot) = party_slot_of(&text, species) else {
                        self.stuck(format!("step {}: no {species} in the party to teach {item} to", self.at + 1));
                        return Reply::Calls(vec![Call::wait(10)]);
                    };
                    self.at += 1;
                    self.teaching = true;
                    // `Hm02Fly` teaches Fly, `Tm13IceBeam` Ice Beam.
                    self.taught = Some((slot, item.get(4..).unwrap_or(item).to_string()));
                    return Reply::call("use_field_move", serde_json::json!({
                        "move": "teach", "item": item, "slot": slot, "summary": "teaching" }));
                }
                Step::Trade(npc) => Intent::Row(npc).resolve(request),
                Step::AtPc(op) => {
                    if self.pc_sent.as_ref().is_some_and(|report| text.contains(&format!("{report} done"))) {
                        self.pc_sent = None;
                        if matches!(op, Pc::ChangeBox(_) | Pc::Release(_)) {
                            self.box_full = false;
                        }
                        self.saw(op.way());
                        self.at += 1;
                        continue;
                    }
                    return self.pc(request, *op);
                }
                Step::TrashCans => {
                    if text.contains("motorized door") { self.at += 1; continue }
                    let (first, tried, last) = &mut self.bins;
                    if text.contains("1st electric lock opened") {
                        *first = *last;
                        tried.clear();
                    } else if text.contains("locks were reset") {
                        *first = None;
                        tried.clear();
                    }
                    let bins: Vec<(String, (u8, u8))> = rows.iter()
                        .filter(|(id, _)| id.contains(":TrashCan"))
                        .filter_map(|(id, what)| bin_at(what).map(|at| (id.clone(), at)))
                        .collect();
                    // The second switch is always beside the first, one bin away on the grid.
                    let next = bins.iter()
                        .filter(|(_, at)| !tried.contains(at) && Some(*at) != *first)
                        .find(|(_, at)| first.is_none_or(|f| f.0.abs_diff(at.0) + f.1.abs_diff(at.1) == 2))
                        .cloned();
                    match next {
                        Some((id, at)) => { tried.insert(at); *last = Some(at); Some(id) }
                        // Every neighbour searched: the switches moved, so start again.
                        None => { *first = None; tried.clear(); None }
                    }
                }
            };
            return match resolved {
                Some(id) => {
                    self.unresolved = 0;
                    match &step {
                        // Done by the location, checked at the top of a later turn.
                        Step::Go(_) => {}
                        Step::Repeat(_) => {
                            self.reissued += 1;
                            if self.reissued > Self::MAX_REISSUES {
                                self.stuck(format!("step {} ({step:?}) re-issued {} times on {map}", self.at + 1, self.reissued));
                            }
                        }
                        Step::Hunt { .. } | Step::Train { .. } | Step::Clear(_) | Step::GoTo(_) | Step::Explore { .. }
                        | Step::TrashCans | Step::Coins(_) => {}
                        Step::Trade(_) => self.at += 1,
                        // A hop toward the row is not the row.
                        Step::Take(fragment) => {
                            if rows.iter().any(|(row, what)| *row == id && what.contains(fragment)) {
                                self.at += 1;
                            }
                        }
                        Step::Gift(_) => { self.came = Came::Given; self.at += 1 }
                        _ => self.at += 1,
                    }
                    if matches!(step, Step::Hunt { .. }) { self.came = Came::Caught }
                    self.choose(request, id)
                }
                None => {
                    // `Go` is done once the last map is the location.
                    if let Step::Go(path) = &step
                        && request.location().as_deref() == path.last().copied()
                    {
                        self.at += 1;
                        continue;
                    }
                    self.unresolved += 1;
                    if self.unresolved >= Self::PATIENCE {
                        self.stuck(format!("step {} of {} ({step:?}) had no row on {map} for {} turns:\n{}",
                            self.at + 1, self.steps.len(), Self::PATIENCE, request.situation()));
                    }
                    Reply::Calls(vec![Call::wait(20)])
                }
            };
        }
    }
}

impl Brain for CompletionBrain {
    fn respond(&mut self, request: &TurnRequest) -> Reply {
        *self.turns.lock().expect("not poisoned") += 1;
        if request.is_summary() {
            return Reply::Content("Playing the whole game, a step list at a time.".into());
        }
        if request.is_stuck() {
            return Reply::call("press_buttons", serde_json::json!({ "buttons": ["a"], "why": "no decision point" }));
        }
        // Whatever kind of turn it arrives on: a walk resumed after a battle asks nothing between.
        if let Some(Step::Train { until, way, .. }) = self.steps.get(self.at).cloned()
            && request.situation().contains(until)
        {
            self.saw(way);
            self.at += 1;
        }
        if request.is_battle() {
            return self.battle(request);
        }
        if request.has_tool("set_nickname") {
            return self.nickname();
        }
        if request.has_tool("buy_item") {
            return self.mart();
        }
        if request.has_tool("forget_move") {
            // A machine taught on purpose replaces the first move that is not an HM's.
            let hm = ["cut", "fly", "surf", "strength", "flash"];
            let slot = self.teaching.then(|| request.menu_rows().into_iter()
                .find(|(_, what)| !hm.contains(&what.split_whitespace().next().unwrap_or("").to_ascii_lowercase().as_str()))
                .map(|(id, _)| id)).flatten();
            return match slot {
                Some(slot) => Reply::call("forget_move", serde_json::json!({ "slot": slot.parse::<u8>().unwrap_or(0), "summary": "making room" })),
                None => Reply::call("forget_move", serde_json::json!({ "summary": "keeping what it knows" })),
            };
        }
        if !request.has_tool("choose_action") {
            return Reply::Calls(vec![Call::wait(1)]);
        }
        let mut reply = self.overworld(request);
        if !self.armed {
            self.armed = true;
            let arm = Call::new("set_battle_script", serde_json::json!({ "script": SCRIPT, "purpose": "fight trainers, hand back wild battles" }));
            if let Reply::Calls(calls) = &mut reply {
                calls.insert(0, arm);
            }
        }
        reply
    }
}

/// What one phase leaves behind.
pub struct Played {
    pub run: LlmRun,
    pub ledger: Arc<Mutex<Ledger>>,
}

/// Play `steps` from `fixture`, feeding the ledger every tick, and fail with the stuck report.
pub fn play(fixture: &'static [u8], name: &'static str, steps: Vec<Step>, game_minutes: u64, wall: Duration) -> Played {
    let ledger = Arc::new(Mutex::new(Ledger::default()));
    let brain = CompletionBrain::new(steps, Arc::clone(&ledger));
    let (stuck, turns) = (Arc::clone(&brain.stuck), Arc::clone(&brain.turns));
    let total = brain.steps.len();
    let done = Arc::new(Mutex::new(false));
    let finished = Arc::clone(&done);
    let brain = FinishFlag { brain, finished };
    let mut run = LlmRun::builder(fixture)
        .named(name)
        .game_time(Duration::from_mins(game_minutes))
        .options(crate::pokemon::options::SERVED_OPTIONS)
        .with_coverage()
        .start(Box::new(brain));
    run.with_cheats(Cheats::story(999_999));
    {
        let list = checklist(run.fixture().gb.core().mmu());
        *ledger.lock().expect("not poisoned") = Ledger::new(&list);
    }

    let started = std::time::Instant::now();
    run.tick_until(wall, |run| {
        if let Ok(state) = run.fixture().try_game_state() {
            ledger.lock().expect("not poisoned").observe(&state, run.fixture().gb.core().mmu());
            // Master Balls for every catch outside the Safari Zone, as the legendary legs have.
            let balls = state.bag.iter().find(|item| item.id == ItemId::MasterBall).map_or(0, |item| item.quantity);
            // Only in the overworld: topped up mid-catch, the ball driver never sees one spent.
            if balls < 5 && state.mode == crate::pokemon::encoding::GameMode::Overworld
                && state.bag.iter().count() < crate::pokemon::bag::Bag::MAX_ITEMS
            {
                run.fixture().api().debug_give_item(ItemId::MasterBall, 10 - balls).ok();
            }
        }
        *done.lock().expect("not poisoned") || stuck.lock().expect("not poisoned").is_some()
    });
    let turns = *turns.lock().expect("not poisoned");
    println!("[completion:{name}] {total} steps, {turns} turns, {:?} of game time in {:?}",
             run.fixture().total_cycles.to_duration(), started.elapsed());
    if let Some(why) = stuck.lock().expect("not poisoned").clone() {
        // Where it stuck, to load and look at.
        let at = std::env::temp_dir().join(format!("{name}-stuck.bin"));
        run.fixture().gb.save_state_to_file(at.to_str().expect("a UTF-8 path")).ok();
        panic!("[completion:{name}] stuck (saved to {}): {why}", at.display());
    }
    assert!(*done.lock().expect("not poisoned"), "[completion:{name}] ran out of wall clock");
    let log = run.coverage().expect("coverage was asked for");
    let defects: Vec<String> = log.entries()
        .filter(|entry| matches!(entry.verdict, crate::pokemon::integration_tests::coverage::Verdict::Defect { .. }))
        .map(|entry| entry.id.clone())
        .collect();
    assert!(defects.is_empty(), "[completion:{name}] the agent could not carry out: {defects:?}");
    Played { run, ledger }
}

/// The brain, with a flag the driver reads to know every step has been taken.
struct FinishFlag {
    brain: CompletionBrain,
    finished: Arc<Mutex<bool>>,
}

impl Brain for FinishFlag {
    fn respond(&mut self, request: &TurnRequest) -> Reply {
        let before = self.brain.at;
        let reply = self.brain.respond(request);
        for step in before..self.brain.at.min(self.brain.steps.len()) {
            println!("[completion] step {} done on {}: {:?}", step + 1,
                     request.location().unwrap_or_default(), self.brain.steps[step]);
        }
        if self.brain.finished() {
            *self.finished.lock().expect("not poisoned") = true;
        }
        reply
    }
}

/// Every entry on `maps`, and every one of `also`, the ledger has not ticked off: a phase's own
/// assertion.
pub fn missing_on(played: &mut Played, maps: &[crate::pokemon::map::Map], also: &[Entry]) -> Vec<Entry> {
    let state = played.run.fixture().game_state();
    let list = checklist(played.run.fixture().gb.core().mmu());
    let ledger = played.ledger.lock().expect("not poisoned");
    let mmu = played.run.fixture().gb.core().mmu();
    ledger.missing(&list, mmu, &state).into_iter()
        .filter(|entry| also.contains(entry) || match entry {
            Entry::Map(map) | Entry::Trainer { map, .. } | Entry::ItemBall { map, .. } => maps.contains(map),
            _ => false,
        })
        .collect()
}

/// Save where a phase ended, for the next phase to start from, under `GB_REGEN_FIXTURES=1`.
pub fn cut(played: &mut Played, name: &str) {
    if crate::pokemon::integration_tests::fixture::regenerating_fixtures() {
        played.run.fixture().save_state_named(&format!("src/pokemon/data/{name}.bin")).expect("the phase's end state saves");
    }
}

/// Pallet Town to the Boulder Badge: the starter, the parcel, the Pokédex and Viridian Forest.
pub fn to_the_boulder_badge() -> Vec<Step> {
    use Step::*;
    vec![
        Go(&["RedsHouse1F"]), Clear(&[]),
        Go(&["PalletTown"]), Clear(&[]),
        // Oak stops the walk north and takes the player to his lab.
        Take("Route1"),
        Gift("SquirtlePokeBall"),
        Clear(&[]),
        Go(&["PalletTown", "Route1"]), Clear(&[]),
        Go(&["ViridianCity"]), Clear(&[]),
        // The clerk hands over Oak's Parcel as the door opens.
        Go(&["ViridianMart"]), Clear(&[]),
        Go(&["ViridianCity", "Route1", "PalletTown", "OaksLab"]), Talk("Oak1"),
        Go(&["PalletTown", "BluesHouse"]), Clear(&[]),
        Go(&["PalletTown", "Route1", "ViridianCity", "ViridianPokecenter"]), Clear(&[]),
        Go(&["ViridianCity", "ViridianSchoolHouse"]), Clear(&[]),
        Go(&["ViridianCity", "ViridianNicknameHouse"]), Clear(&[]),
        Go(&["ViridianCity", "Route22"]), Clear(&[]),
        Go(&["ViridianCity", "Route2"]), Clear(&[]),
        Go(&["ViridianForestSouthGate"]), Clear(&[]),
        Go(&["ViridianForest"]),
        Hunt { species: "Weedle", row: "Grass", ball: "MasterBall", way: Way::WildInGrass },
        // The catch is the last of three in the party: it leads, and fights until it evolves at 7.
        Field(r#"{"move":"reorder_party","slot":2}"#),
        // A Kakuna or Metapod only hardens, and the fight never ends.
        Train { row: "Grass", until: "evolved into", way: Way::EvolvedInBattle, flee: &["Kakuna", "Metapod"] },
        Field(r#"{"move":"reorder_party","slot":1}"#),
        Clear(&[]),
        Go(&["ViridianForestNorthGate"]), Clear(&[]),
        Go(&["Route2", "PewterCity"]), Clear(&[]),
        Go(&["PewterPokecenter"]), Clear(&[]),
        Go(&["PewterCity", "PewterMart"]), Clear(&[]),
        Go(&["PewterCity", "PewterNidoranHouse"]), Clear(&[]),
        Go(&["PewterCity", "PewterSpeechHouse"]), Clear(&[]),
        Go(&["PewterCity", "Museum1F"]), Clear(&[]),
        Go(&["Museum2F"]), Clear(&[]),
        Go(&["Museum1F", "PewterCity", "PewterGym"]), Clear(&[]),
        Go(&["PewterCity"]),
    ]
}

/// The first phase, from the fresh save.
#[test]
#[ignore = "a phase of the completion run; run with --ignored"]
fn completion_phase_boulder_badge() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/start-of-game-state.bin"), "completion-boulder",
                          to_the_boulder_badge(), 240, Duration::from_secs(1800));
    let missing = missing_on(&mut played, &[
        Map::RedsHouse1F, Map::PalletTown, Map::BluesHouse, Map::OaksLab, Map::Route1,
        Map::ViridianCity, Map::ViridianMart, Map::ViridianPokecenter, Map::ViridianSchoolHouse,
        Map::ViridianNicknameHouse, Map::Route22, Map::ViridianForestSouthGate, Map::ViridianForest,
        Map::ViridianForestNorthGate, Map::PewterCity, Map::PewterGym, Map::PewterMart,
        Map::PewterPokecenter, Map::Museum1F, Map::Museum2F, Map::PewterNidoranHouse,
        Map::PewterSpeechHouse,
    ], &[Entry::Badge(0), Entry::Way(Way::GiftStarter), Entry::Way(Way::WildInGrass),
         Entry::Way(Way::EvolvedInBattle), Entry::Way(Way::NicknameAfterAGift)]);
    cut(&mut played, "completion-boulder");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

/// Walk into `building` from wherever the run is, clear it, and walk back out to `town`.
fn visit(building: &'static str, town: &'static str) -> [Step; 3] {
    [Step::GoTo(building), Step::Clear(&[]), Step::GoTo(town)]
}

/// Pewter to Bill: Route 3, Mt Moon, Cerulean and the Cascade Badge, Nugget Bridge and Route 25.
pub fn to_bill() -> Vec<Step> {
    use Step::*;
    let mut steps = vec![
        GoTo("Route3"), Clear(&[]),
        GoTo("Route4"), Clear(&[]),
        // The Magikarp salesman, who counts as a way of obtaining a Pokémon.
        GoTo("MtMoonPokecenter"), Clear(&[]),
        GoTo("Route4"), GoTo("MtMoon1F"),
        Explore { maps: &["MtMoon1F", "MtMoonB1F", "MtMoonB2F"], patience: 600 },
        // Route 4 is two halves, and only B2F's east ladder comes out on the far one.
        GoTo("MtMoonB2F"), Take("MtMoonB1F, arriving at (23, 3)"), Take("Route4"), Tidy, Clear(&[]),
        GoTo("CeruleanCity"), Clear(&[]),
    ];
    for building in ["CeruleanPokecenter", "CeruleanMart", "CeruleanBadgeHouse", "CeruleanTradeHouse",
                     "BikeShop", "CeruleanGym"] {
        steps.extend(visit(building, "CeruleanCity"));
    }
    steps.extend([
        GoTo("Route24"), Clear(&[]),
        // For the Route 2 trade house, which wants an Abra.
        Hunt { species: "Abra", row: "Grass", ball: "MasterBall", way: Way::WildInGrass },
        GoTo("Route25"), Clear(&[]),
        // Bill hands over nothing to a full bag.
        Tidy,
        GoTo("BillsHouse"), Talk("BillPokemon"), Talk("CellSeparator1"), Talk("Bill1"), Clear(&[]),
        // The officer at the robbed house steps aside once Bill is himself again.
        GoTo("CeruleanCity"), GoTo("CeruleanTrashedHouse"), Clear(&[]),
        // Its back door comes out on Cerulean's other terrace, where the thief is.
        Take("CeruleanCity, arriving at (28, 10)"), Clear(&[]),
    ]);
    steps
}

#[test]
#[ignore = "a phase of the completion run; run with --ignored"]
fn completion_phase_bill() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-boulder.bin"), "completion-bill",
                          to_bill(), 240, Duration::from_secs(1800));
    let missing = missing_on(&mut played, &[
        Map::Route3, Map::MtMoonPokecenter, Map::MtMoon1F, Map::MtMoonB1F, Map::MtMoonB2F,
        Map::CeruleanPokecenter, Map::CeruleanMart, Map::CeruleanBadgeHouse, Map::CeruleanTradeHouse,
        Map::BikeShop, Map::CeruleanGym, Map::Route24, Map::Route25, Map::BillsHouse,
        Map::CeruleanTrashedHouse,
    ], &[Entry::Badge(1), Entry::Way(Way::BoughtMagikarp), Entry::KeyItem(vec![ItemId::SSTicket as u8])]);
    cut(&mut played, "completion-bill");
    // Behind a tree on Route 25, for the phase after Cut.
    let later = [Entry::ItemBall { map: Map::Route25, object: 10, item: ItemId::Tm19SeismicToss as u8 }];
    let missing: Vec<Entry> = missing.into_iter().filter(|entry| !later.contains(entry)).collect();
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

/// The Pokémon Tower-free half of the south: Route 5 and 6, Vermilion and the S.S. Anne, Cut, and
/// Lt. Surge.
pub fn to_the_thunder_badge() -> Vec<Step> {
    use Step::*;
    let mut steps = vec![
        // Route 5 is reached only from the terrace behind the robbed house.
        GoTo("CeruleanTrashedHouse"), Take("CeruleanCity, arriving at (28, 10)"),
        GoTo("Route5"), Clear(&[]),
        GoTo("UndergroundPathRoute5"), Clear(&[]),
        GoTo("UndergroundPathNorthSouth"), Clear(&[]),
        GoTo("UndergroundPathRoute6"), Clear(&[]),
        GoTo("Route6"), Clear(&[]),
        GoTo("VermilionCity"), Clear(&[]),
        Tidy,
    ];
    for building in ["VermilionPokecenter", "VermilionMart", "PokemonFanClub", "VermilionOldRodHouse",
                     "VermilionPidgeyHouse"] {
        steps.extend(visit(building, "VermilionCity"));
    }
    steps.extend([
        // The Old Rod's one catch is a Magikarp, at any water's edge.
        Hunt { species: "Magikarp", row: "Fish", ball: "MasterBall", way: Way::OldRod },
        GoTo("Route11"), Clear(&[]),
        // For the Vermilion trade, which gives a Farfetch'd that can learn both Cut and Fly.
        Hunt { species: "Spearow", row: "Grass", ball: "MasterBall", way: Way::WildInGrass },
        // The party is full, so the catch went to the box.
        GoTo("VermilionPokecenter"),
        AtPc(Pc::Deposit("Kakuna")), AtPc(Pc::Withdraw("Spearow")), AtPc(Pc::Release("Kakuna")),
        GoTo("VermilionCity"), GoTo("VermilionTradeHouse"), Trade("LittleGirl"), Clear(&[]),
        GoTo("VermilionCity"), Tidy,
        GoTo("VermilionDock"), Clear(&[]),
        GoTo("SSAnne1F"),
        Explore { maps: &["SSAnne1F", "SSAnne2F", "SSAnne3F", "SSAnneB1F", "SSAnneBow", "SSAnneKitchen",
                          "SSAnneCaptainsRoom", "SSAnne1FRooms", "SSAnne2FRooms", "SSAnneB1FRooms"], patience: 800 },
        GoTo("VermilionDock"), GoTo("VermilionCity"),
        Teach { item: "Hm01Cut", species: "Farfetchd" },
        Take("cut down the tree"),
        GoTo("VermilionGym"), Clear(&[]), TrashCans, Clear(&[]),
        GoTo("VermilionCity"),
    ]);
    steps
}

#[test]
#[ignore = "a phase of the completion run; run with --ignored"]
fn completion_phase_thunder_badge() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-bill.bin"), "completion-thunder",
                          to_the_thunder_badge(), 300, Duration::from_secs(2400));
    let missing = missing_on(&mut played, &[
        Map::Route5, Map::UndergroundPathRoute5, Map::UndergroundPathNorthSouth,
        Map::UndergroundPathRoute6, Map::Route6, Map::VermilionCity, Map::VermilionPokecenter,
        Map::VermilionMart, Map::PokemonFanClub, Map::VermilionOldRodHouse, Map::VermilionPidgeyHouse,
        Map::VermilionTradeHouse, Map::VermilionDock, Map::SSAnne1F, Map::SSAnne2F, Map::SSAnne3F,
        Map::SSAnneB1F, Map::SSAnneBow, Map::SSAnneKitchen, Map::SSAnneCaptainsRoom, Map::SSAnne1FRooms,
        Map::SSAnne2FRooms, Map::SSAnneB1FRooms, Map::VermilionGym,
    ], &[Entry::Badge(2), Entry::Way(Way::OldRod), Entry::Machine(ItemId::Hm01Cut as u8)]);
    cut(&mut played, "completion-thunder");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

/// Diglett's Cave and Route 2's east side, the Day Care, Route 25's tree, Rock Tunnel, Lavender
/// and the way west under Saffron to Celadon.
pub fn to_celadon() -> Vec<Step> {
    use Step::*;
    let mut steps = vec![
        Collect(true),
        // Out of the gym's corner, which the tree closes again behind every visit.
        Take("cut down the tree"),
        GoTo("Route11"), GoTo("DiglettsCaveRoute11"), Clear(&[]), GoTo("DiglettsCave"),
        Hunt { species: "*", row: "Pace", ball: "MasterBall", way: Way::WildOnACaveFloor },
        GoTo("DiglettsCaveRoute2"), Clear(&[]), GoTo("Route2"),
        Explore { maps: &["Route2", "Route2TradeHouse", "Route2Gate"], patience: 300 },
        GoTo("DiglettsCaveRoute2"), GoTo("DiglettsCave"), GoTo("DiglettsCaveRoute11"), GoTo("Route11"),
        GoTo("VermilionCity"), GoTo("Route6"), GoTo("UndergroundPathRoute6"), GoTo("UndergroundPathNorthSouth"),
        GoTo("UndergroundPathRoute5"), GoTo("Route5"), GoTo("CeruleanCity"),
        GoTo("CeruleanTrashedHouse"), GoTo("CeruleanCity"),
        // The tree on the main terrace is the other way down to Route 5, and the only one to the
        // Day Care.
        Take("cut down the tree"), GoTo("Route5"), GoTo("Daycare"), DayCare(Some("Squirtle")),
        GoTo("Route5"), GoTo("CeruleanCity"),
        GoTo("Route24"), GoTo("Route25"), Take("cut down the tree"), Talk("TMSeismicToss"),
        GoTo("Route24"), GoTo("CeruleanCity"),
        GoTo("CeruleanTrashedHouse"), Take("CeruleanCity, arriving at (28, 10)"),
        GoTo("Route9"), Explore { maps: &["Route9"], patience: 200 },
        GoTo("Route10"), Explore { maps: &["Route10", "RockTunnelPokecenter"], patience: 200 },
        GoTo("RockTunnel1F"), Explore { maps: &["RockTunnel1F", "RockTunnelB1F"], patience: 600 },
        GoTo("RockTunnel1F"), Take("Route10, arriving at (9, 5"), Clear(&[]),
        GoTo("LavenderTown"), Clear(&[]),
    ];
    for building in ["LavenderPokecenter", "LavenderMart", "LavenderCuboneHouse", "MrFujisHouse", "NameRatersHouse"] {
        steps.extend(visit(building, "LavenderTown"));
    }
    steps.extend([
        GoTo("PokemonTower1F"), Clear(&[]), GoTo("PokemonTower2F"), Clear(&[]),
        GoTo("PokemonTower1F"), GoTo("LavenderTown"),
        GoTo("Route12"), Clear(&["Snorlax"]), GoTo("Route12Gate1F"), Clear(&[]), GoTo("Route12Gate2F"), Clear(&[]),
        GoTo("Route12Gate1F"), GoTo("Route12"), GoTo("LavenderTown"),
        GoTo("Route8"), Explore { maps: &["Route8"], patience: 200 },
        GoTo("Route8Gate"), Clear(&[]), GoTo("Route8"),
        GoTo("UndergroundPathRoute8"), Clear(&[]), GoTo("UndergroundPathWestEast"), Clear(&[]),
        GoTo("UndergroundPathRoute7"), Clear(&[]), GoTo("Route7"), Clear(&[]),
        GoTo("Route7Gate"), Clear(&[]), GoTo("CeladonCity"),
    ]);
    steps
}

#[test]
#[ignore = "a phase of the completion run; run with --ignored"]
fn completion_phase_celadon() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-thunder.bin"), "completion-celadon",
                          to_celadon(), 360, Duration::from_secs(2400));
    let missing = missing_on(&mut played, &[
        Map::DiglettsCaveRoute11, Map::DiglettsCave, Map::DiglettsCaveRoute2, Map::Route2, Map::Route2TradeHouse,
        Map::Route2Gate, Map::Daycare, Map::Route25, Map::Route9, Map::Route10, Map::RockTunnelPokecenter,
        Map::RockTunnel1F, Map::RockTunnelB1F, Map::LavenderTown, Map::LavenderPokecenter, Map::LavenderMart,
        Map::LavenderCuboneHouse, Map::MrFujisHouse, Map::NameRatersHouse, Map::PokemonTower1F,
        Map::PokemonTower2F, Map::Route12Gate1F, Map::Route12Gate2F, Map::Route8, Map::Route8Gate,
        Map::UndergroundPathRoute8, Map::UndergroundPathWestEast, Map::UndergroundPathRoute7, Map::Route7,
        Map::Route7Gate,
    ], &[Entry::Way(Way::WildOnACaveFloor)]);
    cut(&mut played, "completion-celadon");
    // On the water by the Power Plant, for the phase after Surf.
    let later = [Entry::Trainer { map: Map::Route10, index: 0 }];
    let missing: Vec<Entry> = missing.into_iter().filter(|entry| !later.contains(entry)).collect();
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

/// Celadon: the Mart and its roof, the Mansion's Eevee, the Game Corner's prizes, Erika, and Route
/// 16's Fly house; then Fly back to Cerulean for the Bicycle.
pub fn to_the_rainbow_badge() -> Vec<Step> {
    use Step::*;
    vec![
        Collect(true),
        GoTo("CeladonPokecenter"), Clear(&[]),
        AtPc(Pc::Deposit("Magikarp")), AtPc(Pc::Deposit("Magikarp")), AtPc(Pc::ChangeBox(2)),
        GoTo("CeladonCity"), Explore { maps: &["CeladonCity"], patience: 200 },
        GoTo("CeladonMart1F"), Clear(&[]),
        GoTo("CeladonMart2F"), Tidy, Talk("Clerk2"),
        Buy(&[("Tm32DoubleTeam", 1), ("Tm33Reflect", 1), ("Tm02RazorWind", 1), ("Tm07HornDrill", 1),
              ("Tm37EggBomb", 1), ("Tm01MegaPunch", 1), ("Tm05MegaKick", 1), ("Tm09TakeDown", 1),
              ("Tm17Submission", 1)]),
        Tidy, Clear(&[]),
        GoTo("CeladonMart3F"), Clear(&[]),
        GoTo("CeladonMart4F"), Talk("Clerk"),
        Buy(&[("FireStone", 1), ("WaterStone", 1), ("ThunderStone", 1), ("LeafStone", 1)]), Clear(&[]),
        GoTo("CeladonMart5F"), Clear(&[]),
        GoTo("CeladonMartRoof"), Clear(&["LittleGirl"]),
        // The girl trades a different machine for each drink, and takes the one she is shown.
        Take("buy a FRESH WATER"), Talk("LittleGirl"), Take("buy a SODA POP"), Talk("LittleGirl"),
        Take("buy a LEMONADE"), Talk("LittleGirl"),
        // One more, for the guards at Saffron's gates.
        Take("buy a FRESH WATER"),
        GoTo("CeladonMart5F"), GoTo("CeladonMartElevator"), Field(r#"{"move":"elevator","map":"CeladonMart1F"}"#),
        GoTo("CeladonMart1F"), GoTo("CeladonCity"),
        GoTo("CeladonMansion1F"),
        Explore { maps: &["CeladonMansion1F", "CeladonMansion2F", "CeladonMansion3F", "CeladonMansionRoof",
                          "CeladonMansionRoofHouse"], patience: 200 },
        // The exploring took the Eevee in the roof house.
        Evolve { item: "FireStone", species: "Eevee" },
        GoTo("CeladonMansionRoof"), GoTo("CeladonMansion3F"), GoTo("CeladonMansion2F"), GoTo("CeladonMansion1F"),
        GoTo("CeladonCity"),
        GoTo("CeladonDiner"), Clear(&[]), GoTo("CeladonCity"),
        GoTo("CeladonHotel"), Clear(&[]), GoTo("CeladonCity"),
        GoTo("CeladonChiefHouse"), Clear(&[]), GoTo("CeladonCity"),
        GoTo("GameCorner"), Clear(&[]),
        Coins(9900),
        GoTo("CeladonCity"), GoTo("GameCornerPrizeRoom"), Clear(&[]),
        // The machines are items, and the bag holds twenty kinds.
        Tidy,
        Prize("Abra"), Field(r#"{"move":"prize","item":"Tm15HyperBeam"}"#), Field(r#"{"move":"prize","item":"Tm23DragonRage"}"#),
        GoTo("CeladonCity"), GoTo("GameCorner"), Coins(7700),
        GoTo("CeladonCity"), GoTo("GameCornerPrizeRoom"), Field(r#"{"move":"prize","item":"Tm50Substitute"}"#),
        GoTo("CeladonCity"), GoTo("CeladonGym"), Explore { maps: &["CeladonGym"], patience: 200 },
        GoTo("CeladonCity"), GoTo("Route16"), Explore { maps: &["Route16"], patience: 100 },
        // The Fly house is on the west side, through the gate's top corridor, which has no guard.
        Take("Route16Gate1F, arriving at (7, 2)"), Take("Route16, arriving at (17, 4)"),
        Explore { maps: &["Route16", "Route16FlyHouse"], patience: 100 },
        GoTo("Route16"), Take("Route16Gate1F, arriving at (0, 2)"), Take("Route16, arriving at (24, 4)"),
        Teach { item: "Hm02Fly", species: "Farfetchd" },
        Field(r#"{"move":"fly","map":"CeruleanCity"}"#),
        GoTo("BikeShop"), Clear(&[]), GoTo("CeruleanCity"),
        Field(r#"{"move":"fly","map":"CeladonCity"}"#),
        GoTo("CeladonCity"),
    ]
}

#[test]
#[ignore = "a phase of the completion run; run with --ignored"]
fn completion_phase_rainbow_badge() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-celadon.bin"), "completion-rainbow",
                          to_the_rainbow_badge(), 360, Duration::from_secs(2400));
    let missing = missing_on(&mut played, &[
        Map::CeladonCity, Map::CeladonPokecenter, Map::CeladonMart1F, Map::CeladonMart2F, Map::CeladonMart3F,
        Map::CeladonMart4F, Map::CeladonMart5F, Map::CeladonMartRoof, Map::CeladonMartElevator,
        Map::CeladonMansion1F, Map::CeladonMansion2F, Map::CeladonMansion3F, Map::CeladonMansionRoof,
        Map::CeladonMansionRoofHouse, Map::CeladonDiner, Map::CeladonHotel, Map::CeladonChiefHouse,
        Map::GameCorner, Map::GameCornerPrizeRoom, Map::CeladonGym, Map::Route16FlyHouse, Map::Route16Gate1F,
        Map::BikeShop,
    ], &[Entry::Badge(3), Entry::Way(Way::GiftEevee), Entry::Way(Way::EvolvedByStone),
         Entry::Way(Way::GameCornerPrize), Entry::Way(Way::PcChangeBox),
         Entry::Machine(ItemId::Hm02Fly as u8), Entry::Machine(ItemId::Tm13IceBeam as u8),
         Entry::Machine(ItemId::Tm48RockSlide as u8), Entry::Machine(ItemId::Tm49TriAttack as u8),
         Entry::Machine(ItemId::Tm15HyperBeam as u8), Entry::Machine(ItemId::Tm23DragonRage as u8),
         Entry::Machine(ItemId::Tm50Substitute as u8), Entry::KeyItem(vec![ItemId::Bicycle as u8])]);
    cut(&mut played, "completion-rainbow");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

/// The Rocket Hideout and the Silph Scope, Pokémon Tower and the Poké Flute, and the two Snorlax
/// the flute wakes.
pub fn to_the_poke_flute() -> Vec<Step> {
    use Step::*;
    vec![
        Collect(true),
        GoTo("GameCorner"), Take("look behind the poster"),
        GoTo("RocketHideoutB1F"),
        Explore { maps: &["RocketHideoutB1F", "RocketHideoutB2F", "RocketHideoutB3F", "RocketHideoutB4F"],
                  patience: 900 },
        // Each floor's lift lobby is a room of its own, and B4F's is the only way into Giovanni's
        // half of that floor, so the lift is both the ride and the route.
        GoTo("RocketHideoutB4F"), GoTo("RocketHideoutElevator"),
        Field(r#"{"move":"elevator","map":"RocketHideoutB1F"}"#),
        // The lobby has a Rocket in it, and beating him is what opens its door.
        Talk("Rocket5"),
        // The ride ends outside the lift, so the next one starts by walking back in.
        GoTo("RocketHideoutElevator"), Field(r#"{"move":"elevator","map":"RocketHideoutB4F"}"#),
        Explore { maps: &["RocketHideoutB1F", "RocketHideoutB2F", "RocketHideoutB3F", "RocketHideoutB4F"],
                  patience: 900 },
        GoTo("RocketHideoutElevator"), Field(r#"{"move":"elevator","map":"RocketHideoutB2F"}"#),
        GoTo("RocketHideoutB1F"), GoTo("GameCorner"), GoTo("CeladonCity"),
        Field(r#"{"move":"fly","map":"LavenderTown"}"#), GoTo("LavenderTown"),
        // The tower fills the bag, and Mr Fuji hands over nothing it has no room for.
        Tidy,
        GoTo("PokemonTower1F"),
        Explore { maps: &["PokemonTower1F", "PokemonTower2F", "PokemonTower3F", "PokemonTower4F",
                          "PokemonTower5F", "PokemonTower6F", "PokemonTower7F"], patience: 900 },
        // Mr Fuji's thanks puts the player in his house, with the flute.
        GoTo("LavenderTown"), Tidy, GoTo("MrFujisHouse"), Clear(&[]), GoTo("LavenderTown"),
        // Route 12 is two halves either side of its gate, and the Snorlax sleeps on the south one.
        GoTo("Route12"), GoTo("Route12Gate1F"), Take("Route12, arriving at (11, 2"),
        UseItemOn { item: "PokeFlute", row: "Snorlax" },
        Explore { maps: &["Route12"], patience: 400 },
        GoTo("Route12SuperRodHouse"), Clear(&[]), GoTo("Route12"),
        Field(r#"{"move":"fly","map":"CeladonCity"}"#), GoTo("CeladonCity"),
        GoTo("Route16"), UseItemOn { item: "PokeFlute", row: "Snorlax" },
        // The gate's stairs are in its lower hall, past where the Snorlax slept.
        GoTo("Route16Gate1F"), GoTo("Route16Gate2F"), Clear(&[]), GoTo("Route16Gate1F"), Clear(&[]),
        // Out of the gate's lower hall onto the cycling road's side, where the bikers are.
        Take("Route16, arriving at (17, 1"), Clear(&[]),
        // That side leads on to Route 17, and Fly is refused indoors, so fly from here.
        Field(r#"{"move":"fly","map":"CeladonCity"}"#), GoTo("CeladonCity"),
    ]
}

#[test]
#[ignore = "a phase of the completion run; run with --ignored"]
fn completion_phase_poke_flute() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-rainbow.bin"), "completion-flute",
                          to_the_poke_flute(), 420, Duration::from_secs(2400));
    let missing = missing_on(&mut played, &[
        Map::RocketHideoutB1F, Map::RocketHideoutB2F, Map::RocketHideoutB3F, Map::RocketHideoutB4F,
        Map::RocketHideoutElevator, Map::PokemonTower1F, Map::PokemonTower2F, Map::PokemonTower3F,
        Map::PokemonTower4F, Map::PokemonTower5F, Map::PokemonTower6F, Map::PokemonTower7F,
        Map::Route12, Map::Route12SuperRodHouse, Map::Route16, Map::Route16Gate2F,
    ], &[Entry::Way(Way::SnorlaxOnRoute12), Entry::Way(Way::SnorlaxOnRoute16),
         Entry::KeyItem(vec![ItemId::SilphScope as u8]), Entry::KeyItem(vec![ItemId::PokeFlute as u8]),
         Entry::KeyItem(vec![ItemId::LiftKey as u8]), Entry::KeyItem(vec![ItemId::SuperRod as u8])]);
    cut(&mut played, "completion-flute");
    // South of the gate on Route 12, on the stretch the Fuchsia phase walks.
    let later = [Entry::ItemBall { map: Map::Route12, object: 9, item: ItemId::Tm16PayDay as u8 }];
    let missing: Vec<Entry> = missing.into_iter().filter(|entry| !later.contains(entry)).collect();
    assert!(missing.is_empty(), "the phase left {missing:?}");
}
