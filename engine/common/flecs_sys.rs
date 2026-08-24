//! Raw FFI bindings for the Flecs C library (amalgamation).
//! Only the subset needed by the deerpunk engine.
#![allow(non_camel_case_types, non_upper_case_globals)]

use std::os::raw::{c_char, c_int, c_void};

// ── Core integer / entity types ──

pub type ecs_entity_t = u64;
pub type ecs_id_t = u64;
pub type ecs_size_t = i32;
pub type ecs_ftime_t = f32;

// ── Opaque handles ──

/// Opaque world handle.
#[repr(C)]
pub struct ecs_world_t {
    _private: [u8; 0],
}

/// Opaque query handle.
#[repr(C)]
pub struct ecs_query_t {
    _private: [u8; 0],
}

/// Opaque iterator — always passed as `*mut ecs_iter_t`.
#[repr(C)]
pub struct ecs_iter_t {
    _private: [u8; 0],
}

/// Opaque query iterator — kept entirely in C, never exposed to Rust.
#[repr(C)]
pub struct flecs_query_iter_t {
    _private: [u8; 0],
}

// ── Term / query descriptor types ──

#[repr(C)]
pub struct ecs_term_ref_t {
    pub id: ecs_entity_t,
    pub name: *const c_char,
}

impl Default for ecs_term_ref_t {
    fn default() -> Self {
        Self {
            id: 0,
            name: std::ptr::null(),
        }
    }
}

/// Access mode for a query term.
pub const EcsInOutDefault: i16 = 0;
pub const EcsInOutNone: i16 = 1;
pub const EcsInOutFilter: i16 = 2;
pub const EcsInOut: i16 = 3;
pub const EcsIn: i16 = 4;
pub const EcsOut: i16 = 5;

/// Term operator.
pub const EcsAnd: i16 = 0;
pub const EcsOr: i16 = 1;
pub const EcsOptional: i16 = 3;

#[repr(C)]
pub struct ecs_term_t {
    pub id: ecs_id_t,
    pub src: ecs_term_ref_t,
    pub first: ecs_term_ref_t,
    pub second: ecs_term_ref_t,
    pub trav: ecs_entity_t,
    pub inout: i16,
    pub oper: i16,
    pub field_index: i8,
    pub flags_: u16,
}

impl Default for ecs_term_t {
    fn default() -> Self {
        // Zeroed = all fields 0, which is a valid empty term.
        unsafe { std::mem::zeroed() }
    }
}

pub const FLECS_TERM_COUNT_MAX: usize = 32;

#[repr(C)]
pub struct ecs_query_desc_t {
    pub _canary: i32,
    pub terms: [ecs_term_t; FLECS_TERM_COUNT_MAX],
    pub expr: *const c_char,
    pub cache_kind: i32,
    pub flags: u32,
    pub order_by_callback: *mut c_void,
    pub order_by_table_callback: *mut c_void,
    pub order_by: ecs_entity_t,
    pub group_by: ecs_id_t,
    pub group_by_callback: *mut c_void,
    pub on_group_create: *mut c_void,
    pub on_group_delete: *mut c_void,
    pub group_by_ctx: *mut c_void,
    pub group_by_ctx_free: *mut c_void,
    pub ctx: *mut c_void,
    pub binding_ctx: *mut c_void,
    pub ctx_free: *mut c_void,
    pub binding_ctx_free: *mut c_void,
    pub entity: ecs_entity_t,
}

