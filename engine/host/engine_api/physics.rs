use crate::engine_api::engine_api::EngineApiMethod;
use deerpunk_engine_common::physics::ColliderPart;

/// Out-param for [`tramp_physics_cast_capsule`]. Must match engine-api layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CapsuleCastHitOut {
    pub hit: u32,
    pub start_solid: u32,
    pub toi: f32,
    pub normal: [f32; 3],
    pub point: [f32; 3],
}

/// `v1:physics:create_shape(abi_ptr, abi_len) -> handle` (0 = fail).
pub extern "C" fn tramp_physics_create_shape(
    engine: *mut u8,
    abi_ptr: *const u8,
    abi_len: usize,
) -> u32 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    if abi_ptr.is_null() || abi_len == 0 {
        return 0;
    }
    let abi = unsafe { std::slice::from_raw_parts(abi_ptr, abi_len) };
    host.server_engine.physics.create_shape(abi)
}

/// `v1:physics:destroy_shape(handle)`
pub extern "C" fn tramp_physics_destroy_shape(engine: *mut u8, handle: u32) -> i32 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    host.server_engine.physics.destroy_shape(handle);
    0
}

/// `v1:physics:create_parts(parts_ptr, count) -> handle` (0 = fail).
/// `parts_ptr` = array of [`ColliderPart`].
pub extern "C" fn tramp_physics_create_parts(
    engine: *mut u8,
    parts_ptr: *const ColliderPart,
    count: usize,
) -> u32 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    if parts_ptr.is_null() || count == 0 {
        return 0;
    }
    let parts = unsafe { std::slice::from_raw_parts(parts_ptr, count) };
    host.server_engine.physics.create_parts(parts)
}

/// `v1:physics:destroy_parts(handle)`
pub extern "C" fn tramp_physics_destroy_parts(engine: *mut u8, handle: u32) -> i32 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    host.server_engine.physics.destroy_parts(handle);
    0
}

/// `v1:physics:cast_capsule(center, half_height, radius, dir, max_dist, exclude, out) -> i32`
///
/// `center`/`dir` = `[f32; 3]`. `out` = [`CapsuleCastHitOut`]. Returns 1 hit / 0 miss / -1 error.
pub extern "C" fn tramp_physics_cast_capsule(
    engine: *mut u8,
    center: *const f32,
    half_height: f32,
    radius: f32,
    dir: *const f32,
    max_dist: f32,
    exclude_entity: u64,
    out: *mut CapsuleCastHitOut,
) -> i32 {
    if center.is_null() || dir.is_null() || out.is_null() {
        return -1;
    }
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let center = unsafe { [*center, *center.add(1), *center.add(2)] };
    let dir = unsafe { [*dir, *dir.add(1), *dir.add(2)] };
    let out = unsafe { &mut *out };
    *out = CapsuleCastHitOut::default();
    match host.server_engine.physics.cast_capsule(
        center,
        half_height,
        radius,
        dir,
        max_dist,
        exclude_entity,
    ) {
        Some(hit) => {
            out.hit = 1;
            out.start_solid = u32::from(hit.start_solid);
            out.toi = hit.toi;
            out.normal = hit.normal;
            out.point = hit.point;
            1
        }
        None => 0,
    }
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:physics:create_shape",
        callback: tramp_physics_create_shape as *const (),
        flags: 0,
    }
}
inventory::submit! {
    EngineApiMethod {
        name: "v1:physics:destroy_shape",
        callback: tramp_physics_destroy_shape as *const (),
        flags: 0,
    }
}
inventory::submit! {
    EngineApiMethod {
        name: "v1:physics:create_parts",
        callback: tramp_physics_create_parts as *const (),
        flags: 0,
    }
}
inventory::submit! {
    EngineApiMethod {
        name: "v1:physics:destroy_parts",
        callback: tramp_physics_destroy_parts as *const (),
        flags: 0,
    }
}
inventory::submit! {
    EngineApiMethod {
        name: "v1:physics:cast_capsule",
        callback: tramp_physics_cast_capsule as *const (),
        flags: 0,
    }
}
