use crate::pokemon::pokemon::PokemonType;
use PokemonType::*;
use MoveCategory::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PokemonMove {
    pub name: PokemonMoveName,
    pub pp: u8
}

impl PokemonMove {
    pub fn new(name: PokemonMoveName) -> Self {
        Self {
            name,
            pp: name.metadata().pp
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::Display, strum_macros::FromRepr)]
#[repr(u8)]
pub enum PokemonMoveName {
    Pound = 0x1,
    KarateChop = 0x2,
    DoubleSlap = 0x3,
    CometPunch = 0x4,
    MegaPunch = 0x5,
    PayDay = 0x6,
    FirePunch = 0x7,
    IcePunch = 0x8,
    ThunderPunch = 0x9,
    Scratch = 0x0A,
    ViceGrip = 0x0B,
    Guillotine = 0x0C,
    RazorWind = 0x0D,
    SwordsDance = 0x0E,
    Cut = 0x0F,
    Gust = 0x10,
    WingAttack = 0x11,
    Whirlwind = 0x12,
    Fly = 0x13,
    Bind = 0x14,
    Slam = 0x15,
    VineWhip = 0x16,
    Stomp = 0x17,
    DoubleKick = 0x18,
    MegaKick = 0x19,
    JumpKick = 0x1A,
    RollingKick = 0x1B,
    SandAttack = 0x1C,
    Headbutt = 0x1D,
    HornAttack = 0x1E,
    FuryAttack = 0x1F,
    HornDrill = 0x20,
    Tackle = 0x21,
    BodySlam = 0x22,
    Wrap = 0x23,
    TakeDown = 0x24,
    Thrash = 0x25,
    DoubleEdge = 0x26,
    TailWhip = 0x27,
    PoisonSting = 0x28,
    Twineedle = 0x29,
    PinMissile = 0x2A,
    Leer = 0x2B,
    Bite = 0x2C,
    Growl = 0x2D,
    Roar = 0x2E,
    Sing = 0x2F,
    Supersonic = 0x30,
    SonicBoom = 0x31,
    Disable = 0x32,
    Acid = 0x33,
    Ember = 0x34,
    Flamethrower = 0x35,
    Mist = 0x36,
    WaterGun = 0x37,
    HydroPump = 0x38,
    Surf = 0x39,
    IceBeam = 0x3A,
    Blizzard = 0x3B,
    Psybeam = 0x3C,
    BubbleBeam = 0x3D,
    AuroraBeam = 0x3E,
    HyperBeam = 0x3F,
    Peck = 0x40,
    DrillPeck = 0x41,
    Submission = 0x42,
    LowKick = 0x43,
    Counter = 0x44,
    SeismicToss = 0x45,
    Strength = 0x46,
    Absorb = 0x47,
    MegaDrain = 0x48,
    LeechSeed = 0x49,
    Growth = 0x4A,
    RazorLeaf = 0x4B,
    SolarBeam = 0x4C,
    PoisonPowder = 0x4D,
    StunSpore = 0x4E,
    SleepPowder = 0x4F,
    PetalDance = 0x50,
    StringShot = 0x51,
    DragonRage = 0x52,
    FireSpin = 0x53,
    Thundershock = 0x54,
    Thunderbolt = 0x55,
    ThunderWave = 0x56,
    Thunder = 0x57,
    RockThrow = 0x58,
    Earthquake = 0x59,
    Fissure = 0x5A,
    Dig = 0x5B,
    Toxic = 0x5C,
    Confusion = 0x5D,
    Psychic = 0x5E,
    Hypnosis = 0x5F,
    Meditate = 0x60,
    Agility = 0x61,
    QuickAttack = 0x62,
    Rage = 0x63,
    Teleport = 0x64,
    NightShade = 0x65,
    Mimic = 0x66,
    Screech = 0x67,
    DoubleTeam = 0x68,
    Recover = 0x69,
    Harden = 0x6A,
    Minimize = 0x6B,
    Smokescreen = 0x6C,
    ConfuseRay = 0x6D,
    Withdraw = 0x6E,
    DefenseCurl = 0x6F,
    Barrier = 0x70,
    LightScreen = 0x71,
    Haze = 0x72,
    Reflect = 0x73,
    FocusEnergy = 0x74,
    Bide = 0x75,
    Metronome = 0x76,
    MirrorMove = 0x77,
    SelfDestruct = 0x78,
    EggBomb = 0x79,
    Lick = 0x7A,
    Smog = 0x7B,
    Sludge = 0x7C,
    BoneClub = 0x7D,
    FireBlast = 0x7E,
    Waterfall = 0x7F,
    Clamp = 0x80,
    Swift = 0x81,
    SkullBash = 0x82,
    SpikeCannon = 0x83,
    Constrict = 0x84,
    Amnesia = 0x85,
    Kinesis = 0x86,
    SoftBoiled = 0x87,
    HiJumpKick = 0x88,
    Glare = 0x89,
    DreamEater = 0x8A,
    PoisonGas = 0x8B,
    Barrage = 0x8C,
    LeechLife = 0x8D,
    LovelyKiss = 0x8E,
    SkyAttack = 0x8F,
    Transform = 0x90,
    Bubble = 0x91,
    DizzyPunch = 0x92,
    Spore = 0x93,
    Flash = 0x94,
    Psywave = 0x95,
    Splash = 0x96,
    AcidArmor = 0x97,
    CrabHammer = 0x98,
    Explosion = 0x99,
    FurySwipes = 0x9A,
    Bonemerang = 0x9B,
    Rest = 0x9C,
    RockSlide = 0x9D,
    HyperFang = 0x9E,
    Sharpen = 0x9F,
    Conversion = 0xA0,
    TriAttack = 0xA1,
    SuperFang = 0xA2,
    Slash = 0xA3,
    Substitute = 0xA4,
    Struggle = 0xA5,
}

impl PokemonMoveName {
    pub fn metadata(&self) -> &'static PokemonMoveMetadata {
        use PokemonMoveName::*;

