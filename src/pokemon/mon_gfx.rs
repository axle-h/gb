//! Pokémon front sprites, decompressed out of the cartridge the binary already carries.
//!
//! The sibling of [`crate::pokemon::badge_gfx`], and the same bargain: nothing is committed, nothing
//! is read from disk, the art comes from the same ROM bytes the emulator boots. The difference is
//! that a badge is four tiles you can slice straight out of the ROM, and a Pokémon pic is
//! *compressed* — so this module is a port of `pokered`'s `home/uncompress.asm`
//! (`_UncompressSpriteData`) and the alignment half of `home/pics.asm`.
//!
//! # The format
//!
//! A pic is a bitstream, MSB-first within each byte. Byte 0 is the dimensions — high nybble width,
//! low nybble height, both in 8×8 tiles. After it:
//!
//! 1. One bit says which of two 1bpp planes receives the first chunk.
//! 2. A chunk is written **two pixels at a time, column-major**, in four passes over the same
//!    `width × height*8` byte area — the first pass filling bits 7-6 of every byte, then 5-4, 3-2,
//!    1-0. A "column" here is 8 pixels wide and the whole sprite tall, and the stream walks it top
//!    to bottom four times before moving 8 pixels right.
//! 3. Within a chunk the stream alternates between literal pixel pairs and runs of zero pairs. A
//!    `00` literal is the escape into a run; the run length is length-prefixed (`n` ones, a zero,
//!    then `n + 1` bits) plus an offset of `2^(n+1) - 1`, which is what stops any length having two
//!    encodings.
//! 4. Before the *second* chunk, one or two bits give the unpack mode: `0` → the planes are
//!    differential-encoded independently; `10` → chunk 1 is differential-encoded and XORed into
//!    chunk 2; `11` → both are differential-decoded and then XORed.
//!
//! ⚠️ **The differential decode runs along rows, and its running value resets per row** — the
//! opposite axis to the one the bitstream was written along. Getting that backwards produces a
//! sprite that is recognisably the right Pokémon with horizontal smears through it, which is exactly
//! the kind of wrong that looks right in a thumbnail.
//! [`tests::the_decompressor_matches_upstreams_own_2bpp`] is the guard.
//!
//! ⚠️ **`wSpriteFlipped` is not implemented, and must not be needed.** The game sets it only for
//! back pics and the player's own sprite in battle; a front pic is never flipped. Nothing here has a
//! caller that could ask for one, and the four flipped decode tables are simply absent.
//!
//! # Where the bytes are
//!
//! Addressed through the game's own base-stats table rather than through the 152 generated
//! `…PicFront` constants: `BaseStats + (dex - 1) * 28`, whose bytes 11-12 are the front-pic pointer.
//! That is one ladder of five ranges (`UncompressMonSprite`'s) instead of 151 match arms, and it
//! reads the table `GetMonHeader` reads. Both symbols it needs are `build.rs`-generated, so an
//! address that moves upstream is still a compile error rather than a wrong sprite.

use crate::pokemon::rom_gfx::rom_slice;
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::symbols::pokered_symbols;
use crate::pokemon::symbols::{DmgBank, DmgPointer};

/// The sprite buffer is 7×7 tiles and every pic is centred in it, so every sprite this module
/// returns is the same size whatever the Pokémon's own dimensions are.
pub const PIC_TILES: usize = 7;
/// 56×56 pixels.
pub const PIC_PX: usize = PIC_TILES * 8;

/// `SPRITEBUFFERSIZE`: one 1bpp plane of the 7×7-tile canvas.
const PLANE_BYTES: usize = PIC_TILES * PIC_TILES * 8;
/// `BASE_DATA_SIZE`.
const BASE_DATA_SIZE: usize = 28;
/// `BASE_FRONTPIC` — where the front-pic pointer sits within a base-stats entry.
const BASE_FRONTPIC: usize = 11;

