//! The window composed in memory, RGBA row by row, so the same picture goes to SDL and to a PNG.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        (self.x..self.right()).contains(&x) && (self.y..self.bottom()).contains(&y)
    }

    pub fn intersect(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        Rect::new(x, y, (right - x).max(0) as u32, (bottom - y).max(0) as u32)
    }
}

pub type Rgb = [u8; 3];

pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Pixels {
    pub const BYTES_PER_PIXEL: usize = 4;

    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, rgba: vec![0; width as usize * height as usize * Self::BYTES_PER_PIXEL] }
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    pub fn pitch(&self) -> usize {
        self.width as usize * Self::BYTES_PER_PIXEL
    }

    fn offset(&self, x: i32, y: i32) -> usize {
        (y as usize * self.width as usize + x as usize) * Self::BYTES_PER_PIXEL
    }

    pub fn fill(&mut self, rect: Rect, colour: Rgb) {
        let rect = rect.intersect(&self.bounds());
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                let at = self.offset(x, y);
                self.rgba[at..at + 4].copy_from_slice(&[colour[0], colour[1], colour[2], 0xff]);
            }
        }
    }

    /// `colour` laid over the pixel at `coverage` of 255, if the pixel is inside `clip`.
    pub fn blend(&mut self, x: i32, y: i32, colour: Rgb, coverage: u8, clip: &Rect) {
        if coverage == 0 || !clip.contains(x, y) || !self.bounds().contains(x, y) {
            return;
        }
        let at = self.offset(x, y);
        for (channel, &value) in self.rgba[at..at + 3].iter_mut().zip(&colour) {
            *channel = ((value as u32 * coverage as u32 + *channel as u32 * (255 - coverage as u32)) / 255) as u8;
        }
    }

    /// An RGBA picture of `width` pixels a row, each drawn `scale` by `scale` from (`x`, `y`).
    pub fn blit_scaled(&mut self, picture: &[u8], width: usize, x: i32, y: i32, scale: u32) {
        let height = picture.len() / Self::BYTES_PER_PIXEL / width;
        let scale = scale as usize;
        for row in 0..height * scale {
            let to_y = y + row as i32;
            if !(0..self.height as i32).contains(&to_y) {
                continue;
            }
            let from_row = &picture[row / scale * width * Self::BYTES_PER_PIXEL..][..width * Self::BYTES_PER_PIXEL];
            for column in 0..width * scale {
                let to_x = x + column as i32;
                if !(0..self.width as i32).contains(&to_x) {
                    continue;
                }
                let from = column / scale * Self::BYTES_PER_PIXEL;
                let at = self.offset(to_x, to_y);
                self.rgba[at..at + 3].copy_from_slice(&from_row[from..from + 3]);
                self.rgba[at + 3] = 0xff;
            }
        }
    }

    pub fn save_png(&self, path: &std::path::Path) -> Result<(), String> {
        image::save_buffer(path, &self.rgba, self.width, self.height, image::ExtendedColorType::Rgba8)
            .map_err(|e| format!("could not write {}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scaled_picture_is_drawn_in_blocks_and_clipped_to_the_window() {
        let mut pixels = Pixels::new(5, 3);
        // Two pixels, red then green, drawn at 2x from (1, 1): rows 1 and 2, columns 1 to 4.
        pixels.blit_scaled(&[255, 0, 0, 255, 0, 255, 0, 255], 2, 1, 1, 2);
        let at = |x: usize, y: usize| &pixels.rgba[(y * 5 + x) * 4..][..3];
        assert_eq!(at(0, 1), [0, 0, 0]);
        assert_eq!(at(2, 2), [255, 0, 0]);
        assert_eq!(at(3, 1), [0, 255, 0]);
        assert_eq!(at(4, 2), [0, 255, 0]);
        assert_eq!(at(1, 0), [0, 0, 0]);
    }
}
