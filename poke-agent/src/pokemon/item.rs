#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, strum_macros::Display, strum_macros::FromRepr)]
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
    // Field-move HMs (item ids $C4–$C8). Needed to detect/teach Cut, Fly, Surf, etc.
    Hm01Cut = 0xC4,
    Hm02Fly = 0xC5,
    Hm03Surf = 0xC6,
    Hm04Strength = 0xC7,
    Hm05Flash = 0xC8,
    // TMs, 0xC9 (TM01) to 0xFA (TM50), in the cartridge's own order
    // (`add_tm`, `constants/item_constants.asm`).
    //
    // ⚠️ **All fifty are named, and the twelve that used to be here were not enough.** `Bag` drops
    // every id `ItemId` cannot name, and `observe::bag` is what `read_bag` answers with, so an
    // unnamed TM was invisible to the model *and* uncounted against the 20-slot ceiling. The
    // deployed run of 2026-08-27 was told `slots_used: 3` over a bag holding four things, and the
    // fourth (TM01) turned up on screen with nothing having ever mentioned it; the model filed a
    // bug about the "unrelated TM34/bag menu prompt" it thought it had triggered. A bag that
    // silently fills is the worse half: a pickup into a full bag is refused in a way that reads
    // from outside exactly like a pickup that worked. There is nothing to weigh here, since
    // `from_repr` and `item_by_name` both pick a variant up for free.
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
    /// The cheapest of the three **Game Corner prize TMs** at 3300 coins, and so the one
    /// workstream F proves the prize room's `GiveItem` branch with.
    Tm23DragonRage = 0xDF,
    Tm24Thunderbolt = 0xE0,
    Tm25Thunder = 0xE1,
    Tm26Earthquake = 0xE2,
    Tm27Fissure = 0xE3,
    /// DIG doubles as a reusable Escape Rope out of any cave.
    Tm28Dig = 0xE4,
    /// Given away by the old man in `MrPsychicsHouse` for nothing at all.
    Tm29Psychic = 0xE5,
    Tm30Teleport = 0xE6,
    /// The Copycat's swap for a **Poké Doll**. Her script checks `IsItemInBag POKE_DOLL` and
    /// silently says nothing else without one, so this TM arriving is the only evidence the doll
    /// was in hand (`scripts/CopycatsHouse2F.asm:22`).
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
    /// A free pickup on Route 24 and the **only** paralysis move the party can learn (Slowpoke is
    /// the sole compatible member). Workstream D throws balls at the legendaries behind it: a
    /// status ailment is worth 12 off Rand1 in the Gen 1 catch formula, which on a catch-rate-3
    /// target is the difference between ~2 % and ~9 % per ball. See `postgame::legendaries`.
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
    ///
    /// Transcribed from `data/items/key_items.asm`, which is the authority — it is a flat bit array
    /// over every item id, and the set is not guessable from the names: the **fossils**, the
    /// **fishing rods** and the badges are all key items, while the Nugget, the Poké Doll and the
    /// vending-machine drinks are not. Only ids this enum names are listed; TMs are never key items.
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
}