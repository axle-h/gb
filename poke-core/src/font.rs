use crate::gfx::font::FONT;

/// `FontGraphics`, 1bpp, as `CopyVideoDataDouble` writes it: each byte serves as both planes.
pub const FONT_BYTES: [u8; 2 * FONT.len()] = {
    let mut result = [0; 2 * FONT.len()];
    let mut i = 0;
    while i < FONT.len() {
        result[i * 2] = FONT[i];
        result[i * 2 + 1] = FONT[i];
        i += 1;
    }
    result
};

pub fn render_font_string(indexes: &[usize], menu_characters: bool) -> String {
    let mut result = String::new();
    for &index in indexes {
        match index {
            0..=25 => result.push((b'A' + index as u8) as char),
            26 => result.push('('),
            27 => result.push(')'),
            28 => result.push(':'),
            29 => result.push(';'),
            30 => result.push('['),
            31 => result.push(']'),
            32..=57 => result.push((b'a' + index as u8 - 32) as char),
            58 => result.push('é'),
            59 => result += "'d",
            60 => result += "'l",
            61 => result += "'s",
            62 => result += "'t",
            63 => result += "'v",
            64..=95 => result.push(' '),
            // Glyph 96 is `'`, not `,`.
            96 => result.push('\''),
            97 => result += "Poké",
            98 => result += "mon",
            99 => result.push('-'),
            100 => result += "'r",
            101 => result += "'m",
            102 => result.push('?'),
            103 => result.push('!'),
            104 => result.push('.'),
            105 => result += "ァ",
            106 => result += "ゥ",
            107 => result += "ェ",
            // These are used for menu cursors
            108 if menu_characters => result += "▷",
            109 if menu_characters => result += "▶",
            110 if menu_characters => result += "▼",
            111 => result += "♂",
            112 => result += "¥",
            113 => result += "⨯",
            114 => result.push('.'),
            115 => result.push('/'),
            116 => result.push(','),
            117 => result.push('♀'),
            118..=127 => result.push((b'0' + index as u8 - 118) as char),
            _ => {}
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_bytes_is_correct_length() {
        assert_eq!(FONT_BYTES.len(), 0x800);
        assert!(FONT_BYTES.iter().any(|&b| b > 0), "font_bytes should not be all zeros");
    }

    #[test]
    fn font_bytes_duplicates_correctly() {
        for i in 0..0x400 {
            assert_eq!(FONT_BYTES[i * 2], FONT_BYTES[i * 2 + 1]);
        }
    }

    #[test]
    fn font_bytes_are_correct() {
        // Validate the first 10 bytes
        let expected = [0x10, 0x10, 0x28, 0x28, 0x28, 0x28, 0x44, 0x44, 0x7c, 0x7c];
        for i in 0..10 {
            assert_eq!(FONT_BYTES[i], expected[i], "Byte {} does not match expected value", i);
        }
    }
}
