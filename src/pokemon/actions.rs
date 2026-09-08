use std::fmt::Display;
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::pokemon::map::Map;
use crate::pokemon::tile::MetaTile;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverworldAction {
    pub map: Map,
    pub origin: Point8,
    pub destination: Point8,
    pub tile: MetaTile,
    pub route: Vec<JoypadButton>,
}

impl OverworldAction {
    /// The id of this action: stable across a re-sort, unique within a map, and readable enough that
    /// a model quoting it back is obviously quoting the right thing.
    ///
    /// ⚠️ **The one definition, and it lives here rather than in `llm::tools` because it is not only
    /// the model's.** `tools::overworld_id` renders the action menu with it, `AgentEvent`'s
    /// [`StartedOverworldAction`](crate::pokemon::agent::AgentEvent::StartedOverworldAction) carries
    /// it so a coverage log can key on the same string, and `resolve_overworld` matches on it by
    /// string equality. Two spellings of an id is two spellings of a key.
    ///
    /// ⚠️ **`MetaTile::id_kind`, never its `Display`.** The `Display` is prose written for the status
    /// log ("the warp to OaksLab") and is free to be reworded; an id is a key.
    ///
    /// ⚠️ **The map prefix looks redundant beside the turn's own header and is not.**
    /// `resolve_overworld` re-mints ids against whatever map the player is on *now*, and the answer
    /// to a turn can land after a warp — so without the prefix, `5,6:Warp` chosen in Oak's lab could
    /// match a warp that happens to sit at (5, 6) in Pallet Town and be carried out silently.
    pub fn id(&self) -> String {
        // ⭐ **A sprite is keyed on the object, with no coordinate at all, because it has no
        // coordinate that holds still.** `destination` for a sprite row is the *approach tile* —
        // the square the player stands on to face it — which `MetaTileMap::actions` re-picks as the
        // nearest of the four (or six, through a counter) every time the player moves. So one
        // object minted an id per square it could be talked to from, and a walking NPC minted one
        // per square × per step. Measured on the C3 sweep of 2026-09-08: **270 sprite ids covering
        // 136 objects**, 134 of them redundant and 26% of the whole frontier, with
        // `ViridianCity:Youngster1` alone spending eleven — ten of which the walk "completed" by
        // talking to the same Youngster ten times, believing each was a new action. It is the same
        // fault as the boulder goal below, one layer over, and with the same answer: key on the
        // thing that does not move.
        //
        // ⚠️ **The key is `map + name`, and that it is unique is a fact about the ROM rather than a
        // hope** — 919 sprite constants across 208 maps, no name repeated within a map, which is
        // what `tests::a_sprite_name_is_unique_within_its_map` below pins. Two objects sharing an id
        // would be worse than the churn this replaces.
        //
        // ⚠️ **So a sprite id has two fields where every other id has three**, and anything reading
        // one must keep taking the map off the front (`split`) and the kind off the back (`rsplit`)
        // rather than counting fields. The coordinate that was there was not a key and was not even
        // true: it named a square beside the person, never the person.
        if matches!(self.tile, MetaTile::Sprite(_)) {
            return format!("{}:{}", self.map, self.tile.id_kind());
        }
        // ⚠️ **A boulder goal is keyed on its *target*, not on where the walk starts.** For every
        // other row `destination` is the thing itself and holds still; for a goal it is the square
        // the first shove is made from, which the solver re-picks after every push. See
        // `MetaTile::id_kind`'s note for what that cost.
        let at = match self.tile {
            MetaTile::BoulderGoal { at, .. } => at,
            _ => self.destination,
        };
        format!("{}:{},{}:{}", self.map, at.x, at.y, self.tile.id_kind())
    }
}

impl PartialOrd for OverworldAction {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OverworldAction {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.tile.cmp(&other.tile)
    }
}

