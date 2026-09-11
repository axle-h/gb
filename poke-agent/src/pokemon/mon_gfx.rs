//! Pokémon front sprites, decompressed out of the cartridge the binary already carries.

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

/// A pic's own dimensions in 8×8 tiles, which is the first byte of its compressed data.
#[cfg(test)]
fn front_pic_size(species: PokemonSpecies) -> (usize, usize) {
    let dimensions = front_pic(species)[0];
    ((dimensions >> 4) as usize, (dimensions & 0xF) as usize)
}

// ── Finding the pic
// ──────────────────────────────────────────────────────────────────────────────

/// The compressed pic, as a slice running to the end of its bank.
fn front_pic(species: PokemonSpecies) -> &'static [u8] {
    let entry = base_stats_entry(species);
    let address = u16::from_le_bytes([entry[BASE_FRONTPIC], entry[BASE_FRONTPIC + 1]]);
    rom_slice(DmgPointer { bank: pic_bank(species), address })
}

/// The 28-byte base-stats entry.
pub(crate) fn base_stats_entry(species: PokemonSpecies) -> &'static [u8] {
    let pointer = if species == PokemonSpecies::Mew {
        pokered_symbols::MewBaseStats
    } else {
        let dex = species.metadata().pokedex_number as u16;
        pokered_symbols::BaseStats + (dex - 1) * BASE_DATA_SIZE as u16
    };
    &rom_slice(pointer)[..BASE_DATA_SIZE]
}

/// `UncompressMonSprite`'s bank ladder, on the internal index rather than the Pokédex number.
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

// ── The bitstream
// ────────────────────────────────────────────────────────────────────────────────

/// `ReadNextInputBit`: MSB-first, one byte at a time.
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

// ── Decompression
// ────────────────────────────────────────────────────────────────────────────────

/// Two 1bpp planes and the size they cover, each plane `width * height * 8` bytes packed as
/// `width` columns of `height * 8` rows.
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

    // `wSpriteLoadFlags` bit 0: which plane the first chunk goes to.
    let first = usize::from(input.bit() == 1);
    let mut planes = [vec![0u8; width * rows], vec![0u8; width * rows]];

    read_chunk(&mut input, &mut planes[first], width, rows);
    // The mode is read at the top of the *second* chunk, after the first has been consumed — not
    // up front with the dimensions and the plane bit.
    let mode = if input.bit() == 0 { 0 } else { input.bit() + 1 };
    read_chunk(&mut input, &mut planes[1 - first], width, rows);

    // `UnpackSprite`.
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

/// Where the next pixel pair goes: `MoveToNextBufferPosition`, which in the original terminates
/// the decompression loop by unwinding the stack out from under it.
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

/// One 1bpp chunk — `UncompressSpriteDataLoop`.
fn read_chunk(input: &mut BitReader, plane: &mut [u8], width: usize, rows: usize) {
    let mut cursor = Cursor::new(width, rows);
    // One opening bit says which of the two states the chunk starts in.
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

/// `SpriteDifferentialDecode`.
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
    /// `DecodeNybble0Table` / `DecodeNybble1Table` as `(high, low)` pairs — the `dn` macro packs
    /// two nybbles into each byte.
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

// ── Alignment
// ────────────────────────────────────────────────────────────────────────────────────

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

    /// The upstream file name for a species.
    fn sprite_file_name(species: PokemonSpecies) -> String {
        match species {
            PokemonSpecies::NidoranMale => "nidoranm".to_string(),
            PokemonSpecies::NidoranFemale => "nidoranf".to_string(),
            PokemonSpecies::MrMime => "mr.mime".to_string(),
            other => other.metadata().name.to_lowercase(),
        }
    }

    /// `make` leaves the *uncompressed* form of every pic beside the compressed one, so
    /// upstream's own build output is available as an oracle.
    fn upstream_2bpp(species: PokemonSpecies) -> Option<Vec<u8>> {
        std::fs::read(format!("pokered/gfx/pokemon/front/{}.2bpp", sprite_file_name(species))).ok()
    }

    /// Reverse [`front_pic_shades`]'s canvas back into the tile stream a `.2bpp` file holds: each
    /// tile eight `(low, high)` byte pairs.
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

    /// FNV-1a.
    fn checksum(shades: &[u8; PIC_PX * PIC_PX]) -> u32 {
        let mut hash = 0x811C_9DC5u32;
        for &shade in shades.iter() {
            hash = (hash ^ shade as u32).wrapping_mul(0x0100_0193);
        }
        hash
    }

    /// The one test that can *prove* the port rather than merely exercise it.
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

    /// Writes `src/pokemon/data/gfx/front_pic_checksums.bin`.
    #[test]
    #[ignore = "a tool: writes the checksum fixture"]
    #[cfg(feature = "slow-tests")]
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
    /// different Pokémon.
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

    /// The canvas is the game's own: centred horizontally, standing on the bottom edge.
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
    /// bank still contains pic data.
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
