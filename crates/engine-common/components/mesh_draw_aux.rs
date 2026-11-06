//! Per-draw data copied into [`MeshUniform::material_data`] (y/z/w) during mesh pass.

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshDrawAux {
    /// Crack overlay destroy stage 0..9 (`block_break.wgsl` reads `material_data.y`).
    pub crack_stage: u32,
    /// Chunk pop-in spawn time as `f32` bits (`voxel*.wgsl` when non-zero).
    pub reveal_spawn_bits: u32,
    pub _pad: [u32; 2],
}

impl MeshDrawAux {
    pub fn crack(stage: u8) -> Self {
        Self {
            crack_stage: stage.min(9) as u32,
            reveal_spawn_bits: 0,
            _pad: [0, 0],
        }
    }

    pub fn chunk_reveal(spawn_time: f32) -> Self {
        Self {
            crack_stage: 0,
            reveal_spawn_bits: spawn_time.to_bits(),
            _pad: [0, 0],
        }
    }

    /// Value for [`MeshDraw::uniform_aux`] / `MeshUniform.material_data.y`.
    pub fn material_data_y(self) -> u32 {
        if self.reveal_spawn_bits != 0 {
            self.reveal_spawn_bits
        } else {
            self.crack_stage
        }
    }
}
