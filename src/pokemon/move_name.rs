use crate::pokemon::pokemon::PokemonType;
use PokemonType::*;

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
    #[strum(serialize = "Karate Chop")] KarateChop = 0x2,
    Doubleslap = 0x3,
    #[strum(serialize = "Comet Punch")] CometPunch = 0x4,
    #[strum(serialize = "Mega Punch")] MegaPunch = 0x5,
    #[strum(serialize = "Pay Day")] PayDay = 0x6,
    #[strum(serialize = "Fire Punch")] FirePunch = 0x7,
    #[strum(serialize = "Ice Punch")] IcePunch = 0x8,
    Thunderpunch = 0x9,
    Scratch = 0xA,
    Vicegrip = 0xB,
    Guillotine = 0xC,
    #[strum(serialize = "Razor Wind")] RazorWind = 0xD,
    #[strum(serialize = "Swords Dance")] SwordsDance = 0xE,
    Cut = 0xF,
    Gust = 0x10,
    #[strum(serialize = "Wing Attack")] WingAttack = 0x11,
    Whirlwind = 0x12,
    Fly = 0x13,
    Bind = 0x14,
    Slam = 0x15,
    #[strum(serialize = "Vine Whip")] VineWhip = 0x16,
    Stomp = 0x17,
    #[strum(serialize = "Double Kick")] DoubleKick = 0x18,
    #[strum(serialize = "Mega Kick")] MegaKick = 0x19,
    #[strum(serialize = "Jump Kick")] JumpKick = 0x1A,
    #[strum(serialize = "Rolling Kick")] RollingKick = 0x1B,
    #[strum(serialize = "Sand Attack")] SandAttack = 0x1C,
    Headbutt = 0x1D,
    #[strum(serialize = "Horn Attack")] HornAttack = 0x1E,
    #[strum(serialize = "Fury Attack")] FuryAttack = 0x1F,
    #[strum(serialize = "Horn Drill")] HornDrill = 0x20,
    Tackle = 0x21,
    #[strum(serialize = "Body Slam")] BodySlam = 0x22,
    Wrap = 0x23,
    #[strum(serialize = "Take Down")] TakeDown = 0x24,
    Thrash = 0x25,
    #[strum(serialize = "Double Edge")] DoubleEdge = 0x26,
    #[strum(serialize = "Tail Whip")] TailWhip = 0x27,
    #[strum(serialize = "Poison Sting")] PoisonSting = 0x28,
    Twineedle = 0x29,
    #[strum(serialize = "Pin Missile")] PinMissile = 0x2A,
    Leer = 0x2B,
    Bite = 0x2C,
    Growl = 0x2D,
    Roar = 0x2E,
    Sing = 0x2F,
    Supersonic = 0x30,
    Sonicboom = 0x31,
    Disable = 0x32,
    Acid = 0x33,
    Ember = 0x34,
    Flamethrower = 0x35,
    Mist = 0x36,
    #[strum(serialize = "Water Gun")] WaterGun = 0x37,
    #[strum(serialize = "Hydro Pump")] HydroPump = 0x38,
    Surf = 0x39,
    #[strum(serialize = "Ice Beam")] IceBeam = 0x3A,
    Blizzard = 0x3B,
    Psybeam = 0x3C,
    Bubblebeam = 0x3D,
    #[strum(serialize = "Aurora Beam")] AuroraBeam = 0x3E,
    #[strum(serialize = "Hyper Beam")] HyperBeam = 0x3F,
    Peck = 0x40,
    #[strum(serialize = "Drill Peck")] DrillPeck = 0x41,
    Submission = 0x42,
    #[strum(serialize = "Low Kick")] LowKick = 0x43,
    Counter = 0x44,
    #[strum(serialize = "Seismic Toss")] SeismicToss = 0x45,
    Strength = 0x46,
    Absorb = 0x47,
    #[strum(serialize = "Mega Drain")] MegaDrain = 0x48,
    #[strum(serialize = "Leech Seed")] LeechSeed = 0x49,
    Growth = 0x4A,
    #[strum(serialize = "Razor Leaf")] RazorLeaf = 0x4B,
    Solarbeam = 0x4C,
    Poisonpowder = 0x4D,
    #[strum(serialize = "Stun Spore")] StunSpore = 0x4E,
    #[strum(serialize = "Sleep Powder")] SleepPowder = 0x4F,
    #[strum(serialize = "Petal Dance")] PetalDance = 0x50,
    #[strum(serialize = "String Shot")] StringShot = 0x51,
    #[strum(serialize = "Dragon Rage")] DragonRage = 0x52,
    #[strum(serialize = "Fire Spin")] FireSpin = 0x53,
    Thundershock = 0x54,
    Thunderbolt = 0x55,
    #[strum(serialize = "Thunder Wave")] ThunderWave = 0x56,
    Thunder = 0x57,
    #[strum(serialize = "Rock Throw")] RockThrow = 0x58,
    Earthquake = 0x59,
    Fissure = 0x5A,
    Dig = 0x5B,
    Toxic = 0x5C,
    Confusion = 0x5D,
    #[strum(serialize = "Psychic")] Psychic = 0x5E,
    Hypnosis = 0x5F,
    Meditate = 0x60,
    Agility = 0x61,
    #[strum(serialize = "Quick Attack")] QuickAttack = 0x62,
    Rage = 0x63,
    Teleport = 0x64,
    #[strum(serialize = "Night Shade")] NightShade = 0x65,
    Mimic = 0x66,
    Screech = 0x67,
    #[strum(serialize = "Double Team")] DoubleTeam = 0x68,
    Recover = 0x69,
    Harden = 0x6A,
    Minimize = 0x6B,
    Smokescreen = 0x6C,
    #[strum(serialize = "Confuse Ray")] ConfuseRay = 0x6D,
    Withdraw = 0x6E,
    #[strum(serialize = "Defense Curl")] DefenseCurl = 0x6F,
    Barrier = 0x70,
    #[strum(serialize = "Light Screen")] LightScreen = 0x71,
    Haze = 0x72,
    Reflect = 0x73,
    #[strum(serialize = "Focus Energy")] FocusEnergy = 0x74,
    Bide = 0x75,
    Metronome = 0x76,
    #[strum(serialize = "Mirror Move")] MirrorMove = 0x77,
    Selfdestruct = 0x78,
    #[strum(serialize = "Egg Bomb")] EggBomb = 0x79,
    Lick = 0x7A,
    Smog = 0x7B,
    Sludge = 0x7C,
    #[strum(serialize = "Bone Club")] BoneClub = 0x7D,
    #[strum(serialize = "Fire Blast")] FireBlast = 0x7E,
    Waterfall = 0x7F,
    Clamp = 0x80,
    Swift = 0x81,
    #[strum(serialize = "Skull Bash")] SkullBash = 0x82,
    #[strum(serialize = "Spike Cannon")] SpikeCannon = 0x83,
    Constrict = 0x84,
    Amnesia = 0x85,
    Kinesis = 0x86,
    Softboiled = 0x87,
    #[strum(serialize = "Hi Jump Kick")] HiJumpKick = 0x88,
    Glare = 0x89,
    #[strum(serialize = "Dream Eater")] DreamEater = 0x8A,
    #[strum(serialize = "Poison Gas")] PoisonGas = 0x8B,
    Barrage = 0x8C,
    #[strum(serialize = "Leech Life")] LeechLife = 0x8D,
    #[strum(serialize = "Lovely Kiss")] LovelyKiss = 0x8E,
    #[strum(serialize = "Sky Attack")] SkyAttack = 0x8F,
    Transform = 0x90,
    Bubble = 0x91,
    #[strum(serialize = "Dizzy Punch")] DizzyPunch = 0x92,
    Spore = 0x93,
    Flash = 0x94,
    Psywave = 0x95,
    Splash = 0x96,
    #[strum(serialize = "Acid Armor")] AcidArmor = 0x97,
    Crabhammer = 0x98,
    Explosion = 0x99,
    #[strum(serialize = "Fury Swipes")] FurySwipes = 0x9A,
    Bonemerang = 0x9B,
    Rest = 0x9C,
    #[strum(serialize = "Rock Slide")] RockSlide = 0x9D,
    #[strum(serialize = "Hyper Fang")] HyperFang = 0x9E,
    Sharpen = 0x9F,
    Conversion = 0xA0,
    #[strum(serialize = "Tri Attack")] TriAttack = 0xA1,
    #[strum(serialize = "Super Fang")] SuperFang = 0xA2,
    Slash = 0xA3,
    Substitute = 0xA4,
    Struggle = 0xA5,
}

