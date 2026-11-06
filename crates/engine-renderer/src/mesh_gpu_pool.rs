//! GPU mesh cache keyed by [`Mesh::content_id`] with reference counting.
//!
//! Each slot also records which [`MeshAllocator`] owns its GPU region so
//! that [`release`] can call `free` on the right allocator when refs hit
//! zero — fixing the GPU-memory leak under streaming (voxel chunks).
//!
//! Uploads are keyed by `(content_id, vertex_layout_key)` so different shaders
//! can pack the same CPU mesh with different `#@vertex` layouts.

use std::collections::HashMap;

use deerpunk_engine_common::resource::mesh_content::MESH_CONTENT_INVALID;

use crate::mesh_allocator::MeshHandle;

/// Id of an allocator in the renderer's allocator table. `0` is the
/// default built-in slab allocator.
pub type AllocatorId = u32;

#[derive(Clone, Debug)]
struct Slot {
    allocator_id: AllocatorId,
    refs: u32,
    /// `VertexLayoutSpec::layout_key()` → GPU sub-allocation.
    uploads: HashMap<u64, MeshHandle>,
}

/// Refcounted GPU uploads. Id `0` is invalid; ids are never reused after release.
#[derive(Default)]
pub struct MeshGpuPool {
    slots: HashMap<u64, Slot>,
    live_count: u32,
}

impl MeshGpuPool {
    pub fn live_count(&self) -> u32 {
        self.live_count
    }

    pub fn refs(&self, content_id: u64) -> u32 {
        if content_id == MESH_CONTENT_INVALID {
            return 0;
        }
        self.slots.get(&content_id).map(|s| s.refs).unwrap_or(0)
    }

    pub fn is_retained(&self, content_id: u64) -> bool {
        self.refs(content_id) > 0
    }

    /// Increment owners. Call when an entity (or other owner) starts using
    /// this mesh content. `allocator_id` selects which allocator owns the
    /// GPU region once uploaded.
    pub fn retain(&mut self, content_id: u64, allocator_id: AllocatorId) {
        assert_ne!(content_id, MESH_CONTENT_INVALID, "mesh content_id unset");
        if let Some(slot) = self.slots.get_mut(&content_id) {
            slot.refs += 1;
            return;
        }
        self.slots.insert(
            content_id,
            Slot {
                allocator_id,
                refs: 1,
                uploads: HashMap::new(),
            },
        );
        self.live_count += 1;
    }

    /// Decrement owners. Returns `(allocator_id, handles)` when refs hit zero
    /// so the caller can call `allocator.free(handle)` on each upload.
    pub fn release(&mut self, content_id: u64) -> Option<(AllocatorId, Vec<MeshHandle>)> {
        if content_id == MESH_CONTENT_INVALID {
            return None;
        }
        let slot = self.slots.get_mut(&content_id)?;
        assert!(slot.refs > 0, "mesh_release underflow content_id={content_id}");
        slot.refs -= 1;
        if slot.refs != 0 {
            return None;
        }
        let allocator_id = slot.allocator_id;
        let handles: Vec<MeshHandle> = slot
            .uploads
            .values()
            .copied()
            .filter(|h| h.is_valid())
            .collect();
        self.slots.remove(&content_id);
        self.live_count -= 1;
        if handles.is_empty() {
            None
        } else {
            Some((allocator_id, handles))
        }
    }

    /// Drop all slots (disconnect / map teardown). Returns the list of
    /// `(allocator_id, handle)` pairs the caller should `free`.
    pub fn drain_for_free(&mut self) -> Vec<(AllocatorId, MeshHandle)> {
        let mut out = Vec::new();
        for slot in self.slots.values() {
            for h in slot.uploads.values() {
                if h.is_valid() {
                    out.push((slot.allocator_id, *h));
                }
            }
        }
        self.slots.clear();
        self.live_count = 0;
        out
    }

    pub fn reset(&mut self) {
        self.slots.clear();
        self.live_count = 0;
    }

    pub fn handle(&self, content_id: u64, layout_key: u64) -> Option<MeshHandle> {
        if self.refs(content_id) == 0 {
            return None;
        }
        self.slots
            .get(&content_id)
            .and_then(|s| s.uploads.get(&layout_key).copied())
    }

    pub fn allocator_id(&self, content_id: u64) -> AllocatorId {
        self.slots
            .get(&content_id)
            .map(|s| s.allocator_id)
            .unwrap_or(0)
    }

    pub fn set_uploaded(&mut self, content_id: u64, layout_key: u64, handle: MeshHandle) {
        let slot = self
            .slots
            .get_mut(&content_id)
            .expect("set_uploaded: content_id not retained");
        slot.uploads.insert(layout_key, handle);
    }

    pub fn needs_upload(&self, content_id: u64, layout_key: u64) -> bool {
        self.slots
            .get(&content_id)
            .map(|s| !s.uploads.contains_key(&layout_key))
            .unwrap_or(false)
    }

    /// All retained meshes with a valid GPU handle (for debug / `r_showalloc`).
    pub fn live_handles(&self) -> Vec<(AllocatorId, MeshHandle)> {
        let mut out = Vec::new();
        for slot in self.slots.values() {
            for h in slot.uploads.values() {
                if h.is_valid() {
                    out.push((slot.allocator_id, *h));
                }
            }
        }
        out
    }
}
