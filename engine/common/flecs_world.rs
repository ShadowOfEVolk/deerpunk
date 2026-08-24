//! Safe Rust wrapper around Flecs C API.
//!
//! Provides a hecs-like API surface using Flecs under the hood.
//! Component registration is string-based (no TypeId), solving the DLL boundary problem.
//! Components are identified by `ComponentId` (u64) which is stable across compilation units.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_void;
use std::ptr;

use crate::flecs_sys as sys;

// ── Type aliases ──

/// Entity handle — a u64, same as Flecs entity ID.
pub type Entity = u64;

/// Component identifier — a u64, returned by `register_component`.
pub type ComponentId = u64;

// ── FlecsWorld ──

/// Safe wrapper around a Flecs world.
pub struct FlecsWorld {
    world: *mut sys::ecs_world_t,
    /// Map from component name to ComponentId.
    component_ids: HashMap<String, ComponentId>,
}

unsafe impl Send for FlecsWorld {}
unsafe impl Sync for FlecsWorld {}

impl FlecsWorld {
    /// Create a new world.
    pub fn new() -> Self {
        let world = unsafe { sys::ecs_mini() };
        assert!(!world.is_null(), "ecs_mini failed");
        Self {
            world,
            component_ids: HashMap::new(),
        }
    }

    /// Get raw pointer to the world (for FFI / vtable calls).
    pub fn raw_ptr(&self) -> *mut sys::ecs_world_t {
        self.world
    }

    /// Get mutable raw pointer to the world.
    pub fn raw_ptr_mut(&mut self) -> *mut sys::ecs_world_t {
        self.world
    }

    /// Register a component by name. Returns ComponentId.
    /// If already registered, returns existing ID.
    ///
    /// Component IDs are stable across DLL boundaries because they are
    /// entity IDs in Flecs, not Rust TypeIds.
    pub fn register_component(&mut self, name: &str, size: usize, alignment: usize) -> ComponentId {
        if let Some(&id) = self.component_ids.get(name) {
            return id;
        }

        let c_name = CString::new(name).unwrap();

        // Use the C shim which handles ecs_component_desc_t construction correctly
        let comp_id = unsafe {
            sys::flecs_register_component(
                self.world,
                0, // no existing entity
                c_name.as_ptr(),
                size as i32,
                alignment as i32,
            )
        };

        self.component_ids.insert(name.to_string(), comp_id);
        comp_id
    }

    /// Create an OnAdd observer for a component.
    /// The C callback (`on_physics_body_added` in shim.c) calls `flecs_physics_body_register`.
    pub fn observe_physics_body(&self, component_id: ComponentId) {
        unsafe {
            sys::flecs_observe_physics_body(self.world, component_id);
        }
    }

    /// Create a new entity with no components.
    pub fn spawn(&mut self) -> Entity {
        unsafe { sys::ecs_new(self.world) }
    }

    /// Create a new entity with a component attached.
    pub fn spawn_with<T: Copy>(&mut self, component_id: ComponentId, value: &T) -> Entity {
        let entity = unsafe { sys::ecs_new(self.world) };
        self.set(entity, component_id, value);
        entity
    }

    /// Set a component on an entity.
    pub fn set<T: Copy>(&self, entity: Entity, component_id: ComponentId, value: &T) {
        let sz = std::mem::size_of::<T>();
        if sz == 0 {
            panic!("set<T> called with zero-size type for component_id={} entity={}", component_id, entity);
        }
        unsafe {
            sys::ecs_set_id(
                self.world,
                entity,
                component_id,
                sz,
                value as *const T as *const c_void,
            );
        }
    }

    /// Set raw bytes on an entity.
    pub fn set_raw(&self, entity: Entity, component_id: ComponentId, ptr: *const c_void, size: usize) {
        if size == 0 {
            panic!("set_raw called with size=0 for component_id={} entity={}", component_id, entity);
        }
        unsafe {
            sys::ecs_set_id(self.world, entity, component_id, size, ptr);
        }
    }

