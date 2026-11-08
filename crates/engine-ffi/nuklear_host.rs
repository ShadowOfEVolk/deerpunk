//! Host Nuklear draw trampolines. Valid only during the host UI frame.

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NuklearDrawHost {
    pub begin_overlay: Option<unsafe extern "C" fn(*const u8, f32, f32, f32, f32) -> i32>,
    pub end_window: Option<unsafe extern "C" fn()>,
    pub label: Option<unsafe extern "C" fn(*const u8)>,
    pub separator: Option<unsafe extern "C" fn()>,
}

impl Default for NuklearDrawHost {
    fn default() -> Self {
        Self {
            begin_overlay: None,
            end_window: None,
            label: None,
            separator: None,
        }
    }
}

impl NuklearDrawHost {
    pub fn is_valid(&self) -> bool {
        self.begin_overlay.is_some() && self.end_window.is_some() && self.label.is_some()
    }
}
