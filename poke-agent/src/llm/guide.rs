//! The walkthrough, served one chapter at a time.

use crate::pokemon::badge::Badge;

/// Chapter `i` is what to do when [`Badge::ORDER`]`[i]` is the next badge to win.
const CHAPTERS: [&str; 9] = [
    include_str!("guide/00-boulder-badge.md"),
    include_str!("guide/01-cascade-badge.md"),
    include_str!("guide/02-thunder-badge.md"),
    include_str!("guide/03-rainbow-badge.md"),
    include_str!("guide/04-soul-badge.md"),
    include_str!("guide/05-marsh-badge.md"),
    include_str!("guide/06-volcano-badge.md"),
    include_str!("guide/07-earth-badge.md"),
    include_str!("guide/08-elite-four.md"),
];

/// The index of the first badge in [`Badge::ORDER`] not held, or 8 when all of them are.
pub fn chapter_index(badges: Badge) -> usize {
    Badge::ORDER.iter().position(|badge| !badges.contains(*badge)).unwrap_or(CHAPTERS.len() - 1)
}

pub fn chapter(badges: Badge) -> &'static str {
    CHAPTERS[chapter_index(badges)]
}

/// What the model's last `read_guide` is worth on the turn being rendered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GuideStatus {
    /// The chapter last read is the one [`chapter`] would hand over now.
    #[default]
    Current,
    /// A badge has been won since the last read, so [`chapter`] now answers with a different one.
    Stale { index: usize },
}

/// Whether the [`chapter_index`] a `read_guide` was last answered from still matches `badges`.
pub fn status(badges: Badge, last_read: Option<usize>) -> GuideStatus {
    let now = chapter_index(badges);
    match last_read {
        Some(read) if read != now => GuideStatus::Stale { index: now },
        _ => GuideStatus::Current,
    }
}

/// What the chapter at `index` is about, as a noun phrase for the nudge to name.
pub fn chapter_goal(index: usize) -> String {
    match Badge::ORDER.get(index) {
        Some(badge) => format!("the {badge}"),
        None => "the Elite Four".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::map::Map;
    use strum::IntoEnumIterator;

    #[test]
    fn the_chapter_is_the_next_badge_to_win_rather_than_the_number_held() {
        assert_eq!(chapter_index(Badge::empty()), 0);
        assert_eq!(chapter_index(Badge::BoulderBadge), 1);
        assert_eq!(chapter_index(Badge::all()), 8);

        // The case a popcount gets wrong: two badges, but the second one is not the second badge.
        let out_of_order = Badge::BoulderBadge | Badge::ThunderBadge;
        assert_eq!(out_of_order.bits().count_ones(), 2);
        assert_eq!(chapter_index(out_of_order), 1, "the missing Cascade Badge is what to go and get");
        assert!(chapter(out_of_order).contains("Misty"));
    }

    #[test]
    fn a_read_goes_stale_when_a_badge_moves_the_chapter_and_not_before() {
        let one = Badge::BoulderBadge;
        assert_eq!(status(one, Some(1)), GuideStatus::Current, "read after Brock, still before Misty");
        assert_eq!(status(one, Some(0)), GuideStatus::Stale { index: 1 }, "read before Brock, Brock is beaten");

        // Never read is `Current`, not stale.
        assert_eq!(status(one, None), GuideStatus::Current);
        assert_eq!(status(Badge::empty(), None), GuideStatus::Current);

        // Two badges out of order is still the Cascade chapter, as `chapter_index` says.
        let out_of_order = Badge::BoulderBadge | Badge::ThunderBadge;
        assert_eq!(status(out_of_order, Some(1)), GuideStatus::Current);

        for index in 0..=8 {
            assert!(!chapter_goal(index).is_empty(), "chapter {index}");
        }
        assert_eq!(chapter_goal(0), format!("the {}", Badge::BoulderBadge));
        assert_eq!(chapter_goal(8), "the Elite Four");
    }

    /// A place name in the guide is a key the model copies into `read_route`.
    #[test]
    fn every_place_the_guide_names_is_a_real_map() {
        let mut checked = 0;
        for (index, chapter) in CHAPTERS.iter().enumerate() {
            for name in chapter.split('`').skip(1).step_by(2) {
                assert!(
                    Map::iter().any(|map| map.to_string() == name),
                    "chapter {index} names `{name}`, which is not a Map variant",
                );
                checked += 1;
            }
        }
        assert!(checked > 100, "only {checked} names checked; the guide cannot have shrunk this far");
    }

    /// A chapter is paid for on every request until a compaction takes it.
    #[test]
    fn no_chapter_outgrows_what_it_costs_to_carry() {
        for (index, chapter) in CHAPTERS.iter().enumerate() {
            assert!(!chapter.is_empty(), "chapter {index} is empty");
            assert!(chapter.len() < 3_500, "chapter {index} is {} bytes", chapter.len());
        }
    }
}
