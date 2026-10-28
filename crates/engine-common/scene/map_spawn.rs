//! Map-entity pose and keyvalues after prefab components are applied.

use std::collections::HashMap;

use glam::{EulerRot, Quat, Vec3};

use crate::components::{MapKv, TransformComponent};
use crate::world::{EntityId, World};

use super::map_light::apply_point_light_from_mapkv;

/// GoldSrc map entity fields applied on the host after prefab component apply.
#[derive(Clone, Debug, Default)]
pub struct MapSpawnOpts {
    pub origin: Option<Vec3>,
    /// Pitch, yaw, roll in degrees (GoldSrc `angles`).
    pub angles: Option<Vec3>,
    pub kv: HashMap<String, String>,
}

pub fn apply_map_spawn_tail(world: &World, entity: EntityId, opts: &MapSpawnOpts) {
    apply_transform_from_map(world, entity, opts);
    if !opts.kv.is_empty() {
        let kv_id = world.component_init::<MapKv>();
        world.set(entity, kv_id, MapKv::new(opts.kv.clone()));
        post_map_kv_applied(world, entity);
    }
}

/// After [`MapKv`] is set (prefab apply or map spawn tail).
pub fn post_map_kv_applied(world: &World, entity: EntityId) {
    apply_point_light_from_mapkv(world, entity);
    let kv_id = world.component_init::<MapKv>();
    if !world.has(entity, kv_id) {
        return;
    }
    let ptr = world.get_raw(entity, kv_id) as *const MapKv;
    if ptr.is_null() {
        return;
    }
    let kv = unsafe { &*ptr };
    super::studio_present::sync_studio_presentation(world, entity);
}

fn apply_transform_from_map(world: &World, entity: EntityId, opts: &MapSpawnOpts) {
    if opts.origin.is_none() && opts.angles.is_none() {
        return;
    }
    let t_id = world.component_init::<TransformComponent>();
    let mut t = world
        .get::<TransformComponent>(entity, t_id)
        .unwrap_or_default();
    if let Some(origin) = opts.origin {
        t.position = origin;
    }
    if let Some(angles) = opts.angles {
        t.rotation = angles_to_quat(angles);
    }
    world.set(entity, t_id, t);
}

fn angles_to_quat(angles: Vec3) -> Quat {
    let pitch = angles.x.to_radians();
    let yaw = angles.y.to_radians();
    let roll = angles.z.to_radians();
    Quat::from_euler(EulerRot::ZYX, yaw, pitch, roll)
}
