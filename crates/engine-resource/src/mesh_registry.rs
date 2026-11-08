//! Host-owned `Arc<Mesh>` storage; [`MeshHandle`] is safe across FFI.
//!
//! ECS entities reference meshes via [`MeshHandle`]. Refcounts are updated when
//! components change ([`Self::attach`] / [`Self::detach`] from client mesh
//! lifecycle on `client_world:set` / destroy). Slot reuse uses generation
//! bumps so stale handles resolve to `None`.

use std::alloc::{alloc, handle_alloc_error, Layout};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

fn alloc_vec<T>(cap: usize) -> Vec<T> {
    let layout = Layout::array::<T>(cap).expect("mesh arena layout");
    let ptr = unsafe { alloc(layout) };
    if ptr.is_null() {
        handle_alloc_error(layout);
    }
    unsafe { Vec::from_raw_parts(ptr as *mut T, 0, cap) }
}

/// Power-of-two bins for CPU mesh buffers. Blocks are recycled, not returned to the system allocator.
const ARENA_MIN_CLASS: usize = 6; // 64
const ARENA_MAX_CLASS: usize = 20; // 1 MiB
const ARENA_BINS: usize = ARENA_MAX_CLASS - ARENA_MIN_CLASS + 1;

struct CpuMeshArena {
    bytes: [Vec<Vec<u8>>; ARENA_BINS],
    indices: [Vec<Vec<u32>>; ARENA_BINS],
}

impl CpuMeshArena {
    fn new() -> Self {
        Self {
            bytes: std::array::from_fn(|_| Vec::new()),
            indices: std::array::from_fn(|_| Vec::new()),
        }
    }

    fn class(len: usize) -> Option<usize> {
        if len == 0 {
            return None;
        }
        let cap = len.next_power_of_two().max(1 << ARENA_MIN_CLASS);
        let class = cap.trailing_zeros() as usize;
        if class > ARENA_MAX_CLASS {
            return None;
        }
        Some(class - ARENA_MIN_CLASS)
    }

    fn refit_bytes(&mut self, buf: &mut Vec<u8>, len: usize) {
        if buf.len() == len {
            return;
        }
        if Self::class(len) == Self::class(buf.capacity().max(1)) && buf.capacity() >= len {
            buf.resize(len, 0);
            return;
        }
        let mut fresh = self.take_bytes(len);
        fresh.resize(len, 0);
        let old = std::mem::replace(buf, fresh);
        self.recycle_bytes(old);
    }

    fn refit_indices(&mut self, buf: &mut Vec<u32>, len: usize) {
        if buf.len() == len {
            return;
        }
        if Self::class(len) == Self::class(buf.capacity().max(1)) && buf.capacity() >= len {
            buf.resize(len, 0);
            return;
        }
        let mut fresh = self.take_indices(len);
        fresh.resize(len, 0);
        let old = std::mem::replace(buf, fresh);
        self.recycle_indices(old);
    }

    fn adopt(&mut self, mesh: &mut Mesh) {
        if !mesh.vertex_bytes.is_empty() {
            let mut fresh = self.take_bytes(mesh.vertex_bytes.len());
            fresh.extend_from_slice(&mesh.vertex_bytes);
            mesh.vertex_bytes = fresh;
        }
        if !mesh.indices.is_empty() {
            let mut fresh = self.take_indices(mesh.indices.len());
            fresh.extend_from_slice(&mesh.indices);
            mesh.indices = fresh;
        }
    }

    fn recycle_mesh(&mut self, mesh: Mesh) {
        self.recycle_bytes(mesh.vertex_bytes);
        self.recycle_indices(mesh.indices);
    }

    fn take_bytes(&mut self, len: usize) -> Vec<u8> {
        let Some(class) = Self::class(len) else {
            return Vec::with_capacity(len);
        };
        self.bytes[class]
            .pop()
            .unwrap_or_else(|| alloc_vec(1 << (class + ARENA_MIN_CLASS)))
    }

    fn take_indices(&mut self, len: usize) -> Vec<u32> {
        let Some(class) = Self::class(len) else {
            return Vec::with_capacity(len);
        };
        self.indices[class]
            .pop()
            .unwrap_or_else(|| alloc_vec(1 << (class + ARENA_MIN_CLASS)))
    }

