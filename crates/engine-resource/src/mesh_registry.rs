//! Host-owned `Arc<Mesh>` storage; [`MeshHandle`] is safe across FFI.
//!
//! ECS entities reference meshes via [`MeshHandle`]. Refcounts are updated when
//! components change ([`Self::attach`] / [`Self::detach`] from client mesh
//! lifecycle on `client_world:set` / destroy). Slot reuse uses generation
//! bumps so stale handles resolve to `None`.

use std::collections::{HashMap, HashSet};
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
    /// Packed [`MeshHandle`] → entity reference count.
    handle_refs: HashMap<usize, u32>,
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
            handle_refs: HashMap::new(),
        }
    }

    /// Insert a mesh; refcount starts at **0** until [`Self::attach`].
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

    /// Entity (or other owner) started referencing this handle.
    pub fn attach(&mut self, handle: MeshHandle) {
        if !handle.is_valid() {
            return;
        }
        debug_assert!(
            self.get(handle).is_some(),
            "attach unknown or stale MeshHandle idx={} gen={}",
            handle.idx(),
            handle.gen()
        );
        *self.handle_refs.entry(handle.0).or_insert(0) += 1;
    }

    /// Owner dropped the handle. Frees the registry slot when refs hit zero.
    pub fn detach(&mut self, handle: MeshHandle) {
        if !handle.is_valid() {
            return;
        }
        let Some(count) = self.handle_refs.get_mut(&handle.0) else {
            return;
        };
        debug_assert!(*count > 0, "detach underflow idx={} gen={}", handle.idx(), handle.gen());
        *count -= 1;
        if *count > 0 {
            return;
        }
        self.handle_refs.remove(&handle.0);
        self.drop_slot(handle);
    }

    fn drop_slot(&mut self, handle: MeshHandle) {
        let idx = handle.idx() as usize;
        let Some(slot) = self.slots.get_mut(idx) else {
            return;
        };
        if slot.gen != handle.gen() || slot.mesh.is_none() {
            return;
        }
        slot.mesh = None;
        slot.gen = slot.gen.wrapping_add(1);
        self.free.push(idx);
    }

    /// Legacy full-world sweep (debug / recovery). Prefer [`Self::attach`] / [`Self::detach`].
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
            let index_still_referenced = live
                .iter()
                .any(|&h| MeshHandle(h).idx() as usize == idx);
            if index_still_referenced {
                continue;
            }
            slot.mesh = None;
            slot.gen = slot.gen.wrapping_add(1);
            self.free.push(idx);
            self.handle_refs.remove(&packed);
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
        self.handle_refs.clear();
    }
}

impl Default for MeshRegistry {
    fn default() -> Self {
        Self::new()
    }
}
