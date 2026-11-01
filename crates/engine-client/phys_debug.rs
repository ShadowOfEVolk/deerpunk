//! Host physics collider wireframe (`r_phys_debug`): camera ray → worldspawn part.

use crate::client_engine::{DebugColor, DebugDrawBuf};
use deerpunk_engine_common::components::{CameraComponent, TransformComponent, Worldspawn};
use deerpunk_engine_common::physics::{
    CharacterController, Collider, PhysicsBackend, ShapeGeom, PARTS_HANDLE_INVALID,
    SHAPE_HANDLE_INVALID,
};
use deerpunk_engine_common::world::World;
use glam::{Quat, Vec3};

use deerpunk_engine_common::cvar::CvarSystem;

const MAX_DIST: f32 = 4096.0;

/// Fill `out` with wireframe of the worldspawn collider part under the crosshair.
pub fn tick(
    cvar: &CvarSystem,
    client_world: &World,
    server_world: &World,
    physics: &dyn PhysicsBackend,
    out: &mut DebugDrawBuf,
) {
    if cvar.get_usize_or("r_phys_debug", 0) == 0 {
        return;
    }
    let Some((origin, dir)) = camera_ray(client_world) else {
        return;
    };

    // Aim marker (always) — proves debug_draw → GPU path is alive.
    let tip = [
        origin[0] + dir[0] * 32.0,
        origin[1] + dir[1] * 32.0,
        origin[2] + dir[2] * 32.0,
    ];
    out.line(origin, tip, DebugColor::YELLOW);

    draw_player_capsules(server_world, out);

    let Some(hit) = pick_world_part(server_world, physics, origin, dir) else {
        return;
    };

    draw_part(out, &hit, DebugColor::CYAN);
    let p = hit.hit_point;
    let s = 4.0;
    out.line([p[0] - s, p[1], p[2]], [p[0] + s, p[1], p[2]], DebugColor::RED);
    out.line([p[0], p[1] - s, p[2]], [p[0], p[1] + s, p[2]], DebugColor::RED);
    out.line([p[0], p[1], p[2] - s], [p[0], p[1], p[2] + s], DebugColor::RED);
}

/// Capsule wireframe from ECS feet + CC dims (bottom sphere at feet).
fn draw_player_capsules(world: &World, out: &mut DebugDrawBuf) {
    let t_id = world.component_init::<TransformComponent>();
    let cc_id = world.component_init::<CharacterController>();
    for e in world.inspect_entities() {
        if !world.has(e.id, t_id) || !world.has(e.id, cc_id) {
            continue;
        }
        let t = unsafe { &*(world.get_raw(e.id, t_id) as *const TransformComponent) };
        let cc = unsafe { &*(world.get_raw(e.id, cc_id) as *const CharacterController) };
        let r = cc.radius.max(0.1);
        let hh = cc.half_height.max(0.1);
        let feet = t.position;
        let center = feet + Vec3::new(0.0, 0.0, hh + r);
        let bottom = feet + Vec3::new(0.0, 0.0, r);
        let top = feet + Vec3::new(0.0, 0.0, 2.0 * hh + r);
        // Vertical axis + end rings + feet marker.
        out.line(bottom.to_array(), top.to_array(), DebugColor::GREEN);
        out.circle(bottom.to_array(), r, [0.0, 0.0, 1.0], 20, DebugColor::GREEN);
        out.circle(top.to_array(), r, [0.0, 0.0, 1.0], 20, DebugColor::GREEN);
        out.circle(center.to_array(), r, [0.0, 0.0, 1.0], 16, DebugColor::YELLOW);
        let s = 3.0;
        out.line(
            [feet.x - s, feet.y, feet.z],
            [feet.x + s, feet.y, feet.z],
            DebugColor::RED,
        );
        out.line(
            [feet.x, feet.y - s, feet.z],
            [feet.x, feet.y + s, feet.z],
            DebugColor::RED,
        );
    }
}

