use crate::move_name::PokemonMoveName;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, strum_macros::Display, strum_macros::FromRepr, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum ItemId {
    MasterBall = 0x01,
    UltraBall = 0x02,
    GreatBall = 0x03,
    PokeBall = 0x04,
    TownMap = 0x05,
    Bicycle = 0x06,
    Surfboard = 0x07,
    SafariBall = 0x08,
    Pokedex = 0x09,
    MoonStone = 0x0A,
    Antidote = 0x0B,
    BurnHeal = 0x0C,
    IceHeal = 0x0D,
    Awakening = 0x0E,
    ParlyzHeal = 0x0F,
    FullRestore = 0x10,
    MaxPotion = 0x11,
    HyperPotion = 0x12,
    SuperPotion = 0x13,
    Potion = 0x14,
    BoulderBadge = 0x15,
    CascadeBadge = 0x16,
    ThunderBadge = 0x17,
    RainbowBadge = 0x18,
    SoulBadge = 0x19,
    MarshBadge = 0x1A,
    VolcanoBadge = 0x1B,
    EarthBadge = 0x1C,
    EscapeRope = 0x1D,
    Repel = 0x1E,
    OldAmber = 0x1F,
    FireStone = 0x20,
    ThunderStone = 0x21,
    WaterStone = 0x22,
    HpUp = 0x23,
    Protein = 0x24,
    Iron = 0x25,
    Carbos = 0x26,
    Calcium = 0x27,
    RareCandy = 0x28,
    DomeFossil = 0x29,
    HelixFossil = 0x2A,
    SecretKey = 0x2B,
    UnusedItem2C = 0x2C,
    BikeVoucher = 0x2D,
    XAccuracy = 0x2E,
    LeafStone = 0x2F,
    CardKey = 0x30,
    Nugget = 0x31,
    UnusedItem32 = 0x32,
    PokeDoll = 0x33,
    FullHeal = 0x34,
    Revive = 0x35,
    MaxRevive = 0x36,
    GuardSpec = 0x37,
    SuperRepel = 0x38,
    MaxRepel = 0x39,
    DireHit = 0x3A,
    Coin = 0x3B,
    FreshWater = 0x3C,
    SodaPop = 0x3D,
    Lemonade = 0x3E,
    SSTicket = 0x3F,
    GoldTeeth = 0x40,
    XAttack = 0x41,
    XDefend = 0x42,
    XSpeed = 0x43,
    XSpecial = 0x44,
    CoinCase = 0x45,
    OaksParcel = 0x46,
    Itemfinder = 0x47,
    SilphScope = 0x48,
    PokeFlute = 0x49,
    LiftKey = 0x4A,
    ExpAll = 0x4B,
    OldRod = 0x4C,
    GoodRod = 0x4D,
    SuperRod = 0x4E,
    PpUp = 0x4F,
    Ether = 0x50,
    MaxEther = 0x51,
    Elixer = 0x52,
    MaxElixer = 0x53,
    // An elevator's buttons are item ids, so that its floor list is an item list and its names are
    // in `ItemNames` beside the items' own.
    FloorB2F = 0x54,
    FloorB1F = 0x55,
    Floor1F = 0x56,
    Floor2F = 0x57,
    Floor3F = 0x58,
    Floor4F = 0x59,
    Floor5F = 0x5A,
    Floor6F = 0x5B,
    Floor7F = 0x5C,
    Floor8F = 0x5D,
    Floor9F = 0x5E,
    Floor10F = 0x5F,
    Floor11F = 0x60,
    FloorB4F = 0x61,
    // Field-move HMs (item ids $C4–$C8).
    Hm01Cut = 0xC4,
    Hm02Fly = 0xC5,
    Hm03Surf = 0xC6,
    Hm04Strength = 0xC7,
    Hm05Flash = 0xC8,
    // TMs, 0xC9 (TM01) to 0xFA (TM50), in the cartridge's own order (`add_tm`,
    // `constants/item_constants.asm`).
    Tm01MegaPunch = 0xC9,
    Tm02RazorWind = 0xCA,
    Tm03SwordsDance = 0xCB,
    Tm04Whirlwind = 0xCC,
    Tm05MegaKick = 0xCD,
    Tm06Toxic = 0xCE,
    Tm07HornDrill = 0xCF,
    Tm08BodySlam = 0xD0,
    Tm09TakeDown = 0xD1,
    Tm10DoubleEdge = 0xD2,
    Tm11Bubblebeam = 0xD3,
    Tm12WaterGun = 0xD4,
    Tm13IceBeam = 0xD5,
    /// Ice, 120 power: the Elite-Four Lance answer, found in Mansion B1F.
    Tm14Blizzard = 0xD6,
    Tm15HyperBeam = 0xD7,
    Tm16PayDay = 0xD8,
    Tm17Submission = 0xD9,
    Tm18Counter = 0xDA,
    Tm19SeismicToss = 0xDB,
    Tm20Rage = 0xDC,
    Tm21MegaDrain = 0xDD,
    Tm22Solarbeam = 0xDE,
    /// The cheapest Game Corner prize TM, so the one that proves the prize room's `GiveItem` branch.
    Tm23DragonRage = 0xDF,
    Tm24Thunderbolt = 0xE0,
    Tm25Thunder = 0xE1,
    Tm26Earthquake = 0xE2,
    Tm27Fissure = 0xE3,
    /// DIG doubles as a reusable Escape Rope out of any cave.
    Tm28Dig = 0xE4,
    Tm29Psychic = 0xE5,
    Tm30Teleport = 0xE6,
    /// The Copycat's swap for a Poké Doll.
    Tm31Mimic = 0xE7,
    Tm32DoubleTeam = 0xE8,
    Tm33Reflect = 0xE9,
    /// BIDE, the bag's most useless item: tossed to make room.
    Tm34Bide = 0xEA,
    Tm35Metronome = 0xEB,
    Tm36Selfdestruct = 0xEC,
    Tm37EggBomb = 0xED,
    Tm38FireBlast = 0xEE,
    Tm39Swift = 0xEF,
    Tm40SkullBash = 0xF0,
    Tm41Softboiled = 0xF1,
    Tm42DreamEater = 0xF2,
    Tm43SkyAttack = 0xF3,
    Tm44Rest = 0xF4,
    /// A free pickup on Route 24 and the only paralysis move the party can learn (Slowpoke is the
    /// sole compatible member).
    Tm45ThunderWave = 0xF5,
    Tm46Psywave = 0xF6,
    Tm47Explosion = 0xF7,
    Tm48RockSlide = 0xF8,
    Tm49TriAttack = 0xF9,
    Tm50Substitute = 0xFA,

}

