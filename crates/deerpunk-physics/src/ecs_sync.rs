//! Physics sync systems (manual `run_system` from prepare / writeback, not in the default pipeline).

use super::rapier_backend::RapierPhysicsBackend;
use deerpunk_engine_world::{EcsCallback, EntityId, SystemSchedule, World};

extern "C-unwind" fn system_physics_sync_in(
    entities: *const EntityId,
    count: u32,
    dt: f32,
    ctx: *mut std::ffi::c_void,
) {
    if entities.is_null() || ctx.is_null() || count == 0 {
        return;
    }
    let backend = unsafe { &mut *(ctx as *mut RapierPhysicsBackend) };
    if backend.world_view.is_null() {
        return;
    }
    let world = unsafe { &*backend.world_view };
    let dt = dt.max(0.0);
    for i in 0..count as usize {
        let entity = unsafe { *entities.add(i) };
        backend.integrate_character(world, entity, dt);
    }
}

extern "C-unwind" fn system_physics_writeback_characters(
    entities: *const EntityId,
    count: u32,
    _dt: f32,
    ctx: *mut std::ffi::c_void,
) {
    if entities.is_null() || ctx.is_null() || count == 0 {
        return;
    }
    let backend = unsafe { &mut *(ctx as *mut RapierPhysicsBackend) };
    if backend.world_view.is_null() {
        return;
    }
    let world = unsafe { &*backend.world_view };
    for i in 0..count as usize {
        let entity = unsafe { *entities.add(i) };
        backend.writeback_character(world, entity);
    }
}

extern "C-unwind" fn system_physics_writeback_dynamics(
    entities: *const EntityId,
    count: u32,
    _dt: f32,
    ctx: *mut std::ffi::c_void,
) {
    if entities.is_null() || ctx.is_null() || count == 0 {
        return;
    }
    let backend = unsafe { &mut *(ctx as *mut RapierPhysicsBackend) };
    if backend.world_view.is_null() {
        return;
    }
    let world = unsafe { &*backend.world_view };
    for i in 0..count as usize {
        let entity = unsafe { *entities.add(i) };
        backend.writeback_dynamic(world, entity);
    }
}

/// Bridge prepare pass: create missing `RigidBody` bodies (idempotent), then
/// teleport static/kinematic to Transform and copy `Velocity` into dynamic
/// bodies. Runs in prepare, before PhysicsSyncIn.
extern "C-unwind" fn system_physics_rigid_body_prepare(
    entities: *const EntityId,
    count: u32,
    _dt: f32,
    ctx: *mut std::ffi::c_void,
) {
    if entities.is_null() || ctx.is_null() || count == 0 {
        return;
    }
    let backend = unsafe { &mut *(ctx as *mut RapierPhysicsBackend) };
    if backend.world_view.is_null() {
        return;
    }
    let world = unsafe { &*backend.world_view };
    for i in 0..count as usize {
        let entity = unsafe { *entities.add(i) };
        backend.prepare_rigid_body(world, entity);
    }
}

/// Bridge writeback pass: dynamic `RigidBody` bodies copy pose → Transform and
/// body velocities → `Velocity`.
extern "C-unwind" fn system_physics_rigid_body_writeback(
    entities: *const EntityId,
    count: u32,
    _dt: f32,
    ctx: *mut std::ffi::c_void,
) {
    if entities.is_null() || ctx.is_null() || count == 0 {
        return;
    }
    let backend = unsafe { &mut *(ctx as *mut RapierPhysicsBackend) };
    if backend.world_view.is_null() {
        return;
    }
    let world = unsafe { &*backend.world_view };
    for i in 0..count as usize {
        let entity = unsafe { *entities.add(i) };
        backend.writeback_rigid_body(world, entity);
    }
}

pub fn register_sync_systems(backend: &mut RapierPhysicsBackend, world: &World) {
    let ptr = backend as *mut RapierPhysicsBackend as *mut std::ffi::c_void;
    backend.set_sync_system_ids(
        world.register_system(
            "PhysicsRigidBodyPrepare",
            "RigidBody, TransformComponent",
            SystemSchedule::Manual,
            ptr,
            system_physics_rigid_body_prepare as EcsCallback,
        ),
        world.register_system(
            "PhysicsSyncIn",
            "CharacterController, TransformComponent",
            SystemSchedule::Manual,
            ptr,
            system_physics_sync_in as EcsCallback,
        ),
        world.register_system(
            "PhysicsWritebackCharacters",
            "CharacterController, TransformComponent",
            SystemSchedule::Manual,
            ptr,
            system_physics_writeback_characters as EcsCallback,
        ),
        world.register_system(
            "PhysicsWritebackDynamics",
            "PhysCollider, TransformComponent",
            SystemSchedule::Manual,
            ptr,
            system_physics_writeback_dynamics as EcsCallback,
        ),
        world.register_system(
            "PhysicsRigidBodyWriteback",
            "RigidBody, TransformComponent",
            SystemSchedule::Manual,
            ptr,
            system_physics_rigid_body_writeback as EcsCallback,
        ),
    );
}
