//! Marks the client view camera entity (child of the player pawn).

/// Flecs/FFI reject zero-sized components; value unused.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewCameraTag {
    pub _tag: u8,
}

impl ViewCameraTag {
    pub const fn marker() -> Self {
        Self { _tag: 0 }
    }
}
