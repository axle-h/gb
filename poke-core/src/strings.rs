use std::fmt::{Debug, Display};
use unicode_segmentation::UnicodeSegmentation;
use crate::charmap::{encode, TEXT_OF};
use crate::gfx::font::FONT;

#[derive(Clone, PartialEq, Eq, Default)]
pub struct PokemonString(pub Vec<u8>);

impl PokemonString {
    pub const TERMINATOR: u8 = 0x50;

    pub fn from_slice(slice: &[u8]) -> Self {
        // Read until terminated
        let mut vec = Vec::new();
        for &b in slice {
            if b == Self::TERMINATOR {
                break;
            }
            vec.push(b);
        }
        PokemonString(vec)
    }

    /// Each grapheme as the byte whose text it is, and `$00` for one that is no byte's.
    pub fn from_string(string: &str) -> Self {
        let mut vec: Vec<u8> = string.graphemes(true).map(byte_of).collect();
        vec.push(Self::TERMINATOR);
        PokemonString(vec)
    }

    pub fn to_default_string(&self) -> String {
        self.to_string("Ash", "Gary").unwrap()
    }

    /// The string as a reader wants it: the names and words the control codes stand for spelled
    /// out, and a run of anything the font does not draw as one space.
    pub fn to_string(&self, trainer_name: &str, rival_name: &str) -> Result<String, String> {
        let mut text = String::new();
        let mut last_char_empty = false;
        for &byte in self.0.iter() {
            let spelled = match TEXT_OF[byte as usize] {
                Some("@") => break,
                Some("<PLAYER>") => trainer_name,
                Some("<RIVAL>") => rival_name,
                Some("<PKMN>") => "Pokémon",
                Some("<PK>") => "Poké",
                Some("<MN>") => "mon",
                Some("<DOT>") => ".",
                Some("<……>") => "……",
                Some("<PC>") => "PC",
                Some("<TM>") => "TM",
                Some("<TRAINER>") => "TRAINER",
                Some("<ROCKET>") => "ROCKET",
                Some(code @ ("<TARGET>" | "<USER>")) => code,
                Some(glyph) if drawn(byte) && !glyph.starts_with('<') => glyph,
                _ => {
                    if !last_char_empty {
                        text.push(' ');
                        last_char_empty = true;
                    }
                    continue;
                }
            };
            text.push_str(spelled);
            last_char_empty = false;
        }
        Ok(text.trim().to_string())
    }
}

/// The byte `grapheme` is the text of, or `$00`.
fn byte_of(grapheme: &str) -> u8 {
    match encode(grapheme).as_deref() {
        Ok(&[byte]) if TEXT_OF[byte as usize] == Some(grapheme) => byte,
        _ => 0x00,
    }
}

/// Whether the font has a glyph for `byte` that is not blank: it starts at `$80`, and the kana
/// Red leaves untranslated are blank tiles in it.
fn drawn(byte: u8) -> bool {
    const FIRST_GLYPH: u8 = 0x80;
    const GLYPH_BYTES: usize = 8;
    byte.checked_sub(FIRST_GLYPH).is_some_and(|glyph| FONT[glyph as usize * GLYPH_BYTES..][..GLYPH_BYTES].iter().any(|&row| row != 0))
}

impl Display for PokemonString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_default_string())
    }
}

impl Debug for PokemonString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_default_string())
    }
}

impl From<&str> for PokemonString {
    fn from(s: &str) -> Self {
        Self::from_string(s)
    }
}

impl From<String> for PokemonString {
    fn from(s: String) -> Self {
        Self::from_string(&s)
    }
}

impl From<&[u8]> for PokemonString {
    fn from(s: &[u8]) -> Self {
        Self::from_slice(s)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_reads_back_as_it_was_written() {
        for name in ["NIDORAN♂", "MR.MIME", "FARFETCH'D", "Lv 12 (x/y)?!", "▶ ♀-9"] {
            assert_eq!(PokemonString::from_string(name).to_default_string(), name);
        }
    }

    #[test]
    fn control_codes_are_spelled_out() {
        let bytes = encode("<PLAYER> and <RIVAL>, <PKMN> <PK><MN> <TM>@").unwrap();
        assert_eq!(PokemonString(bytes).to_string("RED", "BLUE").unwrap(), "RED and BLUE, Pokémon Pokémon TM");
    }

    /// Katakana shares its bytes with the English letters, and the kana the font leaves blank read
    /// as one space.
    #[test]
    fn what_the_font_does_not_draw_is_not_read() {
        assert_eq!(PokemonString::from_string("アA").0, [0x00, 0x80, PokemonString::TERMINATOR]);
        assert_eq!(PokemonString(encode("A<LINE>たちB").unwrap()).to_default_string(), "A B");
    }
}
