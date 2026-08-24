use std::ffi::c_void;

pub type ExternSystemFn = extern "C" fn(delta_time: f32, user_data: *mut c_void);

// ── WorldApiVtable ─────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WorldApiVtable {
    pub register_component: extern "C" fn(
        name: *const u8,
        name_len: usize,
        size: usize,
        alignment: usize,
    ) -> u64,

    pub get: extern "C" fn(entity: u64, comp: u64) -> *mut c_void,
    pub set: extern "C" fn(entity: u64, comp: u64, data: *mut c_void),
    pub has: extern "C" fn(entity: u64, comp: u64) -> bool,
    pub remove: extern "C" fn(entity: u64, comp: u64),

    pub spawn: extern "C" fn() -> u64,
    pub spawn_bundle: extern "C" fn(
        comp_ids: *const u64,
        comp_data: *const *mut c_void,
        count: usize,
    ) -> u64,
    pub destroy: extern "C" fn(entity: u64),
    pub clear: extern "C" fn(),

    pub query: extern "C" fn(
        require: *const u64,
        require_n: usize,
        exclude: *const u64,
        exclude_n: usize,
        out: *mut u64,
        max: usize,
    ) -> usize,

    pub query_count: extern "C" fn(
        require: *const u64,
        require_n: usize,
        exclude: *const u64,
        exclude_n: usize,
    ) -> usize,

    pub query_matches: extern "C" fn(
        entity: u64,
        require: *const u64,
        require_n: usize,
        exclude: *const u64,
        exclude_n: usize,
    ) -> bool,

    pub free_value: extern "C" fn(ptr: *mut c_void),

    pub resolve_component: extern "C" fn(name: *const u8, name_len: usize) -> u64,
}

// ── ScheduleApiVtable ──────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ScheduleApiVtable {
    pub add_system: extern "C" fn(
        system_fn: ExternSystemFn,
        user_data: *mut c_void,
    ),

    pub clear: extern "C" fn(),
    pub run: extern "C" fn(delta_time: f32),
}

// ── InputApiVtable ────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InputApiVtable {
    pub is_key_down: extern "C" fn(key: u32) -> bool,
    pub mouse_dx: extern "C" fn() -> f32,
    pub mouse_dy: extern "C" fn() -> f32,
}

// ── ImGuiApiVtable ────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ImGuiApiVtable {
    pub begin_window: extern "C" fn(name: *const u8, name_len: usize) -> bool,
    pub end_window: extern "C" fn(),
    pub text: extern "C" fn(text: *const u8, text_len: usize),
    pub value_f32: extern "C" fn(label: *const u8, label_len: usize, value: f32),
    pub value_vec3: extern "C" fn(label: *const u8, label_len: usize, x: f32, y: f32, z: f32),
    pub separator: extern "C" fn(),
}

// ── PhysicsApiVtable ────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PhysicsApiVtable {
    pub spawn_dynamic_box: extern "C" fn(
        entity: u64,
        x: f32, y: f32, z: f32,
        half_x: f32, half_y: f32, half_z: f32,
        mass: f32,
    ),
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
    pub input: InputApiVtable,
    pub imgui: ImGuiApiVtable,
    pub physics: PhysicsApiVtable,

    pub initialize: extern "C" fn(gamedir: *const u8, gamedir_len: usize),
    pub frame: extern "C" fn(delta_time: f32) -> i32,
    pub shutdown: extern "C" fn(),

    pub with_world_mut: extern "C" fn(
        callback: WorldMutCallback,
        user_data: *mut c_void,
        free_user_data: FreeUserDataFn,
    ),

    pub with_physics: extern "C" fn(
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
