//! Built-in WGSL used when a gamelib shader fails preprocess / naga reflect.

/// Magenta error material — matches default mesh pipeline bind groups + extended vertex (voxel-style).
pub const ERROR_MESH_WGSL: &str = r#"
struct MeshUniform {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    material_data: vec4<u32>,
    camera_pos: vec3<f32>,
    _pad_cam: f32,
};

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

@group(0) @binding(0) var<uniform> mu: MeshUniform;
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
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let world_pos = mu.model * vec4(in.position, 1.0);
    var out: VsOut;
    out.pos = mu.view_proj * world_pos;
    out.world_pos = world_pos.xyz;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Classic engine "missing shader" magenta (linear HDR).
    let err = vec3<f32>(0.92, 0.08, 0.78);
    let stripe = step(0.5, fract(in.world_pos.x * 0.35 + in.world_pos.y * 0.35));
    let col = mix(err * 0.55, err, stripe);
    return vec4(col, 1.0);
}
"#;

/// Fullscreen fallback when `sky.wgsl` / `cloud.wgsl` fail GPU compile.
pub const ERROR_SKY_WGSL: &str = r#"
struct SkyUniform {
    inv_view_proj: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    camera_pos: vec4<f32>,
    params: array<vec4<f32>, 8>,
    sky_color: vec4<f32>,
    zenith: vec4<f32>,
};

@group(0) @binding(0) var<uniform> sky: SkyUniform;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var ndc = vec2<f32>(0.0, 0.0);
    if vi == 0u { ndc = vec2(-1.0, -1.0); }
    else if vi == 1u { ndc = vec2(3.0, -1.0); }
    else { ndc = vec2(-1.0, 3.0); }
    var out: VsOut;
    out.pos = vec4(ndc, 0.0, 1.0);
    out.ndc = ndc;
    return out;
}

fn sky_view_ray_dir(inv_view_proj: mat4x4<f32>, ndc: vec2<f32>) -> vec3<f32> {
    let p0 = inv_view_proj * vec4(ndc, 0.0, 1.0);
    let p1 = inv_view_proj * vec4(ndc, 1.0, 1.0);
    return normalize(p1.xyz / p1.w - p0.xyz / p1.w);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let _ = sky_view_ray_dir(sky.inv_view_proj, in.ndc);
    return vec4(0.55, 0.05, 0.65, 1.0);
}
"#;
