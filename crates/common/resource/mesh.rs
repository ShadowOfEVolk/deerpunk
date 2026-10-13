use glam::Vec3;

use super::mesh_content::{allocate_mesh_content_id, MESH_CONTENT_INVALID};

#[derive(Clone, Debug)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Per-vertex lightmap UVs (empty → [0,0], sample white LM).
    pub lightmap_uvs: Vec<[f32; 2]>,
    /// Per-vertex lightstyle indices (f32, 255 = unused). Empty → all unused.
    pub lightmap_styles: Vec<[f32; 4]>,
    /// Per-vertex albedo atlas rect `[min_u, min_v, size_u, size_v]` (empty → full [0,0,1,1]).
    pub atlas_rects: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    /// Unique content id for GPU cache + refcount (`0` = invalid until assigned).
    pub content_id: u64,
}

impl Mesh {
    pub fn empty() -> Self {
        Self {
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            lightmap_uvs: Vec::new(),
            lightmap_styles: Vec::new(),
            atlas_rects: Vec::new(),
            indices: Vec::new(),
            content_id: allocate_mesh_content_id(),
        }
    }

    pub fn content_id(&self) -> u64 {
        self.content_id
    }

    /// Assign an id when building via struct literal (`content_id: 0`).
    pub fn ensure_content_id(&mut self) {
        if self.content_id == MESH_CONTENT_INVALID {
            self.content_id = allocate_mesh_content_id();
        }
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn index_count(&self) -> usize {
        self.indices.len()
    }

    pub fn scale(&mut self, s: f32) {
        for p in &mut self.positions {
            p[0] *= s;
            p[1] *= s;
            p[2] *= s;
        }
    }

    /// Инвертирует V текстурных координат (OBJ → wgpu: у wgpu (0,0) — верхний левый угол изображения).
    pub fn flip_uv_v(&mut self) {
        for uv in &mut self.uvs {
            uv[1] = 1.0 - uv[1];
        }
    }

    /// Создаёт простой куб как fallback-меш (центрирован, 2×2×2, 6 граней по 2 треугольника).
    pub fn cube() -> Self {
        Self::cube_sized(Vec3::ONE, Vec3::ZERO)
    }

    /// Axis-aligned box: unit cube (±1) × `half_extents`, then shifted by `center`.
    /// Example player hull: `half_extents=(16,16,36)`, `center=(0,0,36)` → feet at z=0.
    pub fn cube_sized(half_extents: Vec3, center: Vec3) -> Self {
        let (mut p, n, u, i) = build_cube();
        for v in &mut p {
            v[0] = v[0] * half_extents.x + center.x;
            v[1] = v[1] * half_extents.y + center.y;
            v[2] = v[2] * half_extents.z + center.z;
        }
        Self {
            positions: p,
            normals: n,
            uvs: u,
            lightmap_uvs: Vec::new(),
            lightmap_styles: Vec::new(),
            atlas_rects: Vec::new(),
            indices: i,
            content_id: allocate_mesh_content_id(),
        }
    }
}

fn build_cube() -> (Vec<[f32;3]>, Vec<[f32;3]>, Vec<[f32;2]>, Vec<u32>) {
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    let mut uv = Vec::new();
    let mut idx = Vec::new();
    let faces: [([f32;3], [f32;3], [f32;3], [f32;3], [f32;3]); 6] = [
        ([ 0.0, 0.0,-1.0], [-1.0,-1.0,-1.0], [ 1.0,-1.0,-1.0], [ 1.0, 1.0,-1.0], [-1.0, 1.0,-1.0]),
        ([ 0.0, 0.0, 1.0], [ 1.0,-1.0, 1.0], [-1.0,-1.0, 1.0], [-1.0, 1.0, 1.0], [ 1.0, 1.0, 1.0]),
        ([ 1.0, 0.0, 0.0], [ 1.0,-1.0,-1.0], [ 1.0, 1.0,-1.0], [ 1.0, 1.0, 1.0], [ 1.0,-1.0, 1.0]),
        ([-1.0, 0.0, 0.0], [-1.0,-1.0, 1.0], [-1.0, 1.0, 1.0], [-1.0, 1.0,-1.0], [-1.0,-1.0,-1.0]),
        ([ 0.0, 1.0, 0.0], [-1.0, 1.0,-1.0], [ 1.0, 1.0,-1.0], [ 1.0, 1.0, 1.0], [-1.0, 1.0, 1.0]),
        ([ 0.0,-1.0, 0.0], [-1.0,-1.0, 1.0], [ 1.0,-1.0, 1.0], [ 1.0,-1.0,-1.0], [-1.0,-1.0,-1.0]),
    ];
    let uvs = [[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,1.0]];
    for (n, v0, v1, v2, v3) in &faces {
        let base = pos.len() as u32;
        pos.extend_from_slice(&[*v0, *v1, *v2, *v3]);
        nrm.extend_from_slice(&[*n, *n, *n, *n]);
        uv.extend_from_slice(&uvs);
        idx.extend_from_slice(&[base, base+2, base+1, base, base+3, base+2]);
    }
    (pos, nrm, uv, idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_has_correct_counts() {
        let m = Mesh::cube();
        assert_eq!(m.positions.len(), 24);
        assert_eq!(m.indices.len(), 36);
    }
}
