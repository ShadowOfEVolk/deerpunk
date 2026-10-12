//! Read/write replicated field paths on a World entity.

use glam::{Quat, Vec3};

use crate::components::{TransformComponent, Worldspawn};
use crate::network::field_path::{FieldPathId, FieldValue};
use crate::world::{EntityId, World};

/// Read a hot field from the server entity into a wire value.
pub fn read_field(world: &World, entity: EntityId, path: FieldPathId) -> Option<FieldValue> {
    match path {
        FieldPathId::TransformPosition => {
            let t = get_transform(world, entity)?;
            Some(FieldValue::Vec3(t.position.to_array()))
        }
        FieldPathId::TransformRotation => {
            let t = get_transform(world, entity)?;
            Some(FieldValue::Quat(t.rotation.to_array()))
        }
        FieldPathId::TransformScale => {
            let t = get_transform(world, entity)?;
            Some(FieldValue::Vec3(t.scale.to_array()))
        }
        FieldPathId::WorldspawnMap => {
            let ws_id = world.component_init::<Worldspawn>();
            if !world.has(entity, ws_id) {
                return None;
            }
            let ptr = world.get_raw(entity, ws_id) as *const Worldspawn;
            if ptr.is_null() {
                return None;
            }
            let map = unsafe { (*ptr).map_str() }.to_string();
            Some(FieldValue::Str(map))
        }
        FieldPathId::CameraComponent
        | FieldPathId::CharacterController
        | FieldPathId::SoundEmitter
        | FieldPathId::Material
        | FieldPathId::Worldspawn
        | FieldPathId::Mesh3D => Some(FieldValue::PrefabDefault),
    }
}

/// Apply a field value onto a client (or server) entity.
pub fn apply_field(world: &World, entity: EntityId, path: FieldPathId, value: &FieldValue) {
    match (path, value) {
        (FieldPathId::TransformPosition, FieldValue::Vec3(v)) => {
            with_transform_mut(world, entity, |t| {
                t.position = Vec3::from_array(*v);
            });
        }
        (FieldPathId::TransformRotation, FieldValue::Quat(v)) => {
            with_transform_mut(world, entity, |t| {
                t.rotation = Quat::from_array(*v);
            });
        }
        (FieldPathId::TransformScale, FieldValue::Vec3(v)) => {
            with_transform_mut(world, entity, |t| {
                t.scale = Vec3::from_array(*v);
            });
        }
        (FieldPathId::WorldspawnMap, FieldValue::Str(map)) => {
            // Worldspawn.map override must be applied via scene apply before BSP place.
            // Here we only stamp the component if it already exists.
            let ws_id = world.component_init::<Worldspawn>();
            if world.has(entity, ws_id) {
                let ptr = world.get_raw(entity, ws_id) as *mut Worldspawn;
                if !ptr.is_null() {
                    unsafe {
                        (*ptr).set_map(map);
                    }
                }
            }
        }
        (_, FieldValue::PrefabDefault) => {}
        _ => {
            log::warn!("[net] apply_field mismatch path={path:?} value={value:?}");
        }
    }
}

fn get_transform(world: &World, entity: EntityId) -> Option<TransformComponent> {
    let id = world.component_init::<TransformComponent>();
    if !world.has(entity, id) {
        return None;
    }
    let ptr = world.get_raw(entity, id) as *const TransformComponent;
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { *ptr })
}

fn with_transform_mut(world: &World, entity: EntityId, f: impl FnOnce(&mut TransformComponent)) {
    let id = world.component_init::<TransformComponent>();
    if !world.has(entity, id) {
        let mut t = TransformComponent::default();
        f(&mut t);
        world.set(entity, id, t);
        return;
    }
    let ptr = world.get_raw(entity, id) as *mut TransformComponent;
    if ptr.is_null() {
        return;
    }
    unsafe {
        f(&mut *ptr);
    }
}
