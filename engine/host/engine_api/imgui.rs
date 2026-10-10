use crate::engine_api::engine_api::EngineApiMethod;

// ── ImGui trampolines ───────────────────────────────────────

pub extern "C" fn tramp_imgui_begin_window(_engine: *mut u8, name: *const u8, name_len: usize) -> bool {
    let slice = unsafe { std::slice::from_raw_parts(name, name_len) };
    if let Ok(name_str) = std::str::from_utf8(slice) {
        if let Ok(c_name) = std::ffi::CString::new(name_str) {
            unsafe {
                imgui::sys::igBegin(c_name.as_ptr(), std::ptr::null_mut(), 0)
            }
        } else {
            false
        }
    } else {
        false
    }
}

pub extern "C" fn tramp_imgui_end_window(_engine: *mut u8) {
    unsafe { imgui::sys::igEnd(); }
}

pub extern "C" fn tramp_imgui_text(_engine: *mut u8, text: *const u8, text_len: usize) {
    let slice = unsafe { std::slice::from_raw_parts(text, text_len) };
    if let Ok(text_str) = std::str::from_utf8(slice) {
        if let Ok(c_text) = std::ffi::CString::new(text_str) {
            unsafe { imgui::sys::igTextUnformatted(c_text.as_ptr(), std::ptr::null()); }
        }
    }
}

pub extern "C" fn tramp_imgui_value_f32(_engine: *mut u8, label: *const u8, label_len: usize, value: f32) {
    let slice = unsafe { std::slice::from_raw_parts(label, label_len) };
    if let Ok(label_str) = std::str::from_utf8(slice) {
        if let Ok(c_label) = std::ffi::CString::new(label_str) {
            unsafe { imgui::sys::igTextUnformatted(c_label.as_ptr(), std::ptr::null()); }
            unsafe { imgui::sys::igSameLine(0.0, -1.0); }
            unsafe { imgui::sys::igText(b"%.2f\0".as_ptr() as *const i8, value as f64); }
        }
    }
}

pub extern "C" fn tramp_imgui_value_vec3(
    _engine: *mut u8,
    label: *const u8,
    label_len: usize,
    x: f32,
    y: f32,
    z: f32,
) {
    let slice = unsafe { std::slice::from_raw_parts(label, label_len) };
    if let Ok(label_str) = std::str::from_utf8(slice) {
        if let Ok(c_label) = std::ffi::CString::new(label_str) {
            unsafe {
                imgui::sys::igTextUnformatted(c_label.as_ptr(), std::ptr::null());
                imgui::sys::igSameLine(0.0, -1.0);
                imgui::sys::igText(
                    b"(%.2f, %.2f, %.2f)\0".as_ptr() as *const i8,
                    x as f64, y as f64, z as f64,
                );
            }
        }
    }
}

pub extern "C" fn tramp_imgui_separator(_engine: *mut u8) {
    unsafe { imgui::sys::igSeparator(); }
}

// ── Inventory registrations ──────────────────────────────────

inventory::submit! { EngineApiMethod { name: "v1:imgui:begin_window", callback: tramp_imgui_begin_window as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:imgui:end_window", callback: tramp_imgui_end_window as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:imgui:text", callback: tramp_imgui_text as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:imgui:value_f32", callback: tramp_imgui_value_f32 as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:imgui:value_vec3", callback: tramp_imgui_value_vec3 as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:imgui:separator", callback: tramp_imgui_separator as *const (), flags: 0 } }
