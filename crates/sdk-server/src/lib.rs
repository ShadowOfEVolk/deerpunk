//! deerpunk-sdk-server — Safe Rust wrapper around the C ABI vtable.
//!
//! The engine creates a `#[repr(C)]` vtable with function pointers.
//! This crate wraps those pointers into ergonomic Rust methods.

use std::ffi::c_void;

pub use deerpunk_engine_host::vtable::EngineApiVtable;

pub type EntityId = u64;
pub type ComponentId = u64;

// ── InputApi ────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub struct InputApi {
    vtable: deerpunk_engine_host::vtable::InputApiVtable,
}

impl InputApi {
    pub fn is_key_down(&self, key: u32) -> bool {
        (self.vtable.is_key_down)(key)
    }
    pub fn mouse_dx(&self) -> f32 {
        (self.vtable.mouse_dx)()
    }
    pub fn mouse_dy(&self) -> f32 {
        (self.vtable.mouse_dy)()
    }
}

// ── ImGuiApi ────────────────────────────────────────────────

pub struct ImGuiApi {
    vtable: deerpunk_engine_host::vtable::ImGuiApiVtable,
}

impl ImGuiApi {
    pub fn begin_window(&self, name: &str) -> bool {
        let bytes = name.as_bytes();
        (self.vtable.begin_window)(bytes.as_ptr(), bytes.len())
    }

    pub fn end_window(&self) {
        (self.vtable.end_window)();
    }

    pub fn text(&self, text: &str) {
        let bytes = text.as_bytes();
        (self.vtable.text)(bytes.as_ptr(), bytes.len());
    }

    pub fn value_f32(&self, label: &str, value: f32) {
        let bytes = label.as_bytes();
        (self.vtable.value_f32)(bytes.as_ptr(), bytes.len(), value);
    }

    pub fn value_vec3(&self, label: &str, x: f32, y: f32, z: f32) {
        let bytes = label.as_bytes();
        (self.vtable.value_vec3)(bytes.as_ptr(), bytes.len(), x, y, z);
    }

    pub fn separator(&self) {
        (self.vtable.separator)();
    }
}

// ── PhysicsApi ────────────────────────────────────────────────

pub struct PhysicsApi {
    vtable: deerpunk_engine_host::vtable::PhysicsApiVtable,
}

impl PhysicsApi {
    pub fn spawn_dynamic_box(
        &self,
        entity: EntityId,
        x: f32, y: f32, z: f32,
        half_x: f32, half_y: f32, half_z: f32,
        mass: f32,
    ) {
        (self.vtable.spawn_dynamic_box)(entity, x, y, z, half_x, half_y, half_z, mass);
    }
}

// ── EngineApi (top-level, owns vtable data) ───────────────────

pub struct EngineApi {
    pub world: WorldApi,
    pub schedule: ScheduleApi,
    pub input: InputApi,
    pub imgui: ImGuiApi,
    pub physics: PhysicsApi,
    raw: EngineApiVtable,
}
unsafe impl Send for EngineApi {}
unsafe impl Sync for EngineApi {}

impl EngineApi {
    pub fn new(vtable: Box<EngineApiVtable>) -> Self {
        Self {
            world: WorldApi { vtable: vtable.world },
            schedule: ScheduleApi { vtable: vtable.schedule },
            input: InputApi { vtable: vtable.input },
            imgui: ImGuiApi { vtable: vtable.imgui },
            physics: PhysicsApi { vtable: vtable.physics },
            raw: *vtable,
        }
    }

    pub fn from_handle(handle: deerpunk_engine_host::vtable::EngineApiHandle) -> Self {
        let raw = unsafe { *handle.vtable_ptr() };
        Self {
            world: WorldApi { vtable: raw.world },
            schedule: ScheduleApi { vtable: raw.schedule },
            input: InputApi { vtable: raw.input },
            imgui: ImGuiApi { vtable: raw.imgui },
            physics: PhysicsApi { vtable: raw.physics },
            raw,
        }
    }

