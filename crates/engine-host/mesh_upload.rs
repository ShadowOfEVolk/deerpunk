//! Build [`Mesh`] from gamelib FFI upload descriptors.

use deerpunk_engine_common::resource::mesh::Mesh;

use std::ffi::c_void;

#[repr(C)]
pub struct MeshCreateDesc {
    pub positions: *const f32,
    pub vertex_count: u32,
    pub normals: *const f32,
    pub uvs: *const f32,
    pub indices: *const c_void,
    pub index_count: u32,
    pub joint_indices: *const c_void,
    pub joint_weights: *const f32,
    /// Per-vertex LM UVs (`vertex_count * 2` floats), optional.
    pub lightmap_uvs: *const f32,
    /// Per-vertex style indices (`vertex_count * 4` floats), optional.
    pub lightmap_styles: *const f32,
    /// Per-vertex atlas rects (`vertex_count * 4` floats), optional.
    pub atlas_rects: *const f32,
    /// Compact / prepacked interleaved vertices (`vertex_byte_len` bytes), optional.
    pub packed_stride: u32,
    pub vertex_bytes: *const u8,
    pub vertex_byte_len: u32,
    /// Which registered mesh allocator should own this mesh's GPU region.
    /// `0` = the engine's built-in slab allocator (default).
    pub allocator_id: u32,
}

pub fn mesh_from_desc(desc: &MeshCreateDesc) -> Option<Mesh> {
    if desc.positions.is_null() || desc.vertex_count == 0 {
        return None;
    }
    let vc = desc.vertex_count as usize;
    let pos = unsafe { std::slice::from_raw_parts(desc.positions, vc * 3) };
    let mut mesh = Mesh::empty();
    mesh.positions.reserve(vc);
    for i in 0..vc {
        mesh.positions.push([pos[i * 3], pos[i * 3 + 1], pos[i * 3 + 2]]);
    }
    if !desc.normals.is_null() {
        let nrm = unsafe { std::slice::from_raw_parts(desc.normals, vc * 3) };
        for i in 0..vc {
            mesh.normals.push([nrm[i * 3], nrm[i * 3 + 1], nrm[i * 3 + 2]]);
        }
    }
    if !desc.uvs.is_null() {
        let uv = unsafe { std::slice::from_raw_parts(desc.uvs, vc * 2) };
        for i in 0..vc {
            mesh.uvs.push([uv[i * 2], uv[i * 2 + 1]]);
        }
    }
    if !desc.indices.is_null() && desc.index_count > 0 {
        mesh.indices = unsafe {
            std::slice::from_raw_parts(desc.indices as *const u32, desc.index_count as usize)
                .to_vec()
        };
    }
    if !desc.joint_indices.is_null() {
        let ji = unsafe { std::slice::from_raw_parts(desc.joint_indices as *const u8, vc * 4) };
        for i in 0..vc {
            mesh.joint_indices.push([ji[i * 4], ji[i * 4 + 1], ji[i * 4 + 2], ji[i * 4 + 3]]);
        }
    }
    if !desc.joint_weights.is_null() {
        let jw = unsafe { std::slice::from_raw_parts(desc.joint_weights, vc * 4) };
        for i in 0..vc {
            mesh.joint_weights.push([
                jw[i * 4],
                jw[i * 4 + 1],
                jw[i * 4 + 2],
                jw[i * 4 + 3],
            ]);
        }
    }
    if !desc.lightmap_uvs.is_null() {
        let lm = unsafe { std::slice::from_raw_parts(desc.lightmap_uvs, vc * 2) };
        for i in 0..vc {
            mesh.lightmap_uvs.push([lm[i * 2], lm[i * 2 + 1]]);
        }
    }
    if !desc.lightmap_styles.is_null() {
        let ls = unsafe { std::slice::from_raw_parts(desc.lightmap_styles, vc * 4) };
        for i in 0..vc {
            mesh.lightmap_styles.push([ls[i * 4], ls[i * 4 + 1], ls[i * 4 + 2], ls[i * 4 + 3]]);
        }
    }
    if !desc.atlas_rects.is_null() {
        let ar = unsafe { std::slice::from_raw_parts(desc.atlas_rects, vc * 4) };
        for i in 0..vc {
            mesh.atlas_rects.push([ar[i * 4], ar[i * 4 + 1], ar[i * 4 + 2], ar[i * 4 + 3]]);
        }
    }
    if desc.packed_stride > 0 {
        if desc.vertex_bytes.is_null() || desc.vertex_byte_len == 0 {
            panic!("mesh create: packed_stride set but vertex_bytes missing");
        }
        let expected = vc * desc.packed_stride as usize;
        if desc.vertex_byte_len as usize != expected {
            panic!(
                "mesh create: vertex_byte_len {} != vertex_count * packed_stride ({expected})",
                desc.vertex_byte_len
            );
        }
        mesh.packed_stride = desc.packed_stride;
        mesh.vertex_bytes =
            unsafe { std::slice::from_raw_parts(desc.vertex_bytes, expected).to_vec() };
    }
    mesh.allocator_id = desc.allocator_id;
    mesh.ensure_content_id();
    mesh.ensure_local_bounds();
    Some(mesh)
}

/// Packed GPU buffer + caller-supplied AABB. No SoA `positions` and no bounds scan.
pub struct MeshPackedDesc {
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub packed_stride: u32,
    pub vertex_bytes: *const u8,
    pub vertex_byte_len: u32,
    pub indices: *const u32,
    pub index_count: u32,
    pub allocator_id: u32,
}

pub fn mesh_from_packed(desc: &MeshPackedDesc) -> Option<Mesh> {
    if desc.packed_stride == 0 || desc.vertex_bytes.is_null() || desc.vertex_byte_len == 0 {
        return None;
    }
    if desc.vertex_byte_len as usize % desc.packed_stride as usize != 0 {
        return None;
    }
    let mut mesh = Mesh::empty();
    mesh.positions.clear();
    mesh.packed_stride = desc.packed_stride;
    mesh.vertex_bytes =
        unsafe { std::slice::from_raw_parts(desc.vertex_bytes, desc.vertex_byte_len as usize) }
            .to_vec();
    if !desc.indices.is_null() && desc.index_count > 0 {
        mesh.indices = unsafe {
            std::slice::from_raw_parts(desc.indices, desc.index_count as usize).to_vec()
        };
    }
    mesh.bounds_min = desc.bounds_min;
    mesh.bounds_max = desc.bounds_max;
    mesh.allocator_id = desc.allocator_id;
    mesh.ensure_content_id();
    if !mesh.has_local_bounds() || !mesh.uses_prepacked_vertices() {
        return None;
    }
    Some(mesh)
}