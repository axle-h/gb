use crate::geometry::Point8;
use crate::pokemon::map::MapSprite;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Sprite {
    pub index: u8,
    pub picture_id: PictureId,
    pub position: Point8,
    pub on_screen: bool,
    pub hidden: bool,
    pub facing: SpriteFacing,
    pub name: &'static str
}

/// Which way an overworld sprite is facing — `wSpriteStateData1 + 9`.
///
/// ⚠️ **This encoding is the sprite byte's own — `$0` down, `$4` up, `$8` left, `$C` right
/// (`pokered/ram/wram.asm:96`) — and it is *not*
/// [`PlayerFacingDirection`](crate::pokemon::map_metadata::PlayerFacingDirection)'s.** That is a bit
/// mask (`Up = 8, Down = 4, Left = 2, Right = 1`) living on a different byte, `wPlayerDirection`.
/// The two collide on `4` and `8`, where they mean opposite things, so reading one with the other's
/// table points half the people on a map the wrong way and nothing anywhere fails.
///
/// The value doubles as the row index into pokered's `SpriteFacingAndAnimationTable`, whose rows are
/// four bytes (`dw tiles, dw oam`) — so `facing as usize * 4` lands on the standing frame's entry.
#[derive(Debug, Copy, Clone, Default, Eq, PartialEq, strum_macros::Display, strum_macros::FromRepr)]
#[repr(u8)]
pub enum SpriteFacing {
    #[default]
    Down = 0x0,
    Up = 0x4,
    Left = 0x8,
    Right = 0xC,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, strum_macros::FromRepr)]
#[repr(u8)]
pub enum PictureId {
    Red = 0x01,
    Blue = 0x02,
    Oak = 0x03,
    Youngster = 0x04,
    Monster = 0x05,
    CoolTrainerFemale = 0x06,
    CoolTrainerMale = 0x07,
    LittleGirl = 0x08,
    Bird = 0x09,
    MiddleAgedMan = 0x0a,
    Gambler = 0x0b,
    SuperNerd = 0x0c,
    Girl = 0x0d,
    Hiker = 0x0e,
    Beauty = 0x0f,
    Gentleman = 0x10,
    Daisy = 0x11,
    Biker = 0x12,
    Sailor = 0x13,
    Cook = 0x14,
    BikeShopClerk = 0x15,
    MrFuji = 0x16,
    Giovanni = 0x17,
    Rocket = 0x18,
    Channeler = 0x19,
    Waiter = 0x1a,
    SilphWorkerFemale = 0x1b,
    MiddleAgedWoman = 0x1c,
    BrunetteGirl = 0x1d,
    Lance = 0x1e,
    UnusedScientist = 0x1f,
    Scientist = 0x20,
    Rocker = 0x21,
    Swimmer = 0x22,
    SafariZoneWorker = 0x23,
    GymGuide = 0x24,
    Gramps = 0x25,
    Clerk = 0x26,
    FishingGuru = 0x27,
    Granny = 0x28,
    Nurse = 0x29,
    LinkReceptionist = 0x2a,
    SilphPresident = 0x2b,
    SilphWorkerMale = 0x2c,
    Warden = 0x2d,
    Captain = 0x2e,
    Fisher = 0x2f,
    Koga = 0x30,
    Guard = 0x31,
    UnusedGuard = 0x32,
    Mom = 0x33,
    BaldingGuy = 0x34,
    LittleBoy = 0x35,
    UnusedGameBoyKid = 0x36,
    GameBoyKid = 0x37,
    Fairy = 0x38,
    Agatha = 0x39,
    Bruno = 0x3a,
    Lorelei = 0x3b,
    Seel = 0x3c,
    PokeBall = 0x3d,
    Fossil = 0x3e,
    Boulder = 0x3f,
    Paper = 0x40,
    Pokedex = 0x41,
    Clipboard = 0x42,
    Snorlax = 0x43,
    UnusedOldAmber = 0x44,
    OldAmber = 0x45,
    UnusedGamblerAsleep1 = 0x46,
    UnusedGamblerAsleep2 = 0x47,
    GamblerAsleep = 0x48,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::map_metadata::PlayerFacingDirection;

    /// The one thing about [`SpriteFacing`] that is not self-evident, pinned against the line of
    /// pokered that says so: `ram/wram.asm:96`, "facing direction ($0: down, $4: up, $8: left,
    /// $c: right)".
    ///
    /// ⚠️ It is held here rather than left to the reader because the *other* facing type in this
    /// crate disagrees with it on both of the values they share, and a swap is silent: the map still
    /// renders, the people on it are just looking the wrong way.
    #[test]
    fn sprite_facing_is_the_sprite_bytes_encoding_and_not_the_players() {
        assert_eq!(
            [SpriteFacing::Down as u8, SpriteFacing::Up as u8,
             SpriteFacing::Left as u8, SpriteFacing::Right as u8],
            [0x0, 0x4, 0x8, 0xC],
            "wSpriteStateData1 + 9, per pokered/ram/wram.asm:96");

        // The collision that makes the mix-up worth a test: `4` and `8` are legal in both encodings
        // and mean different things in each.
        assert_eq!(PlayerFacingDirection::Down as u8, SpriteFacing::Up as u8);
        assert_eq!(PlayerFacingDirection::Up as u8, SpriteFacing::Left as u8);

        // Every value the table indexes round-trips, and nothing between them does — the byte is a
        // row index into `SpriteFacingAndAnimationTable`, whose rows are four bytes wide.
        for facing in [SpriteFacing::Down, SpriteFacing::Up, SpriteFacing::Left, SpriteFacing::Right] {
            assert_eq!(SpriteFacing::from_repr(facing as u8), Some(facing));
        }
        for stride in [1u8, 2, 3, 5, 6, 7, 0xD, 0xF] {
            assert_eq!(SpriteFacing::from_repr(stride), None, "${stride:X} is not a facing");
        }
    }
}