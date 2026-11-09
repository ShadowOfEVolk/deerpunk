//! Built-in WGSL used when a gamelib shader fails preprocess / naga reflect.

/// Magenta error material — matches default mesh pipeline bind groups + extended vertex (voxel-style).
pub const ERROR_MESH_WGSL: &str = r#"
struct MeshObjectUniform {
    model: mat4x4<f32>,
    material_data: vec4<u32>,
};

struct MeshViewUniform {
    view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    _pad_cam: f32,
};

struct MeshUniform {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    material_data: vec4<u32>,
    camera_pos: vec3<f32>,
    _pad_cam: f32,
};

@group(0) @binding(0) var<storage, read> mesh_uniform_slots: array<MeshObjectUniform>;
@group(0) @binding(2) var<uniform> mesh_view: MeshViewUniform;

fn mesh_mu_slot(slot: u32) -> MeshUniform {
    let src = mesh_uniform_slots[slot];
    return MeshUniform(mesh_view.view_proj, src.model, src.material_data, mesh_view.camera_pos, 0.0);
}

struct MaterialData {
    color: vec4<f32>,
    params: vec4<f32>,
    albedo_idx: u32,
    normal_idx: u32,
    orm_idx: u32,
    _pad: u32,
    emission: vec4<f32>,
    sun_dir: vec4<f32>,
    ambient: vec4<f32>,
};

struct PointLight {
    position: vec3<f32>,
    range: f32,
    color: vec3<f32>,
    intensity: f32,
};

struct LightsUniform {
    count: u32,
    scene_time_bits: u32,
    zenith: vec4<f32>,
    sun_dir: vec4<f32>,
    ambient: vec4<f32>,
    lights: array<PointLight, 32>,
};

#@shader pass=mesh_opaque queue=1000

@group(1) @binding(0) var u_tex: texture_2d<f32>;
@group(1) @binding(1) var u_sampler: sampler;
@group(2) @binding(0) var<storage, read> materials: array<MaterialData>;
@group(2) @binding(1) var u_normal: texture_2d<f32>;
@group(2) @binding(2) var u_orm: texture_2d<f32>;
@group(2) @binding(3) var u_mat_sampler: sampler;
@group(3) @binding(0) var<uniform> lights_u: LightsUniform;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) atlas_rect: vec4<f32>,
    @location(4) lm_uv: vec2<f32>,
    @location(5) lm_styles: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) @interpolate(flat) draw_slot: u32,
};

@vertex
fn vs_main(in: VsIn, @builtin(instance_index) draw_slot: u32) -> VsOut {
    let mu = mesh_mu_slot(draw_slot);
    let world_pos = mu.model * vec4(in.position, 1.0);
    var out: VsOut;
    out.pos = mu.view_proj * world_pos;
    out.world_pos = world_pos.xyz;
    out.draw_slot = draw_slot;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let _mu = mesh_mu_slot(in.draw_slot);
    // Classic engine "missing shader" magenta (linear HDR).
    let err = vec3<f32>(0.92, 0.08, 0.78);
    let stripe = step(0.5, fract(in.world_pos.x * 0.35 + in.world_pos.y * 0.35));
    let col = mix(err * 0.55, err, stripe);
    return vec4(col, 1.0);
}
"#;
