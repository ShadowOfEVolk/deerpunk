//! Billboard quad emission into a `Mesh`.
//!
//! Two paths:
//! - [`push_billboard_quad`] — legacy SoA mesh fill (games that pack their own
//!   vertices, tile-indexed UVs).
//! - [`push_billboard_vertex`] / [`push_billboard_indices`] — packed
//!   interleaved vertices matching the engine particle shader layout
//!   `#@vertex position float3, uv float2, atlas_rect float4, baked_color unorm8x4`
//!   (40-byte stride, little-endian). This is what the engine particle mesh
//!   builder uses since GPU upload requires prepacked `vertex_bytes`.

use deerpunk_engine_common::resource::mesh::Mesh;
use glam::Vec3;

/// Vertex layout stride for the engine particle shader.
pub const PARTICLE_VERTEX_STRIDE: u32 = 40;

/// Billboard orientation basis: camera-facing, Z-up constrained, spun by `rot`.
/// Returns `(right, up)` unit vectors scaled by `half`.
pub fn billboard_axes(center: Vec3, half: f32, rot: f32, camera: Vec3) -> (Vec3, Vec3) {
    let mut face = camera - center;
    face.z = 0.0;
    if face.length_squared() < 1e-6 {
        face = Vec3::X;
    } else {
        face = face.normalize();
    }
    let mut right = Vec3::new(-face.y, face.x, 0.0);
    let c = rot.cos();
    let s = rot.sin();
    right = Vec3::new(right.x * c - right.y * s, right.x * s + right.y * c, 0.0);
    if right.length_squared() < 1e-8 {
        right = Vec3::X;
    } else {
        right = right.normalize();
    }
    let up = Vec3::Z;
    (right * half, up * half)
}

/// Legacy SoA quad fill (u8 tile + square span via an [`AtlasRects`] impl).
pub trait AtlasRects {
    fn rect_for_tile(&self, tile: u8) -> [f32; 4];
}

#[allow(clippy::too_many_arguments)]
pub fn push_billboard_quad(
    mesh: &mut Mesh,
    center: Vec3,
    half: f32,
    rot: f32,
    tile: u8,
    atlas: &impl AtlasRects,
    camera: Vec3,
    lm: [f32; 4],
    lm_uv_x: f32,
    lm_uv_y: f32,
    uv_min: [f32; 2],
    uv_span: f32,
) {
    let (right, up) = billboard_axes(center, half, rot, camera);
    let verts = [
        center - right - up,
        center + right - up,
        center + right + up,
        center - right + up,
    ];
    let u0 = uv_min[0];
    let v0 = uv_min[1];
    let uvs = [
        [u0, v0],
        [u0 + uv_span, v0],
        [u0 + uv_span, v0 + uv_span],
        [u0, v0 + uv_span],
    ];
    let rect = atlas.rect_for_tile(tile);
    let base = mesh.positions.len() as u32;
    for (v, uv) in verts.iter().zip(uvs) {
        mesh.positions.push(v.to_array());
        mesh.normals.push([0.0, 0.0, 1.0]);
        mesh.uvs.push(uv);
        mesh.atlas_rects.push(rect);
        mesh.lightmap_uvs.push([lm_uv_x, lm_uv_y]);
        mesh.lightmap_styles.push(lm);
    }
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Packed 40-byte particle vertex:
/// position float3 | uv float2 | atlas_rect float4 | color unorm8x4.
pub fn encode_particle_vertex(out: &mut Vec<u8>, pos: Vec3, uv: [f32; 2], rect: [f32; 4], tint: [f32; 4]) {
    for v in pos.to_array() {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in uv {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in rect {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for c in tint {
        let b = (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        out.push(b);
    }
}

/// Push one camera-facing billboard quad as packed vertices + indices.
///
/// `uv` is the sprite rect `[u0, v0, u1, v1]` in the bound texture (already
/// resolved by the caller — template atlas tile or a payload override).
/// `tint` rides the vertex color; alpha carries fade/opacity.
#[allow(clippy::too_many_arguments)]
pub fn push_billboard_vertex(
    mesh: &mut Mesh,
    center: Vec3,
    half: f32,
    rot: f32,
    uv: [f32; 4],
    rect: [f32; 4],
    tint: [f32; 4],
    camera: Vec3,
) {
    let (right, up) = billboard_axes(center, half, rot, camera);
    let verts = [
        center - right - up,
        center + right - up,
        center + right + up,
        center - right + up,
    ];
    let [u0, v0, u1, v1] = uv;
    let uvs = [
        [u0, v0],
        [u1, v0],
        [u1, v1],
        [u0, v1],
    ];
    debug_assert_eq!(mesh.vertex_bytes.len() % PARTICLE_VERTEX_STRIDE as usize, 0);
    let base = (mesh.vertex_bytes.len() / PARTICLE_VERTEX_STRIDE as usize) as u32;
    for (v, uv2) in verts.iter().zip(uvs) {
        encode_particle_vertex(&mut mesh.vertex_bytes, *v, uv2, rect, tint);
    }
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    mesh.ensure_local_bounds();
}
