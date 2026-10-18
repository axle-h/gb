//! The model's filesystem. Every tool but `choose_action` reads, writes or edits one of these
//! paths, so the tool list never changes and a new kind of data is a new file. [`resolve`] is the
//! path table; [`Node::on_the_emulator_thread`] says who answers a read.

use serde_json::{Value, json};

use crate::llm::protocol::{ToolCall, ToolSpec};
use crate::llm::worker::ToolAnswer;
use crate::pokemon::GameState;
use crate::pokemon::map::Map;
use crate::pokemon::observe;
use crate::pokemon::world_graph::WorldGraph;

pub const READ_FILE: &str = "read_file";
pub const WRITE_FILE: &str = "write_file";
pub const EDIT_FILE: &str = "edit_file";

/// How many lines a read answers when it does not say.
pub const DEFAULT_LIMIT: usize = 400;

/// The tree, one line per file, as the system prompt lists it and an unknown path answers.
macro_rules! tree {
    () => {
        "\
map.png              the map picture, from the cartridge's own tiles
screen.png           the Game Boy screen as it is now
party.md             every party member: stats, types, moves, PP
bag.md               items with quantity and price, money, slots used
pc.md                the open box and PC item storage
guide/current.md     the walkthrough chapter for your badges
guide/<chapter>.md   every chapter
routes.md            the maps you have walked
routes/<Map>.md      the way from where you stand to <Map>; every map has one
plan.md              your plan; read, write, edit
battle-script.rhai   your battle script; a write or edit validates and arms it
battle-script.md     the script API
issues/<name>.md     write one to report a problem with the agent; each reads back
"
    };
}
pub(crate) use tree;

pub const TREE: &str = tree!();

pub const MAP_PNG: &str = "map.png";
pub const SCREEN_PNG: &str = "screen.png";
pub const PLAN: &str = "plan.md";
pub const BATTLE_SCRIPT: &str = "battle-script.rhai";
pub const BATTLE_SCRIPT_DOCS: &str = "battle-script.md";

/// One path of the tree, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Root,
    MapPng,
    ScreenPng,
    Party,
    Bag,
    Pc,
    GuideDir,
    GuideCurrent,
    Guide(usize),
    /// `routes.md`, and `routes/` read as a directory.
    Routes,
    Route(Map),
    /// `routes/<name>.md` where the name is no map.
    NoSuchMap(String),
    Plan,
    BattleScript,
    BattleScriptDocs,
    IssuesDir,
    /// `issues/<name>.md`, the name normalised to end in `.md`.
    Issue(String),
    Unknown(String),
}

impl Node {
    /// The path as the answer names it.
    pub fn path(&self) -> String {
        match self {
            Self::Root => "/".to_string(),
            Self::MapPng => MAP_PNG.to_string(),
            Self::ScreenPng => SCREEN_PNG.to_string(),
            Self::Party => "party.md".to_string(),
            Self::Bag => "bag.md".to_string(),
            Self::Pc => "pc.md".to_string(),
            Self::GuideDir => "guide/".to_string(),
            Self::GuideCurrent => "guide/current.md".to_string(),
            Self::Guide(index) => format!("guide/{}", crate::llm::guide::chapter_at(*index).map_or("", |(file, _)| file)),
            Self::Routes => "routes.md".to_string(),
            Self::Route(map) => format!("routes/{map}.md"),
            Self::NoSuchMap(name) => format!("routes/{name}"),
            Self::Plan => PLAN.to_string(),
            Self::BattleScript => BATTLE_SCRIPT.to_string(),
            Self::BattleScriptDocs => BATTLE_SCRIPT_DOCS.to_string(),
            Self::IssuesDir => "issues/".to_string(),
            Self::Issue(name) => format!("issues/{name}"),
            Self::Unknown(path) => path.clone(),
        }
    }

