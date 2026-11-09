//! Gamelib-facing C callback types (MethodDB / DLL ABI).

/// `v1:world:query_execute` gamelib callback.
pub type WorldQueryCallback =
    extern "C" fn(u64, *const *const std::ffi::c_void, usize, *mut std::ffi::c_void);

/// `v1:schedule:add_system` gamelib system fn (runs after `server_frame`, before `ecs_progress`).
pub type ScheduleSystemFn = extern "C" fn(f32, *mut std::ffi::c_void);

/// `v1:world:system_register` — once per matching Flecs table.
/// `entities` / `fields` / `sizes` are valid only for that call.
/// `fields[term]` is query term `term`; row `i` is `fields[term] + i * sizes[term]`.
pub type WorldProgressSystemFn = extern "C" fn(
    entities: *const u64,
    count: u32,
    fields: *const *const std::ffi::c_void,
    field_count: u32,
    sizes: *const usize,
    dt: f32,
    user_data: *mut std::ffi::c_void,
);