    pub fn clone_ref(&self) -> Self {
        Self {
            world: self.world.clone(),
            schedule: self.schedule.clone(),
            input: self.input,
            imgui: ImGuiApi { vtable: self.raw.imgui },
            physics: PhysicsApi { vtable: self.raw.physics },
            raw: self.raw,
        }
    }

    pub fn vtable_raw(&self) -> &EngineApiVtable {
        &self.raw
    }

    pub fn initialize(&self, gamedir: &str) -> Result<(), String> {
        let bytes = gamedir.as_bytes();
        (self.raw.initialize)(bytes.as_ptr(), bytes.len());
        Ok(())
    }

    pub fn frame(&self, delta_time: f32) -> Result<(), String> {
        if (self.raw.frame)(delta_time) == 0 {
            Ok(())
        } else {
            Err("frame failed".to_string())
        }
    }

    pub fn shutdown(&self) {
        (self.raw.shutdown)();
    }

    // ── Physics ─────────────────────────────────────────────────

    pub fn with_physics<R>(&self, f: impl FnOnce(&deerpunk_engine_host::common::physics::PhysicsWorld) -> R) -> R {
        let physics_ptr = unsafe {
            deerpunk_engine_host::physics_ptr_from_ctx()
        };
        f(unsafe { &*physics_ptr })
    }

