//! Read/write replicated fields via [`NetFieldRegistry`] + DPK1 layout metadata.

use std::path::Path;

use std::mem::offset_of;

use glam::{Quat, Vec3};

use std::collections::HashSet;

use crate::components::{
    MapKv, Mesh3D, NetworkEntity, StudioModel, TransformComponent, Worldspawn,
};
use crate::layout_meta::WireKind;
use crate::material::DrawMaterial;
use crate::network::field_path::{FieldValue, NetFieldId};
use crate::network::net_field::{NetFieldKind, NetFieldRegistry};
use crate::resource::{MeshRegistry, ResourceManager};
use crate::world::{EntityId, World};

pub const MAPKV_STUDIO_MODEL_KEY: &str = "model";

pub fn read_field(world: &World, entity: EntityId, field: NetFieldId) -> Option<FieldValue> {
    let reg = NetFieldRegistry::global()
        .lock()
        .expect("net field registry lock poisoned");
    let desc = reg.get(field)?;
    match &desc.kind {
        NetFieldKind::WholeComponent { .. } => Some(FieldValue::PrefabDefault),
        NetFieldKind::Mesh3dUri => read_mesh3d_uri(world, entity)
            .map(FieldValue::Str)
            .or(Some(FieldValue::PrefabDefault)),
        NetFieldKind::DrawMaterialUri => read_draw_material_uri(world, entity)
            .map(FieldValue::Str)
            .or(Some(FieldValue::PrefabDefault)),
        NetFieldKind::MapKvStudioModel => read_mapkv_studio_model(world, entity)
            .map(FieldValue::Str)
            .or(Some(FieldValue::PrefabDefault)),
        NetFieldKind::WorldspawnMap => read_worldspawn_map(world, entity),
        NetFieldKind::Memory {
            component_id,
            offset,
            wire,
            component,
            field: fname,
        } => {
            if component == "Worldspawn" && fname == "map" {
                return read_worldspawn_map(world, entity);
            }
            read_memory_field(world, entity, *component_id, *offset, *wire)
        }
    }
}

pub fn cold_field_value(world: &World, entity: EntityId, field: NetFieldId) -> FieldValue {
    read_field(world, entity, field).unwrap_or(FieldValue::PrefabDefault)
}

/// Build `StreamIn` cold/hot lists: prefab `NetworkEntity` paths plus transform baseline on **cold**
/// when not listed in `hot` (map-placed pose for `env_sprite`, doors, etc.).
pub fn stream_in_field_snapshot(
    world: &World,
    entity: EntityId,
    net: &NetworkEntity,
) -> (Vec<(NetFieldId, FieldValue)>, Vec<(NetFieldId, FieldValue)>) {
    let hot_paths: HashSet<u16> = net.hot_paths().map(|p| p.as_u16()).collect();

    let mut cold: Vec<(NetFieldId, FieldValue)> = net
        .cold_paths()
        .map(|p| (p, cold_field_value(world, entity, p)))
        .collect();
    let mut cold_paths: HashSet<u16> = cold.iter().map(|(p, _)| p.as_u16()).collect();

    for path in [
        NetFieldId::TRANSFORM_POSITION,
        NetFieldId::TRANSFORM_ROTATION,
        NetFieldId::TRANSFORM_SCALE,
    ] {
        let id = path.as_u16();
        if hot_paths.contains(&id) || cold_paths.contains(&id) {
            continue;
        }
        if let Some(val) = read_field(world, entity, path) {
            cold.push((path, val));
            cold_paths.insert(id);
        }
    }

    let studio_id = world.component_init::<StudioModel>();
    if world.has(entity, studio_id) {
        let path = NetFieldId::MAPKV_STUDIO_MODEL;
        let id = path.as_u16();
        if !hot_paths.contains(&id) && !cold_paths.contains(&id) {
            if let Some(uri) = read_mapkv_studio_model(world, entity) {
                if !uri.is_empty() {
                    cold.push((path, FieldValue::Str(uri)));
                    cold_paths.insert(id);
                }
            }
        }
    }

    let hot: Vec<(NetFieldId, FieldValue)> = net
        .hot_paths()
        .filter_map(|p| read_field(world, entity, p).map(|v| (p, v)))
        .collect();

    (cold, hot)
}

pub fn apply_field(world: &World, entity: EntityId, field: NetFieldId, value: &FieldValue) {
    let _ = apply_field_ex(world, None, None, None, entity, field, value);
}

