use crate::engine_api::engine_api::EngineApiMethod;

// ── Input trampolines ─────────────────────────────────────────

pub extern "C" fn tramp_is_key_down(_engine: *mut u8, key: u32) -> bool {
    deerpunk_engine_common::input::is_down(key as u16)
}

pub extern "C" fn tramp_mouse_dx(_engine: *mut u8) -> f32 {
    deerpunk_engine_common::input::mouse_dx()
}

pub extern "C" fn tramp_mouse_dy(_engine: *mut u8) -> f32 {
    deerpunk_engine_common::input::mouse_dy()
}

// ── Inventory registrations ──────────────────────────────────

inventory::submit! { EngineApiMethod { name: "v1:input:is_key_down", callback: tramp_is_key_down as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:input:mouse_dx", callback: tramp_mouse_dx as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:input:mouse_dy", callback: tramp_mouse_dy as *const (), flags: 0 } }
