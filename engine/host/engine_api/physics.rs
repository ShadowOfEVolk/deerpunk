use crate::engine_api::engine_api::EngineApiMethod;
use std::ffi::c_void;

// ── Physics trampolines (stubs — mechanism only) ────────────

pub extern "C" fn tramp_physics_spawn_dynamic_box(
    _engine: *mut u8,
    _entity: u64,
    _x: f32, _y: f32, _z: f32,
    _half_x: f32, _half_y: f32, _half_z: f32,
    _mass: f32,
) {
}

// ── Inventory registrations ──────────────────────────────────

inventory::submit! { EngineApiMethod { name: "v1:physics:spawn_dynamic_box", callback: tramp_physics_spawn_dynamic_box as *const (), flags: 0 } }
