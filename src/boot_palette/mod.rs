//! The CGB boot ROM's DMG-compatibility palettes.
//!
//! A cartridge whose header byte `0x143` has bit 7 clear is a DMG-only game. Run one on a Game Boy
//! Color and the **boot ROM**, not the game, picks a three-palette set from the cartridge title and
//! writes it into CGB palette RAM before handing over. That is the whole reason Pokémon Red is
//! red-tinted on a CGB and greyscale on a DMG.
//!
//! # Where the tables come from
//!
//! [`tables`] is generated mechanically from **SameBoy's `BootROMs/cgb_boot.asm`** (`master`,
//! fetched 2026-08-05) — a reimplementation of the boot ROM that assembles to the same data. It is
//! the authoritative source the plan names; this gambatte checkout does **not** implement the
//! feature at all (it renders DMG mode through a flat greyscale ramp, `video.cpp:126-128`), so it
//! could not be used as a reference here.
//!
//! Two deliberate departures from that file, both noted where they occur: SameBoy's four
//! "exclusive" palette combinations and two extra palettes are dropped, because they are SameBoy
//! additions rather than boot-ROM data; and the `$80` flag on a combination index is stripped,
//! because it selects boot *artwork*, not colour.
//!
//! # Not implemented: the button-combination overrides
//!
//! Holding a direction plus A/B during boot picks one of twelve alternate palettes
//! (`KeyCombinationPalettes`). `gb` starts the cartridge directly rather than emulating the boot
//! ROM, so there is no window during which a combination could be held. Skipped deliberately — it
//! is a user-facing convenience, not accuracy, and nothing in the plan depends on it.

mod tables;

use tables::*;

/// The three palettes the boot ROM writes: eight raw RGB555 bytes each, ready for
/// [`crate::cgb_palette::PaletteBank::set_palette`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootPalettes {
    /// CGB BG palette 0.
    pub background: [u8; 8],
    /// CGB OBJ palette 0 — selected when the sprite's `OBP0`/`OBP1` bit is clear.
    pub object0: [u8; 8],
    /// CGB OBJ palette 1.
    pub object1: [u8; 8],
    /// Which entry of `PaletteCombinations` this is. Diagnostics and tests.
    pub combination: u8,
}

/// The palette set a real CGB boot ROM would install for this cartridge.
///
/// Falls back to combination 0 — the boot ROM's own default — for a non-Nintendo cartridge or a
/// title checksum that is not in the table, which is what the ROM does (`GetPaletteIndex`
/// returns 0 from both `.notNintendo` and the end of `.searchLoop`).
pub fn for_cartridge(rom: &[u8]) -> BootPalettes {
    let combination = combination_index(rom).unwrap_or(0);
    let [object0, object1, background] = COMBINATIONS[combination as usize];
    BootPalettes {
        background: colors_at(background),
        object0: colors_at(object0),
        object1: colors_at(object1),
        combination,
    }
}

/// The title checksum: the low byte of the sum of header bytes `0x134..=0x143`. For
/// `POKEMON RED` this is `0x14`.
pub fn title_checksum(rom: &[u8]) -> u8 {
    rom.get(0x134..0x144)
        .map(|title| title.iter().fold(0u8, |sum, &b| sum.wrapping_add(b)))
        .unwrap_or(0)
}

/// What the boot ROM leaves in **`B`** when handing a DMG-only cartridge to CGB hardware: the
/// title checksum for a first-party cartridge, `0x00` for anything else.
///
/// This is a register value rather than a palette, but it comes from the same two header checks
/// — the checksum is computed by `GetPaletteIndex` and left in `hTitleChecksum`, which the boot
/// ROM's last block loads into `B` (SameBoy `cgb_boot.asm`, `Preboot`). Keeping it here means the
/// licensee rule lives in exactly one place. See [`crate::registers::RegisterSet::boot`].
pub fn compatibility_b_register(rom: &[u8]) -> u8 {
    if is_nintendo(rom) { title_checksum(rom) } else { 0 }
}

/// `None` when the boot ROM would fall through to its default — either the cartridge is not
/// first-party or its checksum is unknown.
fn combination_index(rom: &[u8]) -> Option<u8> {
    if !is_nintendo(rom) {
        return None;
    }
    let checksum = title_checksum(rom);
    // The 4th title letter, consulted only for the ambiguous tail of the table.
    let fourth = *rom.get(0x137)?;

    TITLE_CHECKSUMS
        .iter()
        .enumerate()
        .find(|&(index, &entry)| {
            entry == checksum
                && (index < FIRST_DUPLICATE
                    || DUPLICATE_4TH_LETTERS[index - FIRST_DUPLICATE] == fourth)
        })
        .map(|(index, _)| COMBINATION_PER_CHECKSUM[index])
}

/// The boot ROM only colours first-party cartridges: old licensee `0x01`, or the escape value
/// `0x33` with new licensee `"01"` (`GetPaletteIndex`).
fn is_nintendo(rom: &[u8]) -> bool {
    match rom.get(0x14B) {
        Some(0x33) => rom.get(0x144..0x146) == Some(b"01"),
        Some(0x01) => true,
        _ => false,
    }
}

