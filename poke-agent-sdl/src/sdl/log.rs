//! The panel under the two screens: both games' lines, wrapped to the panel, newest at the bottom.

use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Emulated,
    Native,
    /// The window itself: the agent switched on or off, a failure of its own.
    Window,
}

impl Source {
    pub fn prefix(self) -> &'static str {
        match self {
            Source::Emulated => "EMU  ",
            Source::Native => "NAT  ",
            Source::Window => "WIN  ",
        }
    }

    pub fn of(side: crate::sdl::games::Side) -> Self {
        match side {
            crate::sdl::games::Side::Emulated => Source::Emulated,
            crate::sdl::games::Side::Native => Source::Native,
        }
    }
}

/// One wrapped row: its source, its text, and whether it carries on the row above.
pub type Row<'a> = (Source, &'a str, bool);

pub struct Log {
    /// Each line as pushed, and its rows.
    entries: VecDeque<(Source, String, Vec<String>)>,
    width: u32,
    rows: usize,
    /// Rows the view is lifted off the newest, which new lines leave where it is.
    scroll: usize,
}

impl Log {
    const CAPACITY: usize = 2000;

    pub fn new(width: u32) -> Self {
        Self { entries: VecDeque::new(), width, rows: 0, scroll: 0 }
    }

    /// `text` from `source`, its prefix on the first row and the rest indented to clear it.
    pub fn push(&mut self, source: Source, text: &str, measure: &mut impl FnMut(&str) -> u32) {
        let rows = wrap(&format!("{}{text}", source.prefix()), self.width, measure(source.prefix()), measure);
        if self.scroll > 0 {
            self.scroll += rows.len();
        }
        self.rows += rows.len();
        self.entries.push_back((source, text.to_string(), rows));
        while self.entries.len() > Self::CAPACITY {
            let (_, _, dropped) = self.entries.pop_front().unwrap();
            self.rows -= dropped.len();
        }
    }

    /// Wrap everything again at a new `width`, back at the newest.
    pub fn rewrap(&mut self, width: u32, measure: &mut impl FnMut(&str) -> u32) {
        let entries = std::mem::take(&mut self.entries);
        *self = Self::new(width);
        for (source, text, _) in entries {
            self.push(source, &text, measure);
        }
    }

    /// Move the view `by` rows, older for positive, never above the oldest row or below the newest.
    pub fn scroll(&mut self, by: i32, visible: usize) {
        let top = self.rows.saturating_sub(visible);
        self.scroll = (self.scroll as i64 + by as i64).clamp(0, top as i64) as usize;
    }

    /// The `visible` rows the view shows, oldest first.
    pub fn view(&self, visible: usize) -> Vec<Row<'_>> {
        let top = self.rows.saturating_sub(visible);
        let first = top.saturating_sub(self.scroll.min(top));
        self.entries
            .iter()
            .flat_map(|(source, _, rows)| rows.iter().enumerate().map(move |(i, row)| (*source, row.as_str(), i > 0)))
            .skip(first)
            .take(visible)
            .collect()
    }
}

/// `text` broken into rows no wider than `width` at spaces, a word too wide for a row broken where
/// it must; every row after the first is `indent` narrower, since it is drawn that far in.
pub fn wrap(text: &str, width: u32, indent: u32, measure: &mut impl FnMut(&str) -> u32) -> Vec<String> {
    let mut rows = Vec::new();
    let mut row = String::new();
    for word in text.split(' ') {
        let room = if rows.is_empty() { width } else { width.saturating_sub(indent) };
        let candidate = if row.is_empty() { word.to_string() } else { format!("{row} {word}") };
        if measure(&candidate) <= room {
            row = candidate;
            continue;
        }
        if !row.is_empty() {
            rows.push(std::mem::take(&mut row));
        }
        for c in word.chars() {
            let room = if rows.is_empty() { width } else { width.saturating_sub(indent) };
            row.push(c);
            if measure(&row) > room && row.chars().count() > 1 {
                row.pop();
                rows.push(std::mem::replace(&mut row, c.to_string()));
            }
        }
    }
    if !row.is_empty() || rows.is_empty() {
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A character a unit wide, so widths read as lengths.
    fn chars(text: &str) -> u32 {
        text.chars().count() as u32
    }

    #[test]
    fn a_line_wraps_at_spaces_and_breaks_a_word_too_long_for_a_row() {
        assert_eq!(wrap("one two three", 7, 0, &mut chars), ["one two", "three"]);
        assert_eq!(wrap("abcdefghij", 4, 0, &mut chars), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("ab cdefgh", 4, 0, &mut chars), ["ab", "cdef", "gh"]);
        assert_eq!(wrap("", 4, 0, &mut chars), [""]);
    }

    #[test]
    fn rows_after_the_first_lose_the_indent() {
        assert_eq!(wrap("EMU  aa bb cc dd", 10, 5, &mut chars), ["EMU  aa bb", "cc dd"]);
        assert_eq!(wrap("EMU  aaaa bbbbbb", 10, 5, &mut chars), ["EMU  aaaa", "bbbbb", "b"]);
    }

    fn log(width: u32, lines: usize) -> Log {
        let mut log = Log::new(width);
        for i in 0..lines {
            log.push(if i % 2 == 0 { Source::Emulated } else { Source::Native }, &format!("line {i}"), &mut chars);
        }
        log
    }

    fn texts(log: &Log, visible: usize) -> Vec<String> {
        log.view(visible).into_iter().map(|(_, text, _)| text.to_string()).collect()
    }

    #[test]
    fn the_newest_rows_are_at_the_bottom_with_their_game() {
        let log = log(100, 5);
        assert_eq!(texts(&log, 2), ["NAT  line 3", "EMU  line 4"]);
        assert_eq!(log.view(2)[1].0, Source::Emulated);
        assert_eq!(texts(&log, 10).len(), 5);
    }

    #[test]
    fn the_wheel_scrolls_back_and_stops_at_either_end() {
        let mut log = log(100, 10);
        log.scroll(3, 4);
        assert_eq!(texts(&log, 4), ["NAT  line 3", "EMU  line 4", "NAT  line 5", "EMU  line 6"]);
        log.scroll(100, 4);
        assert_eq!(texts(&log, 4)[0], "EMU  line 0");
        log.scroll(-100, 4);
        assert_eq!(texts(&log, 4)[3], "NAT  line 9");
    }

    #[test]
    fn a_view_scrolled_back_holds_still_as_lines_arrive() {
        let mut log = log(20, 10);
        log.scroll(2, 3);
        let before = texts(&log, 3);
        log.push(Source::Window, "a line long enough to wrap onto more rows than one", &mut chars);
        assert_eq!(texts(&log, 3), before);
        log.scroll(-100, 3);
        assert_eq!(texts(&log, 3), ["enough to wrap", "onto more rows", "than one"]);
        assert_eq!(log.view(3)[2], (Source::Window, "than one", true));
    }

    #[test]
    fn rewrapping_keeps_every_line() {
        let mut log = Log::new(10);
        log.push(Source::Native, "aaaa bbbb cccc", &mut chars);
        assert_eq!(texts(&log, 9), ["NAT  aaaa", "bbbb", "cccc"]);
        log.rewrap(100, &mut chars);
        assert_eq!(texts(&log, 9), ["NAT  aaaa bbbb cccc"]);
    }
}