    /// Whether the read needs the game, and so goes to the policy at its next poll.
    pub fn on_the_emulator_thread(&self) -> bool {
        matches!(
            self,
            Self::MapPng | Self::Party | Self::Bag | Self::Pc | Self::GuideCurrent | Self::Guide(_)
                | Self::Routes | Self::Route(_)
        )
    }
}

/// Lower-cased and stripped of a leading `/` or `./`, so `/Party.MD` is `party.md`.
fn normalise(path: &str) -> String {
    let path = path.trim().trim_start_matches("./").trim_start_matches('/');
    path.to_ascii_lowercase()
}

/// `name` without a trailing `.md`, case-insensitively.
fn stem(name: &str) -> &str {
    match name.len().checked_sub(3).and_then(|at| Some(at).zip(name.get(at..))) {
        Some((at, tail)) if tail.eq_ignore_ascii_case(".md") => &name[..at],
        _ => name,
    }
}

/// The path table. Case and a missing `.md` are forgiven.
pub fn resolve(path: &str) -> Node {
    let raw = path.trim().trim_start_matches("./").trim_start_matches('/');
    let lower = normalise(path);
    match stem(&lower) {
        "" | "." => return Node::Root,
        "map.png" | "map" => return Node::MapPng,
        "screen.png" | "screen" => return Node::ScreenPng,
        "party" => return Node::Party,
        "bag" => return Node::Bag,
        "pc" => return Node::Pc,
        "plan" => return Node::Plan,
        "battle-script.rhai" => return Node::BattleScript,
        "battle-script" => return Node::BattleScriptDocs,
        "routes" | "routes/" => return Node::Routes,
        "guide" | "guide/" => return Node::GuideDir,
        "issues" | "issues/" => return Node::IssuesDir,
        _ => {}
    }
    if let Some(chapter) = lower.strip_prefix("guide/") {
        let chapter = stem(chapter);
        if chapter == "current" {
            return Node::GuideCurrent;
        }
        let found = crate::llm::guide::files().position(|file| {
            let file = stem(file);
            same_name(file, chapter) || file.split_once('-').is_some_and(|(_, badge)| same_name(badge, chapter))
        });
        return match found {
            Some(index) => Node::Guide(index),
            None => Node::Unknown(raw.to_string()),
        };
    }
    if lower.starts_with("routes/") {
        let name = stem(&raw["routes/".len()..]);
        return match crate::llm::tools::map_by_name(name) {
            Some(map) => Node::Route(map),
            None => Node::NoSuchMap(format!("{name}.md")),
        };
    }
    if lower.starts_with("issues/") {
        let name = stem(raw["issues/".len()..].trim());
        return match name.is_empty() || name.contains('/') {
            true => Node::Unknown(raw.to_string()),
            false => Node::Issue(format!("{name}.md")),
        };
    }
    Node::Unknown(raw.to_string())
}

fn same_name(a: &str, b: &str) -> bool {
    let squash = |name: &str| -> String {
        name.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
    };
    squash(a) == squash(b)
}

// ── The calls ────────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRead {
    pub node: Node,
    /// 1-based.
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

/// One file call, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileCall {
    Read(FileRead),
    Write { node: Node, content: String },
    Edit { node: Node, old: String, new: String },
}