/// One Pokémon's front sprite as shade indices, row-major, `0` (lightest) to `3` (darkest) — the
/// 2bpp values themselves, not colours, exactly as [`crate::pokemon::badge_gfx::badge_shades`]
/// returns them. What they look like is the caller's business.
///
/// The sprite is centred horizontally and stands on the bottom of the 56×56 canvas, which is what
/// `AlignSpriteDataCentered` does and therefore where the game itself puts it.
pub fn front_pic_shades(species: PokemonSpecies) -> [u8; PIC_PX * PIC_PX] {
    let Pic { width, height, low, high } = decompress(front_pic(species));
    let low = align_centred(&low, width, height);
    let high = align_centred(&high, width, height);

    let mut shades = [0u8; PIC_PX * PIC_PX];
    for y in 0..PIC_PX {
        for x in 0..PIC_PX {
            // A byte is 8 pixels wide and the canvas is column-major: 7 columns of 56 rows.
            let byte = (x / 8) * PIC_PX + y;
            let bit = 7 - (x % 8);
            shades[y * PIC_PX + x] = ((high[byte] >> bit) & 1) << 1 | ((low[byte] >> bit) & 1);
        }
    }
    shades
}

/// A pic's own dimensions in 8×8 tiles, which is the first byte of its compressed data. Only the
/// tests need it — [`front_pic_shades`] always returns the full canvas.
#[cfg(test)]
fn front_pic_size(species: PokemonSpecies) -> (usize, usize) {
    let dimensions = front_pic(species)[0];
    ((dimensions >> 4) as usize, (dimensions & 0xF) as usize)
}

// ── Finding the pic ──────────────────────────────────────────────────────────────────────────────

/// The compressed pic, as a slice running to the end of its bank.
fn front_pic(species: PokemonSpecies) -> &'static [u8] {
    let entry = base_stats_entry(species);
    let address = u16::from_le_bytes([entry[BASE_FRONTPIC], entry[BASE_FRONTPIC + 1]]);
    rom_slice(DmgPointer { bank: pic_bank(species), address })
}

/// The 28-byte base-stats entry.
///
/// ⚠️ Indexed by **Pokédex** number, unlike everything else here, because that is how the table is
/// laid out — `GetMonHeader` runs the internal index through `IndexToPokedex` first. And Mew is not
/// in the table at all: it has an entry of its own in another bank, so a lookup that forgets it
/// reads Mewtwo's, which is a valid pointer into the wrong bank and decodes to noise rather than
/// failing.
///
/// Shared with [`crate::pokemon::learnset`], which reads the TM/HM flag array out of the same 28
/// bytes: the Mew arm above is the whole reason it is one function rather than two.
pub(crate) fn base_stats_entry(species: PokemonSpecies) -> &'static [u8] {
    let pointer = if species == PokemonSpecies::Mew {
        pokered_symbols::MewBaseStats
    } else {
        let dex = species.metadata().pokedex_number as u16;
        pokered_symbols::BaseStats + (dex - 1) * BASE_DATA_SIZE as u16
    };
    &rom_slice(pointer)[..BASE_DATA_SIZE]
}

/// `UncompressMonSprite`'s bank ladder, on the **internal** index rather than the Pokédex number.
/// The fossil and ghost pics it also handles are not species and are unreachable from here, so only
/// Mew's arm survives beside the five "Pics" banks.
fn pic_bank(species: PokemonSpecies) -> DmgBank {
    let bank = match species as u8 {
        _ if species == PokemonSpecies::Mew => 0x01,
        0x00..=0x1E => 0x09, // "Pics 1", through Tangela
        0x1F..=0x49 => 0x0A, // "Pics 2", through Moltres
        0x4A..=0x73 => 0x0B, // "Pics 3", through Beedrill + 1
        0x74..=0x98 => 0x0C, // "Pics 4", through Starmie
        0x99..=0xFF => 0x0D, // "Pics 5"
    };
    DmgBank::ROM { bank }
}

// ── The bitstream ────────────────────────────────────────────────────────────────────────────────

/// `ReadNextInputBit`: MSB-first, one byte at a time. It never checks for the end of the input,
/// because the stream is terminated by the *output* filling up rather than by running out.
struct BitReader {
    data: &'static [u8],
    position: usize,
    remaining: u8,
}

impl BitReader {
    fn new(data: &'static [u8]) -> Self {
        Self { data, position: 0, remaining: 0 }
    }

    fn byte(&mut self) -> u8 {
        let byte = self.data[self.position];
        self.position += 1;
        byte
    }

    fn bit(&mut self) -> u8 {
        if self.remaining == 0 {
            self.remaining = 8;
            self.position += 1;
        }
        self.remaining -= 1;
        (self.data[self.position - 1] >> self.remaining) & 1
    }

    /// Two bits, the first read being the high one — `WriteSpriteBitsToBuffer`'s argument.
    fn pair(&mut self) -> u8 {
        let high = self.bit();
        high << 1 | self.bit()
    }
}

