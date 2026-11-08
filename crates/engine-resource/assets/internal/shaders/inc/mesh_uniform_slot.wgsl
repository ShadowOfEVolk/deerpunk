// Per-draw object data (storage) + one camera uniform for the whole pass.
// `#include "internal://shaders/inc/mesh_uniform_slot.wgsl"`

struct MeshObjectUniform {
    model: mat4x4<f32>,
    material_data: vec4<u32>,
}

struct MeshViewUniform {
    view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    _pad_cam: f32,
}

// Same field names shaders already read (`mu.view_proj`, `mu.model`, `mu.camera_pos`).
struct MeshUniform {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    material_data: vec4<u32>,
    camera_pos: vec3<f32>,
    _pad_cam: f32,
}

@group(0) @binding(0) var<storage, read> mesh_uniform_slots: array<MeshObjectUniform>;
@group(0) @binding(2) var<uniform> mesh_view: MeshViewUniform;

fn mesh_mu_slot(slot: u32) -> MeshUniform {
    let src = mesh_uniform_slots[slot];
    return MeshUniform(
        mesh_view.view_proj,
        src.model,
        src.material_data,
        mesh_view.camera_pos,
        0.0,
    );
}
