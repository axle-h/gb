//! The walkthrough, served one chapter at a time.
//!
//! Nine chapters, one per badge the player does not have yet: what to do next, in order, and what
//! is standing in the way. They are ordinary markdown in `src/llm/guide/`, `include_str!`'d, and
//! handed to the model **verbatim** — there is no template and nothing is rendered, so what is on
//! disk is exactly what is sent, which is the only cheap way to review it.
//!
//! ⚠️ **The chapter is chosen by the first badge the player is *missing*, not by how many they
//! have.** The two agree in an ordinary run and come apart in exactly the case where the answer
//! matters: a player holding Boulder and Thunder but not Cascade has two badges, and what they need
//! to be told is how to get the Cascade Badge. A popcount would send them to Vermilion.
//!
//! ⚠️ **Every backticked name in the prose is a [`Map`] variant** and nothing else is backticked.
//! That is what makes the guide's place names the same strings as the turn's `Location:` line, the
//! action menu's ids and `read_route`'s argument, so a model can copy one straight across.
//! `every_place_the_guide_names_is_a_real_map` is the guard, and it is not decoration: the prose is
//! free to be reworded, the keys are not.
//!
//! ⚠️ **This is our own text, not the source's.** It is written from facts checked against the
//! disassembly where the disassembly settles them — the Viridian Gym's door test in chapter 7 is
//! read off `ViridianCityCheckGymOpenScript`, not off any guide. See `src/llm/guide/README.md` for
//! provenance.

use crate::pokemon::badge::Badge;

/// Chapter `i` is what to do when [`Badge::ORDER`]`[i]` is the next badge to win. The ninth is the
/// Elite Four, which is what is left once all eight are held.
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

/// How far through the game the badges say the player is: the index of the first badge in
/// [`Badge::ORDER`] they do not hold, or 8 when they hold all of them.
pub fn chapter_index(badges: Badge) -> usize {
    Badge::ORDER.iter().position(|badge| !badges.contains(*badge)).unwrap_or(CHAPTERS.len() - 1)
}

/// The stretch of the game the player is in the middle of, whole.
pub fn chapter(badges: Badge) -> &'static str {
    CHAPTERS[chapter_index(badges)]
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

        // ⚠️ The case a popcount gets wrong: two badges, but the second one is not the second badge.
        let out_of_order = Badge::BoulderBadge | Badge::ThunderBadge;
        assert_eq!(out_of_order.bits().count_ones(), 2);
        assert_eq!(chapter_index(out_of_order), 1, "the missing Cascade Badge is what to go and get");
        assert!(chapter(out_of_order).contains("Misty"));
    }

    /// ⚠️ A place name in the guide is a **key**: the model copies it into `read_route` or matches it
    /// against the turn's own `Location:` line. Backticks mean "this is a `Map`" and mean nothing
    /// else, so a floor suffix (`` `B4F` ``) or a collective name (`` `SilphCo` ``) is a bug even
    /// though both read perfectly well.
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

    /// The guide is carried in the context until a compaction takes it, so a chapter that grew into
    /// the thousands would be paid for on every request of the chapter it describes.
    #[test]
    fn no_chapter_outgrows_what_it_costs_to_carry() {
        for (index, chapter) in CHAPTERS.iter().enumerate() {
            assert!(!chapter.is_empty(), "chapter {index} is empty");
            assert!(chapter.len() < 3_500, "chapter {index} is {} bytes", chapter.len());
        }
    }
}
