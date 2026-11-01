//! Demo playback: write a view stamp onto the client camera entity (ECS).

use deerpunk_engine_common::components::{CameraComponent, TransformComponent};
use deerpunk_engine_common::physics::CHAR_EYE_HEIGHT;
use deerpunk_engine_common::world::World;
use glam::{Quat, Vec3};

use crate::client_engine::LocalViewStamp;
use crate::camera_util::ensure_child_world_camera;

pub fn stamp_view_on_entity(
    client_world: &World,
    pawn: u64,
    stamp: LocalViewStamp,
) {
    let eye = if stamp.eye_height > 0.0 {
        stamp.eye_height
    } else {
        CHAR_EYE_HEIGHT
    };
    let cam_ent = ensure_child_world_camera(client_world, pawn, eye);
    let tid = client_world.component_init::<TransformComponent>();
    let cid = client_world.component_init::<CameraComponent>();
    let pitch = stamp.pitch.clamp(-1.5, 1.5);
    let local_rot = Quat::from_rotation_x(pitch) * Quat::from_rotation_y(stamp.roll);
    client_world.set(
        cam_ent,
        tid,
        TransformComponent {
            position: Vec3::new(0.0, 0.0, eye),
            rotation: local_rot,
            scale: Vec3::ONE,
        },
    );
    client_world.set(
        pawn,
        tid,
        TransformComponent {
            position: client_world
                .get::<TransformComponent>(pawn, tid)
                .map(|t| t.position)
                .unwrap_or(Vec3::ZERO),
            rotation: Quat::from_rotation_z(-stamp.yaw),
            scale: client_world
                .get::<TransformComponent>(pawn, tid)
                .map(|t| t.scale)
                .unwrap_or(Vec3::ONE),
        },
    );
    if !client_world.get_raw(cam_ent, cid).is_null() {
        let ptr = client_world.get_raw(cam_ent, cid);
        unsafe {
            let c = &mut *(ptr as *mut CameraComponent);
            c.fov_y = stamp.fov_y.max(1.0_f32.to_radians());
        }
    }
}
