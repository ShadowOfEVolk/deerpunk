//! Flecs systems for physics sync (manual `ecs_run` from prepare / writeback, not in default pipeline).

use std::ffi::CString;

use flecs_ecs_sys as sys;

use super::rapier_backend::RapierPhysicsBackend;
use crate::world::World;

fn register_manual_system(
    world: &World,
    backend: *mut RapierPhysicsBackend,
    name: &str,
    query: &str,
    callback: sys::ecs_iter_action_t,
) -> u64 {
    let c_name = CString::new(name).expect("system name");
    let c_query = CString::new(query).expect("system query");
    unsafe {
        let mut entity_desc: sys::ecs_entity_desc_t = std::mem::zeroed();
        entity_desc.id = 0;
        entity_desc.name = c_name.as_ptr();
        let mut flecs_desc: sys::ecs_system_desc_t = std::mem::zeroed();
        flecs_desc.entity = sys::ecs_entity_init(world.as_ptr(), &entity_desc);
        flecs_desc.query.expr = c_query.as_ptr();
        flecs_desc.callback = callback;
        flecs_desc.ctx = backend as *mut RapierPhysicsBackend as *mut std::ffi::c_void;
        flecs_desc.phase = 0;
        sys::ecs_system_init(world.as_ptr(), &flecs_desc).into()
    }
}

pub fn register_sync_systems(backend: &mut RapierPhysicsBackend, world: &World) {
    let ptr = backend as *mut RapierPhysicsBackend;
    backend.set_sync_system_ids(
        register_manual_system(
            world,
            ptr,
            "PhysicsSyncIn",
            "CharacterController, Transform",
            Some(system_physics_sync_in),
        ),
        register_manual_system(
            world,
            ptr,
            "PhysicsWritebackCharacters",
            "CharacterController, Transform",
            Some(system_physics_writeback_characters),
        ),
        register_manual_system(
            world,
            ptr,
            "PhysicsWritebackDynamics",
            "Collider, Transform",
            Some(system_physics_writeback_dynamics),
        ),
    );
}

unsafe extern "C-unwind" fn system_physics_sync_in(it: *mut sys::ecs_iter_t) {
    unsafe {
        if it.is_null() {
            return;
        }
        let it_ref = &*it;
        if it_ref.ctx.is_null() || it_ref.count <= 0 {
            return;
        }
        let backend = &mut *(it_ref.ctx as *mut RapierPhysicsBackend);
        if backend.world_view.is_null() {
            return;
        }
        let world = unsafe { &*backend.world_view };
        let dt = it_ref.delta_time.max(0.0);
        for i in 0..it_ref.count as isize {
            let entity = *it_ref.entities.add(i as usize);
            backend.integrate_character(world, entity, dt);
        }
    }
}

unsafe extern "C-unwind" fn system_physics_writeback_characters(it: *mut sys::ecs_iter_t) {
    unsafe {
        if it.is_null() {
            return;
        }
        let it_ref = &*it;
        if it_ref.ctx.is_null() || it_ref.count <= 0 {
            return;
        }
        let backend = &mut *(it_ref.ctx as *mut RapierPhysicsBackend);
        if backend.world_view.is_null() {
            return;
        }
        let world = unsafe { &*backend.world_view };
        for i in 0..it_ref.count as isize {
            let entity = *it_ref.entities.add(i as usize);
            backend.writeback_character(world, entity);
        }
    }
}

unsafe extern "C-unwind" fn system_physics_writeback_dynamics(it: *mut sys::ecs_iter_t) {
    unsafe {
        if it.is_null() {
            return;
        }
        let it_ref = &*it;
        if it_ref.ctx.is_null() || it_ref.count <= 0 {
            return;
        }
        let backend = &mut *(it_ref.ctx as *mut RapierPhysicsBackend);
        if backend.world_view.is_null() {
            return;
        }
        let world = unsafe { &*backend.world_view };
        for i in 0..it_ref.count as isize {
            let entity = *it_ref.entities.add(i as usize);
            backend.writeback_dynamic(world, entity);
        }
    }
}
