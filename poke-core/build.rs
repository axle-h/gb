use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use regex::Regex;

fn main() -> std::io::Result<()> {
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("pokered_symbols.rs");
    let mut output = File::create(dest_path)?;

    let sym_file = File::open("../vendor/pokered/pokered.sym")?;
    let reader = BufReader::new(sym_file);

    let entry_regex = Regex::new(r"^([0-9a-fA-F]{2}):([0-9a-fA-F]{4})\s+(\w+)$").unwrap();
    let trainer_header_regex = Regex::new(r"^\w+TrainerHeader(\d+)$").unwrap();
    let mut trainer_headers = Vec::new();
    // The far-text bodies, which every `text_far` points at and which carry the text commands.
    let text_label_regex = Regex::new(r"^_\w*Text\w*$").unwrap();
    let mut text_labels = Vec::new();
    // A header's own label abbreviates its map (`Mansion4`); the map script it follows does not.
    let mut map_script = String::new();
    let const_regex = Regex::new(r"^([0-9a-fA-F]{2})\s+(\w+)$").unwrap();

    writeln!(output, "// Auto-generated from pokered.sym")?;
    writeln!(output, "")?;
    writeln!(output, "#[allow(non_upper_case_globals)]")?;
    writeln!(output, "#[allow(dead_code)]")?;
    writeln!(output, "pub mod pokered_symbols {{")?;
    writeln!(output, "    use super::{{DmgPointer, DmgBank}};")?;
    writeln!(output, "    use DmgBank::*;")?;

    for line in reader.lines() {
        let line = line?;
        if let Some(caps) = entry_regex.captures(&line) {
            let bank_id: u8 = u8::from_str_radix(&caps[1], 16).unwrap();
            let address: u16 = u16::from_str_radix(&caps[2], 16).unwrap();
            let name = &caps[3];

            if let Some(map) = name.strip_suffix("_Script") {
                map_script = map.to_string();
            }
            if text_label_regex.is_match(name) && (0x4000..0x8000).contains(&address) {
                text_labels.push(format!("(\"{name}\", {name})"));
            }
            if let Some(caps) = trainer_header_regex.captures(name) {
                trainer_headers.push(format!("(\"{map_script}\", {}, {name})", &caps[1]));
            }
            if let Some(bank_type) = infer_bank(name, bank_id, address) {
                writeln!(output, "    pub const {}: DmgPointer = DmgPointer {{ bank: {}, address: 0x{:04X} }};",
                         name, bank_type, address)?;
            }
        } else if let Some(caps) = const_regex.captures(&line) {
            let value: u8 = u8::from_str_radix(&caps[1], 16).unwrap();
            let name = &caps[2];
            writeln!(output, "    pub const {}: u8 = 0x{:02X};", name, value)?;
        }
    }

    // Every `trainer` header a map script declares, as (its map, its index, where).
    writeln!(output, "    pub const TRAINER_HEADERS: &[(&str, u8, DmgPointer)] = &[{}];", trainer_headers.join(", "))?;
    // Every far-text body, as (its label, where), so a sweep can decode all of them.
    writeln!(output, "    pub const TEXT_LABELS: &[(&str, DmgPointer)] = &[{}];", text_labels.join(", "))?;
    writeln!(output, "}}")?;
    writeln!(output, "")?;

    // `wEventFlags` bit indices, and `wToggleableObjectFlags` ones.
    write_consts(&mut output, "pokered_events", "../vendor/pokered/constants/event_constants.asm")?;
    write_consts(&mut output, "pokered_toggles", "../vendor/pokered/constants/toggle_constants.asm")?;

    write_local_labels(&mut output, "../vendor/pokered/pokered.sym")?;
    write_map_script_consts(&mut output, "../vendor/pokered/scripts")?;

    println!("cargo:rerun-if-changed=../vendor/pokered/pokered.sym");
    println!("cargo:rerun-if-changed=../vendor/pokered/scripts");
    println!("cargo:rerun-if-changed=../vendor/pokered/constants/event_constants.asm");
    println!("cargo:rerun-if-changed=../vendor/pokered/constants/toggle_constants.asm");

    write_charmap(Path::new(&out_dir).join("charmap.rs"))
}