struct PartHit {
    geom: ShapeGeom,
    center: [f32; 3],
    rot_xyzw: [f32; 4],
    /// For convex: world-space vertices.
    world_verts: Vec<[f32; 3]>,
    hit_point: [f32; 3],
    t: f32,
}

use crate::camera_util::camera_world_at_entity;

fn camera_ray(world: &World) -> Option<([f32; 3], [f32; 3])> {
    let cam_id = world.component_init::<CameraComponent>();
    for e in world.inspect_entities() {
        if !world.has(e.id, cam_id) {
            continue;
        }
        let Some((eye, rot)) = camera_world_at_entity(world, e.id) else {
            continue;
        };
        let dir = (rot * Vec3::Y).normalize_or_zero();
        if dir.length_squared() < 1e-8 {
            continue;
        }
        return Some((eye.to_array(), dir.to_array()));
    }
    None
}

fn pick_world_part(
    world: &World,
    physics: &dyn PhysicsBackend,
    origin: [f32; 3],
    dir: [f32; 3],
) -> Option<PartHit> {
    let col_id = world.component_init::<Collider>();
    let t_id = world.component_init::<TransformComponent>();
    let ws_id = world.component_init::<Worldspawn>();

    let o = Vec3::from_array(origin);
    let d = Vec3::from_array(dir);
    let mut best: Option<PartHit> = None;

    for e in world.inspect_entities() {
        if !world.has(e.id, ws_id) || !world.has(e.id, col_id) {
            continue;
        }
        let col = unsafe { &*(world.get_raw(e.id, col_id) as *const Collider) };
        if col.parts == PARTS_HANDLE_INVALID {
            continue;
        }
        let Some(parts) = physics.debug_parts(col.parts) else {
            continue;
        };
        let (ent_pos, ent_rot) = if world.has(e.id, t_id) {
            let t = unsafe { &*(world.get_raw(e.id, t_id) as *const TransformComponent) };
            (t.position, t.rotation)
        } else {
            (Vec3::ZERO, Quat::IDENTITY)
        };

        for part in &parts {
            if part.shape == SHAPE_HANDLE_INVALID {
                continue;
            }
            let Some(geom) = physics.debug_shape(part.shape) else {
                continue;
            };
            let part_pos = Vec3::from_array(part.local_pos);
            let part_rot = Quat::from_xyzw(
                part.local_rot[0],
                part.local_rot[1],
                part.local_rot[2],
                part.local_rot[3],
            );
            let world_pos = ent_pos + ent_rot * part_pos;
            let world_rot = ent_rot * part_rot;
            let rot_xyzw = [world_rot.x, world_rot.y, world_rot.z, world_rot.w];

            let hit = match &geom {
                ShapeGeom::Box { half_extents } => {
                    ray_obb(o, d, world_pos, Vec3::from_array(*half_extents), world_rot)
                        .map(|t| (t, o + d * t, Vec::new()))
                }
                ShapeGeom::Ball { radius } => {
                    ray_sphere(o, d, world_pos, *radius).map(|t| (t, o + d * t, Vec::new()))
                }
                ShapeGeom::Convex { vertices } => {
                    let world_verts: Vec<[f32; 3]> = vertices
                        .iter()
                        .map(|v| {
                            let p = ent_pos + ent_rot * (part_pos + part_rot * Vec3::from_array(*v));
                            [p.x, p.y, p.z]
                        })
                        .collect();
                    let (min, max) = aabb_of(&world_verts)?;
                    ray_aabb(o, d, min, max).map(|t| (t, o + d * t, world_verts))
                }
                ShapeGeom::HalfSpace { .. } => None,
                ShapeGeom::TriMesh { vertices, indices } => {
                    let world_verts: Vec<[f32; 3]> = vertices
                        .iter()
                        .map(|v| {
                            let p = ent_pos + ent_rot * (part_pos + part_rot * Vec3::from_array(*v));
                            [p.x, p.y, p.z]
                        })
                        .collect();
                    let (min, max) = aabb_of(&world_verts)?;
                    ray_aabb(o, d, min, max).map(|t| (t, o + d * t, world_verts))
                }
                ShapeGeom::VoxelVolume { origin, .. } => {
                    // Debug pick: ray vs chunk AABB (full 16³ volume).
                    let min = Vec3::new(origin[0] as f32, origin[1] as f32, origin[2] as f32);
                    let s = deerpunk_engine_common::physics::VOXEL_VOLUME_SIZE as f32;
                    let max = min + Vec3::new(s, s, s);
                    ray_aabb(o, d, min, max).map(|t| (t, o + d * t, Vec::new()))
                }
            };

            let Some((t, point, world_verts)) = hit else {
                continue;
            };
            if t < 0.0 || t > MAX_DIST {
                continue;
            }
            if best.as_ref().map(|b| t < b.t).unwrap_or(true) {
                best = Some(PartHit {
                    geom: geom.clone(),
                    center: world_pos.to_array(),
                    rot_xyzw,
                    world_verts,
                    hit_point: point.to_array(),
                    t,
                });
            }
        }
    }
    best
}

