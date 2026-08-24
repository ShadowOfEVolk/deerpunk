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

// ── Global engine context (singleton) ─────────────────────────

use std::sync::atomic::{AtomicPtr, Ordering};
use std::ffi::c_void;

static ENGINE_CTX: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

fn set_engine_ctx(ctx: *mut c_void) {
    ENGINE_CTX.store(ctx, Ordering::Release);
}

fn engine_ctx() -> *mut c_void {
    ENGINE_CTX.load(Ordering::Acquire)
}

/// Access world directly from global engine context.
/// Used by sdk-server's with_world_mut bridge.
pub unsafe fn world_ptr_from_ctx() -> *mut hecs::World {
    unsafe { &mut (*(engine_ctx() as *mut ServerEngine)).ecs_world as *mut hecs::World }
}

/// Access physics_world directly from global engine context.
pub unsafe fn physics_ptr_from_ctx() -> *mut deerpunk_engine_common::physics::PhysicsWorld {
    unsafe { &mut (*(engine_ctx() as *mut ServerEngine)).physics_world as *mut deerpunk_engine_common::physics::PhysicsWorld }
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

        let ctx = &mut self.server_engine as *mut ServerEngine as *mut c_void;
        set_engine_ctx(ctx);

        let world_vtable = WorldApiVtable {
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

// ── Helper: get ServerEngine from global static ────────────────

unsafe fn from_engine() -> &'static mut ServerEngine {
    unsafe { &mut *(engine_ctx() as *mut ServerEngine) }
}

/// Access individual fields via raw pointer WITHOUT creating &mut ServerEngine.
/// This avoids aliasing mutable references when trampolines are called reentrantly
/// (e.g. during schedule.run() where tramp_frame holds the ServerEngine while
/// system callbacks call tramp_get/tramp_set).
mod field {
    use std::collections::HashMap;
    use super::{ServerEngine, api::{ComponentEntry, ComponentId}};
    use super::engine_ctx;

    pub unsafe fn ecs_world() -> &'static mut hecs::World {
        unsafe { &mut (*(engine_ctx() as *mut ServerEngine)).ecs_world }
    }

    pub unsafe fn components() -> &'static HashMap<ComponentId, ComponentEntry> {
        unsafe { &(*(engine_ctx() as *mut ServerEngine)).components }
    }

    pub unsafe fn components_mut() -> &'static mut HashMap<ComponentId, ComponentEntry> {
        unsafe { &mut (*(engine_ctx() as *mut ServerEngine)).components }
    }

    pub unsafe fn schedule() -> &'static mut super::Schedule {
        unsafe { &mut (*(engine_ctx() as *mut ServerEngine)).schedule }
    }

    pub unsafe fn entities() -> &'static mut Vec<u64> {
        unsafe { &mut (*(engine_ctx() as *mut ServerEngine)).entities }
    }

    pub unsafe fn physics_world() -> &'static mut deerpunk_engine_common::physics::PhysicsWorld {
        unsafe { &mut (*(engine_ctx() as *mut ServerEngine)).physics_world }
    }
}

use std::any::Any;

extern "C" fn tramp_register_component(
    name: *const u8,
    name_len: usize,
    has_fn: crate::vtable::CompHasFn,
    get_fn: crate::vtable::CompGetFn,
    set_fn: crate::vtable::CompSetFn,
    remove_fn: crate::vtable::CompRemoveFn,
) -> u64 {
    let se = unsafe { from_engine() };
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name, name_len))
    };
    se.register_component_with_fns(name, has_fn, get_fn, set_fn, remove_fn)
}

extern "C" fn tramp_get(
    entity: u64,
    comp: u64,
) -> *mut c_void {
    let world = unsafe { field::ecs_world() };
    let components = unsafe { field::components() };
    let entity = api::entity_to_hecs(entity);
    let Some(entry) = components.get(&comp) else {
        return std::ptr::null_mut();
    };
    match (entry.get_boxed)(world, entity) {
        Some(bx) => Box::into_raw(Box::new(bx)) as *mut c_void,
        None => std::ptr::null_mut(),
    }
}

extern "C" fn tramp_set(
    entity: u64,
    comp: u64,
    data: *mut c_void,
) {
    let world = unsafe { field::ecs_world() };
    let components = unsafe { field::components() };
    let boxed = unsafe { Box::from_raw(data as *mut Box<dyn Any>) };
    let entity = api::entity_to_hecs(entity);
    if let Some(entry) = components.get(&comp) {
        (entry.set_boxed)(world, entity, *boxed);
    }
}

extern "C" fn tramp_has(
    entity: u64,
    comp: u64,
) -> bool {
    let world = unsafe { field::ecs_world() };
    let components = unsafe { field::components() };
    let entity = api::entity_to_hecs(entity);
    let Some(entry) = components.get(&comp) else {
        return false;
    };
    (entry.has)(world, entity)
}

