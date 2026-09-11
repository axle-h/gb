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

/// How a gift, a catch or a trade ends up in the party, for the nickname prompt that follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Came { Caught, Given, Traded }

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
    /// Choose the row whose id ends in `:{row}` until a wild `species` is caught with `ball`.
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
    /// End a turn without moving.
    Wait,
}

/// What a tidy keeps besides what cannot be tossed: the balls, the stones and the candy.
const KEEP: &[ItemId] = &[
    ItemId::MasterBall, ItemId::UltraBall, ItemId::GreatBall, ItemId::PokeBall, ItemId::RareCandy,
    ItemId::MoonStone, ItemId::FireStone, ItemId::WaterStone, ItemId::ThunderStone, ItemId::LeafStone,
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

/// The map an exit row leads to, from its description.
fn destination(id: &str, description: &str) -> Option<String> {
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
    /// A ball was thrown at the hunted species and the catch not yet seen.
    thrown: bool,
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
            orders: VecDeque::new(), thrown: false, came: Came::Given, named: 0, party_was_full: false,
            graph: Default::default(), travelled: Default::default(), offered: HashSet::new(), barren: 0,
            last_walk: None, here: String::new(), pockets: Default::default(), pocket_edges: Default::default(), left_by: None,
            exploring: 0, tidying: None, ledger,
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

    fn menu_listing(request: &TurnRequest) -> String {
        request.menu_rows().iter().map(|(id, what)| format!("  - `{id}` — {what}")).collect::<Vec<_>>().join("\n")
    }

    /// The wild battle turn: throw at the hunted species, and run from everything else.
    fn battle(&mut self, request: &TurnRequest) -> Reply {
        let ids = request.menu_ids();
        let situation = request.situation();
        let foe = situation.lines().find_map(|line| line.strip_prefix("Enemy: "))
            .and_then(|rest| rest.split_whitespace().next()).unwrap_or("").to_string();
        let choose = |id: &str| Reply::call("choose_battle_action", serde_json::json!({ "id": id, "summary": "as planned" }));
        if let Some(Step::Hunt { species, ball, .. }) = self.steps.get(self.at).cloned() {
            if foe.eq_ignore_ascii_case(species) {
                // The Safari Zone's menu has one ball and no bag.
                let throw = if ids.iter().any(|id| id == "ball") { "ball".to_string() } else { format!("item:{ball}") };
                if ids.contains(&throw) {
                    self.thrown = true;
                    return choose(&throw);
                }
            }
        }
        let training = match self.steps.get(self.at) {
            Some(Step::Train { flee, .. }) => !flee.iter().any(|species| foe.eq_ignore_ascii_case(species)),
            _ => false,
        };
        if !training && ids.iter().any(|id| id == "run") {
            return choose("run");
        }
        match ids.iter().find(|id| id.starts_with("fight:")) {
            Some(id) => choose(id),
            None => Reply::Calls(vec![Call::wait(10)]),
        }
    }

    fn nickname(&mut self) -> Reply {
        // Every catch asks for a name, which makes the prompt the proof of the catch.
        if self.thrown {
            self.thrown = false;
            if let Some(Step::Hunt { way, .. }) = self.steps.get(self.at).cloned() {
                self.saw(way);
                if self.party_was_full {
                    self.saw(Way::CaughtToTheBox);
                }
                self.at += 1;
            }
        }
        let way = match self.came {
            Came::Caught => Way::NicknameAfterACatch,
            Came::Given => Way::NicknameAfterAGift,
            Came::Traded => Way::NicknameAfterATrade,
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
            .filter(|id| (id.matches(':').count() == 1 && !id.contains("Boulder")) || id.ends_with(":CutTree"))
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
        self.chosen.entry(map).or_default().insert(id.clone());
        Reply::call("choose_action", serde_json::json!({ "id": id, "resume_after_battle": true, "summary": "as planned" }))
    }

    /// One turn of a tidy: read the bag, then toss what it holds that nothing needs.
    fn tidy(&mut self, request: &TurnRequest) -> Option<Reply> {
        match self.tidying.as_mut()? {
            None => {
                let bag = request.messages.iter().rev().filter(|m| m.role == "tool")
                    .find_map(|m| serde_json::from_str::<serde_json::Value>(&m.text).ok()
                        .filter(|value| value.get("slots_used").is_some()));
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

    fn overworld(&mut self, request: &TurnRequest) -> Reply {
        let text = request.situation().to_string();
        // A pickup the bag had no room for: take it back, and make room. Only for that reason: a
        // starter's ball in Oak's lab is not picked up either, and never will be.
        if let Some((map, id, name, _)) = self.last_walk.as_ref()
            && text.contains(&format!("nothing was picked up: the {name}"))
            && text.contains("No more room")
        {
            if let Some(chosen) = self.chosen.get_mut(map) {
                chosen.remove(id);
            }
            self.tidying.get_or_insert(None);
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
                Step::Take(fragment) => Intent::Says(fragment).resolve(request),
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
                    self.route(request, target).or_else(|| Intent::Enter(target).resolve(request))
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
                Step::Tidy => { self.at += 1; continue }
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
                        Step::Hunt { .. } | Step::Train { .. } | Step::Clear(_) | Step::GoTo(_) | Step::Explore { .. } => {}
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
                            self.at + 1, self.steps.len(), Self::PATIENCE, Self::menu_listing(request)));
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
            return Reply::call("forget_move", serde_json::json!({ "summary": "keeping what it knows" }));
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
        panic!("[completion:{name}] stuck: {why}");
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