/// Four consecutive colours starting at a **byte** offset into [`PALETTES`]. A handful of
/// combinations start mid-palette on purpose and read across the boundary.
fn colors_at(offset: u8) -> [u8; 8] {
    let first = offset as usize / 2;
    let mut bytes = [0u8; 8];
    for i in 0..4 {
        let color = PALETTES[first + i];
        bytes[i * 2] = color as u8;
        bytes[i * 2 + 1] = (color >> 8) as u8;
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⭐ The headline case. `POKEMON RED` has no CGB flag, so a real Game Boy Color colours it
    /// from the title checksum alone. Every step is asserted, because the next agent should not
    /// have to re-derive any of it.
    #[test]
    fn pokemon_red_resolves_to_combination_13() {
        let rom = crate::pokemon::roms::POKERED;
        assert_eq!(&rom[0x134..0x13F], b"POKEMON RED");
        assert_eq!(rom[0x143], 0x00, "DMG-only cartridge");
        assert_eq!(title_checksum(rom), 0x14);
        assert_eq!(rom[0x137], b'E', "4th letter, unused here — 0x14 is unambiguous");

        let palettes = for_cartridge(rom);
        assert_eq!(palettes.combination, 13, "SameBoy: `palette_index 13 ; POKEMON RED`");

        // Combination 13 is `palette_comb 3, 4, 4`: OBJ0 from pool palette 3, OBJ1 and BG from 4.
        assert_eq!(palettes.object0, [0xFF, 0x7F, 0xEF, 0x1B, 0x00, 0x02, 0x00, 0x00]);
        assert_eq!(palettes.object1, [0xFF, 0x7F, 0x1F, 0x42, 0xF2, 0x1C, 0x00, 0x00]);
        assert_eq!(palettes.background, palettes.object1);

        // ...which is the red ramp: white, salmon, dark red, black.
        use crate::lcd_palette::LcdColor;
        assert_eq!(LcdColor::from_rgb555(0x7FFF), LcdColor::rgb(0xFF, 0xFF, 0xFF));
        assert_eq!(LcdColor::from_rgb555(0x421F), LcdColor::rgb(0xFF, 0x84, 0x84));
        assert_eq!(LcdColor::from_rgb555(0x1CF2), LcdColor::rgb(0x94, 0x39, 0x39));
        assert_eq!(LcdColor::from_rgb555(0x0000), LcdColor::rgb(0x00, 0x00, 0x00));
    }

    /// The ambiguous tail: `TETRIS ATTACK` and `MOGURANYA` both check out at `0xB3`, and only the
    /// 4th letter tells them apart. If the disambiguation is dropped, both land on the first match.
    #[test]
    fn the_fourth_letter_separates_duplicate_checksums() {
        fn resolve(title: &str, licensee_escape: bool) -> Option<u8> {
            let mut rom = vec![0u8; 0x150];
            rom[0x134..0x134 + title.len()].copy_from_slice(title.as_bytes());
            if licensee_escape {
                rom[0x14B] = 0x33;
                rom[0x144..0x146].copy_from_slice(b"01");
            } else {
                rom[0x14B] = 0x01;
            }
            combination_index(&rom)
        }

        // Both titles are padded to 16 bytes with NULs, so the checksum is the letters alone.
        assert_eq!(title_checksum_of("MOGURANYA"), 0xB3);
        assert_eq!(title_checksum_of("TETRIS ATTACK"), 0xB3);
        assert_eq!(resolve("MOGURANYA", false), Some(17), "4th letter 'U'");
        assert_eq!(resolve("TETRIS ATTACK", true), Some(29), "4th letter 'R'");
        // Same checksum, a 4th letter matching neither -> no entry, so the default.
        assert_eq!(resolve("MOGXRANYA", false), None);
    }

    fn title_checksum_of(title: &str) -> u8 {
        title.bytes().fold(0u8, |sum, b| sum.wrapping_add(b))
    }

    /// A third-party cartridge is never coloured, however familiar its title.
    #[test]
    fn non_nintendo_cartridges_get_the_default() {
        let mut rom = crate::pokemon::roms::POKERED.to_vec();
        rom[0x14B] = 0x9C; // some other publisher
        assert_eq!(combination_index(&rom), None);
        assert_eq!(for_cartridge(&rom).combination, 0);
    }

    /// Every combination must address four colours that exist, including the four that
    /// deliberately start mid-palette.
    #[test]
    fn every_combination_is_in_range() {
        for (i, combination) in COMBINATIONS.iter().enumerate() {
            for &offset in combination {
                assert_eq!(offset % 2, 0, "combination {i} offset {offset} is not colour-aligned");
                assert!(
                    offset as usize / 2 + 4 <= PALETTES.len(),
                    "combination {i} offset {offset} runs past the palette pool"
                );
            }
        }
        for (i, &combination) in COMBINATION_PER_CHECKSUM.iter().enumerate() {
            assert!((combination as usize) < COMBINATIONS.len(), "checksum entry {i}");
        }
        assert_eq!(TITLE_CHECKSUMS.len(), COMBINATION_PER_CHECKSUM.len());
        assert_eq!(DUPLICATE_4TH_LETTERS.len(), TITLE_CHECKSUMS.len() - FIRST_DUPLICATE);
    }
}