// ── Decompression ────────────────────────────────────────────────────────────────────────────────

/// Two 1bpp planes and the size they cover, each plane `width * height * 8` bytes packed as `width`
/// columns of `height * 8` rows.
///
/// The planes come out in Game Boy 2bpp order — `sSpriteBuffer1`'s chunk is the low bit of each
/// pixel and `sSpriteBuffer2`'s the high bit, which is the order `InterlaceMergeSpriteBuffers`
/// writes them in.
struct Pic {
    width: usize,
    height: usize,
    low: Vec<u8>,
    high: Vec<u8>,
}

fn decompress(data: &'static [u8]) -> Pic {
    let mut input = BitReader::new(data);

    let dimensions = input.byte();
    let (width, height) = ((dimensions >> 4) as usize, (dimensions & 0xF) as usize);
    assert!(
        (1..=PIC_TILES).contains(&width) && (1..=PIC_TILES).contains(&height),
        "a {width}×{height}-tile pic does not fit the 7×7 sprite buffer",
    );
    let rows = height * 8;

    // `wSpriteLoadFlags` bit 0: which plane the first chunk goes to. The second goes to the other,
    // and which is which decides the source and destination of the XOR below.
    let first = usize::from(input.bit() == 1);
    let mut planes = [vec![0u8; width * rows], vec![0u8; width * rows]];

    read_chunk(&mut input, &mut planes[first], width, rows);
    // ⚠️ The mode is read at the top of the *second* chunk, after the first has been consumed —
    // not up front with the dimensions and the plane bit.
    let mode = if input.bit() == 0 { 0 } else { input.bit() + 1 };
    read_chunk(&mut input, &mut planes[1 - first], width, rows);

    // `UnpackSprite`. The chunk read first is the source, the one read second the destination.
    let (source, destination) = (first, 1 - first);
    match mode {
        0 => {
            differential_decode(&mut planes[0], width, rows);
            differential_decode(&mut planes[1], width, rows);
        }
        1 => {
            differential_decode(&mut planes[source], width, rows);
            xor_into(&mut planes, source, destination);
        }
        _ => {
            differential_decode(&mut planes[destination], width, rows);
            differential_decode(&mut planes[source], width, rows);
            xor_into(&mut planes, source, destination);
        }
    }

    let [low, high] = planes;
    Pic { width, height, low, high }
}

/// Where the next pixel pair goes: `MoveToNextBufferPosition`, which in the original terminates the
/// decompression loop by unwinding the stack out from under it.
struct Cursor {
    column: usize,
    /// Counts *down* 3 → 0, and is half the bit position within the byte.
    pass: u8,
    row: usize,
    width: usize,
    rows: usize,
}

impl Cursor {
    fn new(width: usize, rows: usize) -> Self {
        Self { column: 0, pass: 3, row: 0, width, rows }
    }

    fn index(&self) -> usize {
        self.column * self.rows + self.row
    }

    /// `false` once every column has had all four passes, which is the chunk's only terminator.
    fn advance(&mut self) -> bool {
        self.row += 1;
        if self.row < self.rows {
            return true;
        }
        self.row = 0;
        if self.pass > 0 {
            self.pass -= 1;
            return true;
        }
        self.pass = 3;
        self.column += 1;
        self.column < self.width
    }
}

/// One 1bpp chunk — `UncompressSpriteDataLoop`. Runs of zeros OR nothing into an already-zeroed
/// plane, so only the literals write; all a run does is move the cursor.
fn read_chunk(input: &mut BitReader, plane: &mut [u8], width: usize, rows: usize) {
    let mut cursor = Cursor::new(width, rows);
    // One opening bit says which of the two states the chunk starts in. After that a `00` literal is
    // the escape into a run, and a run always hands back to the literals.
    let mut zeros = input.bit() == 0;
    loop {
        if zeros {
            for _ in 0..zero_run(input) {
                if !cursor.advance() {
                    return;
                }
            }
        }
        loop {
            let value = input.pair();
            if value == 0 {
                break;
            }
            plane[cursor.index()] |= value << (cursor.pass * 2);
            if !cursor.advance() {
                return;
            }
        }
        zeros = true;
    }
}

/// The length of a run of zero pairs: `n` ones then a zero give the width, `n + 1` bits give the
/// value, and `2^(n+1) - 1` is added so that no length has two encodings.
fn zero_run(input: &mut BitReader) -> u32 {
    let mut ones = 0u32;
    while input.bit() == 1 {
        ones += 1;
    }
    let mut value = 0u32;
    for _ in 0..=ones {
        value = value << 1 | input.bit() as u32;
    }
    value + (1 << (ones + 1)) - 1
}

