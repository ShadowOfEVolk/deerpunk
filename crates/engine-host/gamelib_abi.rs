//! Gamelib-facing C callback types (MethodDB / DLL ABI).

/// `v1:world:query_execute` gamelib callback.
pub type WorldQueryCallback =
    extern "C" fn(u64, *const *const std::ffi::c_void, usize, *mut std::ffi::c_void);

/// `v1:schedule:add_system` gamelib system fn (runs after `server_frame`, before `ecs_progress`).
pub type ScheduleSystemFn = extern "C" fn(f32, *mut std::ffi::c_void);

/// `v1:world:system_register` — runs inside Flecs `ecs_progress` (OnUpdate) when query matches.
pub type WorldProgressSystemFn = extern "C" fn(*mut std::ffi::c_void);