fn draw_part(out: &mut DebugDrawBuf, hit: &PartHit, color: DebugColor) {
    match &hit.geom {
        ShapeGeom::Box { half_extents } => {
            out.obb(hit.center, *half_extents, hit.rot_xyzw, color);
        }
        ShapeGeom::Ball { radius } => {
            out.sphere(hit.center, *radius, color);
        }
        ShapeGeom::Convex { .. } => {
            out.convex_wire(&hit.world_verts, color);
        }
        ShapeGeom::HalfSpace { .. } => {}
        ShapeGeom::TriMesh { .. } => {
            out.convex_wire(&hit.world_verts, color);
        }
        ShapeGeom::VoxelVolume { origin, .. } => {
            // Debug AABB of the whole chunk volume (16³ unit cells).
            let min = Vec3::new(origin[0] as f32, origin[1] as f32, origin[2] as f32);
            let max = min + Vec3::new(
                deerpunk_engine_common::physics::VOXEL_VOLUME_SIZE as f32,
                deerpunk_engine_common::physics::VOXEL_VOLUME_SIZE as f32,
                deerpunk_engine_common::physics::VOXEL_VOLUME_SIZE as f32,
            );
            out.aabb(min.into(), max.into(), color);
        }
    }
}

fn aabb_of(verts: &[[f32; 3]]) -> Option<(Vec3, Vec3)> {
    let first = verts.first()?;
    let mut min = Vec3::from_array(*first);
    let mut max = min;
    for v in verts.iter().skip(1) {
        let p = Vec3::from_array(*v);
        min = min.min(p);
        max = max.max(p);
    }
    Some((min, max))
}

fn ray_aabb(o: Vec3, d: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let mut tmin = 0.0f32;
    let mut tmax = MAX_DIST;
    for i in 0..3 {
        let di = d[i];
        let oi = o[i];
        if di.abs() < 1e-8 {
            if oi < min[i] || oi > max[i] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / di;
        let mut t0 = (min[i] - oi) * inv;
        let mut t1 = (max[i] - oi) * inv;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        tmin = tmin.max(t0);
        tmax = tmax.min(t1);
        if tmin > tmax {
            return None;
        }
    }
    if tmax < 0.0 {
        return None;
    }
    Some(if tmin >= 0.0 { tmin } else { tmax })
}

fn ray_obb(o: Vec3, d: Vec3, center: Vec3, he: Vec3, rot: Quat) -> Option<f32> {
    let inv = rot.conjugate();
    let local_o = inv * (o - center);
    let local_d = inv * d;
    ray_aabb(local_o, local_d, -he, he)
}

fn ray_sphere(o: Vec3, d: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let oc = o - center;
    let a = d.dot(d);
    let b = 2.0 * oc.dot(d);
    let c = oc.dot(oc) - radius * radius;
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    let t0 = (-b - s) / (2.0 * a);
    let t1 = (-b + s) / (2.0 * a);
    if t0 >= 0.0 {
        Some(t0)
    } else if t1 >= 0.0 {
        Some(t1)
    } else {
        None
    }
}
