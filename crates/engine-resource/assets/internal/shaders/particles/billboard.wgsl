// Engine built-in billboard particle shader.
// `internal://shaders/particles/billboard.wgsl` — additive blending, 4x4 Bayer alpha cut.
//
// Vertex layout (40-byte stride, packed by engine-particles):
//   position float3 | uv float2 | atlas_rect float4 | baked_color unorm8x4 (tint rgba)

#@shader pass=mesh_transparent queue=2000 blend=additive

#@vertex position float3, uv float2, atlas_rect float4, baked_color unorm8x4

#@prop _BaseColor color [1,1,1,1]
#include "internal://shaders/inc/mesh_uniform_slot.wgsl"

#@prop _BaseMap texture2d white
@group(1) @binding(0) var u_tex: texture_2d<f32>;

@group(1) @binding(1) var u_sampler: sampler;

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

@group(2) @binding(0) var<storage, read> materials: array<MaterialData>;

@group(3) @binding(0) var<uniform> lights_u: LightsUniform;

struct PointLight {
    position: vec3<f32>,
    range: f32,
    color: vec3<f32>,
    intensity: f32,
};

struct LightsUniform {
    count: u32,
    scene_time_bits: u32,
    fog_start_bits: u32,
    fog_falloff_bits: u32,
    zenith: vec4<f32>,
    sun_dir: vec4<f32>,
    ambient: vec4<f32>,
    lights: array<PointLight, 32>,
};

#include "internal://shaders/inc/dither.wgsl"

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) atlas_rect: vec4<f32>,
    @location(3) tint: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) atlas_rect: vec4<f32>,
    @location(2) tint: vec4<f32>,
    @location(4) @interpolate(flat) draw_slot: u32,
};

@vertex
fn vs_main(in: VsIn, @builtin(instance_index) draw_slot: u32) -> VsOut {
    let mu = mesh_mu_slot(draw_slot);
    // Vertices are already world-space billboards; model is identity.
    let world_pos = mu.model * vec4(in.position, 1.0);
    var out: VsOut;
    out.pos = mu.view_proj * world_pos;
    out.uv = in.uv;
    out.atlas_rect = in.atlas_rect;
    out.tint = in.tint;
    out.draw_slot = draw_slot;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = mix(in.atlas_rect.xy, in.atlas_rect.zw, in.uv);
    var tex = textureSample(u_tex, u_sampler, uv);
    if tex.a < 0.01 {
        discard;
    }
    // Bayer alpha cut: hard edge instead of soft alpha smear (no sorting).
    let screen = vec2<f32>(in.pos.xy);
    if !dither_keep(screen, tex.a * in.tint.a) {
        discard;
    }
    let mu = mesh_mu_slot(in.draw_slot);
    let mat = materials[mu.material_data.x];
    let rgb = tex.rgb * in.tint.rgb * mat.color.rgb;
    return vec4(rgb, 1.0);
}
