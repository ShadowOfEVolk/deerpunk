//! GPU mesh pool usage (live handles + buffer caps) for `r_showalloc`.

use std::collections::BTreeMap;

use crate::mesh_allocator::{MeshBufferPool, MESH_INDEX_SIZE};
use crate::mesh_gpu_pool::{AllocatorId, MeshGpuPool};

#[derive(Clone, Debug, Default)]
pub struct SideView {
    pub cap: u32,
    /// Merged occupied intervals `[offset, offset+len)` in vertex or index units.
    pub used: Vec<(u32, u32)>,
}

impl SideView {
    pub fn used_units(&self) -> u32 {
        self.used.iter().map(|&(_, len)| len).sum()
    }

    pub fn free_units(&self) -> u32 {
        self.cap.saturating_sub(self.used_units())
    }
}

#[derive(Clone, Debug)]
pub struct BufferView {
    pub buffer_id: u32,
    pub stride: u32,
    pub allocator_id: AllocatorId,
    pub verts: SideView,
    pub indices: SideView,
}

#[derive(Clone, Debug, Default)]
pub struct MeshAllocReport {
    pub buffers: Vec<BufferView>,
    pub live_mesh_count: u32,
}

pub fn build_report(
    pool: &MeshBufferPool,
    gpu: &MeshGpuPool,
    allocator_filter: Option<AllocatorId>,
) -> MeshAllocReport {
    let mut by_buffer: BTreeMap<u32, BufferScratch> = BTreeMap::new();

    for (alloc_id, handle) in gpu.live_handles() {
        if allocator_filter.is_some_and(|f| f != alloc_id) {
            continue;
        }
        if !handle.is_valid() {
            continue;
        }
        let entry = by_buffer.entry(handle.buffer_id).or_insert_with(|| {
            let (vc, ic) = pool.buffer_caps(handle.buffer_id);
            BufferScratch {
                stride: pool.stride(handle.buffer_id),
                allocator_id: alloc_id,
                vert_cap: vc,
                idx_cap: ic,
                vert_used: Vec::new(),
                idx_used: Vec::new(),
            }
        });
        entry.allocator_id = alloc_id;
        entry
            .vert_used
            .push((handle.vert_offset, handle.vert_count));
        entry
            .idx_used
            .push((handle.idx_offset, handle.idx_count));
    }

    for (buffer_id, vert_cap, idx_cap, stride) in pool.enumerate_buffers() {
        if allocator_filter.is_some() && !by_buffer.contains_key(&buffer_id) {
            // Still list empty GPU buffers when filtering? Skip unless has handles.
            continue;
        }
        by_buffer.entry(buffer_id).or_insert_with(|| BufferScratch {
            stride,
            allocator_id: 0,
            vert_cap,
            idx_cap,
            vert_used: Vec::new(),
            idx_used: Vec::new(),
        });
    }

    let live_mesh_count = gpu.live_count();
    let buffers = by_buffer
        .into_iter()
        .map(|(buffer_id, s)| BufferView {
            buffer_id,
            stride: s.stride,
            allocator_id: s.allocator_id,
            verts: SideView {
                cap: s.vert_cap,
                used: merge_intervals(&s.vert_used),
            },
            indices: SideView {
                cap: s.idx_cap,
                used: merge_intervals(&s.idx_used),
            },
        })
        .collect();

    MeshAllocReport {
        buffers,
        live_mesh_count,
    }
}

struct BufferScratch {
    stride: u32,
    allocator_id: AllocatorId,
    vert_cap: u32,
    idx_cap: u32,
    vert_used: Vec<(u32, u32)>,
    idx_used: Vec<(u32, u32)>,
}

fn merge_intervals(raw: &[(u32, u32)]) -> Vec<(u32, u32)> {
    if raw.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<(u32, u32)> = raw
        .iter()
        .filter(|&(_, len)| *len > 0)
        .copied()
        .collect();
    sorted.sort_by_key(|&(off, _)| off);
    let mut out: Vec<(u32, u32)> = Vec::new();
    for (off, len) in sorted {
        let end = off.saturating_add(len);
        if let Some((last_off, last_len)) = out.last_mut() {
            let last_end = last_off.saturating_add(*last_len);
            if off <= last_end {
                if end > last_end {
                    *last_len = end - *last_off;
                }
                continue;
            }
        }
        out.push((off, len));
    }
    out
}

pub fn index_unit_bytes() -> u32 {
    MESH_INDEX_SIZE
}
