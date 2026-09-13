//! `engine/items/inventory.asm` and `engine/items/get_bag_item_quantity.asm`: the bag and the PC's
//! item box, as the count-and-slots list the cartridge keeps and the quirks of writing to it.

use poke_core::bag::BagItem;
use poke_core::item::{is_key_item, ItemId};
use serde::{Deserialize, Serialize};

/// `BAG_ITEM_CAPACITY`, `PC_ITEM_CAPACITY`: how many *slots*, not how many items.
pub const BAG_ITEM_CAPACITY: u8 = 20;
pub const PC_ITEM_CAPACITY: u8 = 50;

/// A count byte, then `(id, quantity)` slots, then `$FF`. Which inventory it is decides only its
/// capacity, which is why the cartridge passes that as an address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    pub items: Vec<BagItem>,
    pub capacity: u8,
}

/// The byte that ends the slots, and that a quantity of 255 is indistinguishable from.
const TERMINATOR: u8 = 0xFF;

fn slot_at(bytes: &[u8], i: usize) -> BagItem {
    let id = ItemId::from_repr(bytes[i * 2]).expect("an item in a slot");
    BagItem::new(id, bytes[i * 2 + 1])
}

/// `AddItemToInventory_`'s arguments, as its fixture stores them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddInput {
    pub inventory: Inventory,
    pub item: ItemId,
    pub quantity: u8,
}

/// `RemoveItemFromInventory_`'s, likewise. `slot` is `wWhichPokemon`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveInput {
    pub inventory: Inventory,
    pub slot: u8,
    pub quantity: u8,
}

impl Inventory {
    pub fn bag(items: Vec<BagItem>) -> Self {
        Self { items, capacity: BAG_ITEM_CAPACITY }
    }

    pub fn pc(items: Vec<BagItem>) -> Self {
        Self { items, capacity: PC_ITEM_CAPACITY }
    }

    /// `AddItemToInventory_`, returning its carry. `quantity` is never capped on the way in, so a
    /// new slot can be made holding more than 99.
    ///
    /// Room for a new slot is worked out once, before anything is written, and never again: a
    /// quantity that spills twice can therefore take the inventory one slot past its capacity.
    pub fn add(&mut self, item: ItemId, quantity: u8) -> bool {
        let room = (self.items.len() as u8).wrapping_sub(self.capacity) != 0;
        let mut quantity = quantity;
        let mut slot = 0;
        while slot < self.items.len() {
            if self.items[slot].id != item {
                slot += 1;
                continue;
            }
            // The sum is one byte, so 99 + 200 is 43 and goes into the slot whole.
            let total = self.items[slot].quantity.wrapping_add(quantity);
            if total < 100 {
                self.items[slot].quantity = total;
                return true;
            }
            if !room {
                return false;
            }
            // The slot is filled to 99 and the rest carries on down the same list, so one call
            // can fill several slots of one item.
            self.items[slot].quantity = 99;
            quantity = total - 99;
            slot += 1;
        }
        if !room {
            return false;
        }
        self.items.push(BagItem::new(item, quantity));
        true
    }

    /// `RemoveItemFromInventory_`. The subtraction is one byte and is not checked, so removing
    /// more than a slot holds wraps it round and keeps the slot; only landing exactly on zero
    /// takes the slot out.
    ///
    /// An emptied slot is closed up by copying the bytes after it up two, stopping at the first
    /// `$FF` copied. Quantities are copied by that loop as well as ids, so a slot holding exactly
    /// 255 reads as the terminator: the shift stops there, every slot past it stays where it was,
    /// and the count still drops by one, which loses the last slot and doubles one of the others.
    pub fn remove(&mut self, slot: usize, quantity: u8) {
        let left = self.items[slot].quantity.wrapping_sub(quantity);
        self.items[slot].quantity = left;
        if left != 0 {
            return;
        }
        let mut bytes = self.bytes();
        let (mut destination, mut source) = (slot * 2, slot * 2 + 2);
        loop {
            let byte = bytes[source];
            bytes[destination] = byte;
            source += 1;
            destination += 1;
            if byte == TERMINATOR {
                break;
            }
        }
        self.items = (0..self.items.len() - 1).map(|i| slot_at(&bytes, i)).collect();
    }

    /// The slots as the cartridge lays them out: the pairs, the terminator, and `$FF` to the end
    /// of the region, which is what a shift that runs past the terminator reads.
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = vec![TERMINATOR; self.capacity as usize * 2 + 1];
        for (i, item) in self.items.iter().enumerate() {
            bytes[i * 2] = item.id as u8;
            bytes[i * 2 + 1] = item.quantity;
        }
        bytes
    }

    /// `GetQuantityOfItemInBag`: the first slot's quantity, or 0 when the item is not there. The
    /// routine reads `wNumBagItems` whatever it is handed, so it only ever answers for the bag.
    pub fn quantity_of(&self, item: ItemId) -> u8 {
        self.items.iter().find(|slot| slot.id == item).map_or(0, |slot| slot.quantity)
    }

    /// `TossItem_`'s refusal: an HM or a key item is too important to throw away. The yes/no box
    /// and the messages belong to the mode that asks.
    pub fn may_toss(item: ItemId) -> bool {
        !item.is_hm() && !is_key_item(item)
    }
}

#[cfg(test)]
mod tests {
    use crate::fixtures::cases;
    use super::*;

