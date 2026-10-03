//! What the window shows, composed into [`Pixels`]: the routing toggle, the tour and speed buttons
//! and a status line across the top, the emulated screen left of the native one, each under its
//! title and the tour's standing on it, and the log beneath.

use gb::ppu::{LCD_HEIGHT, LCD_WIDTH};
use pokered::gfx::colour::ColourMode;
use crate::sdl::font::Text;
use crate::sdl::games::{Games, Side};
use crate::sdl::log::{Log, Source};
use crate::sdl::pixels::{Pixels, Rect, Rgb};
use crate::sdl::routing::Routing;
use crate::sdl::speed::Speed;
use crate::sdl::tour::Standing;

const MARGIN: u32 = 12;
const TOGGLE_WIDTH: u32 = 104;
const TOUR_WIDTH: u32 = 72;
const SPEED_WIDTH: u32 = 100;
const BAR_HEIGHT: u32 = 28;
const TITLE_HEIGHT: u32 = 22;
const LOG_ROWS: u32 = 12;
const FONT_SIZE: f32 = 16.0;

const BACKGROUND: Rgb = [0x20, 0x20, 0x24];
const PANEL: Rgb = [0x12, 0x12, 0x14];
const BUTTON: Rgb = [0x3a, 0x3a, 0x40];
const SELECTED: Rgb = [0xc8, 0x32, 0x32];
/// A toggle that cannot be pressed while a tour plays, and the one it stays on.
const DISABLED: Rgb = [0x2a, 0x2a, 0x2e];
const DISABLED_SELECTED: Rgb = [0x5a, 0x2a, 0x2a];
const LABEL: Rgb = [0xf0, 0xf0, 0xf0];
const DIM: Rgb = [0x9a, 0x9a, 0xa0];
const WAITING: Rgb = [0xff, 0xd0, 0x60];

fn colour(source: Source) -> Rgb {
    match source {
        Source::Emulated => [0xff, 0x8a, 0x80],
        Source::Native => [0x80, 0xc8, 0xff],
        Source::Window => DIM,
    }
}

/// Where everything is, for a scale and the size of the native picture, which the SGB border grows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub scale: u32,
    pub width: u32,
    pub height: u32,
    /// One per [`Routing::ALL`], in order.
    pub toggle: [Rect; 3],
    pub tour: Rect,
    pub speed: Rect,
    pub status: Rect,
    pub emulated: Rect,
    pub native: Rect,
    pub log: Rect,
    pub line_height: u32,
}

impl Layout {
    pub fn new(scale: u32, native: (usize, usize), line_height: u32) -> Self {
        let emulated_size = (LCD_WIDTH as u32 * scale, LCD_HEIGHT as u32 * scale);
        let native_size = (native.0 as u32 * scale, native.1 as u32 * scale);
        let width = MARGIN * 3 + emulated_size.0 + native_size.0;
        let toggle = std::array::from_fn(|i| {
            Rect::new((MARGIN + i as u32 * (TOGGLE_WIDTH + MARGIN / 2)) as i32, MARGIN as i32, TOGGLE_WIDTH, BAR_HEIGHT)
        });
        let tour = Rect::new(toggle[2].right() + MARGIN as i32, MARGIN as i32, TOUR_WIDTH, BAR_HEIGHT);
        let speed = Rect::new(tour.right() + (MARGIN / 2) as i32, MARGIN as i32, SPEED_WIDTH, BAR_HEIGHT);
        let status_x = speed.right() as u32 + MARGIN;
        let status = Rect::new(status_x as i32, MARGIN as i32, width.saturating_sub(status_x + MARGIN), BAR_HEIGHT);
        let screens_y = (MARGIN * 2 + BAR_HEIGHT + TITLE_HEIGHT) as i32;
        let emulated = Rect::new(MARGIN as i32, screens_y, emulated_size.0, emulated_size.1);
        let native = Rect::new(emulated.right() + MARGIN as i32, screens_y, native_size.0, native_size.1);
        let log_y = emulated.bottom().max(native.bottom()) + MARGIN as i32;
        let log = Rect::new(MARGIN as i32, log_y, width - MARGIN * 2, LOG_ROWS * line_height + MARGIN);
        Self { scale, width, height: log.bottom() as u32 + MARGIN, toggle, tour, speed, status, emulated, native, log, line_height }
    }

