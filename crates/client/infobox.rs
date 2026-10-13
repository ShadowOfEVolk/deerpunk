//! Client debug infobox (`cl_showfps` / `cl_showpos` / `net_show`) — ImGui overlay.

use deerpunk_engine_common::components::TransformComponent;
use deerpunk_engine_common::physics::CharacterController;
use deerpunk_engine_common::world::World;
use std::time::Instant;

/// Extra lines from host (`net_show`).
#[derive(Clone, Debug, Default)]
pub struct NetHudInfo {
    pub lines: Vec<String>,
}

/// Draw top-left HUD. Call only inside ImGui NewFrame.
///
/// `view_yaw` / `view_pitch` — client view stamp (radians).
pub fn tick(
    show_fps: bool,
    show_pos: i32,
    world: &World,
    local_entity: Option<u64>,
    view_yaw: f32,
    view_pitch: f32,
    net: Option<&NetHudInfo>,
    render: Option<&[String]>,
) {
    let net_lines = net.map(|n| n.lines.as_slice()).unwrap_or(&[]);
    let render_lines = render.unwrap_or(&[]);
    if !show_fps && show_pos <= 0 && net_lines.is_empty() && render_lines.is_empty() {
        return;
    }

    let mut fps_lines: Vec<String> = Vec::new();
    let mut pos_lines: Vec<String> = Vec::new();

    if show_fps {
        fps_lines.push(format!("{:.0} fps", smoothed_fps()));
    }

    if show_pos > 0 {
        let (pos, body_yaw) = local_pose(world, local_entity);
        let to_deg = 180.0 / std::f32::consts::PI;
        let ang_y = body_yaw * to_deg;
        let view_p = view_pitch * to_deg;
        let view_y = view_yaw * to_deg;

        if let Some(p) = pos {
            pos_lines.push(format!("pos:  {:.2} {:.2} {:.2}", p[0], p[1], p[2]));
        } else {
            pos_lines.push("pos:  —".into());
        }
        pos_lines.push(format!("ang:  {:.2} {:.2} {:.2}", 0.0, ang_y, 0.0));
        pos_lines.push(format!("view: {view_p:.2} {view_y:.2} {:.2}", 0.0));

        if show_pos >= 2 {
            if let Some((v, grounded)) = local_move_state(world, local_entity) {
                let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                if show_pos >= 3 {
                    pos_lines.push(format!(
                        "vel:  {:.2} {:.2} {:.2} ({speed:.2})",
                        v[0], v[1], v[2]
                    ));
                } else {
                    pos_lines.push(format!("vel:  {speed:.2}"));
                }
                pos_lines.push(format!("onground: {}", if grounded { 1 } else { 0 }));
            } else {
                pos_lines.push("vel:  —".into());
            }
        }
    }

    if fps_lines.is_empty() && pos_lines.is_empty() && net_lines.is_empty() && render_lines.is_empty() {
        return;
    }

    if !begin_overlay("##infobox", 8.0, 8.0) {
        end_window();
        return;
    }
    for line in &fps_lines {
        text(line);
    }
    if !fps_lines.is_empty() && (!pos_lines.is_empty() || !net_lines.is_empty()) {
        separator();
    }
    for line in &pos_lines {
        text(line);
    }
    if !pos_lines.is_empty() && (!net_lines.is_empty() || !render_lines.is_empty()) {
        separator();
    }
    for line in net_lines {
        text(line);
    }
    if !net_lines.is_empty() && !render_lines.is_empty() {
        separator();
    }
    for line in render_lines {
        text(line);
    }
    end_window();
}

fn local_pose(world: &World, local_entity: Option<u64>) -> (Option<[f32; 3]>, f32) {
    let Some(ent) = local_entity else {
        return (None, 0.0);
    };
    let tid = world.component_init::<TransformComponent>();
    let Some(t) = world.get::<TransformComponent>(ent, tid) else {
        return (None, 0.0);
    };
    let yaw = -2.0 * t.rotation.z.atan2(t.rotation.w);
    (Some(t.position.to_array()), yaw)
}

fn local_move_state(world: &World, local_entity: Option<u64>) -> Option<([f32; 3], bool)> {
    let ent = local_entity?;
    let cc_id = world.component_init::<CharacterController>();
    world
        .get::<CharacterController>(ent, cc_id)
        .map(|cc| (cc.velocity, cc.grounded != 0))
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

fn begin_overlay(name: &str, x: f32, y: f32) -> bool {
    let Ok(c_name) = std::ffi::CString::new(name) else {
        return false;
    };
    unsafe {
        imgui::sys::igSetNextWindowPos(
            imgui::sys::ImVec2 { x, y },
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
        imgui::sys::igBegin(c_name.as_ptr(), &mut open, flags as i32)
    }
}

fn end_window() {
    unsafe { imgui::sys::igEnd() };
}

fn text(s: &str) {
    if let Ok(c) = std::ffi::CString::new(s) {
        unsafe { imgui::sys::igTextUnformatted(c.as_ptr(), std::ptr::null()) };
    }
}

fn separator() {
    unsafe { imgui::sys::igSeparator() };
}