/// Parse a `read_file`, `write_file` or `edit_file` call, or say what is wrong with it. `None` for
/// any other tool.
pub fn parse(call: &ToolCall) -> Option<Result<FileCall, String>> {
    let name = call.function.name.as_str();
    if ![READ_FILE, WRITE_FILE, EDIT_FILE].contains(&name) {
        return None;
    }
    let arguments = match call.arguments() {
        Ok(arguments) => arguments,
        Err(failure) => return Some(Err(format!("{failure}. Send the arguments as a JSON object and try again."))),
    };
    let text = |key: &str| arguments.get(key).and_then(Value::as_str).map(str::to_string);
    let Some(path) = text("path").filter(|path| !path.trim().is_empty()) else {
        return Some(Err(format!("`{name}` needs a `path`. The files are:\n{TREE}")));
    };
    let node = resolve(&path);
    let number = |key: &str| arguments.get(key).and_then(Value::as_u64).map(|n| n.min(u64::from(u32::MAX)) as usize);
    Some(match name {
        READ_FILE => Ok(FileCall::Read(FileRead { node, offset: number("offset"), limit: number("limit") })),
        WRITE_FILE => match text("content") {
            Some(content) => Ok(FileCall::Write { node, content }),
            None => Err("`write_file` needs a `content`: the whole file, as it is to be. An empty \
                         string empties it."
                .to_string()),
        },
        _ => match (text("old_string"), text("new_string")) {
            (Some(old), Some(new)) if old.is_empty() => {
                let _ = new;
                Err("`old_string` is empty. Copy the text to replace from a read of the file, or \
                     use `write_file` to replace the whole of it."
                    .to_string())
            }
            (Some(old), Some(new)) if old == new => {
                Err("`old_string` and `new_string` are the same, so that edit changes nothing.".to_string())
            }
            (Some(old), Some(new)) => Ok(FileCall::Edit { node, old, new }),
            _ => Err("`edit_file` needs an `old_string` and a `new_string`.".to_string()),
        },
    })
}

/// What a write to a file that takes none answers.
pub fn not_writable(node: &Node) -> String {
    let what = match node {
        Node::Unknown(_) => format!("There is no file `{}`.", node.path()),
        Node::Issue(_) => "An issue is filed by writing a new file under `issues/`; one already \
                           filed cannot be edited. Write another to add to it."
            .to_string(),
        Node::Root | Node::GuideDir | Node::IssuesDir | Node::Routes if node.path().ends_with('/') => {
            format!("`{}` is a directory.", node.path())
        }
        _ => format!("`{}` is read-only.", node.path()),
    };
    format!(
        "{what} Nothing was written. The writable files are `{PLAN}`, `{BATTLE_SCRIPT}`, and a new \
         file under `issues/`."
    )
}

/// `edit_file`'s rule: `old` must occur in `text` exactly once.
pub fn apply_edit(path: &str, text: &str, old: &str, new: &str) -> Result<String, String> {
    match text.matches(old).count() {
        1 => Ok(text.replacen(old, new, 1)),
        0 => Err(format!(
            "`old_string` does not occur in `{path}`, so nothing was changed. Read the file and copy \
             the text exactly, spaces and line breaks included."
        )),
        many => Err(format!(
            "`old_string` occurs {many} times in `{path}`, so nothing was changed. Include enough \
             of the text around it to pick out one."
        )),
    }
}

/// A text file's answer: the header line naming the file, the range and the total, then the lines.
pub fn window(path: &str, text: &str, offset: Option<usize>, limit: Option<usize>) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if total == 0 {
        return format!("{path}, empty");
    }
    let first = offset.unwrap_or(1).max(1);
    if first > total {
        return format!("{path} has {total} lines, so there is nothing from line {first}.");
    }
    let last = (first - 1 + limit.unwrap_or(DEFAULT_LIMIT).max(1)).min(total);
    let mut out = format!("{path}, lines {first}–{last} of {total}\n");
    for line in &lines[first - 1..last] {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// What reading an unknown path answers: the tree.
pub fn no_such_file(path: &str) -> String {
    format!("There is no file `{path}`. The files are:\n{TREE}")
}

/// `routes/<name>` where the name is no map: the closest names.
pub fn no_such_map(path: &str) -> String {
    use strum::IntoEnumIterator;
    let wanted = stem(path.trim_start_matches("routes/")).to_ascii_lowercase();
    let mut scored: Vec<(usize, String)> = Map::iter()
        .map(|map| map.to_string())
        .map(|name| (distance(&wanted, &name.to_ascii_lowercase()), name))
        .collect();
    scored.sort();
    let closest: Vec<String> = scored.into_iter().take(5).map(|(_, name)| format!("routes/{name}.md")).collect();
    format!(
        "There is no map called `{}` in this game. The closest names are {}. `routes.md` lists the \
         maps you have walked.",
        stem(path.trim_start_matches("routes/")),
        closest.join(", "),
    )
}

/// Levenshtein distance, plus a bonus for a name that contains the other: a fragment such as
/// `cerulean` should find `CeruleanCity` before `Route2`.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1).min(row[j] + 1).min(diagonal + usize::from(ca != cb));
            diagonal = above;
        }
    }
    let contained = !a.is_empty() && b.windows(a.len()).any(|window| window == a.as_slice());
    row[b.len()].saturating_sub(if contained { b.len() } else { 0 })
}

