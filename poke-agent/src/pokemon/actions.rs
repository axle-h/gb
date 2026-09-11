use std::fmt::Display;
use gb::geometry::Point8;
use gb::joypad::JoypadButton;
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
    /// The id of this action: stable across a re-sort, unique within a map, and readable enough
    /// that a model quoting it back is obviously quoting the right thing.
    pub fn id(&self) -> String {
        // A sprite is keyed on the object, with no coordinate at all, because it has no
        // coordinate that holds still.
        if matches!(self.tile, MetaTile::Sprite(_)) {
            return format!("{}:{}", self.map, self.tile.id_kind());
        }
        // A boulder goal is keyed on its *target*, not on where the walk starts.
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
    /// The imperative form — this is a menu entry, one row of what the policy may choose next, so
    /// it leads with the verb where [`MetaTile`]'s own `Display` is a noun phrase.
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

    /// The fact [`OverworldAction::id`] keys a sprite on `map + name` rests on.
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

    /// One object, one id, wherever you happen to be standing.
    #[test]
    fn a_sprite_row_is_one_id_wherever_you_stand_to_face_it() {
        let sprite = MetaTile::Sprite("Old Man");
        let ids: Vec<String> = [(17, 5), (18, 5), (18, 7), (19, 7), (20, 6)].into_iter()
            .map(|(x, y)| row(sprite, Point8 { x, y }).id())
            .collect();
        assert_eq!(ids, vec!["ViridianCity:OldMan".to_string(); 5],
                   "the four squares an NPC can be faced from are one decision, not four");
    }

    /// And every other row keeps its coordinate, because for those the destination *is* the
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
