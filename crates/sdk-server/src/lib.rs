//! deerpunk-sdk-server — Safe Rust wrapper around the C ABI vtable.
//!
//! The engine creates a `#[repr(C)]` vtable with function pointers.
//! This crate wraps those pointers into ergonomic Rust methods.

use std::any::Any;
use std::ffi::c_void;

pub use deerpunk_engine_host::vtable::{CompGetFn, CompHasFn, CompRemoveFn, CompSetFn, EngineApiVtable};

pub type EntityId = u64;
pub type ComponentId = u64;

// ── ComponentFns trait (monomorphized per T) ──────────────────

trait ComponentFns {
    extern "C" fn has(world: *mut c_void, entity: u64) -> bool;
    extern "C" fn get(world: *mut c_void, entity: u64) -> *mut c_void;
    extern "C" fn set(world: *mut c_void, entity: u64, data: *mut c_void);
    extern "C" fn remove(world: *mut c_void, entity: u64);
}

struct ComponentFnsImpl<T>(std::marker::PhantomData<T>);

impl<T: Clone + Send + Sync + 'static> ComponentFns for ComponentFnsImpl<T> {
    extern "C" fn has(world: *mut c_void, entity: u64) -> bool {
        unsafe {
            let world = &*(world as *const hecs::World);
            let e = hecs::Entity::from_bits(entity).unwrap();
            world.get::<&T>(e).is_ok()
        }
    }

    extern "C" fn get(world: *mut c_void, entity: u64) -> *mut c_void {
        unsafe {
            let world = &*(world as *const hecs::World);
            let e = hecs::Entity::from_bits(entity).unwrap();
            match world.get::<&T>(e) {
                Ok(comp) => {
                    let owned: T = (*comp).clone();
                    Box::into_raw(Box::new(Box::new(owned) as Box<dyn Any>)) as *mut c_void
                }
                Err(_) => std::ptr::null_mut(),
            }
        }
    }

    extern "C" fn set(world: *mut c_void, entity: u64, data: *mut c_void) {
        unsafe {
            let world = &mut *(world as *mut hecs::World);
            let e = hecs::Entity::from_bits(entity).unwrap();
            let outer: Box<Box<dyn Any>> = Box::from_raw(data as *mut Box<dyn Any>);
            let inner: Box<dyn Any> = *outer;
            if let Ok(comp) = inner.downcast::<T>() {
                world.insert_one(e, *comp).ok();
            }
        }
    }

    extern "C" fn remove(world: *mut c_void, entity: u64) {
        unsafe {
            let world = &mut *(world as *mut hecs::World);
            let e = hecs::Entity::from_bits(entity).unwrap();
            world.remove_one::<T>(e).ok();
        }
    }
}

// ── EngineApi (top-level, owns vtable data) ───────────────────

pub struct EngineApi {
    pub world: WorldApi,
    pub schedule: ScheduleApi,
    raw: EngineApiVtable,
}
unsafe impl Send for EngineApi {}
unsafe impl Sync for EngineApi {}

impl EngineApi {
    pub fn new(vtable: Box<EngineApiVtable>) -> Self {
        Self {
            world: WorldApi { vtable: vtable.world },
            schedule: ScheduleApi { vtable: vtable.schedule },
            raw: *vtable,
        }
    }

    pub fn from_handle(handle: deerpunk_engine_host::vtable::EngineApiHandle) -> Self {
        // Copy vtable data from the handle, then let handle drop normally.
        // Do NOT Box::from_raw on the handle's pointer — that would double-free.
        let raw = unsafe { *handle.vtable_ptr() };
        Self {
            world: WorldApi { vtable: raw.world },
            schedule: ScheduleApi { vtable: raw.schedule },
            raw,
        }
    }

    pub fn clone_ref(&self) -> Self {
        Self {
            world: self.world.clone(),
            schedule: self.schedule.clone(),
            raw: self.raw,
        }
    }

    pub fn vtable_raw(&self) -> &EngineApiVtable {
        &self.raw
    }

    pub fn initialize(&self, gamedir: &str) -> Result<(), String> {
        let bytes = gamedir.as_bytes();
        (self.raw.initialize)(self.raw.world.ctx, bytes.as_ptr(), bytes.len());
        Ok(())
    }

    pub fn frame(&self, delta_time: f32) -> Result<(), String> {
        if (self.raw.frame)(self.raw.world.ctx, delta_time) == 0 {
            Ok(())
        } else {
            Err("frame failed".to_string())
        }
    }

    pub fn shutdown(&self) {
        (self.raw.shutdown)(self.raw.world.ctx);
    }

    /// Transitional bridge: passes `&mut World` to a closure.
    /// Accesses ecs_world field directly via raw pointer to avoid aliasing.
    pub fn with_world_mut<R>(&self, f: impl FnOnce(&mut hecs::World) -> R) -> R {
        let world_ptr = unsafe {
            deerpunk_engine_host::world_ptr_from_ctx(self.raw.world.ctx)
        };
        f(unsafe { &mut *world_ptr })
    }

    // ── Physics ─────────────────────────────────────────────────

    /// Borrow PhysicsWorld immutably. Uses field-level access on the ctx pointer.
    pub fn with_physics<R>(&self, f: impl FnOnce(&deerpunk_engine_host::common::physics::PhysicsWorld) -> R) -> R {
        let physics_ptr = unsafe {
            deerpunk_engine_host::physics_ptr_from_ctx(self.raw.world.ctx)
        };
        f(unsafe { &*physics_ptr })
    }

