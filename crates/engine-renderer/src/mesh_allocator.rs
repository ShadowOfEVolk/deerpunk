//! GPU mesh buffer pool + offset-only slab allocator.
//!
//! The engine owns the GPU vertex/index buffers ([`MeshBufferPool`]); an
//! [`SlabMeshAllocator`] is an offset-only bookkeeper implementing
//! `deerpunk_engine_resource::MeshAllocator`. It packs many meshes into
//! shared buffers (one buffer pair per stride) with best-fit free-lists
//! and coalescing on `free`, so streaming workloads reuse reclaimed
//! ranges instead of growing GPU memory monotonically. Games that want a
//! custom allocation policy can still register their own allocator via
//! the `v1:mesh:register_allocator` FFI.

use deerpunk_engine_common::resource::mesh_allocator as res;

pub use res::MeshAllocHandle as MeshHandle;
pub use res::MeshAllocator;

/// Vertex format: pos+nrm+uv+atlas_rect+lm_uv+lm_styles+pad = 80 bytes (`MeshVertex` in gpu.rs).
pub const MESH_VERTEX_SIZE: u32 = 80;
/// Skinned studio mesh: pos, nrm, uv, joint indices, weights.
pub const SKINNED_MESH_VERTEX_SIZE: u32 = 64;
pub const MESH_INDEX_SIZE: u32 = 4;

/// Smallest slab the pool creates (vertices) — keeps tiny meshes cheap.
pub const MIN_SLAB_VERT_CAP: u32 = 4096;
/// Smallest slab the pool creates (indices) — keeps tiny meshes cheap.
pub const MIN_SLAB_IDX_CAP: u32 = 8192;

/// Size class for new slabs: `request` plus 25% headroom, rounded up to a
/// power of two, never below `min_cap`. Meshes larger than any class get a
/// class of their own, so oversized requests still fit.
pub fn slab_size_class(request: u32, min_cap: u32) -> u32 {
    let want = request.saturating_add(request / 4).max(min_cap);
    // Above the last u32 power of two the request becomes its own class.
    want.checked_next_power_of_two().unwrap_or(want)
}

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
}

impl MeshBufferPool {
    pub fn new() -> Self {
        Self {
            buffers: Vec::new(),
        }
    }

