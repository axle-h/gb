//! The committed `.png` sources converted to the tile data the cartridge `INCBIN`s, as `rgbgfx
//! --colors dmg` and then `tools/gfx` produce it, with the per-file options the Makefile applies,
//! and the committed binaries it `INCBIN`s as they are.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

use super::asm::Asm;

const ROOT: &str = "../vendor/pokered";
/// Every directory holding an `INCBIN` of a picture, plus the top-level `.asm`.
const ASM_DIRS: &[&str] = &["data", "engine", "gfx", "home"];

#[derive(Default, Debug)]
struct Options {
    columns: bool,
    trim_whitespace: bool,
    remove_duplicates: bool,
    interleave: bool,
    preserved: Vec<usize>,
}

/// Writes `<out_dir>/gfx/...` and `<out_dir>/gfx.rs`.
pub fn write_assets(out_dir: &Path, asm: &mut Asm) -> std::io::Result<()> {
    let root = Path::new(ROOT);
    let targets = incbin_targets(root)?;
    let rules = makefile_rules(&fs::read_to_string(root.join("Makefile"))?);

    let mut entries = Vec::new();
    for (target, depth) in &targets {
        let bytes = match depth {
            Depth::Verbatim => fs::read(root.join(target))?,
            Depth::Bpp(depth) => {
                let stem = target.rsplit_once('.').unwrap().0;
                let options = options_for(&rules, target);
                let png = root.join(format!("{stem}.png"));
                convert(&fs::read(&png)?, *depth, &options).unwrap_or_else(|e| panic!("{}: {e}", png.display()))
            }
        };
        let dest = out_dir.join(target);
        fs::create_dir_all(dest.parent().unwrap())?;
        if fs::read(&dest).ok().as_deref() != Some(&bytes[..]) {
            fs::write(&dest, &bytes)?;
        }
        entries.push((target.clone(), bytes.len()));
    }
    let mut source = rust_source(&entries);
    let labels = incbin_labels(root)?;
    sprite_sheets(&mut source, asm, &labels);
    pics(&mut source, asm, &labels);
    tile_id_lists(&mut source, asm, &labels);
    sgb_border_palettes(&mut source, asm);
    sgb_packets(&mut source, asm);
    fishing(&mut source, asm, &labels);
    mon_icons(&mut source, asm, &labels)?;
    objects(&mut source, asm, "POKE_CENTER_OAM", "engine/overworld/healing_machine.asm", "PokeCenterOAMData");
    objects(&mut source, asm, "SMALL_STARS_OAM", "engine/movie/splash.asm", "SmallStarsOAM");
    objects(&mut source, asm, "GAME_FREAK_LOGO_OAM", "engine/movie/splash.asm", "GameFreakLogoOAMData");
    objects(&mut source, asm, "GAME_FREAK_SHOOTING_STAR_OAM", "engine/movie/splash.asm", "GameFreakShootingStarOAMData");
    fs::write(out_dir.join("gfx.rs"), source)?;

    println!("cargo:rerun-if-changed={ROOT}/Makefile");
    for dir in ASM_DIRS {
        println!("cargo:rerun-if-changed={ROOT}/{dir}");
    }
    for asm in top_level_asm(root)? {
        println!("cargo:rerun-if-changed={}", asm.display());
    }
    Ok(())
}

#[derive(Copy, Clone)]
enum Depth {
    Bpp(u8),
    /// Committed as the cartridge includes it.
    Verbatim,
}

