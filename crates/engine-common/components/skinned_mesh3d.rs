use crate::resource::mesh_handle::MeshHandle;

/// Skinned drawable: shares [`BoneBufferComponent`] handle (variant B).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkinnedMesh3D {
    pub mesh: MeshHandle,
    pub bone_buffer: u32,
}

impl SkinnedMesh3D {
    pub fn new(mesh: MeshHandle, bone_buffer: u32) -> Self {
        Self {
            mesh,
            bone_buffer,
        }
    }
}