    fn bag(slots: &[(ItemId, u8)]) -> Inventory {
        Inventory::bag(slots.iter().map(|&(id, q)| BagItem::new(id, q)).collect())
    }

    #[test]
    fn an_item_already_there_stacks_and_a_new_one_takes_a_slot() {
        let mut inventory = bag(&[(ItemId::Potion, 3)]);
        assert!(inventory.add(ItemId::Potion, 2));
        assert_eq!(inventory.quantity_of(ItemId::Potion), 5);
        assert!(inventory.add(ItemId::Antidote, 1));
        assert_eq!(inventory.items.len(), 2);
    }

    #[test]
    fn a_stack_over_99_fills_the_slot_and_spills_into_a_new_one() {
        let mut inventory = bag(&[(ItemId::Potion, 90)]);
        assert!(inventory.add(ItemId::Potion, 20));
        assert_eq!(inventory.items, [BagItem::new(ItemId::Potion, 99), BagItem::new(ItemId::Potion, 11)]);
    }

    #[test]
    fn a_sum_that_wraps_a_byte_lands_in_the_slot_whole() {
        let mut inventory = bag(&[(ItemId::Potion, 99)]);
        assert!(inventory.add(ItemId::Potion, 200), "99 + 200 is 43 in a byte, and 43 is under 100");
        assert_eq!(inventory.items, [BagItem::new(ItemId::Potion, 43)]);
    }

    #[test]
    fn a_new_slot_is_not_capped_at_99() {
        let mut inventory = bag(&[]);
        assert!(inventory.add(ItemId::Potion, 200));
        assert_eq!(inventory.quantity_of(ItemId::Potion), 200);
    }

    #[test]
    fn a_full_inventory_refuses_a_new_item_but_still_stacks() {
        // Items 1 to 20, which run from the Master Ball to the Potion.
        let full: Vec<_> = (0..BAG_ITEM_CAPACITY).map(|i| (ItemId::from_repr(i + 1).unwrap(), 1)).collect();
        let mut inventory = bag(&full);
        assert!(!inventory.add(ItemId::Nugget, 1), "no slot free");
        assert!(inventory.add(ItemId::MasterBall, 1), "but slot 1 is already Master Balls");
        assert_eq!(inventory.quantity_of(ItemId::MasterBall), 2);
    }

    #[test]
    fn removing_a_whole_slot_takes_it_out_and_removing_too_much_wraps() {
        let mut inventory = bag(&[(ItemId::Potion, 2), (ItemId::Antidote, 1)]);
        inventory.remove(0, 2);
        assert_eq!(inventory.items, [BagItem::new(ItemId::Antidote, 1)]);
        inventory.remove(0, 2);
        assert_eq!(inventory.items, [BagItem::new(ItemId::Antidote, 255)], "1 - 2 is 255, and the slot stays");
    }

    #[test]
    fn every_harvested_add_matches() {
        for (input, (inventory, carry), _) in cases::<AddInput, (Inventory, bool)>(include_str!("../../fixtures/items/add_item.jsonl")) {
            let mut got = input.inventory.clone();
            assert_eq!((got.add(input.item, input.quantity), got), (carry, inventory), "{input:?}");
        }
    }

    #[test]
    fn every_harvested_remove_matches() {
        for (input, expected, _) in cases::<RemoveInput, Inventory>(include_str!("../../fixtures/items/remove_item.jsonl")) {
            let mut got = input.inventory.clone();
            got.remove(input.slot as usize, input.quantity);
            assert_eq!(got, expected, "{input:?}");
        }
    }

    #[test]
    fn every_harvested_lookup_matches() {
        for ((inventory, item), quantity, _) in cases::<(Inventory, ItemId), u8>(include_str!("../../fixtures/items/quantity_in_bag.jsonl")) {
            assert_eq!(inventory.quantity_of(item), quantity, "{item} in {inventory:?}");
        }
    }

    /// `price` and `is_key_item` are `poke-core`'s, but the cartridge's own answers are pinned
    /// here, where the fixtures are.
    #[test]
    fn every_harvested_price_and_key_item_matches() {
        for (item, expected, _) in cases::<ItemId, Option<[u8; 3]>>(include_str!("../../fixtures/items/item_price.jsonl")) {
            assert_eq!(poke_core::item::price(item), expected, "{item}");
        }
        for (item, expected, _) in cases::<ItemId, bool>(include_str!("../../fixtures/items/is_key_item.jsonl")) {
            assert_eq!(is_key_item(item), expected, "{item}");
        }
    }

    #[test]
    fn a_quantity_of_255_stops_the_slots_moving_up() {
        let mut inventory = bag(&[(ItemId::Potion, 1), (ItemId::TownMap, 255), (ItemId::Antidote, 4)]);
        inventory.remove(0, 1);
        assert_eq!(inventory.items, [BagItem::new(ItemId::TownMap, 255), BagItem::new(ItemId::TownMap, 255)],
            "255 reads as the terminator, so the copy stops and the Antidote never moves up");
    }

    #[test]
    fn an_hm_and_a_key_item_cannot_be_tossed() {
        assert!(!Inventory::may_toss(ItemId::Hm01Cut));
        assert!(!Inventory::may_toss(ItemId::TownMap));
        assert!(Inventory::may_toss(ItemId::Potion));
        assert!(Inventory::may_toss(ItemId::Tm01MegaPunch), "a TM is not a key item");
    }
}
