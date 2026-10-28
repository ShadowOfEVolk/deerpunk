//! Custom mesh allocator API (Unity-style).
//!
//! The engine owns the GPU vertex/index buffers; a [`MeshAllocator`] is an
//! **offset-only** bookkeeper that decides where (which buffer + offsets) a
//! mesh lives. Implementations are pure u32 math — no `wgpu` types — so they
//! are safe to expose across the FFI boundary and implement inside a game
//! library that does not link the renderer.
//!
//! Default: the engine's built-in [`SlabAllocator`]-equivalent (append-only,
//! no `free`) is used for meshes that do not opt into a custom allocator.
//! A game can register its own allocator (e.g. a region/free-list arena for
//! streaming voxel chunks) via `v1:mesh:register_allocator` and tag meshes
//! with it via `v1:mesh:create_with`.

/// Location of a mesh inside the engine's GPU buffer pool.
///
/// `buffer_id` indexes the engine-owned `Vec<(VB, IB)>`; the offsets are in
/// vertices / indices (not bytes). `stride` is bytes per vertex for the
/// buffer this handle refers to.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MeshAllocHandle {
    /// Index into the engine's buffer pool (`0` is reserved = invalid).
    pub buffer_id: u32,
    pub vert_offset: u32,
    pub vert_count: u32,
    pub idx_offset: u32,
    pub idx_count: u32,
    /// Bytes per vertex (`MESH_VERTEX_SIZE` or `SKINNED_MESH_VERTEX_SIZE`).
    pub stride: u32,
}

impl MeshAllocHandle {
    pub const INVALID: Self = Self {
        buffer_id: 0,
        vert_offset: 0,
        vert_count: 0,
        idx_offset: 0,
        idx_count: 0,
        stride: 0,
    };

    pub fn is_valid(&self) -> bool {
        self.buffer_id != 0 && (self.vert_count > 0 || self.idx_count > 0)
    }

    pub fn vertex_range(&self) -> std::ops::Range<u32> {
        self.vert_offset..self.vert_offset + self.vert_count
    }

    pub fn index_range(&self) -> std::ops::Range<u32> {
        self.idx_offset..self.idx_offset + self.idx_count
    }
}

/// Offset-only mesh allocator. Engine owns the GPU buffers; the allocator
/// decides where a mesh lives within the buffer pool.
///
/// Implementations must be `Send + Sync` and cheap to call from the render
/// thread. No `wgpu` types are used here.
pub trait MeshAllocator: Send + Sync {
    /// Reserve space for `vert_count` vertices and `idx_count` indices with
    /// the given `stride` (bytes/vertex). Returns a handle pointing into the
    /// engine's buffer pool, or [`MeshAllocHandle::INVALID`] on failure.
    fn allocate(
        &mut self,
        vert_count: u32,
        idx_count: u32,
        stride: u32,
    ) -> MeshAllocHandle;

    /// Release the region previously returned by [`allocate`]. After this,
    /// the handle is invalid and the region may be reused by a future
    /// `allocate`. Implementations without reclamation may no-op.
    fn free(&mut self, handle: MeshAllocHandle);

    /// The engine created a new GPU buffer (`buffer_id`) with the given
    /// capacities and `stride`, and offers it to this allocator for future
    /// `allocate` calls. Allocators that own their own buffers may no-op;
    /// offset-only allocators must record it so `allocate` can place meshes
    /// in it. Default no-op.
    fn add_buffer(
        &mut self,
        buffer_id: u32,
        vert_cap: u32,
        idx_cap: u32,
        stride: u32,
    ) {
        let _ = (buffer_id, vert_cap, idx_cap, stride);
    }
}

// ---------------------------------------------------------------------------
// FFI vtable: lets a game library (cdylib) register its own allocator.
// ---------------------------------------------------------------------------

/// Function-pointer table implementing [`MeshAllocator`] across the FFI
/// boundary. The game constructs this and passes it to
/// `v1:mesh:register_allocator`; the engine wraps it as a `MeshAllocator`.
#[repr(C)]
pub struct MeshAllocatorVTable {
    /// `fn(ctx, vert_count, idx_count, stride, out: *mut MeshAllocHandle)`.
    /// Must write a valid [`MeshAllocHandle`] to `out` or
    /// [`MeshAllocHandle::INVALID`] if no known buffer has room.
    pub allocate: extern "C" fn(
        ctx: *mut u8,
        vert_count: u32,
        idx_count: u32,
        stride: u32,
        out: *mut MeshAllocHandle,
    ),
    /// `fn(ctx, handle: MeshAllocHandle)`. Release the region.
    pub free: extern "C" fn(ctx: *mut u8, handle: MeshAllocHandle),
    /// `fn(ctx, buffer_id, vert_cap, idx_cap, stride)`. The engine created
    /// a new GPU buffer and offers it to this allocator.
    pub add_buffer: extern "C" fn(
        ctx: *mut u8,
        buffer_id: u32,
        vert_cap: u32,
        idx_cap: u32,
        stride: u32,
    ),
}

/// A registered custom allocator: vtable + opaque context pointer owned by
/// the game library.
#[repr(C)]
pub struct MeshAllocatorRegistration {
    pub vtable: MeshAllocatorVTable,
    pub ctx: *mut u8,
}

/// Adapter wrapping a [`MeshAllocatorRegistration`] as a [`MeshAllocator`].
/// Lives in the engine; the game keeps ownership of `ctx` and must keep it
/// alive until the allocator is unregistered (engine shutdown / level end).
pub struct ForeignMeshAllocator {
    reg: MeshAllocatorRegistration,
}

impl ForeignMeshAllocator {
    pub fn new(reg: MeshAllocatorRegistration) -> Self {
        Self { reg }
    }

    pub fn registration(&self) -> &MeshAllocatorRegistration {
        &self.reg
    }
}

impl MeshAllocator for ForeignMeshAllocator {
    fn allocate(
        &mut self,
        vert_count: u32,
        idx_count: u32,
        stride: u32,
    ) -> MeshAllocHandle {
        let mut out = MeshAllocHandle::INVALID;
        (self.reg.vtable.allocate)(self.reg.ctx, vert_count, idx_count, stride, &mut out);
        out
    }

    fn free(&mut self, handle: MeshAllocHandle) {
        (self.reg.vtable.free)(self.reg.ctx, handle);
    }

    fn add_buffer(
        &mut self,
        buffer_id: u32,
        vert_cap: u32,
        idx_cap: u32,
        stride: u32,
    ) {
        (self.reg.vtable.add_buffer)(self.reg.ctx, buffer_id, vert_cap, idx_cap, stride);
    }
}

unsafe impl Send for ForeignMeshAllocator {}
unsafe impl Sync for ForeignMeshAllocator {}
