//! A frame as the smallest PNG it fits in: indexed at the fewest bits its colours need, which on DMG
//! and SGB is two, and RGB only for a frame of more than 256 colours.

use std::collections::HashMap;

use crate::lcd_palette::LcdColor;

pub fn png(pixels: &[LcdColor], width: u32, height: u32) -> Vec<u8> {
    assert_eq!(pixels.len(), (width * height) as usize, "a {width}x{height} frame");
    let mut index: HashMap<LcdColor, u8> = HashMap::new();
    let mut palette = Vec::new();
    for &pixel in pixels {
        if !index.contains_key(&pixel) {
            if palette.len() == 256 {
                return rgb(pixels, width, height);
            }
            index.insert(pixel, palette.len() as u8);
            palette.push(pixel);
        }
    }

    let (bits, depth) = match palette.len() {
        0..=2 => (1, png::BitDepth::One),
        3..=4 => (2, png::BitDepth::Two),
        5..=16 => (4, png::BitDepth::Four),
        _ => (8, png::BitDepth::Eight),
    };
    let row_bytes = (width as usize * bits).div_ceil(8);
    let mut packed = vec![0u8; row_bytes * height as usize];
    for (i, pixel) in pixels.iter().enumerate() {
        let (y, x) = (i / width as usize, i % width as usize);
        let bit = x * bits;
        packed[y * row_bytes + bit / 8] |= index[pixel] << (8 - bits - bit % 8);
    }
    let palette: Vec<u8> = palette.iter().flat_map(|colour| colour.to_rgb().0).collect();

    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(depth);
    encoder.set_palette(palette);
    encoder.set_compression(png::Compression::High);
    write(encoder, &packed);
    out
}

fn rgb(pixels: &[LcdColor], width: u32, height: u32) -> Vec<u8> {
    let bytes: Vec<u8> = pixels.iter().flat_map(|colour| colour.to_rgb().0).collect();
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    write(encoder, &bytes);
    out
}

/// Into a `Vec`, which cannot fail to take the bytes.
fn write(encoder: png::Encoder<&mut Vec<u8>>, data: &[u8]) {
    let mut writer = encoder.write_header().expect("a PNG header writes to memory");
    writer.write_image_data(data).expect("a frame of the declared size writes to memory");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(png: &[u8]) -> (png::OutputInfo, Vec<u8>) {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().unwrap();
        let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buffer).unwrap();
        buffer.truncate(info.buffer_size());
        (info, buffer)
    }

    #[test]
    fn a_four_shade_frame_is_two_bit_indexed_and_decodes_to_itself() {
        let shades = [LcdColor::rgb(0xFF, 0xFF, 0xFF), LcdColor::rgb(0xAA, 0xAA, 0xAA),
                      LcdColor::rgb(0x55, 0x55, 0x55), LcdColor::rgb(0, 0, 0)];
        let pixels: Vec<LcdColor> = (0..160 * 144).map(|i| shades[(i * 7 / 13) % 4]).collect();
        let png = png(&pixels, 160, 144);
        let info = png::Decoder::new(std::io::Cursor::new(&png)).read_info().unwrap().info().clone();
        assert_eq!((info.color_type, info.bit_depth), (png::ColorType::Indexed, png::BitDepth::Two));
        let (_, rgb) = decode(&png);
        let expected: Vec<u8> = pixels.iter().flat_map(|c| c.to_rgb().0).collect();
        assert_eq!(rgb, expected);
    }

    #[test]
    fn a_frame_of_more_than_256_colours_is_rgb() {
        let pixels: Vec<LcdColor> = (0..160 * 144u32).map(|i| LcdColor::rgb(i as u8, (i >> 8) as u8, 0)).collect();
        let png = png(&pixels, 160, 144);
        let (info, rgb) = decode(&png);
        assert_eq!(info.color_type, png::ColorType::Rgb);
        assert_eq!(rgb, pixels.iter().flat_map(|c| c.to_rgb().0).collect::<Vec<u8>>());
    }
}
