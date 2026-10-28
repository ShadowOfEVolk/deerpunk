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

pub struct ClientApi {
    ctx: *mut u8,
    view_ray_fn: ViewRayFn,
    scene_time_fn: SceneTimeFn,
    debug_aabb_fn: DebugAabbFn,
}

unsafe impl Send for ClientApi {}
unsafe impl Sync for ClientApi {}

impl ClientApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            view_ray_fn: unsafe { std::mem::transmute(resolve(r, "v1:client:view_ray")) },
            scene_time_fn: unsafe { std::mem::transmute(resolve(r, "v1:client:scene_time")) },
            debug_aabb_fn: unsafe { std::mem::transmute(resolve(r, "v1:client:debug_aabb")) },
        }
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