impl ItemId {
    /// True for the items pokered's `IsKeyItem` refuses to sell or toss ("I can't put a PRICE on
    /// that!" / "That's too important!").
    pub const fn is_key_item(self) -> bool {
        matches!(self,
            Self::TownMap | Self::Bicycle | Self::Surfboard | Self::SafariBall | Self::Pokedex
            | Self::BoulderBadge | Self::CascadeBadge | Self::ThunderBadge | Self::RainbowBadge
            | Self::SoulBadge | Self::MarshBadge | Self::VolcanoBadge | Self::EarthBadge
            | Self::OldAmber | Self::DomeFossil | Self::HelixFossil | Self::SecretKey
            | Self::UnusedItem2C | Self::BikeVoucher | Self::CardKey | Self::SSTicket
            | Self::GoldTeeth | Self::CoinCase | Self::OaksParcel | Self::Itemfinder
            | Self::SilphScope | Self::PokeFlute | Self::LiftKey
            | Self::OldRod | Self::GoodRod | Self::SuperRod)
    }

    /// True for HM01–HM05 (item ids `$C4`–`$C8`), which `IsItemHM` also refuses to sell or toss —
    /// they are reusable and one-per-cartridge.
    pub const fn is_hm(self) -> bool {
        (self as u8) >= Self::Hm01Cut as u8 && (self as u8) <= Self::Hm05Flash as u8
    }

