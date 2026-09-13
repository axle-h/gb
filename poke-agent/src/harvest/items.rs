use poke_core::bag::BagItem;
use poke_core::item::ItemId;
use pokered::systems::inventory::{AddInput, Inventory, RemoveInput, BAG_ITEM_CAPACITY, PC_ITEM_CAPACITY};
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointer};
use super::Oracle;

fn oracle() -> Oracle {
    Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"))
}

/// Where an inventory of this capacity lives: the count byte, then the slots.
fn region(capacity: u8) -> (DmgPointer, DmgPointer) {
    match capacity {
        BAG_ITEM_CAPACITY => (sym::wNumBagItems, sym::wBagItems),
        PC_ITEM_CAPACITY => (sym::wNumBoxItems, sym::wBoxItems),
        other => panic!("no inventory holds {other} slots"),
    }
}

/// The whole region, so nothing an earlier case left is read as a slot. Past the terminator it is
/// `$FF` too: a spill from the last slot reads on past it, and only `$FF` there stops the scan.
fn write_inventory(oracle: &mut Oracle, inventory: &Inventory) -> DmgPointer {
    let (count, slots) = region(inventory.capacity);
    let mut bytes = vec![0xFF; inventory.capacity as usize * 2 + 1];
    for (i, slot) in inventory.items.iter().enumerate() {
        bytes[i * 2] = slot.id as u8;
        bytes[i * 2 + 1] = slot.quantity;
    }
    oracle.write(count, &[inventory.items.len() as u8]);
    oracle.write(slots, &bytes);
    count
}

fn read_inventory(oracle: &Oracle, capacity: u8) -> Inventory {
    let (count, slots) = region(capacity);
    let len = oracle.read(count, 1)[0] as usize;
    let bytes = oracle.read(slots, len * 2);
    let items = bytes.chunks(2)
        .map(|slot| BagItem::new(ItemId::from_repr(slot[0]).expect("an item in a slot"), slot[1]))
        .collect();
    Inventory { items, capacity }
}

/// `AddItemToInventory_`, returning its carry beside the inventory it left.
fn add_item(oracle: &mut Oracle, input: &AddInput) -> (Inventory, bool) {
    let count = write_inventory(oracle, &input.inventory);
    oracle.write(sym::wCurItem, &[input.item as u8]);
    oracle.write(sym::wItemQuantity, &[input.quantity]);
    oracle.registers_mut().set_hl(count.address);
    oracle.call(sym::AddItemToInventory_);
    let carry = oracle.registers().flags.c;
    assert_eq!(oracle.read(sym::wItemQuantity, 1)[0], input.quantity, "the quantity is put back");
    (read_inventory(oracle, input.inventory.capacity), carry)
}

/// `RemoveItemFromInventory_`.
fn remove_item(oracle: &mut Oracle, input: &RemoveInput) -> Inventory {
    let count = write_inventory(oracle, &input.inventory);
    oracle.write(sym::wWhichPokemon, &[input.slot]);
    oracle.write(sym::wItemQuantity, &[input.quantity]);
    oracle.registers_mut().set_hl(count.address);
    oracle.call(sym::RemoveItemFromInventory_);
    read_inventory(oracle, input.inventory.capacity)
}

/// `GetQuantityOfItemInBag`, a predef: it takes `b` back from `wPredefBC`. It reads `wNumBagItems`
/// outright rather than taking an inventory, so only a bag can be asked.
fn quantity_in_bag(oracle: &mut Oracle, inventory: &Inventory, item: ItemId) -> u8 {
    write_inventory(oracle, inventory);
    oracle.write(sym::wPredefBC, &[item as u8, 0]);
    oracle.call(sym::GetQuantityOfItemInBag);
    oracle.registers().b
}

/// `GetItemPrice` into `hItemPrice`, three BCD bytes. `wItemPrices` is the pointer the mart code
/// leaves there, and `wListMenuID` must not be the moves list, which reads a different table.
fn item_price(oracle: &mut Oracle, item: ItemId) -> Option<[u8; 3]> {
    const SENTINEL: [u8; 3] = [0xAA, 0xBB, 0xCC];
    oracle.write(sym::hItemPrice, &SENTINEL);
    oracle.write(sym::wItemPrices, &sym::ItemPrices.address.to_le_bytes());
    oracle.write(sym::wListMenuID, &[0]);
    oracle.write(sym::wCurItem, &[item as u8]);
    oracle.call(sym::GetItemPrice);
    let price: [u8; 3] = oracle.read(sym::hItemPrice, 3).try_into().unwrap();
    (price != SENTINEL).then_some(price)
}