    pub fn with_physics_mut<R>(&self, f: impl FnOnce(&mut deerpunk_engine_host::common::physics::PhysicsWorld) -> R) -> R {
        let physics_ptr = unsafe {
            deerpunk_engine_host::physics_ptr_from_ctx()
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
    /// Register a component by name.
    /// Returns a ComponentId that is stable across DLL boundaries.
    /// The engine already registered this component and knows its size.
    /// For engine-registered components (Transform, CameraComponent, etc.), this returns the existing ID.
    /// For DLL-registered components, pass the correct size via `register_component_with_size`.
    pub fn register_component(&self, name: &str) -> ComponentId {
        let name_bytes = name.as_bytes();
        (self.vtable.register_component)(
            name_bytes.as_ptr(),
            name_bytes.len(),
            0, // size=0: engine uses its registered size (for engine types like Transform)
            0, // alignment=0: engine uses its registered alignment
        )
    }

    /// Register a DLL-owned component by name with explicit size and alignment.
    /// Use std::mem::size_of::<T>() and std::mem::align_of::<T>() for the correct values.
    pub fn register_component_with_size<T>(&self, name: &str) -> ComponentId {
        let name_bytes = name.as_bytes();
        (self.vtable.register_component)(
            name_bytes.as_ptr(),
            name_bytes.len(),
            std::mem::size_of::<T>(),
            std::mem::align_of::<T>(),
        )
    }

    pub fn query(&self, require: &[ComponentId], exclude: &[ComponentId]) -> Vec<EntityId> {
        let max = 4096;
        let mut buf = vec![0u64; max];
        let count = (self.vtable.query)(
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

    /// Get a raw pointer to a component on an entity.
    /// The returned pointer is valid as long as the entity exists and the component is not removed.
    pub fn get_raw(&self, entity: EntityId, comp: ComponentId) -> Option<*const c_void> {
        let ptr = (self.vtable.get)(entity, comp);
        if ptr.is_null() { None } else { Some(ptr as *const c_void) }
    }

    /// Get a copy of a component on an entity.
    /// The type T must have the same layout as the registered component.
    /// Uses raw pointer copy — safe for types that are repr(C) and have the same size.
    pub fn get<T: 'static>(&self, entity: EntityId, comp: ComponentId) -> Option<T> {
        let ptr = (self.vtable.get)(entity, comp);
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { std::ptr::read_unaligned(ptr as *const T) })
        }
    }

    /// Set a component on an entity from raw pointer.
    pub fn set_raw(&self, entity: EntityId, comp: ComponentId, ptr: *const c_void, size: usize) {
        (self.vtable.set)(entity, comp, ptr as *mut c_void);
    }

    /// Set a component on an entity.
    /// The value is passed by pointer; Flecs copies the registered size bytes.
    pub fn set<T>(&self, entity: EntityId, comp: ComponentId, value: T) {
        let ptr = &value as *const T as *mut c_void;
        (self.vtable.set)(entity, comp, ptr);
        std::mem::forget(value); // Don't drop — Flecs owns the data now
    }

    pub fn has(&self, entity: EntityId, comp: ComponentId) -> bool {
        (self.vtable.has)(entity, comp)
    }

    pub fn remove(&self, entity: EntityId, comp: ComponentId) {
        (self.vtable.remove)(entity, comp);
    }

    pub fn spawn(&self) -> EntityId {
        (self.vtable.spawn)()
    }

    /// Spawn an entity with components. Each component is (id, data_ptr, size).
    /// The data must be valid raw bytes of the component.
    pub fn spawn_bundle_raw(
        &self,
        components: &[(ComponentId, *const c_void, usize)],
    ) -> EntityId {
        let count = components.len();
        if count == 0 {
            return self.spawn();
        }

        let ids: Vec<u64> = components.iter().map(|(id, _, _)| *id).collect();
        let data: Vec<*mut c_void> = components.iter()
            .map(|(_, ptr, _)| *ptr as *mut c_void)
            .collect();

        (self.vtable.spawn_bundle)(
            ids.as_ptr(),
            data.as_ptr(),
            count,
        )
    }

    /// Spawn an entity with components. Each component is a (component_id, Box<dyn Any>).
    /// The Box<dyn Any>'s data pointer is extracted and passed to the engine as raw bytes.
    /// This is for backward compatibility — prefer spawn_bundle_raw for new code.
    pub fn spawn_bundle(
        &self,
        components: Vec<(ComponentId, Box<dyn std::any::Any>)>,
    ) -> EntityId {
        let count = components.len();
        if count == 0 {
            return self.spawn();
        }

        let ids: Vec<u64> = components.iter().map(|(id, _)| *id).collect();
        let data: Vec<*mut c_void> = components
            .into_iter()
            .map(|(_, d)| {
                // Extract the data pointer from the Box<dyn Any> fat pointer.
                // For a Box<T>, this gives a pointer to the T value on the heap.
                // The engine copies registered_size bytes from this pointer into Flecs.
                // NOTE: the Box will drop the T after this, potentially freeing Vec data.
                // This is safe for Copy types and types without heap allocations.
                let raw = Box::into_raw(d) as *const u8 as *mut c_void;
                raw
            })
            .collect();

        let result = (self.vtable.spawn_bundle)(
            ids.as_ptr(),
            data.as_ptr(),
            count,
        );

        // Note: we intentionally do NOT free the Box-allocated data here.
        // The engine has copied the bytes into Flecs storage via ecs_set_id.
        // The Box-allocated memory will be leaked as a temporary measure.
        // TODO: implement proper deep-copy for non-Copy types.

        result
    }

    pub fn destroy(&self, entity: EntityId) {
        (self.vtable.destroy)(entity);
    }

    pub fn clear(&self) {
        (self.vtable.clear)();
    }

    pub fn query_count(&self, require: &[ComponentId], exclude: &[ComponentId]) -> usize {
        (self.vtable.query_count)(
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
            entity,
            require.as_ptr(),
            require.len(),
            exclude.as_ptr(),
            exclude.len(),
        )
    }

    pub fn resolve_component(&self, name: &str) -> Option<ComponentId> {
        let name_bytes = name.as_bytes();
        let id = (self.vtable.resolve_component)(name_bytes.as_ptr(), name_bytes.len());
        if id == u64::MAX { None } else { Some(id) }
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
        (self.vtable.add_system)(tramp::<S>, boxed);
    }

    pub fn clear(&self) {
        (self.vtable.clear)();
    }

    pub fn run(&self, delta_time: f32) {
        (self.vtable.run)(delta_time);
    }
}
