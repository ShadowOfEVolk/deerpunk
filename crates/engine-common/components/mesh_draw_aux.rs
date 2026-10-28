//! Per-draw data copied into [`MeshUniform::material_data`] (y/z/w) during mesh pass.

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshDrawAux {
    /// Crack overlay destroy stage 0..9 (`block_break.wgsl` reads `material_data.y`).
    pub crack_stage: u32,
    pub _pad: [u32; 3],
}

impl MeshDrawAux {
    pub fn crack(stage: u8) -> Self {
        Self {
            crack_stage: stage.min(9) as u32,
            _pad: [0, 0, 0],
        }
    }
}
