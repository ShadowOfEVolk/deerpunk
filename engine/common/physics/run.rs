use glam::{Quat, Vec3};
use hecs::{Entity, World};
use rapier3d::math::Pose3;

use super::{PhysicsWorld, RigidBody};
use crate::transform::Transform;

/// Фаза 1: Для active-тел — телефонируем позицию из Transform + применяем velocity.
/// Для не-active тел — пропускаем (пусть rapier сам интегрирует).
pub fn physics_sync_in(physics: &mut PhysicsWorld, world: &World) {
    let entries: Vec<(Transform, u64)> = world
        .query::<(&Transform, &RigidBody)>()
        .iter()
        .map(|(tf, rb)| (tf.clone(), rb.handle))
        .collect();

    for (transform, handle) in entries {
        let handle = RigidBody::decode(handle);
        if let Some(rb) = physics.inner.bodies.get_mut(handle) {
            rb.set_position(
                Pose3::from_parts(transform.position, transform.rotation),
                true,
            );
        }
    }
}

/// Фаза 2: Шаг симуляции физического движка.
pub fn physics_step(physics: &mut PhysicsWorld, dt: f32) -> super::world::RapierProfile {
    physics.step(dt);
    physics.last_profile()
}

/// Фаза 3: Копируем позиции и ориентацию из rapier-тел обратно в Transform.
pub fn physics_sync_out(physics: &PhysicsWorld, world: &mut World) {
    let updates: Vec<(Entity, Vec3, Quat)> = world
        .query::<(Entity, &RigidBody)>()
        .iter()
        .filter_map(|(entity, rb)| {
            let handle = RigidBody::decode(rb.handle);

            let (position, rotation) = physics
                .inner
                .bodies
                .get(handle)
                .map(|rb| (rb.translation(), rb.rotation().clone()))
                .unwrap();

            Some((entity, position, rotation))
        })
        .collect();

    for (entity, pos, rot) in updates {
        if let Ok(mut t) = world.get::<&mut Transform>(entity) {
            t.position = pos;
            t.rotation = rot;
        }
    }
}
