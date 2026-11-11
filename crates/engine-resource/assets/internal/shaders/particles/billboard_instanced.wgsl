// Engine built-in instanced particle shader (native particle pass).
// Drawn by `renderer/features/particles.rs`: one static corner quad
// (vertex-step) + one `GpuParticle` per instance (instance-step, 40-byte
// stride, see `engine-common/particle_gpu.rs`). Pipeline state is owned by
// the pass: additive blend, depth test on / write off, 4x4 Bayer alpha cut.
// The dither include is resolved by the renderer at module creation (the
// shader is compiled directly, not through the material registry).

struct ParticleView {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

@group(0) @binding(0) var<uniform> pv: ParticleView;

@group(1) @binding(0) var u_tex: texture_2d<f32>;
@group(1) @binding(1) var u_sampler: sampler;

#include "internal://shaders/inc/dither.wgsl"

struct VsIn {
    // Static quad (vertex step).
    @location(0) corner: vec2<f32>,
    @location(1) quad_uv: vec2<f32>,
    // GpuParticle instance (instance step, stride 40).
    @location(2) center: vec3<f32>,
    @location(3) size: f32,
    @location(4) rot: f32,
    @location(5) uv_rect: vec4<f32>,
    @location(6) tint: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) uv_rect: vec4<f32>,
    @location(2) tint: vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    // Spin the static corner in the quad plane, then scale by `size`.
    let c = cos(in.rot);
    let s = sin(in.rot);
    let corner = vec2<f32>(
        in.corner.x * c - in.corner.y * s,
        in.corner.x * s + in.corner.y * c,
    ) * in.size;
    // Cylindrical Z-up billboard basis toward the rendering camera
    // (fallback +X when degenerate). One instance buffer serves every
    // camera of the frame.
    var face = vec3<f32>(pv.camera_pos.x - in.center.x, pv.camera_pos.y - in.center.y, 0.0);
    if dot(face, face) < 1e-6 {
        face = vec3<f32>(1.0, 0.0, 0.0);
    } else {
        face = normalize(face);
    }
    let right = vec3<f32>(-face.y, face.x, 0.0);
    let up = vec3<f32>(0.0, 0.0, 1.0);
    let world = in.center + corner.x * right + corner.y * up;
    var out: VsOut;
    // Camera-relative projection (engine convention, see
    // `mesh_clip_from_world`): view_proj maps camera-space offsets, not
    // absolute world positions.
    out.pos = pv.view_proj * vec4<f32>(world - pv.camera_pos.xyz, 1.0);
    out.uv = in.quad_uv;
    out.uv_rect = in.uv_rect;
    out.tint = in.tint;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = mix(in.uv_rect.xy, in.uv_rect.zw, in.uv);
    let tex = textureSample(u_tex, u_sampler, uv);
    if tex.a < 0.01 {
        discard;
    }
    // Bayer alpha cut: hard edge instead of soft alpha smear (no sorting).
    let screen = vec2<f32>(in.pos.xy);
    if !dither_keep(screen, tex.a * in.tint.a) {
        discard;
    }
    return vec4<f32>(tex.rgb * in.tint.rgb, 1.0);
}