/// What reading `guide/` answers.
pub fn guide_listing() -> String {
    std::iter::once("guide/current.md".to_string())
        .chain(crate::llm::guide::files().map(|file| format!("guide/{file}")))
        .map(|line| line + "\n")
        .collect()
}

// ── The schemas ──────────────────────────────────────────────────────────────────────────────────

/// The three file tools, in the order they are sent.
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            READ_FILE,
            format!(
                "Read a file, or list a directory. Does NOT end the turn. The files are listed in \
                 the system prompt, and a path that is not one answers with the list. Text comes \
                 back a line at a time: `offset` is the first line to read, counting from 1, and \
                 `limit` how many (default {DEFAULT_LIMIT}); the answer's first line names the \
                 file, the range and how many lines it has. `map.png` and `screen.png` answer with \
                 a picture, attached to the message after the result."
            ),
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "A file from the list, as `party.md` or `routes/CeruleanCity.md`." },
                    "offset": { "type": "integer", "minimum": 1, "description": "The first line to read, from 1." },
                    "limit": { "type": "integer", "minimum": 1, "description": "How many lines to read." },
                },
                "required": ["path"],
                "additionalProperties": false,
            }),
        ),
        ToolSpec::new(
            WRITE_FILE,
            format!(
                "Replace a whole file with `content`. Does NOT end the turn. Writable: `{PLAN}`, \
                 `{BATTLE_SCRIPT}` (validated, then armed from your next battle decision; empty \
                 goes back to the default), and a new file under `issues/`, which files a report \
                 for a developer. Everything else is read-only."
            ),
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file to write." },
                    "content": { "type": "string", "description": "The whole file, as it is to be." },
                },
                "required": ["path", "content"],
                "additionalProperties": false,
            }),
        ),
        ToolSpec::new(
            EDIT_FILE,
            format!(
                "Replace `old_string` with `new_string` in `{PLAN}` or `{BATTLE_SCRIPT}`. Does NOT \
                 end the turn. `old_string` must occur in the file exactly once, copied exactly, \
                 or nothing is changed and the answer says why. An edit to the battle script is \
                 validated and armed as a write is."
            ),
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file to edit." },
                    "old_string": { "type": "string", "description": "The text to replace, exactly as it is in the file." },
                    "new_string": { "type": "string", "description": "What to put in its place." },
                },
                "required": ["path", "old_string", "new_string"],
                "additionalProperties": false,
            }),
        ),
    ]
}

// ── Reads the policy answers ─────────────────────────────────────────────────────────────────────

/// Whether this read hands over the chapter the badges pick, for the stale-guide note.
pub fn reads_current_guide(call: &ToolCall, badges: crate::pokemon::badge::Badge) -> bool {
    match parse(call) {
        Some(Ok(FileCall::Read(read))) => match read.node {
            Node::GuideCurrent => true,
            Node::Guide(index) => index == crate::llm::guide::chapter_index(badges),
            _ => false,
        },
        _ => false,
    }
}

