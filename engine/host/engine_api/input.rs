use crate::engine_api::engine_api::EngineApiMethod;

// ── Input trampolines ─────────────────────────────────────────

pub extern "C" fn tramp_is_key_down(engine: *mut u8, key: u32) -> bool {
    let host = unsafe { &*(engine as *const crate::Host) };
    match &host.client_engine {
        Some(ce) => ce.input.is_down(key as u16),
        None => false,
    }
}

pub extern "C" fn tramp_mouse_dx(engine: *mut u8) -> f32 {
    let host = unsafe { &*(engine as *const crate::Host) };
    match &host.client_engine {
        Some(ce) => ce.input.mouse_dx(),
        None => 0.0,
    }
}

pub extern "C" fn tramp_mouse_dy(engine: *mut u8) -> f32 {
    let host = unsafe { &*(engine as *const crate::Host) };
    match &host.client_engine {
        Some(ce) => ce.input.mouse_dy(),
        None => 0.0,
    }
}

// ── Inventory registrations ──────────────────────────────────

inventory::submit! { EngineApiMethod { name: "v1:input:is_key_down", callback: tramp_is_key_down as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:input:mouse_dx", callback: tramp_mouse_dx as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:input:mouse_dy", callback: tramp_mouse_dy as *const (), flags: 0 } }