    /// An elevator's button, which is an item id only so that its floor list is an item list. It is
    /// never in a bag, has no price and no key-item flag, so anything walking the item ids to find
    /// what a player can hold skips it.
    pub const fn is_floor(self) -> bool {
        (self as u8) >= Self::FloorB2F as u8 && (self as u8) <= Self::FloorB4F as u8
    }
}
/// `GetItemName`: charmap bytes, unterminated.
pub fn name(item: ItemId) -> Vec<u8> {
    let id = item as u8;
    if id >= ItemId::Hm01Cut as u8 {
        // `GetMachineName`: an HM is numbered as if it were the TM `NUM_HMS` later.
        let (prefix, number) = if id >= ItemId::Tm01MegaPunch as u8 {
            (crate::charmap::encode("TM").unwrap(), id - ItemId::Tm01MegaPunch as u8 + 1)
        } else {
            (crate::charmap::encode("HM").unwrap(), id - ItemId::Hm01Cut as u8 + 1)
        };
        let digit = |d: u8| 0xF6 + d;
        return [prefix, vec![digit(number / 10), digit(number % 10)]].concat();
    }
    crate::charmap::encode(crate::tables::ITEM_NAMES[id as usize - 1]).expect("an item name is in the charmap")
}

/// `GetItemPrice`: the price as three BCD bytes, most significant first. `None` for an HM, which
/// `GetMachinePrice` refuses before it writes anything, leaving whatever the last item cost on
/// screen.
pub fn price(item: ItemId) -> Option<[u8; 3]> {
    use crate::tables::{ITEM_PRICES, TECHNICAL_MACHINE_PRICES};
    let id = item as u8;
    if item.is_hm() {
        return None;
    }
    if id >= ItemId::Tm01MegaPunch as u8 {
        // `GetMachinePrice`: in thousands.
        let thousands = TECHNICAL_MACHINE_PRICES[(id - ItemId::Tm01MegaPunch as u8) as usize];
        return Some([0, thousands << 4, 0]);
    }
    Some(bcd3(ITEM_PRICES[id as usize - 1]))
}

/// A price as the three BCD bytes the cartridge stores and prints, most significant first.
pub fn bcd3(value: u32) -> [u8; 3] {
    assert!(value < 1_000_000, "{value} is more than six digits");
    let digits = |n: u32| (n / 10 % 10 << 4 | n % 10) as u8;
    [digits(value / 10_000), digits(value / 100), digits(value)]
}

/// `IsKeyItem_`: an HM counts, a TM does not, and everything below them is one bit of
/// `KeyItemFlags`.
pub fn is_key_item(item: ItemId) -> bool {
    let id = item as u8;
    if id >= ItemId::Hm01Cut as u8 {
        return item.is_hm();
    }
    // The floors come after the last flag, where the cartridge reads padding and then whatever follows.
    crate::tables::KEY_ITEM_FLAGS.get(id as usize - 1).copied().unwrap_or(false)
}

/// `TMToMove` over `TechnicalMachines`: the move a machine teaches. The HMs follow the 50 TMs in
/// the table even though their item ids come first.
pub fn machine_move(item: ItemId) -> Option<PokemonMoveName> {
    const NUM_TMS: u8 = 50;
    let id = item as u8;
    let index = if item.is_hm() {
        NUM_TMS + id - ItemId::Hm01Cut as u8
    } else if id >= ItemId::Tm01MegaPunch as u8 {
        id - ItemId::Tm01MegaPunch as u8
    } else {
        return None;
    };
    PokemonMoveName::from_repr(crate::tables::TECHNICAL_MACHINES[index as usize])
}

/// `UsableItems_PartyMenu`: the items that ask which Pokémon to use them on.
pub fn opens_party_menu(item: ItemId) -> bool {
    crate::tables::USABLE_ITEMS_PARTY_MENU.contains(&(item as u8))
}

