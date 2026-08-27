use std::fmt::{Display, Formatter};
use crate::mmu::MMU;
use crate::pokemon::item::ItemId;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::ram::{RAM, ROM};


#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct BagItem {
    pub id:       ItemId,
    pub quantity: u8,
}

impl BagItem {
    pub const fn new(id: ItemId, quantity: u8) -> Self {
        Self { id, quantity }
    }
}

impl Display for BagItem {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} x{}", self.id, self.quantity)
    }
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct Bag(Vec<BagItem>);

impl Bag {
    pub const MAX_ITEMS: usize = 20;

    pub fn new(items: Vec<BagItem>) -> Self {
        if items.len() > Self::MAX_ITEMS {
            let mut items = items.clone();
            items.truncate(Self::MAX_ITEMS);
            Self(items)
        } else {
            Self(items)
        }
    }

    pub fn from_slice(slice: &[BagItem]) -> Self {
        Self(slice.to_vec())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty() || self.iter().all(|i| i.quantity == 0)
    }

    pub fn iter(&self) -> impl Iterator<Item = &BagItem> {
        self.0.iter()
    }

    pub fn push(&mut self, item: BagItem) -> Result<(), String> {
        if self.len() >= Self::MAX_ITEMS {
            Err("bag is full".to_string())
        } else {
            Ok(self.0.push(item))
        }
    }

    pub fn contains(&self, item: &ItemId) -> bool {
        self.0.iter().find(|i| &i.id == item && i.quantity > 0).is_some()
    }

    pub fn best_pokeball(&self) -> Option<&BagItem> {
        self.0.iter()
            .filter(|item| item.quantity > 0
                && matches!(item.id, ItemId::MasterBall | ItemId::UltraBall | ItemId::GreatBall | ItemId::PokeBall | ItemId::SafariBall))
            .min_by_key(|item| item.id) // the pokeballs item id's are ordered by effectiveness in the data already
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bag_with(items: &[(ItemId, u8)]) -> Bag {
        Bag::new(items.iter().map(|&(id, quantity)| BagItem::new(id, quantity)).collect())
    }

    #[test]
    fn test_best_pokeball_empty_bag() {
        let bag = Bag::default();
        assert_eq!(bag.best_pokeball(), None);
    }

    #[test]
    fn test_best_pokeball_only_pokeball() {
        let bag = bag_with(&[(ItemId::PokeBall, 5)]);
        assert_eq!(bag.best_pokeball().map(|i| i.id), Some(ItemId::PokeBall));
    }

    #[test]
    fn test_best_pokeball_prefers_masterball() {
        let bag = bag_with(&[
            (ItemId::PokeBall, 10),
            (ItemId::GreatBall, 5),
            (ItemId::MasterBall, 1),
        ]);
        assert_eq!(bag.best_pokeball().map(|i| i.id), Some(ItemId::MasterBall));
    }

    #[test]
    fn test_best_pokeball_ultraball_over_greatball() {
        let bag = bag_with(&[
            (ItemId::GreatBall, 3),
            (ItemId::UltraBall, 2),
            (ItemId::SafariBall, 10),
        ]);
        assert_eq!(bag.best_pokeball().map(|i| i.id), Some(ItemId::UltraBall));
    }

    #[test]
    fn test_best_pokeball_safari_ball_only() {
        let bag = bag_with(&[(ItemId::SafariBall, 30)]);
        assert_eq!(bag.best_pokeball().map(|i| i.id), Some(ItemId::SafariBall));
    }

    #[test]
    fn test_best_pokeball_ignores_zero_quantity() {
        let bag = bag_with(&[
            (ItemId::MasterBall, 0),
            (ItemId::UltraBall, 0),
            (ItemId::GreatBall, 2),
        ]);
        assert_eq!(bag.best_pokeball().map(|i| i.id), Some(ItemId::GreatBall));
    }

    #[test]
    fn test_best_pokeball_no_balls_in_bag() {
        let bag = bag_with(&[(ItemId::Antidote, 5), (ItemId::Potion, 3)]);
        assert_eq!(bag.best_pokeball(), None);
    }
}

pub trait BagReader {
    fn read_bag(&self) -> Bag;
    /// PC item storage (`wNumBoxItems`/`wBoxItems`) — the same count-then-pairs layout as the bag,
    /// which is why one trait serves both.
    fn read_pc_items(&self) -> Bag;
}

impl BagReader for MMU {
    fn read_bag(&self) -> Bag {
        let count = self.read_pointer(&pokered_symbols::wNumBagItems) as usize;
        let base  = pokered_symbols::wBagItems.address;
        let items = (0..count)
            .filter_map(|i| {
                let item_base = base + i as u16 * 2;
                if let Some(id) = ItemId::from_repr(self.read(item_base)) {
                    Some(BagItem { id, quantity: self.read(item_base + 1) })
                } else {
                    None
                }
            })
            .collect();
        Bag::new(items)
    }

    fn read_pc_items(&self) -> Bag {
        let count = self.read_pointer(&pokered_symbols::wNumBoxItems) as usize;
        let base  = pokered_symbols::wBoxItems.address;
        Bag::new(
            (0..count)
                .filter_map(|i| {
                    let item_base = base + i as u16 * 2;
                    ItemId::from_repr(self.read(item_base))
                        .map(|id| BagItem { id, quantity: self.read(item_base + 1) })
                })
                .collect(),
        )
    }
}

pub trait BagWriter {
    /// Replaces the bag contents with the given items.
    fn write_bag(&mut self, items: &Bag);
}

impl BagWriter for MMU {
    fn write_bag(&mut self, items: &Bag) {
        let count = items.len() as u8;
        self.write(pokered_symbols::wNumBagItems.address, count);
        let base = pokered_symbols::wBagItems.address;
        for (i, item) in items.iter().enumerate() {
            self.write(base + i as u16 * 2,     item.id as u8);
            self.write(base + i as u16 * 2 + 1, item.quantity.min(99));
        }
        // FF terminator
        self.write(base + count as u16 * 2, 0xFF);
    }
}