    /// Create a new buffer pair for `stride` and return its id. The caller
    /// (an allocator) is informed via its `add_buffer` trait method. Caps
    /// are used as given — size classes are decided by the caller (see
    /// [`slab_size_class`]).
    pub fn add(&mut self, device: &wgpu::Device, vert_cap: u32, idx_cap: u32, stride: u32) -> u32 {
        let vert_cap = vert_cap.max(1);
        let idx_cap = idx_cap.max(1);
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

impl Default for MeshBufferPool {
    /// Empty pool; the first `add` creates the first buffer pair.
    fn default() -> Self {
        Self::new()
    }
}
// ---------------------------------------------------------------------------

/// One tracked buffer inside the slab allocator (offset-only, no wgpu).
///
/// Vertex and index space are tracked independently per buffer (they live
/// in separate wgpu buffers keyed by the same `buffer_id`): each side is a
/// sorted, non-overlapping free-list of `(offset, count)` ranges.
struct SlabInfo {
    buffer_id: u32,
    vert_cap: u32,
    idx_cap: u32,
    stride: u32,
    /// Sorted free ranges in vertex space.
    vert_free: Vec<(u32, u32)>,
    /// Sorted free ranges in index space.
    idx_free: Vec<(u32, u32)>,
}

impl SlabInfo {
    fn new(buffer_id: u32, vert_cap: u32, idx_cap: u32, stride: u32) -> Self {
        Self {
            buffer_id,
            vert_cap,
            idx_cap,
            stride,
            vert_free: vec![(0, vert_cap)],
            idx_free: vec![(0, idx_cap)],
        }
    }

    /// Best-fit plan for one side: smallest free range that fits `need`.
    /// Returns `(index into free list, offset)`; `usize::MAX` marks a
    /// zero-size request that consumes nothing.
    fn plan_side(free: &[(u32, u32)], need: u32, cap: u32) -> Option<(usize, u32)> {
        if need == 0 {
            return Some((usize::MAX, 0));
        }
        free.iter()
            .enumerate()
            .filter(|&(_, &(off, count))| count >= need && off.saturating_add(need) <= cap)
            .min_by_key(|&(_, &(_, count))| count)
            .map(|(i, &(off, _))| (i, off))
    }

    /// Commit a plan produced by [`SlabInfo::plan_side`]: split the chosen
    /// free range, removing it when fully consumed.
    fn commit_side(free: &mut Vec<(u32, u32)>, slot: usize, off: u32, need: u32) {
        if slot == usize::MAX {
            return;
        }
        let (_, count) = free[slot];
        let remaining = count - need;
        if remaining == 0 {
            free.remove(slot);
        } else {
            free[slot] = (off + need, remaining);
        }
    }

    /// Return `(off, count)` to a free-list, coalescing with adjacent
    /// ranges so the list stays sorted, non-overlapping and compact.
    fn free_side(free: &mut Vec<(u32, u32)>, off: u32, count: u32) {
        if count == 0 {
            return;
        }
        let mut start = off;
        let end = off + count;
        // Coalesce with a previous range ending exactly at `off`.
        let prev = free.partition_point(|&(o, _)| o < off);
        if prev > 0 {
            let (po, pc) = free[prev - 1];
            if po + pc == start {
                start = po;
                free.remove(prev - 1);
            }
        }
        // Coalesce with a following range starting exactly at `end`.
        let next = free.partition_point(|&(o, _)| o < start);
        let mut stop = end;
        if next < free.len() {
            let (no, nc) = free[next];
            if no == end {
                stop = no + nc;
                free.remove(next);
            }
        }
        free.insert(next, (start, stop - start));
    }
}

/// Default offset-only allocator with free-list reuse. Packs many meshes
/// into shared buffers (one buffer pair per stride); `free` returns ranges
/// to the slab's free-lists (with coalescing) and later allocations reuse
/// them (best-fit), so streaming workloads keep GPU memory bounded.
/// Vertex and index ranges are always carved from the same slab, keeping
/// `MeshHandle` a single `(buffer_id, ranges)` pair.
pub struct SlabMeshAllocator {
    slabs: Vec<SlabInfo>,
}

impl SlabMeshAllocator {
    pub fn new() -> Self {
        Self { slabs: Vec::new() }
    }
}

impl MeshAllocator for SlabMeshAllocator {
    fn allocate(&mut self, vert_count: u32, idx_count: u32, stride: u32) -> MeshHandle {
        for s in &mut self.slabs {
            if s.stride != stride {
                continue;
            }
            // Plan both sides before committing either — a mesh must live
            // in a single buffer pair.
            if let (Some((v_slot, v_off)), Some((i_slot, i_off))) = (
                SlabInfo::plan_side(&s.vert_free, vert_count, s.vert_cap),
                SlabInfo::plan_side(&s.idx_free, idx_count, s.idx_cap),
            ) {
                SlabInfo::commit_side(&mut s.vert_free, v_slot, v_off, vert_count);
                SlabInfo::commit_side(&mut s.idx_free, i_slot, i_off, idx_count);
                return MeshHandle {
                    buffer_id: s.buffer_id,
                    vert_offset: v_off,
                    vert_count,
                    idx_offset: i_off,
                    idx_count,
                    stride,
                };
            }
        }
        // No room in known buffers — engine will grow the pool, call
        // `add_buffer`, and retry.
        MeshHandle::INVALID
    }

    fn free(&mut self, handle: MeshHandle) {
        let Some(s) = self
            .slabs
            .iter_mut()
            .find(|s| s.buffer_id == handle.buffer_id)
        else {
            return;
        };
        SlabInfo::free_side(&mut s.vert_free, handle.vert_offset, handle.vert_count);
        SlabInfo::free_side(&mut s.idx_free, handle.idx_offset, handle.idx_count);
    }

    fn add_buffer(&mut self, buffer_id: u32, vert_cap: u32, idx_cap: u32, stride: u32) {
        self.slabs
            .push(SlabInfo::new(buffer_id, vert_cap, idx_cap, stride));
    }
}

impl Default for SlabMeshAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(cap: u32) -> SlabMeshAllocator {
        let mut a = SlabMeshAllocator::new();
        a.add_buffer(1, cap, cap * 2, 80);
        a
    }

    #[test]
    fn alloc_reuses_freed_range() {
        let mut a = setup(100);
        let h1 = a.allocate(40, 80, 80);
        let h2 = a.allocate(40, 80, 80);
        assert!(h1.is_valid() && h2.is_valid());
        a.free(h1);
        // Must land exactly in h1's reclaimed range, not new space.
        let h3 = a.allocate(40, 80, 80);
        assert_eq!(h3.vert_offset, h1.vert_offset);
        assert_eq!(h3.idx_offset, h1.idx_offset);
        // Slab still has room for one more before exhaustion.
        assert!(a.allocate(20, 40, 80).is_valid());
    }

    #[test]
    fn free_coalesces_adjacent_ranges() {
        let mut a = setup(120);
        let h1 = a.allocate(40, 80, 80);
        let h2 = a.allocate(40, 80, 80);
        let h3 = a.allocate(40, 80, 80);
        a.free(h2);
        a.free(h1);
        // h1+h2 coalesce into one 80-wide hole at offset 0.
        let h4 = a.allocate(80, 160, 80);
        assert!(h4.is_valid());
        assert_eq!(h4.vert_offset, 0);
        // Only h3's space remains: a full-slab request must fail now.
        assert!(!a.allocate(120, 240, 80).is_valid());
        a.free(h3);
        a.free(h4);
        // Everything coalesced back: the whole slab is one free range.
        let h5 = a.allocate(120, 240, 80);
        assert!(h5.is_valid());
        assert_eq!(h5.vert_offset, 0);
        assert_eq!(h5.idx_offset, 0);
    }

    #[test]
    fn interleaved_alloc_free_never_overlaps() {
        let mut a = setup(64);
        let mut live: Vec<MeshHandle> = Vec::new();
        for i in 0..8 {
            let h = a.allocate(8, 16, 80);
            assert!(h.is_valid(), "alloc {i} failed");
            if i % 2 == 0 {
                a.free(h);
            } else {
                live.push(h);
            }
        }
        // Squeeze new allocations into the freed holes.
        for i in 0..4 {
            let h = a.allocate(4, 8, 80);
            assert!(h.is_valid(), "refill alloc {i} failed");
            live.push(h);
        }
        for (i, x) in live.iter().enumerate() {
            for y in live.iter().skip(i + 1) {
                assert!(
                    x.vertex_range().is_empty()
                        || y.vertex_range().is_empty()
                        || !x.vertex_range().contains(&y.vertex_range().start)
                            && !y.vertex_range().contains(&x.vertex_range().start),
                    "vertex ranges overlap: {:?} vs {:?}",
                    x.vertex_range(),
                    y.vertex_range()
                );
                assert!(
                    x.index_range().is_empty()
                        || y.index_range().is_empty()
                        || !x.index_range().contains(&y.index_range().start)
                            && !y.index_range().contains(&x.index_range().start),
                    "index ranges overlap: {:?} vs {:?}",
                    x.index_range(),
                    y.index_range()
                );
            }
        }
    }

    #[test]
    fn oversized_request_returns_invalid() {
        let mut a = setup(64);
        assert!(!a.allocate(65, 8, 80).is_valid());
        assert!(!a.allocate(8, 129, 80).is_valid()); // idx cap is 128
                                                     // A request larger than the whole slab is still reportable via
                                                     // INVALID; the caller grows the pool with a fitting size class.
        assert!(!a.allocate(1_000_000, 2_000_000, 80).is_valid());
        // Slab is untouched and still usable.
        assert!(a.allocate(64, 128, 80).is_valid());
    }

    #[test]
    fn exact_full_slab_allocation() {
        let mut a = setup(64);
        let h = a.allocate(64, 128, 80);
        assert!(h.is_valid());
        assert!(!a.allocate(1, 1, 80).is_valid());
        a.free(h);
        assert!(a.allocate(64, 128, 80).is_valid());
    }

    #[test]
    fn free_of_invalid_and_unknown_handles_is_ignored() {
        let mut a = setup(64);
        a.free(MeshHandle::INVALID);
        a.free(MeshHandle {
            buffer_id: 99,
            vert_offset: 0,
            vert_count: 4,
            idx_offset: 0,
            idx_count: 4,
            stride: 80,
        });
        // Slab untouched.
        assert!(a.allocate(64, 128, 80).is_valid());
    }

    #[test]
    fn size_class_ladder() {
        assert_eq!(slab_size_class(0, MIN_SLAB_VERT_CAP), 4096);
        assert_eq!(slab_size_class(1, MIN_SLAB_VERT_CAP), 4096);
        assert_eq!(slab_size_class(4_000, MIN_SLAB_VERT_CAP), 8192);
        assert_eq!(slab_size_class(300_000, MIN_SLAB_VERT_CAP), 524_288);
        // Huge meshes get a class of their own — no fixed ceiling.
        assert_eq!(slab_size_class(1_000_000, MIN_SLAB_VERT_CAP), 2_097_152);
        assert!(slab_size_class(1_000_000, MIN_SLAB_VERT_CAP) >= 1_000_000);
        assert_eq!(slab_size_class(0, MIN_SLAB_IDX_CAP), 8192);
        assert!(slab_size_class(u32::MAX - 10, MIN_SLAB_VERT_CAP) >= u32::MAX - 10);
    }
}
