/// GPU mesh slab allocator.
///
/// Packs multiple meshes into shared vertex/index buffers to minimize
/// draw-call state changes. Large meshes that don't fit get dedicated slabs.

/// Handle returned by `MeshAllocator::allocate()`.
/// Identifies a mesh's location within the GPU buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MeshHandle {
    pub slab_id: u32,
    pub vertex_offset: u32,
    pub vertex_count: u32,
    pub index_offset: u32,
    pub index_count: u32,
}

impl MeshHandle {
    pub fn vertex_range(&self) -> std::ops::Range<u32> {
        self.vertex_offset..self.vertex_offset + self.vertex_count
    }

    pub fn index_range(&self) -> std::ops::Range<u32> {
        self.index_offset..self.index_offset + self.index_count
    }
}

/// One slab = one pair of GPU buffers (vertex + index).
/// Multiple meshes pack into the same slab until it's full.
struct Slab {
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    vertex_cap: u32,
    index_cap: u32,
    vertex_used: u32,
    index_used: u32,
}

pub struct MeshAllocator {
    slabs: Vec<Slab>,
    default_vertex_cap: u32,
    default_index_cap: u32,
}

/// Vertex format: pos+nrm+uv+atlas_rect+lm_uv+lm_styles+pad = 80 bytes (`MeshVertex` in gpu.rs).
pub const MESH_VERTEX_SIZE: u32 = 80;
pub const MESH_INDEX_SIZE: u32 = 4;

impl MeshAllocator {
    pub fn new(device: &wgpu::Device, initial_vertex_cap: u32, initial_index_cap: u32) -> Self {
        let mut allocator = Self {
            slabs: Vec::new(),
            default_vertex_cap: initial_vertex_cap,
            default_index_cap: initial_index_cap,
        };
        allocator.create_slab(device, initial_vertex_cap, initial_index_cap);
        allocator
    }

    fn create_slab(&mut self, device: &wgpu::Device, vertex_cap: u32, index_cap: u32) -> u32 {
        let vertex_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh_slab_vb"),
            size: (vertex_cap * MESH_VERTEX_SIZE) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh_slab_ib"),
            size: (index_cap * MESH_INDEX_SIZE) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let id = self.slabs.len() as u32;
        self.slabs.push(Slab {
            vertex_buf,
            index_buf,
            vertex_cap,
            index_cap,
            vertex_used: 0,
            index_used: 0,
        });
        id
    }

    /// Upload a mesh into the allocator's buffers.
    ///
    /// `vertices` — raw bytes of MeshVertex array.
    /// `indices` — &[u32] index data.
    pub fn allocate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: &[u8],
        indices: &[u32],
    ) -> MeshHandle {
        let vertex_count = (vertices.len() as u32) / MESH_VERTEX_SIZE;
        let index_count = indices.len() as u32;

        let slab_idx = self.find_or_create_slab(device, vertex_count, index_count);
        let slab = &mut self.slabs[slab_idx as usize];

        let vertex_offset = slab.vertex_used;
        let index_offset = slab.index_used;

        queue.write_buffer(
            &slab.vertex_buf,
            (vertex_offset * MESH_VERTEX_SIZE) as u64,
            vertices,
        );

        // Indices stay mesh-local: draws bind a sliced VB/IB, so index 0 = first
        // vertex of this mesh (not an absolute slab vertex).
        queue.write_buffer(
            &slab.index_buf,
            (index_offset * MESH_INDEX_SIZE) as u64,
            bytemuck::cast_slice(indices),
        );

        slab.vertex_used += vertex_count;
        slab.index_used += index_count;

        MeshHandle {
            slab_id: slab_idx,
            vertex_offset,
            vertex_count,
            index_offset,
            index_count,
        }
    }

    fn find_or_create_slab(&mut self, device: &wgpu::Device, vertices: u32, indices: u32) -> u32 {
        for (i, slab) in self.slabs.iter().enumerate() {
            if slab.vertex_used + vertices <= slab.vertex_cap
                && slab.index_used + indices <= slab.index_cap
            {
                return i as u32;
            }
        }
        let cap_v = vertices.max(self.default_vertex_cap);
        let cap_i = indices.max(self.default_index_cap);
        self.create_slab(device, cap_v, cap_i)
    }

    /// Vertex buffer slice for a mesh handle.
    pub fn vertex_buffer(&self, handle: MeshHandle) -> &wgpu::Buffer {
        &self.slabs[handle.slab_id as usize].vertex_buf
    }

    /// Index buffer slice for a mesh handle.
    pub fn index_buffer(&self, handle: MeshHandle) -> &wgpu::Buffer {
        &self.slabs[handle.slab_id as usize].index_buf
    }

    /// Vertex byte range for `buffer.slice()`.
    pub fn vertex_byte_range(&self, handle: MeshHandle) -> std::ops::Range<u64> {
        let start = (handle.vertex_offset * MESH_VERTEX_SIZE) as u64;
        start..start + (handle.vertex_count * MESH_VERTEX_SIZE) as u64
    }

    /// Index byte range for `buffer.slice()`.
    pub fn index_byte_range(&self, handle: MeshHandle) -> std::ops::Range<u64> {
        let start = (handle.index_offset * MESH_INDEX_SIZE) as u64;
        start..start + (handle.index_count * MESH_INDEX_SIZE) as u64
    }

    /// Reset all slabs for next frame (keeps buffers, resets counters).
    pub fn reset(&mut self) {
        for slab in &mut self.slabs {
            slab.vertex_used = 0;
            slab.index_used = 0;
        }
    }
}
