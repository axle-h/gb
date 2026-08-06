/// Byte at header `0x134 + n` summed over 16 bytes, indexed against this table. 94 entries:
/// the first 65 are unambiguous, the remaining 29 share checksums with an earlier
/// entry and are disambiguated by the title's 4th letter.
pub(super) const TITLE_CHECKSUMS: [u8; 94] = [
    0x00, 0x88, 0x16, 0x36, 0xD1, 0xDB, 0xF2, 0x3C, 0x8C, 0x92, 0x3D, 0x5C,
    0x58, 0xC9, 0x3E, 0x70, 0x1D, 0x59, 0x69, 0x19, 0x35, 0xA8, 0x14, 0xAA,
    0x75, 0x95, 0x99, 0x34, 0x6F, 0x15, 0xFF, 0x97, 0x4B, 0x90, 0x17, 0x10,
    0x39, 0xF7, 0xF6, 0xA2, 0x49, 0x4E, 0x43, 0x68, 0xE0, 0x8B, 0xF0, 0xCE,
    0x0C, 0x29, 0xE8, 0xB7, 0x86, 0x9A, 0x52, 0x01, 0x9D, 0x71, 0x9C, 0xBD,
    0x5D, 0x6D, 0x67, 0x3F, 0x6B, 0xB3, 0x46, 0x28, 0xA5, 0xC6, 0xD3, 0x27,
    0x61, 0x18, 0x66, 0x6A, 0xBF, 0x0D, 0xF4, 0xB3, 0x46, 0x28, 0xA5, 0xC6,
    0xD3, 0x27, 0x61, 0x18, 0x66, 0x6A, 0xBF, 0x0D, 0xF4, 0xB3,
];

/// The first index in [`TITLE_CHECKSUMS`] whose checksum is ambiguous.
pub(super) const FIRST_DUPLICATE: usize = 65;

/// The 4th title letter that disambiguates each entry from [`FIRST_DUPLICATE`] onwards.
pub(super) const DUPLICATE_4TH_LETTERS: &[u8; 29] = b"BEFAARBEKEK R-URAR INAILICE R";

/// Palette-combination index for each [`TITLE_CHECKSUMS`] entry. SameBoy's `$80` flag
/// ("game requires the DMG boot tilemap") is stripped: it selects boot *artwork*, not colour.
pub(super) const COMBINATION_PER_CHECKSUM: [u8; 94] = [
     0,  4,  5, 35, 34,  3, 31, 15, 10,  5, 19, 36,
     7, 37, 30, 44, 21, 32, 31, 20,  5, 33, 13, 14,
     5, 29,  5, 18,  9,  3,  2, 26, 25, 25, 41, 42,
    26, 45, 42, 45, 36, 38, 26, 42, 30, 41, 34, 34,
     5, 42,  6,  5, 33, 25, 42, 42, 40,  2, 16, 25,
    42, 42,  5,  0, 39, 36, 22, 25,  6, 32, 12, 36,
    11, 39, 18, 39, 24, 31, 50, 17, 46,  6, 27,  0,
    47, 41, 41,  0,  0, 19, 34, 23, 18, 29,
];

