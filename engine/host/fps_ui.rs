//! Top-left FPS counter for `r_showfps`.

use std::time::Instant;

use crate::cvar::CvarSystem;

/// Draw plain FPS text in the top-left when `r_showfps` ≠ 0.
pub fn tick(cvar: &CvarSystem) {
    if !cvar.get_bool_or("r_showfps", false) {
        return;
    }

    let fps = smoothed_fps();
    let label = format!("{fps:.0} fps\0");

    unsafe {
        imgui::sys::igSetNextWindowPos(
            imgui::sys::ImVec2 { x: 8.0, y: 8.0 },
            imgui::sys::ImGuiCond_Always as i32,
            imgui::sys::ImVec2 { x: 0.0, y: 0.0 },
        );
        imgui::sys::igSetNextWindowBgAlpha(0.0);
        let flags = imgui::sys::ImGuiWindowFlags_NoDecoration
            | imgui::sys::ImGuiWindowFlags_NoInputs
            | imgui::sys::ImGuiWindowFlags_NoNav
            | imgui::sys::ImGuiWindowFlags_NoMove
            | imgui::sys::ImGuiWindowFlags_NoSavedSettings
            | imgui::sys::ImGuiWindowFlags_AlwaysAutoResize
            | imgui::sys::ImGuiWindowFlags_NoFocusOnAppearing;
        let mut open = true;
        if imgui::sys::igBegin(
            b"##r_showfps\0".as_ptr() as *const i8,
            &mut open,
            flags as i32,
        ) {
            imgui::sys::igText(label.as_ptr() as *const i8);
        }
        imgui::sys::igEnd();
    }
}

fn smoothed_fps() -> f32 {
    thread_local! {
        static STATE: std::cell::RefCell<(Option<Instant>, f32)> =
            const { std::cell::RefCell::new((None, 0.0)) };
    }
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        let now = Instant::now();
        let fps = if let Some(prev) = s.0 {
            let dt = now.duration_since(prev).as_secs_f32().max(1e-6);
            let instant = 1.0 / dt;
            if s.1 <= 0.0 {
                instant
            } else {
                // Light EMA so the number is readable.
                s.1 * 0.9 + instant * 0.1
            }
        } else {
            0.0
        };
        s.0 = Some(now);
        s.1 = fps;
        fps
    })
}
