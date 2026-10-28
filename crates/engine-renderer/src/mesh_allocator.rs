//! GPU mesh buffer pool + offset-only slab allocator.
//!
//! The engine owns the GPU vertex/index buffers ([`MeshBufferPool`]); an
//! [`SlabMeshAllocator`] is an offset-only bookkeeper implementing
//! `deerpunk_engine_resource::MeshAllocator`. The default allocator is
//! append-only (no `free`) — fine for long-lived meshes (studio models,
//! HUD). Streaming meshes (voxel chunks) should use a game-provided
//! allocator with real `free` (registered via `v1:mesh:register_allocator`).

use deerpunk_engine_common::resource::mesh_allocator as res;

pub use res::MeshAllocHandle as MeshHandle;
pub use res::MeshAllocator;

/// Vertex format: pos+nrm+uv+atlas_rect+lm_uv+lm_styles+pad = 80 bytes (`MeshVertex` in gpu.rs).
pub const MESH_VERTEX_SIZE: u32 = 80;
/// Skinned studio mesh: pos, nrm, uv, joint indices, weights.
pub const SKINNED_MESH_VERTEX_SIZE: u32 = 64;
pub const MESH_INDEX_SIZE: u32 = 4;

/// One GPU buffer pair (VB + IB) with a fixed vertex stride.
pub struct BufferPair {
    pub vertex_buf: wgpu::Buffer,
    pub index_buf: wgpu::Buffer,
    pub vert_cap: u32,
    pub idx_cap: u32,
    pub stride: u32,
}

/// Engine-owned pool of GPU vertex/index buffers, indexed by `buffer_id`.
///
/// Allocators refer to buffers by id; the renderer resolves ids to
/// `wgpu::Buffer`s here for binding. `buffer_id` values start at 1 —
/// `0` is reserved for the invalid handle (see [`MeshHandle::INVALID`]).
/// Internally buffers are stored dense from index 0; accessors subtract 1
/// from the id. An invalid [`MeshHandle`] (zero counts) slices to an empty
/// range, so binding it is a harmless no-op draw.
pub struct MeshBufferPool {
    buffers: Vec<BufferPair>,
    default_vertex_cap: u32,
    default_index_cap: u32,
}

impl MeshBufferPool {
    pub fn new(default_vertex_cap: u32, default_index_cap: u32) -> Self {
        Self {
            buffers: Vec::new(),
            default_vertex_cap,
            default_index_cap,
        }
    }

    /// Create a new buffer pair for `stride` and return its id. The caller
    /// (an allocator) is informed via its `add_buffer` trait method.
    pub fn add(
        &mut self,
        device: &wgpu::Device,
        vert_cap: u32,
        idx_cap: u32,
        stride: u32,
    ) -> u32 {
        let vert_cap = vert_cap.max(self.default_vertex_cap);
        let idx_cap = idx_cap.max(self.default_index_cap);
        let vertex_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh_pool_vb"),
            size: (vert_cap as u64) * stride as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh_pool_ib"),
            size: (idx_cap as u64) * MESH_INDEX_SIZE as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Reserve id 0 for the invalid handle; real buffers start at 1.
        let id = (self.buffers.len() as u32) + 1;
        self.buffers.push(BufferPair {
            vertex_buf,
            index_buf,
            vert_cap,
            idx_cap,
            stride,
        });
        id
    }

    fn buffer_index(buffer_id: u32) -> usize {
        debug_assert!(buffer_id != 0, "buffer_id 0 is reserved (invalid)");
        (buffer_id - 1) as usize
    }

    pub fn vertex_buffer(&self, buffer_id: u32) -> &wgpu::Buffer {
        &self.buffers[Self::buffer_index(buffer_id)].vertex_buf
    }

    pub fn index_buffer(&self, buffer_id: u32) -> &wgpu::Buffer {
        &self.buffers[Self::buffer_index(buffer_id)].index_buf
    }

    pub fn stride(&self, buffer_id: u32) -> u32 {
        self.buffers[Self::buffer_index(buffer_id)].stride
    }

    pub fn buffer_caps(&self, buffer_id: u32) -> (u32, u32) {
        let p = &self.buffers[Self::buffer_index(buffer_id)];
        (p.vert_cap, p.idx_cap)
    }

