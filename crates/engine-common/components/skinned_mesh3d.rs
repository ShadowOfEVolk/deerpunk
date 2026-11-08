use crate::components::render_layer::RENDER_LAYER_DEFAULT_DRAW;
use crate::resource::mesh_handle::MeshHandle;

/// Skinned drawable: shares [`BoneBufferComponent`] handle (variant B).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkinnedMesh3D {
    pub mesh: MeshHandle,
    pub bone_buffer: u32,
    /// Camera bits. Same meaning as [`super::Mesh3D::layer_mask`].
    pub layer_mask: u32,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub world_min: [f32; 3],
    pub world_max: [f32; 3],
    pub baked_pos: [f32; 3],
    pub baked_rot: [f32; 4],
    pub baked_scale: [f32; 3],
    pub world_baked: u32,
}

impl Default for SkinnedMesh3D {
    fn default() -> Self {
        Self {
            mesh: MeshHandle::INVALID,
            bone_buffer: 0,
            layer_mask: RENDER_LAYER_DEFAULT_DRAW,
            bounds_min: [f32::INFINITY; 3],
            bounds_max: [f32::NEG_INFINITY; 3],
            world_min: [0.0; 3],
            world_max: [0.0; 3],
            baked_pos: [0.0; 3],
            baked_rot: [0.0; 4],
            baked_scale: [0.0; 3],
            world_baked: 0,
        }
    }
}

impl SkinnedMesh3D {
    pub fn new(mesh: MeshHandle, bone_buffer: u32) -> Self {
        Self {
            mesh,
            bone_buffer,
            ..Self::default()
        }
    }

    pub fn set_local_bounds(mut self, min: [f32; 3], max: [f32; 3]) -> Self {
        self.bounds_min = min;
        self.bounds_max = max;
        self.world_baked = 0;
        self
    }

    pub fn has_local_bounds(&self) -> bool {
        self.bounds_min[0] <= self.bounds_max[0]
            && self.bounds_min[1] <= self.bounds_max[1]
            && self.bounds_min[2] <= self.bounds_max[2]
    }

    pub fn fresh_world(
        &self,
        pos: [f32; 3],
        rot: [f32; 4],
        scale: [f32; 3],
    ) -> Option<([f32; 3], [f32; 3])> {
        if self.world_baked == 0
            || self.baked_pos != pos
            || self.baked_rot != rot
            || self.baked_scale != scale
        {
            return None;
        }
        Some((self.world_min, self.world_max))
    }

    pub fn bake_world(
        &mut self,
        pos: [f32; 3],
        rot: [f32; 4],
        scale: [f32; 3],
        world_min: [f32; 3],
        world_max: [f32; 3],
    ) {
        self.baked_pos = pos;
        self.baked_rot = rot;
        self.baked_scale = scale;
        self.world_min = world_min;
        self.world_max = world_max;
        self.world_baked = 1;
    }
}