/// `IsKeyItem_` into `wIsKeyItem`.
fn is_key_item(oracle: &mut Oracle, item: ItemId) -> bool {
    oracle.write(sym::wCurItem, &[item as u8]);
    oracle.call(sym::IsKeyItem_);
    oracle.read(sym::wIsKeyItem, 1)[0] != 0
}

/// The oracle's own check, against what the bag plainly does.
#[test]
fn the_oracle_stacks_spills_and_empties_a_slot() {
    let mut oracle = oracle();
    let bag = |slots: &[(ItemId, u8)]| Inventory::bag(slots.iter().map(|&(id, q)| BagItem::new(id, q)).collect());

    let input = AddInput { inventory: bag(&[(ItemId::Potion, 3)]), item: ItemId::Potion, quantity: 2 };
    assert_eq!(add_item(&mut oracle, &input), (bag(&[(ItemId::Potion, 5)]), true));

    let input = AddInput { inventory: bag(&[(ItemId::Potion, 90)]), item: ItemId::Potion, quantity: 20 };
    assert_eq!(add_item(&mut oracle, &input), (bag(&[(ItemId::Potion, 99), (ItemId::Potion, 11)]), true),
        "the slot fills to 99 and the rest starts another");

    let input = RemoveInput { inventory: bag(&[(ItemId::Potion, 2), (ItemId::Antidote, 1)]), slot: 0, quantity: 2 };
    assert_eq!(remove_item(&mut oracle, &input), bag(&[(ItemId::Antidote, 1)]), "an emptied slot closes up");

    assert_eq!(quantity_in_bag(&mut oracle, &bag(&[(ItemId::Potion, 7)]), ItemId::Potion), 7);
    assert_eq!(quantity_in_bag(&mut oracle, &bag(&[(ItemId::Potion, 7)]), ItemId::Antidote), 0);
    assert_eq!(item_price(&mut oracle, ItemId::Potion), Some([0x00, 0x03, 0x00]), "¥300");
    assert_eq!(item_price(&mut oracle, ItemId::Hm01Cut), None, "an HM writes nothing");
    assert!(is_key_item(&mut oracle, ItemId::TownMap) && !is_key_item(&mut oracle, ItemId::Potion));
}

#[cfg(feature = "slow-tests")]
mod harvest {
    use rand::rngs::StdRng;
    use rand::{RngExt, SeedableRng};
    use super::super::{write_fixture, Case};
    use super::*;

    /// Every item the cartridge has a row for, machines included.
    fn every_item() -> Vec<ItemId> {
        (1..=u8::MAX).filter_map(ItemId::from_repr).collect()
    }

    fn an_item(rng: &mut StdRng) -> ItemId {
        let all = every_item();
        all[rng.random_range(0..all.len())]
    }

    /// A quantity worth trying: the boundaries a slot is compared against, or anything.
    fn a_quantity(rng: &mut StdRng) -> u8 {
        match rng.random_range(0..4) {
            0 => [0, 1, 98, 99, 100, 101, 255][rng.random_range(0..7)],
            _ => rng.random(),
        }
    }

    fn an_inventory(rng: &mut StdRng) -> Inventory {
        let capacity = if rng.random_bool(0.75) { BAG_ITEM_CAPACITY } else { PC_ITEM_CAPACITY };
        let len = match rng.random_range(0..5) {
            0 => 0,
            1 => capacity as usize,
            2 => capacity as usize - 1,
            _ => rng.random_range(1..capacity as usize),
        };
        let items = (0..len).map(|_| BagItem::new(an_item(rng), a_quantity(rng))).collect();
        Inventory { items, capacity }
    }

