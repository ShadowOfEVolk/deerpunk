//! Stable id types for the material system.
//!
//! Three layers of indirection (do not confuse):
//! - [`ShaderId`] — compiled shader variant (path + defines). Stable across frames.
//! - [`MaterialHandle`] — index into `MaterialStore`. Stable after load, dedup by path.
//! - [`GpuMaterialIndex`] — row in GPU SSBO. Renderer-internal, may shift on reload.

/// Compiled shader variant id. 0 = none / fallback.
pub type ShaderId = u32;
pub const SHADER_ID_NONE: ShaderId = 0;

/// Handle into `MaterialStore`. 0 = white/default material.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MaterialHandle(pub u32);

impl MaterialHandle {
    pub const NONE: Self = Self(0);
    pub fn is_none(self) -> bool {
        self.0 == 0
    }
}

pub const MATERIAL_HANDLE_NONE: MaterialHandle = MaterialHandle::NONE;

/// Row index in the GPU material SSBO. Renderer-internal only.
pub type GpuMaterialIndex = u32;