impl Display for OverworldAction {
    /// The imperative form — this is a menu entry, one row of what the policy may choose next, so it
    /// leads with the verb where [`MetaTile`]'s own `Display` is a noun phrase.
    ///
    /// ⚠️ **Every arm that has a target names it.** The three that used to fall through to
    /// `{other}` were the vague ones: a surf crossing read as `ConnectionWater` and did not say
    /// which map it led to, and `Pc`/`CutTree` named the tile rather than the thing to do with it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.tile {
            MetaTile::Warp { to_map, to_position }       => write!(f, "Warp → {to_map} {to_position}"),
            MetaTile::Connection { to_map, to_position } => write!(f, "Go to {to_map} {to_position}"),
            MetaTile::ConnectionWater(to_map)            => write!(f, "Surf to {to_map}"),
            MetaTile::Sprite(n)     => write!(f, "Talk to {n}"),
            MetaTile::Grass         => write!(f, "Walk in grass"),
            MetaTile::Pc            => write!(f, "Use the PC"),
            MetaTile::CutTree       => write!(f, "Cut the tree"),
            MetaTile::Cut { at }    => write!(f, "Cut the tree at {at}"),
            MetaTile::Fish { rod }  => write!(f, "Fish with the {}", rod.name()),
            other                   => write!(f, "{other}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    fn row(tile: MetaTile, destination: Point8) -> OverworldAction {
        OverworldAction { map: Map::ViridianCity, origin: Point8 { x: 0, y: 0 },
                          destination, tile, route: vec![] }
    }

    /// ⭐ **The fact [`OverworldAction::id`] keys a sprite on `map + name` rests on.** A sprite id
    /// carries no coordinate, so two objects on one map sharing a name would share an id — and one
    /// id for two things is a worse failure than the churn dropping the coordinate fixed, because
    /// the walk would score one of them and never see the other. This is a property of the ROM's own
    /// object tables rather than a convention anyone maintains, so it is asserted rather than
    /// assumed: 919 sprites across 208 maps at the time of writing, no repeat within any map.
    #[test]
    fn a_sprite_name_is_unique_within_its_map() {
        let mut checked = 0;
        for map in Map::iter() {
            let names: Vec<&str> = map.sprites().iter().map(|s| s.name).collect();
            for (i, name) in names.iter().enumerate() {
                assert!(!names[..i].contains(name),
                        "{map} has two sprites called {name:?}; a sprite id is `{map}:{name}` and \
                         the two would be indistinguishable");
            }
            checked += names.len();
        }
        assert!(checked > 900, "only {checked} sprites — the sprite table has shrunk unexpectedly");
    }

    /// ⭐ **One object, one id, wherever you happen to be standing.** `MetaTileMap::actions` sets a
    /// sprite row's `destination` to the *nearest* square it can be faced from, so this used to
    /// mint a different id per approach: the C3 sweep of 2026-09-08 carried eleven ids for the one
    /// Youngster in Viridian City and completed ten of them, each time talking to the same person.
    #[test]
    fn a_sprite_row_is_one_id_wherever_you_stand_to_face_it() {
        let sprite = MetaTile::Sprite("Old Man");
        let ids: Vec<String> = [(17, 5), (18, 5), (18, 7), (19, 7), (20, 6)].into_iter()
            .map(|(x, y)| row(sprite, Point8 { x, y }).id())
            .collect();
        assert_eq!(ids, vec!["ViridianCity:OldMan".to_string(); 5],
                   "the four squares an NPC can be faced from are one decision, not four");
    }

    /// ⚠️ **And every other row keeps its coordinate**, because for those the destination *is* the
    /// thing: two warps on one map are two decisions and the square is what tells them apart.
    #[test]
    fn a_row_that_is_not_a_sprite_still_carries_its_square() {
        let warp = MetaTile::Warp { to_map: Map::ViridianPokecenter,
                                    to_position: Point8 { x: 3, y: 7 } };
        assert_eq!(row(warp, Point8 { x: 23, y: 26 }).id(), "ViridianCity:23,26:Warp");
        assert_ne!(row(warp, Point8 { x: 23, y: 26 }).id(),
                   row(warp, Point8 { x: 33, y: 18 }).id());
    }
}
