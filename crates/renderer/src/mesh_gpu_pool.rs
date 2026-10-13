//! GPU mesh cache keyed by [`Mesh::content_id`] with reference counting.

use std::collections::HashMap;

use deerpunk_engine_common::resource::mesh_content::MESH_CONTENT_INVALID;

use crate::mesh_allocator::MeshHandle;

#[derive(Clone, Copy, Debug)]
struct Slot {
    handle: Option<MeshHandle>,
    refs: u32,
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

    /// Increment owners. Call when an entity (or other owner) starts using this mesh content.
    pub fn retain(&mut self, content_id: u64) {
        assert_ne!(content_id, MESH_CONTENT_INVALID, "mesh content_id unset");
        if let Some(slot) = self.slots.get_mut(&content_id) {
            slot.refs += 1;
            return;
        }
        self.slots.insert(
            content_id,
            Slot {
                handle: None,
                refs: 1,
            },
        );
        self.live_count += 1;
    }

    /// Decrement owners. Drops cache entry when refs hit zero (slab bytes are not reclaimed).
    pub fn release(&mut self, content_id: u64) {
        if content_id == MESH_CONTENT_INVALID {
            return;
        }
        let Some(slot) = self.slots.get_mut(&content_id) else {
            return;
        };
        assert!(slot.refs > 0, "mesh_release underflow content_id={content_id}");
        slot.refs -= 1;
        if slot.refs == 0 {
            self.slots.remove(&content_id);
            self.live_count -= 1;
        }
    }

    /// Drop all slots (disconnect / map teardown). Does not shrink GPU slabs.
    pub fn reset(&mut self) {
        self.slots.clear();
        self.live_count = 0;
    }

    pub fn handle(&self, content_id: u64) -> Option<MeshHandle> {
        self.slots
            .get(&content_id)
            .and_then(|s| s.handle)
            .filter(|_| self.refs(content_id) > 0)
    }

    pub fn set_uploaded(&mut self, content_id: u64, handle: MeshHandle) {
        let slot = self
            .slots
            .get_mut(&content_id)
            .expect("set_uploaded: content_id not retained");
        slot.handle = Some(handle);
    }

    pub fn needs_upload(&self, content_id: u64) -> bool {
        self.slots
            .get(&content_id)
            .map(|s| s.handle.is_none())
            .unwrap_or(false)
    }
}