/// Answer one read that needs the game, from what the policy holds at a poll. The text is the
/// whole file; the worker windows it.
pub fn service_read(
    call: &ToolCall,
    state: &GameState,
    readout: &dyn observe::Readout,
    graph: &WorldGraph,
) -> ToolAnswer {
    let node = match parse(call) {
        Some(Ok(FileCall::Read(read))) => read.node,
        _ => return ToolAnswer::text(format!("`{}` is not a read the agent answers.", call.function.name)),
    };
    match node {
        // A `MetaTileMap`, never pixels: the picture is drawn on the worker thread.
        Node::MapPng => ToolAnswer {
            json: map_text(state),
            map: Some(Box::new(state.map.clone())),
            is_dark: state.map_is_dark,
        },
        Node::Party => ToolAnswer::text(party_text(state)),
        Node::Bag => ToolAnswer::text(bag_text(state, readout)),
        Node::Pc => ToolAnswer::text(pc_text(state, readout)),
        Node::GuideCurrent => {
            let index = crate::llm::guide::chapter_index(state.badges);
            let (file, text) = crate::llm::guide::chapter_at(index).unwrap_or(("", ""));
            ToolAnswer::text(format!("This is guide/{file}, the chapter for your badges.\n\n{text}"))
        }
        Node::Guide(index) => ToolAnswer::text(crate::llm::guide::chapter_at(index).map_or("", |(_, text)| text)),
        Node::Routes => ToolAnswer::text(routes_text(state, graph)),
        Node::Route(to) => ToolAnswer::text(route_text(state, graph, to)),
        other => ToolAnswer::text(format!("`{}` is not a read the agent answers.", other.path())),
    }
}

pub(crate) fn party_text(state: &GameState) -> String {
    let party = observe::party(state);
    if party.is_empty() {
        return "(no Pokémon in the party)\n".to_string();
    }
    party
        .iter()
        .map(|mon| {
            let name = match &mon.nickname {
                Some(nickname) => format!("{nickname} ({})", mon.species),
                None => mon.species.clone(),
            };
            let moves: Vec<String> = mon
                .moves
                .iter()
                .map(|m| {
                    let power = m.power.map_or("no damage".to_string(), |power| format!("{power} power"));
                    format!("{} {}/{} PP ({}, {power}, {}% accuracy)", m.name, m.pp, m.max_pp, m.move_type, m.accuracy)
                })
                .collect();
            format!(
                "{}. {name} Lv{}, {}, {}/{} HP, {}{}; Attack {}, Defense {}, Speed {}, Special {}; moves: {}\n",
                mon.slot,
                mon.level,
                mon.types.join("/"),
                mon.hp,
                mon.max_hp,
                mon.status,
                if mon.fainted { ", fainted" } else { "" },
                mon.attack,
                mon.defense,
                mon.speed,
                mon.special,
                if moves.is_empty() { "none".to_string() } else { moves.join("; ") },
            )
        })
        .collect()
}

pub(crate) fn bag_text(state: &GameState, readout: &dyn observe::Readout) -> String {
    let bag = observe::bag(state, readout);
    let mut out = format!("Money: ¥{}\nSlots: {} of {} used\n", bag.money, bag.slots_used, bag.slots_total);
    for item in &bag.items {
        out.push_str(&match item.price {
            Some(price) => format!("{} ×{}, ¥{price} each in a mart\n", item.item, item.quantity),
            None => format!("{} ×{}, no mart sells it\n", item.item, item.quantity),
        });
    }
    if bag.items.is_empty() {
        out.push_str("(the bag is empty)\n");
    }
    out
}

pub(crate) fn pc_text(state: &GameState, readout: &dyn observe::Readout) -> String {
    let pc = observe::pc(state, readout);
    let mut out = format!(
        "Box {} of {} is open, {} of {} slots used. Only the open box can be read.\nParty: {} of 6\n",
        pc.open_box, pc.boxes_total, pc.slots_used, pc.slots_total, pc.party_size,
    );
    for mon in &pc.pokemon {
        out.push_str(&format!(
            "box slot {}: {} Lv{} ({}), {} HP; moves: {}\n",
            mon.box_slot,
            mon.species,
            mon.level,
            mon.nickname,
            mon.hp,
            if mon.moves.is_empty() { "none".to_string() } else { mon.moves.join(", ") },
        ));
    }
    if pc.pokemon.is_empty() {
        out.push_str("(the open box is empty)\n");
    }
    for item in &pc.stored_items {
        out.push_str(&format!("stored item: {item}\n"));
    }
    if pc.stored_items.is_empty() {
        out.push_str("(PC item storage is empty)\n");
    }
    out
}