    /// The largest whole scale, at least 1, whose window fits in `bounds`.
    pub fn fit(bounds: (u32, u32), native: (usize, usize), line_height: u32) -> Self {
        (2..=6)
            .map(|scale| Self::new(scale, native, line_height))
            .take_while(|layout| layout.width <= bounds.0 && layout.height <= bounds.1)
            .last()
            .unwrap_or_else(|| Self::new(1, native, line_height))
    }

    /// The log's text area, inside the panel's padding.
    pub fn log_text(&self) -> Rect {
        let pad = MARGIN as i32 / 2;
        Rect::new(self.log.x + pad, self.log.y + pad, self.log.width - MARGIN, self.log.height - MARGIN)
    }

    pub fn log_rows(&self) -> usize {
        LOG_ROWS as usize
    }

    /// The button at (`x`, `y`).
    pub fn control_at(&self, x: i32, y: i32) -> Option<Control> {
        if let Some(i) = self.toggle.iter().position(|rect| rect.contains(x, y)) {
            Some(Control::Routing(Routing::ALL[i]))
        } else if self.tour.contains(x, y) {
            Some(Control::Tour)
        } else if self.speed.contains(x, y) {
            Some(Control::Speed)
        } else {
            None
        }
    }

    /// The game whose half of the window `x` is in, the halves meeting midway between the screens.
    pub fn side_at(&self, x: i32) -> Side {
        if x < (self.emulated.right() + self.native.x) / 2 { Side::Emulated } else { Side::Native }
    }
}

/// A button in the top bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Routing(Routing),
    /// Starts the grand tour on both games, or stops the one playing.
    Tour,
    /// Cycles [`Speed`].
    Speed,
}

/// Everything about the window that is not a game.
pub struct View {
    pub routing: Routing,
    pub speed: Speed,
    pub colours: ColourMode,
    pub text: Text,
    pub log: Log,
    pub layout: Layout,
    pub pixels: Pixels,
    /// The status line, right of the buttons.
    pub status: String,
}

impl View {
    pub fn new(fit: impl Fn((usize, usize), u32) -> Layout) -> Result<Self, String> {
        let text = Text::roboto(FONT_SIZE)?;
        let colours = ColourMode::Dmg;
        let layout = fit(colours.size(), text.line_height());
        Ok(Self {
            routing: Routing::default(),
            speed: Speed::default(),
            colours,
            log: Log::new(layout.log_text().width),
            pixels: Pixels::new(layout.width, layout.height),
            layout,
            text,
            status: String::new(),
        })
    }

    pub fn say(&mut self, source: Source, line: String) {
        let text = &mut self.text;
        self.log.push(source, &line, &mut |s: &str| text.width(s));
    }

    /// Lay the window out again, for a native picture of a new size.
    pub fn relayout(&mut self, layout: Layout) {
        if layout != self.layout {
            self.pixels = Pixels::new(layout.width, layout.height);
            let text = &mut self.text;
            self.log.rewrap(layout.log_text().width, &mut |s: &str| text.width(s));
            self.layout = layout;
        }
    }