/// `SpriteDifferentialDecode`. An input bit of 0 preserves the previous pixel and 1 toggles it, and
/// the running value restarts at 0 on every row — so this walks *rows*, striding across a
/// column-major buffer, while the bitstream that filled it walked columns.
fn differential_decode(plane: &mut [u8], width: usize, rows: usize) {
    for row in 0..rows {
        let mut previous = 0u8;
        for column in 0..width {
            let at = column * rows + row;
            let high = decode_nybble(plane[at] >> 4, previous);
            let low = decode_nybble(plane[at] & 0xF, high);
            plane[at] = high << 4 | low;
            previous = low;
        }
    }
}

/// `DifferentialDecodeNybble`: four toggle-or-hold bits at a time, seeded by the last bit of the
/// nybble before it.
fn decode_nybble(nybble: u8, previous: u8) -> u8 {
    /// `DecodeNybble0Table` / `DecodeNybble1Table` as `(high, low)` pairs — the `dn` macro packs two
    /// nybbles into each byte. Table 1 is table 0 rotated by four, which is the "the previous bit
    /// was 1" case and inverts every output.
    const TABLES: [[(u8, u8); 8]; 2] = [
        [(0x0, 0x1), (0x3, 0x2), (0x7, 0x6), (0x4, 0x5), (0xF, 0xE), (0xC, 0xD), (0x8, 0x9), (0xB, 0xA)],
        [(0xF, 0xE), (0xC, 0xD), (0x8, 0x9), (0xB, 0xA), (0x0, 0x1), (0x3, 0x2), (0x7, 0x6), (0x4, 0x5)],
    ];
    let (high, low) = TABLES[(previous & 1) as usize][(nybble >> 1) as usize];
    if nybble & 1 == 1 { low } else { high }
}

/// `XorSpriteChunks`: the chunk read second is the destination.
fn xor_into(planes: &mut [Vec<u8>; 2], source: usize, destination: usize) {
    for index in 0..planes[source].len() {
        planes[destination][index] ^= planes[source][index];
    }
}

// ── Alignment ────────────────────────────────────────────────────────────────────────────────────