    /// The cases worth forcing, which random ones reach rarely or never.
    fn edge_cases() -> Vec<AddInput> {
        let bag = |slots: &[(ItemId, u8)]| Inventory::bag(slots.iter().map(|&(id, q)| BagItem::new(id, q)).collect());
        let full: Vec<_> = (0..BAG_ITEM_CAPACITY).map(|i| (ItemId::from_repr(i + 1).unwrap(), 1)).collect();
        let nearly_full: Vec<_> = (0..BAG_ITEM_CAPACITY - 1).map(|i| (ItemId::from_repr(i + 1).unwrap(), 99)).collect();
        vec![
            AddInput { inventory: bag(&[]), item: ItemId::Potion, quantity: 200 },
            AddInput { inventory: bag(&[(ItemId::Potion, 90)]), item: ItemId::Potion, quantity: 20 },
            AddInput { inventory: bag(&[(ItemId::Potion, 99)]), item: ItemId::Potion, quantity: 200 },
            AddInput { inventory: bag(&[(ItemId::Potion, 99), (ItemId::Potion, 99)]), item: ItemId::Potion, quantity: 100 },
            AddInput { inventory: bag(&full), item: ItemId::Potion, quantity: 1 },
            AddInput { inventory: bag(&full), item: ItemId::MasterBall, quantity: 1 },
            AddInput { inventory: bag(&full), item: ItemId::MasterBall, quantity: 200 },
            AddInput { inventory: bag(&nearly_full), item: ItemId::MasterBall, quantity: 50 },
            AddInput { inventory: bag(&[(ItemId::Potion, 0)]), item: ItemId::Potion, quantity: 0 },
            AddInput { inventory: Inventory::pc(vec![]), item: ItemId::Potion, quantity: 99 },
        ]
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/items/*.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_items() {
        let mut oracle = oracle();
        let mut rng = StdRng::seed_from_u64(0xBA6);

        let mut inputs = edge_cases();
        while inputs.len() < 500 {
            let inventory = an_inventory(&mut rng);
            // Half the time aim at an item already in there, so the stacking path is taken.
            let item = match inventory.items.first() {
                Some(slot) if rng.random_bool(0.5) => slot.id,
                _ => an_item(&mut rng),
            };
            inputs.push(AddInput { inventory, item, quantity: a_quantity(&mut rng) });
        }
        let cases: Vec<Case<AddInput, (Inventory, bool)>> = inputs.into_iter()
            .map(|input| { let output = add_item(&mut oracle, &input); Case { input, output, rng: vec![] } })
            .collect();
        write_fixture("items", "add_item", &cases);

        let cases: Vec<Case<RemoveInput, Inventory>> = (0..400).map(|_| {
            let mut inventory = an_inventory(&mut rng);
            if inventory.items.is_empty() {
                inventory.items.push(BagItem::new(an_item(&mut rng), a_quantity(&mut rng)));
            }
            let slot = rng.random_range(0..inventory.items.len());
            // Often take exactly what is there, which is the only way a slot closes up.
            let quantity = if rng.random_bool(0.4) { inventory.items[slot].quantity } else { a_quantity(&mut rng) };
            let input = RemoveInput { inventory, slot: slot as u8, quantity };
            let output = remove_item(&mut oracle, &input);
            Case { input, output, rng: vec![] }
        }).collect();
        write_fixture("items", "remove_item", &cases);

        let cases: Vec<Case<(Inventory, ItemId), u8>> = (0..200).map(|_| {
            // The routine only ever reads the bag, so a box would be answered from the wrong list.
            let mut inventory = an_inventory(&mut rng);
            inventory.items.truncate(BAG_ITEM_CAPACITY as usize);
            inventory.capacity = BAG_ITEM_CAPACITY;
            let item = match inventory.items.first() {
                Some(slot) if rng.random_bool(0.5) => slot.id,
                _ => an_item(&mut rng),
            };
            let output = quantity_in_bag(&mut oracle, &inventory, item);
            Case { input: (inventory, item), output, rng: vec![] }
        }).collect();
        write_fixture("items", "quantity_in_bag", &cases);

        let cases: Vec<Case<ItemId, Option<[u8; 3]>>> = every_item().into_iter()
            .map(|item| Case { input: item, output: item_price(&mut oracle, item), rng: vec![] })
            .collect();
        write_fixture("items", "item_price", &cases);

        let cases: Vec<Case<ItemId, bool>> = every_item().into_iter()
            .map(|item| Case { input: item, output: is_key_item(&mut oracle, item), rng: vec![] })
            .collect();
        write_fixture("items", "is_key_item", &cases);
    }
}
