pub use hecs::*;

pub mod schedule;
pub use schedule::Schedule;

pub mod api;
pub use api::{ComponentId, EntityId};

pub mod vtable;
pub use vtable::{EngineApiHandle, EngineApiVtable};

pub mod server_engine;
pub use server_engine::ServerEngine;

pub mod client_engine;
pub use client_engine::ClientEngine;

pub mod common {
    pub use deerpunk_engine_common::*;
    pub mod input { pub use deerpunk_engine_common::input::*; }
    pub mod ipc { pub use deerpunk_engine_common::ipc::*; }
    pub mod frame_buffer { pub use deerpunk_engine_common::frame_buffer::*; }
}

#[cfg(feature = "dedicated-server")]
pub mod server {
    pub use deerpunk_engine_server::*;
}

#[cfg(feature = "client")]
pub mod client {
    pub use deerpunk_engine_client::*;
}

/// Access world directly from ctx pointer without creating &mut ServerEngine.
/// Used by sdk-server's with_world_mut bridge to avoid aliasing during schedule.run().
pub unsafe fn world_ptr_from_ctx(ctx: *mut std::ffi::c_void) -> *mut hecs::World {
    unsafe { &mut (*(ctx as *mut ServerEngine)).ecs_world as *mut hecs::World }
}

/// Access physics_world directly from ctx pointer without creating &mut ServerEngine.
pub unsafe fn physics_ptr_from_ctx(ctx: *mut std::ffi::c_void) -> *mut deerpunk_engine_common::physics::PhysicsWorld {
    unsafe { &mut (*(ctx as *mut ServerEngine)).physics_world as *mut deerpunk_engine_common::physics::PhysicsWorld }
}

pub struct Host {
    pub server_engine: ServerEngine,
    pub client_engine: Option<ClientEngine>,
}

impl Host {
    pub fn new() -> Self {
        Self {
            server_engine: ServerEngine::new(),
            client_engine: Some(ClientEngine),
        }
    }

    pub fn build_vtable(&mut self) -> vtable::EngineApiHandle {
        use crate::vtable::{EngineApiHandle, EngineApiVtable, ScheduleApiVtable, WorldApiVtable};

        let ctx = &mut self.server_engine as *mut ServerEngine as *mut std::ffi::c_void;

        let world_vtable = WorldApiVtable {
            ctx,
            register_component: tramp_register_component,
            get: tramp_get,
            set: tramp_set,
            has: tramp_has,
            remove: tramp_remove,
            spawn: tramp_spawn,
            spawn_bundle: tramp_spawn_bundle,
            destroy: tramp_destroy,
            clear: tramp_clear_world,
            query: tramp_query,
            query_count: tramp_query_count,
            query_matches: tramp_query_matches,
            free_value: tramp_free_value,
        };

        let schedule_vtable = ScheduleApiVtable {
            ctx,
            add_system: tramp_add_system,
            clear: tramp_clear_systems,
            run: tramp_schedule_run,
        };

        let vtable = Box::new(EngineApiVtable {
            world: world_vtable,
            schedule: schedule_vtable,
            initialize: tramp_initialize,
            frame: tramp_frame,
            shutdown: tramp_shutdown,
            with_world_mut: tramp_with_world_mut,
            with_physics: tramp_with_physics,
        });

        EngineApiHandle::new(vtable)
    }
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}

// ── extern "C" trampolines ──────────────────────────────────────

unsafe fn from_ptr<'a>(ptr: *mut std::ffi::c_void) -> &'a mut ServerEngine {
    unsafe { &mut *(ptr as *mut ServerEngine) }
}

/// Access individual fields via raw pointer WITHOUT creating &mut ServerEngine.
/// This avoids aliasing mutable references when trampolines are called reentrantly
/// (e.g. during schedule.run() where tramp_frame holds the ServerEngine while
/// system callbacks call tramp_get/tramp_set).
mod field {
    use super::ServerEngine;
    use std::ffi::c_void;
    use std::collections::HashMap;
    use super::api::{ComponentEntry, ComponentId};

