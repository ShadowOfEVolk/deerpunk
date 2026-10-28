//! Host-owned `Arc<Mesh>` storage; [`MeshHandle`] is safe across FFI.
//!
//! Asset-store model: meshes live here until no entity references them.
//! Lifetime is managed by [`MeshRegistry::sweep`] (called from the client
//! mesh-lifecycle sync each frame), **not** by manual `release` from game code.
//! A stale handle (slot reused after the mesh was swept) resolves to `None`
//! via the embedded generation, never to another mesh.

use std::collections::HashSet;
use std::sync::Arc;

use super::mesh::Mesh;
use super::mesh_handle::MeshHandle;

struct Slot {
    gen: u32,
    mesh: Option<Arc<Mesh>>,
}

pub struct MeshRegistry {
    slots: Vec<Slot>,
    free: Vec<usize>,
    next: usize,
}

impl MeshRegistry {
    pub fn new() -> Self {
        // Slot 0 is reserved so `MeshHandle::INVALID` (idx=0) never resolves.
        Self {
            slots: vec![Slot {
                gen: 0,
                mesh: None,
            }],
            free: Vec::new(),
            next: 1,
        }
    }

    /// Insert a mesh; the returned handle is valid until the next [`sweep`]
    /// finds no live references to it.
    pub fn insert(&mut self, mesh: Mesh) -> MeshHandle {
        let mut mesh = mesh;
        mesh.ensure_local_bounds();
        let arc = Arc::new(mesh);
        let idx = if let Some(i) = self.free.pop() {
            self.slots[i].mesh = Some(arc);
            i
        } else {
            let i = self.next;
            self.next += 1;
            if i >= self.slots.len() {
                self.slots.push(Slot {
                    gen: 0,
                    mesh: Some(arc),
                });
            } else {
                self.slots[i] = Slot {
                    gen: 0,
                    mesh: Some(arc),
                };
            }
            i
        };
        MeshHandle::from_parts(idx as u32, self.slots[idx].gen)
    }

    pub fn get(&self, handle: MeshHandle) -> Option<&Arc<Mesh>> {
        if !handle.is_valid() {
            return None;
        }
        let slot = self.slots.get(handle.idx() as usize)?;
        if slot.gen != handle.gen() {
            return None;
        }
        slot.mesh.as_ref()
    }

    /// `Arc<Mesh>` content id (used for GPU pool dedup). `0` if not present.
    pub fn content_id(&self, handle: MeshHandle) -> u64 {
        self.get(handle)
            .map(|m| m.content_id())
            .unwrap_or(0)
    }

    /// Free every slot whose packed handle is **not** in `live`.
    ///
    /// Bumps the slot generation so any stale handle (e.g. held in a game cache
    /// that outlived the entity) resolves to `None` instead of a reused mesh.
    /// Called once per frame from the client mesh-lifecycle sync, before
    /// `RenderExtractor::extract`.
    pub fn sweep(&mut self, live: &HashSet<usize>) {
        for idx in 1..self.slots.len() {
            let slot = &mut self.slots[idx];
            if slot.mesh.is_none() {
                continue;
            }
            let packed = MeshHandle::from_parts(idx as u32, slot.gen).0;
            if live.contains(&packed) {
                continue;
            }
            // Stale handles (old generation, same idx) still appear in `live` after remesh;
            // do not reuse the slot until every referencing component is gone.
            let index_still_referenced = live
                .iter()
                .any(|&h| MeshHandle(h).idx() as usize == idx);
            if index_still_referenced {
                continue;
            }
            slot.mesh = None;
            slot.gen = slot.gen.wrapping_add(1);
            self.free.push(idx);
        }
    }

    /// Drop everything (level change / shutdown). Generations are reset, so
    /// any handle held elsewhere becomes invalid.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.slots.push(Slot {
            gen: 0,
            mesh: None,
        });
        self.free.clear();
        self.next = 1;
    }
}

impl Default for MeshRegistry {
    fn default() -> Self {
        Self::new()
    }
}