/// What `make` builds, keyed by the path it writes: every `gfx/` picture the source `INCBIN`s,
/// with its bit depth, and every committed blockset, tilemap and run-length map. A `.pic` is
/// compressed from a `.2bpp`, which is what is generated for it.
fn incbin_targets(root: &Path) -> std::io::Result<BTreeMap<String, Depth>> {
    let incbin = Regex::new(r#"INCBIN\s+"(gfx/[^"]+)\.(1bpp|2bpp|pic|bst|tilemap|rle)""#).unwrap();
    let mut files = top_level_asm(root)?;
    for dir in ASM_DIRS {
        walk(&root.join(dir), &mut files)?;
    }
    let mut targets = BTreeMap::new();
    for file in files.iter().filter(|f| matches!(f.extension().and_then(|e| e.to_str()), Some("asm" | "inc"))) {
        for caps in incbin.captures_iter(&fs::read_to_string(file)?) {
            let (extension, depth) = match &caps[2] {
                verbatim @ ("bst" | "tilemap" | "rle") => (verbatim, Depth::Verbatim),
                "1bpp" => ("1bpp", Depth::Bpp(1)),
                _ => ("2bpp", Depth::Bpp(2)),
            };
            // A picture included at both depths is two assets of one name, which `rust_source` refuses.
            targets.insert(format!("{}.{extension}", &caps[1]), depth);
        }
    }
    Ok(targets)
}

/// A label to the asset its line `INCBIN`s whole, as `incbin_targets` keys it.
fn incbin_labels(root: &Path) -> std::io::Result<BTreeMap<String, String>> {
    let labelled = Regex::new(r#"(?m)^(\w+)::?\s*INCBIN\s+"(gfx/[^"]+)\.(1bpp|2bpp|pic|bst|tilemap|rle)"\s*$"#).unwrap();
    let mut files = top_level_asm(root)?;
    for dir in ASM_DIRS {
        walk(&root.join(dir), &mut files)?;
    }
    // A label alone on its line names what the next line's label does, as `ChiefPic` does.
    let alias = Regex::new(r"(?m)^(\w+)::?[ \t]*\r?\n(\w+)::?[ \t]*INCBIN").unwrap();
    let mut labels = BTreeMap::new();
    for file in files.iter().filter(|f| f.extension().is_some_and(|e| e == "asm")) {
        let source = fs::read_to_string(file)?;
        for caps in labelled.captures_iter(&source) {
            let extension = match &caps[3] { "pic" => "2bpp", other => other };
            labels.insert(caps[1].to_string(), format!("{}.{extension}", &caps[2]));
        }
        for caps in alias.captures_iter(&source) {
            if let Some(path) = labels.get(&caps[2]).cloned() {
                labels.insert(caps[1].to_string(), path);
            }
        }
    }
    Ok(labels)
}

/// `SpriteSheetPointerTable` as `SPRITE_SHEETS`, a picture id less one to its whole sheet and the
/// tiles of it that load.
fn sprite_sheets(out: &mut String, asm: &mut Asm, labels: &BTreeMap<String, String>) {
    out.push_str("pub const SPRITE_SHEETS: &[(&[u8], usize)] = &[\n");
    for row in asm.rows("data/sprites/sprites.asm", "SpriteSheetPointerTable") {
        assert_eq!(row.op, "overworld_sprite", "{row:?}");
        let path = labels.get(&row.args[0]).unwrap_or_else(|| panic!("`{}` INCBINs no whole asset", row.args[0]));
        writeln!(out, "    ({}, {}),", const_path(path), asm.eval(&row.args[1])).unwrap();
    }
    out.push_str("];\n");
}

/// The pics the cartridge reaches through a pointer table rather than by name: `MON_PICS`, each
/// species' front and back in Pokédex order, Mew's from its own entry outside `BaseStats`; and
/// `TRAINER_PICS`, by trainer class less one.
fn pics(out: &mut String, asm: &mut Asm, labels: &BTreeMap<String, String>) {
    let asset = |label: &str| const_path(labels.get(label).unwrap_or_else(|| panic!("`{label}` INCBINs no whole asset")));
    let mut rows = asm.rows("data/pokemon/base_stats.asm", "BaseStats");
    rows.extend(asm.rows("data/pokemon/mew.asm", "MewBaseStats"));
    let entries: Vec<_> = rows.chunks(11).map(|entry| {
        let pointers = entry.iter().find(|row| row.op == "dw").unwrap_or_else(|| panic!("no pic pointers in {entry:?}"));
        format!("({}, {})", asset(&pointers.args[0]), asset(&pointers.args[1]))
    }).collect();
    writeln!(out, "pub const MON_PICS: [(&[u8], &[u8]); {}] = [{}];", entries.len(), entries.join(", ")).unwrap();

    let rows = asm.rows("data/trainers/pic_pointers_money.asm", "TrainerPicAndMoneyPointers");
    let entries: Vec<_> = rows.iter().map(|row| {
        assert_eq!(row.op, "pic_money", "{row:?}");
        asset(&row.args[0])
    }).collect();
    writeln!(out, "pub const TRAINER_PICS: [&[u8]; {}] = [{}];", entries.len(), entries.join(", ")).unwrap();
}

/// `TileIDListPointerTable` as `TILE_ID_LISTS`, a `TILEMAP_*` id to its tile ids, width and height.
fn tile_id_lists(out: &mut String, asm: &mut Asm, labels: &BTreeMap<String, String>) {
    out.push_str("pub const TILE_ID_LISTS: &[(&[u8], usize, usize)] = &[\n");
    for row in asm.rows("data/tilemaps.asm", "TileIDListPointerTable") {
        assert_eq!(row.op, "tile_ids", "{row:?}");
        let path = labels.get(&row.args[0]).unwrap_or_else(|| panic!("`{}` INCBINs no whole asset", row.args[0]));
        writeln!(out, "    ({}, {}, {}),", const_path(path), asm.eval(&row.args[1]), asm.eval(&row.args[2])).unwrap();
    }
    out.push_str("];\n");
}

/// The three palettes `BorderPalettes` carries after its tilemap, SNES palettes 4 to 6 as RGB555.
/// The padding between them is where `PCT_TRN` expects each palette's other twelve colours.
fn sgb_border_palettes(out: &mut String, asm: &mut Asm) {
    let shifts = ["B_COLOR_RED", "B_COLOR_GREEN", "B_COLOR_BLUE"].map(|name| asm.eval(name));
    let rows = asm.rows("data/sgb/sgb_border.asm", "BorderPalettes");
    let ops: Vec<&str> = rows.iter().map(|row| row.op.as_str()).collect();
    let palette = ["RGB"; 4];
    let expected = [&["INCBIN", "ds"][..], &palette, &["ds"], &palette, &["ds"], &palette, &["ds"]].concat();
    assert_eq!(ops, expected, "BorderPalettes is laid out differently");
    let colours: Vec<String> = rows.iter().filter(|row| row.op == "RGB").map(|row| {
        assert_eq!(row.args.len(), 3, "{row:?}");
        let word: i64 = (0..3).map(|channel| asm.eval(&row.args[channel]) << shifts[channel]).sum();
        format!("{word:#06x}")
    }).collect();
    let palettes: Vec<String> = colours.chunks(4).map(|c| format!("[{}]", c.join(", "))).collect();
    writeln!(out, "pub const SGB_BORDER_PALETTES: [[u16; 4]; 3] = [{}];", palettes.join(", ")).unwrap();
}

/// Every `PalPacket_*` and `BlkPacket_*` in `data/sgb/sgb_packets.asm` as the bytes the cartridge
/// sends, as many sixteen-byte packets as its first byte counts: `sgb_packets::BLK_PACKET_WHOLE_SCREEN`.
fn sgb_packets(out: &mut String, asm: &mut Asm) {
    const FILE: &str = "data/sgb/sgb_packets.asm";
    let source = fs::read_to_string(Path::new(ROOT).join(FILE)).unwrap();
    let label = Regex::new(r"(?m)^((?:Pal|Blk)Packet_\w+):").unwrap();
    out.push_str("pub mod sgb_packets {\n");
    for caps in label.captures_iter(&source) {
        let name = &caps[1];
        let mut bytes = Vec::new();
        for row in asm.rows(FILE, name) {
            let arg = |i: usize| asm.eval(&row.args[i]);
            match row.op.as_str() {
                "ATTR_BLK" => bytes.extend([((4 << 3) + arg(0) * 6 / 16 + 1) as u8, arg(0) as u8]),
                "ATTR_BLK_DATA" => {
                    bytes.extend([arg(0), arg(1) + (arg(2) << 2) + (arg(3) << 4), arg(4), arg(5), arg(6), arg(7)].map(|b| b as u8));
                }
                "PAL_SET" => {
                    bytes.push((0xA << 3) + 1);
                    (0..4).for_each(|i| bytes.extend((arg(i) as u16).to_le_bytes()));
                    bytes.extend([0; 7]);
                }
                "ds" => bytes.extend(std::iter::repeat_n(row.args.get(1).map_or(0, |_| arg(1)) as u8, arg(0) as usize)),
                "db" => bytes.extend((0..row.args.len()).map(|i| asm.byte(&row.args[i]))),
                op => panic!("{name}: `{op}` is not a packet row"),
            }
        }
        let len = (bytes[0] & 7) as usize * 16;
        assert!(bytes.len() >= len, "{name} is {} bytes and sends {len}", bytes.len());
        let upper: String = name.chars().enumerate().flat_map(|(i, c)| {
            let gap = i > 0 && c.is_ascii_uppercase() && !name[..i].ends_with('_');
            gap.then_some('_').into_iter().chain([c.to_ascii_uppercase()])
        }).collect();
        writeln!(out, "    pub const {upper}: &[u8] = &{:?};", &bytes[..len]).unwrap();
    }
    out.push_str("}\n");
}

/// `RedFishingTiles` as `RED_FISHING_TILES`, each picture with the tile it loads to past
/// `vNPCSprites`, and `FishingRodOAM` as `FISHING_ROD_OAM`, one object a facing.
fn fishing(out: &mut String, asm: &mut Asm, labels: &BTreeMap<String, String>) {
    const FILE: &str = "engine/overworld/player_animations.asm";
    out.push_str("pub const RED_FISHING_TILES: &[(&[u8], u8)] = &[\n");
    for row in asm.rows(FILE, "RedFishingTiles") {
        assert_eq!(row.op, "fishing_gfx", "{row:?}");
        let path = labels.get(&row.args[0]).unwrap_or_else(|| panic!("`{}` INCBINs no whole asset", row.args[0]));
        let tiles = asm.eval(&row.args[1]);
        let whole = fs::read(Path::new(&std::env::var("OUT_DIR").unwrap()).join(path)).unwrap().len() as i64 / 16;
        assert_eq!(tiles, whole, "{row:?} copies other than the whole picture");
        writeln!(out, "    ({}, {}),", const_path(path), asm.eval(&row.args[2])).unwrap();
    }
    out.push_str("];\n");
    objects(out, asm, "FISHING_ROD_OAM", FILE, "FishingRodOAM");
}

/// `MonPartySpritePointers` as `MON_PARTY_SPRITES`: a picture, the first of its tiles copied, how
/// many, and the sprite tile they go to. A label can start partway into its picture, as the icons'
/// second frames do; and a read past a whole picture's end goes on into the next `INCBIN` in its
/// file, one row a picture: `PokeBallSprite`'s eight tiles are the ball and `FossilSprite`, the
/// helix icon.
fn mon_icons(out: &mut String, asm: &mut Asm, labels: &BTreeMap<String, String>) -> std::io::Result<()> {
    const FILE: &str = "engine/gfx/mon_icons.asm";
    let tile = asm.eval("TILE_SIZE");
    let tiles_of = |path: &str| fs::read(Path::new(&std::env::var("OUT_DIR").unwrap()).join(path)).unwrap().len() as i64 / tile;
    out.push_str("pub const MON_PARTY_SPRITES: &[(&[u8], usize, usize, u8)] = &[\n");
    for row in asm.rows("data/icon_pointers.asm", "MonPartySpritePointers") {
        assert_eq!((row.op.as_str(), row.args.len()), ("mon_icon_header", 4), "{row:?}");
        let label = &row.args[0];
        let (mut first, mut count, mut to) = (asm.eval(&row.args[1]), asm.eval(&row.args[2]), asm.eval(&row.args[3]));
        if labels.contains_key(label) {
            for path in incbins_from(label)? {
                let here = count.min(tiles_of(&path) - first);
                writeln!(out, "    ({}, {first}, {here}, {to}),", const_path(&path)).unwrap();
                (first, count, to) = (0, count - here, to + here);
                if count == 0 {
                    break;
                }
            }
            assert_eq!(count, 0, "{row:?} reads past the end of its file");
            continue;
        }
        let [incbin] = &asm.rows(FILE, label)[..] else { panic!("`{label}` is not one INCBIN") };
        let args: Vec<String> = incbin.args.iter().flat_map(|arg| asm.equs(arg).map_or_else(|| vec![arg.clone()], super::asm::split_args)).collect();
        let [path, start, length] = &args[..] else { panic!("{incbin:?}") };
        assert_eq!(incbin.op, "INCBIN", "{incbin:?}");
        let (start, length) = (asm.eval(start) / tile, asm.eval(length) / tile);
        assert!(first + count <= length, "{row:?} reads past `{label}`");
        writeln!(out, "    ({}, {}, {count}, {to}),", const_path(path.trim_matches('"')), start + first).unwrap();
    }
    out.push_str("];\n");
    Ok(())
}

/// The picture `label` `INCBIN`s whole, and each one `INCBIN`ed on the lines after it.
fn incbins_from(label: &str) -> std::io::Result<Vec<String>> {
    let incbin = Regex::new(r#"^(\w+)::?\s*INCBIN\s+"(gfx/[^"]+\.2bpp)"\s*$"#).unwrap();
    let mut files = Vec::new();
    for dir in ASM_DIRS {
        walk(&Path::new(ROOT).join(dir), &mut files)?;
    }
    for file in files.iter().filter(|f| f.extension().is_some_and(|e| e == "asm")) {
        let source = fs::read_to_string(file)?;
        let mut lines = source.lines().map(|line| incbin.captures(line));
        if lines.by_ref().any(|caps| caps.is_some_and(|caps| &caps[1] == label)) {
            let mut paths = vec![Regex::new(&format!(r#"(?m)^{label}::?\s*INCBIN\s+"([^"]+)""#)).unwrap().captures(&source).unwrap()[1].to_string()];
            paths.extend(lines.map_while(|caps| Some(caps?[2].to_string())));
            return Ok(paths);
        }
    }
    panic!("`{label}` INCBINs no whole picture")
}

/// A table of `dbsprite` rows as `[y, x, tile, attributes]` objects.
fn objects(out: &mut String, asm: &mut Asm, name: &str, file: &str, label: &str) {
    let objects: Vec<String> = asm.rows(file, label).iter().map(|row| {
        assert_eq!(row.op, "dbsprite", "{row:?}");
        let arg = |i: usize| asm.eval(&row.args[i]);
        format!("[{}, {}, {}, {}]", (arg(1) * 8) % 0x100 + arg(3), (arg(0) * 8) % 0x100 + arg(2), arg(4), arg(5))
    }).collect();
    writeln!(out, "pub const {name}: [[u8; 4]; {}] = [{}];", objects.len(), objects.join(", ")).unwrap();
}

fn top_level_asm(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = fs::read_dir(root)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    files.retain(|f| f.extension().is_some_and(|e| e == "asm"));
    files.sort();
    Ok(files)
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?.map(|e| e.map(|e| e.path())).collect::<Result<_, _>>()?;
    entries.sort();
    for path in entries {
        if path.is_dir() {
            walk(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

/// The Makefile's target-specific `tools/gfx += ...` and `RGBGFXFLAGS += ...` lines, as
/// (target pattern, variable, flags).
fn makefile_rules(makefile: &str) -> Vec<(String, String, String)> {
    let rule = Regex::new(r"^(\S+\.[12]bpp):\s*(tools/gfx|RGBGFXFLAGS)\s*\+=\s*(.*?)\s*$").unwrap();
    makefile
        .lines()
        .filter_map(|line| rule.captures(line))
        .map(|c| (c[1].to_string(), c[2].to_string(), c[3].to_string()))
        .collect()
}

fn options_for(rules: &[(String, String, String)], target: &str) -> Options {
    let mut options = Options::default();
    for (pattern, variable, flags) in rules {
        let matches = match pattern.split_once('%') {
            Some((prefix, suffix)) => {
                target.len() > prefix.len() + suffix.len() && target.starts_with(prefix) && target.ends_with(suffix)
            }
            None => pattern == target,
        };
        if !matches {
            continue;
        }
        for flag in flags.split_whitespace() {
            match (variable.as_str(), flag) {
                ("RGBGFXFLAGS", "--columns") => options.columns = true,
                ("tools/gfx", "--trim-whitespace") => options.trim_whitespace = true,
                ("tools/gfx", "--remove-duplicates") => options.remove_duplicates = true,
                ("tools/gfx", "--interleave") => options.interleave = true,
                // The width `--interleave` needs is read from the PNG being converted anyway.
                ("tools/gfx", f) if f.starts_with("--png=") => {}
                ("tools/gfx", f) if f.starts_with("--preserve=") => {
                    options.preserved.extend(f["--preserve=".len()..].split(',').map(parse_c_integer));
                }
                _ => panic!("the Makefile gives {target} `{variable} += {flag}`, which the asset pipeline does not implement"),
            }
        }
    }
    options
}

/// `strtoul(s, NULL, 0)`.
fn parse_c_integer(s: &str) -> usize {
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => usize::from_str_radix(hex, 16).unwrap(),
        None if s.len() > 1 && s.starts_with('0') => usize::from_str_radix(&s[1..], 8).unwrap(),
        None => s.parse().unwrap(),
    }
}

fn convert(png: &[u8], depth: u8, options: &Options) -> Result<Vec<u8>, String> {
    let (width, height, grays) = decode_gray(png)?;
    if width % 8 != 0 || height % 8 != 0 {
        return Err(format!("{width}×{height} is not a whole number of tiles"));
    }
    let mut data = rgbgfx(width, height, &grays, depth, options.columns);
    let tile_size = depth as usize * 8;
    let mut preserved = options.preserved.clone();
    if options.trim_whitespace {
        trim_whitespace(&mut data, tile_size, &preserved);
    }
    if options.interleave {
        interleave(&mut data, tile_size, width);
    }
    if options.remove_duplicates {
        // `--interleave` doubles the unit `tools/gfx` deduplicates by.
        let unit = if options.interleave { tile_size * 2 } else { tile_size };
        remove_duplicates(&mut data, unit, &mut preserved);
    }
    Ok(data)
}

/// The image as one 8-bit gray level per pixel. `rgbgfx` refuses a colour or a transparent pixel
/// in DMG mode, and so does this.
fn decode_gray(png: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("image too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (width, height) = (info.width as usize, info.height as usize);
    let channels = info.color_type.samples();
    let mut grays = Vec::with_capacity(width * height);
    for y in 0..height {
        let row = &buf[y * info.line_size..][..width * channels];
        for pixel in row.chunks_exact(channels) {
            let (gray, opaque) = match pixel {
                [v] => (*v, true),
                [v, a] => (*v, *a == 0xff),
                [r, g, b] => (*r, r == g && g == b),
                [r, g, b, a] => (*r, r == g && g == b && *a == 0xff),
                _ => unreachable!(),
            };
            if !opaque {
                return Err(format!("pixel ({}, {y}) is not an opaque gray", grays.len() % width));
            }
            grays.push(gray);
        }
    }
    Ok((width, height, grays))
}

/// `rgbgfx --colors dmg`: each gray falls into one of `2^depth` equal bins, lightest first, and
/// tiles are read row by row, or column by column under `--columns`.
fn rgbgfx(width: usize, height: usize, grays: &[u8], depth: u8, columns: bool) -> Vec<u8> {
    let bins = 1usize << depth;
    let (tiles_wide, tiles_high) = (width / 8, height / 8);
    let order: Vec<(usize, usize)> = if columns {
        (0..tiles_wide).flat_map(|x| (0..tiles_high).map(move |y| (x, y))).collect()
    } else {
        (0..tiles_high).flat_map(|y| (0..tiles_wide).map(move |x| (x, y))).collect()
    };
    let mut out = Vec::with_capacity(order.len() * 8 * depth as usize);
    for (tx, ty) in order {
        for row in 0..8 {
            let (mut low, mut high) = (0u8, 0u8);
            for col in 0..8 {
                let gray = grays[(ty * 8 + row) * width + tx * 8 + col] as usize;
                let index = (255 - gray) * bins / 256;
                low |= ((index & 1) as u8) << (7 - col);
                high |= ((index >> 1 & 1) as u8) << (7 - col);
            }
            out.push(low);
            if depth == 2 {
                out.push(high);
            }
        }
    }
    out
}

/// Drops trailing blank tiles, bar a preserved one; never the first.
fn trim_whitespace(data: &mut Vec<u8>, tile_size: usize, preserved: &[usize]) {
    let mut size = data.len();
    let mut i = data.len().saturating_sub(tile_size);
    while i > 0 && data[i..i + tile_size].iter().all(|&b| b == 0) && !preserved.contains(&(i / tile_size)) {
        size = i;
        i -= tile_size;
    }
    data.truncate(size);
}

/// Reorders a sheet of 8×8 tiles into 8×16 sprite order: each pair of rows becomes top, bottom,
/// top, bottom.
fn interleave(data: &mut Vec<u8>, tile_size: usize, width: usize) {
    let tiles_wide = width / 8;
    let num_tiles = data.len() / tile_size;
    let mut interleaved = vec![0; num_tiles * tile_size];
    for i in 0..num_tiles {
        let row = i / tiles_wide;
        let tile = i * 2 - if row % 2 == 1 { tiles_wide * (row + 1) - 1 } else { tiles_wide * row };
        interleaved[tile * tile_size..][..tile_size].copy_from_slice(&data[i * tile_size..][..tile_size]);
    }
    *data = interleaved;
}

/// Keeps the first of each distinct tile, in order. A preserved index, which counts positions
/// after earlier removals, is kept even when it repeats one.
fn remove_duplicates(data: &mut Vec<u8>, tile_size: usize, preserved: &mut [usize]) {
    let len = data.len() / tile_size * tile_size;
    let exists = |data: &[u8], j: usize, kept: usize| {
        (0..kept).any(|k| data[k * tile_size..][..tile_size] == data[j..j + tile_size])
    };
    let mut kept = 0;
    let (mut j, mut removed) = (0, 0);
    while j < len {
        while j < len && exists(data, j, kept) {
            if preserved.contains(&(j / tile_size - removed)) {
                break;
            }
            let index = j / tile_size - removed;
            preserved.iter_mut().filter(|p| **p >= index).for_each(|p| *p -= 1);
            j += tile_size;
            removed += 1;
        }
        if j >= len {
            break;
        }
        data.copy_within(j..j + tile_size, kept * tile_size);
        kept += 1;
        j += tile_size;
    }
    data.truncate(kept * tile_size);
}

/// A module per directory under `gfx/` and a `&[u8; N]` per asset, plus `ALL`, keyed by the path
/// `make` would write the file to.
fn rust_source(entries: &[(String, usize)]) -> String {
    #[derive(Default)]
    struct Dir {
        consts: BTreeMap<String, (String, usize)>,
        dirs: BTreeMap<String, Dir>,
    }
    let mut root = Dir::default();
    for (path, len) in entries {
        let mut parts: Vec<String> = const_path(path).split("::").map(str::to_string).collect();
        let name = parts.pop().unwrap();
        let dir = parts.iter().fold(&mut root, |d, p| d.dirs.entry(p.to_string()).or_default());
        let previous = dir.consts.insert(name.clone(), (path.clone(), *len));
        assert!(previous.is_none(), "two assets are named {name} in {path}'s directory");
    }
    fn emit(dir: &Dir, depth: usize, out: &mut String) {
        let indent = "    ".repeat(depth);
        for (name, (path, len)) in &dir.consts {
            writeln!(out, "{indent}pub const {name}: &[u8; {len}] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{path}\"));").unwrap();
        }
        for (name, sub) in &dir.dirs {
            writeln!(out, "{indent}pub mod {name} {{").unwrap();
            emit(sub, depth + 1, out);
            writeln!(out, "{indent}}}").unwrap();
        }
    }
    let mut out = String::from("// Generated by build.rs from vendor/pokered/gfx.\n");
    emit(&root, 0, &mut out);
    out.push_str("pub const ALL: &[(&str, &[u8])] = &[\n");
    for (path, _) in entries {
        writeln!(out, "    (\"{path}\", include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{path}\"))),").unwrap();
    }
    out.push_str("];\n");
    out
}

/// `gfx/sprites/red.2bpp` as `sprites::RED`. A tilemap or a run-length map shares its stem with
/// the tiles it arranges, so it keeps its extension: `sgb::RED_BORDER_TILEMAP`.
fn const_path(path: &str) -> String {
    let mut parts: Vec<&str> = path.strip_prefix("gfx/").unwrap().split('/').collect();
    let (stem, extension) = parts.pop().unwrap().rsplit_once('.').unwrap();
    let stem = match extension {
        "tilemap" | "rle" => format!("{stem}_{extension}"),
        _ => stem.to_string(),
    };
    let mut name: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert(0, '_');
    }
    parts.iter().map(|p| p.to_string()).chain([name]).collect::<Vec<_>>().join("::")
}
