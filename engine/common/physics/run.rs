use glam::{Quat, Vec3};
use rapier3d::math::Pose3;

use super::{PhysicsWorld, RigidBodyHandle};
use crate::flecs_world::FlecsWorld;
use crate::transform::Transform;

/// Фаза 1: Копируем позицию из Transform для всех тел с RigidBodyHandle.
pub fn physics_sync_in(physics: &mut PhysicsWorld, world: &FlecsWorld) {
    let transform_id = world.resolve_component("Transform").expect("Transform not registered");
    let rb_id = world.resolve_component("RigidBodyHandle").expect("RigidBodyHandle not registered");

    let mut entries: Vec<(Transform, u64)> = Vec::new();

    world.query(
        &[crate::flecs_world::QueryTerm::read(transform_id),
          crate::flecs_world::QueryTerm::read(rb_id)],
        |iter| {
            let transforms: Option<&[Transform]> = iter.field(0);
            let rbs: Option<&[RigidBodyHandle]> = iter.field(1);
            if let (Some(transforms), Some(rbs)) = (transforms, rbs) {
                for i in 0..iter.batch_count() as usize {
                    entries.push((transforms[i], rbs[i].0));
                }
            }
        },
    );

    for (transform, handle) in entries {
        let handle = RigidBodyHandle::decode(handle);
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
pub fn physics_sync_out(physics: &PhysicsWorld, world: &FlecsWorld) {
    let transform_id = world.resolve_component("Transform").expect("Transform not registered");
    let rb_id = world.resolve_component("RigidBodyHandle").expect("RigidBodyHandle not registered");

    let mut updates: Vec<(u64, Vec3, Quat)> = Vec::new();

    world.query(
        &[crate::flecs_world::QueryTerm::read(transform_id),
          crate::flecs_world::QueryTerm::read(rb_id)],
        |iter| {
            let entities = iter.entities_ptr();
            if entities.is_null() { return; }
            let entities = unsafe { std::slice::from_raw_parts(entities, iter.batch_count() as usize) };
            let rbs: Option<&[RigidBodyHandle]> = iter.field(1);
            if let Some(rbs) = rbs {
                for i in 0..iter.batch_count() as usize {
                    let entity = entities[i];
                    let handle = RigidBodyHandle::decode(rbs[i].0);
                    if let Some(rb) = physics.inner.bodies.get(handle) {
                        updates.push((entity, rb.translation(), *rb.rotation()));
                    }
                }
            }
        },
    );

    for (entity, pos, rot) in updates {
        if world.has(entity, transform_id) {
            if let Some(mut t) = world.get::<Transform>(entity, transform_id) {
                t.position = pos;
                t.rotation = rot;
                world.set(entity, transform_id, &t);
            }
        }
    }
}
