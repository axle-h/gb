use crate::geometry::Point8;

#[derive(Debug, Copy, Clone)]
pub struct Sprite {
    pub index: u8,
    pub picture_id: u8,
    pub movement_status: SpriteMovementStatus,
    pub position: Point8,
    pub screen_position: Point8,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, strum_macros::FromRepr)]
#[repr(u8)]
pub enum SpriteMovementStatus {
    Uninitialised = 0,
    Ready,
    Delayed,
    Moving
}