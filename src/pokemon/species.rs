use crate::pokemon::pokemon::{Pokemon, PokemonStats, PokemonType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::Display, strum_macros::FromRepr)]
#[repr(u8)]
pub enum PokemonSpecies {
    Rhydon = 0x1,
    Kangaskhan = 0x2,
    NidoranMale = 0x3,
    Clefairy = 0x4,
    Spearow = 0x5,
    Voltorb = 0x6,
    Nidoking = 0x7,
    Slowbro = 0x8,
    Ivysaur = 0x9,
    Exeggutor = 0x0A,
    Lickitung = 0x0B,
    Exeggcute = 0x0C,
    Grimer = 0x0D,
    Gengar = 0x0E,
    NidoranFemale = 0x0F,
    Nidoqueen = 0x10,
    Cubone = 0x11,
    Rhyhorn = 0x12,
    Lapras = 0x13,
    Arcanine = 0x14,
    Mew = 0x15,
    Gyarados = 0x16,
    Shellder = 0x17,
    Tentacool = 0x18,
    Gastly = 0x19,
    Scyther = 0x1A,
    Staryu = 0x1B,
    Blastoise = 0x1C,
    Pinsir = 0x1D,
    Tangela = 0x1E,
    Growlithe = 0x21,
    Onix = 0x22,
    Fearow = 0x23,
    Pidgey = 0x24,
    Slowpoke = 0x25,
    Kadabra = 0x26,
    Graveler = 0x27,
    Chansey = 0x28,
    Machoke = 0x29,
    MrMime = 0x2A,
    Hitmonlee = 0x2B,
    Hitmonchan = 0x2C,
    Arbok = 0x2D,
    Parasect = 0x2E,
    Psyduck = 0x2F,
    Drowzee = 0x30,
    Golem = 0x31,
    Magmar = 0x33,
    Electabuzz = 0x35,
    Magneton = 0x36,
    Koffing = 0x37,
    Mankey = 0x39,
    Seel = 0x3A,
    Diglett = 0x3B,
    Tauros = 0x3C,
    Farfetchd = 0x40,
    Venonat = 0x41,
    Dragonite = 0x42,
    Doduo = 0x46,
    Poliwag = 0x47,
    Jynx = 0x48,
    Moltres = 0x49,
    Articuno = 0x4A,
    Zapdos = 0x4B,
    Ditto = 0x4C,
    Meowth = 0x4D,
    Krabby = 0x4E,
    Vulpix = 0x52,
    Ninetales = 0x53,
    Pikachu = 0x54,
    Raichu = 0x55,
    Dratini = 0x58,
    Dragonair = 0x59,
    Kabuto = 0x5A,
    Kabutops = 0x5B,
    Horsea = 0x5C,
    Seadra = 0x5D,
    Sandshrew = 0x60,
    Sandslash = 0x61,
    Omanyte = 0x62,
    Omastar = 0x63,
    Jigglypuff = 0x64,
    Wigglytuff = 0x65,
    Eevee = 0x66,
    Flareon = 0x67,
    Jolteon = 0x68,
    Vaporeon = 0x69,
    Machop = 0x6A,
    Zubat = 0x6B,
    Ekans = 0x6C,
    Paras = 0x6D,
    Poliwhirl = 0x6E,
    Poliwrath = 0x6F,
    Weedle = 0x70,
    Kakuna = 0x71,
    Beedrill = 0x72,
    Dodrio = 0x74,
    Primeape = 0x75,
    Dugtrio = 0x76,
    Venomoth = 0x77,
    Dewgong = 0x78,
    Caterpie = 0x7B,
    Metapod = 0x7C,
    Butterfree = 0x7D,
    Machamp = 0x7E,
    Golduck = 0x80,
    Hypno = 0x81,
    Golbat = 0x82,
    Mewtwo = 0x83,
    Snorlax = 0x84,
    Magikarp = 0x85,
    Muk = 0x88,
    Kingler = 0x8A,
    Cloyster = 0x8B,
    Electrode = 0x8D,
    Clefable = 0x8E,
    Weezing = 0x8F,
    Persian = 0x90,
    Marowak = 0x91,
    Haunter = 0x93,
    Abra = 0x94,
    Alakazam = 0x95,
    Pidgeotto = 0x96,
    Pidgeot = 0x97,
    Starmie = 0x98,
    Bulbasaur = 0x99,
    Venusaur = 0x9A,
    Tentacruel = 0x9B,
    Goldeen = 0x9D,
    Seaking = 0x9E,
    Ponyta = 0xA3,
    Rapidash = 0xA4,
    Rattata = 0xA5,
    Raticate = 0xA6,
    Nidorino = 0xA7,
    Nidorina = 0xA8,
    Geodude = 0xA9,
    Porygon = 0xAA,
    Aerodactyl = 0xAB,
    Magnemite = 0xAD,
    Charmander = 0xB0,
    Squirtle = 0xB1,
    Charmeleon = 0xB2,
    Wartortle = 0xB3,
    Charizard = 0xB4,
    Oddish = 0xB9,
    Gloom = 0xBA,
    Vileplume = 0xBB,
    Bellsprout = 0xBC,
    Weepinbell = 0xBD,
    Victreebel = 0xBE,
}

