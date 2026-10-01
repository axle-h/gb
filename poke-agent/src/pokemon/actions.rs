use std::fmt::Display;
use poke_core::geometry::Point8;
use gb::joypad::JoypadButton;
use crate::pokemon::map::Map;
use crate::pokemon::tile::MetaTile;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverworldAction {
    pub map: Map,
    /// The square the id names where the tile carries none: a door's or a map edge opening's first
    /// square in reading order, which holds still while `destination`, the nearest, moves.
    pub key: Point8,
    pub destination: Point8,
    pub tile: MetaTile,
    pub route: Vec<JoypadButton>,
}

/// The kinds a map offers one row of, keyed `map:kind`: the square such a row walks to is only the
/// nearest of many, and moves with every step.
pub const ONE_PER_MAP: [&str; 5] = ["Grass", "Pace", "PaceOnWater", "Fish", "Slots"];

impl OverworldAction {
    /// Stable across a re-sort, unique within a map, and plainly the right row when quoted back.
    pub fn id(&self) -> String {
        // A sprite's id has no coordinate, because it has none that holds still.
        let kind = self.tile.id_kind();
        if matches!(self.tile, MetaTile::Sprite(_)) || ONE_PER_MAP.contains(&kind.as_ref()) {
            return format!("{}:{kind}", self.map);
        }
        // Every other row is keyed on the thing, never on the square it is approached from, which
        // `actions()` re-picks whenever anyone moves: the id has to outlive the turn.
        let at = match self.tile {
            MetaTile::BoulderGoal { at, .. } => at,
            MetaTile::BoulderPush { boulder, .. } => boulder,
            MetaTile::Cut { at } => at,
            MetaTile::Switch { ordinal, .. } => crate::pokemon::tile_map::hidden_objects_for(self.map)
                .get(ordinal as usize - 1).map_or(self.key, |site| site.at),
            _ => self.key,
        };
        format!("{}:{},{}:{kind}", self.map, at.x, at.y)
    }

    /// Whether `id` names a person or an object: the only `map:name` id that is not [`ONE_PER_MAP`].
    pub fn names_a_sprite(id: &str) -> bool {
        id.matches(':').count() == 1 && id.rsplit(':').next().is_some_and(|kind| !ONE_PER_MAP.contains(&kind))
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
    /// The imperative, as a menu row, where [`MetaTile`]'s own `Display` is a noun phrase.
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
            MetaTile::Slots         => write!(f, "Play the slots"),
            other                   => write!(f, "{other}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    fn row(tile: MetaTile, destination: Point8) -> OverworldAction {
        OverworldAction { map: Map::ViridianCity, key: destination, destination, tile, route: vec![] }
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

    /// A switch is pressed from whichever side is nearest and free, and keyed on the object.
    #[test]
    fn a_switch_row_is_one_id_whichever_side_it_is_pressed_from() {
        let machine = MetaTile::Switch { object: crate::pokemon::tile::HiddenObject::VendingMachine, ordinal: 1 };
        let ids: Vec<String> = [(10, 2), (9, 1)].into_iter()
            .map(|(x, y)| OverworldAction { map: Map::CeladonMartRoof, ..row(machine, Point8 { x, y }) }.id())
            .collect();
        assert_eq!(ids, vec!["CeladonMartRoof:10,1:VendingMachine1".to_string(); 2]);
    }

    /// Every other row keeps its square: two warps on one map are two decisions.
    #[test]
    fn a_row_that_is_not_a_sprite_still_carries_its_square() {
        let warp = MetaTile::Warp { to_map: Map::ViridianPokecenter,
                                    to_position: Point8 { x: 3, y: 7 } };
        assert_eq!(row(warp, Point8 { x: 23, y: 26 }).id(), "ViridianCity:23,26:Warp");
        assert_ne!(row(warp, Point8 { x: 23, y: 26 }).id(),
                   row(warp, Point8 { x: 33, y: 18 }).id());
    }
}
