use std::ffi::c_void;

pub type ExternSystemFn = extern "C" fn(delta_time: f32, user_data: *mut c_void);

pub type CompHasFn = extern "C" fn(world: *mut c_void, entity: u64) -> bool;
pub type CompGetFn = extern "C" fn(world: *mut c_void, entity: u64) -> *mut c_void;
pub type CompSetFn = extern "C" fn(world: *mut c_void, entity: u64, data: *mut c_void);
pub type CompRemoveFn = extern "C" fn(world: *mut c_void, entity: u64);

// ── WorldApiVtable ─────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WorldApiVtable {
    pub ctx: *mut c_void,

    pub register_component: extern "C" fn(
        ctx: *mut c_void,
        name: *const u8,
        name_len: usize,
        has: CompHasFn,
        get: CompGetFn,
        set: CompSetFn,
        remove: CompRemoveFn,
    ) -> u64,

    pub get: extern "C" fn(ctx: *mut c_void, entity: u64, comp: u64) -> *mut c_void,
    pub set: extern "C" fn(ctx: *mut c_void, entity: u64, comp: u64, data: *mut c_void),
    pub has: extern "C" fn(ctx: *mut c_void, entity: u64, comp: u64) -> bool,
    pub remove: extern "C" fn(ctx: *mut c_void, entity: u64, comp: u64),

    pub spawn: extern "C" fn(ctx: *mut c_void) -> u64,
    pub spawn_bundle: extern "C" fn(
        ctx: *mut c_void,
        comp_ids: *const u64,
        comp_data: *const *mut c_void,
        count: usize,
    ) -> u64,
    pub destroy: extern "C" fn(ctx: *mut c_void, entity: u64),
    pub clear: extern "C" fn(ctx: *mut c_void),

    pub query: extern "C" fn(
        ctx: *mut c_void,
        require: *const u64,
        require_n: usize,
        exclude: *const u64,
        exclude_n: usize,
        out: *mut u64,
        max: usize,
    ) -> usize,

    pub query_count: extern "C" fn(
        ctx: *mut c_void,
        require: *const u64,
        require_n: usize,
        exclude: *const u64,
        exclude_n: usize,
    ) -> usize,

    pub query_matches: extern "C" fn(
        ctx: *mut c_void,
        entity: u64,
        require: *const u64,
        require_n: usize,
        exclude: *const u64,
        exclude_n: usize,
    ) -> bool,

    pub free_value: extern "C" fn(ptr: *mut c_void),
}

// ── ScheduleApiVtable ──────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ScheduleApiVtable {
    pub ctx: *mut c_void,

    pub add_system: extern "C" fn(
        ctx: *mut c_void,
        system_fn: ExternSystemFn,
        user_data: *mut c_void,
    ),

    pub clear: extern "C" fn(ctx: *mut c_void),
    pub run: extern "C" fn(ctx: *mut c_void, delta_time: f32),
}

// ── EngineApiVtable (top-level) ────────────────────────────────

pub type WorldMutCallback = extern "C" fn(world: *mut c_void, user_data: *mut c_void);
pub type FreeUserDataFn = extern "C" fn(user_data: *mut c_void);
pub type PhysicsCallback = extern "C" fn(physics: *mut c_void, user_data: *mut c_void);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct EngineApiVtable {
    pub world: WorldApiVtable,
    pub schedule: ScheduleApiVtable,

    pub initialize: extern "C" fn(ctx: *mut c_void, gamedir: *const u8, gamedir_len: usize),
    pub frame: extern "C" fn(ctx: *mut c_void, delta_time: f32) -> i32,
    pub shutdown: extern "C" fn(ctx: *mut c_void),

    pub with_world_mut: extern "C" fn(
        ctx: *mut c_void,
        callback: WorldMutCallback,
        user_data: *mut c_void,
        free_user_data: FreeUserDataFn,
    ),

    pub with_physics: extern "C" fn(
        ctx: *mut c_void,
        callback: PhysicsCallback,
        user_data: *mut c_void,
    ),
}

// ── EngineApiHandle (opaque) ──────────────────────────────────

pub struct EngineApiHandle {
    pub(crate) vtable: Box<EngineApiVtable>,
}

impl EngineApiHandle {
    pub fn new(vtable: Box<EngineApiVtable>) -> Self {
        Self { vtable }
    }

    pub fn vtable_ptr(&self) -> *const EngineApiVtable {
        &*self.vtable as *const EngineApiVtable
    }

    pub fn world(&self) -> &WorldApiVtable {
        &self.vtable.world
    }

    pub fn schedule(&self) -> &ScheduleApiVtable {
        &self.vtable.schedule
    }
}
