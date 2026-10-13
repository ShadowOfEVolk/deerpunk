//! Client entity wireframe (`r_drawentities`) — HL-style debug overlays.

use deerpunk_engine_client::client_engine::{DebugColor, DebugDrawBuf};
use deerpunk_engine_common::components::{CameraComponent, Mesh3D, NetworkEntity, TransformComponent};
use deerpunk_engine_common::world::World;
use glam::{Quat, Vec3};

use crate::cvar::CvarSystem;

/// `0` off, `1` bbox, `2` eye lines, `3` bbox+lines, `4` mesh AABB (tight).
pub fn tick(cvar: &CvarSystem, client_world: &World, out: &mut DebugDrawBuf) {
    let mode = cvar.get_usize_or("r_drawentities", 0);
    if mode == 0 {
        return;
    }
    let Some((eye, _dir)) = camera_eye(client_world) else {
        return;
    };

    let t_id = client_world.component_init::<TransformComponent>();
    let ne_id = client_world.component_init::<NetworkEntity>();
    let mesh_id = client_world.component_init::<Mesh3D>();

    let mut count = 0u32;
    for e in client_world.inspect_entities() {
        if !client_world.has(e.id, t_id) {
            continue;
        }
        let t = unsafe { &*(client_world.get_raw(e.id, t_id) as *const TransformComponent) };
        let prefab = if client_world.has(e.id, ne_id) {
            let ne = unsafe { &*(client_world.get_raw(e.id, ne_id) as *const NetworkEntity) };
            ne.prefab_str()
        } else {
            ""
        };

        let brush_like = prefab.starts_with("func_") || prefab.starts_with("trigger_");
        let (min, max) = if client_world.has(e.id, mesh_id) {
            let m = unsafe { &*(client_world.get_raw(e.id, mesh_id) as *const Mesh3D) };
            if brush_like {
                mesh_aabb_brush_world(m)
            } else if mode >= 4 {
                mesh_aabb_world(m, t)
            } else {
                default_aabb(t)
            }
        } else if brush_like {
            continue;
        } else {
            default_aabb(t)
        };

        let color = color_for_prefab(prefab);
        let draw_box = if brush_like {
            mode >= 1
        } else {
            mode == 1 || mode == 3 || mode >= 4
        };
        if draw_box {
            out.aabb(min, max, color);
        }
        if mode == 2 || mode == 3 {
            let center = [
                (min[0] + max[0]) * 0.5,
                (min[1] + max[1]) * 0.5,
                (min[2] + max[2]) * 0.5,
            ];
            out.line(eye, center, DebugColor::YELLOW);
        }
        count += 1;
    }

    if mode >= 1 {
        static LAST: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if LAST.swap(count, std::sync::atomic::Ordering::Relaxed) != count {
            log::info!("[r_drawentities] client entities drawn={count} (mode={mode})");
        }
    }
}

fn camera_eye(world: &World) -> Option<([f32; 3], [f32; 3])> {
    let cam_id = world.component_init::<CameraComponent>();
    let t_id = world.component_init::<TransformComponent>();
    for e in world.inspect_entities() {
        if !world.has(e.id, cam_id) || !world.has(e.id, t_id) {
            continue;
        }
        let t = unsafe { &*(world.get_raw(e.id, t_id) as *const TransformComponent) };
        let cam = unsafe { &*(world.get_raw(e.id, cam_id) as *const CameraComponent) };
        let eye = t.position + Vec3::new(0.0, 0.0, cam.eye_height);
        let forward = t.rotation * Vec3::X;
        return Some((eye.to_array(), forward.normalize_or_zero().to_array()));
    }
    None
}

fn default_aabb(t: &TransformComponent) -> ([f32; 3], [f32; 3]) {
    let h = 16.0;
    let c = t.position;
    let p = [c.x, c.y, c.z + h];
    obb_to_aabb(p, [h, h, h], t.rotation)
}

/// BSP `*N` meshes: vertices are already in world space (no entity translation).
fn mesh_aabb_brush_world(mesh: &Mesh3D) -> ([f32; 3], [f32; 3]) {
    let Some(m) = mesh.mesh.as_ref() else {
        return ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    };
    if m.positions.is_empty() {
        return ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    }
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for v in &m.positions {
        for i in 0..3 {
            min[i] = min[i].min(v[i]);
            max[i] = max[i].max(v[i]);
        }
    }
    (min, max)
}

fn mesh_aabb_world(mesh: &Mesh3D, t: &TransformComponent) -> ([f32; 3], [f32; 3]) {
    let Some(m) = mesh.mesh.as_ref() else {
        return default_aabb(t);
    };
    if m.positions.is_empty() {
        return default_aabb(t);
    }
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for v in &m.positions {
        let w = t.rotation * Vec3::from_array(*v) + t.position;
        let a = w.to_array();
        for i in 0..3 {
            min[i] = min[i].min(a[i]);
            max[i] = max[i].max(a[i]);
        }
    }
    (min, max)
}

fn obb_to_aabb(center: [f32; 3], half: [f32; 3], rot: Quat) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for sx in [-1.0f32, 1.0] {
        for sy in [-1.0f32, 1.0] {
            for sz in [-1.0f32, 1.0] {
                let local = Vec3::new(sx * half[0], sy * half[1], sz * half[2]);
                let w = rot * local + Vec3::from_array(center);
                let a = w.to_array();
                for i in 0..3 {
                    min[i] = min[i].min(a[i]);
                    max[i] = max[i].max(a[i]);
                }
            }
        }
    }
    (min, max)
}

fn color_for_prefab(prefab: &str) -> DebugColor {
    if prefab.starts_with("func_") {
        return DebugColor::CYAN;
    }
    if prefab == "player" {
        return DebugColor::GREEN;
    }
    if prefab.is_empty() {
        return DebugColor::RED;
    }
    DebugColor::BLUE
}
