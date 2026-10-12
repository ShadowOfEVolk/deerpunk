use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

/// Must match `deerpunk_engine_common::physics::ColliderPart` layout.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ColliderPart {
    pub shape: u32,
    pub local_pos: [f32; 3],
    /// Unit quaternion xyzw.
    pub local_rot: [f32; 4],
}

impl ColliderPart {
    pub fn identity(shape: u32) -> Self {
        Self {
            shape,
            local_pos: [0.0; 3],
            local_rot: [0.0, 0.0, 0.0, 1.0],
        }
    }

    pub fn with_pos(shape: u32, local_pos: [f32; 3]) -> Self {
        Self {
            shape,
            local_pos,
            local_rot: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

/// Out-param for [`PhysicsApiResolved::cast_capsule`]. Must match host trampoline.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CapsuleCastHit {
    pub hit: u32,
    pub start_solid: u32,
    pub toi: f32,
    pub normal: [f32; 3],
    pub point: [f32; 3],
    pub entity: u64,
}

type CreateShapeFn = extern "C" fn(*mut u8, *const u8, usize) -> u32;
type DestroyShapeFn = extern "C" fn(*mut u8, u32) -> i32;
type CreatePartsFn = extern "C" fn(*mut u8, *const ColliderPart, usize) -> u32;
type DestroyPartsFn = extern "C" fn(*mut u8, u32) -> i32;
type CastCapsuleFn = extern "C" fn(
    *mut u8,
    *const f32,
    f32,
    f32,
    *const f32,
    f32,
    u64,
    *mut CapsuleCastHit,
) -> i32;

type TraceLineFn = extern "C" fn(
    *mut u8,
    f32,
    f32,
    f32,
    f32,
    f32,
    f32,
    u64,
    *mut PhysicsTraceResult,
) -> i32;

/// Rapier segment trace (`v1:physics:trace_line`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PhysicsTraceResult {
    pub hit: u32,
    pub entity: u64,
    pub fraction: f32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

impl Default for PhysicsTraceResult {
    fn default() -> Self {
        Self {
            hit: 0,
            entity: 0,
            fraction: 1.0,
            point: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
        }
    }
}

pub struct PhysicsApiResolved {
    ctx: *mut u8,
    create_shape_fn: CreateShapeFn,
    destroy_shape_fn: DestroyShapeFn,
    create_parts_fn: CreatePartsFn,
    destroy_parts_fn: DestroyPartsFn,
    cast_capsule_fn: CastCapsuleFn,
    trace_line_fn: TraceLineFn,
}

unsafe impl Send for PhysicsApiResolved {}
unsafe impl Sync for PhysicsApiResolved {}

impl PhysicsApiResolved {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            create_shape_fn: unsafe { std::mem::transmute(resolve(r, "v1:physics:create_shape")) },
            destroy_shape_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:physics:destroy_shape"))
            },
            create_parts_fn: unsafe { std::mem::transmute(resolve(r, "v1:physics:create_parts")) },
            destroy_parts_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:physics:destroy_parts"))
            },
            cast_capsule_fn: unsafe { std::mem::transmute(resolve(r, "v1:physics:cast_capsule")) },
            trace_line_fn: unsafe { std::mem::transmute(resolve(r, "v1:physics:trace_line")) },
        }
    }

    /// Register shape ABI blob → arena handle (`0` = fail).
    pub fn create_shape(&self, abi: &[u8]) -> u32 {
        (self.create_shape_fn)(self.ctx, abi.as_ptr(), abi.len())
    }

    pub fn destroy_shape(&self, handle: u32) {
        let _ = (self.destroy_shape_fn)(self.ctx, handle);
    }

    /// Register compound parts list → parts handle (`0` = fail).
    pub fn create_parts(&self, parts: &[ColliderPart]) -> u32 {
        (self.create_parts_fn)(self.ctx, parts.as_ptr(), parts.len())
    }

    pub fn destroy_parts(&self, handle: u32) {
        let _ = (self.destroy_parts_fn)(self.ctx, handle);
    }

    /// Swept centered capsule. `dir` need not be unit; distance capped by `max_dist`.
    /// `exclude_entity` = 0 to exclude none.
    pub fn cast_capsule(
        &self,
        center: [f32; 3],
        half_height: f32,
        radius: f32,
        dir: [f32; 3],
        max_dist: f32,
        exclude_entity: u64,
    ) -> Option<CapsuleCastHit> {
        let mut out = CapsuleCastHit::default();
        let rc = (self.cast_capsule_fn)(
            self.ctx,
            center.as_ptr(),
            half_height,
            radius,
            dir.as_ptr(),
            max_dist,
            exclude_entity,
            &mut out,
        );
        if rc < 0 {
            return None;
        }
        if out.hit != 0 {
            Some(out)
        } else {
            None
        }
    }

    /// Segment trace vs Rapier colliders (`exclude_entity` = 0 for none).
    pub fn trace_line(
        &self,
        origin: [f32; 3],
        end: [f32; 3],
        exclude_entity: u64,
    ) -> Option<PhysicsTraceResult> {
        let mut out = PhysicsTraceResult::default();
        let hit = (self.trace_line_fn)(
            self.ctx,
            origin[0],
            origin[1],
            origin[2],
            end[0],
            end[1],
            end[2],
            exclude_entity,
            &mut out,
        );
        if hit != 0 {
            Some(out)
        } else {
            None
        }
    }
}