/// `AlignSpriteDataCentered`: drop the `width × height` sprite into the 7×7 canvas, centred
/// horizontally and pushed to the bottom vertically — a Pokémon stands on the floor of its box
/// rather than floating in the middle of it.
fn align_centred(plane: &[u8], width: usize, height: usize) -> [u8; PLANE_BYTES] {
    let mut canvas = [0u8; PLANE_BYTES];
    let left = (PIC_TILES + 1 - width) / 2;
    let top = PIC_TILES - height;
    for column in 0..width {
        let from = column * height * 8;
        let to = (left + column) * PIC_PX + top * 8;
        canvas[to..to + height * 8].copy_from_slice(&plane[from..from + height * 8]);
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    /// The upstream file name for a species. Four of the 151 do not fall out of the display name.
    fn sprite_file_name(species: PokemonSpecies) -> String {
        match species {
            PokemonSpecies::NidoranMale => "nidoranm".to_string(),
            PokemonSpecies::NidoranFemale => "nidoranf".to_string(),
            PokemonSpecies::MrMime => "mr.mime".to_string(),
            other => other.metadata().name.to_lowercase(),
        }
    }

    /// `make` leaves the *uncompressed* form of every pic beside the compressed one, so upstream's
    /// own build output is available as an oracle.
    ///
    /// ⚠️ **Read from disk, not `include_bytes!`.** `.dockerignore` excludes `pokered/**/*.2bpp` and
    /// the Dockerfile's build stage copies only `pokered.gbc` and `pokered.sym`, so a compile-time
    /// dependency on these would build here and fail in the container.
    fn upstream_2bpp(species: PokemonSpecies) -> Option<Vec<u8>> {
        std::fs::read(format!("pokered/gfx/pokemon/front/{}.2bpp", sprite_file_name(species))).ok()
    }

    /// Reverse [`front_pic_shades`]'s canvas back into the tile stream a `.2bpp` file holds: each
    /// tile eight `(low, high)` byte pairs.
    ///
    /// ⚠️ **Row-major tiles, left to right and then down.** The ROM's *decompressed* form is
    /// column-major — that is what `AlignSpriteDataCentered` builds — but `.2bpp` is rgbgfx's plain
    /// output and pokered's Makefile passes it no `--columns` for pics, so the file on disk is the
    /// other way round. Comparing against the wrong one produces a diff on four fifths of the bytes
    /// with both sides looking like drawn sprites, which is an hour to spot.
    fn as_2bpp(shades: &[u8; PIC_PX * PIC_PX], width: usize, height: usize) -> Vec<u8> {
        let (left, top) = ((PIC_TILES + 1 - width) / 2, PIC_TILES - height);
        let mut bytes = Vec::with_capacity(width * height * 16);
        for tile in 0..height {
            for column in 0..width {
                for row in 0..8 {
                    let y = (top + tile) * 8 + row;
                    let (mut low, mut high) = (0u8, 0u8);
                    for x in 0..8 {
                        let shade = shades[y * PIC_PX + (left + column) * 8 + x];
                        low |= (shade & 1) << (7 - x);
                        high |= ((shade >> 1) & 1) << (7 - x);
                    }
                    bytes.push(low);
                    bytes.push(high);
                }
            }
        }
        bytes
    }

    /// FNV-1a. Any stable hash would do; this one is four lines and needs no dependency.
    fn checksum(shades: &[u8; PIC_PX * PIC_PX]) -> u32 {
        let mut hash = 0x811C_9DC5u32;
        for &shade in shades.iter() {
            hash = (hash ^ shade as u32).wrapping_mul(0x0100_0193);
        }
        hash
    }

    /// The one test that can *prove* the port rather than merely exercise it.
    ///
    /// It is skipped, loudly, in a checkout that has never run `make -C pokered` — the crate needs
    /// only `pokered.gbc` to compile, so that is a real state to be in.
    /// [`every_front_pic_matches_its_committed_checksum`] is what covers those builds.
    #[test]
    fn the_decompressor_matches_upstreams_own_2bpp() {
        let mut checked = 0;
        for species in PokemonSpecies::iter() {
            let Some(expected) = upstream_2bpp(species) else { continue };
            let (width, height) = front_pic_size(species);
            let actual = as_2bpp(&front_pic_shades(species), width, height);
            assert_eq!(
                actual.len(),
                expected.len(),
                "{species} decoded to {width}×{height} tiles, which is not the size of its .2bpp",
            );
            let wrong = actual.iter().zip(&expected).filter(|(a, b)| a != b).count();
            assert_eq!(wrong, 0, "{species}: {wrong} of {} bytes differ from upstream's", expected.len());
            checked += 1;
        }
        if checked == 0 {
            println!("no .2bpp files — run `make -C pokered` to give this test its oracle");
            return;
        }
        assert_eq!(checked, 151, "some .2bpp files were found but not all of them");
    }

    /// The net for a build with no submodule artifacts, and the reason the fixture exists at all: it
    /// was generated from output the test above had already proved byte-identical to upstream's.
    /// Regenerate with `dump_front_pic_checksums` (see below) after a *deliberate* change.
    ///
    /// ⚠️ **It lives in `data/gfx/`, not in `data/`, and that is not tidiness.**
    /// `savestate::tests::every_committed_fixture_decodes` walks `src/pokemon/data/*.bin` and tries
    /// to `load_state` every one of them — so a fixture of any other kind dropped in beside them
    /// fails a test three modules away with a save-state error message.
    #[test]
    fn every_front_pic_matches_its_committed_checksum() {
        let expected = include_bytes!("data/gfx/front_pic_checksums.bin");
        assert_eq!(expected.len(), 151 * 4, "one u32 per species, in Pokédex order");
        for species in PokemonSpecies::iter() {
            let dex = species.metadata().pokedex_number as usize;
            let at = (dex - 1) * 4;
            let want = u32::from_le_bytes(expected[at..at + 4].try_into().unwrap());
            assert_eq!(checksum(&front_pic_shades(species)), want, "{species} (#{dex})");
        }
    }

    /// Writes `src/pokemon/data/gfx/front_pic_checksums.bin`. A tool rather than a test, hence the
    /// feature gate and the `#[ignore]` on top of it — and it must only be run when
    /// `the_decompressor_matches_upstreams_own_2bpp` is green, since that is the only thing that
    /// makes the fixture mean anything.
    #[test]
    #[ignore = "a tool: writes the checksum fixture"]
    #[cfg(feature = "diagnostics")]
    fn dump_front_pic_checksums() {
        let mut bytes = vec![0u8; 151 * 4];
        for species in PokemonSpecies::iter() {
            let at = (species.metadata().pokedex_number as usize - 1) * 4;
            bytes[at..at + 4].copy_from_slice(&checksum(&front_pic_shades(species)).to_le_bytes());
        }
        std::fs::write("src/pokemon/data/gfx/front_pic_checksums.bin", &bytes).expect("write the fixture");
        println!("wrote {} bytes of front-pic checksums", bytes.len());
    }

    /// Offset arithmetic one tile out still produces a plausible-looking sprite — of half a
    /// different Pokémon. So: all 151, all different, none blank, none a solid block.
    #[test]
    fn every_species_decodes_to_a_distinct_drawn_sprite() {
        let sprites: Vec<_> = PokemonSpecies::iter().map(|s| (s, front_pic_shades(s))).collect();
        assert_eq!(sprites.len(), 151);

        for (species, shades) in &sprites {
            let mut used = shades.to_vec();
            used.sort_unstable();
            used.dedup();
            assert!(used.len() >= 3, "{species} uses only {} shades — {used:?}", used.len());

            let drawn = shades.iter().filter(|&&s| s != 0).count();
            assert!(
                (PIC_PX * 4..PIC_PX * PIC_PX * 9 / 10).contains(&drawn),
                "{species} has {drawn} non-background pixels of {}",
                PIC_PX * PIC_PX,
            );
        }

        for (index, (species, first)) in sprites.iter().enumerate() {
            for (other, second) in sprites.iter().skip(index + 1) {
                assert_ne!(first, second, "{species} and {other} decoded identically");
            }
        }
    }

    /// The canvas is the game's own: centred horizontally, standing on the bottom edge. A sprite
    /// centred vertically as well looks fine on its own and wrong beside the rest of the party.
    #[test]
    fn sprites_are_centred_horizontally_and_stand_on_the_bottom() {
        let mut reached_the_floor = 0;
        for species in PokemonSpecies::iter() {
            let shades = front_pic_shades(species);
            let (width, height) = front_pic_size(species);
            let left = (PIC_TILES + 1 - width) / 2;

            for y in 0..PIC_PX {
                for x in 0..PIC_PX {
                    let inside = (left * 8..(left + width) * 8).contains(&x) && y >= (PIC_TILES - height) * 8;
                    assert!(inside || shades[y * PIC_PX + x] == 0, "{species} has ink at ({x}, {y})");
                }
            }
            if (0..PIC_PX).any(|x| shades[(PIC_PX - 1) * PIC_PX + x] != 0) {
                reached_the_floor += 1;
            }
        }
        // Bottom-alignment is only observable on sprites that use their full box, but plenty do —
        // if none did, the assertion above would pass just as well with the sprite floated.
        assert!(reached_the_floor > 50, "only {reached_the_floor} sprites touch the bottom row");
    }

    /// Mew is not in `BaseStats`, and a lookup that ignores that reads Mewtwo's entry — a valid
    /// pointer into the wrong bank, so it decodes to noise rather than failing.
    #[test]
    fn mew_comes_from_its_own_base_stats_entry() {
        assert_eq!(pic_bank(PokemonSpecies::Mew), DmgBank::ROM { bank: 0x01 });
        assert_eq!(base_stats_entry(PokemonSpecies::Mew)[0], 151, "Mew's own entry, whose dex number is 151");
    }

    /// The bank ladder is a chain of ranges and an off-by-one in any of them is silent: the wrong
    /// bank still contains pic data. These are the boundaries `UncompressMonSprite` names.
    #[test]
    fn the_bank_ladder_matches_uncompress_mon_sprite() {
        use PokemonSpecies::*;
        for (species, bank) in [
            (Tangela, 0x09),   // 0x1E, the last of "Pics 1"
            (Growlithe, 0x0A), // 0x21, the first index past it
            (Moltres, 0x0A),   // 0x49, the last of "Pics 2"
            (Dratini, 0x0B),   // 0x58, inside "Pics 3"
            (Starmie, 0x0C),   // 0x98, the last of "Pics 4"
            (Bulbasaur, 0x0D), // 0x99, the first of "Pics 5"
            (Mew, 0x01),       // 0x15, which would otherwise fall in "Pics 1"
        ] {
            assert_eq!(pic_bank(species), DmgBank::ROM { bank }, "{species} (index {:#04X})", species as u8);
        }
    }
}

