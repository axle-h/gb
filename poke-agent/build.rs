//! `pokered_symbols` and `pokered_local_labels` from `pokered.sym`: a constant per symbol the
//! assembled cartridge exports, so a symbol that moves or goes upstream is a compile error here.

use std::fmt::Write as _;
use std::path::Path;

const SYM: &str = "../vendor/pokered/pokered.sym";

fn main() {
    println!("cargo:rerun-if-changed={SYM}");
    let sym = std::fs::read_to_string(SYM).unwrap_or_else(|e| panic!("{SYM}: {e}; run `make -C vendor/pokered pokered.gbc`"));
    let mut globals = String::new();
    let mut parents: Vec<(&str, String)> = Vec::new();
    for line in sym.lines() {
        let Some((place, name)) = line.split_once(char::is_whitespace) else { continue };
        let name = name.trim();
        match place.split_once(':') {
            Some((bank, address)) if is_hex(bank, 2) && is_hex(address, 4) => {
                let (bank, address) = (u8::from_str_radix(bank, 16).unwrap(), u16::from_str_radix(address, 16).unwrap());
                match name.split_once('.') {
                    None if is_word(name) => {
                        writeln!(globals, "    pub const {name}: DmgPointer = {};", pointer(name, bank, address)).unwrap();
                    }
                    Some((parent, local)) if is_word(parent) && is_word(local) => {
                        let local = if RUST_KEYWORDS.contains(&local) { format!("r#{local}") } else { local.to_string() };
                        let item = format!("        pub const {local}: DmgPointer = {};\n", pointer(parent, bank, address));
                        match parents.iter_mut().find(|(name, _)| *name == parent) {
                            Some((_, items)) => items.push_str(&item),
                            None => parents.push((parent, item)),
                        }
                    }
                    _ => {}
                }
            }
            None if is_hex(place, 2) && is_word(name) => {
                writeln!(globals, "    pub const {name}: u8 = 0x{place};").unwrap();
            }
            _ => {}
        }
    }

    let mut out = String::from("#[allow(non_upper_case_globals, dead_code)]\npub mod pokered_symbols {\n    use poke_core::pointer::{DmgBank, DmgPointer};\n");
    out.push_str(&globals);
    out.push_str("}\n\n#[allow(non_upper_case_globals, non_snake_case, dead_code)]\npub mod pokered_local_labels {\n");
    for (parent, items) in parents {
        writeln!(out, "    pub mod {parent} {{\n        use poke_core::pointer::{{DmgBank, DmgPointer}};\n{items}    }}").unwrap();
    }
    out.push_str("}\n");
    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("pokered_symbols.rs"), out).unwrap();
}

/// The memory a symbol is in, which its prefix names: `w` WRAM, `s` SRAM, `v` VRAM, `h` HRAM, and
/// anything else ROM.
fn pointer(name: &str, bank: u8, address: u16) -> String {
    let bank = match name.as_bytes()[0] {
        b'w' => "DmgBank::WRAM".to_string(),
        b's' => format!("DmgBank::SRAM {{ bank: 0x{bank:02X} }}"),
        b'v' => "DmgBank::VRAM".to_string(),
        b'h' => "DmgBank::HRAM".to_string(),
        _ => format!("DmgBank::ROM {{ bank: 0x{bank:02X} }}"),
    };
    format!("DmgPointer {{ bank: {bank}, address: 0x{address:04X} }}")
}

fn is_hex(text: &str, digits: usize) -> bool {
    text.len() == digits && text.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_word(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_alphanumeric() || c == '_')
}

const RUST_KEYWORDS: &[&str] = &["as", "break", "const", "continue", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static", "struct",
    "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box",
    "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen"];