impl PokemonMoveName {
    pub fn metadata(&self) -> &'static PokemonMoveMetadata {
        match self {
            PokemonMoveName::Pound => &PokemonMoveMetadata::POUND,
            PokemonMoveName::KarateChop => &PokemonMoveMetadata::KARATE_CHOP,
            PokemonMoveName::Doubleslap => &PokemonMoveMetadata::DOUBLESLAP,
            PokemonMoveName::CometPunch => &PokemonMoveMetadata::COMET_PUNCH,
            PokemonMoveName::MegaPunch => &PokemonMoveMetadata::MEGA_PUNCH,
            PokemonMoveName::PayDay => &PokemonMoveMetadata::PAY_DAY,
            PokemonMoveName::FirePunch => &PokemonMoveMetadata::FIRE_PUNCH,
            PokemonMoveName::IcePunch => &PokemonMoveMetadata::ICE_PUNCH,
            PokemonMoveName::Thunderpunch => &PokemonMoveMetadata::THUNDERPUNCH,
            PokemonMoveName::Scratch => &PokemonMoveMetadata::SCRATCH,
            PokemonMoveName::Vicegrip => &PokemonMoveMetadata::VICEGRIP,
            PokemonMoveName::Guillotine => &PokemonMoveMetadata::GUILLOTINE,
            PokemonMoveName::RazorWind => &PokemonMoveMetadata::RAZOR_WIND,
            PokemonMoveName::SwordsDance => &PokemonMoveMetadata::SWORDS_DANCE,
            PokemonMoveName::Cut => &PokemonMoveMetadata::CUT,
            PokemonMoveName::Gust => &PokemonMoveMetadata::GUST,
            PokemonMoveName::WingAttack => &PokemonMoveMetadata::WING_ATTACK,
            PokemonMoveName::Whirlwind => &PokemonMoveMetadata::WHIRLWIND,
            PokemonMoveName::Fly => &PokemonMoveMetadata::FLY,
            PokemonMoveName::Bind => &PokemonMoveMetadata::BIND,
            PokemonMoveName::Slam => &PokemonMoveMetadata::SLAM,
            PokemonMoveName::VineWhip => &PokemonMoveMetadata::VINE_WHIP,
            PokemonMoveName::Stomp => &PokemonMoveMetadata::STOMP,
            PokemonMoveName::DoubleKick => &PokemonMoveMetadata::DOUBLE_KICK,
            PokemonMoveName::MegaKick => &PokemonMoveMetadata::MEGA_KICK,
            PokemonMoveName::JumpKick => &PokemonMoveMetadata::JUMP_KICK,
            PokemonMoveName::RollingKick => &PokemonMoveMetadata::ROLLING_KICK,
            PokemonMoveName::SandAttack => &PokemonMoveMetadata::SAND_ATTACK,
            PokemonMoveName::Headbutt => &PokemonMoveMetadata::HEADBUTT,
            PokemonMoveName::HornAttack => &PokemonMoveMetadata::HORN_ATTACK,
            PokemonMoveName::FuryAttack => &PokemonMoveMetadata::FURY_ATTACK,
            PokemonMoveName::HornDrill => &PokemonMoveMetadata::HORN_DRILL,
            PokemonMoveName::Tackle => &PokemonMoveMetadata::TACKLE,
            PokemonMoveName::BodySlam => &PokemonMoveMetadata::BODY_SLAM,
            PokemonMoveName::Wrap => &PokemonMoveMetadata::WRAP,
            PokemonMoveName::TakeDown => &PokemonMoveMetadata::TAKE_DOWN,
            PokemonMoveName::Thrash => &PokemonMoveMetadata::THRASH,
            PokemonMoveName::DoubleEdge => &PokemonMoveMetadata::DOUBLE_EDGE,
            PokemonMoveName::TailWhip => &PokemonMoveMetadata::TAIL_WHIP,
            PokemonMoveName::PoisonSting => &PokemonMoveMetadata::POISON_STING,
            PokemonMoveName::Twineedle => &PokemonMoveMetadata::TWINEEDLE,
            PokemonMoveName::PinMissile => &PokemonMoveMetadata::PIN_MISSILE,
            PokemonMoveName::Leer => &PokemonMoveMetadata::LEER,
            PokemonMoveName::Bite => &PokemonMoveMetadata::BITE,
            PokemonMoveName::Growl => &PokemonMoveMetadata::GROWL,
            PokemonMoveName::Roar => &PokemonMoveMetadata::ROAR,
            PokemonMoveName::Sing => &PokemonMoveMetadata::SING,
            PokemonMoveName::Supersonic => &PokemonMoveMetadata::SUPERSONIC,
            PokemonMoveName::Sonicboom => &PokemonMoveMetadata::SONICBOOM,
            PokemonMoveName::Disable => &PokemonMoveMetadata::DISABLE,
            PokemonMoveName::Acid => &PokemonMoveMetadata::ACID,
            PokemonMoveName::Ember => &PokemonMoveMetadata::EMBER,
            PokemonMoveName::Flamethrower => &PokemonMoveMetadata::FLAMETHROWER,
            PokemonMoveName::Mist => &PokemonMoveMetadata::MIST,
            PokemonMoveName::WaterGun => &PokemonMoveMetadata::WATER_GUN,
            PokemonMoveName::HydroPump => &PokemonMoveMetadata::HYDRO_PUMP,
            PokemonMoveName::Surf => &PokemonMoveMetadata::SURF,
            PokemonMoveName::IceBeam => &PokemonMoveMetadata::ICE_BEAM,
            PokemonMoveName::Blizzard => &PokemonMoveMetadata::BLIZZARD,
            PokemonMoveName::Psybeam => &PokemonMoveMetadata::PSYBEAM,
            PokemonMoveName::Bubblebeam => &PokemonMoveMetadata::BUBBLEBEAM,
            PokemonMoveName::AuroraBeam => &PokemonMoveMetadata::AURORA_BEAM,
            PokemonMoveName::HyperBeam => &PokemonMoveMetadata::HYPER_BEAM,
            PokemonMoveName::Peck => &PokemonMoveMetadata::PECK,
            PokemonMoveName::DrillPeck => &PokemonMoveMetadata::DRILL_PECK,
            PokemonMoveName::Submission => &PokemonMoveMetadata::SUBMISSION,
            PokemonMoveName::LowKick => &PokemonMoveMetadata::LOW_KICK,
            PokemonMoveName::Counter => &PokemonMoveMetadata::COUNTER,
            PokemonMoveName::SeismicToss => &PokemonMoveMetadata::SEISMIC_TOSS,
            PokemonMoveName::Strength => &PokemonMoveMetadata::STRENGTH,
            PokemonMoveName::Absorb => &PokemonMoveMetadata::ABSORB,
            PokemonMoveName::MegaDrain => &PokemonMoveMetadata::MEGA_DRAIN,
            PokemonMoveName::LeechSeed => &PokemonMoveMetadata::LEECH_SEED,
            PokemonMoveName::Growth => &PokemonMoveMetadata::GROWTH,
            PokemonMoveName::RazorLeaf => &PokemonMoveMetadata::RAZOR_LEAF,
            PokemonMoveName::Solarbeam => &PokemonMoveMetadata::SOLARBEAM,
            PokemonMoveName::Poisonpowder => &PokemonMoveMetadata::POISONPOWDER,
            PokemonMoveName::StunSpore => &PokemonMoveMetadata::STUN_SPORE,
            PokemonMoveName::SleepPowder => &PokemonMoveMetadata::SLEEP_POWDER,
            PokemonMoveName::PetalDance => &PokemonMoveMetadata::PETAL_DANCE,
            PokemonMoveName::StringShot => &PokemonMoveMetadata::STRING_SHOT,
            PokemonMoveName::DragonRage => &PokemonMoveMetadata::DRAGON_RAGE,
            PokemonMoveName::FireSpin => &PokemonMoveMetadata::FIRE_SPIN,
            PokemonMoveName::Thundershock => &PokemonMoveMetadata::THUNDERSHOCK,
            PokemonMoveName::Thunderbolt => &PokemonMoveMetadata::THUNDERBOLT,
            PokemonMoveName::ThunderWave => &PokemonMoveMetadata::THUNDER_WAVE,
            PokemonMoveName::Thunder => &PokemonMoveMetadata::THUNDER,
            PokemonMoveName::RockThrow => &PokemonMoveMetadata::ROCK_THROW,
            PokemonMoveName::Earthquake => &PokemonMoveMetadata::EARTHQUAKE,
            PokemonMoveName::Fissure => &PokemonMoveMetadata::FISSURE,
            PokemonMoveName::Dig => &PokemonMoveMetadata::DIG,
            PokemonMoveName::Toxic => &PokemonMoveMetadata::TOXIC,
            PokemonMoveName::Confusion => &PokemonMoveMetadata::CONFUSION,
            PokemonMoveName::Psychic => &PokemonMoveMetadata::PSYCHIC_M,
            PokemonMoveName::Hypnosis => &PokemonMoveMetadata::HYPNOSIS,
            PokemonMoveName::Meditate => &PokemonMoveMetadata::MEDITATE,
            PokemonMoveName::Agility => &PokemonMoveMetadata::AGILITY,
            PokemonMoveName::QuickAttack => &PokemonMoveMetadata::QUICK_ATTACK,
            PokemonMoveName::Rage => &PokemonMoveMetadata::RAGE,
            PokemonMoveName::Teleport => &PokemonMoveMetadata::TELEPORT,
            PokemonMoveName::NightShade => &PokemonMoveMetadata::NIGHT_SHADE,
            PokemonMoveName::Mimic => &PokemonMoveMetadata::MIMIC,
            PokemonMoveName::Screech => &PokemonMoveMetadata::SCREECH,
            PokemonMoveName::DoubleTeam => &PokemonMoveMetadata::DOUBLE_TEAM,
            PokemonMoveName::Recover => &PokemonMoveMetadata::RECOVER,
            PokemonMoveName::Harden => &PokemonMoveMetadata::HARDEN,
            PokemonMoveName::Minimize => &PokemonMoveMetadata::MINIMIZE,
            PokemonMoveName::Smokescreen => &PokemonMoveMetadata::SMOKESCREEN,
            PokemonMoveName::ConfuseRay => &PokemonMoveMetadata::CONFUSE_RAY,
            PokemonMoveName::Withdraw => &PokemonMoveMetadata::WITHDRAW,
            PokemonMoveName::DefenseCurl => &PokemonMoveMetadata::DEFENSE_CURL,
            PokemonMoveName::Barrier => &PokemonMoveMetadata::BARRIER,
            PokemonMoveName::LightScreen => &PokemonMoveMetadata::LIGHT_SCREEN,
            PokemonMoveName::Haze => &PokemonMoveMetadata::HAZE,
            PokemonMoveName::Reflect => &PokemonMoveMetadata::REFLECT,
            PokemonMoveName::FocusEnergy => &PokemonMoveMetadata::FOCUS_ENERGY,
            PokemonMoveName::Bide => &PokemonMoveMetadata::BIDE,
            PokemonMoveName::Metronome => &PokemonMoveMetadata::METRONOME,
            PokemonMoveName::MirrorMove => &PokemonMoveMetadata::MIRROR_MOVE,
            PokemonMoveName::Selfdestruct => &PokemonMoveMetadata::SELFDESTRUCT,
            PokemonMoveName::EggBomb => &PokemonMoveMetadata::EGG_BOMB,
            PokemonMoveName::Lick => &PokemonMoveMetadata::LICK,
            PokemonMoveName::Smog => &PokemonMoveMetadata::SMOG,
            PokemonMoveName::Sludge => &PokemonMoveMetadata::SLUDGE,
            PokemonMoveName::BoneClub => &PokemonMoveMetadata::BONE_CLUB,
            PokemonMoveName::FireBlast => &PokemonMoveMetadata::FIRE_BLAST,
            PokemonMoveName::Waterfall => &PokemonMoveMetadata::WATERFALL,
            PokemonMoveName::Clamp => &PokemonMoveMetadata::CLAMP,
            PokemonMoveName::Swift => &PokemonMoveMetadata::SWIFT,
            PokemonMoveName::SkullBash => &PokemonMoveMetadata::SKULL_BASH,
            PokemonMoveName::SpikeCannon => &PokemonMoveMetadata::SPIKE_CANNON,
            PokemonMoveName::Constrict => &PokemonMoveMetadata::CONSTRICT,
            PokemonMoveName::Amnesia => &PokemonMoveMetadata::AMNESIA,
            PokemonMoveName::Kinesis => &PokemonMoveMetadata::KINESIS,
            PokemonMoveName::Softboiled => &PokemonMoveMetadata::SOFTBOILED,
            PokemonMoveName::HiJumpKick => &PokemonMoveMetadata::HI_JUMP_KICK,
            PokemonMoveName::Glare => &PokemonMoveMetadata::GLARE,
            PokemonMoveName::DreamEater => &PokemonMoveMetadata::DREAM_EATER,
            PokemonMoveName::PoisonGas => &PokemonMoveMetadata::POISON_GAS,
            PokemonMoveName::Barrage => &PokemonMoveMetadata::BARRAGE,
            PokemonMoveName::LeechLife => &PokemonMoveMetadata::LEECH_LIFE,
            PokemonMoveName::LovelyKiss => &PokemonMoveMetadata::LOVELY_KISS,
            PokemonMoveName::SkyAttack => &PokemonMoveMetadata::SKY_ATTACK,
            PokemonMoveName::Transform => &PokemonMoveMetadata::TRANSFORM,
            PokemonMoveName::Bubble => &PokemonMoveMetadata::BUBBLE,
            PokemonMoveName::DizzyPunch => &PokemonMoveMetadata::DIZZY_PUNCH,
            PokemonMoveName::Spore => &PokemonMoveMetadata::SPORE,
            PokemonMoveName::Flash => &PokemonMoveMetadata::FLASH,
            PokemonMoveName::Psywave => &PokemonMoveMetadata::PSYWAVE,
            PokemonMoveName::Splash => &PokemonMoveMetadata::SPLASH,
            PokemonMoveName::AcidArmor => &PokemonMoveMetadata::ACID_ARMOR,
            PokemonMoveName::Crabhammer => &PokemonMoveMetadata::CRABHAMMER,
            PokemonMoveName::Explosion => &PokemonMoveMetadata::EXPLOSION,
            PokemonMoveName::FurySwipes => &PokemonMoveMetadata::FURY_SWIPES,
            PokemonMoveName::Bonemerang => &PokemonMoveMetadata::BONEMERANG,
            PokemonMoveName::Rest => &PokemonMoveMetadata::REST,
            PokemonMoveName::RockSlide => &PokemonMoveMetadata::ROCK_SLIDE,
            PokemonMoveName::HyperFang => &PokemonMoveMetadata::HYPER_FANG,
            PokemonMoveName::Sharpen => &PokemonMoveMetadata::SHARPEN,
            PokemonMoveName::Conversion => &PokemonMoveMetadata::CONVERSION,
            PokemonMoveName::TriAttack => &PokemonMoveMetadata::TRI_ATTACK,
            PokemonMoveName::SuperFang => &PokemonMoveMetadata::SUPER_FANG,
            PokemonMoveName::Slash => &PokemonMoveMetadata::SLASH,
            PokemonMoveName::Substitute => &PokemonMoveMetadata::SUBSTITUTE,
            PokemonMoveName::Struggle => &PokemonMoveMetadata::STRUGGLE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, strum_macros::Display)]
