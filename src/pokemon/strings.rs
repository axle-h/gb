use std::fmt::{Debug, Display};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, PartialEq, Eq)]
pub struct PokemonString(pub Vec<u8>);

impl PokemonString {
    pub const TERMINATOR: u8 = 0x50;

    pub fn from_slice(slice: &[u8]) -> Self {
        // read until terminated
        let mut vec = Vec::new();
        for &b in slice {
            if b == Self::TERMINATOR {
                break;
            }
            println!("{:2x}", b);
            vec.push(b);
        }
        PokemonString(vec)
    }

    pub fn from_string(string: &str) -> Self {
        // https://bulbapedia.bulbagarden.net/wiki/Character_encoding_(Generation_I)
        let graphemes = string.graphemes(true);
        let mut vec = Vec::new();
        for grapheme in graphemes {
            let byte = if grapheme.bytes().count() > 1 {
                // unicode
                match grapheme {
                    "ァ" => 0xE9,
                    "ゥ" => 0xEA,
                    "ェ" => 0xEB,
                    "▷" => 0xEC,
                    "▶" => 0xED,
                    "▼" => 0xEE,
                    "♂" => 0xEF,
                    "×" => 0xF1,
                    "♀" => 0xF5,
                    _ => 0x00
                }
            } else {
                // ascii
                let char = grapheme.bytes().next().unwrap();
                match char {
                    b'A'..=b'Z' => (char - b'A') + 0x80,
                    b'a'..=b'z' => (char - b'a') + 0xA0,
                    b'0'..=b'9' => (char - b'0') + 0xF6,
                    b'(' => 0x9A,
                    b')' => 0x9B,
                    b':' => 0x9C,
                    b';' => 0x9D,
                    b'[' => 0x9E,
                    b']' => 0x9F,
                    b'\'' => 0xE0,
                    b'-' => 0xE3,
                    b'?' => 0xE6,
                    b'!' => 0xE7,
                    b'.' => 0xE8,
                    b'/' => 0xF3,
                    b',' => 0xF4,
                    b' ' => 0x7F,
                    _ => 0x00
                }
            };
            vec.push(byte);
        }
        vec.push(Self::TERMINATOR);
        PokemonString(vec)
    }

    pub fn to_string(&self, trainer_name: &str, rival_name: &str) -> Result<String, String> {
        // https://bulbapedia.bulbagarden.net/wiki/Character_encoding_(Generation_I)
        let mut utf8 = vec![];
        let mut last_char_empty = false;
        for &byte in self.0.iter() {
            match byte {
                0x4A => utf8.extend_from_slice("Pokémon".as_bytes()), // pk mn characters
                0x50 => break, // end: marks the end of a string
                0x52 => utf8.extend_from_slice(trainer_name.as_bytes()), // trainer name
                0x53 => utf8.extend_from_slice(rival_name.as_bytes()), // rival name
                0x56 => utf8.extend_from_slice("……".as_bytes()),
                0x59 => utf8.extend_from_slice("<TARGET>".as_bytes()), // TODO PlaceMoveTargetsName::
                0x5A => utf8.extend_from_slice("<USER>".as_bytes()), // TODO PlaceMoveUsersName::
                0x5B => utf8.extend_from_slice("PC".as_bytes()),
                0x5C => utf8.extend_from_slice("TM".as_bytes()),
                0x5D => utf8.extend_from_slice("TRAINER".as_bytes()),
                0x5E => utf8.extend_from_slice("ROCKET".as_bytes()),
                0x80..=0x99 => utf8.push(byte - 0x80 + b'A'), // A-Z
                0x9A => utf8.push(b'('),
                0x9B => utf8.push(b')'),
                0x9C => utf8.push(b':'),
                0x9D => utf8.push(b';'),
                0x9E => utf8.push(b'['),
                0x9F => utf8.push(b']'),
                0xA0..=0xB9 => utf8.push(byte - 0xA0 + b'a'), // a-z
                0xBA => utf8.push(b'e'),
                0xBB => utf8.push(b'd'),
                0xBC => utf8.push(b'l'),
                0xBD => utf8.push(b's'),
                0xBE => utf8.push(b't'),
                0xBF => utf8.push(b'v'),
                0xE0 => utf8.push(b'\''),
                0xE1 => utf8.extend_from_slice("Poké".as_bytes()), // pk character
                0xE2 => utf8.extend_from_slice("mon".as_bytes()), // mn character
                0xE3 => utf8.push(b'-'),
                0xE4 => utf8.push(b'r'),
                0xE5 => utf8.push(b'm'),
                0xE6 => utf8.push(b'?'),
                0xE7 => utf8.push(b'!'),
                0xE8 => utf8.push(b'.'),
                0xE9 => utf8.extend_from_slice("ァ".as_bytes()),
                0xEA => utf8.extend_from_slice("ゥ".as_bytes()),
                0xEB => utf8.extend_from_slice("ェ".as_bytes()),
                0xEC => utf8.extend_from_slice("▷".as_bytes()),
                0xED => utf8.extend_from_slice("▶".as_bytes()),
                0xEE => utf8.extend_from_slice("▼".as_bytes()),
                0xEF => utf8.extend_from_slice("♂".as_bytes()),
                0xF1 => utf8.extend_from_slice("×".as_bytes()),
                0xF2 => utf8.push(b'.'),
                0xF3 => utf8.push(b'/'),
                0xF4 => utf8.push(b','),
                0xF5 => utf8.extend_from_slice("♀".as_bytes()),
                0xF6..=0xFF => utf8.push(byte - 0xF6 + b'0'), // 0-9
                _ => {
                    if !last_char_empty {
                        utf8.push(b' ');
                        last_char_empty = true;
                    }
                    continue;
                }
            }
            last_char_empty = false;
        }
        std::str::from_utf8(&utf8)
            .map_err(|_| "Invalid UTF-8 in string".to_string())
            .map(|s| s.trim().to_string())
    }
}

impl Display for PokemonString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_string("Ash", "Gary").unwrap())
    }
}

impl Debug for PokemonString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_string("Ash", "Gary").unwrap())
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