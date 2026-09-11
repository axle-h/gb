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
    writeln!(output, "}}")?;
    writeln!(output, "")?;

    // `wEventFlags` bit indices, and `wToggleableObjectFlags` ones.
    write_consts(&mut output, "pokered_events", "../vendor/pokered/constants/event_constants.asm")?;
    write_consts(&mut output, "pokered_toggles", "../vendor/pokered/constants/toggle_constants.asm")?;

    println!("cargo:rerun-if-changed=../vendor/pokered/pokered.sym");
    println!("cargo:rerun-if-changed=../vendor/pokered/constants/event_constants.asm");
    println!("cargo:rerun-if-changed=../vendor/pokered/constants/toggle_constants.asm");

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
    for line in source.lines() {
        let line = line.split(';').next().unwrap_or("").trim();
        let mut words = line.split_whitespace();
        match (words.next(), words.next()) {
            (Some("const_def"), _) => next = 0,
            (Some("const"), Some(name)) => {
                writeln!(output, "    pub const {name}: u16 = {next};")?;
                next += 1;
            }
            (Some("const_skip"), count) => next += count.map_or(1, parse_number),
            (Some("const_next"), Some(value)) => next = parse_number(value),
            _ => {}
        }
    }
    writeln!(output, "}}")
}

fn parse_number(text: &str) -> u32 {
    match text.strip_prefix('$') {
        Some(hex) => u32::from_str_radix(hex, 16).unwrap(),
        None => text.parse().unwrap(),
    }
}