#[repr(u8)]
pub enum PokemonMoveEffect {
    #[strum(serialize = "No Additional Effect")] NoAdditionalEffect = 0x00,
    #[strum(serialize = "Poison Side 1")] PoisonSide1 = 0x02,
    #[strum(serialize = "Drain Hp")] DrainHp = 0x03,
    #[strum(serialize = "Burn Side 1")] BurnSide1 = 0x04,
    #[strum(serialize = "Freeze Side 1")] FreezeSide1 = 0x05,
    #[strum(serialize = "Paralyze Side 1")] ParalyzeSide1 = 0x06,
    Explode = 0x07,
    #[strum(serialize = "Dream Eater")] DreamEater = 0x08,
    #[strum(serialize = "Mirror Move")] MirrorMove = 0x09,
    #[strum(serialize = "Attack Up 1")] AttackUp1 = 0x0A,
    #[strum(serialize = "Defense Up 1")] DefenseUp1 = 0x0B,
    #[strum(serialize = "Speed Up 1")] SpeedUp1 = 0x0C,
    #[strum(serialize = "Special Up 1")] SpecialUp1 = 0x0D,
    #[strum(serialize = "Accuracy Up 1")] AccuracyUp1 = 0x0E,
    #[strum(serialize = "Evasion Up 1")] EvasionUp1 = 0x0F,
    #[strum(serialize = "Pay Day")] PayDay = 0x10,
    Swift = 0x11,
    #[strum(serialize = "Attack Down 1")] AttackDown1 = 0x12,
    #[strum(serialize = "Defense Down 1")] DefenseDown1 = 0x13,
    #[strum(serialize = "Speed Down 1")] SpeedDown1 = 0x14,
    #[strum(serialize = "Special Down 1")] SpecialDown1 = 0x15,
    #[strum(serialize = "Accuracy Down 1")] AccuracyDown1 = 0x16,
    #[strum(serialize = "Evasion Down 1")] EvasionDown1 = 0x17,
    Conversion = 0x18,
    Haze = 0x19,
    Bide = 0x1A,
    #[strum(serialize = "Thrash Petal Dance")] ThrashPetalDance = 0x1B,
    #[strum(serialize = "Switch And Teleport")] SwitchAndTeleport = 0x1C,
    #[strum(serialize = "Two To Five Attacks")] TwoToFiveAttacks = 0x1D,
    #[strum(serialize = "Flinch Side 1")] FlinchSide1 = 0x1F,
    Sleep = 0x20,
    #[strum(serialize = "Poison Side 2")] PoisonSide2 = 0x21,
    #[strum(serialize = "Burn Side 2")] BurnSide2 = 0x22,
    #[strum(serialize = "Paralyze Side 2")] ParalyzeSide2 = 0x24,
    #[strum(serialize = "Flinch Side 2")] FlinchSide2 = 0x25,
    #[strum(serialize = "One Hit KO")] OneHitKO = 0x26,
    Charge = 0x27,
    #[strum(serialize = "Super Fang")] SuperFang = 0x28,
    #[strum(serialize = "Special Damage")] SpecialDamage = 0x29,
    Trapping = 0x2A,
    Fly = 0x2B,
    #[strum(serialize = "Attack Twice")] AttackTwice = 0x2C,
    #[strum(serialize = "Jump Kick")] JumpKick = 0x2D,
    Mist = 0x2E,
    #[strum(serialize = "Focus Energy")] FocusEnergy = 0x2F,
    Recoil = 0x30,
    Confusion = 0x31,
    #[strum(serialize = "Attack Up 2")] AttackUp2 = 0x32,
    #[strum(serialize = "Defense Up 2")] DefenseUp2 = 0x33,
    #[strum(serialize = "Speed Up 2")] SpeedUp2 = 0x34,
    #[strum(serialize = "Special Up 2")] SpecialUp2 = 0x35,
    #[strum(serialize = "Accuracy Up 2")] AccuracyUp2 = 0x36,
    #[strum(serialize = "Evasion Up 2")] EvasionUp2 = 0x37,
    Heal = 0x38,
    Transform = 0x39,
    #[strum(serialize = "Attack Down 2")] AttackDown2 = 0x3A,
    #[strum(serialize = "Defense Down 2")] DefenseDown2 = 0x3B,
    #[strum(serialize = "Speed Down 2")] SpeedDown2 = 0x3C,
    #[strum(serialize = "Special Down 2")] SpecialDown2 = 0x3D,
    #[strum(serialize = "Accuracy Down 2")] AccuracyDown2 = 0x3E,
    #[strum(serialize = "Evasion Down 2")] EvasionDown2 = 0x3F,
    #[strum(serialize = "Light Screen")] LightScreen = 0x40,
    Reflect = 0x41,
    Poison = 0x42,
    Paralyze = 0x43,
    #[strum(serialize = "Attack Down Side")] AttackDownSide = 0x44,
    #[strum(serialize = "Defense Down Side")] DefenseDownSide = 0x45,
    #[strum(serialize = "Speed Down Side")] SpeedDownSide = 0x46,
    #[strum(serialize = "Special Down Side")] SpecialDownSide = 0x47,
    #[strum(serialize = "Confusion Side")] ConfusionSide = 0x4C,
    Twineedle = 0x4D,
    Substitute = 0x4F,
    #[strum(serialize = "Hyper Beam")] HyperBeam = 0x50,
    Rage = 0x51,
    Mimic = 0x52,
    Metronome = 0x53,
    #[strum(serialize = "Leech Seed")] LeechSeed = 0x54,
    Splash = 0x55,
    Disable = 0x56,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PokemonMoveMetadata {
    pub name: &'static str,
    pub move_type: PokemonType,
    pub effect: PokemonMoveEffect,
    pub power: Option<u8>,
    pub accuracy: u8,
    pub pp: u8,
}



impl PokemonMoveMetadata {
    pub const fn new(name: &'static str, move_type: PokemonType, effect: PokemonMoveEffect, power: Option<u8>, accuracy: u8, pp: u8) -> Self {
        assert!(accuracy <= 100);
        assert!(pp <= 40);
        Self { name, move_type, effect, power, accuracy, pp }
    }

