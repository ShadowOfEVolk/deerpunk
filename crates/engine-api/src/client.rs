use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

type ViewRayFn = extern "C" fn(*mut u8, *mut f32) -> i32;
type SceneTimeFn = extern "C" fn(*mut u8) -> f32;
type DebugAabbFn = extern "C" fn(
    *mut u8,
    *const f32,
    *const f32,
    f32,
    f32,
    f32,
    f32,
) -> i32;
pub use deerpunk_engine_common::mesh_submit::{MeshSubmit, MeshSubmitBin, MeshSubmitBinset};

/// Old name for a single bin. Prefer [`MeshSubmitBin`].
pub type MeshSubmitRange = MeshSubmitBin;

type SubmitMeshFn = extern "C" fn(
    *mut u8,
    u64,
    u64,
    u32,
    u32,
    u32,
    *const f32,
    *const f32,
    *const f32,
    u32,
    u32,
    u8,
) -> i32;
type SubmitFn = extern "C" fn(*mut u8, *const MeshSubmit) -> i32;
type VisPlanesFn = extern "C" fn(*mut u8, *mut f32) -> i32;
type SubmitMeshRangesFn = extern "C" fn(
    *mut u8,
    u64,
    u64,
    u32,
    u32,
    u32,
    *const f32,
    *const f32,
    *const f32,
    *const MeshSubmitRange,
    u32,
) -> i32;

pub struct ClientApi {
    ctx: *mut u8,
    view_ray_fn: ViewRayFn,
    scene_time_fn: SceneTimeFn,
    debug_aabb_fn: DebugAabbFn,
    submit_mesh_fn: SubmitMeshFn,
    submit_mesh_ranges_fn: SubmitMeshRangesFn,
    submit_fn: SubmitFn,
    vis_planes_fn: VisPlanesFn,
}

unsafe impl Send for ClientApi {}
unsafe impl Sync for ClientApi {}

impl ClientApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            view_ray_fn: unsafe { crate::fn_from_erased(resolve(r, "v1:client:view_ray")) },
            scene_time_fn: unsafe { crate::fn_from_erased(resolve(r, "v1:client:scene_time")) },
            debug_aabb_fn: unsafe { crate::fn_from_erased(resolve(r, "v1:client:debug_aabb")) },
            submit_mesh_fn: unsafe { crate::fn_from_erased(resolve(r, "v1:client:submit_mesh")) },
            submit_mesh_ranges_fn: unsafe {
                crate::fn_from_erased(resolve(r, "v1:client:submit_mesh_ranges"))
            },
            submit_fn: unsafe { crate::fn_from_erased(resolve(r, "v1:client:submit")) },
            vis_planes_fn: unsafe { crate::fn_from_erased(resolve(r, "v1:client:vis_planes")) },
        }
    }

    /// Six clip planes from the extracted camera, `(nx, ny, nz, d)` each.
    /// `None` when no camera was extracted; treat every chunk as visible.
    pub fn vis_planes(&self) -> Option<[[f32; 4]; 6]> {
        let mut raw = [0.0f32; 24];
        if (self.vis_planes_fn)(self.ctx, raw.as_mut_ptr()) == 0 {
            return None;
        }
        let mut planes = [[0.0f32; 4]; 6];
        for i in 0..6 {
            planes[i].copy_from_slice(&raw[i * 4..i * 4 + 4]);
        }
        Some(planes)
    }

    /// Queue every binset in `desc`. The model matrix is built once.
    /// Bounds are rebuilt only when the mesh handle changes.
    pub fn submit(&self, desc: &MeshSubmit) -> bool {
        (self.submit_fn)(self.ctx, desc) == 0
    }

    /// Append a static mesh to the current `renderer.prepare` draw list.
    /// `rotation` is xyzw. Returns false when prepare is not active or the mesh/material is missing.
    pub fn submit_mesh(
        &self,
        entity: u64,
        mesh: u64,
        material: u32,
        uniform_aux: u32,
        layer_mask: u32,
        position: [f32; 3],
        rotation: [f32; 4],
        scale: [f32; 3],
        first_index: u32,
        index_count: u32,
        face: u8,
    ) -> bool {
        (self.submit_mesh_fn)(
            self.ctx,
            entity,
            mesh,
            material,
            uniform_aux,
            layer_mask,
            position.as_ptr(),
            rotation.as_ptr(),
            scale.as_ptr(),
            first_index,
            index_count,
            face,
        ) == 0
    }

    /// One bin: shared mesh, material, and transform, many index spans.
    /// Mesh lookup and the model matrix run once. Empty `index_count` spans are skipped.
    pub fn submit_mesh_ranges(
        &self,
        entity: u64,
        mesh: u64,
        material: u32,
        uniform_aux: u32,
        layer_mask: u32,
        position: [f32; 3],
        rotation: [f32; 4],
        scale: [f32; 3],
        ranges: &[MeshSubmitRange],
    ) -> bool {
        if ranges.is_empty() {
            return false;
        }
        (self.submit_mesh_ranges_fn)(
            self.ctx,
            entity,
            mesh,
            material,
            uniform_aux,
            layer_mask,
            position.as_ptr(),
            rotation.as_ptr(),
            scale.as_ptr(),
            ranges.as_ptr(),
            ranges.len() as u32,
        ) == 0
    }

    /// Seconds since client start (`advance_time`); matches `lights_u.scene_time_bits` in mesh shaders.
    pub fn scene_time(&self) -> f32 {
        (self.scene_time_fn)(self.ctx)
    }

    /// World-space eye origin + normalized look direction (Z-up). `None` if no local pawn.
    pub fn view_ray(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut out = [0.0f32; 6];
        if (self.view_ray_fn)(self.ctx, out.as_mut_ptr()) != 0 {
            return None;
        }
        Some(([out[0], out[1], out[2]], [out[3], out[4], out[5]]))
    }

    /// Wireframe AABB for this frame (merged into debug overlay before render).
    pub fn debug_aabb(&self, min: [f32; 3], max: [f32; 3], rgba: [f32; 4]) {
        (self.debug_aabb_fn)(
            self.ctx,
            min.as_ptr(),
            max.as_ptr(),
            rgba[0],
            rgba[1],
            rgba[2],
            rgba[3],
        );
    }
}
