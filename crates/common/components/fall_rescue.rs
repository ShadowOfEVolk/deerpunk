//! Teleport entity back when it falls below [`FallRescue::min_z`].

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FallRescue {
    /// Teleport when `position.z < min_z`.
    pub min_z: f32,
}

impl Default for FallRescue {
    fn default() -> Self {
        Self { min_z: -512.0 }
    }
}