    pub const POUND: Self = Self::new("Pound", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(40), 100, 35);
    pub const KARATE_CHOP: Self = Self::new("Karate Chop", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(50), 100, 25);
    pub const DOUBLESLAP: Self = Self::new("Doubleslap", Normal, PokemonMoveEffect::TwoToFiveAttacks, Some(15), 85, 10);
    pub const COMET_PUNCH: Self = Self::new("Comet Punch", Normal, PokemonMoveEffect::TwoToFiveAttacks, Some(18), 85, 15);
    pub const MEGA_PUNCH: Self = Self::new("Mega Punch", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(80), 85, 20);
    pub const PAY_DAY: Self = Self::new("Pay Day", Normal, PokemonMoveEffect::PayDay, Some(40), 100, 20);
    pub const FIRE_PUNCH: Self = Self::new("Fire Punch", Fire, PokemonMoveEffect::BurnSide1, Some(75), 100, 15);
    pub const ICE_PUNCH: Self = Self::new("Ice Punch", Ice, PokemonMoveEffect::FreezeSide1, Some(75), 100, 15);
    pub const THUNDERPUNCH: Self = Self::new("Thunderpunch", Electric, PokemonMoveEffect::ParalyzeSide1, Some(75), 100, 15);
    pub const SCRATCH: Self = Self::new("Scratch", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(40), 100, 35);
    pub const VICEGRIP: Self = Self::new("Vicegrip", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(55), 100, 30);
    pub const GUILLOTINE: Self = Self::new("Guillotine", Normal, PokemonMoveEffect::OneHitKO, None, 30, 5);
    pub const RAZOR_WIND: Self = Self::new("Razor Wind", Normal, PokemonMoveEffect::Charge, Some(80), 75, 10);
    pub const SWORDS_DANCE: Self = Self::new("Swords Dance", Normal, PokemonMoveEffect::AttackUp2, None, 100, 30);
    pub const CUT: Self = Self::new("Cut", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(50), 95, 30);
    pub const GUST: Self = Self::new("Gust", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(40), 100, 35);
    pub const WING_ATTACK: Self = Self::new("Wing Attack", Flying, PokemonMoveEffect::NoAdditionalEffect, Some(35), 100, 35);
    pub const WHIRLWIND: Self = Self::new("Whirlwind", Normal, PokemonMoveEffect::SwitchAndTeleport, None, 85, 20);
    pub const FLY: Self = Self::new("Fly", Flying, PokemonMoveEffect::Fly, Some(70), 95, 15);
    pub const BIND: Self = Self::new("Bind", Normal, PokemonMoveEffect::Trapping, Some(15), 75, 20);
    pub const SLAM: Self = Self::new("Slam", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(80), 75, 20);
    pub const VINE_WHIP: Self = Self::new("Vine Whip", Grass, PokemonMoveEffect::NoAdditionalEffect, Some(35), 100, 10);
    pub const STOMP: Self = Self::new("Stomp", Normal, PokemonMoveEffect::FlinchSide2, Some(65), 100, 20);
    pub const DOUBLE_KICK: Self = Self::new("Double Kick", Fighting, PokemonMoveEffect::AttackTwice, Some(30), 100, 30);
    pub const MEGA_KICK: Self = Self::new("Mega Kick", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(120), 75, 5);
    pub const JUMP_KICK: Self = Self::new("Jump Kick", Fighting, PokemonMoveEffect::JumpKick, Some(70), 95, 25);
    pub const ROLLING_KICK: Self = Self::new("Rolling Kick", Fighting, PokemonMoveEffect::FlinchSide2, Some(60), 85, 15);
    pub const SAND_ATTACK: Self = Self::new("Sand Attack", Normal, PokemonMoveEffect::AccuracyDown1, None, 100, 15);
    pub const HEADBUTT: Self = Self::new("Headbutt", Normal, PokemonMoveEffect::FlinchSide2, Some(70), 100, 15);
    pub const HORN_ATTACK: Self = Self::new("Horn Attack", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(65), 100, 25);
    pub const FURY_ATTACK: Self = Self::new("Fury Attack", Normal, PokemonMoveEffect::TwoToFiveAttacks, Some(15), 85, 20);
    pub const HORN_DRILL: Self = Self::new("Horn Drill", Normal, PokemonMoveEffect::OneHitKO, None, 30, 5);
    pub const TACKLE: Self = Self::new("Tackle", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(35), 95, 35);
    pub const BODY_SLAM: Self = Self::new("Body Slam", Normal, PokemonMoveEffect::ParalyzeSide2, Some(85), 100, 15);
    pub const WRAP: Self = Self::new("Wrap", Normal, PokemonMoveEffect::Trapping, Some(15), 85, 20);
    pub const TAKE_DOWN: Self = Self::new("Take Down", Normal, PokemonMoveEffect::Recoil, Some(90), 85, 20);
    pub const THRASH: Self = Self::new("Thrash", Normal, PokemonMoveEffect::ThrashPetalDance, Some(90), 100, 20);
    pub const DOUBLE_EDGE: Self = Self::new("Double Edge", Normal, PokemonMoveEffect::Recoil, Some(100), 100, 15);
    pub const TAIL_WHIP: Self = Self::new("Tail Whip", Normal, PokemonMoveEffect::DefenseDown1, None, 100, 30);
    pub const POISON_STING: Self = Self::new("Poison Sting", Poison, PokemonMoveEffect::PoisonSide1, Some(15), 100, 35);
    pub const TWINEEDLE: Self = Self::new("Twineedle", Bug, PokemonMoveEffect::Twineedle, Some(25), 100, 20);
    pub const PIN_MISSILE: Self = Self::new("Pin Missile", Bug, PokemonMoveEffect::TwoToFiveAttacks, Some(14), 85, 20);
    pub const LEER: Self = Self::new("Leer", Normal, PokemonMoveEffect::DefenseDown1, None, 100, 30);
    pub const BITE: Self = Self::new("Bite", Normal, PokemonMoveEffect::FlinchSide1, Some(60), 100, 25);
    pub const GROWL: Self = Self::new("Growl", Normal, PokemonMoveEffect::AttackDown1, None, 100, 40);
    pub const ROAR: Self = Self::new("Roar", Normal, PokemonMoveEffect::SwitchAndTeleport, None, 100, 20);
    pub const SING: Self = Self::new("Sing", Normal, PokemonMoveEffect::Sleep, None, 55, 15);
    pub const SUPERSONIC: Self = Self::new("Supersonic", Normal, PokemonMoveEffect::Confusion, None, 55, 20);
    pub const SONICBOOM: Self = Self::new("Sonicboom", Normal, PokemonMoveEffect::SpecialDamage, None, 90, 20);
    pub const DISABLE: Self = Self::new("Disable", Normal, PokemonMoveEffect::Disable, None, 55, 20);
    pub const ACID: Self = Self::new("Acid", Poison, PokemonMoveEffect::DefenseDownSide, Some(40), 100, 30);
    pub const EMBER: Self = Self::new("Ember", Fire, PokemonMoveEffect::BurnSide1, Some(40), 100, 25);
    pub const FLAMETHROWER: Self = Self::new("Flamethrower", Fire, PokemonMoveEffect::BurnSide1, Some(95), 100, 15);
    pub const MIST: Self = Self::new("Mist", Ice, PokemonMoveEffect::Mist, None, 100, 30);
    pub const WATER_GUN: Self = Self::new("Water Gun", Water, PokemonMoveEffect::NoAdditionalEffect, Some(40), 100, 25);
    pub const HYDRO_PUMP: Self = Self::new("Hydro Pump", Water, PokemonMoveEffect::NoAdditionalEffect, Some(120), 80, 5);
    pub const SURF: Self = Self::new("Surf", Water, PokemonMoveEffect::NoAdditionalEffect, Some(95), 100, 15);
    pub const ICE_BEAM: Self = Self::new("Ice Beam", Ice, PokemonMoveEffect::FreezeSide1, Some(95), 100, 10);
    pub const BLIZZARD: Self = Self::new("Blizzard", Ice, PokemonMoveEffect::FreezeSide1, Some(120), 90, 5);
    pub const PSYBEAM: Self = Self::new("Psybeam", Psychic, PokemonMoveEffect::ConfusionSide, Some(65), 100, 20);
    pub const BUBBLEBEAM: Self = Self::new("Bubblebeam", Water, PokemonMoveEffect::SpeedDownSide, Some(65), 100, 20);
    pub const AURORA_BEAM: Self = Self::new("Aurora Beam", Ice, PokemonMoveEffect::AttackDownSide, Some(65), 100, 20);
    pub const HYPER_BEAM: Self = Self::new("Hyper Beam", Normal, PokemonMoveEffect::HyperBeam, Some(150), 90, 5);
    pub const PECK: Self = Self::new("Peck", Flying, PokemonMoveEffect::NoAdditionalEffect, Some(35), 100, 35);
    pub const DRILL_PECK: Self = Self::new("Drill Peck", Flying, PokemonMoveEffect::NoAdditionalEffect, Some(80), 100, 20);
    pub const SUBMISSION: Self = Self::new("Submission", Fighting, PokemonMoveEffect::Recoil, Some(80), 80, 25);
    pub const LOW_KICK: Self = Self::new("Low Kick", Fighting, PokemonMoveEffect::FlinchSide2, Some(50), 90, 20);
    pub const COUNTER: Self = Self::new("Counter", Fighting, PokemonMoveEffect::NoAdditionalEffect, None, 100, 20);
    pub const SEISMIC_TOSS: Self = Self::new("Seismic Toss", Fighting, PokemonMoveEffect::SpecialDamage, None, 100, 20);
    pub const STRENGTH: Self = Self::new("Strength", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(80), 100, 15);
    pub const ABSORB: Self = Self::new("Absorb", Grass, PokemonMoveEffect::DrainHp, Some(20), 100, 20);
    pub const MEGA_DRAIN: Self = Self::new("Mega Drain", Grass, PokemonMoveEffect::DrainHp, Some(40), 100, 10);
    pub const LEECH_SEED: Self = Self::new("Leech Seed", Grass, PokemonMoveEffect::LeechSeed, None, 90, 10);
    pub const GROWTH: Self = Self::new("Growth", Normal, PokemonMoveEffect::SpecialUp1, None, 100, 40);
    pub const RAZOR_LEAF: Self = Self::new("Razor Leaf", Grass, PokemonMoveEffect::NoAdditionalEffect, Some(55), 95, 25);
    pub const SOLARBEAM: Self = Self::new("Solarbeam", Grass, PokemonMoveEffect::Charge, Some(120), 100, 10);
    pub const POISONPOWDER: Self = Self::new("Poisonpowder", Poison, PokemonMoveEffect::Poison, None, 75, 35);
    pub const STUN_SPORE: Self = Self::new("Stun Spore", Grass, PokemonMoveEffect::Paralyze, None, 75, 30);
    pub const SLEEP_POWDER: Self = Self::new("Sleep Powder", Grass, PokemonMoveEffect::Sleep, None, 75, 15);
    pub const PETAL_DANCE: Self = Self::new("Petal Dance", Grass, PokemonMoveEffect::ThrashPetalDance, Some(70), 100, 20);
    pub const STRING_SHOT: Self = Self::new("String Shot", Bug, PokemonMoveEffect::SpeedDown1, None, 95, 40);
    pub const DRAGON_RAGE: Self = Self::new("Dragon Rage", Dragon, PokemonMoveEffect::SpecialDamage, None, 100, 10);
    pub const FIRE_SPIN: Self = Self::new("Fire Spin", Fire, PokemonMoveEffect::Trapping, Some(15), 70, 15);
    pub const THUNDERSHOCK: Self = Self::new("Thundershock", Electric, PokemonMoveEffect::ParalyzeSide1, Some(40), 100, 30);
    pub const THUNDERBOLT: Self = Self::new("Thunderbolt", Electric, PokemonMoveEffect::ParalyzeSide1, Some(95), 100, 15);
    pub const THUNDER_WAVE: Self = Self::new("Thunder Wave", Electric, PokemonMoveEffect::Paralyze, None, 100, 20);
    pub const THUNDER: Self = Self::new("Thunder", Electric, PokemonMoveEffect::ParalyzeSide1, Some(120), 70, 10);
    pub const ROCK_THROW: Self = Self::new("Rock Throw", Rock, PokemonMoveEffect::NoAdditionalEffect, Some(50), 65, 15);
    pub const EARTHQUAKE: Self = Self::new("Earthquake", Ground, PokemonMoveEffect::NoAdditionalEffect, Some(100), 100, 10);
    pub const FISSURE: Self = Self::new("Fissure", Ground, PokemonMoveEffect::OneHitKO, None, 30, 5);
    pub const DIG: Self = Self::new("Dig", Ground, PokemonMoveEffect::Charge, Some(100), 100, 10);
    pub const TOXIC: Self = Self::new("Toxic", Poison, PokemonMoveEffect::Poison, None, 85, 10);
    pub const CONFUSION: Self = Self::new("Confusion", Psychic, PokemonMoveEffect::ConfusionSide, Some(50), 100, 25);
    pub const PSYCHIC_M: Self = Self::new("Psychic M", Psychic, PokemonMoveEffect::SpecialDownSide, Some(90), 100, 10);
    pub const HYPNOSIS: Self = Self::new("Hypnosis", Psychic, PokemonMoveEffect::Sleep, None, 60, 20);
    pub const MEDITATE: Self = Self::new("Meditate", Psychic, PokemonMoveEffect::AttackUp1, None, 100, 40);
    pub const AGILITY: Self = Self::new("Agility", Psychic, PokemonMoveEffect::SpeedUp2, None, 100, 30);
    pub const QUICK_ATTACK: Self = Self::new("Quick Attack", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(40), 100, 30);
    pub const RAGE: Self = Self::new("Rage", Normal, PokemonMoveEffect::Rage, Some(20), 100, 20);
    pub const TELEPORT: Self = Self::new("Teleport", Psychic, PokemonMoveEffect::SwitchAndTeleport, None, 100, 20);
    pub const NIGHT_SHADE: Self = Self::new("Night Shade", Ghost, PokemonMoveEffect::SpecialDamage, None, 100, 15);
    pub const MIMIC: Self = Self::new("Mimic", Normal, PokemonMoveEffect::Mimic, None, 100, 10);
    pub const SCREECH: Self = Self::new("Screech", Normal, PokemonMoveEffect::DefenseDown2, None, 85, 40);
    pub const DOUBLE_TEAM: Self = Self::new("Double Team", Normal, PokemonMoveEffect::EvasionUp1, None, 100, 15);
    pub const RECOVER: Self = Self::new("Recover", Normal, PokemonMoveEffect::Heal, None, 100, 20);
    pub const HARDEN: Self = Self::new("Harden", Normal, PokemonMoveEffect::DefenseUp1, None, 100, 30);
    pub const MINIMIZE: Self = Self::new("Minimize", Normal, PokemonMoveEffect::EvasionUp1, None, 100, 20);
    pub const SMOKESCREEN: Self = Self::new("Smokescreen", Normal, PokemonMoveEffect::AccuracyDown1, None, 100, 20);
    pub const CONFUSE_RAY: Self = Self::new("Confuse Ray", Ghost, PokemonMoveEffect::Confusion, None, 100, 10);
    pub const WITHDRAW: Self = Self::new("Withdraw", Water, PokemonMoveEffect::DefenseUp1, None, 100, 40);
    pub const DEFENSE_CURL: Self = Self::new("Defense Curl", Normal, PokemonMoveEffect::DefenseUp1, None, 100, 40);
    pub const BARRIER: Self = Self::new("Barrier", Psychic, PokemonMoveEffect::DefenseUp2, None, 100, 30);
    pub const LIGHT_SCREEN: Self = Self::new("Light Screen", Psychic, PokemonMoveEffect::LightScreen, None, 100, 30);
    pub const HAZE: Self = Self::new("Haze", Ice, PokemonMoveEffect::Haze, None, 100, 30);
    pub const REFLECT: Self = Self::new("Reflect", Psychic, PokemonMoveEffect::Reflect, None, 100, 20);
    pub const FOCUS_ENERGY: Self = Self::new("Focus Energy", Normal, PokemonMoveEffect::FocusEnergy, None, 100, 30);
    pub const BIDE: Self = Self::new("Bide", Normal, PokemonMoveEffect::Bide, None, 100, 10);
    pub const METRONOME: Self = Self::new("Metronome", Normal, PokemonMoveEffect::Metronome, None, 100, 10);
    pub const MIRROR_MOVE: Self = Self::new("Mirror Move", Flying, PokemonMoveEffect::MirrorMove, None, 100, 20);
    pub const SELFDESTRUCT: Self = Self::new("Selfdestruct", Normal, PokemonMoveEffect::Explode, Some(130), 100, 5);
    pub const EGG_BOMB: Self = Self::new("Egg Bomb", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(100), 75, 10);
    pub const LICK: Self = Self::new("Lick", Ghost, PokemonMoveEffect::ParalyzeSide2, Some(20), 100, 30);
    pub const SMOG: Self = Self::new("Smog", Poison, PokemonMoveEffect::PoisonSide2, Some(20), 70, 20);
    pub const SLUDGE: Self = Self::new("Sludge", Poison, PokemonMoveEffect::PoisonSide2, Some(65), 100, 20);
    pub const BONE_CLUB: Self = Self::new("Bone Club", Ground, PokemonMoveEffect::FlinchSide1, Some(65), 85, 20);
    pub const FIRE_BLAST: Self = Self::new("Fire Blast", Fire, PokemonMoveEffect::BurnSide2, Some(120), 85, 5);
    pub const WATERFALL: Self = Self::new("Waterfall", Water, PokemonMoveEffect::NoAdditionalEffect, Some(80), 100, 15);
    pub const CLAMP: Self = Self::new("Clamp", Water, PokemonMoveEffect::Trapping, Some(35), 75, 10);
    pub const SWIFT: Self = Self::new("Swift", Normal, PokemonMoveEffect::Swift, Some(60), 100, 20);
    pub const SKULL_BASH: Self = Self::new("Skull Bash", Normal, PokemonMoveEffect::Charge, Some(100), 100, 15);
    pub const SPIKE_CANNON: Self = Self::new("Spike Cannon", Normal, PokemonMoveEffect::TwoToFiveAttacks, Some(20), 100, 15);
    pub const CONSTRICT: Self = Self::new("Constrict", Normal, PokemonMoveEffect::SpeedDownSide, Some(10), 100, 35);
    pub const AMNESIA: Self = Self::new("Amnesia", Psychic, PokemonMoveEffect::SpecialUp2, None, 100, 20);
    pub const KINESIS: Self = Self::new("Kinesis", Psychic, PokemonMoveEffect::AccuracyDown1, None, 80, 15);
    pub const SOFTBOILED: Self = Self::new("Softboiled", Normal, PokemonMoveEffect::Heal, None, 100, 10);
    pub const HI_JUMP_KICK: Self = Self::new("Hi Jump Kick", Fighting, PokemonMoveEffect::JumpKick, Some(85), 90, 20);
    pub const GLARE: Self = Self::new("Glare", Normal, PokemonMoveEffect::Paralyze, None, 75, 30);
    pub const DREAM_EATER: Self = Self::new("Dream Eater", Psychic, PokemonMoveEffect::DreamEater, Some(100), 100, 15);
    pub const POISON_GAS: Self = Self::new("Poison Gas", Poison, PokemonMoveEffect::Poison, None, 55, 40);
    pub const BARRAGE: Self = Self::new("Barrage", Normal, PokemonMoveEffect::TwoToFiveAttacks, Some(15), 85, 20);
    pub const LEECH_LIFE: Self = Self::new("Leech Life", Bug, PokemonMoveEffect::DrainHp, Some(20), 100, 15);
    pub const LOVELY_KISS: Self = Self::new("Lovely Kiss", Normal, PokemonMoveEffect::Sleep, None, 75, 10);
    pub const SKY_ATTACK: Self = Self::new("Sky Attack", Flying, PokemonMoveEffect::Charge, Some(140), 90, 5);
    pub const TRANSFORM: Self = Self::new("Transform", Normal, PokemonMoveEffect::Transform, None, 100, 10);
    pub const BUBBLE: Self = Self::new("Bubble", Water, PokemonMoveEffect::SpeedDownSide, Some(20), 100, 30);
    pub const DIZZY_PUNCH: Self = Self::new("Dizzy Punch", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(70), 100, 10);
    pub const SPORE: Self = Self::new("Spore", Grass, PokemonMoveEffect::Sleep, None, 100, 15);
    pub const FLASH: Self = Self::new("Flash", Normal, PokemonMoveEffect::AccuracyDown1, None, 70, 20);
    pub const PSYWAVE: Self = Self::new("Psywave", Psychic, PokemonMoveEffect::SpecialDamage, None, 80, 15);
    pub const SPLASH: Self = Self::new("Splash", Normal, PokemonMoveEffect::Splash, None, 100, 40);
    pub const ACID_ARMOR: Self = Self::new("Acid Armor", Poison, PokemonMoveEffect::DefenseUp2, None, 100, 40);
    pub const CRABHAMMER: Self = Self::new("Crabhammer", Water, PokemonMoveEffect::NoAdditionalEffect, Some(90), 85, 10);
    pub const EXPLOSION: Self = Self::new("Explosion", Normal, PokemonMoveEffect::Explode, Some(170), 100, 5);
    pub const FURY_SWIPES: Self = Self::new("Fury Swipes", Normal, PokemonMoveEffect::TwoToFiveAttacks, Some(18), 80, 15);
    pub const BONEMERANG: Self = Self::new("Bonemerang", Ground, PokemonMoveEffect::AttackTwice, Some(50), 90, 10);
    pub const REST: Self = Self::new("Rest", Psychic, PokemonMoveEffect::Heal, None, 100, 10);
    pub const ROCK_SLIDE: Self = Self::new("Rock Slide", Rock, PokemonMoveEffect::NoAdditionalEffect, Some(75), 90, 10);
    pub const HYPER_FANG: Self = Self::new("Hyper Fang", Normal, PokemonMoveEffect::FlinchSide1, Some(80), 90, 15);
    pub const SHARPEN: Self = Self::new("Sharpen", Normal, PokemonMoveEffect::AttackUp1, None, 100, 30);
    pub const CONVERSION: Self = Self::new("Conversion", Normal, PokemonMoveEffect::Conversion, None, 100, 30);
    pub const TRI_ATTACK: Self = Self::new("Tri Attack", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(80), 100, 10);
    pub const SUPER_FANG: Self = Self::new("Super Fang", Normal, PokemonMoveEffect::SuperFang, None, 90, 10);
    pub const SLASH: Self = Self::new("Slash", Normal, PokemonMoveEffect::NoAdditionalEffect, Some(70), 100, 20);
    pub const SUBSTITUTE: Self = Self::new("Substitute", Normal, PokemonMoveEffect::Substitute, None, 100, 10);
    pub const STRUGGLE: Self = Self::new("Struggle", Normal, PokemonMoveEffect::Recoil, Some(50), 100, 10);

}