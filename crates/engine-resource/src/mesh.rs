use glam::Vec3;

use super::mesh_content::{allocate_mesh_content_id, MESH_CONTENT_INVALID};

#[derive(Clone, Debug)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Per-vertex lightmap UVs (empty → [0,0], sample white LM).
    pub lightmap_uvs: Vec<[f32; 2]>,
    /// Per-vertex user data (`Mesh::lightmap_styles` when present; game-defined).
    pub lightmap_styles: Vec<[f32; 4]>,
    /// Per-vertex albedo atlas rect `[min_u, min_v, size_u, size_v]` (empty → full [0,0,1,1]).
    pub atlas_rects: Vec<[f32; 4]>,
    /// Atlas tile index for compact `light_tile` packing (parallel to [`positions`]; empty → 0).
    pub atlas_tile: Vec<u16>,
    /// Pre-packed interleaved vertices (`vertex_size` bytes per vert). Upload skips SoA packer.
    pub vertex_bytes: Vec<u8>,
    /// Non-zero → [`vertex_bytes`] is authoritative for GPU upload (must match material `#@vertex` stride).
    pub vertex_size: u32,
    pub indices: Vec<u32>,
    /// Skinning joint indices (4 per vertex). Empty → static mesh.
    pub joint_indices: Vec<[u8; 4]>,
    /// Skinning weights (normalized). Parallel to [`joint_indices`].
    pub joint_weights: Vec<[f32; 4]>,
    /// Unique content id for GPU cache + refcount (`0` = invalid until assigned).
    pub content_id: u64,
    /// Bumped by [`MeshRegistry::replace`](super::mesh_registry::MeshRegistry::replace)
    /// and [`MeshRegistry::mark_dirty`](super::mesh_registry::MeshRegistry::mark_dirty).
    /// The renderer mirrors this into [`MeshRegistry::mesh_revisions`] and re-uploads
    /// when that entry differs from the revision it last sent to the GPU.
    /// Games do not touch GPU.
    pub gpu_revision: u64,
    /// Which registered [`MeshAllocator`](deerpunk_engine_resource::mesh_allocator::MeshAllocator)
    /// owns this mesh's GPU region. `0` = default built-in slab allocator.
    /// Set via `v1:mesh:create_with` when a game wants a custom allocator
    /// (e.g. a region arena for streaming voxel chunks).
    pub allocator_id: u32,
    /// Object-space AABB. Invariant after the mesh is built: set once at upload
    /// (`mesh_from_desc` / packed desc / loader / primitive). Readers never recompute it.
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
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
            atlas_tile: Vec::new(),
            vertex_bytes: Vec::new(),
            vertex_size: 0,
            indices: Vec::new(),
            joint_indices: Vec::new(),
            joint_weights: Vec::new(),
            content_id: allocate_mesh_content_id(),
            gpu_revision: 1,
            allocator_id: 0,
            bounds_min: [f32::INFINITY, f32::INFINITY, f32::INFINITY],
            bounds_max: [f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY],
        }
    }

    /// Fill [`bounds_min`] / [`bounds_max`] from [`positions`].
    /// Only while building a mesh (upload, loader, primitive). Not for readers.
    pub fn ensure_local_bounds(&mut self) {
        if self.positions.is_empty() {
            self.bounds_min = [0.0, 0.0, 0.0];
            self.bounds_max = [0.0, 0.0, 0.0];
            return;
        }
        let mut min = [f32::INFINITY, f32::INFINITY, f32::INFINITY];
        let mut max = [f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
        for p in &self.positions {
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        self.bounds_min = min;
        self.bounds_max = max;
    }

    pub fn has_local_bounds(&self) -> bool {
        self.bounds_min[0] <= self.bounds_max[0]
            && self.bounds_min[1] <= self.bounds_max[1]
            && self.bounds_min[2] <= self.bounds_max[2]
    }

    pub fn is_skinned(&self) -> bool {
        !self.joint_indices.is_empty()
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
        let stride = self.vertex_size as usize;
        if stride == 0 || self.vertex_bytes.len() % stride != 0 {
            return 0;
        }
        self.vertex_bytes.len() / stride
    }

    pub fn uses_prepacked_vertices(&self) -> bool {
        let stride = self.vertex_size as usize;
        self.vertex_size > 0
            && stride > 0
            && !self.vertex_bytes.is_empty()
            && self.vertex_bytes.len() % stride == 0
            && (self.positions.is_empty()
                || self.vertex_bytes.len() == self.positions.len() * stride)
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
        let mut m = Self {
            positions: p,
            normals: n,
            uvs: u,
            lightmap_uvs: Vec::new(),
            lightmap_styles: Vec::new(),
            atlas_rects: Vec::new(),
            atlas_tile: Vec::new(),
            vertex_bytes: Vec::new(),
            vertex_size: 0,
            indices: i,
            joint_indices: Vec::new(),
            joint_weights: Vec::new(),
            content_id: allocate_mesh_content_id(),
            gpu_revision: 1,
            allocator_id: 0,
            bounds_min: [f32::INFINITY, f32::INFINITY, f32::INFINITY],
            bounds_max: [f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY],
        };
        m.ensure_local_bounds();
        m
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
