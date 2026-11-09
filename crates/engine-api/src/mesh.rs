use deerpunk_engine_common::resource::mesh::Mesh;
use deerpunk_engine_common::resource::mesh_allocator::MeshAllocatorVTable;
use deerpunk_engine_common::resource::mesh_handle::MeshHandle;

use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> Option<*const ()> {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        None
    } else {
        Some(ptr)
    }
}

use std::ffi::c_void;

type MeshCreateFn = extern "C" fn(
    *mut u8,
    *const f32,
    u32,
    *const f32,
    *const f32,
    *const c_void,
    u32,
    *const c_void,
    *const f32,
    *const f32,
    *const f32,
    *const f32,
    u32,
    *mut u8,
    u32,
) -> usize;

/// `v1:mesh:create_with` — like `create` plus a trailing `allocator_id: u32`
/// that tags the mesh with a registered custom allocator.
type MeshCreateWithFn = extern "C" fn(
    *mut u8,
    *const f32,
    u32,
    *const f32,
    *const f32,
    *const c_void,
    u32,
    *const c_void,
    *const f32,
    *const f32,
    *const f32,
    *const f32,
    u32,
    *mut u8,
    u32,
    u32,
) -> usize;

/// `v1:mesh:register_allocator(name, vtable_bytes, alloc_ctx) -> u32`.
type MeshRegisterAllocatorFn =
    unsafe extern "C" fn(*mut u8, *const u8, usize, *const u8, usize, *mut u8) -> u32;

/// `v1:mesh:update` — same payload as `create`, first arg is existing handle. Returns 1 on success.
type MeshUpdateFn = extern "C" fn(
    *mut u8,
    usize,
    *const f32,
    u32,
    *const f32,
    *const f32,
    *const c_void,
    u32,
    *const c_void,
    *const f32,
    *const f32,
    *const f32,
    *const f32,
    u32,
    *mut u8,
    u32,
) -> i32;

type MeshPackedUpdateFn = extern "C" fn(
    *mut u8,
    usize,
    f32,
    f32,
    f32,
    f32,
    f32,
    f32,
    u32,
    *mut u8,
    u32,
    *const c_void,
    u32,
) -> i32;

type MeshPackedCreateFn = extern "C" fn(
    *mut u8,
    f32,
    f32,
    f32,
    f32,
    f32,
    f32,
    u32,
    *mut u8,
    u32,
    *const c_void,
    u32,
    u32,
) -> usize;

type MeshGetBufferFn = extern "C" fn(*mut u8, usize, u32, u32, *mut u64, *mut u64) -> i32;
type MeshMarkDirtyFn = extern "C" fn(*mut u8, usize) -> i32;

pub struct MeshApi {
    ctx: *mut u8,
    create_fn: Option<MeshCreateFn>,
    create_with_fn: Option<MeshCreateWithFn>,
    update_fn: Option<MeshUpdateFn>,
    update_packed_fn: Option<MeshPackedUpdateFn>,
    create_packed_fn: Option<MeshPackedCreateFn>,
    get_buffer_fn: Option<MeshGetBufferFn>,
    mark_dirty_fn: Option<MeshMarkDirtyFn>,
    register_allocator_fn: Option<MeshRegisterAllocatorFn>,
    release_fn: Option<extern "C" fn(*mut u8, usize)>,
    pin_fn: Option<extern "C" fn(*mut u8, usize)>,
    unpin_fn: Option<extern "C" fn(*mut u8, usize)>,
    content_id_fn: Option<extern "C" fn(*mut u8, usize) -> u64>,
}

unsafe impl Send for MeshApi {}
unsafe impl Sync for MeshApi {}