    /// All GPU buffer pairs: `(buffer_id, vert_cap, idx_cap, stride)`.
    pub fn enumerate_buffers(&self) -> Vec<(u32, u32, u32, u32)> {
        self.buffers
            .iter()
            .enumerate()
            .map(|(i, p)| ((i + 1) as u32, p.vert_cap, p.idx_cap, p.stride))
            .collect()
    }

    pub fn handle_fits(&self, handle: MeshHandle) -> bool {
        if !handle.is_valid() {
            return false;
        }
        let (vc, ic) = self.buffer_caps(handle.buffer_id);
        if handle.vert_offset.saturating_add(handle.vert_count) > vc
            || handle.idx_offset.saturating_add(handle.idx_count) > ic
        {
            return false;
        }
        let stride = handle.stride.max(1);
        let vb = self.vertex_buffer(handle.buffer_id);
        let ib = self.index_buffer(handle.buffer_id);
        let v_end = (handle.vert_offset as u64) * stride as u64
            + (handle.vert_count as u64) * stride as u64;
        let i_end = (handle.idx_offset as u64) * MESH_INDEX_SIZE as u64
            + (handle.idx_count as u64) * MESH_INDEX_SIZE as u64;
        v_end <= vb.size() && i_end <= ib.size()
    }

    /// Vertex byte range for `buffer.slice()`.
    pub fn vertex_byte_range(&self, handle: MeshHandle) -> std::ops::Range<u64> {
        let stride = handle.stride.max(1);
        let start = (handle.vert_offset * stride) as u64;
        start..start + (handle.vert_count * stride) as u64
    }

    /// Index byte range for `buffer.slice()`.
    pub fn index_byte_range(&self, handle: MeshHandle) -> std::ops::Range<u64> {
        let start = (handle.idx_offset * MESH_INDEX_SIZE) as u64;
        start..start + (handle.idx_count * MESH_INDEX_SIZE) as u64
    }
}

// ---------------------------------------------------------------------------

/// One tracked buffer inside the slab allocator (offset-only, no wgpu).
struct SlabInfo {
    buffer_id: u32,
    vert_cap: u32,
    idx_cap: u32,
    vert_used: u32,
    idx_used: u32,
    stride: u32,
}

/// Default offset-only allocator: append-only, no `free`. Packs many meshes
/// into shared buffers (one buffer per stride). Suitable for long-lived
/// meshes (studio models, HUD). Streaming meshes should use a
/// game-provided allocator with real `free`.
pub struct SlabMeshAllocator {
    slabs: Vec<SlabInfo>,
}

impl SlabMeshAllocator {
    pub fn new() -> Self {
        Self { slabs: Vec::new() }
    }
}

impl MeshAllocator for SlabMeshAllocator {
    fn allocate(
        &mut self,
        vert_count: u32,
        idx_count: u32,
        stride: u32,
    ) -> MeshHandle {
        for s in &mut self.slabs {
            if s.stride == stride
                && s.vert_used + vert_count <= s.vert_cap
                && s.idx_used + idx_count <= s.idx_cap
            {
                let h = MeshHandle {
                    buffer_id: s.buffer_id,
                    vert_offset: s.vert_used,
                    vert_count,
                    idx_offset: s.idx_used,
                    idx_count,
                    stride,
                };
                s.vert_used += vert_count;
                s.idx_used += idx_count;
                return h;
            }
        }
        // No room in known buffers — engine will grow the pool, call
        // `add_buffer`, and retry.
        MeshHandle::INVALID
    }

    fn free(&mut self, _handle: MeshHandle) {
        // Append-only: no reclamation. Long-lived meshes only.
    }

    fn add_buffer(
        &mut self,
        buffer_id: u32,
        vert_cap: u32,
        idx_cap: u32,
        stride: u32,
    ) {
        self.slabs.push(SlabInfo {
            buffer_id,
            vert_cap,
            idx_cap,
            vert_used: 0,
            idx_used: 0,
            stride,
        });
    }
}

impl Default for SlabMeshAllocator {
    fn default() -> Self {
        Self::new()
    }
}
