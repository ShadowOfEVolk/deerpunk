//! Marker: entity excluded from draw-material extract query (gamelib sets/clears).

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshDrawHidden {
    /// Flecs/FFI reject zero-sized components; value unused.
    pub _tag: u8,
}

impl MeshDrawHidden {
    pub const fn marker() -> Self {
        Self { _tag: 0 }
    }
}
