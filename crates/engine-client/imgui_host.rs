//! Fill [`deerpunk_engine_ffi::ImGuiDrawHost`] with pointers into the host executable's imgui-sys.

use deerpunk_engine_ffi::{ImGuiDrawHost, ImGuiVec2, IMGUI_DRAW_HOST_VERSION};

unsafe extern "C" fn host_ig_set_next_window_size(size: ImGuiVec2, cond: i32) {
    imgui::sys::igSetNextWindowSize(
        imgui::sys::ImVec2 { x: size.x, y: size.y },
        cond,
    );
}

unsafe extern "C" fn host_ig_begin(title: *const u8, open: *mut bool, flags: i32) -> bool {
    imgui::sys::igBegin(title as *const i8, open, flags)
}

unsafe extern "C" fn host_ig_end() {
    imgui::sys::igEnd();
}

unsafe extern "C" fn host_ig_separator() {
    imgui::sys::igSeparator();
}

unsafe extern "C" fn host_ig_text_unformatted(text_begin: *const u8, text_end: *const u8) {
    imgui::sys::igTextUnformatted(text_begin as *const i8, text_end as *const i8);
}

pub fn draw_host_table() -> ImGuiDrawHost {
    ImGuiDrawHost {
        version: IMGUI_DRAW_HOST_VERSION,
        ig_set_next_window_size: Some(host_ig_set_next_window_size),
        ig_begin: Some(host_ig_begin),
        ig_end: Some(host_ig_end),
        ig_separator: Some(host_ig_separator),
        ig_text_unformatted: Some(host_ig_text_unformatted),
    }
}