        match self {
            Pound => &PokemonMoveMetadata::POUND,
            KarateChop => &PokemonMoveMetadata::KARATE_CHOP,
            DoubleSlap => &PokemonMoveMetadata::DOUBLE_SLAP,
            CometPunch => &PokemonMoveMetadata::COMET_PUNCH,
            MegaPunch => &PokemonMoveMetadata::MEGA_PUNCH,
            PayDay => &PokemonMoveMetadata::PAY_DAY,
            FirePunch => &PokemonMoveMetadata::FIRE_PUNCH,
            IcePunch => &PokemonMoveMetadata::ICE_PUNCH,
            ThunderPunch => &PokemonMoveMetadata::THUNDER_PUNCH,
            Scratch => &PokemonMoveMetadata::SCRATCH,
            ViceGrip => &PokemonMoveMetadata::VICE_GRIP,
            Guillotine => &PokemonMoveMetadata::GUILLOTINE,
            RazorWind => &PokemonMoveMetadata::RAZOR_WIND,
            SwordsDance => &PokemonMoveMetadata::SWORDS_DANCE,
            Cut => &PokemonMoveMetadata::CUT,
            Gust => &PokemonMoveMetadata::GUST,
            WingAttack => &PokemonMoveMetadata::WING_ATTACK,
            Whirlwind => &PokemonMoveMetadata::WHIRLWIND,
            Fly => &PokemonMoveMetadata::FLY,
            Bind => &PokemonMoveMetadata::BIND,
            Slam => &PokemonMoveMetadata::SLAM,
            VineWhip => &PokemonMoveMetadata::VINE_WHIP,
            Stomp => &PokemonMoveMetadata::STOMP,
            DoubleKick => &PokemonMoveMetadata::DOUBLE_KICK,
            MegaKick => &PokemonMoveMetadata::MEGA_KICK,
            JumpKick => &PokemonMoveMetadata::JUMP_KICK,
            RollingKick => &PokemonMoveMetadata::ROLLING_KICK,
            SandAttack => &PokemonMoveMetadata::SAND_ATTACK,
            Headbutt => &PokemonMoveMetadata::HEADBUTT,
            HornAttack => &PokemonMoveMetadata::HORN_ATTACK,
            FuryAttack => &PokemonMoveMetadata::FURY_ATTACK,
            HornDrill => &PokemonMoveMetadata::HORN_DRILL,
            Tackle => &PokemonMoveMetadata::TACKLE,
            BodySlam => &PokemonMoveMetadata::BODY_SLAM,
            Wrap => &PokemonMoveMetadata::WRAP,
            TakeDown => &PokemonMoveMetadata::TAKE_DOWN,
            Thrash => &PokemonMoveMetadata::THRASH,
            DoubleEdge => &PokemonMoveMetadata::DOUBLE_EDGE,
            TailWhip => &PokemonMoveMetadata::TAIL_WHIP,
            PoisonSting => &PokemonMoveMetadata::POISON_STING,
            Twineedle => &PokemonMoveMetadata::TWINEEDLE,
            PinMissile => &PokemonMoveMetadata::PIN_MISSILE,
            Leer => &PokemonMoveMetadata::LEER,
            Bite => &PokemonMoveMetadata::BITE,
            Growl => &PokemonMoveMetadata::GROWL,
            Roar => &PokemonMoveMetadata::ROAR,
            Sing => &PokemonMoveMetadata::SING,
            Supersonic => &PokemonMoveMetadata::SUPERSONIC,
            SonicBoom => &PokemonMoveMetadata::SONIC_BOOM,
            Disable => &PokemonMoveMetadata::DISABLE,
            Acid => &PokemonMoveMetadata::ACID,
            Ember => &PokemonMoveMetadata::EMBER,
            Flamethrower => &PokemonMoveMetadata::FLAMETHROWER,
            Mist => &PokemonMoveMetadata::MIST,
            WaterGun => &PokemonMoveMetadata::WATER_GUN,
            HydroPump => &PokemonMoveMetadata::HYDRO_PUMP,
            Surf => &PokemonMoveMetadata::SURF,
            IceBeam => &PokemonMoveMetadata::ICE_BEAM,
            Blizzard => &PokemonMoveMetadata::BLIZZARD,
            Psybeam => &PokemonMoveMetadata::PSYBEAM,
            BubbleBeam => &PokemonMoveMetadata::BUBBLE_BEAM,
            AuroraBeam => &PokemonMoveMetadata::AURORA_BEAM,
            HyperBeam => &PokemonMoveMetadata::HYPER_BEAM,
            Peck => &PokemonMoveMetadata::PECK,
            DrillPeck => &PokemonMoveMetadata::DRILL_PECK,
            Submission => &PokemonMoveMetadata::SUBMISSION,
            LowKick => &PokemonMoveMetadata::LOW_KICK,
            Counter => &PokemonMoveMetadata::COUNTER,
            SeismicToss => &PokemonMoveMetadata::SEISMIC_TOSS,
            Strength => &PokemonMoveMetadata::STRENGTH,
            Absorb => &PokemonMoveMetadata::ABSORB,
            MegaDrain => &PokemonMoveMetadata::MEGA_DRAIN,
            LeechSeed => &PokemonMoveMetadata::LEECH_SEED,
            Growth => &PokemonMoveMetadata::GROWTH,
            RazorLeaf => &PokemonMoveMetadata::RAZOR_LEAF,
            SolarBeam => &PokemonMoveMetadata::SOLAR_BEAM,
            PoisonPowder => &PokemonMoveMetadata::POISON_POWDER,
            StunSpore => &PokemonMoveMetadata::STUN_SPORE,
            SleepPowder => &PokemonMoveMetadata::SLEEP_POWDER,
            PetalDance => &PokemonMoveMetadata::PETAL_DANCE,
            StringShot => &PokemonMoveMetadata::STRING_SHOT,
            DragonRage => &PokemonMoveMetadata::DRAGON_RAGE,
            FireSpin => &PokemonMoveMetadata::FIRE_SPIN,
            Thundershock => &PokemonMoveMetadata::THUNDER_SHOCK,
            Thunderbolt => &PokemonMoveMetadata::THUNDERBOLT,
            ThunderWave => &PokemonMoveMetadata::THUNDER_WAVE,
            Thunder => &PokemonMoveMetadata::THUNDER,
            RockThrow => &PokemonMoveMetadata::ROCK_THROW,
            Earthquake => &PokemonMoveMetadata::EARTHQUAKE,
            Fissure => &PokemonMoveMetadata::FISSURE,
            Dig => &PokemonMoveMetadata::DIG,
            Toxic => &PokemonMoveMetadata::TOXIC,
            Confusion => &PokemonMoveMetadata::CONFUSION,
            Psychic => &PokemonMoveMetadata::PSYCHIC,
            Hypnosis => &PokemonMoveMetadata::HYPNOSIS,
            Meditate => &PokemonMoveMetadata::MEDITATE,
            Agility => &PokemonMoveMetadata::AGILITY,
            QuickAttack => &PokemonMoveMetadata::QUICK_ATTACK,
            Rage => &PokemonMoveMetadata::RAGE,
            Teleport => &PokemonMoveMetadata::TELEPORT,
            NightShade => &PokemonMoveMetadata::NIGHT_SHADE,
            Mimic => &PokemonMoveMetadata::MIMIC,
            Screech => &PokemonMoveMetadata::SCREECH,
            DoubleTeam => &PokemonMoveMetadata::DOUBLE_TEAM,
            Recover => &PokemonMoveMetadata::RECOVER,
            Harden => &PokemonMoveMetadata::HARDEN,
            Minimize => &PokemonMoveMetadata::MINIMIZE,
            Smokescreen => &PokemonMoveMetadata::SMOKESCREEN,
            ConfuseRay => &PokemonMoveMetadata::CONFUSE_RAY,
            Withdraw => &PokemonMoveMetadata::WITHDRAW,
            DefenseCurl => &PokemonMoveMetadata::DEFENSE_CURL,
            Barrier => &PokemonMoveMetadata::BARRIER,
            LightScreen => &PokemonMoveMetadata::LIGHT_SCREEN,
            Haze => &PokemonMoveMetadata::HAZE,
            Reflect => &PokemonMoveMetadata::REFLECT,
            FocusEnergy => &PokemonMoveMetadata::FOCUS_ENERGY,
            Bide => &PokemonMoveMetadata::BIDE,
            Metronome => &PokemonMoveMetadata::METRONOME,
            MirrorMove => &PokemonMoveMetadata::MIRROR_MOVE,
            SelfDestruct => &PokemonMoveMetadata::SELF_DESTRUCT,
            EggBomb => &PokemonMoveMetadata::EGG_BOMB,
            Lick => &PokemonMoveMetadata::LICK,
            Smog => &PokemonMoveMetadata::SMOG,
            Sludge => &PokemonMoveMetadata::SLUDGE,
            BoneClub => &PokemonMoveMetadata::BONE_CLUB,
            FireBlast => &PokemonMoveMetadata::FIRE_BLAST,
            Waterfall => &PokemonMoveMetadata::WATERFALL,
            Clamp => &PokemonMoveMetadata::CLAMP,
            Swift => &PokemonMoveMetadata::SWIFT,
            SkullBash => &PokemonMoveMetadata::SKULL_BASH,
            SpikeCannon => &PokemonMoveMetadata::SPIKE_CANNON,
            Constrict => &PokemonMoveMetadata::CONSTRICT,
            Amnesia => &PokemonMoveMetadata::AMNESIA,
            Kinesis => &PokemonMoveMetadata::KINESIS,
            SoftBoiled => &PokemonMoveMetadata::SOFT_BOILED,
            HiJumpKick => &PokemonMoveMetadata::HIGH_JUMP_KICK,
            Glare => &PokemonMoveMetadata::GLARE,
            DreamEater => &PokemonMoveMetadata::DREAM_EATER,
            PoisonGas => &PokemonMoveMetadata::POISON_GAS,
            Barrage => &PokemonMoveMetadata::BARRAGE,
            LeechLife => &PokemonMoveMetadata::LEECH_LIFE,
            LovelyKiss => &PokemonMoveMetadata::LOVELY_KISS,
            SkyAttack => &PokemonMoveMetadata::SKY_ATTACK,
            Transform => &PokemonMoveMetadata::TRANSFORM,
            Bubble => &PokemonMoveMetadata::BUBBLE,
            DizzyPunch => &PokemonMoveMetadata::DIZZY_PUNCH,
            Spore => &PokemonMoveMetadata::SPORE,
            Flash => &PokemonMoveMetadata::FLASH,
            Psywave => &PokemonMoveMetadata::PSYWAVE,
            Splash => &PokemonMoveMetadata::SPLASH,
            AcidArmor => &PokemonMoveMetadata::ACID_ARMOR,
            CrabHammer => &PokemonMoveMetadata::CRABHAMMER,
            Explosion => &PokemonMoveMetadata::EXPLOSION,
            FurySwipes => &PokemonMoveMetadata::FURY_SWIPES,
            Bonemerang => &PokemonMoveMetadata::BONEMERANG,
            Rest => &PokemonMoveMetadata::REST,
            RockSlide => &PokemonMoveMetadata::ROCK_SLIDE,
            HyperFang => &PokemonMoveMetadata::HYPER_FANG,
            Sharpen => &PokemonMoveMetadata::SHARPEN,
            Conversion => &PokemonMoveMetadata::CONVERSION,
            TriAttack => &PokemonMoveMetadata::TRI_ATTACK,
            SuperFang => &PokemonMoveMetadata::SUPER_FANG,
            Slash => &PokemonMoveMetadata::SLASH,
            Substitute => &PokemonMoveMetadata::SUBSTITUTE,
            Struggle => &PokemonMoveMetadata::STRUGGLE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveCategory {
    Physical,
    Special,
    Status
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PokemonMoveMetadata {
    pub name: &'static str,
    pub move_type: PokemonType,
    pub category: MoveCategory,
    pub power: Option<u8>,
    pub accuracy: Option<u8>,
    pub pp: u8,
}



impl PokemonMoveMetadata {
    pub const fn new(name: &'static str, move_type: PokemonType, category: MoveCategory, power: Option<u8>, accuracy: Option<u8>, pp: u8) -> Self {
        assert!(matches!(accuracy, None | Some(0..=100)));
        Self { name, move_type, category, power, accuracy, pp }
    }

    pub const POUND: Self = Self::new("Pound", Normal, Physical, Some(40), Some(100), 35);
    pub const KARATE_CHOP: Self = Self::new("Karate Chop", Fighting, Physical, Some(50), Some(100), 25);
    pub const DOUBLE_SLAP: Self = Self::new("Double Slap", Normal, Physical, Some(15), Some(85), 10);
    pub const COMET_PUNCH: Self = Self::new("Comet Punch", Normal, Physical, Some(18), Some(85), 15);
    pub const MEGA_PUNCH: Self = Self::new("Mega Punch", Normal, Physical, Some(80), Some(85), 20);
    pub const PAY_DAY: Self = Self::new("Pay Day", Normal, Physical, Some(40), Some(100), 20);
    pub const FIRE_PUNCH: Self = Self::new("Fire Punch", Fire, Physical, Some(75), Some(100), 15);
    pub const ICE_PUNCH: Self = Self::new("Ice Punch", Ice, Physical, Some(75), Some(100), 15);
    pub const THUNDER_PUNCH: Self = Self::new("Thunder Punch", Electric, Physical, Some(75), Some(100), 15);
    pub const SCRATCH: Self = Self::new("Scratch", Normal, Physical, Some(40), Some(100), 35);
    pub const VICE_GRIP: Self = Self::new("Vice Grip", Normal, Physical, Some(55), Some(100), 30);
    pub const GUILLOTINE: Self = Self::new("Guillotine", Normal, Physical, None, Some(30), 5);
    pub const RAZOR_WIND: Self = Self::new("Razor Wind", Normal, Special, Some(80), Some(100), 10);
    pub const SWORDS_DANCE: Self = Self::new("Swords Dance", Normal, Status, None, None, 20);
    pub const CUT: Self = Self::new("Cut", Normal, Physical, Some(50), Some(95), 30);
    pub const GUST: Self = Self::new("Gust", Flying, Special, Some(40), Some(100), 35);
    pub const WING_ATTACK: Self = Self::new("Wing Attack", Flying, Physical, Some(60), Some(100), 35);
    pub const WHIRLWIND: Self = Self::new("Whirlwind", Normal, Status, None, None, 20);
    pub const FLY: Self = Self::new("Fly", Flying, Physical, Some(90), Some(95), 15);
    pub const BIND: Self = Self::new("Bind", Normal, Physical, Some(15), Some(85), 20);
    pub const SLAM: Self = Self::new("Slam", Normal, Physical, Some(80), Some(75), 20);
    pub const VINE_WHIP: Self = Self::new("Vine Whip", Grass, Physical, Some(45), Some(100), 25);
    pub const STOMP: Self = Self::new("Stomp", Normal, Physical, Some(65), Some(100), 20);
    pub const DOUBLE_KICK: Self = Self::new("Double Kick", Fighting, Physical, Some(30), Some(100), 30);
    pub const MEGA_KICK: Self = Self::new("Mega Kick", Normal, Physical, Some(120), Some(75), 5);
    pub const JUMP_KICK: Self = Self::new("Jump Kick", Fighting, Physical, Some(100), Some(95), 10);
    pub const ROLLING_KICK: Self = Self::new("Rolling Kick", Fighting, Physical, Some(60), Some(85), 15);
    pub const SAND_ATTACK: Self = Self::new("Sand Attack", Ground, Status, None, Some(100), 15);
    pub const HEADBUTT: Self = Self::new("Headbutt", Normal, Physical, Some(70), Some(100), 15);
    pub const HORN_ATTACK: Self = Self::new("Horn Attack", Normal, Physical, Some(65), Some(100), 25);
    pub const FURY_ATTACK: Self = Self::new("Fury Attack", Normal, Physical, Some(15), Some(85), 20);
    pub const HORN_DRILL: Self = Self::new("Horn Drill", Normal, Physical, None, Some(30), 5);
    pub const TACKLE: Self = Self::new("Tackle", Normal, Physical, Some(40), Some(100), 35);
    pub const BODY_SLAM: Self = Self::new("Body Slam", Normal, Physical, Some(85), Some(100), 15);
    pub const WRAP: Self = Self::new("Wrap", Normal, Physical, Some(15), Some(90), 20);
    pub const TAKE_DOWN: Self = Self::new("Take Down", Normal, Physical, Some(90), Some(85), 20);
    pub const THRASH: Self = Self::new("Thrash", Normal, Physical, Some(120), Some(100), 10);
    pub const DOUBLE_EDGE: Self = Self::new("Double-Edge", Normal, Physical, Some(120), Some(100), 15);
    pub const TAIL_WHIP: Self = Self::new("Tail Whip", Normal, Status, None, Some(100), 30);
    pub const POISON_STING: Self = Self::new("Poison Sting", Poison, Physical, Some(15), Some(100), 35);
    pub const TWINEEDLE: Self = Self::new("Twineedle", Bug, Physical, Some(25), Some(100), 20);
    pub const PIN_MISSILE: Self = Self::new("Pin Missile", Bug, Physical, Some(25), Some(95), 20);
    pub const LEER: Self = Self::new("Leer", Normal, Status, None, Some(100), 30);
    pub const BITE: Self = Self::new("Bite", Normal, Physical, Some(60), Some(100), 25);
    pub const GROWL: Self = Self::new("Growl", Normal, Status, None, Some(100), 40);
    pub const ROAR: Self = Self::new("Roar", Normal, Status, None, None, 20);
    pub const SING: Self = Self::new("Sing", Normal, Status, None, Some(55), 15);
    pub const SUPERSONIC: Self = Self::new("Supersonic", Normal, Status, None, Some(55), 20);
    pub const SONIC_BOOM: Self = Self::new("Sonic Boom", Normal, Special, None, Some(90), 20);
    pub const DISABLE: Self = Self::new("Disable", Normal, Status, None, Some(100), 20);
    pub const ACID: Self = Self::new("Acid", Poison, Special, Some(40), Some(100), 30);
    pub const EMBER: Self = Self::new("Ember", Fire, Special, Some(40), Some(100), 25);
    pub const FLAMETHROWER: Self = Self::new("Flamethrower", Fire, Special, Some(90), Some(100), 15);
    pub const MIST: Self = Self::new("Mist", Ice, Status, None, None, 30);
    pub const WATER_GUN: Self = Self::new("Water Gun", Water, Special, Some(40), Some(100), 25);
    pub const HYDRO_PUMP: Self = Self::new("Hydro Pump", Water, Special, Some(110), Some(80), 5);
    pub const SURF: Self = Self::new("Surf", Water, Special, Some(90), Some(100), 15);
    pub const ICE_BEAM: Self = Self::new("Ice Beam", Ice, Special, Some(90), Some(100), 10);
    pub const BLIZZARD: Self = Self::new("Blizzard", Ice, Special, Some(110), Some(70), 5);
    pub const PSYBEAM: Self = Self::new("Psybeam", Psychic, Special, Some(65), Some(100), 20);
    pub const BUBBLE_BEAM: Self = Self::new("Bubble Beam", Water, Special, Some(65), Some(100), 20);
    pub const AURORA_BEAM: Self = Self::new("Aurora Beam", Ice, Special, Some(65), Some(100), 20);
    pub const HYPER_BEAM: Self = Self::new("Hyper Beam", Normal, Special, Some(150), Some(90), 5);
    pub const PECK: Self = Self::new("Peck", Flying, Physical, Some(35), Some(100), 35);
    pub const DRILL_PECK: Self = Self::new("Drill Peck", Flying, Physical, Some(80), Some(100), 20);
    pub const SUBMISSION: Self = Self::new("Submission", Fighting, Physical, Some(80), Some(80), 20);
    pub const LOW_KICK: Self = Self::new("Low Kick", Fighting, Physical, None, Some(100), 20);
    pub const COUNTER: Self = Self::new("Counter", Fighting, Physical, None, Some(100), 20);
    pub const SEISMIC_TOSS: Self = Self::new("Seismic Toss", Fighting, Physical, None, Some(100), 20);
    pub const STRENGTH: Self = Self::new("Strength", Normal, Physical, Some(80), Some(100), 15);
    pub const ABSORB: Self = Self::new("Absorb", Grass, Special, Some(20), Some(100), 25);
    pub const MEGA_DRAIN: Self = Self::new("Mega Drain", Grass, Special, Some(40), Some(100), 15);
    pub const LEECH_SEED: Self = Self::new("Leech Seed", Grass, Status, None, Some(90), 10);
    pub const GROWTH: Self = Self::new("Growth", Normal, Status, None, None, 20);
    pub const RAZOR_LEAF: Self = Self::new("Razor Leaf", Grass, Physical, Some(55), Some(95), 25);
    pub const SOLAR_BEAM: Self = Self::new("Solar Beam", Grass, Special, Some(120), Some(100), 10);
    pub const POISON_POWDER: Self = Self::new("Poison Powder", Poison, Status, None, Some(75), 35);
    pub const STUN_SPORE: Self = Self::new("Stun Spore", Grass, Status, None, Some(75), 30);
    pub const SLEEP_POWDER: Self = Self::new("Sleep Powder", Grass, Status, None, Some(75), 15);
    pub const PETAL_DANCE: Self = Self::new("Petal Dance", Grass, Special, Some(120), Some(100), 10);
    pub const STRING_SHOT: Self = Self::new("String Shot", Bug, Status, None, Some(95), 40);
    pub const DRAGON_RAGE: Self = Self::new("Dragon Rage", Dragon, Special, None, Some(100), 10);
    pub const FIRE_SPIN: Self = Self::new("Fire Spin", Fire, Special, Some(35), Some(85), 15);
    pub const THUNDER_SHOCK: Self = Self::new("Thunder Shock", Electric, Special, Some(40), Some(100), 30);
    pub const THUNDERBOLT: Self = Self::new("Thunderbolt", Electric, Special, Some(90), Some(100), 15);
    pub const THUNDER_WAVE: Self = Self::new("Thunder Wave", Electric, Status, None, Some(90), 20);
    pub const THUNDER: Self = Self::new("Thunder", Electric, Special, Some(110), Some(70), 10);
    pub const ROCK_THROW: Self = Self::new("Rock Throw", Rock, Physical, Some(50), Some(90), 15);
    pub const EARTHQUAKE: Self = Self::new("Earthquake", Ground, Physical, Some(100), Some(100), 10);
    pub const FISSURE: Self = Self::new("Fissure", Ground, Physical, None, Some(30), 5);
    pub const DIG: Self = Self::new("Dig", Ground, Physical, Some(80), Some(100), 10);
    pub const TOXIC: Self = Self::new("Toxic", Poison, Status, None, Some(90), 10);
    pub const CONFUSION: Self = Self::new("Confusion", Psychic, Special, Some(50), Some(100), 25);
    pub const PSYCHIC: Self = Self::new("Psychic", Psychic, Special, Some(90), Some(100), 10);
    pub const HYPNOSIS: Self = Self::new("Hypnosis", Psychic, Status, None, Some(60), 20);
    pub const MEDITATE: Self = Self::new("Meditate", Psychic, Status, None, None, 40);
    pub const AGILITY: Self = Self::new("Agility", Psychic, Status, None, None, 30);
    pub const QUICK_ATTACK: Self = Self::new("Quick Attack", Normal, Physical, Some(40), Some(100), 30);
    pub const RAGE: Self = Self::new("Rage", Normal, Physical, Some(20), Some(100), 20);
    pub const TELEPORT: Self = Self::new("Teleport", Psychic, Status, None, None, 20);
    pub const NIGHT_SHADE: Self = Self::new("Night Shade", Ghost, Special, None, Some(100), 15);
    pub const MIMIC: Self = Self::new("Mimic", Normal, Status, None, None, 10);
    pub const SCREECH: Self = Self::new("Screech", Normal, Status, None, None, 40);
    pub const DOUBLE_TEAM: Self = Self::new("Double Team", Normal, Status, None, None, 15);
    pub const RECOVER: Self = Self::new("Recover", Normal, Status, None, None, 5);
    pub const HARDEN: Self = Self::new("Harden", Normal, Status, None, None, 30);
    pub const MINIMIZE: Self = Self::new("Minimize", Normal, Status, None, None, 10);
    pub const SMOKESCREEN: Self = Self::new("Smokescreen", Normal, Status, None, Some(100), 20);
    pub const CONFUSE_RAY: Self = Self::new("Confuse Ray", Ghost, Status, None, Some(100), 10);
    pub const WITHDRAW: Self = Self::new("Withdraw", Water, Status, None, None, 40);
    pub const DEFENSE_CURL: Self = Self::new("Defense Curl", Normal, Status, None, None, 40);
    pub const BARRIER: Self = Self::new("Barrier", Psychic, Status, None, None, 20);
    pub const LIGHT_SCREEN: Self = Self::new("Light Screen", Psychic, Status, None, None, 30);
    pub const HAZE: Self = Self::new("Haze", Ice, Status, None, None, 30);
    pub const REFLECT: Self = Self::new("Reflect", Psychic, Status, None, None, 20);
    pub const FOCUS_ENERGY: Self = Self::new("Focus Energy", Normal, Status, None, None, 30);
    pub const BIDE: Self = Self::new("Bide", Normal, Physical, None, None, 10);
    pub const METRONOME: Self = Self::new("Metronome", Normal, Status, None, None, 10);
    pub const MIRROR_MOVE: Self = Self::new("Mirror Move", Flying, Status, None, None, 20);
    pub const SELF_DESTRUCT: Self = Self::new("Self-Destruct", Normal, Physical, Some(200), Some(100), 5);
    pub const EGG_BOMB: Self = Self::new("Egg Bomb", Normal, Physical, Some(100), Some(75), 10);
    pub const LICK: Self = Self::new("Lick", Ghost, Physical, Some(30), Some(100), 30);
    pub const SMOG: Self = Self::new("Smog", Poison, Special, Some(30), Some(70), 20);
    pub const SLUDGE: Self = Self::new("Sludge", Poison, Special, Some(65), Some(100), 20);
    pub const BONE_CLUB: Self = Self::new("Bone Club", Ground, Physical, Some(65), Some(85), 20);
    pub const FIRE_BLAST: Self = Self::new("Fire Blast", Fire, Special, Some(110), Some(85), 5);
    pub const WATERFALL: Self = Self::new("Waterfall", Water, Physical, Some(80), Some(100), 15);
    pub const CLAMP: Self = Self::new("Clamp", Water, Physical, Some(35), Some(85), 15);
    pub const SWIFT: Self = Self::new("Swift", Normal, Special, Some(60), None, 20);
    pub const SKULL_BASH: Self = Self::new("Skull Bash", Normal, Physical, Some(130), Some(100), 10);
    pub const SPIKE_CANNON: Self = Self::new("Spike Cannon", Normal, Physical, Some(20), Some(100), 15);
    pub const CONSTRICT: Self = Self::new("Constrict", Normal, Physical, Some(10), Some(100), 35);
    pub const AMNESIA: Self = Self::new("Amnesia", Psychic, Status, None, None, 20);
    pub const KINESIS: Self = Self::new("Kinesis", Psychic, Status, None, Some(80), 15);
    pub const SOFT_BOILED: Self = Self::new("Soft-Boiled", Normal, Status, None, None, 5);
    pub const HIGH_JUMP_KICK: Self = Self::new("High Jump Kick", Fighting, Physical, Some(130), Some(90), 10);
    pub const GLARE: Self = Self::new("Glare", Normal, Status, None, Some(100), 30);
    pub const DREAM_EATER: Self = Self::new("Dream Eater", Psychic, Special, Some(100), Some(100), 15);
    pub const POISON_GAS: Self = Self::new("Poison Gas", Poison, Status, None, Some(90), 40);
    pub const BARRAGE: Self = Self::new("Barrage", Normal, Physical, Some(15), Some(85), 20);
    pub const LEECH_LIFE: Self = Self::new("Leech Life", Bug, Physical, Some(80), Some(100), 10);
    pub const LOVELY_KISS: Self = Self::new("Lovely Kiss", Normal, Status, None, Some(75), 10);
    pub const SKY_ATTACK: Self = Self::new("Sky Attack", Flying, Physical, Some(140), Some(90), 5);
    pub const TRANSFORM: Self = Self::new("Transform", Normal, Status, None, None, 10);
    pub const BUBBLE: Self = Self::new("Bubble", Water, Special, Some(40), Some(100), 30);
    pub const DIZZY_PUNCH: Self = Self::new("Dizzy Punch", Normal, Physical, Some(70), Some(100), 10);
    pub const SPORE: Self = Self::new("Spore", Grass, Status, None, Some(100), 15);
    pub const FLASH: Self = Self::new("Flash", Normal, Status, None, Some(100), 20);
    pub const PSYWAVE: Self = Self::new("Psywave", Psychic, Special, None, Some(100), 15);
    pub const SPLASH: Self = Self::new("Splash", Normal, Status, None, None, 40);
    pub const ACID_ARMOR: Self = Self::new("Acid Armor", Poison, Status, None, None, 20);
    pub const CRABHAMMER: Self = Self::new("Crabhammer", Water, Physical, Some(100), Some(90), 10);
    pub const EXPLOSION: Self = Self::new("Explosion", Normal, Physical, Some(250), Some(100), 5);
    pub const FURY_SWIPES: Self = Self::new("Fury Swipes", Normal, Physical, Some(18), Some(80), 15);
    pub const BONEMERANG: Self = Self::new("Bonemerang", Ground, Physical, Some(50), Some(90), 10);
    pub const REST: Self = Self::new("Rest", Psychic, Status, None, None, 5);
    pub const ROCK_SLIDE: Self = Self::new("Rock Slide", Rock, Physical, Some(75), Some(90), 10);
    pub const HYPER_FANG: Self = Self::new("Hyper Fang", Normal, Physical, Some(80), Some(90), 15);
    pub const SHARPEN: Self = Self::new("Sharpen", Normal, Status, None, None, 30);
    pub const CONVERSION: Self = Self::new("Conversion", Normal, Status, None, None, 30);
    pub const TRI_ATTACK: Self = Self::new("Tri Attack", Normal, Special, Some(80), Some(100), 10);
    pub const SUPER_FANG: Self = Self::new("Super Fang", Normal, Physical, None, Some(90), 10);
    pub const SLASH: Self = Self::new("Slash", Normal, Physical, Some(70), Some(100), 20);
    pub const SUBSTITUTE: Self = Self::new("Substitute", Normal, Status, None, None, 10);
    pub const STRUGGLE: Self = Self::new("Struggle", Normal, Physical, Some(50), None, 1);

}