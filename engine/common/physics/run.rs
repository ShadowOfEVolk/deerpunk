use glam::Quat;
use hecs::{Entity, World};

use super::{PhysicsWorld, RigidBody};
use crate::transform::Transform;

/// Фаза 1: Для active-тел — телефонируем позицию из Transform + применяем velocity.
/// Для не-active тел — пропускаем (пусть rapier сам интегрирует).
pub fn physics_sync_in(physics: &mut PhysicsWorld, world: &World) {
    let entries: Vec<([f32; 3], f32, [f32; 3], f32, u64)> = world
        .query::<(&Transform, &RigidBody)>()
        .iter()
        .filter(|(_, rb)| rb.active)
        .map(|(tf, rb)| {
            (
                tf.position.into(),
                tf.yaw(),
                rb.velocity,
                rb.angular_velocity,
                rb.handle,
            )
        })
        .collect();

    for (pos, yaw, vel, ang_vel, handle) in entries {
        let handle = RigidBody::decode(handle);
        physics.set_body_transform(handle, pos[0], pos[1], pos[2], yaw);
        physics.set_body_velocity(handle, vel[0], vel[1], ang_vel);
    }
}

/// Фаза 2: Шаг симуляции физического движка.
pub fn physics_step(physics: &mut PhysicsWorld, dt: f32) -> super::world::RapierProfile {
    physics.step(dt);
    physics.last_profile()
}

/// Фаза 3: Копируем позиции и ориентацию из rapier-тел обратно в Transform.
pub fn physics_sync_out(physics: &PhysicsWorld, world: &mut World) {
    let updates: Vec<(Entity, [f32; 3], (f32, f32, f32))> = world
        .query::<(Entity, &RigidBody)>()
        .iter()
        .filter_map(|(entity, rb)| {
            let handle = RigidBody::decode(rb.handle);
            let pos = physics.body_position(handle)?;
            let rot = physics.body_rotation(handle).unwrap_or((0.0, 0.0, 0.0));
            Some((entity, pos, rot))
        })
        .collect();

    for (entity, pos, (rx, ry, rz)) in updates {
        if let Ok(mut t) = world.get::<&mut Transform>(entity) {
            t.position.x = pos[0];
            t.position.y = pos[1];
            t.position.z = pos[2];
            t.rotation = Quat::from_euler(glam::EulerRot::XYZ, rx, ry, rz);
        }
    }
}
