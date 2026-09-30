include!(concat!(env!("OUT_DIR"), "/charmap.rs"));

/// Encodes `text` as rgbasm's `charmap` does: the longest entry matching at each position.
pub fn encode(text: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let (key, byte) = CHARMAP.iter()
            .filter(|(key, _)| rest.starts_with(key))
            .max_by_key(|(key, _)| key.len())
            .ok_or_else(|| format!("no charmap entry at {rest:?}"))?;
        bytes.push(*byte);
        rest = &rest[key.len()..];
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_and_letters() {
        assert_eq!(encode("Hi<LINE>!@").unwrap(), [0x87, 0xA8, 0x4F, 0xE7, 0x50]);
    }

    #[test]
    fn the_longest_entry_wins() {
        assert_eq!(encode("'d").unwrap(), [0xBB]);
        assert_eq!(encode("'").unwrap(), [0xE0]);
    }

    /// Each byte's text encodes back to it, which is what makes the pick one text per byte.
    #[test]
    fn every_byte_s_text_encodes_to_it() {
        for (byte, text) in TEXT_OF.iter().enumerate().filter_map(|(byte, text)| Some((byte, (*text)?))) {
            assert_eq!(encode(text), Ok(vec![byte as u8]), "{text:?}");
        }
    }

    /// Where the charmap names a byte twice, the English font's glyph is its text.
    #[test]
    fn a_byte_with_two_keys_reads_as_the_font_draws_it() {
        assert_eq!([0x80, 0xE0, 0xE6, 0xED, 0xF0].map(|byte| TEXT_OF[byte]), [Some("A"), Some("'"), Some("?"), Some("▶"), Some("¥")]);
    }

    #[test]
    fn an_unknown_character_is_an_error() {
        assert!(encode("~").is_err());
    }
}
