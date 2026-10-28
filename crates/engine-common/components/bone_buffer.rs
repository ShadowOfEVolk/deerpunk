//! Client-only runtime bone palette handle (matrices live in engine pool).

/// One studio / skeleton instance. Matrices uploaded via host `v1:bone_buffer:write`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoneBufferComponent {
    pub handle: u32,
    pub bone_count: u16,
    pub _pad: u16,
}

impl BoneBufferComponent {
    pub const INVALID: u32 = 0;

    pub fn new(handle: u32, bone_count: u16) -> Self {
        Self {
            handle,
            bone_count,
            _pad: 0,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.handle != Self::INVALID && self.bone_count > 0
    }
}