    /// Get a copy of a component on an entity.
    pub fn get<T: Copy>(&self, entity: Entity, component_id: ComponentId) -> Option<T> {
        let ptr = unsafe { sys::flecs_get_id_wrapper(self.world, entity, component_id) };
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { *(ptr as *const T) })
        }
    }

    /// Get a raw pointer to a component on an entity.
    pub fn get_raw(&self, entity: Entity, component_id: ComponentId) -> Option<*const c_void> {
        let ptr = unsafe { sys::flecs_get_id_wrapper(self.world, entity, component_id) };
        if ptr.is_null() { None } else { Some(ptr) }
    }

    /// Get a mutable pointer to a component on an entity.
    pub fn get_mut_raw(&self, entity: Entity, component_id: ComponentId) -> Option<*mut c_void> {
        let ptr = unsafe { sys::flecs_get_mut_id_wrapper(self.world, entity, component_id) };
        if ptr.is_null() { None } else { Some(ptr) }
    }

    /// Check if entity has a component.
    pub fn has(&self, entity: Entity, component_id: ComponentId) -> bool {
        unsafe { sys::flecs_has_id_wrapper(self.world, entity, component_id) }
    }

    /// Add a component tag to an entity (no data).
    pub fn add(&self, entity: Entity, component_id: ComponentId) {
        unsafe { sys::ecs_add_id(self.world, entity, component_id) }
    }

    /// Remove a component from an entity.
    pub fn remove(&self, entity: Entity, component_id: ComponentId) {
        unsafe { sys::ecs_remove_id(self.world, entity, component_id) }
    }

    /// Delete an entity and all its components.
    pub fn delete(&self, entity: Entity) {
        unsafe { sys::ecs_delete(self.world, entity) }
    }

    /// Check if entity is valid.
    pub fn is_valid(&self, entity: Entity) -> bool {
        unsafe { sys::ecs_is_valid(self.world, entity) }
    }

    /// Resolve a component name to its ComponentId.
    pub fn resolve_component(&self, name: &str) -> Option<ComponentId> {
        self.component_ids.get(name).copied()
    }

    /// Get ComponentId of a registered component (panics if not registered).
    pub fn component_id(&self, name: &str) -> ComponentId {
        *self.component_ids.get(name).unwrap_or_else(|| {
            panic!("Component '{}' not registered", name);
        })
    }

    /// Run the frame (progress the world).
    pub fn progress(&self, delta_time: f32) -> bool {
        unsafe { sys::ecs_progress(self.world, delta_time) }
    }

    /// Create a query (returns handle that caller must manage).
    pub fn create_query(&self, terms: &[QueryTerm]) -> QueryHandle {
        QueryHandle::new(self.world, terms)
    }

    /// Run a query with a callback. The callback receives a QueryIter.
    pub fn query<F>(&self, terms: &[QueryTerm], mut f: F)
    where
        F: FnMut(&mut QueryIter),
    {
        let query = QueryHandle::new(self.world, terms);
        let mut iter = QueryIter::new(self.world, &query);
        while iter.next_batch() {
            f(&mut iter);
        }
    }

    /// Get the number of alive entities in the world.
    pub fn entity_count(&self) -> usize {
        let entities = unsafe { sys::ecs_get_entities(self.world) };
        entities.alive_count as usize
    }

    /// Lookup an entity by name.
    pub fn lookup(&self, name: &str) -> Option<Entity> {
        let c_name = CString::new(name).unwrap();
        let id = unsafe { sys::ecs_lookup(self.world, c_name.as_ptr()) };
        if id == 0 { None } else { Some(id) }
    }

    /// Set the name of an entity.
    pub fn set_name(&self, entity: Entity, name: &str) {
        let c_name = CString::new(name).unwrap();
        unsafe { sys::ecs_set_name(self.world, entity, c_name.as_ptr()) };
    }

    /// Get the name of an entity.
    pub fn get_name(&self, entity: Entity) -> Option<String> {
        let ptr = unsafe { sys::ecs_get_name(self.world, entity) };
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned())
        }
    }

    /// Quit the world.
    pub fn quit(&self) {
        unsafe { sys::ecs_quit(self.world) }
    }
}

impl Drop for FlecsWorld {
    fn drop(&mut self) {
        unsafe { sys::ecs_fini(self.world) };
    }
}

// ── QueryTerm ──

/// A single term in a query (component + access mode).
#[derive(Clone, Debug)]
pub struct QueryTerm {
    pub component_id: ComponentId,
    pub inout: i16,
    pub oper: i16,
}

impl QueryTerm {
    pub fn read(component_id: ComponentId) -> Self {
        Self { component_id, inout: sys::EcsIn, oper: sys::EcsAnd }
    }

    pub fn write(component_id: ComponentId) -> Self {
        Self { component_id, inout: sys::EcsOut, oper: sys::EcsAnd }
    }

    pub fn read_write(component_id: ComponentId) -> Self {
        Self { component_id, inout: sys::EcsInOut, oper: sys::EcsAnd }
    }
}

// ── QueryHandle ──

/// A Flecs query handle. Must be explicitly dropped.
pub struct QueryHandle {
    query: *mut sys::ecs_query_t,
}

unsafe impl Send for QueryHandle {}

