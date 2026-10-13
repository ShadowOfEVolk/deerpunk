//! Read/write replicated field paths on a World entity.

use std::path::Path;

use glam::{Quat, Vec3};

use crate::components::{MapKv, TransformComponent, Worldspawn};
use crate::network::field_path::{FieldPathId, FieldValue};
use crate::resource::ResourceManager;
use crate::world::{EntityId, World};

/// Map entity key set by gamelib for brush mesh URI replication.
pub const REPLICATE_MESH_URI_KEY: &str = "replicate_mesh_uri";

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
        FieldPathId::Mesh3DMesh | FieldPathId::Mesh3D => read_replicate_mesh_uri(world, entity)
            .map(FieldValue::Str)
            .or(Some(FieldValue::PrefabDefault)),
        FieldPathId::CameraComponent
        | FieldPathId::CharacterController
        | FieldPathId::SoundEmitter
        | FieldPathId::Material
        | FieldPathId::Worldspawn => Some(FieldValue::PrefabDefault),
    }
}

/// Baseline value for a cold path when building `stream.in`.
pub fn cold_field_value(world: &World, entity: EntityId, path: FieldPathId) -> FieldValue {
    match path {
        FieldPathId::WorldspawnMap | FieldPathId::TransformPosition | FieldPathId::TransformScale => {
            read_field(world, entity, path).unwrap_or(FieldValue::PrefabDefault)
        }
        FieldPathId::Mesh3D | FieldPathId::Mesh3DMesh => read_field(world, entity, path)
            .unwrap_or(FieldValue::PrefabDefault),
        _ => FieldValue::PrefabDefault,
    }
}

/// Apply a field value onto a client (or server) entity.
pub fn apply_field(world: &World, entity: EntityId, path: FieldPathId, value: &FieldValue) {
    let _ = apply_field_ex(world, None, None, entity, path, value);
}

/// Like [`apply_field`] but resolves mesh URIs when `gamedir` + `resources` are provided.
pub fn apply_field_ex(
    world: &World,
    gamedir: Option<&Path>,
    resources: Option<&mut ResourceManager>,
    entity: EntityId,
    path: FieldPathId,
    value: &FieldValue,
) -> bool {
    match (path, value) {
        (FieldPathId::TransformPosition, FieldValue::Vec3(v)) => {
            with_transform_mut(world, entity, |t| {
                t.position = Vec3::from_array(*v);
            });
            true
        }
        (FieldPathId::TransformRotation, FieldValue::Quat(v)) => {
            with_transform_mut(world, entity, |t| {
                t.rotation = Quat::from_array(*v);
            });
            true
        }
        (FieldPathId::TransformScale, FieldValue::Vec3(v)) => {
            with_transform_mut(world, entity, |t| {
                t.scale = Vec3::from_array(*v);
            });
            true
        }
        (FieldPathId::Mesh3DMesh, FieldValue::Str(uri))
        | (FieldPathId::Mesh3D, FieldValue::Str(uri)) => {
            if let (Some(gd), Some(res)) = (gamedir, resources) {
                if let Err(e) =
                    crate::scene::apply_mesh3d_uri(world, gd, res, entity, uri)
                {
                    log::warn!("[net] mesh uri '{uri}': {e}");
                }
                true
            } else {
                false
            }
        }
        (FieldPathId::WorldspawnMap, FieldValue::Str(map)) => {
            let ws_id = world.component_init::<Worldspawn>();
            if world.has(entity, ws_id) {
                let ptr = world.get_raw(entity, ws_id) as *mut Worldspawn;
                if !ptr.is_null() {
                    unsafe {
                        (*ptr).set_map(map);
                    }
                }
            }
            true
        }
        (_, FieldValue::PrefabDefault) => true,
        _ => {
            log::warn!("[net] apply_field mismatch path={path:?} value={value:?}");
            false
        }
    }
}

fn read_replicate_mesh_uri(world: &World, entity: EntityId) -> Option<String> {
    let kv_id = world.component_init::<MapKv>();
    if !world.has(entity, kv_id) {
        return None;
    }
    let ptr = world.get_raw(entity, kv_id) as *const MapKv;
    if ptr.is_null() {
        return None;
    }
    let kv = unsafe { &*ptr };
    kv.get(REPLICATE_MESH_URI_KEY).map(str::to_string)
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