    fn recycle_bytes(&mut self, mut buf: Vec<u8>) {
        let Some(class) = Self::class(buf.capacity()) else {
            return;
        };
        if buf.capacity() != 1 << (class + ARENA_MIN_CLASS) {
            return;
        }
        buf.clear();
        self.bytes[class].push(buf);
    }

    fn recycle_indices(&mut self, mut buf: Vec<u32>) {
        let Some(class) = Self::class(buf.capacity()) else {
            return;
        };
        if buf.capacity() != 1 << (class + ARENA_MIN_CLASS) {
            return;
        }
        buf.clear();
        self.indices[class].push(buf);
    }
}

use super::mesh::Mesh;
use super::mesh_handle::MeshHandle;

struct Slot {
    gen: u32,
    mesh: Option<Arc<Mesh>>,
}

pub struct MeshRegistry {
    slots: Vec<Slot>,
    /// `Mesh::gpu_revision` by slot index. Hot upload check reads this, not the `Mesh`.
    /// `0` means the slot is empty.
    mesh_revisions: Vec<u64>,
    free: Vec<usize>,
    next: usize,
    /// Packed [`MeshHandle`] → entity reference count.
    handle_refs: HashMap<usize, u32>,
    /// Recycled CPU vertex/index blocks for packed meshes.
    cpu_arena: CpuMeshArena,
}

impl MeshRegistry {
    pub fn new() -> Self {
        // Slot 0 is reserved so `MeshHandle::INVALID` (idx=0) never resolves.
        Self {
            slots: vec![Slot {
                gen: 0,
                mesh: None,
            }],
            mesh_revisions: vec![0],
            free: Vec::new(),
            next: 1,
            handle_refs: HashMap::new(),
            cpu_arena: CpuMeshArena::new(),
        }
    }

    /// Replace CPU geometry in an existing slot. Keeps [`MeshHandle`], `content_id`,
    /// and `allocator_id`. Bumps [`Mesh::gpu_revision`] so the renderer re-uploads.
    /// Does not allocate or free GPU memory.
    pub fn replace(&mut self, handle: MeshHandle, mut mesh: Mesh) -> bool {
        if !handle.is_valid() {
            return false;
        }
        let idx = handle.idx() as usize;
        let Some(slot) = self.slots.get(idx) else {
            return false;
        };
        if slot.gen != handle.gen() {
            return false;
        }
        let Some(old) = slot.mesh.as_ref() else {
            return false;
        };
        mesh.content_id = old.content_id;
        mesh.allocator_id = old.allocator_id;
        mesh.gpu_revision = old.gpu_revision.wrapping_add(1).max(1);
        assert!(
            mesh.has_local_bounds(),
            "mesh replace: local AABB is required (set at upload)"
        );
        let prev = self.slots[idx].mesh.take();
        if let Some(prev) = prev {
            self.recycle_arc(prev);
        }
        self.cpu_arena.adopt(&mut mesh);
        let revision = mesh.gpu_revision;
        self.slots[idx].mesh = Some(Arc::new(mesh));
        self.set_mesh_revision(idx, revision);
        true
    }

    /// Resize the existing vertex and index storage. Same `Mesh` / `content_id`.
    /// Reallocates only when the requested length differs. Does not mark the GPU dirty.
    ///
    /// Pointers are valid until the next registry call that resizes this slot.
    ///
    /// # Safety invariant
    /// Other `Arc` clones (render snapshot) alias this `Mesh`. Vertex bytes are only
    /// read on this thread during extract, which has already copied them out before
    /// the next gamelib frame. Do not call this while an upload is reading the buffer.
    pub fn resize_buffers(
        &mut self,
        handle: MeshHandle,
        vertex_bytes: usize,
        index_count: usize,
    ) -> Option<(*mut u8, *mut u32)> {
        let ptr = self.mesh_mut_slot(handle)?;
        let mesh = unsafe { &mut *ptr };
        self.cpu_arena.refit_bytes(&mut mesh.vertex_bytes, vertex_bytes);
        self.cpu_arena.refit_indices(&mut mesh.indices, index_count);
        let verts = mesh.vertex_bytes.as_mut_ptr();
        let indices = mesh.indices.as_mut_ptr();
        Some((verts, indices))
    }

    /// Bump [`Mesh::gpu_revision`] so the renderer re-uploads. No GPU calls.
    pub fn mark_dirty(&mut self, handle: MeshHandle) -> bool {
        let revision = {
            let Some(mesh) = self.mesh_mut(handle) else {
                return false;
            };
            mesh.gpu_revision = mesh.gpu_revision.wrapping_add(1).max(1);
            mesh.gpu_revision
        };
        self.set_mesh_revision(handle.idx() as usize, revision);
        true
    }

