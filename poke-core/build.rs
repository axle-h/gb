use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use regex::Regex;

#[path = "build/asm.rs"]
mod asm;
#[path = "build/audio.rs"]
mod audio;
#[path = "build/gfx.rs"]
mod gfx;
#[path = "build/tables.rs"]
mod tables;
#[path = "build/texts.rs"]
mod texts;

fn main() -> std::io::Result<()> {
    let out_dir = env::var("OUT_DIR").unwrap();
    let mut output = File::create(Path::new(&out_dir).join("constants.rs"))?;

    let mut asm = asm::Asm::load(Path::new("../vendor/pokered"));
    let mut script_consts = map_script_consts("../vendor/pokered/scripts")?;
    script_consts.extend(map_script_consts("../vendor/pokered/data/maps/objects")?);
    for (name, value) in &script_consts {
        asm.set(name, *value as i64);
    }
    let mut tables = tables::write(&mut asm);
    tables.push_str(&texts::write(&mut asm));
    std::fs::write(Path::new(&out_dir).join("tables.rs"), tables)?;
    std::fs::write(Path::new(&out_dir).join("audio.rs"), audio::write(&mut asm))?;

    // `wEventFlags` bit indices, and `wToggleableObjectFlags` ones.
    let mut consts = String::new();
    asm.write_consts(&mut consts, "pokered_events", "constants/event_constants.asm");
    asm.write_consts(&mut consts, "pokered_toggles", "constants/toggle_constants.asm");
    output.write_all(consts.as_bytes())?;

    writeln!(output, "#[allow(dead_code)]")?;
    writeln!(output, "pub mod pokered_map_scripts {{")?;
    for (name, value) in &script_consts {
        writeln!(output, "    pub const {name}: u8 = {value};")?;
    }
    writeln!(output, "}}")?;

    println!("cargo:rerun-if-changed=../vendor/pokered/scripts");
    println!("cargo:rerun-if-changed=../vendor/pokered/data/maps/objects");

    gfx::write_assets(Path::new(&out_dir), &mut asm)?;
    write_charmap(Path::new(&out_dir).join("charmap.rs"))
}

/// `constants/charmap.asm` as `(text, byte)` pairs, in file order, and `TEXT_OF`: each byte's
/// key under `gfx/font/font.png`, the glyph the English font draws there, or else its first key.
fn write_charmap(dest: std::path::PathBuf) -> std::io::Result<()> {
    const SOURCE: &str = "../vendor/pokered/constants/charmap.asm";
    const FONT_SECTION: &str = "(from gfx/font/font.png)";
    let entry = Regex::new(r#"^\s*charmap\s+"((?:[^"\\]|\\.)*)",\s*\$([0-9a-fA-F]{2})"#).unwrap();
    let mut output = File::create(dest)?;
    let mut text_of: [Option<String>; 256] = std::array::from_fn(|_| None);
    let (mut in_font, mut font_found) = (false, false);
    writeln!(output, "pub const CHARMAP: &[(&str, u8)] = &[")?;
    for line in BufReader::new(File::open(SOURCE)?).lines() {
        let line = line?;
        if line.starts_with(';') {
            in_font = line.contains(FONT_SECTION);
            font_found |= in_font;
        }
        if let Some(caps) = entry.captures(&line) {
            let text = caps[1].replace("\\\"", "\"");
            let byte = u8::from_str_radix(&caps[2], 16).unwrap() as usize;
            if in_font || text_of[byte].is_none() {
                text_of[byte] = Some(text.clone());
            }
            writeln!(output, "    ({:?}, 0x{}),", text, &caps[2])?;
        }
    }
    writeln!(output, "];")?;
    assert!(font_found, "no `{FONT_SECTION}` section in {SOURCE}");
    writeln!(output, "pub const TEXT_OF: [Option<&str>; 256] = {:?};", text_of)?;
    println!("cargo:rerun-if-changed={SOURCE}");
    Ok(())
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

/// The `TEXT_*` and `SCRIPT_*` indices every map script's `dw_const` tables count out, and the
/// object ids `const_export` counts in `data/maps/objects`: `def_text_pointers` and
/// `object_const_def` from 1, `def_script_pointers` from 0, `const_def n` from n.
fn map_script_consts(dir: &str) -> std::io::Result<Vec<(String, u32)>> {
    let mut files: Vec<_> = std::fs::read_dir(dir)?.map(|entry| entry.map(|e| e.path())).collect::<Result<_, _>>()?;
    files.retain(|path| path.extension().is_some_and(|ext| ext == "asm"));
    files.sort();
    let mut consts = Vec::new();
    for path in files {
        let mut next: u32 = 0;
        for line in std::fs::read_to_string(&path)?.lines() {
            let line = line.split(';').next().unwrap_or("").trim();
            let mut words = line.split_whitespace();
            match (words.next(), words.next()) {
                (Some("def_script_pointers"), _) => next = 0,
                (Some("def_text_pointers") | Some("object_const_def"), _) => next = 1,
                (Some("const_def"), value) => next = value.map_or(0, parse_number),
                (Some("dw_const"), Some(_)) => {
                    consts.push((line.rsplit(',').next().unwrap().trim().to_string(), next));
                    next += 1;
                }
                (Some("const_export"), Some(name)) => {
                    consts.push((name.to_string(), next));
                    next += 1;
                }
                _ => {}
            }
        }
    }
    Ok(consts)
}
