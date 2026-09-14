use poke_core::bag::BagItem;
use poke_core::item::ItemId;
use poke_core::move_name::PokemonMoveName;
use poke_core::species::PokemonSpecies;
use pokered::party::PartyMon;
use pokered::rng::GameRng;
use pokered::systems::add_mon::{new_party_mon, Origin};
use pokered::systems::inventory::{AddInput, Inventory, RemoveInput, BAG_ITEM_CAPACITY, PC_ITEM_CAPACITY};
use pokered::systems::item_use::{ItemOnMon, MachineOnSpecies, Medicine, MedicineMessage};
use pokered::systems::money::PriceInput;
use pokered::systems::pp::PpInput;
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

/// A label `pokered.sym` has and the generated symbols skip: a local one, with a dot in it.
fn local_label(label: &str) -> DmgPointer {
    use crate::pokemon::symbols::DmgBank;
    include_str!("../../../vendor/pokered/pokered.sym").lines()
        .find_map(|line| {
            let (at, name) = line.split_once(' ')?;
            let (bank, address) = at.split_once(':')?;
            (name == label).then(|| DmgPointer {
                bank: DmgBank::ROM { bank: u8::from_str_radix(bank, 16).unwrap() },
                address: u16::from_str_radix(address, 16).unwrap(),
            })
        })
        .unwrap_or_else(|| panic!("no {label} in pokered.sym"))
}

const PARTYMON_STRUCT_LENGTH: u16 = 0x2C;
/// A slot the mon is written to, and a `wPlayerMonNumber` that is not it, so the battle mon's copy
/// the medicine keeps in step is never touched.
const SLOT: u8 = 2;
const NOT_THE_SLOT: u8 = 5;

fn write_party_mon(oracle: &mut Oracle, mon: &PartyMon) -> u16 {
    let at = sym::wPartyMon1 + PARTYMON_STRUCT_LENGTH * SLOT as u16;
    oracle.write(sym::wPartyCount, &[SLOT + 1]);
    oracle.write(sym::wPartySpecies + SLOT as u16, &[mon.mon.species as u8, 0xFF]);
    oracle.write(at, &super::pokemon::encode_party(mon));
    oracle.write(sym::wPlayerMonNumber, &[NOT_THE_SLOT]);
    oracle.write(sym::wIsInBattle, &[0]);
    oracle.write(sym::wPseudoItemID, &[0]);
    at.address
}

fn read_party_mon(oracle: &Oracle) -> PartyMon {
    let at = sym::wPartyMon1 + PARTYMON_STRUCT_LENGTH * SLOT as u16;
    super::pokemon::decode_party(&oracle.read(at, PARTYMON_STRUCT_LENGTH as usize))
}

/// `ItemUseMedicine` entered at `.checkItemType`, where the party menu has answered and the item
/// and the mon are in hand, and stopped before it prints or animates anything.
fn use_medicine(oracle: &mut Oracle, input: &ItemOnMon) -> (Medicine, PartyMon) {
    let at = write_party_mon(oracle, &input.mon);
    oracle.write(sym::wCurItem, &[input.item as u8]);
    oracle.write(sym::wUsedItemOnWhichPokemon, &[SLOT]);
    let registers = oracle.registers_mut();
    registers.set_hl(at);
    registers.set_de(u16::from_be_bytes([SLOT, input.mon.mon.species as u8]));
    let done = local_label("ItemUseMedicine.doneHealing");
    let no_effect = local_label("ItemUseMedicine.healingItemNoEffect");
    let (_, stop) = oracle.call_until(local_label("ItemUseMedicine.checkItemType"), &[done, no_effect]);
    let mon = read_party_mon(oracle);
    if stop == Some(no_effect) {
        return (Medicine::NoEffect, mon);
    }
    assert_eq!(stop, Some(done), "the medicine returned before it finished");
    // `.doneHealing` sorts the two by the item as it now stands, a Full Heal where a Full Restore
    // found nothing to heal.
    let item = oracle.read(sym::wCurItem, 1)[0];
    let messages = [MedicineMessage::Antidote, MedicineMessage::BurnHeal, MedicineMessage::IceHeal,
        MedicineMessage::Awakening, MedicineMessage::ParlyzHeal, MedicineMessage::Potion,
        MedicineMessage::FullHeal, MedicineMessage::Revive];
    if item < ItemId::FullRestore as u8 || item == ItemId::FullHeal as u8 {
        const ANTIDOTE_MSG: u8 = 0xF0;
        let id = oracle.read(sym::wPartyMenuTypeOrMessageID, 1)[0];
        return (Medicine::Cured(messages[(id - ANTIDOTE_MSG) as usize]), mon);
    }
    let word = |pointer| {
        let bytes = oracle.read(pointer, 2);
        u16::from_le_bytes([bytes[0], bytes[1]])
    };
    let reviving = item == ItemId::Revive as u8 || item == ItemId::MaxRevive as u8;
    let message = if reviving { MedicineMessage::Revive } else { MedicineMessage::Potion };
    (Medicine::Healed { old: word(sym::wHPBarOldHP), new: word(sym::wHPBarNewHP), message }, mon)
}