/// `(obj0, obj1, bg)` as **byte** offsets into [`PALETTES`]. Most are palette-aligned;
/// a few (22, 34, 35, 36) deliberately start mid-palette and read across the boundary,
/// which is why this is a flat colour array rather than a 2-D one.
pub(super) const COMBINATIONS: [[u8; 3]; 51] = [
    [ 32,  32, 232], // 0
    [144, 144, 144], // 1
    [160, 160, 160], // 2
    [192, 192, 192], // 3
    [ 72,  72,  72], // 4
    [  0,   0,   0], // 5
    [216, 216, 216], // 6
    [ 40,  40,  40], // 7
    [ 96,  96,  96], // 8
    [208, 208, 208], // 9
    [128,  64,  64], // 10
    [ 32, 224, 224], // 11
    [ 32,  16,  16], // 12
    [ 24,  32,  32], // 13
    [ 32, 232, 232], // 14
    [224,  32, 224], // 15
    [ 16, 136,  16], // 16
    [128, 128,  64], // 17
    [ 32,  32,  56], // 18
    [ 32,  32, 144], // 19
    [ 32,  32, 160], // 20
    [152, 152,  72], // 21
    [ 30,  30,  88], // 22
    [136, 136,  16], // 23
    [ 32,  32,  16], // 24
    [ 32,  32,  24], // 25
    [224, 224,   0], // 26
    [ 24,  24,   0], // 27
    [  0,   0,   8], // 28
    [144, 176, 144], // 29
    [160, 176, 160], // 30
    [192, 176, 192], // 31
    [128, 176,  64], // 32
    [136,  32, 104], // 33
    [222,   0, 112], // 34
    [222,  32, 120], // 35
    [152, 182,  72], // 36
    [128, 224,  80], // 37
    [ 32, 184, 224], // 38
    [136, 176,  16], // 39
    [ 32,   0,  16], // 40
    [ 32, 224,  24], // 41
    [224,  24,   0], // 42
    [ 24, 224,  32], // 43
    [168, 224,  32], // 44
    [ 24, 224,   0], // 45
    [200,  24, 224], // 46
    [  0, 224,  64], // 47
    [ 32,  24, 224], // 48
    [224,  24,  48], // 49
    [ 32, 224, 232], // 50
];

/// The boot ROM's colour pool: 30 four-colour palettes, flattened. RGB555.
pub(super) const PALETTES: [u16; 120] = [
    0x7FFF, 0x32BF, 0x00D0, 0x0000, // 0
    0x639F, 0x4279, 0x15B0, 0x04CB, // 1
    0x7FFF, 0x6E31, 0x454A, 0x0000, // 2
    0x7FFF, 0x1BEF, 0x0200, 0x0000, // 3
    0x7FFF, 0x421F, 0x1CF2, 0x0000, // 4
    0x7FFF, 0x5294, 0x294A, 0x0000, // 5
    0x7FFF, 0x03FF, 0x012F, 0x0000, // 6
    0x7FFF, 0x03EF, 0x01D6, 0x0000, // 7
    0x7FFF, 0x42B5, 0x3DC8, 0x0000, // 8
    0x7E74, 0x03FF, 0x0180, 0x0000, // 9
    0x67FF, 0x77AC, 0x1A13, 0x2D6B, // 10
    0x7ED6, 0x4BFF, 0x2175, 0x0000, // 11
    0x53FF, 0x4A5F, 0x7E52, 0x0000, // 12
    0x4FFF, 0x7ED2, 0x3A4C, 0x1CE0, // 13
    0x03ED, 0x7FFF, 0x255F, 0x0000, // 14
    0x036A, 0x021F, 0x03FF, 0x7FFF, // 15
    0x7FFF, 0x01DF, 0x0112, 0x0000, // 16
    0x231F, 0x035F, 0x00F2, 0x0009, // 17
    0x7FFF, 0x03EA, 0x011F, 0x0000, // 18
    0x299F, 0x001A, 0x000C, 0x0000, // 19
    0x7FFF, 0x027F, 0x001F, 0x0000, // 20
    0x7FFF, 0x03E0, 0x0206, 0x0120, // 21
    0x7FFF, 0x7EEB, 0x001F, 0x7C00, // 22
    0x7FFF, 0x3FFF, 0x7E00, 0x001F, // 23
    0x7FFF, 0x03FF, 0x001F, 0x0000, // 24
    0x03FF, 0x001F, 0x000C, 0x0000, // 25
    0x7FFF, 0x033F, 0x0193, 0x0000, // 26
    0x0000, 0x4200, 0x037F, 0x7FFF, // 27
    0x7FFF, 0x7E8C, 0x7C00, 0x0000, // 28
    0x7FFF, 0x1BEF, 0x6180, 0x0000, // 29
];
