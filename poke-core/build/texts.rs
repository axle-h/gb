//! Every text script in the disassembly, read from its source as the macros that write it.

use std::collections::BTreeMap;
use std::fmt::Write;
use super::asm::{Asm, Row};

/// The objects the Makefile assembles that hold text scripts.
const SOURCES: &[&str] = &["home.asm", "main.asm", "maps.asm", "text.asm"];

/// `TEXTS`, sorted by label: every label, global or `Parent.local`, whose first statement starts a
/// text script. A script that reaches the next label without ending runs on into it, as the
/// cartridge's does.
pub fn write(asm: &mut Asm) -> String {
    let rows: Vec<Row> = SOURCES.iter().flat_map(|file| asm.file(file)).collect();
    let labels = labels(&rows);
    let mut texts = BTreeMap::new();
    let mut dispatches = BTreeMap::new();
    for (at, label) in labels.iter().enumerate() {
        let Some(label) = label else { continue };
        let Some(first) = (at + 1..rows.len()).find(|&i| labels[i].is_none() && !rows[i].op.starts_with("vc_")) else { continue };
        if let Some(dispatch) = dispatch(asm, &rows[first]) {
            dispatches.insert(label.clone(), dispatch);
            continue;
        }
        if !starts_a_script(op(asm, &rows[first])) {
            continue;
        }
        let script = read(asm, &rows, &labels, first, label);
        if let Some(earlier) = texts.insert(label.clone(), script.clone()) {
            assert_eq!(earlier, script, "{label} is two different texts");
        }
    }
    let mut out = String::from("/// Every text script in the disassembly, by label, sorted.\n");
    let items: Vec<String> = texts.iter().map(|(label, script)| format!("({label:?}, &[{}])", script.join(", "))).collect();
    writeln!(out, "pub const TEXTS: &[(&str, &[TextMacro])] = &[{}];", items.join(",\n")).unwrap();
    let items: Vec<String> = dispatches.iter().map(|(label, dispatch)| format!("({label:?}, {dispatch})")).collect();
    writeln!(out, "/// Every text that is a `TX_SCRIPT_*` byte rather than a script, by label, sorted.").unwrap();
    writeln!(out, "pub const TEXT_DISPATCHES: &[(&str, TextDispatch)] = &[{}];", items.join(",\n")).unwrap();
    text_pointers(&rows, &labels, &mut out);
    trainer_headers(asm, &rows, &labels, &mut out);
    out
}

/// A `script_*` macro: the byte `DisplayTextID` dispatches on, and a mart's stock.
fn dispatch(asm: &Asm, row: &Row) -> Option<String> {
    Some(match row.op.as_str() {
        "script_mart" => format!("TextDispatch::Mart(&{:?})", row.args.iter().map(|item| asm.byte(item)).collect::<Vec<_>>()),
        "script_pokecenter_nurse" => "TextDispatch::PokecenterNurse".to_string(),
        "script_bills_pc" => "TextDispatch::BillsPc".to_string(),
        "script_players_pc" => "TextDispatch::PlayersPc".to_string(),
        "script_pokecenter_pc" => "TextDispatch::PokecenterPc".to_string(),
        "script_prize_vendor" => "TextDispatch::PrizeVendor".to_string(),
        "script_cable_club_receptionist" => "TextDispatch::CableClubReceptionist".to_string(),
        "script_vending_machine" => "TextDispatch::VendingMachine".to_string(),
        op => {
            assert!(!op.starts_with("script_"), "`{op}` is a text script this reader does not know");
            return None;
        }
    })
}

/// Every map's text pointer table, sorted by label, each the labels its `dw_const`s list, text id
/// 1 first, and a constant naming each. A table a script swaps in may be plain `dw`s, or begin as
/// `dw`s and go on with `const_def` and `dw_const`s.
fn text_pointers(rows: &[Row], labels: &[Option<String>], out: &mut String) {
    let mut tables = BTreeMap::new();
    for (at, label) in labels.iter().enumerate() {
        let Some(label) = label.as_ref().filter(|label| label.trim_end_matches(char::is_numeric).ends_with("_TextPointers")) else { continue };
        let mut first = at + 1;
        if rows[first].op == "def_text_pointers" {
            assert!(rows[first].args.is_empty(), "{label}: {:?}", rows[first]);
            first += 1;
        } else {
            assert_eq!(rows[first].op, "dw", "{label} is not a table");
        }
        let entries: Vec<&str> = rows[first..].iter().zip(&labels[first..])
            .take_while(|(row, label)| label.is_none() && matches!(row.op.as_str(), "dw" | "dw_const" | "const_def" | "EXPORT"))
            .filter(|(row, _)| matches!(row.op.as_str(), "dw" | "dw_const"))
            .map(|(row, _)| {
                assert_eq!(row.args.len(), if row.op == "dw" { 1 } else { 2 }, "{label}: {row:?}");
                row.args[0].as_str()
            })
            .collect();
        assert!(tables.insert(label, entries).is_none(), "{label} is two tables");
    }
    assert!(rows.iter().zip(labels).skip(1).zip(labels).all(|((row, _), before)| row.op != "def_text_pointers" || before.as_ref().is_some_and(|label| tables.contains_key(label))),
        "a `def_text_pointers` table is not labelled `_TextPointers`");
    let items: Vec<String> = tables.iter().map(|(label, entries)| format!("({label:?}, &{entries:?})")).collect();
    writeln!(out, "/// Every map's text pointer tables, by label, sorted: the text each id names, id 1 first.").unwrap();
    writeln!(out, "pub const TEXT_POINTERS: &[(&str, &[&str])] = &[{}];", items.join(",\n")).unwrap();
    let consts: Vec<String> = tables.keys().enumerate()
        .map(|(i, label)| format!("pub const {label}: TextPointers = TextPointers::new({i});"))
        .collect();
    writeln!(out, "/// Each of `TEXT_POINTERS` by its label.\n#[allow(non_upper_case_globals)]\npub mod text_pointers {{\n\
        use crate::map_objects::TextPointers;\n{}\n}}", consts.join("\n")).unwrap();
}

