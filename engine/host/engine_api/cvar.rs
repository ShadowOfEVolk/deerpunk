//! Cvar trampolines (`v1:cvar:*`).

use crate::engine_api::engine_api::EngineApiMethod;

fn read_str(ptr: *const u8, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    unsafe {
        let bytes = std::slice::from_raw_parts(ptr, len);
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// `v1:cvar:get_f32(name, out)` → 0=ok.
pub extern "C" fn tramp_cvar_get_f32(
    engine: *mut u8,
    name_ptr: *const u8,
    name_len: usize,
    out: *mut f32,
) -> i32 {
    if out.is_null() {
        return -1;
    }
    let name = read_str(name_ptr, name_len);
    let host = unsafe { &*(engine as *const crate::Host) };
    match host.cvar.get_f32(&name) {
        Some(v) => {
            unsafe { *out = v };
            0
        }
        None => -1,
    }
}

/// `v1:cvar:set(name, value)` — set_or_create. → 0=ok.
pub extern "C" fn tramp_cvar_set(
    engine: *mut u8,
    name_ptr: *const u8,
    name_len: usize,
    value_ptr: *const u8,
    value_len: usize,
) -> i32 {
    let name = read_str(name_ptr, name_len);
    let value = read_str(value_ptr, value_len);
    if name.is_empty() {
        return -1;
    }
    let host = unsafe { &*(engine as *const crate::Host) };
    host.cvar.set_or_create(&name, &value);
    0
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:cvar:get_f32",
        callback: tramp_cvar_get_f32 as *const (),
        flags: 0,
    }
}
inventory::submit! {
    EngineApiMethod {
        name: "v1:cvar:set",
        callback: tramp_cvar_set as *const (),
        flags: 0,
    }
}