    /// Borrow PhysicsWorld mutably. Uses field-level access on the ctx pointer.
    pub fn with_physics_mut<R>(&self, f: impl FnOnce(&mut deerpunk_engine_host::common::physics::PhysicsWorld) -> R) -> R {
        let physics_ptr = unsafe {
            deerpunk_engine_host::physics_ptr_from_ctx(self.raw.world.ctx)
        };
        f(unsafe { &mut *physics_ptr })
    }

    // ── Schedule shortcuts ─────────────────────────────────────

    pub fn add_system<S: deerpunk_engine_host::schedule::System + Send + 'static>(&self, system: S) {
        self.schedule.add_system(system);
    }

    pub fn clear_systems(&self) {
        self.schedule.clear();
    }
}

// ── WorldApi (owned vtable copy) ──────────────────────────────

#[derive(Clone)]
pub struct WorldApi {
    vtable: deerpunk_engine_host::vtable::WorldApiVtable,
}

impl WorldApi {
    pub fn register_component<T: Clone + Send + Sync + 'static>(&self, name: &str) -> ComponentId {
        let name_bytes = name.as_bytes();
        (self.vtable.register_component)(
            self.vtable.ctx,
            name_bytes.as_ptr(),
            name_bytes.len(),
            ComponentFnsImpl::<T>::has,
            ComponentFnsImpl::<T>::get,
            ComponentFnsImpl::<T>::set,
            ComponentFnsImpl::<T>::remove,
        )
    }

    pub fn query(&self, require: &[ComponentId], exclude: &[ComponentId]) -> Vec<EntityId> {
        let max = 4096;
        let mut buf = vec![0u64; max];
        let count = (self.vtable.query)(
            self.vtable.ctx,
            require.as_ptr(),
            require.len(),
            exclude.as_ptr(),
            exclude.len(),
            buf.as_mut_ptr(),
            max,
        );
        buf.truncate(count);
        buf
    }

    pub fn get<T: Clone + 'static>(&self, entity: EntityId, comp: ComponentId) -> Option<T> {
        let ptr = (self.vtable.get)(self.vtable.ctx, entity, comp);
        if ptr.is_null() {
            return None;
        }
        let boxed = unsafe { Box::from_raw(ptr as *mut Box<dyn Any>) };
        (*boxed).downcast::<T>().ok().map(|b| *b)
    }

    pub fn set<T: Any>(&self, entity: EntityId, comp: ComponentId, value: T) {
        let ptr = Box::into_raw(Box::new(Box::new(value) as Box<dyn Any>)) as *mut c_void;
        (self.vtable.set)(self.vtable.ctx, entity, comp, ptr);
    }

    pub fn has(&self, entity: EntityId, comp: ComponentId) -> bool {
        (self.vtable.has)(self.vtable.ctx, entity, comp)
    }

    pub fn remove(&self, entity: EntityId, comp: ComponentId) {
        (self.vtable.remove)(self.vtable.ctx, entity, comp);
    }

    pub fn spawn(&self) -> EntityId {
        (self.vtable.spawn)(self.vtable.ctx)
    }

    pub fn spawn_bundle(
        &self,
        components: Vec<(ComponentId, Box<dyn Any>)>,
    ) -> EntityId {
        let count = components.len();
        if count == 0 {
            return self.spawn();
        }

        let ids: Vec<u64> = components.iter().map(|(id, _)| *id).collect();
        let data: Vec<*mut c_void> = components
            .into_iter()
            .map(|(_, d)| Box::into_raw(Box::new(d)) as *mut c_void)
            .collect();

        let result = (self.vtable.spawn_bundle)(
            self.vtable.ctx,
            ids.as_ptr(),
            data.as_ptr(),
            count,
        );
        result
    }

    pub fn destroy(&self, entity: EntityId) {
        (self.vtable.destroy)(self.vtable.ctx, entity);
    }

    pub fn clear(&self) {
        (self.vtable.clear)(self.vtable.ctx);
    }

    pub fn query_count(&self, require: &[ComponentId], exclude: &[ComponentId]) -> usize {
        (self.vtable.query_count)(
            self.vtable.ctx,
            require.as_ptr(),
            require.len(),
            exclude.as_ptr(),
            exclude.len(),
        )
    }

    pub fn query_matches(
        &self,
        entity: EntityId,
        require: &[ComponentId],
        exclude: &[ComponentId],
    ) -> bool {
        (self.vtable.query_matches)(
            self.vtable.ctx,
            entity,
            require.as_ptr(),
            require.len(),
            exclude.as_ptr(),
            exclude.len(),
        )
    }
}

// ── ScheduleApi (owned vtable copy) ───────────────────────────

#[derive(Clone)]
pub struct ScheduleApi {
    vtable: deerpunk_engine_host::vtable::ScheduleApiVtable,
}

impl ScheduleApi {
    pub fn add_system<S: deerpunk_engine_host::schedule::System + Send + 'static>(&self, system: S) {
        extern "C" fn tramp<S: deerpunk_engine_host::schedule::System + Send + 'static>(
            dt: f32,
            user_data: *mut c_void,
        ) {
            unsafe {
                let system = &mut *(user_data as *mut S);
                system.run(dt);
            }
        }

        let boxed = Box::into_raw(Box::new(system)) as *mut c_void;
        (self.vtable.add_system)(self.vtable.ctx, tramp::<S>, boxed);
    }

    pub fn clear(&self) {
        (self.vtable.clear)(self.vtable.ctx);
    }

    pub fn run(&self, delta_time: f32) {
        (self.vtable.run)(self.vtable.ctx, delta_time);
    }
}