/// What rides beside the map picture: everyone on it and the warps, as data.
fn map_text(state: &GameState) -> String {
    let view = observe::map_view(state);
    let mut out = format!(
        "{}, {} by {} squares; you are at ({}, {}) facing {}{}\n",
        view.map,
        view.width,
        view.height,
        view.position.x,
        view.position.y,
        view.facing,
        if view.is_dark { "; it is dark" } else { "" },
    );
    for person in &view.people {
        out.push_str(&format!(
            "{} at ({}, {}){}\n",
            person.name,
            person.position.x,
            person.position.y,
            if person.on_screen { ", on screen" } else { "" },
        ));
    }
    for warp in &view.warps {
        out.push_str(&format!(
            "warp at ({}, {}) to {} ({}, {}){}\n",
            warp.at.x,
            warp.at.y,
            warp.to_map,
            warp.to_position.x,
            warp.to_position.y,
            if warp.reachable_from_here { "" } else { ", not reachable from here" },
        ));
    }
    for connection in &view.connections {
        out.push_str(&format!("edge to {connection}\n"));
    }
    out
}

pub(crate) fn routes_text(state: &GameState, graph: &WorldGraph) -> String {
    let mut out = format!("You are on {}. The maps you have walked, each with its route file:\n", state.map.map);
    for map in observe::known_maps(graph) {
        out.push_str(&format!("{map}: routes/{map}.md\n"));
    }
    out
}