impl PokemonSpecies {
    pub fn metadata(&self) -> &'static PokemonMetadata {
        use PokemonSpecies::*;
        match self {
            Rhydon => &PokemonMetadata::RHYDON,
            Kangaskhan => &PokemonMetadata::KANGASKHAN,
            NidoranMale => &PokemonMetadata::NIDORAN_MALE,
            Clefairy => &PokemonMetadata::CLEFAIRY,
            Spearow => &PokemonMetadata::SPEAROW,
            Voltorb => &PokemonMetadata::VOLTORB,
            Nidoking => &PokemonMetadata::NIDOKING,
            Slowbro => &PokemonMetadata::SLOWBRO,
            Ivysaur => &PokemonMetadata::IVYSAUR,
            Exeggutor => &PokemonMetadata::EXEGGUTOR,
            Lickitung => &PokemonMetadata::LICKITUNG,
            Exeggcute => &PokemonMetadata::EXEGGCUTE,
            Grimer => &PokemonMetadata::GRIMER,
            Gengar => &PokemonMetadata::GENGAR,
            NidoranFemale => &PokemonMetadata::NIDORAN_FEMALE,
            Nidoqueen => &PokemonMetadata::NIDOQUEEN,
            Cubone => &PokemonMetadata::CUBONE,
            Rhyhorn => &PokemonMetadata::RHYHORN,
            Lapras => &PokemonMetadata::LAPRAS,
            Arcanine => &PokemonMetadata::ARCANINE,
            Mew => &PokemonMetadata::MEW,
            Gyarados => &PokemonMetadata::GYARADOS,
            Shellder => &PokemonMetadata::SHELLDER,
            Tentacool => &PokemonMetadata::TENTACOOL,
            Gastly => &PokemonMetadata::GASTLY,
            Scyther => &PokemonMetadata::SCYTHER,
            Staryu => &PokemonMetadata::STARYU,
            Blastoise => &PokemonMetadata::BLASTOISE,
            Pinsir => &PokemonMetadata::PINSIR,
            Tangela => &PokemonMetadata::TANGELA,
            Growlithe => &PokemonMetadata::GROWLITHE,
            Onix => &PokemonMetadata::ONIX,
            Fearow => &PokemonMetadata::FEAROW,
            Pidgey => &PokemonMetadata::PIDGEY,
            Slowpoke => &PokemonMetadata::SLOWPOKE,
            Kadabra => &PokemonMetadata::KADABRA,
            Graveler => &PokemonMetadata::GRAVELER,
            Chansey => &PokemonMetadata::CHANSEY,
            Machoke => &PokemonMetadata::MACHOKE,
            MrMime => &PokemonMetadata::MR_MIME,
            Hitmonlee => &PokemonMetadata::HITMONLEE,
            Hitmonchan => &PokemonMetadata::HITMONCHAN,
            Arbok => &PokemonMetadata::ARBOK,
            Parasect => &PokemonMetadata::PARASECT,
            Psyduck => &PokemonMetadata::PSYDUCK,
            Drowzee => &PokemonMetadata::DROWZEE,
            Golem => &PokemonMetadata::GOLEM,
            Magmar => &PokemonMetadata::MAGMAR,
            Electabuzz => &PokemonMetadata::ELECTABUZZ,
            Magneton => &PokemonMetadata::MAGNETON,
            Koffing => &PokemonMetadata::KOFFING,
            Mankey => &PokemonMetadata::MANKEY,
            Seel => &PokemonMetadata::SEEL,
            Diglett => &PokemonMetadata::DIGLETT,
            Tauros => &PokemonMetadata::TAUROS,
            Farfetchd => &PokemonMetadata::FARFETCHD,
            Venonat => &PokemonMetadata::VENONAT,
            Dragonite => &PokemonMetadata::DRAGONITE,
            Doduo => &PokemonMetadata::DODUO,
            Poliwag => &PokemonMetadata::POLIWAG,
            Jynx => &PokemonMetadata::JYNX,
            Moltres => &PokemonMetadata::MOLTRES,
            Articuno => &PokemonMetadata::ARTICUNO,
            Zapdos => &PokemonMetadata::ZAPDOS,
            Ditto => &PokemonMetadata::DITTO,
            Meowth => &PokemonMetadata::MEOWTH,
            Krabby => &PokemonMetadata::KRABBY,
            Vulpix => &PokemonMetadata::VULPIX,
            Ninetales => &PokemonMetadata::NINETALES,
            Pikachu => &PokemonMetadata::PIKACHU,
            Raichu => &PokemonMetadata::RAICHU,
            Dratini => &PokemonMetadata::DRATINI,
            Dragonair => &PokemonMetadata::DRAGONAIR,
            Kabuto => &PokemonMetadata::KABUTO,
            Kabutops => &PokemonMetadata::KABUTOPS,
            Horsea => &PokemonMetadata::HORSEA,
            Seadra => &PokemonMetadata::SEADRA,
            Sandshrew => &PokemonMetadata::SANDSHREW,
            Sandslash => &PokemonMetadata::SANDSLASH,
            Omanyte => &PokemonMetadata::OMANYTE,
            Omastar => &PokemonMetadata::OMASTAR,
            Jigglypuff => &PokemonMetadata::JIGGLYPUFF,
            Wigglytuff => &PokemonMetadata::WIGGLYTUFF,
            Eevee => &PokemonMetadata::EEVEE,
            Flareon => &PokemonMetadata::FLAREON,
            Jolteon => &PokemonMetadata::JOLTEON,
            Vaporeon => &PokemonMetadata::VAPOREON,
            Machop => &PokemonMetadata::MACHOP,
            Zubat => &PokemonMetadata::ZUBAT,
            Ekans => &PokemonMetadata::EKANS,
            Paras => &PokemonMetadata::PARAS,
            Poliwhirl => &PokemonMetadata::POLIWHIRL,
            Poliwrath => &PokemonMetadata::POLIWRATH,
            Weedle => &PokemonMetadata::WEEDLE,
            Kakuna => &PokemonMetadata::KAKUNA,
            Beedrill => &PokemonMetadata::BEEDRILL,
            Dodrio => &PokemonMetadata::DODRIO,
            Primeape => &PokemonMetadata::PRIMEAPE,
            Dugtrio => &PokemonMetadata::DUGTRIO,
            Venomoth => &PokemonMetadata::VENOMOTH,
            Dewgong => &PokemonMetadata::DEWGONG,
            Caterpie => &PokemonMetadata::CATERPIE,
            Metapod => &PokemonMetadata::METAPOD,
            Butterfree => &PokemonMetadata::BUTTERFREE,
            Machamp => &PokemonMetadata::MACHAMP,
            Golduck => &PokemonMetadata::GOLDUCK,
            Hypno => &PokemonMetadata::HYPNO,
            Golbat => &PokemonMetadata::GOLBAT,
            Mewtwo => &PokemonMetadata::MEWTWO,
            Snorlax => &PokemonMetadata::SNORLAX,
            Magikarp => &PokemonMetadata::MAGIKARP,
            Muk => &PokemonMetadata::MUK,
            Kingler => &PokemonMetadata::KINGLER,
            Cloyster => &PokemonMetadata::CLOYSTER,
            Electrode => &PokemonMetadata::ELECTRODE,
            Clefable => &PokemonMetadata::CLEFABLE,
            Weezing => &PokemonMetadata::WEEZING,
            Persian => &PokemonMetadata::PERSIAN,
            Marowak => &PokemonMetadata::MAROWAK,
            Haunter => &PokemonMetadata::HAUNTER,
            Abra => &PokemonMetadata::ABRA,
            Alakazam => &PokemonMetadata::ALAKAZAM,
            Pidgeotto => &PokemonMetadata::PIDGEOTTO,
            Pidgeot => &PokemonMetadata::PIDGEOT,
            Starmie => &PokemonMetadata::STARMIE,
            Bulbasaur => &PokemonMetadata::BULBASAUR,
            Venusaur => &PokemonMetadata::VENUSAUR,
            Tentacruel => &PokemonMetadata::TENTACRUEL,
            Goldeen => &PokemonMetadata::GOLDEEN,
            Seaking => &PokemonMetadata::SEAKING,
            Ponyta => &PokemonMetadata::PONYTA,
            Rapidash => &PokemonMetadata::RAPIDASH,
            Rattata => &PokemonMetadata::RATTATA,
            Raticate => &PokemonMetadata::RATICATE,
            Nidorino => &PokemonMetadata::NIDORINO,
            Nidorina => &PokemonMetadata::NIDORINA,
            Geodude => &PokemonMetadata::GEODUDE,
            Porygon => &PokemonMetadata::PORYGON,
            Aerodactyl => &PokemonMetadata::AERODACTYL,
            Magnemite => &PokemonMetadata::MAGNEMITE,
            Charmander => &PokemonMetadata::CHARMANDER,
            Squirtle => &PokemonMetadata::SQUIRTLE,
            Charmeleon => &PokemonMetadata::CHARMELEON,
            Wartortle => &PokemonMetadata::WARTORTLE,
            Charizard => &PokemonMetadata::CHARIZARD,
            Oddish => &PokemonMetadata::ODDISH,
            Gloom => &PokemonMetadata::GLOOM,
            Vileplume => &PokemonMetadata::VILEPLUME,
            Bellsprout => &PokemonMetadata::BELLSPROUT,
            Weepinbell => &PokemonMetadata::WEEPINBELL,
            Victreebel => &PokemonMetadata::VICTREEBEL,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExperienceGroup {
    Fast,
    MediumFast,
    MediumSlow,
    Slow,
}

impl ExperienceGroup {
    pub fn level_from_experience(&self, experience: u32) -> u8 {
        self.experience_distribution()
            .binary_search(&experience)
            .map(|exact| exact + 1)
            .unwrap_or_else(|level| level) as u8
    }

    pub fn experience_distribution(&self) -> &[u32; 100] {
        use ExperienceGroup::*;
        match self {
            Fast => &[0, 6, 21, 51, 100, 172, 274, 409, 583, 800, 1064, 1382, 1757, 2195, 2700, 3276, 3930, 4665, 5487, 6400, 7408, 8518, 9733, 11059, 12500, 14060, 15746, 17561, 19511, 21600, 23832, 26214, 28749, 31443, 34300, 37324, 40522, 43897, 47455, 51200, 55136, 59270, 63605, 68147, 72900, 77868, 83058, 88473, 94119, 100000, 106120, 112486, 119101, 125971, 133100, 140492, 148154, 156089, 164303, 172800, 181584, 190662, 200037, 209715, 219700, 229996, 240610, 251545, 262807, 274400, 286328, 298598, 311213, 324179, 337500, 351180, 365226, 379641, 394431, 409600, 425152, 441094, 457429, 474163, 491300, 508844, 526802, 545177, 563975, 583200, 602856, 622950, 643485, 664467, 685900, 707788, 730138, 752953, 776239, 800000],
            MediumFast => &[0, 8, 27, 64, 125, 216, 343, 512, 729, 1000, 1331, 1728, 2197, 2744, 3375, 4096, 4913, 5832, 6859, 8000, 9261, 10648, 12167, 13824, 15625, 17576, 19683, 21952, 24389, 27000, 29791, 32768, 35937, 39304, 42875, 46656, 50653, 54872, 59319, 64000, 68921, 74088, 79507, 85184, 91125, 97336, 103823, 110592, 117649, 125000, 132651, 140608, 148877, 157464, 166375, 175616, 185193, 195112, 205379, 216000, 226981, 238328, 250047, 262144, 274625, 287496, 300763, 314432, 328509, 343000, 357911, 373248, 389017, 405224, 421875, 438976, 456533, 474552, 493039, 512000, 531441, 551368, 571787, 592704, 614125, 636056, 658503, 681472, 704969, 729000, 753571, 778688, 804357, 830584, 857375, 884736, 912673, 941192, 970299, 1000000],
            MediumSlow => &[0, 9, 57, 96, 135, 179, 236, 314, 419, 560, 742, 973, 1261, 1612, 2035, 2535, 3120, 3798, 4575, 5460, 6458, 7577, 8825, 10208, 11735, 13411, 15244, 17242, 19411, 21760, 24294, 27021, 29949, 33084, 36435, 40007, 43808, 47846, 52127, 56660, 61450, 66505, 71833, 77440, 83335, 89523, 96012, 102810, 109923, 117360, 125126, 133229, 141677, 150476, 159635, 169159, 179056, 189334, 199999, 211060, 222522, 234393, 246681, 259392, 272535, 286115, 300140, 314618, 329555, 344960, 360838, 377197, 394045, 411388, 429235, 447591, 466464, 485862, 505791, 526260, 547274, 568841, 590969, 613664, 636935, 660787, 685228, 710266, 735907, 762160, 789030, 816525, 844653, 873420, 902835, 932903, 963632, 995030, 1027103, 1059860],
            Slow => &[0, 10, 33, 80, 156, 270, 428, 640, 911, 1250, 1663, 2160, 2746, 3430, 4218, 5120, 6141, 7290, 8573, 10000, 11576, 13310, 15208, 17280, 19531, 21970, 24603, 27440, 30486, 33750, 37238, 40960, 44921, 49130, 53593, 58320, 63316, 68590, 74148, 80000, 86151, 92610, 99383, 106480, 113906, 121670, 129778, 138240, 147061, 156250, 165813, 175760, 186096, 196830, 207968, 219520, 231491, 243890, 256723, 270000, 283726, 297910, 312558, 327680, 343281, 359370, 375953, 393040, 410636, 428750, 447388, 466560, 486271, 506530, 527343, 548720, 570666, 593190, 616298, 640000, 664301, 689210, 714733, 740880, 767656, 795070, 823128, 851840, 881211, 911250, 941963, 973360, 1005446, 1038230, 1071718, 1105920, 1140841, 1176490, 1212873, 1250000],
        }
    }
    
    pub fn experience_for_level(&self, level: u8) -> u32 {
        self.experience_distribution()[level as usize - 1]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PokemonMetadata {
    pub name: &'static str,
    pub pokedex_number: u8,
    pub base_stats: PokemonStats,
    pub experience_group: ExperienceGroup,
    pub type1: PokemonType,
    pub type2: Option<PokemonType>,
}

impl PokemonMetadata {
    pub const fn new(name: &'static str, pokedex_number: u8, hp: u16, attack: u16, defense: u16, speed: u16, special: u16, experience_group: ExperienceGroup, type1: PokemonType, type2: Option<PokemonType>) -> Self {
        Self { name, pokedex_number, base_stats: PokemonStats::new(hp, attack, defense, speed, special), experience_group, type1, type2 }
    }

    pub const RHYDON: Self = Self::new("Rhydon",112, 105, 130, 120, 40, 45, ExperienceGroup::Slow, PokemonType::Ground, Some(PokemonType::Rock));
    pub const KANGASKHAN: Self = Self::new("Kangaskhan",115, 105, 95, 80, 90, 40, ExperienceGroup::MediumFast, PokemonType::Normal, None);
    pub const NIDORAN_MALE: Self = Self::new("NidoranMale",32, 46, 57, 40, 50, 40, ExperienceGroup::MediumSlow, PokemonType::Poison, None);
    pub const CLEFAIRY: Self = Self::new("Clefairy",35, 70, 45, 48, 35, 60, ExperienceGroup::Fast, PokemonType::Normal, None);
    pub const SPEAROW: Self = Self::new("Spearow",21, 40, 60, 30, 70, 31, ExperienceGroup::MediumFast, PokemonType::Normal, Some(PokemonType::Flying));
    pub const VOLTORB: Self = Self::new("Voltorb",100, 40, 30, 50, 100, 55, ExperienceGroup::MediumFast, PokemonType::Electric, None);
    pub const NIDOKING: Self = Self::new("Nidoking",34, 81, 92, 77, 85, 75, ExperienceGroup::MediumSlow, PokemonType::Poison, Some(PokemonType::Ground));
    pub const SLOWBRO: Self = Self::new("Slowbro",80, 95, 75, 110, 30, 80, ExperienceGroup::MediumFast, PokemonType::Water, Some(PokemonType::Psychic));
    pub const IVYSAUR: Self = Self::new("Ivysaur",2, 60, 62, 63, 60, 80, ExperienceGroup::MediumSlow, PokemonType::Grass, Some(PokemonType::Poison));
    pub const EXEGGUTOR: Self = Self::new("Exeggutor",103, 95, 95, 85, 55, 125, ExperienceGroup::Slow, PokemonType::Grass, Some(PokemonType::Psychic));
    pub const LICKITUNG: Self = Self::new("Lickitung",108, 90, 55, 75, 30, 60, ExperienceGroup::MediumFast, PokemonType::Normal, None);
    pub const EXEGGCUTE: Self = Self::new("Exeggcute",102, 60, 40, 80, 40, 60, ExperienceGroup::Slow, PokemonType::Grass, Some(PokemonType::Psychic));
    pub const GRIMER: Self = Self::new("Grimer",88, 80, 80, 50, 25, 40, ExperienceGroup::MediumFast, PokemonType::Poison, None);
    pub const GENGAR: Self = Self::new("Gengar",94, 60, 65, 60, 110, 130, ExperienceGroup::MediumSlow, PokemonType::Ghost, Some(PokemonType::Poison));
    pub const NIDORAN_FEMALE: Self = Self::new("NidoranFemale",29, 55, 47, 52, 41, 40, ExperienceGroup::MediumSlow, PokemonType::Poison, None);
    pub const NIDOQUEEN: Self = Self::new("Nidoqueen",31, 90, 82, 87, 76, 75, ExperienceGroup::MediumSlow, PokemonType::Poison, Some(PokemonType::Ground));
    pub const CUBONE: Self = Self::new("Cubone",104, 50, 50, 95, 35, 40, ExperienceGroup::MediumFast, PokemonType::Ground, None);
    pub const RHYHORN: Self = Self::new("Rhyhorn",111, 80, 85, 95, 25, 30, ExperienceGroup::Slow, PokemonType::Ground, Some(PokemonType::Rock));
    pub const LAPRAS: Self = Self::new("Lapras",131, 130, 85, 80, 60, 95, ExperienceGroup::Slow, PokemonType::Water, Some(PokemonType::Ice));
    pub const ARCANINE: Self = Self::new("Arcanine",59, 90, 110, 80, 95, 80, ExperienceGroup::Slow, PokemonType::Fire, None);
    pub const MEW: Self = Self::new("Mew",151, 100, 100, 100, 100, 100, ExperienceGroup::MediumSlow, PokemonType::Psychic, None);
    pub const GYARADOS: Self = Self::new("Gyarados",130, 95, 125, 79, 81, 100, ExperienceGroup::Slow, PokemonType::Water, Some(PokemonType::Flying));
    pub const SHELLDER: Self = Self::new("Shellder",90, 30, 65, 100, 40, 45, ExperienceGroup::Slow, PokemonType::Water, None);
    pub const TENTACOOL: Self = Self::new("Tentacool",72, 40, 40, 35, 70, 100, ExperienceGroup::Slow, PokemonType::Water, Some(PokemonType::Poison));
    pub const GASTLY: Self = Self::new("Gastly",92, 30, 35, 30, 80, 100, ExperienceGroup::MediumSlow, PokemonType::Ghost, Some(PokemonType::Poison));
    pub const SCYTHER: Self = Self::new("Scyther",123, 70, 110, 80, 105, 55, ExperienceGroup::MediumFast, PokemonType::Bug, Some(PokemonType::Flying));
    pub const STARYU: Self = Self::new("Staryu",120, 30, 45, 55, 85, 70, ExperienceGroup::Slow, PokemonType::Water, None);
    pub const BLASTOISE: Self = Self::new("Blastoise",9, 79, 83, 100, 78, 85, ExperienceGroup::MediumSlow, PokemonType::Water, None);
    pub const PINSIR: Self = Self::new("Pinsir",127, 65, 125, 100, 85, 55, ExperienceGroup::Slow, PokemonType::Bug, None);
    pub const TANGELA: Self = Self::new("Tangela",114, 65, 55, 115, 60, 100, ExperienceGroup::MediumFast, PokemonType::Grass, None);
    pub const GROWLITHE: Self = Self::new("Growlithe",58, 55, 70, 45, 60, 50, ExperienceGroup::Slow, PokemonType::Fire, None);
    pub const ONIX: Self = Self::new("Onix",95, 35, 45, 160, 70, 30, ExperienceGroup::MediumFast, PokemonType::Rock, Some(PokemonType::Ground));
    pub const FEAROW: Self = Self::new("Fearow",22, 65, 90, 65, 100, 61, ExperienceGroup::MediumFast, PokemonType::Normal, Some(PokemonType::Flying));
    pub const PIDGEY: Self = Self::new("Pidgey",16, 40, 45, 40, 56, 35, ExperienceGroup::MediumSlow, PokemonType::Normal, Some(PokemonType::Flying));
    pub const SLOWPOKE: Self = Self::new("Slowpoke",79, 90, 65, 65, 15, 40, ExperienceGroup::MediumFast, PokemonType::Water, Some(PokemonType::Psychic));
    pub const KADABRA: Self = Self::new("Kadabra",64, 40, 35, 30, 105, 120, ExperienceGroup::MediumSlow, PokemonType::Psychic, None);
    pub const GRAVELER: Self = Self::new("Graveler",75, 55, 95, 115, 35, 45, ExperienceGroup::MediumSlow, PokemonType::Rock, Some(PokemonType::Ground));
    pub const CHANSEY: Self = Self::new("Chansey",113, 250, 5, 5, 50, 105, ExperienceGroup::Fast, PokemonType::Normal, None);
    pub const MACHOKE: Self = Self::new("Machoke",67, 80, 100, 70, 45, 50, ExperienceGroup::MediumSlow, PokemonType::Fighting, None);
    pub const MR_MIME: Self = Self::new("MrMime",122, 40, 45, 65, 90, 100, ExperienceGroup::MediumFast, PokemonType::Psychic, Some(PokemonType::Normal));
    pub const HITMONLEE: Self = Self::new("Hitmonlee",106, 50, 120, 53, 87, 35, ExperienceGroup::MediumFast, PokemonType::Fighting, None);
    pub const HITMONCHAN: Self = Self::new("Hitmonchan",107, 50, 105, 79, 76, 35, ExperienceGroup::MediumFast, PokemonType::Fighting, None);
    pub const ARBOK: Self = Self::new("Arbok",24, 60, 85, 69, 80, 65, ExperienceGroup::MediumFast, PokemonType::Poison, None);
    pub const PARASECT: Self = Self::new("Parasect",47, 60, 95, 80, 30, 80, ExperienceGroup::MediumFast, PokemonType::Bug, Some(PokemonType::Grass));
    pub const PSYDUCK: Self = Self::new("Psyduck",54, 50, 52, 48, 55, 50, ExperienceGroup::MediumFast, PokemonType::Water, None);
    pub const DROWZEE: Self = Self::new("Drowzee",96, 60, 48, 45, 42, 90, ExperienceGroup::MediumFast, PokemonType::Psychic, None);
    pub const GOLEM: Self = Self::new("Golem",76, 80, 110, 130, 45, 55, ExperienceGroup::MediumSlow, PokemonType::Rock, Some(PokemonType::Ground));
    pub const MAGMAR: Self = Self::new("Magmar",126, 65, 95, 57, 93, 85, ExperienceGroup::MediumFast, PokemonType::Fire, None);
    pub const ELECTABUZZ: Self = Self::new("Electabuzz",125, 65, 83, 57, 105, 85, ExperienceGroup::MediumFast, PokemonType::Electric, None);
    pub const MAGNETON: Self = Self::new("Magneton",82, 50, 60, 95, 70, 120, ExperienceGroup::MediumFast, PokemonType::Electric, None);
    pub const KOFFING: Self = Self::new("Koffing",109, 40, 65, 95, 35, 60, ExperienceGroup::MediumFast, PokemonType::Poison, None);
    pub const MANKEY: Self = Self::new("Mankey",56, 40, 80, 35, 70, 35, ExperienceGroup::MediumFast, PokemonType::Fighting, None);
    pub const SEEL: Self = Self::new("Seel",86, 65, 45, 55, 45, 70, ExperienceGroup::MediumFast, PokemonType::Water, None);
    pub const DIGLETT: Self = Self::new("Diglett",50, 10, 55, 25, 95, 45, ExperienceGroup::MediumFast, PokemonType::Ground, None);
    pub const TAUROS: Self = Self::new("Tauros",128, 75, 100, 95, 110, 70, ExperienceGroup::Slow, PokemonType::Normal, None);
    pub const FARFETCHD: Self = Self::new("Farfetchd",83, 52, 65, 55, 60, 58, ExperienceGroup::MediumFast, PokemonType::Normal, Some(PokemonType::Flying));
    pub const VENONAT: Self = Self::new("Venonat",48, 60, 55, 50, 45, 40, ExperienceGroup::MediumFast, PokemonType::Bug, Some(PokemonType::Poison));
    pub const DRAGONITE: Self = Self::new("Dragonite",149, 91, 134, 95, 80, 100, ExperienceGroup::Slow, PokemonType::Dragon, Some(PokemonType::Flying));
    pub const DODUO: Self = Self::new("Doduo",84, 35, 85, 45, 75, 35, ExperienceGroup::MediumFast, PokemonType::Normal, Some(PokemonType::Flying));
    pub const POLIWAG: Self = Self::new("Poliwag",60, 40, 50, 40, 90, 40, ExperienceGroup::MediumSlow, PokemonType::Water, None);
    pub const JYNX: Self = Self::new("Jynx",124, 65, 50, 35, 95, 95, ExperienceGroup::MediumFast, PokemonType::Ice, Some(PokemonType::Psychic));
    pub const MOLTRES: Self = Self::new("Moltres",146, 90, 100, 90, 90, 125, ExperienceGroup::Slow, PokemonType::Fire, Some(PokemonType::Flying));
    pub const ARTICUNO: Self = Self::new("Articuno",144, 90, 85, 100, 85, 125, ExperienceGroup::Slow, PokemonType::Ice, Some(PokemonType::Flying));
    pub const ZAPDOS: Self = Self::new("Zapdos",145, 90, 90, 85, 100, 125, ExperienceGroup::Slow, PokemonType::Electric, Some(PokemonType::Flying));
    pub const DITTO: Self = Self::new("Ditto",132, 48, 48, 48, 48, 48, ExperienceGroup::MediumFast, PokemonType::Normal, None);
    pub const MEOWTH: Self = Self::new("Meowth",52, 40, 45, 35, 90, 40, ExperienceGroup::MediumFast, PokemonType::Normal, None);
    pub const KRABBY: Self = Self::new("Krabby",98, 30, 105, 90, 50, 25, ExperienceGroup::MediumFast, PokemonType::Water, None);
    pub const VULPIX: Self = Self::new("Vulpix",37, 38, 41, 40, 65, 65, ExperienceGroup::MediumFast, PokemonType::Fire, None);
    pub const NINETALES: Self = Self::new("Ninetales",38, 73, 76, 75, 100, 100, ExperienceGroup::MediumFast, PokemonType::Fire, None);
    pub const PIKACHU: Self = Self::new("Pikachu",25, 35, 55, 30, 90, 50, ExperienceGroup::MediumFast, PokemonType::Electric, None);
    pub const RAICHU: Self = Self::new("Raichu",26, 60, 90, 55, 100, 90, ExperienceGroup::MediumFast, PokemonType::Electric, None);
    pub const DRATINI: Self = Self::new("Dratini",147, 41, 64, 45, 50, 50, ExperienceGroup::Slow, PokemonType::Dragon, None);
    pub const DRAGONAIR: Self = Self::new("Dragonair",148, 61, 84, 65, 70, 70, ExperienceGroup::Slow, PokemonType::Dragon, None);
    pub const KABUTO: Self = Self::new("Kabuto",140, 30, 80, 90, 55, 45, ExperienceGroup::MediumFast, PokemonType::Rock, Some(PokemonType::Water));
    pub const KABUTOPS: Self = Self::new("Kabutops",141, 60, 115, 105, 80, 70, ExperienceGroup::MediumFast, PokemonType::Rock, Some(PokemonType::Water));
    pub const HORSEA: Self = Self::new("Horsea",116, 30, 40, 70, 60, 70, ExperienceGroup::MediumFast, PokemonType::Water, None);
    pub const SEADRA: Self = Self::new("Seadra",117, 55, 65, 95, 85, 95, ExperienceGroup::MediumFast, PokemonType::Water, None);
    pub const SANDSHREW: Self = Self::new("Sandshrew",27, 50, 75, 85, 40, 30, ExperienceGroup::MediumFast, PokemonType::Ground, None);
    pub const SANDSLASH: Self = Self::new("Sandslash",28, 75, 100, 110, 65, 55, ExperienceGroup::MediumFast, PokemonType::Ground, None);
    pub const OMANYTE: Self = Self::new("Omanyte",138, 35, 40, 100, 35, 90, ExperienceGroup::MediumFast, PokemonType::Rock, Some(PokemonType::Water));
    pub const OMASTAR: Self = Self::new("Omastar",139, 70, 60, 125, 55, 115, ExperienceGroup::MediumFast, PokemonType::Rock, Some(PokemonType::Water));
    pub const JIGGLYPUFF: Self = Self::new("Jigglypuff",39, 115, 45, 20, 20, 25, ExperienceGroup::Fast, PokemonType::Normal, None);
    pub const WIGGLYTUFF: Self = Self::new("Wigglytuff",40, 140, 70, 45, 45, 50, ExperienceGroup::Fast, PokemonType::Normal, None);
    pub const EEVEE: Self = Self::new("Eevee",133, 55, 55, 50, 55, 65, ExperienceGroup::MediumFast, PokemonType::Normal, None);
    pub const FLAREON: Self = Self::new("Flareon",136, 65, 130, 60, 65, 110, ExperienceGroup::MediumFast, PokemonType::Fire, None);
    pub const JOLTEON: Self = Self::new("Jolteon",135, 65, 65, 60, 130, 110, ExperienceGroup::MediumFast, PokemonType::Electric, None);
    pub const VAPOREON: Self = Self::new("Vaporeon",134, 130, 65, 60, 65, 110, ExperienceGroup::MediumFast, PokemonType::Water, None);
    pub const MACHOP: Self = Self::new("Machop",66, 70, 80, 50, 35, 35, ExperienceGroup::MediumSlow, PokemonType::Fighting, None);
    pub const ZUBAT: Self = Self::new("Zubat",41, 40, 45, 35, 55, 40, ExperienceGroup::MediumFast, PokemonType::Poison, Some(PokemonType::Flying));
    pub const EKANS: Self = Self::new("Ekans",23, 35, 60, 44, 55, 40, ExperienceGroup::MediumFast, PokemonType::Poison, None);
    pub const PARAS: Self = Self::new("Paras",46, 35, 70, 55, 25, 55, ExperienceGroup::MediumFast, PokemonType::Bug, Some(PokemonType::Grass));
    pub const POLIWHIRL: Self = Self::new("Poliwhirl",61, 65, 65, 65, 90, 50, ExperienceGroup::MediumSlow, PokemonType::Water, None);
    pub const POLIWRATH: Self = Self::new("Poliwrath",62, 90, 85, 95, 70, 70, ExperienceGroup::MediumSlow, PokemonType::Water, Some(PokemonType::Fighting));
    pub const WEEDLE: Self = Self::new("Weedle",13, 40, 35, 30, 50, 20, ExperienceGroup::MediumFast, PokemonType::Bug, Some(PokemonType::Poison));
    pub const KAKUNA: Self = Self::new("Kakuna",14, 45, 25, 50, 35, 25, ExperienceGroup::MediumFast, PokemonType::Bug, Some(PokemonType::Poison));
    pub const BEEDRILL: Self = Self::new("Beedrill",15, 65, 80, 40, 75, 45, ExperienceGroup::MediumFast, PokemonType::Bug, Some(PokemonType::Poison));
    pub const DODRIO: Self = Self::new("Dodrio",85, 60, 110, 70, 100, 60, ExperienceGroup::MediumFast, PokemonType::Normal, Some(PokemonType::Flying));
    pub const PRIMEAPE: Self = Self::new("Primeape",57, 65, 105, 60, 95, 60, ExperienceGroup::MediumFast, PokemonType::Fighting, None);
    pub const DUGTRIO: Self = Self::new("Dugtrio",51, 35, 80, 50, 120, 70, ExperienceGroup::MediumFast, PokemonType::Ground, None);
    pub const VENOMOTH: Self = Self::new("Venomoth",49, 70, 65, 60, 90, 90, ExperienceGroup::MediumFast, PokemonType::Bug, Some(PokemonType::Poison));
    pub const DEWGONG: Self = Self::new("Dewgong",87, 90, 70, 80, 70, 95, ExperienceGroup::MediumFast, PokemonType::Water, Some(PokemonType::Ice));
    pub const CATERPIE: Self = Self::new("Caterpie",10, 45, 30, 35, 45, 20, ExperienceGroup::MediumFast, PokemonType::Bug, None);
    pub const METAPOD: Self = Self::new("Metapod",11, 50, 20, 55, 30, 25, ExperienceGroup::MediumFast, PokemonType::Bug, None);
    pub const BUTTERFREE: Self = Self::new("Butterfree",12, 60, 45, 50, 70, 80, ExperienceGroup::MediumFast, PokemonType::Bug, Some(PokemonType::Flying));
    pub const MACHAMP: Self = Self::new("Machamp",68, 90, 130, 80, 55, 65, ExperienceGroup::MediumSlow, PokemonType::Fighting, None);
    pub const GOLDUCK: Self = Self::new("Golduck",55, 80, 82, 78, 85, 80, ExperienceGroup::MediumFast, PokemonType::Water, None);
    pub const HYPNO: Self = Self::new("Hypno",97, 85, 73, 70, 67, 115, ExperienceGroup::MediumFast, PokemonType::Psychic, None);
    pub const GOLBAT: Self = Self::new("Golbat",42, 75, 80, 70, 90, 75, ExperienceGroup::MediumFast, PokemonType::Poison, Some(PokemonType::Flying));
    pub const MEWTWO: Self = Self::new("Mewtwo",150, 106, 110, 90, 130, 154, ExperienceGroup::Slow, PokemonType::Psychic, None);
    pub const SNORLAX: Self = Self::new("Snorlax",143, 160, 110, 65, 30, 65, ExperienceGroup::Slow, PokemonType::Normal, None);
    pub const MAGIKARP: Self = Self::new("Magikarp",129, 20, 10, 55, 80, 20, ExperienceGroup::Slow, PokemonType::Water, None);
    pub const MUK: Self = Self::new("Muk",89, 105, 105, 75, 50, 65, ExperienceGroup::MediumFast, PokemonType::Poison, None);
    pub const KINGLER: Self = Self::new("Kingler",99, 55, 130, 115, 75, 50, ExperienceGroup::MediumFast, PokemonType::Water, None);
    pub const CLOYSTER: Self = Self::new("Cloyster",91, 50, 95, 180, 70, 85, ExperienceGroup::Slow, PokemonType::Water, Some(PokemonType::Ice));
    pub const ELECTRODE: Self = Self::new("Electrode",101, 60, 50, 70, 140, 80, ExperienceGroup::MediumFast, PokemonType::Electric, None);
    pub const CLEFABLE: Self = Self::new("Clefable",36, 95, 70, 73, 60, 85, ExperienceGroup::Fast, PokemonType::Normal, None);
    pub const WEEZING: Self = Self::new("Weezing",110, 65, 90, 120, 60, 85, ExperienceGroup::MediumFast, PokemonType::Poison, None);
    pub const PERSIAN: Self = Self::new("Persian",53, 65, 70, 60, 115, 65, ExperienceGroup::MediumFast, PokemonType::Normal, None);
    pub const MAROWAK: Self = Self::new("Marowak",105, 60, 80, 110, 45, 50, ExperienceGroup::MediumFast, PokemonType::Ground, None);
    pub const HAUNTER: Self = Self::new("Haunter",93, 45, 50, 45, 95, 115, ExperienceGroup::MediumSlow, PokemonType::Ghost, Some(PokemonType::Poison));
    pub const ABRA: Self = Self::new("Abra",63, 25, 20, 15, 90, 105, ExperienceGroup::MediumSlow, PokemonType::Psychic, None);
    pub const ALAKAZAM: Self = Self::new("Alakazam",65, 55, 50, 45, 120, 135, ExperienceGroup::MediumSlow, PokemonType::Psychic, None);
    pub const PIDGEOTTO: Self = Self::new("Pidgeotto",17, 63, 60, 55, 71, 50, ExperienceGroup::MediumSlow, PokemonType::Normal, Some(PokemonType::Flying));
    pub const PIDGEOT: Self = Self::new("Pidgeot",18, 83, 80, 75, 91, 70, ExperienceGroup::MediumSlow, PokemonType::Normal, Some(PokemonType::Flying));
    pub const STARMIE: Self = Self::new("Starmie",121, 60, 75, 85, 115, 100, ExperienceGroup::Slow, PokemonType::Water, Some(PokemonType::Psychic));
    pub const BULBASAUR: Self = Self::new("Bulbasaur",1, 45, 49, 49, 45, 65, ExperienceGroup::MediumSlow, PokemonType::Grass, Some(PokemonType::Poison));
    pub const VENUSAUR: Self = Self::new("Venusaur",3, 80, 82, 83, 80, 100, ExperienceGroup::MediumSlow, PokemonType::Grass, Some(PokemonType::Poison));
    pub const TENTACRUEL: Self = Self::new("Tentacruel",73, 80, 70, 65, 100, 120, ExperienceGroup::Slow, PokemonType::Water, Some(PokemonType::Poison));
    pub const GOLDEEN: Self = Self::new("Goldeen",118, 45, 67, 60, 63, 50, ExperienceGroup::MediumFast, PokemonType::Water, None);
    pub const SEAKING: Self = Self::new("Seaking",119, 80, 92, 65, 68, 80, ExperienceGroup::MediumFast, PokemonType::Water, None);
    pub const PONYTA: Self = Self::new("Ponyta",77, 50, 85, 55, 90, 65, ExperienceGroup::MediumFast, PokemonType::Fire, None);
    pub const RAPIDASH: Self = Self::new("Rapidash",78, 65, 100, 70, 105, 80, ExperienceGroup::MediumFast, PokemonType::Fire, None);
    pub const RATTATA: Self = Self::new("Rattata",19, 30, 56, 35, 72, 25, ExperienceGroup::MediumFast, PokemonType::Normal, None);
    pub const RATICATE: Self = Self::new("Raticate",20, 55, 81, 60, 97, 50, ExperienceGroup::MediumFast, PokemonType::Normal, None);
    pub const NIDORINO: Self = Self::new("Nidorino",33, 61, 72, 57, 65, 55, ExperienceGroup::MediumSlow, PokemonType::Poison, None);
    pub const NIDORINA: Self = Self::new("Nidorina",30, 70, 62, 67, 56, 55, ExperienceGroup::MediumSlow, PokemonType::Poison, None);
    pub const GEODUDE: Self = Self::new("Geodude",74, 40, 80, 100, 20, 30, ExperienceGroup::MediumSlow, PokemonType::Rock, Some(PokemonType::Ground));
    pub const PORYGON: Self = Self::new("Porygon",137, 65, 60, 70, 40, 75, ExperienceGroup::MediumFast, PokemonType::Normal, None);
    pub const AERODACTYL: Self = Self::new("Aerodactyl",142, 80, 105, 65, 130, 60, ExperienceGroup::Slow, PokemonType::Rock, Some(PokemonType::Flying));
    pub const MAGNEMITE: Self = Self::new("Magnemite",81, 25, 35, 70, 45, 95, ExperienceGroup::MediumFast, PokemonType::Electric, None);
    pub const CHARMANDER: Self = Self::new("Charmander",4, 39, 52, 43, 65, 50, ExperienceGroup::MediumSlow, PokemonType::Fire, None);
    pub const SQUIRTLE: Self = Self::new("Squirtle",7, 44, 48, 65, 43, 50, ExperienceGroup::MediumSlow, PokemonType::Water, None);
    pub const CHARMELEON: Self = Self::new("Charmeleon",5, 58, 64, 58, 80, 65, ExperienceGroup::MediumSlow, PokemonType::Fire, None);
    pub const WARTORTLE: Self = Self::new("Wartortle",8, 59, 63, 80, 58, 65, ExperienceGroup::MediumSlow, PokemonType::Water, None);
    pub const CHARIZARD: Self = Self::new("Charizard",6, 78, 84, 78, 100, 85, ExperienceGroup::MediumSlow, PokemonType::Fire, Some(PokemonType::Flying));
    pub const ODDISH: Self = Self::new("Oddish",43, 45, 50, 55, 30, 75, ExperienceGroup::MediumSlow, PokemonType::Grass, Some(PokemonType::Poison));
    pub const GLOOM: Self = Self::new("Gloom",44, 60, 65, 70, 40, 85, ExperienceGroup::MediumSlow, PokemonType::Grass, Some(PokemonType::Poison));
    pub const VILEPLUME: Self = Self::new("Vileplume",45, 75, 80, 85, 50, 100, ExperienceGroup::MediumSlow, PokemonType::Grass, Some(PokemonType::Poison));
    pub const BELLSPROUT: Self = Self::new("Bellsprout",69, 50, 75, 35, 40, 70, ExperienceGroup::MediumSlow, PokemonType::Grass, Some(PokemonType::Poison));
    pub const WEEPINBELL: Self = Self::new("Weepinbell",70, 65, 90, 50, 55, 85, ExperienceGroup::MediumSlow, PokemonType::Grass, Some(PokemonType::Poison));
    pub const VICTREEBEL: Self = Self::new("Victreebel",71, 80, 105, 65, 70, 100, ExperienceGroup::MediumSlow, PokemonType::Grass, Some(PokemonType::Poison));
}