/// `.useVitamin`, stopped where it has raised the stat and is about to name it.
fn use_vitamin(oracle: &mut Oracle, input: &ItemOnMon) -> (bool, PartyMon) {
    let at = write_party_mon(oracle, &input.mon);
    oracle.write(sym::wCurItem, &[input.item as u8]);
    let registers = oracle.registers_mut();
    registers.set_hl(at);
    registers.set_de(u16::from_be_bytes([SLOT, input.mon.mon.species as u8]));
    let rose = local_label("ItemUseMedicine.gotStatName");
    let no_effect = local_label("ItemUseMedicine.vitaminNoEffect");
    let (_, stop) = oracle.call_until(local_label("ItemUseMedicine.useVitamin"), &[rose, no_effect]);
    assert!(stop.is_some(), "the vitamin returned before it finished");
    (stop == Some(rose), read_party_mon(oracle))
}

/// `CanLearnTM` for the machine's move, which answers in `c`.
fn can_learn_tm(oracle: &mut Oracle, input: MachineOnSpecies) -> bool {
    oracle.write(sym::wCurPartySpecies, &[input.species as u8]);
    oracle.write(sym::wMoveNum, &[input.mv as u8]);
    oracle.call(sym::CanLearnTM);
    oracle.registers().c != 0
}

/// `.useVitamin` with a Rare Candy, stopped as `.useRareCandy` calls `RedrawPartyMenu` with the
/// mon changed, or at `.vitaminNoEffect`.
fn use_rare_candy(oracle: &mut Oracle, mon: &PartyMon) -> (bool, PartyMon) {
    let at = write_party_mon(oracle, mon);
    oracle.write(sym::wCurItem, &[ItemId::RareCandy as u8]);
    let registers = oracle.registers_mut();
    registers.set_hl(at);
    registers.set_de(u16::from_be_bytes([SLOT, mon.mon.species as u8]));
    let no_effect = local_label("ItemUseMedicine.vitaminNoEffect");
    let (_, stop) = oracle.call_until(local_label("ItemUseMedicine.useVitamin"), &[sym::RedrawPartyMenu, no_effect]);
    assert!(stop.is_some(), "the Rare Candy returned before it finished");
    (stop == Some(sym::RedrawPartyMenu), read_party_mon(oracle))
}

/// `ItemUsePPRestore.restorePP` on one move of the mon: the byte it leaves, or `None` where it
/// returned with the zero flag, which is its "no effect".
fn restore_pp(oracle: &mut Oracle, input: &PpInput) -> Option<u8> {
    let mon = pp_mon(input);
    write_party_mon(oracle, &mon);
    oracle.write(sym::wWhichPokemon, &[SLOT]);
    oracle.write(sym::wCurrentMenuItem, &[0]);
    oracle.write(sym::wMonDataLocation, &[0]);
    let item = if input.full { ItemId::MaxEther } else { ItemId::Ether };
    oracle.write(sym::wPPRestoreItem, &[item as u8]);
    oracle.call(local_label("ItemUsePPRestore.restorePP"));
    let restored = !oracle.registers().flags.z;
    restored.then(|| read_party_mon(oracle).mon.pp[0])
}

