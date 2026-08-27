use crate::engine_api::engine_api::EngineApiMethod;
use std::ffi::c_void;

// ── Schedule trampolines (stubs — mechanism only) ───────────

pub extern "C" fn tramp_add_system(_engine: *mut u8, _system_fn: extern "C" fn(f32, *mut c_void), _user_data: *mut c_void) {}
pub extern "C" fn tramp_clear_systems(_engine: *mut u8) {}
pub extern "C" fn tramp_schedule_run(_engine: *mut u8, _delta_time: f32) {}

// ── Engine trampolines (stubs) ──────────────────────────────

pub extern "C" fn tramp_initialize(_engine: *mut u8, _gamedir: *const u8, _gamedir_len: usize) {}
pub extern "C" fn tramp_frame(_engine: *mut u8, _delta_time: f32) -> i32 { 0 }
pub extern "C" fn tramp_shutdown(_engine: *mut u8) {}
pub extern "C" fn tramp_with_world_mut(
    _engine: *mut u8,
    _callback: extern "C" fn(*mut c_void, *mut c_void),
    _user_data: *mut c_void,
    _free_user_data: extern "C" fn(*mut c_void),
) {}

// ── Inventory registrations ──────────────────────────────────

inventory::submit! { EngineApiMethod { name: "v1:schedule:add_system", callback: tramp_add_system as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:schedule:clear", callback: tramp_clear_systems as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:schedule:run", callback: tramp_schedule_run as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:engine:initialize", callback: tramp_initialize as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:engine:frame", callback: tramp_frame as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:engine:shutdown", callback: tramp_shutdown as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:engine:with_world_mut", callback: tramp_with_world_mut as *const (), flags: 0 } }
