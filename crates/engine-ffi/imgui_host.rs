//! Host → gamelib ImGui draw trampolines (single imgui instance in the executable).

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ImGuiVec2 {
    pub x: f32,
    pub y: f32,
}

pub const IMGUI_DRAW_HOST_VERSION: u32 = 1;

/// Passed to `client_imgui_frame` instead of a raw `ImGuiContext*`.
#[repr(C)]
pub struct ImGuiDrawHost {
    pub version: u32,
    pub ig_set_next_window_size:
        Option<unsafe extern "C" fn(size: ImGuiVec2, cond: i32)>,
    pub ig_begin:
        Option<unsafe extern "C" fn(title: *const u8, open: *mut bool, flags: i32) -> bool>,
    pub ig_end: Option<unsafe extern "C" fn()>,
    pub ig_separator: Option<unsafe extern "C" fn()>,
    pub ig_text_unformatted:
        Option<unsafe extern "C" fn(text_begin: *const u8, text_end: *const u8)>,
}

impl ImGuiDrawHost {
    pub fn is_valid(&self) -> bool {
        self.version == IMGUI_DRAW_HOST_VERSION
            && self.ig_begin.is_some()
            && self.ig_end.is_some()
            && self.ig_text_unformatted.is_some()
    }
}