    pub unsafe fn ecs_world(ctx: *mut c_void) -> &'static mut hecs::World {
        unsafe { &mut (*(ctx as *mut ServerEngine)).ecs_world }
    }

    pub unsafe fn components(ctx: *mut c_void) -> &'static HashMap<ComponentId, ComponentEntry> {
        unsafe { &(*(ctx as *mut ServerEngine)).components }
    }

    pub unsafe fn components_mut(ctx: *mut c_void) -> &'static mut HashMap<ComponentId, ComponentEntry> {
        unsafe { &mut (*(ctx as *mut ServerEngine)).components }
    }

    pub unsafe fn schedule(ctx: *mut c_void) -> &'static mut super::Schedule {
        unsafe { &mut (*(ctx as *mut ServerEngine)).schedule }
    }

    pub unsafe fn entities(ctx: *mut c_void) -> &'static mut Vec<u64> {
        unsafe { &mut (*(ctx as *mut ServerEngine)).entities }
    }

    pub unsafe fn physics_world(ctx: *mut c_void) -> &'static mut deerpunk_engine_common::physics::PhysicsWorld {
        unsafe { &mut (*(ctx as *mut ServerEngine)).physics_world }
    }
}

use std::any::Any;

extern "C" fn tramp_register_component(
    ctx: *mut std::ffi::c_void,
    name: *const u8,
    name_len: usize,
    has_fn: crate::vtable::CompHasFn,
    get_fn: crate::vtable::CompGetFn,
    set_fn: crate::vtable::CompSetFn,
    remove_fn: crate::vtable::CompRemoveFn,
) -> u64 {
    let se = unsafe { from_ptr(ctx) };
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name, name_len))
    };
    se.register_component_with_fns(name, has_fn, get_fn, set_fn, remove_fn)
}

extern "C" fn tramp_get(
    ctx: *mut std::ffi::c_void,
    entity: u64,
    comp: u64,
) -> *mut std::ffi::c_void {
    let world = unsafe { field::ecs_world(ctx) };
    let components = unsafe { field::components(ctx) };
    let entity = api::entity_to_hecs(entity);
    let Some(entry) = components.get(&comp) else {
        return std::ptr::null_mut();
    };
    match (entry.get_boxed)(world, entity) {
        Some(bx) => Box::into_raw(Box::new(bx)) as *mut std::ffi::c_void,
        None => std::ptr::null_mut(),
    }
}

extern "C" fn tramp_set(
    ctx: *mut std::ffi::c_void,
    entity: u64,
    comp: u64,
    data: *mut std::ffi::c_void,
) {
    let world = unsafe { field::ecs_world(ctx) };
    let components = unsafe { field::components(ctx) };
    let boxed = unsafe { Box::from_raw(data as *mut Box<dyn Any>) };
    let entity = api::entity_to_hecs(entity);
    if let Some(entry) = components.get(&comp) {
        (entry.set_boxed)(world, entity, *boxed);
    }
}

extern "C" fn tramp_has(
    ctx: *mut std::ffi::c_void,
    entity: u64,
    comp: u64,
) -> bool {
    let world = unsafe { field::ecs_world(ctx) };
    let components = unsafe { field::components(ctx) };
    let entity = api::entity_to_hecs(entity);
    let Some(entry) = components.get(&comp) else {
        return false;
    };
    (entry.has)(world, entity)
}

extern "C" fn tramp_remove(
    ctx: *mut std::ffi::c_void,
    entity: u64,
    comp: u64,
) {
    let world = unsafe { field::ecs_world(ctx) };
    let components = unsafe { field::components(ctx) };
    let entity = api::entity_to_hecs(entity);
    if let Some(entry) = components.get(&comp) {
        (entry.remove)(world, entity);
    }
}

extern "C" fn tramp_spawn(ctx: *mut std::ffi::c_void) -> u64 {
    let se = unsafe { from_ptr(ctx) };
    se.spawn()
}

