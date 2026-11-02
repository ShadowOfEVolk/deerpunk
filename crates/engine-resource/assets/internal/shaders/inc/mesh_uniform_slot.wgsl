// Engine built-in mesh uniform array (256-byte stride). Use with @builtin(instance_index).
// `#include "internal://shaders/inc/mesh_uniform_slot.wgsl"`

struct MeshUniform {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    material_data: vec4<u32>,
    camera_pos: vec3<f32>,
    _pad_cam: f32,
    _pad: array<f32, 24>,
}

@group(0) @binding(0) var<storage, read> mesh_uniform_slots: array<MeshUniform>;

fn mesh_mu_slot(slot: u32) -> MeshUniform {
    return mesh_uniform_slots[slot];
}