    /// Hot-path identity of the CPU mesh: generation in the high 32 bits,
    /// [`Mesh::gpu_revision`] in the low 32. `0` when the handle is stale or empty.
    /// Does not touch the mesh allocation. A recycled slot gets a new generation,
    /// so a leftover GPU record cannot match.
    pub fn mesh_revision(&self, handle: MeshHandle) -> u64 {
        if !handle.is_valid() {
            return 0;
        }
        let idx = handle.idx() as usize;
        let Some(slot) = self.slots.get(idx) else {
            return 0;
        };
        if slot.gen != handle.gen() || slot.mesh.is_none() {
            return 0;
        }
        self.mesh_revisions.get(idx).copied().unwrap_or(0)
    }

    fn set_mesh_revision(&mut self, idx: usize, revision: u64) {
        if self.mesh_revisions.len() <= idx {
            self.mesh_revisions.resize(idx + 1, 0);
        }
        let gen = self.slots.get(idx).map(|slot| slot.gen).unwrap_or(0);
        self.mesh_revisions[idx] = ((gen as u64) << 32) | (revision & 0xFFFF_FFFF);
    }

    fn mesh_mut(&mut self, handle: MeshHandle) -> Option<&mut Mesh> {
        if !handle.is_valid() {
            return None;
        }
        let slot = self.slots.get(handle.idx() as usize)?;
        if slot.gen != handle.gen() {
            return None;
        }
        let arc = slot.mesh.as_ref()?;
        // Snapshot holds another Arc; buffers stay in this allocation (no new Mesh).
        Some(unsafe { &mut *(Arc::as_ptr(arc) as *mut Mesh) })
    }

    fn mesh_mut_slot(&self, handle: MeshHandle) -> Option<*mut Mesh> {
        if !handle.is_valid() {
            return None;
        }
        let slot = self.slots.get(handle.idx() as usize)?;
        if slot.gen != handle.gen() {
            return None;
        }
        let arc = slot.mesh.as_ref()?;
        Some(Arc::as_ptr(arc) as *mut Mesh)
    }

    fn recycle_arc(&mut self, arc: Arc<Mesh>) {
        if let Ok(mesh) = Arc::try_unwrap(arc) {
            self.cpu_arena.recycle_mesh(mesh);
        }
    }

    /// Insert a mesh; refcount starts at **0** until [`Self::attach`].
    pub fn insert(&mut self, mesh: Mesh) -> MeshHandle {
        let mut mesh = mesh;
        assert!(
            mesh.has_local_bounds(),
            "mesh insert: local AABB is required (set at upload)"
        );
        self.cpu_arena.adopt(&mut mesh);
        let revision = mesh.gpu_revision;
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
        self.set_mesh_revision(idx, revision);
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
        if self.get(handle).is_none() {
            log::error!(
                "MeshRegistry::attach unknown or stale MeshHandle idx={} gen={}",
                handle.idx(),
                handle.gen()
            );
            return;
        }
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
        let prev = {
            let Some(slot) = self.slots.get_mut(idx) else {
                return;
            };
            if slot.gen != handle.gen() || slot.mesh.is_none() {
                return;
            }
            slot.gen = slot.gen.wrapping_add(1);
            slot.mesh.take()
        };
        self.free.push(idx);
        self.set_mesh_revision(idx, 0);
        if let Some(prev) = prev {
            self.recycle_arc(prev);
        }
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
            let prev = slot.mesh.take();
            slot.gen = slot.gen.wrapping_add(1);
            drop(slot);
            self.free.push(idx);
            self.set_mesh_revision(idx, 0);
            self.handle_refs.remove(&packed);
            if let Some(prev) = prev {
                self.recycle_arc(prev);
            }
        }
    }

    /// Drop everything (level change / shutdown). Generations are reset, so
    /// any handle held elsewhere becomes invalid.
    pub fn clear(&mut self) {
        let held: Vec<_> = self.slots.iter_mut().filter_map(|slot| slot.mesh.take()).collect();
        for prev in held {
            self.recycle_arc(prev);
        }
        self.slots.clear();
        self.slots.push(Slot {
            gen: 0,
            mesh: None,
        });
        self.mesh_revisions.clear();
        self.mesh_revisions.push(0);
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