/// `routes/<Map>.md`, from the map-header graph: no route means not walked yet, never unreachable.
pub(crate) fn route_text(state: &GameState, graph: &WorldGraph, to: Map) -> String {
    let here = state.map.map;
    let Some(hops) = observe::route_from(graph, &state.map, to) else {
        return format!(
            "You have not been to {to}, or no route to it crosses ground you have already walked. \
             You will have to explore towards it. `routes.md` lists where you have been.\n"
        );
    };
    if hops.len() <= 1 {
        return format!("You are on {to} already.\n");
    }
    let mut out = format!("From {here} to {to}, as walked so far:\n");
    for (index, hop) in hops.iter().enumerate().skip(1) {
        let previous = &hops[index - 1].map;
        out.push_str(&match &hop.via {
            Some(via) => format!("{index}. {}, leaving {previous} by the {via}\n", hop.map),
            None => format!("{index}. {}\n", hop.map),
        });
    }
    // The graph knows which maps touch, not which parts of a map do, so say so.
    if let Some(unreachable) = hops.get(1).filter(|hop| hop.reachable_from_here == Some(false)) {
        let blockers = state.map.boundary_blockers();
        out.push_str(&format!(
            "\nThis route is correct and you cannot start it from where you are standing. It leaves \
             {here} by `{}`, and that square is not reachable on foot from here{}. The route graph \
             is built from the game's own map headers, which say which maps touch each other, not \
             which parts of a map touch each other; {here} is split, and you are on the wrong part \
             of it. What joins two parts of one map is usually a door: take a warp you can reach \
             and look for a back exit, or leave by an edge you can reach and come back onto {here} \
             by a different one. The action menu is the list of what you can actually get to.\n",
            unreachable.via.as_deref().unwrap_or("that connection"),
            match blockers.split_first() {
                Some((first, _)) => format!(", and what your side of it ends on is {first}"),
                None => String::new(),
            },
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_path_in_the_tree_resolves_and_case_and_a_missing_md_are_forgiven() {
        for (path, node) in [
            ("map.png", Node::MapPng),
            ("screen.png", Node::ScreenPng),
            ("party.md", Node::Party),
            ("/Party", Node::Party),
            ("./BAG.MD", Node::Bag),
            ("pc.md", Node::Pc),
            ("guide/current.md", Node::GuideCurrent),
            ("guide", Node::GuideDir),
            ("guide/00-boulder-badge.md", Node::Guide(0)),
            ("guide/elite-four", Node::Guide(8)),
            ("routes.md", Node::Routes),
            ("routes/", Node::Routes),
            ("routes/CeruleanCity.md", Node::Route(Map::CeruleanCity)),
            ("routes/ceruleancity", Node::Route(Map::CeruleanCity)),
            ("plan.md", Node::Plan),
            ("battle-script.rhai", Node::BattleScript),
            ("battle-script.md", Node::BattleScriptDocs),
            ("issues/", Node::IssuesDir),
            ("issues/menu-missing-door.md", Node::Issue("menu-missing-door.md".to_string())),
            ("issues/menu-missing-door", Node::Issue("menu-missing-door.md".to_string())),
            ("", Node::Root),
            ("/", Node::Root),
        ] {
            assert_eq!(resolve(path), node, "`{path}`");
        }
        assert!(matches!(resolve("routes/Cerulean.md"), Node::NoSuchMap(_)));
        assert!(matches!(resolve("battle.md"), Node::Unknown(_)), "there is no battle file");
        assert!(matches!(resolve("guide/99-nowhere.md"), Node::Unknown(_)));

        // Every tree line names a path that resolves to something.
        for line in TREE.lines() {
            let path = line.split_whitespace().next().expect("a path");
            let path = path.replace("<chapter>", "01-cascade-badge").replace("<Map>", "PewterCity").replace("<name>", "x");
            assert!(!matches!(resolve(&path), Node::Unknown(_)), "`{path}` from the tree does not resolve");
        }
    }

    #[test]
    fn a_read_names_the_file_the_range_and_the_total_and_counts_from_one() {
        let text: String = (1..=10).map(|n| format!("line {n}\n")).collect();
        let whole = window("plan.md", &text, None, None);
        assert!(whole.starts_with("plan.md, lines 1–10 of 10\nline 1\n"), "{whole}");
        let middle = window("plan.md", &text, Some(4), Some(2));
        assert_eq!(middle, "plan.md, lines 4–5 of 10\nline 4\nline 5\n");
        assert!(window("plan.md", &text, Some(11), None).contains("nothing from line 11"));
        assert_eq!(window("plan.md", "", None, None), "plan.md, empty");
        let long: String = (1..=(DEFAULT_LIMIT + 5)).map(|n| format!("{n}\n")).collect();
        assert!(window("x", &long, None, None).starts_with(&format!("x, lines 1–{DEFAULT_LIMIT} of {}", DEFAULT_LIMIT + 5)));
    }

    #[test]
    fn an_edit_must_find_its_text_exactly_once() {
        assert_eq!(apply_edit("plan.md", "1. a\n2. b\n", "2. b", "2. c").as_deref(), Ok("1. a\n2. c\n"));
        assert!(apply_edit("plan.md", "a a", "a", "b").expect_err("twice").contains("2 times"));
        assert!(apply_edit("plan.md", "a", "z", "b").expect_err("absent").contains("does not occur"));
    }

    #[test]
    fn a_name_that_is_no_map_answers_the_closest_names() {
        let answer = no_such_map("routes/Cerulean.md");
        assert!(answer.contains("routes/CeruleanCity.md"), "{answer}");
        assert!(answer.contains("routes.md"), "and points at the list: {answer}");
    }

    #[test]
    fn the_three_schemas_are_well_formed_and_never_change() {
        let specs = specs();
        assert_eq!(specs.iter().map(|spec| &*spec.function.name).collect::<Vec<&str>>(), [READ_FILE, WRITE_FILE, EDIT_FILE]);
        assert_eq!(serde_json::to_string(&specs).ok(), serde_json::to_string(&super::specs()).ok());
        for spec in &specs {
            assert_eq!(spec.function.parameters["type"], "object");
            assert_eq!(spec.function.parameters["additionalProperties"], false);
        }
    }
}
