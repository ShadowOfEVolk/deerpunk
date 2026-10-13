//! ECS component for material assignment.
//!
//! Thin handle — the entity references a material in `MaterialStore`, not a
//! fat PBR copy. One asset → N entities (shared handle).

use crate::material::handle::MaterialHandle;

/// ECS component: reference to a material asset.
///
/// Attach alongside `TransformComponent + Mesh3D` for renderable entities.
/// `handle == MaterialHandle::NONE` (0) → white/default material.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DrawMaterial {
    /// Index into `MaterialStore`. Stable after load.
    pub handle: MaterialHandle,
}

impl DrawMaterial {
    pub fn new(handle: MaterialHandle) -> Self {
        Self { handle }
    }

    pub fn none() -> Self {
        Self::default()
    }
}