extern "C" fn tramp_spawn_bundle(
    ctx: *mut std::ffi::c_void,
    comp_ids: *const u64,
    comp_data: *const *mut std::ffi::c_void,
    count: usize,
) -> u64 {
    let se = unsafe { from_ptr(ctx) };
    let ids = unsafe { std::slice::from_raw_parts(comp_ids, count) };
    let data = unsafe { std::slice::from_raw_parts(comp_data, count) };
    let mut components = Vec::with_capacity(count);
    for i in 0..count {
        let outer: Box<Box<dyn Any>> = unsafe { Box::from_raw(data[i] as *mut Box<dyn Any>) };
        components.push((ids[i], *outer));
    }
    se.spawn_bundle(components)
}

extern "C" fn tramp_destroy(ctx: *mut std::ffi::c_void, entity: u64) {
    let world = unsafe { field::ecs_world(ctx) };
    let entities = unsafe { field::entities(ctx) };
    let hecs_entity = api::entity_to_hecs(entity);
    let _ = world.despawn(hecs_entity);
    entities.retain(|&e| e != entity);
}

extern "C" fn tramp_clear_world(ctx: *mut std::ffi::c_void) {
    let se = unsafe { from_ptr(ctx) };
    se.clear();
}

extern "C" fn tramp_query(
    ctx: *mut std::ffi::c_void,
    require: *const u64,
    require_n: usize,
    exclude: *const u64,
    exclude_n: usize,
    out: *mut u64,
    max: usize,
) -> usize {
    let world = unsafe { field::ecs_world(ctx) };
    let components = unsafe { field::components(ctx) };
    let entities = unsafe { field::entities(ctx) };
    let req = unsafe { std::slice::from_raw_parts(require, require_n) };
    let exc = if exclude.is_null() || exclude_n == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(exclude, exclude_n) }
    };

    use std::collections::HashSet;
    let require_set: HashSet<u64> = req.iter().copied().collect();
    let exclude_set: HashSet<u64> = exc.iter().copied().collect();

    let has_comp = |entity_id: u64, comp_id: u64| -> bool {
        let hecs_entity = api::entity_to_hecs(entity_id);
        match components.get(&comp_id) {
            Some(entry) => (entry.has)(world, hecs_entity),
            None => false,
        }
    };

    let result: Vec<u64> = if require_set.is_empty() {
        entities.iter()
            .filter(|&&e| !exclude_set.iter().any(|&c| has_comp(e, c)))
            .copied()
            .collect()
    } else {
        entities.iter()
            .filter(|&&e| {
                require_set.iter().all(|&c| has_comp(e, c))
                    && !exclude_set.iter().any(|&c| has_comp(e, c))
            })
            .copied()
            .collect()
    };

    let count = result.len().min(max);
    if !out.is_null() {
        for i in 0..count {
            unsafe {
                *out.add(i) = result[i];
            }
        }
    }
    count
}

extern "C" fn tramp_query_count(
    ctx: *mut std::ffi::c_void,
    require: *const u64,
    require_n: usize,
    exclude: *const u64,
    exclude_n: usize,
) -> usize {
    let world = unsafe { field::ecs_world(ctx) };
    let components = unsafe { field::components(ctx) };
    let entities = unsafe { field::entities(ctx) };
    let req = unsafe { std::slice::from_raw_parts(require, require_n) };
    let exc = if exclude.is_null() || exclude_n == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(exclude, exclude_n) }
    };

    use std::collections::HashSet;
    let require_set: HashSet<u64> = req.iter().copied().collect();
    let exclude_set: HashSet<u64> = exc.iter().copied().collect();

    let has_comp = |entity_id: u64, comp_id: u64| -> bool {
        let hecs_entity = api::entity_to_hecs(entity_id);
        match components.get(&comp_id) {
            Some(entry) => (entry.has)(world, hecs_entity),
            None => false,
        }
    };

    if require_set.is_empty() {
        entities.iter()
            .filter(|&&e| !exclude_set.iter().any(|&c| has_comp(e, c)))
            .count()
    } else {
        entities.iter()
            .filter(|&&e| {
                require_set.iter().all(|&c| has_comp(e, c))
                    && !exclude_set.iter().any(|&c| has_comp(e, c))
            })
            .count()
    }
}