pub fn apply_field_ex(
    world: &World,
    meshes: Option<&mut MeshRegistry>,
    gamedir: Option<&Path>,
    resources: Option<&ResourceManager>,
    entity: EntityId,
    field: NetFieldId,
    value: &FieldValue,
) -> bool {
    let reg = NetFieldRegistry::global()
        .lock()
        .expect("net field registry lock poisoned");
    let Some(desc) = reg.get(field) else {
        return false;
    };
    match (&desc.kind, value) {
        (NetFieldKind::WholeComponent { .. }, FieldValue::PrefabDefault) => true,
        (NetFieldKind::Mesh3dUri, FieldValue::PrefabDefault) => true,
        (NetFieldKind::DrawMaterialUri, FieldValue::PrefabDefault) => true,
        (NetFieldKind::MapKvStudioModel, FieldValue::PrefabDefault) => true,
        (NetFieldKind::MapKvStudioModel, FieldValue::Str(uri)) => {
            apply_mapkv_studio_model(world, entity, uri);
            true
        }
        (NetFieldKind::Mesh3dUri, FieldValue::Str(uri)) => {
            if uri.is_empty() {
                return true;
            }
            crate::scene::set_mesh3d_uri(world, entity, uri);
            if let (Some(gd), Some(res), Some(meshes)) = (gamedir, resources, meshes) {
                match crate::scene::resolve_mesh3d_uri(world, meshes, gd, res, entity) {
                    Ok(()) => log::debug!("[net] Mesh3D resolved ent={entity:#x} '{uri}'"),
                    Err(e) => log::warn!("[net] Mesh3D '{uri}' ent={entity:#x}: {e}"),
                }
            }
            true
        }
        (NetFieldKind::DrawMaterialUri, FieldValue::Str(uri)) => {
            if uri.is_empty() {
                return true;
            }
            crate::scene::set_draw_material_uri(world, entity, uri);
            true
        }
        (NetFieldKind::WorldspawnMap, FieldValue::Str(map)) => {
            apply_worldspawn_map(world, entity, map);
            true
        }
        (
            NetFieldKind::Memory {
                component_id,
                offset,
                wire,
                ..
            },
            value,
        ) => apply_memory_field(world, entity, *component_id, *offset, *wire, value),
        (_, FieldValue::PrefabDefault) => true,
        _ => {
            log::warn!("[net] apply_field mismatch field={field:?} value={value:?}");
            false
        }
    }
}

fn read_memory_field(
    world: &World,
    entity: EntityId,
    component_id: crate::world::ComponentId,
    offset: u32,
    wire: WireKind,
) -> Option<FieldValue> {
    if !world.has(entity, component_id) {
        return None;
    }
    let base = world.get_raw(entity, component_id);
    if base.is_null() {
        return None;
    }
    let p = unsafe { base.add(offset as usize) };
    match wire {
        WireKind::U32 => Some(FieldValue::U32(unsafe { *(p as *const u32) })),
        WireKind::U16 => Some(FieldValue::U32(unsafe { *(p as *const u16) as u32 })),
        WireKind::U8 => Some(FieldValue::U32(unsafe { *(p as *const u8) as u32 })),
        WireKind::Vec3 => {
            let x = unsafe { *(p as *const f32) };
            let y = unsafe { *(p.add(4) as *const f32) };
            let z = unsafe { *(p.add(8) as *const f32) };
            Some(FieldValue::Vec3([x, y, z]))
        }
        WireKind::Quat => {
            let mut arr = [0.0f32; 4];
            for i in 0..4 {
                arr[i] = unsafe { *(p.add(i * 4) as *const f32) };
            }
            Some(FieldValue::Quat(arr))
        }
        _ => None,
    }
}

fn apply_memory_field(
    world: &World,
    entity: EntityId,
    component_id: crate::world::ComponentId,
    offset: u32,
    wire: WireKind,
    value: &FieldValue,
) -> bool {
    match wire {
        WireKind::Vec3 => {
            let FieldValue::Vec3(v) = value else {
                return false;
            };
            if component_id == world.component_init::<TransformComponent>() {
                with_transform_mut(world, entity, |t| {
                    if offset == offset_of!(TransformComponent, position) as u32 {
                        t.position = Vec3::from_array(*v);
                    } else if offset == offset_of!(TransformComponent, scale) as u32 {
                        t.scale = Vec3::from_array(*v);
                    }
                });
                return true;
            }
            write_pod_f32x3(world, entity, component_id, offset, v);
            true
        }
        WireKind::Quat => {
            let FieldValue::Quat(v) = value else {
                return false;
            };
            if component_id == world.component_init::<TransformComponent>()
                && offset == offset_of!(TransformComponent, rotation) as u32
            {
                with_transform_mut(world, entity, |t| {
                    t.rotation = Quat::from_array(*v);
                });
                return true;
            }
            false
        }
        WireKind::U32 => {
            let FieldValue::U32(v) = value else {
                return false;
            };
            write_pod_u32(world, entity, component_id, offset, *v)
        }
        WireKind::U16 => {
            let FieldValue::U32(v) = value else {
                return false;
            };
            write_pod_u16(world, entity, component_id, offset, *v as u16)
        }
        WireKind::U8 => {
            let FieldValue::U32(v) = value else {
                return false;
            };
            write_pod_u8(world, entity, component_id, offset, *v as u8)
        }
        _ => false,
    }
}