/// `UsableItems_CloseMenu`: the items whose use closes the bag.
pub fn closes_menu(item: ItemId) -> bool {
    crate::tables::USABLE_ITEMS_CLOSE_MENU.contains(&(item as u8))
}

/// `GuardDrinksList`: what the Saffron guards will take. Terminated by 0 rather than `-1`.
pub fn is_guard_drink(item: ItemId) -> bool {
    crate::tables::GUARD_DRINKS_LIST.contains(&(item as u8))
}

/// `VendingPrices`: what the Celadon machine sells, each with its own price in BCD.
pub fn vending_prices() -> Vec<(ItemId, [u8; 3])> {
    crate::tables::VENDING_PRICES.iter()
        .map(|&(item, price)| (ItemId::from_repr(item).expect("a vending item"), bcd3(price)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charmap::encode;

    /// `bcd3`: three bytes, most significant first, two digits a byte.
    fn bcd(price: [u8; 3]) -> u32 {
        price.iter().fold(0, |n, &b| n * 100 + (b >> 4) as u32 * 10 + (b & 0xF) as u32)
    }

    #[test]
    fn prices_match_the_table_and_a_machine_costs_its_nybble_of_thousands() {
        assert_eq!(price(ItemId::MasterBall).map(bcd), Some(0));
        assert_eq!(price(ItemId::UltraBall).map(bcd), Some(1200));
        assert_eq!(price(ItemId::Potion).map(bcd), Some(300));
        assert_eq!(price(ItemId::Nugget).map(bcd), Some(10000), "six digits");
        assert_eq!(price(ItemId::Tm01MegaPunch).map(bcd), Some(3000), "the first nybble");
        assert_eq!(price(ItemId::Tm02RazorWind).map(bcd), Some(2000), "the second");
        assert_eq!(price(ItemId::Hm01Cut), None, "an HM is priceless");
    }

    #[test]
    fn a_key_item_is_a_flag_and_every_hm_is_one() {
        assert!(is_key_item(ItemId::TownMap) && is_key_item(ItemId::Bicycle));
        assert!(!is_key_item(ItemId::Potion) && !is_key_item(ItemId::MasterBall));
        assert!(is_key_item(ItemId::Hm01Cut), "an HM cannot be tossed or sold");
        assert!(!is_key_item(ItemId::Tm01MegaPunch), "a TM can");
    }

    #[test]
    fn a_machine_teaches_its_move_and_the_hms_follow_the_tms() {
        use crate::move_name::PokemonMoveName::*;
        assert_eq!(machine_move(ItemId::Tm01MegaPunch), Some(MegaPunch));
        assert_eq!(machine_move(ItemId::Hm01Cut), Some(Cut));
        assert_eq!(machine_move(ItemId::Hm05Flash), Some(Flash));
        assert_eq!(machine_move(ItemId::Potion), None);
    }

    #[test]
    fn the_use_lists_and_the_vending_machine_read_back() {
        assert!(opens_party_menu(ItemId::Potion) && opens_party_menu(ItemId::RareCandy));
        assert!(!opens_party_menu(ItemId::EscapeRope));
        assert!(closes_menu(ItemId::EscapeRope) && closes_menu(ItemId::PokeFlute));
        assert!(!closes_menu(ItemId::Potion));
        assert!(is_guard_drink(ItemId::FreshWater) && !is_guard_drink(ItemId::Potion));
        let vending: Vec<_> = vending_prices().into_iter().map(|(item, p)| (item, bcd(p))).collect();
        assert_eq!(vending, [(ItemId::FreshWater, 200), (ItemId::SodaPop, 300), (ItemId::Lemonade, 350)]);
    }

    #[test]
    fn names_come_from_the_list_and_machines_are_numbered() {
        assert_eq!(name(ItemId::MasterBall), encode("MASTER BALL").unwrap());
        assert_eq!(name(ItemId::Potion), encode("POTION").unwrap());
        assert_eq!(name(ItemId::Hm01Cut), encode("HM01").unwrap());
        assert_eq!(name(ItemId::Tm50Substitute), encode("TM50").unwrap());
    }
}