impl Default for ecs_query_desc_t {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

// ── Component type info (for registration) ──

/// We don't need the full `ecs_type_info_t` — we use `ecs_component_init`
/// with a pre-zeroed descriptor. Component registration uses the simpler
/// `ecs_*_component()` macros internally which call `ecs_component_init`.
/// For Rust, we use the "id" approach: register a new entity as a component.
///
/// The simplest way: create an entity, set its name, then call
/// `ecs_add_id(world, entity, EcsComponent)`. But Flecs actually needs
/// `ecs_component_init` for proper type info.
///
/// We define a safe-sized descriptor blob.

/// Maximum size of `ecs_component_desc_t` + alignment padding.
/// We use a raw byte blob that we zero-fill and only set the fields we need.
pub const ECS_COMPONENT_DESC_SIZE: usize = 1024;

#[repr(C)]
pub struct ecs_component_desc_t {
    pub _canary: i32,
    pub entity: ecs_entity_t,
    pub _rest: [u8; ECS_COMPONENT_DESC_SIZE - 12],
}

// ── System descriptor ──

pub type ecs_iter_action_t = Option<unsafe extern "C" fn(it: *mut ecs_iter_t)>;
pub type ecs_run_action_t = Option<unsafe extern "C" fn(it: *mut ecs_iter_t)>;
pub type ecs_ctx_free_t = Option<unsafe extern "C" fn(ptr: *mut c_void)>;

/// Minimal ecs_system_desc_t — padded generously for ABI stability.
pub const ECS_SYSTEM_DESC_SIZE: usize = 4096;

#[repr(C)]
pub struct ecs_system_desc_t {
    pub _blob: [u8; ECS_SYSTEM_DESC_SIZE],
}

// ── Built-in phase entities ──

extern "C" {
    pub static EcsOnLoad: ecs_entity_t;
    pub static EcsPostLoad: ecs_entity_t;
    pub static EcsPreUpdate: ecs_entity_t;
    pub static EcsOnUpdate: ecs_entity_t;
    pub static EcsOnValidate: ecs_entity_t;
    pub static EcsPostUpdate: ecs_entity_t;
    pub static EcsPreStore: ecs_entity_t;
    pub static EcsOnStore: ecs_entity_t;
    pub static EcsComponent: ecs_entity_t;
}

// ── FFI functions ──

extern "C" {
    // ── World ──
    pub fn ecs_init() -> *mut ecs_world_t;
    pub fn ecs_mini() -> *mut ecs_world_t;
    pub fn ecs_fini(world: *mut ecs_world_t) -> c_int;
    pub fn ecs_progress(world: *mut ecs_world_t, delta_time: ecs_ftime_t) -> bool;
    pub fn ecs_quit(world: *mut ecs_world_t);
    pub fn ecs_should_quit(world: *const ecs_world_t) -> bool;

    // ── Entity lifecycle ──
    pub fn ecs_new(world: *mut ecs_world_t) -> ecs_entity_t;
    pub fn ecs_new_w_id(world: *mut ecs_world_t, component: ecs_id_t) -> ecs_entity_t;
    pub fn ecs_delete(world: *mut ecs_world_t, entity: ecs_entity_t);
    pub fn ecs_is_valid(world: *const ecs_world_t, entity: ecs_entity_t) -> bool;
    pub fn ecs_is_alive(world: *const ecs_world_t, entity: ecs_entity_t) -> bool;

    // ── Component access ──
    pub fn ecs_set_id(
        world: *mut ecs_world_t,
        entity: ecs_entity_t,
        component: ecs_id_t,
        size: usize,
        ptr: *const c_void,
    );
    pub fn flecs_get_id_wrapper(
        world: *const ecs_world_t,
        entity: ecs_entity_t,
        component: ecs_id_t,
    ) -> *const c_void;
    pub fn flecs_get_mut_id_wrapper(
        world: *mut ecs_world_t,
        entity: ecs_entity_t,
        component: ecs_id_t,
    ) -> *mut c_void;
    pub fn flecs_has_id_wrapper(
        world: *const ecs_world_t,
        entity: ecs_entity_t,
        component: ecs_id_t,
    ) -> bool;
    pub fn ecs_add_id(
        world: *mut ecs_world_t,
        entity: ecs_entity_t,
        component: ecs_id_t,
    );
    pub fn ecs_remove_id(
        world: *mut ecs_world_t,
        entity: ecs_entity_t,
        component: ecs_id_t,
    );

    // ── Component registration ──
    pub fn ecs_component_init(
        world: *mut ecs_world_t,
        desc: *const ecs_component_desc_t,
    ) -> ecs_entity_t;

    // ── Entity lookup / naming ──
    pub fn ecs_lookup(world: *const ecs_world_t, name: *const c_char) -> ecs_entity_t;
    pub fn ecs_set_name(
        world: *mut ecs_world_t,
        entity: ecs_entity_t,
        name: *const c_char,
    ) -> ecs_entity_t;
    pub fn ecs_get_name(world: *const ecs_world_t, entity: ecs_entity_t) -> *const c_char;

    // ── Query ──
    pub fn ecs_query_init(
        world: *mut ecs_world_t,
        desc: *const ecs_query_desc_t,
    ) -> *mut ecs_query_t;
    pub fn ecs_query_fini(query: *mut ecs_query_t);

    // ── Opaque query iterator (kept entirely in C) ──
    pub fn flecs_query_iter_init(
        world: *mut ecs_world_t,
        query: *const ecs_query_t,
    ) -> *mut flecs_query_iter_t;
    pub fn flecs_query_iter_next(it: *mut flecs_query_iter_t) -> bool;
    pub fn flecs_query_iter_get_count(it: *mut flecs_query_iter_t) -> i32;
    pub fn flecs_query_iter_get_entity_at(it: *mut flecs_query_iter_t, row: i32) -> ecs_entity_t;
    pub fn flecs_query_iter_get_entities_ptr(it: *mut flecs_query_iter_t) -> *const ecs_entity_t;
    pub fn flecs_query_iter_field(it: *mut flecs_query_iter_t, size: usize, index: i8) -> *mut c_void;
    pub fn flecs_query_iter_field_at(it: *mut flecs_query_iter_t, size: usize, index: i8, row: i32) -> *mut c_void;
    pub fn flecs_query_iter_get_field_count(it: *mut flecs_query_iter_t) -> i8;
    pub fn flecs_query_iter_get_world(it: *mut flecs_query_iter_t) -> *mut ecs_world_t;
    pub fn flecs_query_iter_fini(it: *mut flecs_query_iter_t);

    // ── Pipeline ──
    pub fn ecs_run_pipeline(
        world: *mut ecs_world_t,
        pipeline: ecs_entity_t,
        delta_time: ecs_ftime_t,
    );
    pub fn ecs_set_pipeline(world: *mut ecs_world_t, pipeline: ecs_entity_t);
    pub fn ecs_get_pipeline(world: *const ecs_world_t) -> ecs_entity_t;

    // ── System ──
    pub fn ecs_system_init(
        world: *mut ecs_world_t,
        desc: *const ecs_system_desc_t,
    ) -> ecs_entity_t;

    // ── Misc ──
    pub fn ecs_set_threads(world: *mut ecs_world_t, threads: i32);
    pub fn ecs_get_entities(world: *const ecs_world_t) -> ecs_entities_t;

    // ── C shim ──
    pub fn flecs_register_component(
        world: *mut ecs_world_t,
        existing_entity: ecs_entity_t,
        name: *const c_char,
        size: i32,
        alignment: i32,
    ) -> ecs_entity_t;

    // ── Observer ──
    pub fn flecs_observe_physics_body(
        world: *mut ecs_world_t,
        component_id: ecs_entity_t,
    ) -> ecs_entity_t;
}

/// Returned by `ecs_get_entities`.
#[repr(C)]
pub struct ecs_entities_t {
    pub ids: *const ecs_entity_t,
    pub count: i32,
    pub alive_count: i32,
}