/// `.PPNotMaxedOut`: one more PP Up counted, then `RestoreBonusPP` for that move alone.
fn use_pp_up(oracle: &mut Oracle, input: &PpInput) -> Option<u8> {
    if input.pp >= 3 << 6 {
        return None;
    }
    let mut mon = pp_mon(input);
    mon.mon.pp[0] += 1 << 6;
    write_party_mon(oracle, &mon);
    oracle.write(sym::wWhichPokemon, &[SLOT]);
    oracle.write(sym::wCurrentMenuItem, &[0]);
    oracle.write(sym::wUsingPPUp, &[1]);
    oracle.call(sym::RestoreBonusPP);
    Some(read_party_mon(oracle).mon.pp[0])
}

/// A mon knowing the move in its first slot with the PP byte as given; nothing else is read.
fn pp_mon(input: &PpInput) -> PartyMon {
    let mut mon = new_party_mon(PokemonSpecies::Mew, 50, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
    mon.mon.moves = [Some(input.mv), None, None, None];
    mon.mon.pp = [input.pp, 0, 0, 0];
    mon
}

/// `DisplayChooseQuantityMenu` entered at `.handleNewQuantity` in a mart, stopped as it waits for
/// the next key: the total, and row 10 from the quantity to the box's right edge.
fn choose_quantity_price(oracle: &mut Oracle, input: &PriceInput) -> ([u8; 3], Vec<u8>) {
    const PRICEDITEMLISTMENU: u8 = 2;
    let row = sym::wTileMap + 10 * 20;
    oracle.write(row, &[0x7F; 20]);
    oracle.write(sym::wListMenuID, &[PRICEDITEMLISTMENU]);
    oracle.write(sym::wItemQuantity, &[input.quantity]);
    oracle.write(sym::hItemPrice, &input.each);
    oracle.write(sym::hHalveItemPrices, &[input.halved as u8]);
    let waiting = local_label("DisplayChooseQuantityMenu.waitForKeyPressLoop");
    let (_, stop) = oracle.call_until(local_label("DisplayChooseQuantityMenu.handleNewQuantity"), &[waiting]);
    assert_eq!(stop, Some(waiting));
    (oracle.read(sym::hMoney, 3).try_into().unwrap(), oracle.read(row + 9, 11))
}

/// The oracle's own checks on the item-use arithmetic, worked out by hand.
#[test]
fn the_oracle_heals_raises_and_prices() {
    let mut oracle = oracle();
    let mut mon = new_party_mon(PokemonSpecies::Pidgey, 20, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
    let max = mon.stats[0];
    mon.mon.hp = max - 30;
    let (medicine, healed) = use_medicine(&mut oracle, &ItemOnMon { item: ItemId::Potion, mon: mon.clone() });
    assert_eq!(medicine, Medicine::Healed { old: max - 30, new: max - 10, message: MedicineMessage::Potion });
    assert_eq!(healed.mon.hp, max - 10);
    let (medicine, _) = use_medicine(&mut oracle, &ItemOnMon { item: ItemId::Revive, mon });
    assert_eq!(medicine, Medicine::NoEffect, "a Revive on a mon still standing");

    let mut fed = new_party_mon(PokemonSpecies::Pidgey, 20, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
    fed.mon.stat_exp[0] = 5;
    let (used, after) = use_vitamin(&mut oracle, &ItemOnMon { item: ItemId::HpUp, mon: fed });
    assert!(used);
    assert_eq!(after.mon.stat_exp[0], 10 << 8 | 5);

    let tackle = PokemonMoveName::Tackle;
    assert_eq!(restore_pp(&mut oracle, &PpInput { pp: 20, mv: tackle, full: false }), Some(30));
    assert_eq!(restore_pp(&mut oracle, &PpInput { pp: 35, mv: tackle, full: false }), None);
    assert_eq!(use_pp_up(&mut oracle, &PpInput { pp: 35, mv: tackle, full: false }), Some(1 << 6 | 42));

    assert!(can_learn_tm(&mut oracle, MachineOnSpecies { species: PokemonSpecies::Pidgey, mv: PokemonMoveName::Fly }));
    assert!(!can_learn_tm(&mut oracle, MachineOnSpecies { species: PokemonSpecies::Pidgey, mv: PokemonMoveName::Surf }));
    let candy = new_party_mon(PokemonSpecies::Pidgey, 20, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
    let (used, after) = use_rare_candy(&mut oracle, &candy);
    assert!(used);
    assert_eq!(after.level, 21);

    let (total, row) = choose_quantity_price(&mut oracle, &PriceInput { each: [0, 3, 0], quantity: 3, halved: true });
    assert_eq!(total, [0, 4, 0x50], "¥450");
    assert_eq!(row, [0xF6, 0xF9, 0x7F, 0x7F, 0x7F, 0x7F, 0xF0, 0xFA, 0xFB, 0xF6, 0x7F],
        "03, then ¥450 against the right: for a BCD number `LEADING_ZEROES` is the flag that skips them");
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

    fn a_party_mon(rng: &mut StdRng) -> PartyMon {
        let species = loop {
            if let Some(species) = PokemonSpecies::from_repr(rng.random()) {
                break species;
            }
        };
        let mut mon = new_party_mon(species, rng.random_range(1..=100), rng.random(), &Origin::Trainer, &mut GameRng::tape(vec![]));
        mon.mon.dvs = pokered::systems::stats::Dvs([rng.random(), rng.random()]);
        mon.mon.stat_exp = [(); 5].map(|_| match rng.random_range(0..4) {
            0 => rng.random_range(95..=110) << 8 | rng.random_range(0..=255),
            _ => rng.random(),
        });
        let max = mon.stats[0];
        mon.mon.hp = match rng.random_range(0..6) {
            0 => 0,
            1 => max,
            2 => max.saturating_sub(1),
            3 => max.saturating_sub(rng.random_range(0..=250)),
            _ => rng.random_range(0..=max),
        };
        mon.mon.status = match rng.random_range(0..8) {
            0..=2 => 0,
            3 => rng.random_range(1..=7),
            4 => 1 << rng.random_range(3..=6),
            _ => rng.random(),
        };
        mon
    }

    const MEDICINE: [ItemId; 16] = [ItemId::Antidote, ItemId::BurnHeal, ItemId::IceHeal, ItemId::Awakening,
        ItemId::ParlyzHeal, ItemId::FullRestore, ItemId::MaxPotion, ItemId::HyperPotion, ItemId::SuperPotion,
        ItemId::Potion, ItemId::FullHeal, ItemId::Revive, ItemId::MaxRevive, ItemId::FreshWater,
        ItemId::SodaPop, ItemId::Lemonade];
    const VITAMINS: [ItemId; 5] = [ItemId::HpUp, ItemId::Protein, ItemId::Iron, ItemId::Carbos, ItemId::Calcium];

    fn a_move(rng: &mut StdRng) -> PokemonMoveName {
        loop {
            if let Some(mv) = PokemonMoveName::from_repr(rng.random_range(1..=165)) {
                return mv;
            }
        }
    }

    fn a_pp(rng: &mut StdRng, mv: PokemonMoveName) -> u8 {
        let max = pokered::systems::pp::max_pp(mv, 0);
        let ups = rng.random_range(0..=3) << 6;
        match rng.random_range(0..4) {
            0 => ups | pokered::systems::pp::max_pp(mv, ups),
            1 => ups | max,
            2 => ups | rng.random_range(0..=max),
            _ => rng.random(),
        }
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/items/{use_medicine,use_vitamin,restore_pp,use_pp_up,choose_quantity_price,can_learn_tm,use_rare_candy}.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_item_use() {
        let mut oracle = oracle();
        let mut rng = StdRng::seed_from_u64(0x3ED1C);

        let cases: Vec<Case<ItemOnMon, (Medicine, PartyMon)>> = (0..800).map(|i| {
            let input = ItemOnMon { item: MEDICINE[i % MEDICINE.len()], mon: a_party_mon(&mut rng) };
            let output = use_medicine(&mut oracle, &input);
            Case { input, output, rng: vec![] }
        }).collect();
        write_fixture("items", "use_medicine", &cases);

        let cases: Vec<Case<ItemOnMon, (bool, PartyMon)>> = (0..300).map(|i| {
            let input = ItemOnMon { item: VITAMINS[i % VITAMINS.len()], mon: a_party_mon(&mut rng) };
            let output = use_vitamin(&mut oracle, &input);
            Case { input, output, rng: vec![] }
        }).collect();
        write_fixture("items", "use_vitamin", &cases);

        let cases: Vec<Case<PpInput, Option<u8>>> = (0..400).map(|_| {
            let mv = a_move(&mut rng);
            let input = PpInput { pp: a_pp(&mut rng, mv), mv, full: rng.random_bool(0.5) };
            Case { input, output: restore_pp(&mut oracle, &input), rng: vec![] }
        }).collect();
        write_fixture("items", "restore_pp", &cases);

        let cases: Vec<Case<PpInput, Option<u8>>> = (0..200).map(|_| {
            let mv = a_move(&mut rng);
            let input = PpInput { pp: a_pp(&mut rng, mv), mv, full: false };
            Case { input, output: use_pp_up(&mut oracle, &input), rng: vec![] }
        }).collect();
        write_fixture("items", "use_pp_up", &cases);

        let prices: Vec<[u8; 3]> = every_item().into_iter().filter_map(poke_core::item::price).collect();
        let cases: Vec<Case<PriceInput, ([u8; 3], Vec<u8>)>> = (0..400).map(|_| {
            let each = match rng.random_range(0..3) {
                0 => prices[rng.random_range(0..prices.len())],
                1 => [0, 0, 0],
                _ => [rng.random_range(0..10) << 4 | rng.random_range(0..10), rng.random_range(0..10) << 4 | rng.random_range(0..10), rng.random_range(0..10) << 4 | rng.random_range(0..10)],
            };
            let quantity = match rng.random_range(0..4) {
                0 => [0, 1, 2, 98, 99, 255][rng.random_range(0..6)],
                _ => rng.random_range(1..=99),
            };
            let input = PriceInput { each, quantity, halved: rng.random_bool(0.5) };
            Case { input, output: choose_quantity_price(&mut oracle, &input), rng: vec![] }
        }).collect();
        write_fixture("items", "choose_quantity_price", &cases);

        let machines: Vec<PokemonMoveName> = every_item().into_iter().filter_map(poke_core::item::machine_move).collect();
        let cases: Vec<Case<MachineOnSpecies, bool>> = (0..600).map(|_| {
            let species = a_party_mon(&mut rng).mon.species;
            let input = MachineOnSpecies { species, mv: machines[rng.random_range(0..machines.len())] };
            Case { input, output: can_learn_tm(&mut oracle, input), rng: vec![] }
        }).collect();
        write_fixture("items", "can_learn_tm", &cases);

        let cases: Vec<Case<PartyMon, (bool, PartyMon)>> = (0..300).map(|i| {
            let mut input = a_party_mon(&mut rng);
            if i % 10 == 0 {
                input.level = 100;
            }
            let output = use_rare_candy(&mut oracle, &input);
            Case { input, output, rng: vec![] }
        }).collect();
        write_fixture("items", "use_rare_candy", &cases);
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