extern "C" fn tramp_query_matches(
    ctx: *mut std::ffi::c_void,
    entity: u64,
    require: *const u64,
    require_n: usize,
    exclude: *const u64,
    exclude_n: usize,
) -> bool {
    let world = unsafe { field::ecs_world(ctx) };
    let components = unsafe { field::components(ctx) };
    let req = unsafe { std::slice::from_raw_parts(require, require_n) };
    let exc = if exclude.is_null() || exclude_n == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(exclude, exclude_n) }
    };

    let has_comp = |comp_id: u64| -> bool {
        let hecs_entity = api::entity_to_hecs(entity);
        match components.get(&comp_id) {
            Some(entry) => (entry.has)(world, hecs_entity),
            None => false,
        }
    };

    req.iter().all(|&c| has_comp(c)) && !exc.iter().any(|&c| has_comp(c))
}

extern "C" fn tramp_free_value(ptr: *mut std::ffi::c_void) {
    if !ptr.is_null() {
        unsafe {
            drop(Box::from_raw(ptr as *mut Box<dyn Any>));
        }
    }
}

struct VtableSystem {
    func: crate::vtable::ExternSystemFn,
    data: *mut std::ffi::c_void,
}
unsafe impl Send for VtableSystem {}

impl crate::schedule::System for VtableSystem {
    fn run(&mut self, delta_time: f32) {
        (self.func)(delta_time, self.data);
    }
}

extern "C" fn tramp_add_system(
    ctx: *mut std::ffi::c_void,
    system_fn: crate::vtable::ExternSystemFn,
    user_data: *mut std::ffi::c_void,
) {
    let se = unsafe { from_ptr(ctx) };
    let system = VtableSystem { func: system_fn, data: user_data };
    se.schedule.add_system(system);
}

extern "C" fn tramp_clear_systems(ctx: *mut std::ffi::c_void) {
    let se = unsafe { from_ptr(ctx) };
    se.schedule.clear();
}

extern "C" fn tramp_schedule_run(ctx: *mut std::ffi::c_void, delta_time: f32) {
    unsafe {
        let schedule = field::schedule(ctx);
        schedule.run(delta_time);
    }
}

extern "C" fn tramp_initialize(ctx: *mut std::ffi::c_void, gamedir: *const u8, gamedir_len: usize) {
    let se = unsafe { from_ptr(ctx) };
    let gamedir = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(gamedir, gamedir_len)) };
    let _ = se.initialize(gamedir);
}

extern "C" fn tramp_frame(ctx: *mut std::ffi::c_void, delta_time: f32) -> i32 {
    unsafe {
        deerpunk_engine_common::cmd::cmd().frame();
        let schedule = field::schedule(ctx);
        schedule.run(delta_time);
        // Engine-internal: physics sync runs after all game systems
        let world = field::ecs_world(ctx);
        let physics = field::physics_world(ctx);
        deerpunk_engine_common::physics::physics_sync_in(physics, world);
        deerpunk_engine_common::physics::physics_step(physics, delta_time);
        deerpunk_engine_common::physics::physics_sync_out(physics, world);
    }
    0
}

extern "C" fn tramp_shutdown(ctx: *mut std::ffi::c_void) {
    let se = unsafe { from_ptr(ctx) };
    se.shutdown();
}

extern "C" fn tramp_with_world_mut(
    ctx: *mut std::ffi::c_void,
    callback: crate::vtable::WorldMutCallback,
    user_data: *mut std::ffi::c_void,
    _free_user_data: crate::vtable::FreeUserDataFn,
) {
    let world = unsafe { field::ecs_world(ctx) };
    callback(world as *mut hecs::World as *mut std::ffi::c_void, user_data);
}

// ── Physics trampolines ────────────────────────────────────────


type PhysicsCallback = extern "C" fn(physics: *mut std::ffi::c_void, user_data: *mut std::ffi::c_void);

extern "C" fn tramp_with_physics(
    ctx: *mut std::ffi::c_void,
    callback: PhysicsCallback,
    user_data: *mut std::ffi::c_void,
) {
    unsafe {
        let physics = field::physics_world(ctx);
        callback(physics as *mut deerpunk_engine_common::physics::PhysicsWorld as *mut std::ffi::c_void, user_data);
    }
}
