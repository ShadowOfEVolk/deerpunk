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
) -> usize;

/// `v1:mesh:register_allocator(name, vtable_bytes, alloc_ctx) -> u32`.
type MeshRegisterAllocatorFn =
    unsafe extern "C" fn(*mut u8, *const u8, usize, *const u8, usize, *mut u8) -> u32;

pub struct MeshApi {
    ctx: *mut u8,
    create_fn: Option<MeshCreateFn>,
    create_with_fn: Option<MeshCreateWithFn>,
    register_allocator_fn: Option<MeshRegisterAllocatorFn>,
    release_fn: Option<extern "C" fn(*mut u8, usize)>,
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
            register_allocator_fn: resolve(r, "v1:mesh:register_allocator")
                .map(|p| unsafe { std::mem::transmute(p) }),
            release_fn: resolve(r, "v1:mesh:release").map(|p| unsafe { std::mem::transmute(p) }),
            content_id_fn: resolve(r, "v1:mesh:content_id")
                .map(|p| unsafe { std::mem::transmute(p) }),
        }
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
        ))
    }

    pub fn create_from_mesh(&self, mesh: &Mesh) -> MeshHandle {
        if mesh.positions.is_empty() {
            return MeshHandle::INVALID;
        }
        let vc = mesh.positions.len() as u32;
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
        if mesh.positions.is_empty() {
            return MeshHandle::INVALID;
        }
        let Some(f) = self.create_with_fn else {
            return MeshHandle::INVALID;
        };
        let vc = mesh.positions.len() as u32;
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
            allocator_id,
        ))
    }

    pub fn content_id(&self, handle: MeshHandle) -> u64 {
        let Some(f) = self.content_id_fn else {
            return 0;
        };
        f(self.ctx, handle.0)
    }
}