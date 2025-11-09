use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use regex::Regex;

fn main() -> std::io::Result<()> {
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("pokered_symbols.rs");
    let mut output = File::create(dest_path)?;

    let sym_file = File::open("pokered/pokered.sym")?;
    let reader = BufReader::new(sym_file);

    let entry_regex = Regex::new(r"^([0-9a-fA-F]{2}):([0-9a-fA-F]{4})\s+(\w+)$").unwrap();
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

    writeln!(output, "}}")?;
    writeln!(output, "")?;

    println!("cargo:rerun-if-changed=pokered/pokered.sym");

    Ok(())
}

fn infer_bank(name: &str, bank_id: u8, address: u16) -> Option<String> {
    let first_char = name.chars().next()?;

    match first_char {
        'w' => Some("WRAM".to_string()),
        's' => Some(format!("SRAM {{ bank: 0x{:02X} }}", bank_id)),
        'v' => Some("VRAM".to_string()),
        'h' => Some("HRAM".to_string()),
        _ => Some(format!("ROM {{ bank: 0x{:02X} }}", bank_id)),
    }
}
