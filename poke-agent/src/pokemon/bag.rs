use gb::mmu::MMU;
use crate::pokemon::item::ItemId;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use gb::ram::{RAM, ROM};
pub use poke_core::bag::*;

pub trait BagReader {
    fn read_bag(&self) -> Bag;
    /// PC item storage (`wNumBoxItems`/`wBoxItems`) — the same count-then-pairs layout as the
    /// bag, which is why one trait serves both.
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