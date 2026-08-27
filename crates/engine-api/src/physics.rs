use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

pub struct PhysicsApiResolved {
    ctx: *mut u8,
    spawn_dynamic_box_fn: extern "C" fn(*mut u8, u64, f32, f32, f32, f32, f32, f32, f32),
}

unsafe impl Send for PhysicsApiResolved {}
unsafe impl Sync for PhysicsApiResolved {}

impl PhysicsApiResolved {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            spawn_dynamic_box_fn: unsafe { std::mem::transmute(resolve(r, "v1:physics:spawn_dynamic_box")) },
        }
    }

    pub fn spawn_dynamic_box(
        &self,
        entity: u64,
        x: f32, y: f32, z: f32,
        half_x: f32, half_y: f32, half_z: f32,
        mass: f32,
    ) {
        (self.spawn_dynamic_box_fn)(self.ctx, entity, x, y, z, half_x, half_y, half_z, mass);
    }
}