fn write_pod_u32(
    world: &World,
    entity: EntityId,
    component_id: crate::world::ComponentId,
    offset: u32,
    v: u32,
) -> bool {
    if !world.has(entity, component_id) {
        return false;
    }
    let ptr = world.get_raw(entity, component_id);
    if ptr.is_null() {
        return false;
    }
    unsafe {
        *(ptr.add(offset as usize) as *mut u32) = v;
    }
    true
}

fn write_pod_u16(
    world: &World,
    entity: EntityId,
    component_id: crate::world::ComponentId,
    offset: u32,
    v: u16,
) -> bool {
    if !world.has(entity, component_id) {
        return false;
    }
    let ptr = world.get_raw(entity, component_id);
    if ptr.is_null() {
        return false;
    }
    unsafe {
        *(ptr.add(offset as usize) as *mut u16) = v;
    }
    true
}

fn write_pod_u8(
    world: &World,
    entity: EntityId,
    component_id: crate::world::ComponentId,
    offset: u32,
    v: u8,
) -> bool {
    if !world.has(entity, component_id) {
        return false;
    }
    let ptr = world.get_raw(entity, component_id);
    if ptr.is_null() {
        return false;
    }
    unsafe {
        *(ptr.add(offset as usize) as *mut u8) = v;
    }
    true
}

fn write_pod_f32x3(
    world: &World,
    entity: EntityId,
    component_id: crate::world::ComponentId,
    offset: u32,
    v: &[f32; 3],
) {
    if !world.has(entity, component_id) {
        return;
    }
    let ptr = world.get_raw(entity, component_id);
    if ptr.is_null() {
        return;
    }
    unsafe {
        let p = ptr.add(offset as usize);
        *(p as *mut f32) = v[0];
        *(p.add(4) as *mut f32) = v[1];
        *(p.add(8) as *mut f32) = v[2];
    }
}

fn read_worldspawn_map(world: &World, entity: EntityId) -> Option<FieldValue> {
    let ws_id = world.component_init::<Worldspawn>();
    if !world.has(entity, ws_id) {
        return None;
    }
    let ptr = world.get_raw(entity, ws_id) as *const Worldspawn;
    if ptr.is_null() {
        return None;
    }
    Some(FieldValue::Str(unsafe { (*ptr).map_str() }.to_string()))
}

fn apply_worldspawn_map(world: &World, entity: EntityId, map: &str) {
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

fn read_mapkv_studio_model(world: &World, entity: EntityId) -> Option<String> {
    let kv_id = world.component_init::<MapKv>();
    if !world.has(entity, kv_id) {
        return None;
    }
    let ptr = world.get_raw(entity, kv_id) as *const MapKv;
    if ptr.is_null() {
        return None;
    }
    let kv = unsafe { &*ptr };
    kv.get(MAPKV_STUDIO_MODEL_KEY).map(str::to_string)
}

fn apply_mapkv_studio_model(world: &World, entity: EntityId, model: &str) {
    if model.is_empty() {
        return;
    }
    let kv_id = world.component_init::<MapKv>();
    let mut map = if world.has(entity, kv_id) {
        let ptr = world.get_raw(entity, kv_id) as *const MapKv;
        if ptr.is_null() {
            std::collections::HashMap::new()
        } else {
            unsafe { (*ptr).kv.clone() }
        }
    } else {
        std::collections::HashMap::new()
    };
    map.insert(MAPKV_STUDIO_MODEL_KEY.into(), model.to_string());
    world.set(entity, kv_id, MapKv::new(map));
    crate::scene::sync_studio_presentation(world, entity);
}

fn read_draw_material_uri(world: &World, entity: EntityId) -> Option<String> {
    let id = world.component_init::<DrawMaterial>();
    if !world.has(entity, id) {
        return None;
    }
    let m = world.get::<DrawMaterial>(entity, id)?;
    let s = m.uri_str();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn read_mesh3d_uri(world: &World, entity: EntityId) -> Option<String> {
    let mesh_id = world.component_init::<Mesh3D>();
    if !world.has(entity, mesh_id) {
        return None;
    }
    let m = world.get::<Mesh3D>(entity, mesh_id)?;
    let s = m.uri_str();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
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