    pub fn compose(&mut self, games: &Games) {
        let layout = self.layout.clone();
        let pixels = &mut self.pixels;
        let text = &mut self.text;
        pixels.fill(pixels.bounds(), BACKGROUND);

        let touring = games.touring();
        let mut button = |rect: Rect, label: &str, fill: Rgb, ink: Rgb| {
            pixels.fill(rect, fill);
            let x = rect.x + (rect.width as i32 - text.width(label) as i32) / 2;
            let y = rect.y + (rect.height as i32 - text.line_height() as i32) / 2;
            text.draw(pixels, label, x, y, ink, rect);
        };
        for (rect, routing) in layout.toggle.iter().zip(Routing::ALL) {
            let fill = match (routing == self.routing, touring) {
                (true, false) => SELECTED,
                (false, false) => BUTTON,
                (true, true) => DISABLED_SELECTED,
                (false, true) => DISABLED,
            };
            button(*rect, routing.label(), fill, if touring { DIM } else { LABEL });
        }
        if touring {
            button(layout.tour, "STOP", SELECTED, LABEL);
        } else {
            button(layout.tour, "TOUR", BUTTON, LABEL);
        }
        button(layout.speed, self.speed.label(), BUTTON, LABEL);
        let status_y = layout.status.y + (layout.status.height as i32 - text.line_height() as i32) / 2;
        text.draw(pixels, &self.status, layout.status.x, status_y, DIM, layout.status);

        let colours = self.colours;
        let mode = match colours {
            ColourMode::Dmg => "DMG",
            ColourMode::Gbc => "GBC",
            ColourMode::Sgb => "SGB",
            ColourMode::SgbBorder => "SGB, border",
        };
        let emulated_title = if games.agent_running { "EMULATED   agent".to_string() } else { "EMULATED".to_string() };
        for (side, rect, title) in [(Side::Emulated, layout.emulated, emulated_title), (Side::Native, layout.native, format!("NATIVE   {mode}"))] {
            let title_y = rect.y - TITLE_HEIGHT as i32;
            let clip = Rect::new(rect.x, title_y, rect.width, TITLE_HEIGHT);
            text.draw(pixels, &title, rect.x, title_y, LABEL, clip);
            if let Some(tour) = &games.tour {
                let (standing, progress) = (tour.standing(side), tour.progress(side));
                let said = format!("   step {}/{}   {}", progress.0, progress.1, standing.label());
                let ink = if standing == Standing::Waiting { WAITING } else { DIM };
                let x = rect.x + text.width(&title) as i32;
                text.draw(pixels, &said, x, title_y, ink, clip);
            }
        }
        let lcd: Vec<u8> = games.gb.core().mmu().ppu().lcd().iter().flat_map(|pixel| {
            let [r, g, b] = pixel.to_rgb().0;
            [r, g, b, 0xff]
        }).collect();
        pixels.blit_scaled(&lcd, LCD_WIDTH, layout.emulated.x, layout.emulated.y, layout.scale);
        pixels.blit_scaled(&colours.rgba(games.native.game().screen()), colours.size().0, layout.native.x, layout.native.y, layout.scale);

        pixels.fill(layout.log, PANEL);
        let area = layout.log_text();
        for (row, (source, line, continued)) in self.log.view(layout.log_rows()).into_iter().enumerate() {
            let x = area.x + if continued { text.width(source.prefix()) as i32 } else { 0 };
            let y = area.y + row as i32 * layout.line_height as i32;
            text.draw(pixels, line, x, y, colour(source), area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdl::games::tests::{fresh, tap_a};

    #[test]
    fn the_screens_sit_side_by_side_at_a_whole_scale_over_the_log() {
        let layout = Layout::new(3, ColourMode::Dmg.size(), 19);
        assert_eq!((layout.emulated.width, layout.emulated.height), (480, 432));
        assert_eq!(layout.native.y, layout.emulated.y);
        assert!(layout.emulated.right() < layout.native.x);
        assert!(layout.toggle.iter().all(|rect| rect.bottom() < layout.emulated.y));
        assert!(layout.log.y > layout.emulated.bottom());
        assert_eq!(layout.native.right() + MARGIN as i32, layout.width as i32);

        let border = Layout::new(3, ColourMode::SgbBorder.size(), 19);
        assert_eq!((border.native.width, border.native.height), (768, 672));
        assert!(border.log.y > border.native.bottom());
    }

    #[test]
    fn the_largest_scale_that_fits_the_display_is_chosen() {
        let three = Layout::new(3, ColourMode::Dmg.size(), 19);
        assert_eq!(Layout::fit((three.width, three.height), ColourMode::Dmg.size(), 19).scale, 3);
        assert_eq!(Layout::fit((three.width - 1, three.height), ColourMode::Dmg.size(), 19).scale, 2);
        assert_eq!(Layout::fit((10, 10), ColourMode::Dmg.size(), 19).scale, 1);
    }

    #[test]
    fn a_click_on_a_button_names_it() {
        let layout = Layout::new(2, ColourMode::Dmg.size(), 19);
        for (rect, routing) in layout.toggle.iter().zip(Routing::ALL) {
            assert_eq!(layout.control_at(rect.x + 1, rect.y + 1), Some(Control::Routing(routing)));
        }
        assert_eq!(layout.control_at(layout.tour.x + 1, layout.tour.bottom() - 1), Some(Control::Tour));
        assert_eq!(layout.control_at(layout.speed.right() - 1, layout.speed.y + 1), Some(Control::Speed));
        assert_eq!(layout.control_at(layout.status.x + 1, layout.status.y + 1), None);
        assert_eq!(layout.control_at(layout.emulated.x + 1, layout.emulated.y + 1), None);
    }

    #[test]
    fn the_buttons_sit_in_a_row_left_of_the_status_line() {
        for scale in 1..=4 {
            let layout = Layout::new(scale, ColourMode::Dmg.size(), 19);
            let row = [layout.toggle[0], layout.toggle[1], layout.toggle[2], layout.tour, layout.speed];
            for pair in row.windows(2) {
                assert!(pair[0].right() < pair[1].x, "{scale}: {pair:?}");
                assert_eq!(pair[0].y, pair[1].y);
            }
            assert!(layout.speed.right() < layout.status.x);
            assert!(layout.status.x + layout.status.width as i32 <= layout.width as i32 - MARGIN as i32 || layout.status.width == 0);
        }
    }

    #[test]
    fn a_drop_lands_on_the_half_the_cursor_is_over() {
        for colours in [ColourMode::Dmg, ColourMode::SgbBorder] {
            let layout = Layout::new(2, colours.size(), 19);
            assert_eq!(layout.side_at(0), Side::Emulated);
            assert_eq!(layout.side_at(layout.emulated.right()), Side::Emulated);
            assert_eq!(layout.side_at(layout.native.x), Side::Native);
            assert_eq!(layout.side_at(layout.width as i32 - 1), Side::Native);
            let middle = (layout.emulated.right() + layout.native.x) / 2;
            assert_eq!((layout.side_at(middle - 1), layout.side_at(middle)), (Side::Emulated, Side::Native));
        }
    }

    /// Both games from power-on to Oak's welcome on the same presses, as the window composes them.
    fn past_the_title() -> (Games, View) {
        let mut games = fresh();
        let mut view = View::new(|native, line_height| Layout::new(3, native, line_height)).unwrap();
        tap_a(&mut games, 2400, 40, &mut |source, line| view.say(source, line));
        view.status = "59.7 fps   1x   sound on   agent off".to_string();
        view.compose(&games);
        (games, view)
    }

    #[test]
    fn each_game_says_its_welcome_into_the_log() {
        let (_, view) = past_the_title();
        let said = |source: Source| view.log.view(usize::MAX).iter().any(|&(s, line, _)| s == source && line.contains("Hello there!"));
        assert!(said(Source::Emulated), "{:?}", view.log.view(usize::MAX));
        assert!(said(Source::Native), "{:?}", view.log.view(usize::MAX));
    }

    /// `GB_WINDOW_PNG` names where to write the window as composed with both games past the title,
    /// or, when `GB_WINDOW_EMULATED` or `GB_WINDOW_NATIVE` names a state, loaded as a drop loads it
    /// and played on for two seconds, or, when `GB_WINDOW_TOUR` names a number of frames, that far
    /// into the grand tour on both.
    #[test]
    #[ignore = "a tool: dumps the composed window, to judge the layout by eye"]
    fn dump_window() {
        let path = std::env::var("GB_WINDOW_PNG").expect("GB_WINDOW_PNG names the PNG to write");
        let (mut games, mut view) = past_the_title();
        let states = [("GB_WINDOW_EMULATED", Side::Emulated), ("GB_WINDOW_NATIVE", Side::Native)]
            .map(|(var, side)| std::env::var(var).ok().map(|state| (side, state)));
        if states.iter().any(Option::is_some) {
            for (side, state) in states.into_iter().flatten() {
                games.load_file(side, std::path::Path::new(&state)).unwrap();
                view.say(Source::Window, format!("loaded {state} into the {} game", side.label()));
            }
            for _ in 0..120 {
                games.frame(Routing::Both, pokered::input::Joypad::empty(), &mut |source, line| view.say(source, line));
            }
            view.compose(&games);
        }
        if let Some(frames) = std::env::var("GB_WINDOW_TOUR").ok().map(|frames| frames.parse::<u32>().expect("a number of frames")) {
            games.start_tour().unwrap();
            let mut samples = vec![0.0f32; 48_000 / 8 * 2];
            for _ in 0..frames {
                games.frame(Routing::Both, pokered::input::Joypad::empty(), &mut |source, line| view.say(source, line));
                while games.read_samples(&mut samples) > 0 {}
            }
            view.speed = Speed::Unthrottled;
            view.status = "59.7 fps   41x   sound on   tour 0:03:12".to_string();
            view.compose(&games);
        }
        view.pixels.save_png(std::path::Path::new(&path)).unwrap();
    }
}