/// Every `def_trainers` table, as (the map whose script it follows, its label, its `trainer`s),
/// ending at its `db -1`, and a constant naming each table and each header.
fn trainer_headers(asm: &Asm, rows: &[Row], labels: &[Option<String>], out: &mut String) {
    let mut map = "";
    let mut tables = Vec::new();
    let mut names = std::collections::BTreeMap::new();
    for (at, row) in rows.iter().enumerate() {
        if let Some(script) = labels[at].as_deref().and_then(|label| label.strip_suffix("_Script")) {
            map = script;
        }
        if row.op != "def_trainers" {
            continue;
        }
        // A table with no label of its own starts at its first header's.
        let first = rows[at + 1..].iter().zip(&labels[at + 1..]).map_while(|(_, label)| label.as_deref()).next();
        let table = labels[at - 1].as_deref().or(first).expect("a table with no label");
        let mut sprite = row.args.first().map_or(1, |bit| asm.byte(bit));
        let mut holder = table;
        let mut trainers = Vec::new();
        for (row, label) in rows[at + 1..].iter().zip(&labels[at + 1..]) {
            if let Some(label) = label {
                holder = label;
                continue;
            }
            match (row.op.as_str(), row.args.as_slice()) {
                ("trainer", [event, range, before, end, after]) => {
                    assert!(names.insert(holder.to_string(), (tables.len(), trainers.len())).is_none(), "{holder} is two headers");
                    trainers.push(format!(
                        "Trainer {{ label: {holder:?}, sprite: {sprite}, event: {}, range: {}, before_battle: {before:?}, end_battle: {end:?}, after_battle: {after:?} }}",
                        asm.eval(event), asm.byte(range),
                    ));
                    sprite += 1;
                }
                ("db", [end]) if asm.byte(end) == 0xFF => break,
                _ => panic!("{table}: {row:?}"),
            }
        }
        names.entry(table.to_string()).or_insert((tables.len(), 0));
        tables.push(format!("({map:?}, {table:?}, &[{}])", trainers.join(", ")));
    }
    writeln!(out, "/// Every map's trainer headers, in the source's order, as (its map, the table's label, its trainers).").unwrap();
    writeln!(out, "pub const TRAINER_HEADERS: &[(&str, &str, &[Trainer])] = &[{}];", tables.join(",\n")).unwrap();
    let consts: Vec<String> = names.iter()
        .map(|(label, (table, trainer))| format!("pub const {label}: TrainerRef = TrainerRef::new({table}, {trainer});"))
        .collect();
    writeln!(out, "/// Each of `TRAINER_HEADERS`' tables, and each header in them, by its label.\n#[allow(non_upper_case_globals)]\n\
        pub mod trainers {{\nuse crate::trainer_headers::TrainerRef;\n{}\n}}", consts.join("\n")).unwrap();
}

/// Each row's label, if it is one, local labels qualified by the global label above them.
fn labels(rows: &[Row]) -> Vec<Option<String>> {
    let mut parent = String::new();
    rows.iter().map(|row| {
        let name = row.op.trim_end_matches(':');
        let is_label = row.op.ends_with(':') || (row.op.starts_with('.') && row.args.is_empty());
        if !is_label || name.is_empty() {
            return None;
        }
        Some(match name.strip_prefix('.') {
            Some(local) => format!("{parent}.{local}"),
            None => {
                if !name.contains('.') {
                    parent = name.to_string();
                }
                name.to_string()
            }
        })
    }).collect()
}