extern "C" fn tramp_remove(
    entity: u64,
    comp: u64,
) {
    let world = unsafe { field::ecs_world() };
    let components = unsafe { field::components() };
    let entity = api::entity_to_hecs(entity);
    if let Some(entry) = components.get(&comp) {
        (entry.remove)(world, entity);
    }
}

extern "C" fn tramp_spawn() -> u64 {
    let se = unsafe { from_engine() };
    se.spawn()
}

extern "C" fn tramp_spawn_bundle(
    comp_ids: *const u64,
    comp_data: *const *mut c_void,
    count: usize,
) -> u64 {
    let se = unsafe { from_engine() };
    let ids = unsafe { std::slice::from_raw_parts(comp_ids, count) };
    let data = unsafe { std::slice::from_raw_parts(comp_data, count) };
    let mut components = Vec::with_capacity(count);
    for i in 0..count {
        let outer: Box<Box<dyn Any>> = unsafe { Box::from_raw(data[i] as *mut Box<dyn Any>) };
        components.push((ids[i], *outer));
    }
    se.spawn_bundle(components)
}

extern "C" fn tramp_destroy(entity: u64) {
    let world = unsafe { field::ecs_world() };
    let entities = unsafe { field::entities() };
    let hecs_entity = api::entity_to_hecs(entity);
    let _ = world.despawn(hecs_entity);
    entities.retain(|&e| e != entity);
}

extern "C" fn tramp_clear_world() {
    let se = unsafe { from_engine() };
    se.clear();
}

extern "C" fn tramp_query(
    require: *const u64,
    require_n: usize,
    exclude: *const u64,
    exclude_n: usize,
    out: *mut u64,
    max: usize,
) -> usize {
    let world = unsafe { field::ecs_world() };
    let components = unsafe { field::components() };
    let entities = unsafe { field::entities() };
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
    require: *const u64,
    require_n: usize,
    exclude: *const u64,
    exclude_n: usize,
) -> usize {
    let world = unsafe { field::ecs_world() };
    let components = unsafe { field::components() };
    let entities = unsafe { field::entities() };
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
    entity: u64,
    require: *const u64,
    require_n: usize,
    exclude: *const u64,
    exclude_n: usize,
) -> bool {
    let world = unsafe { field::ecs_world() };
    let components = unsafe { field::components() };
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

extern "C" fn tramp_free_value(ptr: *mut c_void) {
    if !ptr.is_null() {
        unsafe {
            drop(Box::from_raw(ptr as *mut Box<dyn Any>));
        }
    }
}

struct VtableSystem {
    func: crate::vtable::ExternSystemFn,
    data: *mut c_void,
}
unsafe impl Send for VtableSystem {}

impl crate::schedule::System for VtableSystem {
    fn run(&mut self, delta_time: f32) {
        (self.func)(delta_time, self.data);
    }
}

extern "C" fn tramp_add_system(
    system_fn: crate::vtable::ExternSystemFn,
    user_data: *mut c_void,
) {
    let se = unsafe { from_engine() };
    let system = VtableSystem { func: system_fn, data: user_data };
    se.schedule.add_system(system);
}

extern "C" fn tramp_clear_systems() {
    let se = unsafe { from_engine() };
    se.schedule.clear();
}

extern "C" fn tramp_schedule_run(delta_time: f32) {
    unsafe {
        let schedule = field::schedule();
        schedule.run(delta_time);
    }
}

extern "C" fn tramp_initialize(gamedir: *const u8, gamedir_len: usize) {
    let se = unsafe { from_engine() };
    let gamedir = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(gamedir, gamedir_len)) };
    let _ = se.initialize(gamedir);
}

extern "C" fn tramp_frame(delta_time: f32) -> i32 {
    unsafe {
        deerpunk_engine_common::cmd::cmd().frame();
        let schedule = field::schedule();
        schedule.run(delta_time);
        let world = field::ecs_world();
        let physics = field::physics_world();
        deerpunk_engine_common::physics::physics_sync_in(physics, world);
        deerpunk_engine_common::physics::physics_step(physics, delta_time);
        deerpunk_engine_common::physics::physics_sync_out(physics, world);
    }
    0
}

extern "C" fn tramp_shutdown() {
    let se = unsafe { from_engine() };
    se.shutdown();
}

extern "C" fn tramp_with_world_mut(
    callback: crate::vtable::WorldMutCallback,
    user_data: *mut c_void,
    _free_user_data: crate::vtable::FreeUserDataFn,
) {
    let world = unsafe { field::ecs_world() };
    callback(world as *mut hecs::World as *mut c_void, user_data);
}

// ── Physics trampolines ────────────────────────────────────────

type PhysicsCallback = extern "C" fn(physics: *mut c_void, user_data: *mut c_void);

extern "C" fn tramp_with_physics(
    callback: PhysicsCallback,
    user_data: *mut c_void,
) {
    unsafe {
        let physics = field::physics_world();
        callback(physics as *mut deerpunk_engine_common::physics::PhysicsWorld as *mut c_void, user_data);
    }
}