/// `constants/charmap.asm` as `(text, byte)` pairs, in file order.
fn write_charmap(dest: std::path::PathBuf) -> std::io::Result<()> {
    const SOURCE: &str = "../vendor/pokered/constants/charmap.asm";
    let entry = Regex::new(r#"^\s*charmap\s+"((?:[^"\\]|\\.)*)",\s*\$([0-9a-fA-F]{2})"#).unwrap();
    let mut output = File::create(dest)?;
    writeln!(output, "pub const CHARMAP: &[(&str, u8)] = &[")?;
    for line in BufReader::new(File::open(SOURCE)?).lines() {
        let line = line?;
        if let Some(caps) = entry.captures(&line) {
            let text = caps[1].replace("\\\"", "\"");
            writeln!(output, "    ({:?}, 0x{}),", text, &caps[2])?;
        }
    }
    writeln!(output, "];")?;
    println!("cargo:rerun-if-changed={SOURCE}");
    Ok(())
}

fn infer_bank(name: &str, bank_id: u8, _address: u16) -> Option<String> {
    let first_char = name.chars().next()?;

    match first_char {
        'w' => Some("WRAM".to_string()),
        's' => Some(format!("SRAM {{ bank: 0x{:02X} }}", bank_id)),
        'v' => Some("VRAM".to_string()),
        'h' => Some("HRAM".to_string()),
        _ => Some(format!("ROM {{ bank: 0x{:02X} }}", bank_id)),
    }
}


/// A module of the indices an asm file's `const` sequence counts out.
fn write_consts(output: &mut File, module: &str, path: &str) -> std::io::Result<()> {
    let source = std::fs::read_to_string(path)?;
    writeln!(output, "#[allow(dead_code)]")?;
    writeln!(output, "pub mod {module} {{")?;
    let mut next: u32 = 0;
    let mut names = Vec::new();
    for line in source.lines() {
        let line = line.split(';').next().unwrap_or("").trim();
        let (directive, argument) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let argument = argument.trim();
        match directive {
            "const_def" => next = if argument.is_empty() { 0 } else { parse_number(argument) },
            "const" if !argument.is_empty() => {
                let name = argument.split_whitespace().next().unwrap();
                writeln!(output, "    pub const {name}: u16 = {next};")?;
                names.push(format!("(\"{name}\", {name})"));
                next += 1;
            }
            "const_skip" => next += if argument.is_empty() { 1 } else { parse_number(argument) },
            "const_next" => next = parse_number(argument),
            _ => {}
        }
    }
    writeln!(output, "    pub const NAMES: &[(&str, u16)] = &[{}];", names.join(", "))?;
    writeln!(output, "}}")
}

/// A `const_*` argument as rgbasm evaluates it: a sum of `$hex` or decimal terms, since the event
/// list writes one block's start as `$F0 - 2` and dropping the tail shifts every name after it.
fn parse_number(text: &str) -> u32 {
    let mut total: i64 = 0;
    let mut sign: i64 = 1;
    let mut term = String::new();
    let add = |term: &mut String, sign: i64, total: &mut i64| {
        let term = std::mem::take(term);
        let term = term.trim();
        if !term.is_empty() {
            *total += sign * term.strip_prefix('$').map_or_else(|| term.parse().unwrap(), |hex| i64::from_str_radix(hex, 16).unwrap());
        }
    };
    for character in text.chars() {
        match character {
            '+' | '-' => {
                add(&mut term, sign, &mut total);
                sign = if character == '-' { -1 } else { 1 };
            }
            _ => term.push(character),
        }
    }
    add(&mut term, sign, &mut total);
    u32::try_from(total).unwrap()
}

/// Every local label, `Parent.local` in `pokered.sym`, as `pokered_local_labels::Parent::local`: a
/// module per parent label, with a keyword spelled as a raw identifier (`r#loop`).
fn write_local_labels(output: &mut File, path: &str) -> std::io::Result<()> {
    let entry = Regex::new(r"^([0-9a-fA-F]{2}):([0-9a-fA-F]{4})\s+(\w+)\.(\w+)$").unwrap();
    let mut parents: Vec<(String, Vec<String>)> = Vec::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        let Some(caps) = entry.captures(&line) else { continue };
        let bank_id = u8::from_str_radix(&caps[1], 16).unwrap();
        let address = u16::from_str_radix(&caps[2], 16).unwrap();
        let (parent, local) = (&caps[3], &caps[4]);
        let bank = infer_bank(parent, bank_id, address).unwrap();
        let name = if RUST_KEYWORDS.contains(&local) { format!("r#{local}") } else { local.to_string() };
        let item = format!("        pub const {name}: DmgPointer = DmgPointer {{ bank: {bank}, address: 0x{address:04X} }};");
        match parents.iter_mut().find(|(name, _)| name == parent) {
            Some((_, items)) => items.push(item),
            None => parents.push((parent.to_string(), vec![item])),
        }
    }
    writeln!(output, "#[allow(non_upper_case_globals, non_snake_case, dead_code)]")?;
    writeln!(output, "pub mod pokered_local_labels {{")?;
    for (parent, items) in parents {
        writeln!(output, "    pub mod {parent} {{")?;
        writeln!(output, "        use crate::symbols::{{DmgPointer, DmgBank::*}};")?;
        for item in items {
            writeln!(output, "{item}")?;
        }
        writeln!(output, "    }}")?;
    }
    writeln!(output, "}}")
}

const RUST_KEYWORDS: &[&str] = &["as", "break", "const", "continue", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static", "struct",
    "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box",
    "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen"];

/// The `TEXT_*` and `SCRIPT_*` indices every map script's `dw_const` tables count out:
/// `def_text_pointers` from 1, `def_script_pointers` from 0, `const_def n` from n.
fn write_map_script_consts(output: &mut File, dir: &str) -> std::io::Result<()> {
    let mut files: Vec<_> = std::fs::read_dir(dir)?.map(|entry| entry.map(|e| e.path())).collect::<Result<_, _>>()?;
    files.retain(|path| path.extension().is_some_and(|ext| ext == "asm"));
    files.sort();
    writeln!(output, "#[allow(dead_code)]")?;
    writeln!(output, "pub mod pokered_map_scripts {{")?;
    for path in files {
        let mut next: u32 = 0;
        for line in std::fs::read_to_string(&path)?.lines() {
            let line = line.split(';').next().unwrap_or("").trim();
            let mut words = line.split_whitespace();
            match (words.next(), words.next()) {
                (Some("def_script_pointers"), _) => next = 0,
                (Some("def_text_pointers"), _) => next = 1,
                (Some("const_def"), value) => next = value.map_or(0, parse_number),
                (Some("dw_const"), Some(_)) => {
                    let name = line.rsplit(',').next().unwrap().trim();
                    writeln!(output, "    pub const {name}: u8 = {next};")?;
                    next += 1;
                }
                _ => {}
            }
        }
    }
    writeln!(output, "}}")
}