/// A row's op, an `EQUS` alias (`sound_level_up`) resolved.
fn op<'a>(asm: &'a Asm, row: &'a Row) -> &'a str {
    asm.equs(&row.op).unwrap_or(&row.op)
}

/// An op a script can open with; `line` and the rest continue a run already open.
fn starts_a_script(op: &str) -> bool {
    matches!(op, "text" | "text_start" | "text_ram" | "text_decimal" | "text_bcd" | "text_promptbutton" | "text_pause"
        | "text_low" | "text_waitbutton" | "text_scroll" | "text_dots" | "text_asm" | "text_far" | "text_end") || op.starts_with("sound_")
}

/// The script from row `first` to where the cartridge's would end: `text_end`, `text_asm`, or a
/// run ending in `done`, `prompt` or `dex`. `holder` is the label the rows are under.
fn read(asm: &Asm, rows: &[Row], labels: &[Option<String>], first: usize, holder: &str) -> Vec<String> {
    let mut holder = holder.to_string();
    let mut script = Vec::new();
    let mut run: Option<String> = None;
    for (at, row) in rows.iter().enumerate().skip(first) {
        if let Some(label) = &labels[at] {
            holder = label.clone();
            continue;
        }
        let op = op(asm, row);
        let open = |run: &Option<String>| assert!(run.is_some(), "{holder}: `{op}` outside a run");
        let closed = |run: &Option<String>| assert!(run.is_none(), "{holder}: `{op}` inside a run");
        let control = match op {
            "line" => Some("<LINE>"),
            "cont" => Some("<CONT>"),
            "para" => Some("<PARA>"),
            "next" => Some("<NEXT>"),
            "page" => Some("<PAGE>"),
            _ => None,
        };
        let ending = match op {
            "done" => Some("<DONE>"),
            "prompt" => Some("<PROMPT>"),
            "dex" => Some("<DEXEND>"),
            _ => None,
        };
        if let Some(code) = control {
            open(&run);
            run.as_mut().unwrap().push_str(code);
            push_strings(&holder, row, &mut run, &mut script);
            continue;
        }
        if let Some(code) = ending {
            open(&run);
            let mut text = run.take().unwrap();
            text.push_str(code);
            script.push(format!("TextMacro::Run({text:?})"));
            return script;
        }
        if op.starts_with("vc_") {
            continue;
        }
        match op {
            "text" | "text_start" => {
                closed(&run);
                run = Some(String::new());
                push_strings(&holder, row, &mut run, &mut script);
                continue;
            }
            _ => closed(&run),
        }
        let args = &row.args;
        script.push(match (op, args.len()) {
            ("text_end", 0) => return script,
            ("text_asm", 0) => {
                script.push(format!("TextMacro::Asm({holder:?})"));
                return script;
            }
            ("text_far", 1) => format!("TextMacro::Far({:?})", args[0]),
            ("text_ram", 1) => format!("TextMacro::Ram({:?})", args[0]),
            ("text_decimal", 3) => format!("TextMacro::Decimal {{ at: {:?}, bytes: {}, digits: {} }}", args[0], asm.byte(&args[1]), asm.byte(&args[2])),
            ("text_bcd", 2) => format!("TextMacro::Bcd {{ at: {:?}, flags: {} }}", args[0], asm.byte(&args[1])),
            ("text_promptbutton", 0) => "TextMacro::PromptButton".to_string(),
            ("text_pause", 0) => "TextMacro::Pause".to_string(),
            ("text_low", 0) => "TextMacro::Low".to_string(),
            ("text_waitbutton", 0) => "TextMacro::WaitButton".to_string(),
            ("text_scroll", 0) => "TextMacro::Scroll".to_string(),
            ("text_dots", 1) => format!("TextMacro::Dots({})", asm.byte(&args[0])),
            (sound, 0) if sound.starts_with("sound_") => format!("TextMacro::Sound({sound:?})"),
            _ => panic!("{holder}: `{row:?}` is not read as text"),
        });
    }
    panic!("{holder} runs off the end of the source")
}

/// A run's string arguments, appended; an `@` ends the run where it stands.
fn push_strings(holder: &str, row: &Row, run: &mut Option<String>, script: &mut Vec<String>) {
    for arg in &row.args {
        let text = arg.strip_prefix('"').and_then(|text| text.strip_suffix('"'))
            .unwrap_or_else(|| panic!("{holder}: `{arg}` is not a string"));
        assert!(!text.contains(['"', '\\']), "{holder}: `{arg}` is more than one string");
        let current = run.as_mut().unwrap_or_else(|| panic!("{holder}: `{arg}` after the run ended"));
        match text.split_once('@') {
            None => current.push_str(text),
            Some((before, "")) => {
                current.push_str(before);
                script.push(format!("TextMacro::Run({:?})", run.take().unwrap()));
            }
            Some(_) => panic!("{holder}: `{arg}` runs on past its `@`"),
        }
    }
}
