//! Build prefab-style component overrides for [`StreamIn`](super::stream_rep::StreamInRpc).

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::components::{Mesh3D, NetworkEntity, StudioModel, TransformComponent, Worldspawn};
use crate::material::DrawMaterial;
use crate::network::field_apply::read_field;
use crate::network::field_path::{FieldValue, PropertyId};
use crate::property::{PropertyStorage, PropertyRegistry};
use crate::world::{EntityId, World};

/// Server-side: ECS → component JSON merged into client prefab spawn.
pub fn build_stream_in_components(
    world: &World,
    entity: EntityId,
    net: &NetworkEntity,
) -> HashMap<String, Value> {
    let mut components = HashMap::new();
    let hot: std::collections::HashSet<u32> = net.hot_paths().map(|p| p.as_u32()).collect();

    for path in net.cold_paths() {
        match read_field(world, entity, path) {
            Some(FieldValue::PrefabDefault) => {
                merge_whole_component_from_ecs(world, entity, path, &mut components);
            }
            Some(v) => {
                merge_field_into_components(path, &v, &mut components);
            }
            None => {}
        }
    }

    // Always ship presentation components when present on the server entity.
    export_mesh3d(world, entity, &mut components);
    export_draw_material(world, entity, &mut components);

    for path in [
        PropertyId::TRANSFORM_POSITION,
        PropertyId::TRANSFORM_ROTATION,
        PropertyId::TRANSFORM_SCALE,
    ] {
        let id = path.as_u32();
        if hot.contains(&id) {
            continue;
        }
        if let Some(v) = read_field(world, entity, path) {
            merge_field_into_components(path, &v, &mut components);
        }
    }

    if world.has(entity, world.component_init::<StudioModel>()) {
        let path = PropertyId::MAPKV_STUDIO_MODEL;
        if !hot.contains(&path.as_u32())
            && !net
                .cold_paths()
                .any(|p| p.as_u32() == path.as_u32())
        {
            if let Some(FieldValue::Str(uri)) = read_field(world, entity, path) {
                if !uri.is_empty() {
                    merge_field_into_components(path, &FieldValue::Str(uri), &mut components);
                }
            }
        }
    }

    components
}

fn merge_field_into_components(
    path: PropertyId,
    value: &FieldValue,
    out: &mut HashMap<String, Value>,
) {
    match value {
        FieldValue::Str(s) if path == PropertyId::MESH3D_MESH => {
            patch_component(out, "Mesh3D", json!({ "uri": s }));
        }
        FieldValue::Str(s) if path == PropertyId::DRAW_MATERIAL_URI => {
            patch_component(out, "DrawMaterial", json!({ "uri": s }));
        }
        FieldValue::Str(map) if path == PropertyId::WORLDSPAWN_MAP => {
            patch_component(out, "Worldspawn", json!({ "map": map }));
        }
        FieldValue::Vec3(v) if path == PropertyId::TRANSFORM_POSITION => {
            patch_transform(out, "position", v);
        }
        FieldValue::Vec3(v) if path == PropertyId::TRANSFORM_SCALE => {
            patch_transform(out, "scale", v);
        }
        FieldValue::Quat(v) if path == PropertyId::TRANSFORM_ROTATION => {
            let entry = out
                .entry("TransformComponent".into())
                .or_insert_with(|| json!({}));
            if let Some(obj) = entry.as_object_mut() {
                obj.insert(
                    "rotation".into(),
                    Value::Array(v.iter().map(|f| Value::from(*f as f64)).collect()),
                );
            }
        }
        FieldValue::Str(s) if path == PropertyId::MAPKV_STUDIO_MODEL => {
            patch_component(out, "MapKv", json!({ "model": s }));
        }
        FieldValue::U32(v) => {
            let reg = PropertyRegistry::global()
                .lock()
                .expect("net field registry lock poisoned");
            let Some(desc) = reg.get(path) else {
                return;
            };
            let PropertyStorage::Memory { component, field, .. } = &desc.storage else {
                return;
            };
            let entry = out
                .entry(component.clone())
                .or_insert_with(|| json!({}));
            if let Some(obj) = entry.as_object_mut() {
                obj.insert(field.clone(), json!(*v));
            }
        }
        _ => {}
    }
}

fn merge_whole_component_from_ecs(
    world: &World,
    entity: EntityId,
    path: PropertyId,
    out: &mut HashMap<String, Value>,
) {
    let reg = PropertyRegistry::global()
        .lock()
        .expect("net field registry lock poisoned");
    let Some(desc) = reg.get(path) else {
        return;
    };
    let PropertyStorage::WholeComponent { component } = &desc.storage else {
        return;
    };
    match component.as_str() {
        "Mesh3D" => export_mesh3d(world, entity, out),
        "DrawMaterial" => export_draw_material(world, entity, out),
        "Worldspawn" => export_worldspawn(world, entity, out),
        _ => {}
    }
}

fn export_mesh3d(world: &World, entity: EntityId, out: &mut HashMap<String, Value>) {
    let id = world.component_init::<Mesh3D>();
    if !world.has(entity, id) {
        return;
    }
    let Some(m) = world.get::<Mesh3D>(entity, id) else {
        return;
    };
    let uri = m.uri_str();
    if !uri.is_empty() {
        patch_component(out, "Mesh3D", json!({ "uri": uri }));
    }
}

fn export_draw_material(world: &World, entity: EntityId, out: &mut HashMap<String, Value>) {
    let id = world.component_init::<DrawMaterial>();
    if !world.has(entity, id) {
        return;
    }
    let Some(m) = world.get::<DrawMaterial>(entity, id) else {
        return;
    };
    let uri = m.uri_str();
    if !uri.is_empty() {
        patch_component(out, "DrawMaterial", json!({ "uri": uri }));
    }
}

fn export_worldspawn(world: &World, entity: EntityId, out: &mut HashMap<String, Value>) {
    let id = world.component_init::<Worldspawn>();
    if !world.has(entity, id) {
        return;
    }
    let Some(ws) = world.get::<Worldspawn>(entity, id) else {
        return;
    };
    let map = ws.map_str();
    if !map.is_empty() {
        patch_component(out, "Worldspawn", json!({ "map": map }));
    }
}

fn patch_component(out: &mut HashMap<String, Value>, name: &str, value: Value) {
    out.insert(name.to_string(), value);
}

fn patch_transform(out: &mut HashMap<String, Value>, key: &str, v: &[f32; 3]) {
    let entry = out
        .entry("TransformComponent".into())
        .or_insert_with(|| json!({}));
    if let Some(obj) = entry.as_object_mut() {
        obj.insert(
            key.into(),
            Value::Array(vec![
                Value::from(v[0] as f64),
                Value::from(v[1] as f64),
                Value::from(v[2] as f64),
            ]),
        );
    }
}
