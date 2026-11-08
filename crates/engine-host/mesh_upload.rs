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
    pub vertex_size: u32,
    pub vertex_bytes: *const u8,
    pub vertex_byte_len: u32,
    /// Which registered mesh allocator should own this mesh's GPU region.
    /// `0` = the engine's built-in slab allocator (default).
    pub allocator_id: u32,
}

pub fn mesh_from_desc(desc: &MeshCreateDesc) -> Option<Mesh> {
    panic!("mesh from desc");
    if desc.vertex_size == 0 || desc.vertex_bytes.is_null() || desc.vertex_byte_len == 0 {
        return None;
    }
    let vc = desc.vertex_count as usize;
    let mut mesh = Mesh::empty();
    if !desc.positions.is_null() && vc > 0 {
        let pos = unsafe { std::slice::from_raw_parts(desc.positions, vc * 3) };
        mesh.positions.reserve(vc);
        for i in 0..vc {
            mesh.positions
                .push([pos[i * 3], pos[i * 3 + 1], pos[i * 3 + 2]]);
        }
    }
    if !desc.indices.is_null() && desc.index_count > 0 {
        mesh.indices = unsafe {
            std::slice::from_raw_parts(desc.indices as *const u32, desc.index_count as usize)
                .to_vec()
        };
    }
    if desc.vertex_size > 0 {
        if desc.vertex_bytes.is_null() || desc.vertex_byte_len == 0 {
            panic!("mesh create: vertex_size set but vertex_bytes missing");
        }
        let expected = vc * desc.vertex_size as usize;
        if desc.vertex_byte_len as usize != expected {
            panic!(
                "mesh create: vertex_byte_len {} != vertex_count * vertex_size ({expected})",
                desc.vertex_byte_len
            );
        }
        mesh.vertex_size = desc.vertex_size;
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
    pub vertex_size: u32,
    pub vertex_bytes: *const u8,
    pub vertex_byte_len: u32,
    pub indices: *const u32,
    pub index_count: u32,
    pub allocator_id: u32,
}

pub fn mesh_from_packed(desc: &MeshPackedDesc) -> Option<Mesh> {
    if desc.vertex_size == 0 || desc.vertex_bytes.is_null() || desc.vertex_byte_len == 0 {
        return None;
    }
    if desc.vertex_byte_len as usize % desc.vertex_size as usize != 0 {
        return None;
    }
    let mut mesh = Mesh::empty();
    mesh.positions.clear();
    mesh.vertex_size = desc.vertex_size;
    mesh.vertex_bytes =
        unsafe { std::slice::from_raw_parts(desc.vertex_bytes, desc.vertex_byte_len as usize) }
            .to_vec();
    if !desc.indices.is_null() && desc.index_count > 0 {
        mesh.indices =
            unsafe { std::slice::from_raw_parts(desc.indices, desc.index_count as usize).to_vec() };
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
