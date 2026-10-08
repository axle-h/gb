//! The brain that plays the tour: a list of [`Step`]s, each resolved against the menu the turn was
//! sent, and [`FinishFlag`], which hands it one phase after another and says when the last is done.
//!
//! The brain answers from the rendered turn only. What it may be handed is the god-mode boundary:
//! battle strength and money (`Cheats::story`), Master Balls and Full Heals held in the bag
//! ([`STOCK`]), and Rare Candies where a step asks for them. Everything else is earned through the
//! menu.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use strum::IntoEnumIterator;

use crate::tour::completion::{Entry, Ledger, Legend, Way};
use crate::tour::intent::{names_map, Intent};
use crate::tour::turn::{Brain, Call, Reply, TurnRequest};
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::item::ItemId;


/// Trainers are fought by the script; a wild battle is the brain's, because only it knows what the
/// run is hunting. Any status is cured first, from the Full Heals the run holds. A foe is knocked
/// out with the most accurate move that does it at the lowest damage roll, the weakest of those:
/// the strongest can be Blizzard, and a miss hands a Mirror Move user a Blizzard of its own. A
/// frozen lead with no cure is switched out while anyone else can fight: only a Fire move thaws it.
pub const SCRIPT: &str = r#"
if battle.kind != "safari" && !battle.me.fainted && battle.me.status != "" {
    let cure = #{ poisoned: "Antidote", burned: "BurnHeal", frozen: "IceHeal", paralyzed: "ParlyzHeal", asleep: "Awakening" };
    for want in [cure[battle.me.status], "FullHeal", "FullRestore"] {
        for item in battle.bag {
            if item.name == want { battle.use_item(item.name); }
        }
    }
}
if battle.kind == "wild" { battle.ask(); }
if battle.me.fainted {
    for mon in battle.party {
        if !mon.fainted && mon.slot != battle.me.slot { battle.switch_to(mon); }
    }
}
let knockout = ();
for mv in battle.moves {
    if mv.usable && mv.damage * 217 / 255 >= battle.foe.hp && (knockout == () || mv.accuracy > knockout.accuracy
            || mv.accuracy == knockout.accuracy && mv.damage < knockout.damage) {
        knockout = mv;
    }
}
if knockout != () && battle.me.status != "frozen" { battle.fight(knockout); }
if battle.best_move != () && battle.me.status != "frozen" { battle.fight(battle.best_move); }
for mon in battle.party {
    if !mon.fainted && mon.slot != battle.me.slot {
        for mv in mon.moves {
            if mv.usable && mv.damage > 0 { battle.switch_to(mon); }
        }
    }
}
if battle.me.status == "frozen" {
    for item in battle.bag {
        if item.name == "IceHeal" || item.name == "FullHeal" || item.name == "FullRestore" { battle.use_item(item.name); }
    }
}
if battle.best_move != () { battle.fight(battle.best_move); }
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
    /// The row whose description contains this, until the menu stops offering it. What a door
    /// wants where somebody paces between its two tiles: the row is minted from whichever tile is
    /// nearer, so its id moves as they move, and a reply naming the id the turn was rendered with
    /// can land after it has gone.
    Repeat(&'static str),
    /// A `use_field_move`, by its JSON arguments.
    Field(&'static str),
    /// Answer the mart that is open with these orders, then leave.
    Buy(&'static [(&'static str, u32)]),
    /// Choose the row whose id ends in `:{row}` until a wild `species` is caught with `ball`;
    /// `*` is any species not caught yet. A hunt wanders, and wandering crosses a map edge, so
    /// `on` is walked back to whenever the row is missing because the run has drifted off it.
    Hunt { species: &'static str, row: &'static str, ball: &'static str, way: Way, on: &'static str },
    /// Choose the `row` and fight every wild battle, except from the species in `flee`, until a
    /// turn says `until`, then record `way`.
    Train { row: &'static str, until: &'static str, way: Way, flee: &'static [&'static str] },
    /// Walk to `map` over the transitions the brain has been offered so far.
    GoTo(&'static str),
    /// Stand on `map` having come in at `landing`, walking the pockets learned so far to the door
    /// that lands there: for a map of several pockets, where being on it says nothing of which.
    Enter { map: &'static str, landing: (u8, u8) },
    /// Leave by the opening into `map` that lands nearest `landing`. Each opening is a row of its
    /// own and they read alike but for where they land, and the square a row names moves with the
    /// player, so the landing is what picks one out from wherever the run stands.
    Cross { map: &'static str, landing: (u8, u8) },
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
    /// Use the stone or Rare Candy `item` on the first party member of `species`; done once it has
    /// evolved.
    Evolve { item: &'static str, species: &'static str },
    /// From here on, throw a Master Ball at every wild species not caught yet, or stop.
    Collect(bool),
    /// Feed the first party member of `species` a Rare Candy and stop the evolution it starts; done
    /// once it has gained the level and kept its species.
    KeepFromEvolving(&'static str),
    /// Pace the Safari Zone's grass until the game ends on its step count, throwing bait at the
    /// first thing met, a rock at the next, and running from each.
    Safari,
}

/// One `pc_pokemon` operation, on the first Pokémon of a species.
#[derive(Debug, Clone, Copy)]
pub enum Pc {
    Deposit(&'static str),
    /// Whoever is in this party slot, for a slot that holds whatever the collecting last caught.
    DepositSlot(u8),
    Withdraw(&'static str),
    Release(&'static str),
    ChangeBox(u8),
}

impl Pc {
    fn way(self) -> Way {
        match self {
            Pc::Deposit(_) | Pc::DepositSlot(_) => Way::PcDeposit,
            Pc::Withdraw(_) => Way::PcWithdraw,
            Pc::Release(_) => Way::PcRelease,
            Pc::ChangeBox(_) => Way::PcChangeBox,
        }
    }
}

/// What a tidy keeps besides what cannot be tossed: the balls, the stones and the candy.
const KEEP: &[ItemId] = &[
    // The catches, a level evolution, Pikachu's evolution for the Raichu trade, a drink for
    // Saffron's guards, and the battle script's cure.
    ItemId::MasterBall, ItemId::RareCandy, ItemId::ThunderStone, ItemId::FreshWater, ItemId::FullHeal,
];

/// What the run is handed and held at, as `(item, refilled below, refilled to)`, topped up only in
/// the overworld so a driver mid-battle sees what it spends: Master Balls for every catch outside
/// the Safari Zone, as the legendary legs have, and Full Heals for [`SCRIPT`]'s cures.
pub const STOCK: [(ItemId, u8, u8); 2] = [(ItemId::MasterBall, 5, 10), (ItemId::FullHeal, 5, 10)];

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

/// The tile a row's id names, for every row that carries one: `Map:x,y:Kind`. A sprite's id has no
/// coordinate, so it has none.
fn id_at(id: &str) -> Option<(u8, u8)> {
    let (x, y) = id.split(':').nth(1)?.split_once(',')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

/// Where the run came into this map, from the turn's own line. The prompt writes it only for an
/// arrival it still holds and only on the map that arrival belongs to, so its absence is ordinary.
fn entered_at(situation: &str) -> Option<(u8, u8)> {
    coords_after(situation, "Entered this map at (")
}

/// The row to leave this map by for `target`. More than one exit can name the same map: a house
/// with two doors offers both of them as the way to the town outside, and one of those can land in
/// a pocket the rest of that town cannot be reached from. The door the run came in by is the one it
/// knows leads somewhere, so where the turn says where that was, the exit nearest it wins. With no
/// such line, or no exit carrying a coordinate, the first match stands, which is what
/// `Intent::Enter` does on its own.
/// The coordinates written after `lead`, for the one place a description writes them.
fn coords_after(text: &str, lead: &str) -> Option<(u8, u8)> {
    let rest = text.split(lead).nth(1)?;
    let (x, rest) = rest.split_once(", ")?;
    Some((x.parse().ok()?, rest.split(')').next()?.parse().ok()?))
}

/// The row for the opening into `target` that lands nearest `landing`.
fn crossing_toward(request: &TurnRequest, target: &str, landing: (u8, u8)) -> Option<String> {
    request.menu_rows().into_iter()
        .filter(|(id, what)| id.ends_with(":Connection") && names_map(what, target))
        .filter_map(|(id, what)| coords_after(&what, "arriving at (").map(|at| (id, at)))
        .min_by_key(|(_, at)| at.0.abs_diff(landing.0) as u16 + at.1.abs_diff(landing.1) as u16)
        .map(|(id, _)| id)
}

fn enter_toward(request: &TurnRequest, target: &'static str) -> Option<String> {
    let first = Intent::Enter(target).resolve(request);
    let Some(came_in_at) = entered_at(request.situation()) else { return first };
    request.menu_rows().into_iter()
        .filter(|(_, what)| names_map(what, target))
        .filter_map(|(id, _)| id_at(&id).map(|at| (id, at)))
        .min_by_key(|(_, at)| at.0.abs_diff(came_in_at.0) as u16 + at.1.abs_diff(came_in_at.1) as u16)
        .map(|(id, _)| id)
        .or(first)
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
    /// Turns the current [`Step::Explore`] has taken, and how many of them have passed since it
    /// last saw somewhere new or took something.
    exploring: usize,
    explore_idle: usize,
    /// The maps this exploring has stood on.
    explored_maps: HashSet<String>,
    /// The trees this exploring has cut. A tree opened is progress; the same tree grown back after
    /// a battle reloaded the map is not, or a route thick with encounters resets the idle count for
    /// ever and only the patience bound is left to stop it.
    explore_cut: HashSet<String>,
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
    /// A quiz machine was answered and the turn that says whether it was right not yet seen.
    quiz_pending: bool,
    /// The Safari game's actions this brain has seen, which [`Step::Safari`] plays through. Not the
    /// ledger: an earlier visit that ran out of steps would end the step before it began.
    safari_seen: HashSet<Way>,
    /// The party slot a stone was used on.
    evolving: Option<u8>,
    /// The party slot and level a Rare Candy was fed to, its evolution to be stopped.
    kept_from_evolving: Option<(u8, u8)>,
    /// The party's size when a Day Care call was sent.
    day_care_sent: Option<usize>,
    /// Pickups that failed, per map and row.
    pickups_failed: std::collections::HashMap<String, u8>,
    /// Walks given up on the way, per map and row.
    walks_given_up: std::collections::HashMap<String, u8>,
    /// A [`Step::Take`] of a row with a square in it or a [`Step::Talk`], done when the row is
    /// chosen: the map it was taken from, the step, and, once a turn has said, whether the walk to
    /// it was given up, for the next overworld turn to take it again.
    taking: Option<(String, usize, Option<bool>)>,
    /// A [`Step::Field`] sent: the map and the step, to send again if the next turn says the agent
    /// could not reach what the field move was for, a counter someone stood in front of say.
    field_sent: Option<(String, usize)>,
    pub ledger: Arc<Mutex<Ledger>>,
    pub stuck: Arc<Mutex<Option<String>>>,
    pub turns: Arc<Mutex<usize>>,
}

/// How long a step that found no row waits before looking again. A map is still loading for a
/// moment after a warp, and its rows arrive with it, so this has to outlast that.
const UNRESOLVED_TICKS: u64 = 60;

impl CompletionBrain {
    /// Turns a step may find nothing before the run is called stuck. Rows come and go as people
    /// walk: a route is computed with them as obstacles, and half of Silph Co's third floor hangs
    /// off a one-tile gap that a Rocket paces across, so this has to outlast a person.
    const PATIENCE: usize = 80;

    /// How often one place may stop one walk before the target is written off. A worker who
    /// wanders back into the way stops a walk a couple of times and then does not; a prompt built
    /// into the ground stops it for ever, so the net is set well clear of what a walk that is
    /// getting closer has ever needed.
    const RETRIES_PER_PLACE: u8 = 6;
    const MAX_REISSUES: usize = 30;

    pub fn new(steps: Vec<Step>, ledger: Arc<Mutex<Ledger>>) -> Self {
        Self {
            steps, at: 0, armed: false, chosen: Default::default(), unresolved: 0, reissued: 0,
            orders: VecDeque::new(), thrown: None, caught: HashSet::new(), collecting: false, box_full: false,
            ran: Default::default(), running_at: 0, came: Came::Given, named: 0, party_was_full: false,
            graph: Default::default(), travelled: Default::default(), offered: HashSet::new(), barren: 0,
            last_walk: None, here: String::new(), pockets: Default::default(), pocket_edges: Default::default(), left_by: None,
            exploring: 0, explore_idle: 0, explored_maps: HashSet::new(), explore_cut: HashSet::new(), tidying: None, bins: (None, HashSet::new(), None), pc_sent: None, teaching: false, taught: None, pickups_failed: Default::default(), walks_given_up: Default::default(), taking: None, field_sent: None, day_care_sent: None, used_on: false, prize_pending: false, quiz_pending: false, safari_seen: HashSet::new(), evolving: None, kept_from_evolving: None, repeated: (String::new(), 0), ledger,
            stuck: Arc::new(Mutex::new(None)), turns: Arc::new(Mutex::new(0)),
        }
    }

    pub fn finished(&self) -> bool {
        // A machine sent to be taught is checked on the next overworld turn, and a phase that ends
        // on the call never reaches it: the fixture is cut with the move still on its way to the
        // slot, and the phase after opens with a party that cannot do what it was taught. A talk
        // is done when it is chosen and happens after, so the tour's last one waits for its verdict.
        self.at >= self.steps.len() && self.taught.is_none() && self.taking.is_none()
    }

    fn saw(&self, way: Way) {
        self.saw_entry(Entry::Way(way));
    }

    /// What only the moment shows, where the entry is not one of the ways.
    fn saw_entry(&self, entry: Entry) {
        self.ledger.lock().expect("not poisoned").saw(entry);
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
        // The game draws this message over itself, so only the part that always survives is matched.
        if situation.contains("BOX is full") {
            self.box_full = true;
        }
        if matches!(self.steps.get(self.at), Some(Step::Safari)) {
            let action = if !self.safari_seen.contains(&Way::SafariBait) { "bait" }
                else if !self.safari_seen.contains(&Way::SafariRock) { "rock" }
                else { "run" };
            if ids.iter().any(|id| id == action) {
                return choose(action);
            }
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
        // Whatever the menu holds, in preference to nothing: a Safari battle offers a ball, bait,
        // a rock and a run and no `fight:` at all, and a turn answered with `wait` for ever is a
        // battle that never ends and an emulator that runs on burning game time in silence. A ball
        // is never that answer, though: thrown at whatever turned up it fills the box with Pokemon
        // nothing asked for, and a full box then refuses every later throw in the same words for
        // ever. The way out of the battle comes first, then anything that is not a ball.
        if let Some(id) = ids.iter().find(|id| id.starts_with("fight:")) {
            return choose(id);
        }
        let refused = self.box_full;
        let spare = |id: &String| !id.starts_with("read_") && id.as_str() != "wait";
        match ids.iter().find(|id| id.as_str() == "run")
            .or_else(|| ids.iter().find(|id| spare(id) && id.as_str() != "ball"))
            .or_else(|| ids.iter().find(|id| spare(id) && !(refused && id.as_str() == "ball")))
        {
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
                // `buy_item` refuses more than three chained kinds, and the mart closes after one call.
                if self.orders.len() > 3 {
                    self.stuck(format!("step {}: more kinds than one buy_item takes: {:?}", self.at, self.orders));
                }
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
        // Never a legendary: each is its phase's `Hunt`, and one met before the collecting is on is
        // run from, which hides it for the rest of the game.
        let things = rows.iter().map(|(id, _)| id.clone())
            .filter(|id| OverworldAction::names_a_sprite(id) && !id.contains("Boulder"))
            .filter(|id| !Legend::iter().any(|legend| id.ends_with(&format!(":{legend:?}"))))
            .collect();
        // A walk a battle interrupted comes back to the same pocket, which is no passage; one the
        // agent gave up for making no headway is a passage to nowhere, or it is taken for ever.
        if let Some(from) = self.left_by.take()
            && (from.0 != key || request.situation().contains("it stopped making progress"))
        {
            self.pocket_edges.insert(from, key.clone());
        }
        // A map not walked yet in this exploring is progress. A pocket is not: it is known by the
        // exits it offers, so a map that offers different ones on a later visit -- the Safari gate
        // lets you in or lets you leave -- looks new every time, and would reset this for ever.
        if self.explored_maps.insert(map.clone()) {
            self.explore_idle = 0;
        }
        self.pockets.insert(key.clone(), Pocket { map, things, exits });
        self.here = key.clone();
        Some(key)
    }

    /// Whether `pocket` holds anything [`Step::Explore`] has not taken.
    fn has_things(&self, pocket: &Pocket) -> bool {
        let chosen = self.chosen.get(&pocket.map);
        pocket.things.iter().any(|id| !chosen.is_some_and(|c| c.contains(id)))
    }

    /// Whether `pocket` has a way on that has never been walked from it.
    fn has_untaken_exit(&self, pocket: &Pocket, key: &str, maps: &[&str]) -> bool {
        self.untaken_exit(pocket, key, maps).is_some()
    }

    fn untaken_exit(&self, pocket: &Pocket, key: &str, maps: &[&str]) -> Option<String> {
        pocket.exits.iter()
            .find(|(way, _, to)| maps.contains(&to.as_str())
                && !self.pocket_edges.contains_key(&(key.to_string(), way.clone())))
            .map(|(_, id, _)| id.clone())
    }

    /// The first hop from `here` toward the nearest pocket `wanted` picks, over the passages
    /// already walked, skipping any hop that has been walked to death.
    fn nearest(&self, here: &str, maps: &[&str], wanted: impl Fn(&Pocket, &str) -> bool) -> Option<String> {
        let inside = |to: &String| maps.contains(&to.as_str());
        let mut first: std::collections::HashMap<String, String> = Default::default();
        let mut queue = VecDeque::from([here.to_string()]);
        let mut seen = HashSet::from([here.to_string()]);
        while let Some(at) = queue.pop_front() {
            let Some(pocket) = self.pockets.get(&at) else { continue };
            if at != here && wanted(pocket, &at) {
                return first.get(&at).cloned();
            }
            for (way, id, _) in pocket.exits.iter().filter(|(.., to)| inside(to)) {
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

    /// What [`Step::Explore`] takes next from `here`. Things come before passages, and things
    /// anywhere known come before a passage here: a border with several doors is otherwise walked
    /// back and forth, each crossing paying for whatever the grass rolls, while what is left to
    /// pick up waits two rooms away.
    fn explore(&self, here: &str, maps: &[&str]) -> Option<String> {
        let pocket = self.pockets.get(here)?;
        let chosen = self.chosen.get(&pocket.map);
        if let Some(id) = pocket.things.iter().find(|id| !chosen.is_some_and(|c| c.contains(*id))) {
            return Some(id.clone());
        }
        if let Some(hop) = self.nearest(here, maps, |pocket, _| self.has_things(pocket)) {
            return Some(hop);
        }
        if let Some(id) = self.untaken_exit(pocket, here, maps) {
            return Some(id);
        }
        self.nearest(here, maps, |pocket, key| self.has_untaken_exit(pocket, key, maps))
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
        // A person or a thing: what the event says if the walk is given up on the way. A statue
        // too: the Mansion's are pressed in an order, and a press lost to a run of wild battles
        // leaves every floor's doors the wrong way round.
        let name = request.menu_rows().iter().find(|(row, _)| *row == id)
            .and_then(|(_, what)| ["talk to ", "pick up the ", "pick up ", "examine the ", "examine ", "read the "].iter()
                .find_map(|lead| what.strip_prefix(lead)).map(|rest| rest.split(" (").next().unwrap_or(rest).to_string())
                .or_else(|| what.starts_with("press this statue's switch").then(|| "a statue".to_string())));
        self.last_walk = name.map(|name| (map.clone(), id.clone(), name, self.at));
        // Taking a person or a thing is progress; walking through a door on the way is not. So is
        // cutting a tree that was not cut before in this exploring: a cut row carries coordinates
        // and so two colons, and Route 2's eight trees regrow on every battle, so counting each one
        // as standing still spent the whole idle allowance on the work that opens the route.
        if OverworldAction::names_a_sprite(&id) || (id.ends_with(":CutTree") && self.explore_cut.insert(id.clone())) {
            self.explore_idle = 0;
        }
        *self.travelled.entry(format!("{map}|{id}")).or_default() += 1;
        // Either answer to a quiz machine: which one it was is what the next turn says.
        self.quiz_pending |= id.contains(":Quiz");
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
            Pc::DepositSlot(slot) => Some(PcBoxOp::Deposit { slot }),
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
            && text.contains("room for this")
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
        // A walk given up on the way did not happen: take it back, and the step with it. Where it
        // was stopped is what says whether trying again is worth it. A walk stopped somewhere new
        // each time is getting closer, whatever stopped it; a walk stopped at the same place for
        // the same reason will be stopped there again for ever, and a target taken back for ever
        // is one the exploring returns to for ever. The Safari gate's workers stand behind the
        // prompt that asks whether you are leaving, always on the same tile, so approaching one
        // ends the visit every time it is tried.
        // A walk taken up again after every battle is handed back in words of its own once the
        // battles run out, and a floor with an encounter rate like the Power Plant's runs them out.
        if let Some((map, id, name, step)) = self.last_walk.take()
            && let Some(stopped) = text.split(&format!("gave up on {name}")).nth(1)
                .map(|said| said.lines().next().unwrap_or("").trim().to_string())
                .or_else(|| text.contains(&format!("`{id}` has been interrupted by a battle")).then(|| "battles".to_string()))
        {
            let tries = self.walks_given_up.entry(format!("{map}|{id}|{stopped}")).or_default();
            *tries += 1;
            if *tries <= Self::RETRIES_PER_PLACE {
                if let Some(chosen) = self.chosen.get_mut(&map) {
                    chosen.remove(&id);
                }
                self.at = self.at.min(step);
            }
        }
        // A passage, a push or a talk taken and never reached: a battle on the way, say, left the
        // run where it was.
        if let Some((map, step)) = self.field_sent.take()
            && request.location().as_deref() == Some(map.as_str())
            && let Some(since) = text.split("### Since your last decision").nth(1)
            && (since.to_lowercase().contains("can't reach") || since.to_lowercase().contains("could not get next to"))
        {
            let tries = self.walks_given_up.entry(format!("{map}|field|{step}")).or_default();
            *tries += 1;
            if *tries <= Self::RETRIES_PER_PLACE {
                self.at = self.at.min(step);
            }
        }
        if let Some((map, step, Some(gave_up))) = self.taking.clone() {
            self.taking = None;
            if gave_up && request.location().as_deref() == Some(map.as_str()) {
                self.at = self.at.min(step);
            }
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
                        .filter(|id| OverworldAction::names_a_sprite(id))
                        .filter(|id| !id.contains("Boulder") && !skip.iter().any(|s| id.ends_with(&format!(":{s}"))))
                        .find(|id| !chosen.contains(*id)).cloned();
                    if next.is_none() { self.at += 1; continue }
                    next
                }
                Step::Hunt { row, on, .. } if !on.is_empty() && request.location().as_deref() != Some(*on) =>
                    Intent::Enter(on).resolve(request)
                        .or_else(|| self.toward(&here, |_, to| to == *on))
                        .or_else(|| self.route(request, on)),
                Step::Hunt { row, .. } | Step::Train { row, .. } =>
                    rows.iter().map(|(id, _)| id).find(|id| id.ends_with(&format!(":{row}"))).cloned(),
                Step::Field(arguments) => {
                    self.field_sent = Some((map.clone(), self.at));
                    self.at += 1;
                    let mut arguments: serde_json::Value = serde_json::from_str(arguments).expect("a step's JSON");
                    arguments["summary"] = serde_json::json!("as planned");
                    return Reply::call("use_field_move", arguments);
                }
                Step::Buy(_) => {
                    // The mart turn answers it; an overworld turn in between means the mart never
                    // opened, as when the talk to the clerk was an answer the policy never used, so
                    // the talk is made again.
                    self.unresolved += 1;
                    if self.unresolved > Self::PATIENCE {
                        self.stuck(format!("step {} ({step:?}): no mart opened on {map}", self.at + 1));
                    }
                    if self.unresolved % 3 == 0 && matches!(self.at.checked_sub(1).and_then(|at| self.steps.get(at)), Some(Step::Talk(_))) {
                        let tries = self.walks_given_up.entry(format!("{map}|mart|{}", self.at)).or_default();
                        *tries += 1;
                        let tries = *tries;
                        if tries > Self::RETRIES_PER_PLACE {
                            self.stuck(format!("step {} ({step:?}): no mart opened on {map} after {tries} talks", self.at + 1));
                        }
                        self.at -= 1;
                        continue;
                    }
                    return Reply::Calls(vec![Call::wait(10)]);
                }
                Step::GoTo(target) => {
                    // Or passed through since the last turn: once the S.S. Anne has sailed, the dock
                    // walks the player back to Vermilion before anything is asked there.
                    let reached = format!("✓ reached the warp to {target}");
                    let passed = request.situation().split("### Since your last decision").nth(1)
                        .is_some_and(|since| since.match_indices(&reached).any(|(at, _)|
                            !since[at + reached.len()..].starts_with(|c: char| c.is_ascii_alphanumeric())));
                    if request.location().as_deref() == Some(*target) || passed { self.at += 1; continue }
                    enter_toward(request, target)
                        .or_else(|| self.toward(&here, |_, to| to == *target))
                        .or_else(|| self.route(request, target))
                        // Walled in with nothing known beyond: a way on within this map, such as a
                        // tree, not taken from here before, and failing that any way out, as a phase
                        // that opens where the last one's exploring left off knows no map yet.
                        .or_else(|| self.pockets.get(&here).and_then(|pocket| {
                            let untaken = |(way, _, _): &&(String, String, String)|
                                !self.pocket_edges.contains_key(&(here.clone(), way.clone()));
                            pocket.exits.iter().filter(untaken).find(|(_, _, to)| *to == pocket.map)
                                .or_else(|| pocket.exits.iter().find(untaken))
                                .map(|(_, id, _)| id.clone())
                        }))
                }
                Step::Enter { map: target, landing } => {
                    if request.location().as_deref() == Some(*target) && entered_at(request.situation()) == Some(*landing) {
                        self.at += 1;
                        continue
                    }
                    let door = format!("warp to {target}, arriving at ({}, {})", landing.0, landing.1);
                    self.toward(&here, |way, _| way.ends_with(&door))
                }
                Step::Cross { map: target, landing } => {
                    if request.location().as_deref() == Some(*target) { self.at += 1; continue }
                    crossing_toward(request, target, *landing)
                }
                Step::Explore { maps, patience } => {
                    /// Turns an exploring may go without seeing anywhere new or taking anything.
                    /// A pocket is known by the exits it offers, so a map that offers different
                    /// ones on a later visit — the Safari gate lets you in or lets you leave — looks
                    /// like somewhere never stood in, and the search for what is left in it walks
                    /// out and back in for ever. Standing still is the fault, not repetition.
                    const IDLE: usize = 40;
                    self.exploring += 1;
                    self.explore_idle += 1;
                    match self.explore(&here, maps) {
                        Some(id) if self.exploring < *patience && self.explore_idle < IDLE => Some(id),
                        // Nothing reachable from here while something known is unfinished is
                        // people standing in the way: a pocket is known by the passages it offers,
                        // so one they block reads as a pocket never seen, joined to nothing.
                        None if self.explore_idle < IDLE && self.pockets.iter().any(|(key, pocket)|
                            maps.contains(&pocket.map.as_str()) && (self.has_things(pocket) || self.has_untaken_exit(pocket, key, maps))) =>
                            return Reply::Calls(vec![Call::wait(UNRESOLVED_TICKS)]),
                        _ => {
                            self.exploring = 0;
                            self.explore_idle = 0;
                            self.explored_maps.clear();
                            self.explore_cut.clear();
                            self.at += 1;
                            continue
                        }
                    }
                }
                Step::Wait => { self.at += 1; return Reply::Calls(vec![Call::wait(1)]) }
                Step::Collect(on) => { self.collecting = *on; self.at += 1; continue }
                Step::Safari => {
                    if self.safari_seen.contains(&Way::SafariOutOfSteps) {
                        self.at += 1;
                        continue;
                    }
                    rows.iter().map(|(id, _)| id).find(|id| id.ends_with(":Grass")).cloned()
                }
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
                        self.saw(if *item == "RareCandy" { Way::EvolvedByRareCandy } else { Way::EvolvedByStone });
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
                // Reached partway through a turn, after a step that needed no row of its own.
                Step::KeepFromEvolving(species) => {
                    let line = |slot: u8| text.split("### Party").nth(1).and_then(|party| party.lines()
                        .find(|line| line.starts_with(&format!("{slot}. "))).map(str::to_string));
                    let level = |line: &str| line.split(" Lv").nth(1)
                        .and_then(|rest| rest.split_whitespace().next()).and_then(|n| n.parse::<u8>().ok());
                    let kind = |line: &str| line.split(" Lv").next()
                        .and_then(|name| name.rsplit(" the ").next()).map(str::to_string);
                    if let Some((slot, before)) = self.kept_from_evolving {
                        let now = line(slot).unwrap_or_default();
                        if !kind(&now).is_some_and(|kind| same_species(&kind, species))
                            || !level(&now).is_some_and(|level| level > before)
                        {
                            self.stuck(format!("step {}: the {species} in slot {slot} was not kept from evolving: {now}", self.at + 1));
                            return Reply::Calls(vec![Call::wait(10)]);
                        }
                        self.kept_from_evolving = None;
                        self.saw(Way::EvolutionCancelled);
                        self.at += 1;
                        continue;
                    }
                    let Some(slot) = party_slot_of(&text, species) else {
                        self.stuck(format!("step {}: no {species} in the party to feed", self.at + 1));
                        return Reply::Calls(vec![Call::wait(10)]);
                    };
                    self.kept_from_evolving = Some((slot, line(slot).and_then(|line| level(&line)).unwrap_or(0)));
                    return Reply::call("use_field_move", serde_json::json!({
                        "move": "use_item", "item": "RareCandy", "slot": slot, "evolve": false, "summary": "a level, not an evolution" }));
                }
                Step::Tidy => {
                    self.tidying.get_or_insert(None);
                    self.at += 1;
                    match self.tidy(request) {
                        Some(reply) => return reply,
                        None => continue,
                    }
                }
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
                        | Step::TrashCans | Step::Coins(_) | Step::Safari | Step::Cross { .. } | Step::Enter { .. } => {}
                        Step::Trade(_) => self.at += 1,
                        // A hop toward the row is not the row.
                        Step::Take(fragment) => {
                            if rows.iter().any(|(row, what)| *row == id && what.contains(fragment)) {
                                // A row with a square in its id: a passage, a push, a tree.
                                if id.matches(':').count() == 2 {
                                    self.taking = Some((map.clone(), self.at, None));
                                }
                                self.at += 1;
                            }
                        }
                        Step::Gift(_) => { self.came = Came::Given; self.at += 1 }
                        // A statue's press is the Mansion's doors: one lost to a battle on the way
                        // leaves every floor the wrong way round.
                        Step::Talk(_) => {
                            self.taking = Some((map.clone(), self.at, None));
                            self.at += 1;
                        }
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
                    Reply::Calls(vec![Call::wait(UNRESOLVED_TICKS)])
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
        // A quiz machine answered wrong is a battle at once, so a quiz row followed by anything
        // else is the answer its gate wanted. Nothing left in RAM tells the two apart: the gate
        // flag is set by the right answer and by beating the trainer behind it alike.
        if std::mem::take(&mut self.quiz_pending) && !request.is_battle() {
            self.saw_entry(Entry::CinnabarQuiz);
        }
        // The Safari game's own words for what it did, which only the moment shows.
        if request.location().is_some_and(|map| map.starts_with("SafariZone")) {
            let said = request.situation();
            for (words, way) in [("some BAIT", Way::SafariBait), ("a ROCK", Way::SafariRock),
                                 ("Got away safely", Way::SafariRun), ("Time's up", Way::SafariOutOfSteps)] {
                if said.contains(words) {
                    self.saw(way);
                    // A hunt before the step can run a game out of steps, and the step then
                    // plays a game of its own.
                    if matches!(self.steps.get(self.at), Some(Step::Safari)) {
                        self.safari_seen.insert(way);
                    }
                }
            }
        }
        // Whatever kind of turn it arrives on: a walk resumed after a battle asks nothing between.
        if let Some(Step::Train { until, way, .. }) = self.steps.get(self.at).cloned()
            && request.situation().contains(until)
        {
            self.saw(way);
            self.at += 1;
        }
        // Before a battle's turn returns: a walk a wild battle cut short is said on that turn. The
        // latest outcome is the verdict, since a walk cut short can be taken up again after the
        // battle and finish; a first turn with neither an outcome nor a walk begun is a choice the
        // policy never carried out.
        // A row whose id the game moved on from is refused before any walk begins, and the turn
        // that says so has nothing since the last decision to show for it.
        if let Some((_, _, verdict)) = self.taking.as_mut()
            && verdict.is_none() && request.situation().contains("` is no longer available")
        {
            *verdict = Some(true);
        }
        if let Some((_, _, verdict)) = self.taking.as_mut()
            && let Some(since) = request.situation().split("### Since your last decision").nth(1)
        {
            match since.lines().rev().find(|line| line.starts_with("- ✓") || line.starts_with("- ✗ gave up on")) {
                Some(last) => *verdict = Some(last.starts_with("- ✗")),
                None if verdict.is_none() && !since.lines().any(|line| line.starts_with("- → heading for")) =>
                    *verdict = Some(true),
                None => {}
            }
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
        if request.has_tool("mimic_move") {
            return Reply::call("mimic_move", serde_json::json!({ "slot": 0, "summary": "copying the first" }));
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
/// The brain, handed the next phase's steps as each finishes, with a flag the driver reads to know
/// every step of the last has been carried out.
pub struct FinishFlag {
    brain: CompletionBrain,
    phases: VecDeque<Vec<Step>>,
    phase: usize,
    /// The steps of the phases already finished.
    steps_before: usize,
    progress: TourProgress,
}

/// What a driver reads while a tour plays.
#[derive(Clone)]
pub struct TourProgress {
    pub ledger: Arc<Mutex<Ledger>>,
    /// Why the run cannot go on, once it cannot.
    pub stuck: Arc<Mutex<Option<String>>>,
    /// Requests the brain has answered.
    pub turns: Arc<Mutex<usize>>,
    /// Every step of the last phase has been carried out.
    pub finished: Arc<Mutex<bool>>,
    /// Every chosen id the policy could not find again, by kind.
    pub unresolved: Arc<Mutex<BTreeMap<String, usize>>>,
    /// Steps taken across every phase, of [`Self::total`]: counted when the brain answers with the
    /// step's last action, which the game carries out after. It can go back, when a walk given up
    /// on puts the brain back on the step it was for.
    pub steps_done: Arc<AtomicUsize>,
    pub total: usize,
}

impl TourProgress {
    /// Finished, or stuck.
    pub fn is_over(&self) -> bool {
        *self.finished.lock().expect("not poisoned") || self.stuck.lock().expect("not poisoned").is_some()
    }
}

impl FinishFlag {
    /// The brain for `phases`, played back to back, and the handles a driver reads. The ledger
    /// starts empty: the driver sets it from the game's [`checklist`](crate::tour::completion::checklist).
    pub fn new(phases: Vec<Vec<Step>>) -> (Self, TourProgress) {
        let ledger = Arc::new(Mutex::new(Ledger::default()));
        let total: usize = phases.iter().map(Vec::len).sum();
        let mut phases: VecDeque<Vec<Step>> = phases.into();
        let brain = CompletionBrain::new(phases.pop_front().expect("at least one phase"), Arc::clone(&ledger));
        let progress = TourProgress {
            ledger,
            stuck: Arc::clone(&brain.stuck),
            turns: Arc::clone(&brain.turns),
            finished: Arc::new(Mutex::new(false)),
            unresolved: Arc::new(Mutex::new(BTreeMap::new())),
            steps_done: Arc::new(AtomicUsize::new(0)),
            total,
        };
        (Self { brain, phases, phase: 1, steps_before: 0, progress: progress.clone() }, progress)
    }
}

/// The kind of every id the policy refused as gone, counted, since each one is a paid turn to a
/// model: an id is meant to outlive the turn it was offered on.
pub fn print_unresolved(name: &str, unresolved: &Mutex<BTreeMap<String, usize>>) {
    let unresolved = unresolved.lock().expect("not poisoned");
    let rows: Vec<String> = unresolved.iter().map(|(kind, n)| format!("{kind} {n}")).collect();
    println!("[completion:{name}] {} ids no longer available: {}",
             unresolved.values().sum::<usize>(), rows.join(", "));
}

impl Brain for FinishFlag {
    fn respond(&mut self, request: &TurnRequest) -> Reply {
        // Once a turn: the note heads the situation, and a turn's later tool steps repeat it.
        if request.messages.last().is_some_and(|message| message.role == "user") {
            let situation = request.situation();
            let gone = situation.split_once("` is no longer available").map(|(id, _)| (id, "gone"))
                .or_else(|| situation.split_once("` is an id for `").map(|(id, _)| (id, "another map")));
            if let Some((id, why)) = gone
                && let Some(id) = id.rsplit('`').next()
            {
                println!("[completion] unresolved ({why}): {id}");
                let kind = match why {
                    "gone" => id.rsplit(':').next().unwrap_or(id).trim_end_matches(char::is_numeric).to_string(),
                    _ => why.to_string(),
                };
                *self.progress.unresolved.lock().expect("not poisoned").entry(kind).or_default() += 1;
            }
        }
        let before = self.brain.at;
        let was_finished = self.brain.finished();
        let reply = self.brain.respond(request);
        for step in before..self.brain.at.min(self.brain.steps.len()) {
            println!("[completion] step {} done on {}: {:?}", step + 1,
                     request.location().unwrap_or_default(), self.brain.steps[step]);
        }
        self.progress.steps_done.store(self.steps_before + self.brain.at.min(self.brain.steps.len()), Ordering::SeqCst);
        if self.brain.finished() {
            match self.phases.pop_front() {
                Some(steps) => {
                    self.phase += 1;
                    self.steps_before += self.brain.steps.len();
                    println!("[completion] phase {} of {}, from {}", self.phase, self.phase + self.phases.len(),
                             request.location().unwrap_or_default());
                    let mut next = CompletionBrain::new(steps, Arc::clone(&self.brain.ledger));
                    next.stuck = Arc::clone(&self.brain.stuck);
                    next.turns = Arc::clone(&self.brain.turns);
                    self.brain = next;
                }
                // The reply that takes the last step is carried out after this turn, so the run is
                // over at the next free turn: stopping here drops that step, a talk to an aide
                // included.
                None if was_finished && request.has_tool("choose_action") && !request.is_battle() =>
                    *self.progress.finished.lock().expect("not poisoned") = true,
                None => {}
            }
        }
        reply
    }
}
