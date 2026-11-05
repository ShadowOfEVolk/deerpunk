use deerpunk_engine_common::resource::mesh::Mesh;
use glam::Vec3;

pub trait AtlasRects {
    fn rect_for_tile(&self, tile: u8) -> [f32; 4];
}

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
        right = Vec3::X * half;
    } else {
        right = right.normalize() * half;
    }
    let up = Vec3::Z * half;
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
    mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}
