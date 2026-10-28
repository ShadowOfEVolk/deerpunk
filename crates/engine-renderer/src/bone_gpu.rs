//! GPU bone palette blocks (group 0 binding 1 for skinned mesh shaders).

use bytemuck::{Pod, Zeroable};

pub const MAX_GPU_BONES: usize = 128;

/// Column-major bone matrices — matches WGSL `array<mat4x4<f32>, 128>`.
#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuBoneBlock {
    pub matrices: [[[f32; 4]; 4]; MAX_GPU_BONES],
}

impl GpuBoneBlock {
    pub fn from_palette(palette: &[[[f32; 4]; 4]]) -> Self {
        let mut block = Self::zeroed();
        let n = palette.len().min(MAX_GPU_BONES);
        for i in 0..n {
            block.matrices[i] = palette[i];
        }
        if n == 0 {
            block.matrices[0] = [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ];
        }
        block
    }
}

pub const GPU_BONE_BLOCK_SIZE: u64 = std::mem::size_of::<GpuBoneBlock>() as u64;