impl QueryHandle {
    pub fn new(world: *mut sys::ecs_world_t, terms: &[QueryTerm]) -> Self {
        let mut desc: sys::ecs_query_desc_t = unsafe { std::mem::zeroed() };
        desc._canary = 0;

        for (i, term) in terms.iter().enumerate() {
            if i >= sys::FLECS_TERM_COUNT_MAX {
                break;
            }
            desc.terms[i] = sys::ecs_term_t {
                id: term.component_id,
                src: sys::ecs_term_ref_t { id: 0, name: ptr::null() },
                first: sys::ecs_term_ref_t { id: 0, name: ptr::null() },
                second: sys::ecs_term_ref_t { id: 0, name: ptr::null() },
                trav: 0,
                inout: term.inout,
                oper: term.oper,
                field_index: i as i8,
                flags_: 0,
            };
        }

        let query = unsafe { sys::ecs_query_init(world, &desc) };
        assert!(!query.is_null(), "ecs_query_init failed");

        Self { query }
    }

    pub fn raw_ptr(&self) -> *mut sys::ecs_query_t {
        self.query
    }
}

impl Drop for QueryHandle {
    fn drop(&mut self) {
        unsafe { sys::ecs_query_fini(self.query) };
    }
}

// ── QueryIter ──

/// Iterator over query results. All iteration happens through C shim —
/// the ecs_iter_t never crosses the FFI boundary.
pub struct QueryIter {
    iter: *mut sys::flecs_query_iter_t,
    current_count: i32,
    done: bool,
}

unsafe impl Send for QueryIter {}

impl Drop for QueryIter {
    fn drop(&mut self) {
        unsafe { sys::flecs_query_iter_fini(self.iter) };
    }
}

impl QueryIter {
    pub fn new(world: *mut sys::ecs_world_t, query: &QueryHandle) -> Self {
        let iter = unsafe {
            sys::flecs_query_iter_init(world, query.raw_ptr())
        };
        Self {
            iter,
            current_count: 0,
            done: false,
        }
    }

    /// Advance to next batch of entities. Returns false when done.
    pub fn next_batch(&mut self) -> bool {
        if self.done {
            return false;
        }
        let result = unsafe { sys::flecs_query_iter_next(self.iter) };
        if result {
            self.current_count = unsafe { sys::flecs_query_iter_get_count(self.iter) };
        } else {
            self.done = true;
        }
        result
    }

    /// Number of entities in current batch.
    pub fn batch_count(&self) -> i32 {
        self.current_count
    }

    /// Get entity at a specific row in current batch.
    pub fn entity_at(&self, row: i32) -> Entity {
        unsafe { sys::flecs_query_iter_get_entity_at(self.iter, row) }
    }

    /// Get a raw pointer to the component array for a field (contiguous, N elements).
    pub fn field_raw(&self, index: i8) -> *mut c_void {
        unsafe { sys::flecs_query_iter_field(self.iter, 0, index) }
    }

    /// Get a typed slice of a field.
    pub fn field<T: Copy>(&self, index: i8) -> Option<&[T]> {
        let ptr = unsafe { sys::flecs_query_iter_field(self.iter, std::mem::size_of::<T>(), index) };
        if ptr.is_null() {
            None
        } else {
            let count = self.current_count as usize;
            Some(unsafe { std::slice::from_raw_parts(ptr as *const T, count) })
        }
    }

    /// Get a mutable typed slice of a field.
    pub fn field_mut<T: Copy>(&self, index: i8) -> Option<&mut [T]> {
        let ptr = unsafe { sys::flecs_query_iter_field(self.iter, std::mem::size_of::<T>(), index) };
        if ptr.is_null() {
            None
        } else {
            let count = self.current_count as usize;
            Some(unsafe { std::slice::from_raw_parts_mut(ptr as *mut T, count) })
        }
    }

    /// Get a specific entity's component from a field (single-row access).
    pub fn field_at<T: Copy>(&self, index: i8, row: i32) -> Option<T> {
        let ptr = unsafe {
            sys::flecs_query_iter_field_at(self.iter, std::mem::size_of::<T>(), index, row)
        };
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { *(ptr as *const T) })
        }
    }

    /// Iterate over all entities in the current batch.
    pub fn for_each<F: FnMut(Entity, i32)>(&self, mut f: F) {
        let ptr = unsafe { sys::flecs_query_iter_get_entities_ptr(self.iter) };
        let count = self.current_count;
        if ptr.is_null() || count <= 0 {
            return;
        }
        for i in 0..count {
            unsafe { f(*ptr.add(i as usize), i) }
        }
    }

    /// Get entities pointer for the current batch (raw Flecs API).
    pub fn entities_ptr(&self) -> *const u64 {
        unsafe { sys::flecs_query_iter_get_entities_ptr(self.iter) }
    }
}
