use crate::engine_api::engine_api::EngineApiMethod;

// ── ImGui trampolines (stubs) ───────────────────────────────

pub extern "C" fn tramp_imgui_begin_window(_engine: *mut u8, _name: *const u8, _name_len: usize) -> bool {
    false
}

pub extern "C" fn tramp_imgui_end_window(_engine: *mut u8) {}

pub extern "C" fn tramp_imgui_text(_engine: *mut u8, _text: *const u8, _text_len: usize) {}

pub extern "C" fn tramp_imgui_value_f32(_engine: *mut u8, _label: *const u8, _label_len: usize, _value: f32) {}

pub extern "C" fn tramp_imgui_value_vec3(
    _engine: *mut u8,
    _label: *const u8,
    _label_len: usize,
    _x: f32,
    _y: f32,
    _z: f32,
) {
}

pub extern "C" fn tramp_imgui_separator(_engine: *mut u8) {}

// ── Inventory registrations ──────────────────────────────────

inventory::submit! { EngineApiMethod { name: "v1:imgui:begin_window", callback: tramp_imgui_begin_window as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:imgui:end_window", callback: tramp_imgui_end_window as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:imgui:text", callback: tramp_imgui_text as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:imgui:value_f32", callback: tramp_imgui_value_f32 as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:imgui:value_vec3", callback: tramp_imgui_value_vec3 as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:imgui:separator", callback: tramp_imgui_separator as *const (), flags: 0 } }