impl MeshApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            create_fn: resolve(r, "v1:mesh:create").map(|p| unsafe { std::mem::transmute(p) }),
            create_with_fn: resolve(r, "v1:mesh:create_with")
                .map(|p| unsafe { std::mem::transmute(p) }),
            update_fn: resolve(r, "v1:mesh:update").map(|p| unsafe { std::mem::transmute(p) }),
            update_packed_fn: resolve(r, "v1:mesh:update_packed")
                .map(|p| unsafe { std::mem::transmute(p) }),
            create_packed_fn: resolve(r, "v1:mesh:create_packed")
                .map(|p| unsafe { std::mem::transmute(p) }),
            get_buffer_fn: resolve(r, "v1:mesh:get_buffer")
                .map(|p| unsafe { std::mem::transmute(p) }),
            mark_dirty_fn: resolve(r, "v1:mesh:mark_dirty")
                .map(|p| unsafe { std::mem::transmute(p) }),
            register_allocator_fn: resolve(r, "v1:mesh:register_allocator")
                .map(|p| unsafe { std::mem::transmute(p) }),
            release_fn: resolve(r, "v1:mesh:release").map(|p| unsafe { std::mem::transmute(p) }),
            pin_fn: resolve(r, "v1:mesh:pin").map(|p| unsafe { std::mem::transmute(p) }),
            unpin_fn: resolve(r, "v1:mesh:unpin").map(|p| unsafe { std::mem::transmute(p) }),
            content_id_fn: resolve(r, "v1:mesh:content_id")
                .map(|p| unsafe { std::mem::transmute(p) }),
        }
    }

    fn mesh_packed_ffi(mesh: &Mesh) -> (u32, *mut u8, u32) {
        if mesh.vertex_size == 0 {
            return (0, std::ptr::null_mut(), 0);
        }
        debug_assert!(mesh.uses_prepacked_vertices());
        (
            mesh.vertex_size,
            mesh.vertex_bytes.as_ptr() as *mut u8,
            mesh.vertex_bytes.len() as u32,
        )
    }

    pub fn create(
        &self,
        positions: &[f32],
        vertex_count: u32,
        normals: &[f32],
        uvs: &[f32],
        indices: &[u32],
        joint_indices: &[u8],
        joint_weights: &[f32],
        lightmap_uvs: &[f32],
        lightmap_styles: &[f32],
        atlas_rects: &[f32],
        vertex_size: u32,
        vertex_bytes: *mut u8,
        vertex_byte_len: u32,
    ) -> MeshHandle {
        let Some(f) = self.create_fn else {
            return MeshHandle::INVALID;
        };
        MeshHandle(f(
            self.ctx,
            positions.as_ptr(),
            vertex_count,
            if normals.is_empty() {
                std::ptr::null()
            } else {
                normals.as_ptr()
            },
            if uvs.is_empty() {
                std::ptr::null()
            } else {
                uvs.as_ptr()
            },
            if indices.is_empty() {
                std::ptr::null()
            } else {
                indices.as_ptr() as *const c_void
            },
            indices.len() as u32,
            if joint_indices.is_empty() {
                std::ptr::null()
            } else {
                joint_indices.as_ptr() as *const c_void
            },
            if joint_weights.is_empty() {
                std::ptr::null()
            } else {
                joint_weights.as_ptr()
            },
            if lightmap_uvs.is_empty() {
                std::ptr::null()
            } else {
                lightmap_uvs.as_ptr()
            },
            if lightmap_styles.is_empty() {
                std::ptr::null()
            } else {
                lightmap_styles.as_ptr()
            },
            if atlas_rects.is_empty() {
                std::ptr::null()
            } else {
                atlas_rects.as_ptr()
            },
            vertex_size,
            vertex_bytes,
            vertex_byte_len,
        ))
    }

    pub fn create_from_mesh(&self, mesh: &Mesh) -> MeshHandle {
        if mesh.vertex_size == 0 || !mesh.uses_prepacked_vertices() {
            return MeshHandle::INVALID;
        }
        let vc = mesh.vertex_count() as u32;
        let positions: Vec<f32> = mesh
            .positions
            .iter()
            .flat_map(|p| p.iter().copied())
            .collect();
        let normals: Vec<f32> = if mesh.normals.len() == mesh.positions.len() {
            mesh.normals
                .iter()
                .flat_map(|n| n.iter().copied())
                .collect()
        } else {
            Vec::new()
        };
        let uvs: Vec<f32> = if mesh.uvs.len() == mesh.positions.len() {
            mesh.uvs.iter().flat_map(|u| u.iter().copied()).collect()
        } else {
            Vec::new()
        };
        let joint_indices: Vec<u8> = mesh.joint_indices.iter().flat_map(|j| j.iter().copied()).collect();
        let joint_weights: Vec<f32> = mesh
            .joint_weights
            .iter()
            .flat_map(|w| w.iter().copied())
            .collect();
        let lightmap_uvs: Vec<f32> = if mesh.lightmap_uvs.len() == mesh.positions.len() {
            mesh.lightmap_uvs
                .iter()
                .flat_map(|u| u.iter().copied())
                .collect()
        } else {
            Vec::new()
        };
        let lightmap_styles: Vec<f32> = if mesh.lightmap_styles.len() == mesh.positions.len() {
            mesh.lightmap_styles
                .iter()
                .flat_map(|s| s.iter().copied())
                .collect()
        } else {
            Vec::new()
        };
        let atlas_rects: Vec<f32> = if mesh.atlas_rects.len() == mesh.positions.len() {
            mesh.atlas_rects
                .iter()
                .flat_map(|r| r.iter().copied())
                .collect()
        } else {
            Vec::new()
        };
        let (vertex_size, vertex_bytes, vertex_byte_len) = Self::mesh_packed_ffi(mesh);
        self.create(
            &positions,
            vc,
            &normals,
            &uvs,
            &mesh.indices,
            &joint_indices,
            &joint_weights,
            &lightmap_uvs,
            &lightmap_styles,
            &atlas_rects,
            vertex_size,
            vertex_bytes,
            vertex_byte_len,
        )
    }

    /// `v1:mesh:release` is **removed** from the host API. Mesh lifetime is
    /// owned by the `MeshRegistry` and reclaimed by its per-frame sweep once
    /// no entity references the handle. To drop a mesh, remove the `Mesh3D`
    /// component (or destroy the entity); the sweep frees the slot.
    #[deprecated(note = "mesh lifetime is owned by MeshRegistry; use client_world.set/destroy")]
    pub fn release(&self, _handle: MeshHandle) {
        // No-op on the client side; the host entry point panics if invoked.
    }

    /// Register a custom mesh allocator with the engine and get back its
    /// stable id. The engine owns the GPU buffers; `vtable` is an
    /// offset-only bookkeeper the engine calls back on allocate/free. The
    /// game owns `alloc_ctx` and must keep it alive until the allocator is
    /// unregistered (engine shutdown / level end).
    ///
    /// Returns `u32::MAX` on failure. Re-registering the same `name`
    /// returns the previously assigned id (idempotent).
    pub fn register_allocator(
        &self,
        name: &str,
        vtable: &MeshAllocatorVTable,
        alloc_ctx: *mut u8,
    ) -> u32 {
        let Some(f) = self.register_allocator_fn else {
            return u32::MAX;
        };
        let name_bytes = name.as_bytes();
        let vtable_bytes =
            unsafe { std::slice::from_raw_parts(vtable as *const _ as *const u8, std::mem::size_of::<MeshAllocatorVTable>()) };
        unsafe {
            f(
                self.ctx,
                name_bytes.as_ptr(),
                name_bytes.len(),
                vtable_bytes.as_ptr(),
                vtable_bytes.len(),
                alloc_ctx,
            )
        }
    }

    /// Like [`create_from_mesh`](Self::create_from_mesh) but tags the mesh
    /// with a registered custom allocator (`allocator_id`) so the renderer
    /// routes uploads/frees through it instead of the built-in slab.
    pub fn create_from_mesh_with(&self, mesh: &Mesh, allocator_id: u32) -> MeshHandle {
        if mesh.vertex_size == 0 || !mesh.uses_prepacked_vertices() {
            return MeshHandle::INVALID;
        }
        let Some(f) = self.create_with_fn else {
            return MeshHandle::INVALID;
        };
        let vc = mesh.vertex_count() as u32;
        let positions: Vec<f32> = mesh
            .positions
            .iter()
            .flat_map(|p| p.iter().copied())
            .collect();
        let normals: Vec<f32> = if mesh.normals.len() == mesh.positions.len() {
            mesh.normals.iter().flat_map(|n| n.iter().copied()).collect()
        } else {
            Vec::new()
        };
        let uvs: Vec<f32> = if mesh.uvs.len() == mesh.positions.len() {
            mesh.uvs.iter().flat_map(|u| u.iter().copied()).collect()
        } else {
            Vec::new()
        };
        let joint_indices: Vec<u8> =
            mesh.joint_indices.iter().flat_map(|j| j.iter().copied()).collect();
        let joint_weights: Vec<f32> =
            mesh.joint_weights.iter().flat_map(|w| w.iter().copied()).collect();
        let lightmap_uvs: Vec<f32> = if mesh.lightmap_uvs.len() == mesh.positions.len() {
            mesh.lightmap_uvs.iter().flat_map(|u| u.iter().copied()).collect()
        } else {
            Vec::new()
        };
        let lightmap_styles: Vec<f32> = if mesh.lightmap_styles.len() == mesh.positions.len() {
            mesh.lightmap_styles.iter().flat_map(|s| s.iter().copied()).collect()
        } else {
            Vec::new()
        };
        let atlas_rects: Vec<f32> = if mesh.atlas_rects.len() == mesh.positions.len() {
            mesh.atlas_rects.iter().flat_map(|r| r.iter().copied()).collect()
        } else {
            Vec::new()
        };
        let (vertex_size, vertex_bytes, vertex_byte_len) = Self::mesh_packed_ffi(mesh);
        MeshHandle(f(
            self.ctx,
            positions.as_ptr(),
            vc,
            if normals.is_empty() {
                std::ptr::null()
            } else {
                normals.as_ptr()
            },
            if uvs.is_empty() {
                std::ptr::null()
            } else {
                uvs.as_ptr()
            },
            if mesh.indices.is_empty() {
                std::ptr::null()
            } else {
                mesh.indices.as_ptr() as *const c_void
            },
            mesh.indices.len() as u32,
            if joint_indices.is_empty() {
                std::ptr::null()
            } else {
                joint_indices.as_ptr() as *const c_void
            },
            if joint_weights.is_empty() {
                std::ptr::null()
            } else {
                joint_weights.as_ptr()
            },
            if lightmap_uvs.is_empty() {
                std::ptr::null()
            } else {
                lightmap_uvs.as_ptr()
            },
            if lightmap_styles.is_empty() {
                std::ptr::null()
            } else {
                lightmap_styles.as_ptr()
            },
            if atlas_rects.is_empty() {
                std::ptr::null()
            } else {
                atlas_rects.as_ptr()
            },
            vertex_size,
            vertex_bytes,
            vertex_byte_len,
            allocator_id,
        ))
    }

    /// Replace CPU geometry for `handle`. Same handle and content id; bumps `gpu_revision`.
    /// Returns false if the host has no `v1:mesh:update` or the handle is stale.
    /// Does not upload to the GPU — the renderer does that when revision changes.
    pub fn update_from_mesh(&self, handle: MeshHandle, mesh: &Mesh) -> bool {
        if !handle.is_valid() || mesh.vertex_size == 0 || !mesh.uses_prepacked_vertices() {
            return false;
        }
        let Some(f) = self.update_fn else {
            return false;
        };
        let vc = mesh.vertex_count() as u32;
        let positions: Vec<f32> = mesh.positions.iter().flat_map(|p| p.iter().copied()).collect();
        let normals: Vec<f32> = if mesh.normals.len() == mesh.positions.len() {
            mesh.normals.iter().flat_map(|n| n.iter().copied()).collect()
        } else {
            Vec::new()
        };
        let uvs: Vec<f32> = if mesh.uvs.len() == mesh.positions.len() {
            mesh.uvs.iter().flat_map(|u| u.iter().copied()).collect()
        } else {
            Vec::new()
        };
        let joint_indices: Vec<u8> = mesh.joint_indices.iter().flat_map(|j| j.iter().copied()).collect();
        let joint_weights: Vec<f32> = mesh.joint_weights.iter().flat_map(|w| w.iter().copied()).collect();
        let lightmap_uvs: Vec<f32> = if mesh.lightmap_uvs.len() == mesh.positions.len() {
            mesh.lightmap_uvs.iter().flat_map(|u| u.iter().copied()).collect()
        } else {
            Vec::new()
        };
        let lightmap_styles: Vec<f32> = if mesh.lightmap_styles.len() == mesh.positions.len() {
            mesh.lightmap_styles.iter().flat_map(|s| s.iter().copied()).collect()
        } else {
            Vec::new()
        };
        let atlas_rects: Vec<f32> = if mesh.atlas_rects.len() == mesh.positions.len() {
            mesh.atlas_rects.iter().flat_map(|r| r.iter().copied()).collect()
        } else {
            Vec::new()
        };
        let (vertex_size, vertex_bytes, vertex_byte_len) = Self::mesh_packed_ffi(mesh);
        f(
            self.ctx,
            handle.0,
            positions.as_ptr(),
            vc,
            if normals.is_empty() { std::ptr::null() } else { normals.as_ptr() },
            if uvs.is_empty() { std::ptr::null() } else { uvs.as_ptr() },
            if mesh.indices.is_empty() {
                std::ptr::null()
            } else {
                mesh.indices.as_ptr() as *const c_void
            },
            mesh.indices.len() as u32,
            if joint_indices.is_empty() {
                std::ptr::null()
            } else {
                joint_indices.as_ptr() as *const c_void
            },
            if joint_weights.is_empty() { std::ptr::null() } else { joint_weights.as_ptr() },
            if lightmap_uvs.is_empty() { std::ptr::null() } else { lightmap_uvs.as_ptr() },
            if lightmap_styles.is_empty() { std::ptr::null() } else { lightmap_styles.as_ptr() },
            if atlas_rects.is_empty() { std::ptr::null() } else { atlas_rects.as_ptr() },
            vertex_size,
            vertex_bytes,
            vertex_byte_len,
        ) != 0
    }

    /// Packed vertices + caller AABB. Host copies `vertex_bytes`/`indices` only.
    pub fn update_packed(&self, handle: MeshHandle, mesh: &Mesh) -> bool {
        if !handle.is_valid() || !mesh.uses_prepacked_vertices() || !mesh.has_local_bounds() {
            return false;
        }
        let Some(f) = self.update_packed_fn else {
            return false;
        };
        let indices = if mesh.indices.is_empty() {
            std::ptr::null()
        } else {
            mesh.indices.as_ptr() as *const c_void
        };
        f(
            self.ctx,
            handle.0,
            mesh.bounds_min[0],
            mesh.bounds_min[1],
            mesh.bounds_min[2],
            mesh.bounds_max[0],
            mesh.bounds_max[1],
            mesh.bounds_max[2],
            mesh.vertex_size,
            mesh.vertex_bytes.as_ptr() as *mut u8,
            mesh.vertex_bytes.len() as u32,
            indices,
            mesh.indices.len() as u32,
        ) != 0
    }

    pub fn create_packed(&self, mesh: &Mesh, allocator_id: u32) -> MeshHandle {
        if !mesh.uses_prepacked_vertices() || !mesh.has_local_bounds() {
            return MeshHandle::INVALID;
        }
        let Some(f) = self.create_packed_fn else {
            return MeshHandle::INVALID;
        };
        let indices = if mesh.indices.is_empty() {
            std::ptr::null()
        } else {
            mesh.indices.as_ptr() as *const c_void
        };
        MeshHandle(f(
            self.ctx,
            mesh.bounds_min[0],
            mesh.bounds_min[1],
            mesh.bounds_min[2],
            mesh.bounds_max[0],
            mesh.bounds_max[1],
            mesh.bounds_max[2],
            mesh.vertex_size,
            mesh.vertex_bytes.as_ptr() as *mut u8,
            mesh.vertex_bytes.len() as u32,
            indices,
            mesh.indices.len() as u32,
            allocator_id,
        ))
    }

    /// Resize the existing vertex/index storage and copy `mesh` into it, then `mark_dirty`.
    /// No new `Mesh` allocation. Returns false if the host has no `get_buffer`.
    pub fn write_packed_dirty(&self, handle: MeshHandle, mesh: &Mesh) -> bool {
        if !handle.is_valid() || !mesh.uses_prepacked_vertices() {
            return false;
        }
        let Some(get) = self.get_buffer_fn else {
            return false;
        };
        let Some(mark) = self.mark_dirty_fn else {
            return false;
        };
        let mut vert_ptr = 0u64;
        let mut index_ptr = 0u64;
        if get(
            self.ctx,
            handle.0,
            mesh.vertex_bytes.len() as u32,
            mesh.indices.len() as u32,
            &mut vert_ptr,
            &mut index_ptr,
        ) == 0
            || vert_ptr == 0
        {
            return false;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                mesh.vertex_bytes.as_ptr(),
                vert_ptr as *mut u8,
                mesh.vertex_bytes.len(),
            );
            if !mesh.indices.is_empty() && index_ptr != 0 {
                std::ptr::copy_nonoverlapping(
                    mesh.indices.as_ptr(),
                    index_ptr as *mut u32,
                    mesh.indices.len(),
                );
            }
        }
        mark(self.ctx, handle.0) != 0
    }

    /// Keep the registry slot and the GPU upload for a mesh the game owns
    /// (not `Mesh3D`). Pair with [`Self::unpin`] when that owner drops the handle.
    pub fn pin(&self, handle: MeshHandle) {
        if !handle.is_valid() {
            return;
        }
        if let Some(f) = self.pin_fn {
            f(self.ctx, handle.0);
        }
    }

    /// Inverse of [`Self::pin`].
    pub fn unpin(&self, handle: MeshHandle) {
        if !handle.is_valid() {
            return;
        }
        if let Some(f) = self.unpin_fn {
            f(self.ctx, handle.0);
        }
    }

    pub fn content_id(&self, handle: MeshHandle) -> u64 {
        let Some(f) = self.content_id_fn else {
            return 0;
        };
        f(self.ctx, handle.0)
    }
}