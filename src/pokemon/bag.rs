use crate::mmu::MMU;
use crate::pokemon::battle::BagItem;
use crate::pokemon::item::ItemId;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::ram::{RAM, ROM};

pub trait BagReader {
    fn read_bag(&self) -> Vec<BagItem>;
}

impl BagReader for MMU {
    fn read_bag(&self) -> Vec<BagItem> {
        let count = self.read_pointer(&pokered_symbols::wNumBagItems) as usize;
        let base  = pokered_symbols::wBagItems.address;
        (0..count)
            .filter_map(|i| {
                let item_base = base + i as u16 * 2;
                if let Some(id) = ItemId::from_repr(self.read(item_base)) {
                    Some(BagItem { id, quantity: self.read(item_base + 1) })
                } else {
                    None
                }
            })
            .collect()
    }
}

pub trait BagWriter {
    /// Replaces the bag contents with the given items.
    fn write_bag(&mut self, items: &[BagItem]);
}

impl BagWriter for MMU {
    fn write_bag(&mut self, items: &[BagItem]) {
        let count = items.len().min(20) as u8;
        self.write(pokered_symbols::wNumBagItems.address, count);
        let base = pokered_symbols::wBagItems.address;
        for (i, item) in items.iter().take(20).enumerate() {
            self.write(base + i as u16 * 2,     item.id as u8);
            self.write(base + i as u16 * 2 + 1, item.quantity.min(99));
        }
        // FF terminator
        self.write(base + count as u16 * 2, 0xFF);
    }
}