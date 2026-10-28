use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use crate::components::NetworkEntity;
use crate::network::field_path::{FieldValue, NetFieldId};
use crate::resource::{MeshRegistry, ResourceManager};
use crate::world::{EntityId, World};
use json_comments::StripComments;
use serde_json::Value;

use super::apply::{apply_component, apply_prefab_component_value};
use super::format::PrefabFile;
use super::map_spawn::{apply_map_spawn_tail, MapSpawnOpts};
use super::prefab_manager::PrefabManager;

#[derive(Debug)]
pub struct SceneLoadError {
    pub message: String,
}

impl std::fmt::Display for SceneLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SceneLoadError {}

impl From<String> for SceneLoadError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

fn read_jsonc<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, SceneLoadError> {
    let file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let reader = StripComments::new(BufReader::new(file));
    serde_json::from_reader(reader).map_err(|e| format!("parse {}: {e}", path.display()).into())
}

fn prefab_path(gamedir: &Path, name: &str) -> PathBuf {
    let name = name.trim_end_matches(".prefab");
    gamedir.join("prefabs").join(format!("{name}.prefab"))
}

pub fn load_prefab_file(gamedir: &Path, name: &str) -> Result<PrefabFile, SceneLoadError> {
    let path = prefab_path(gamedir, name);
    read_jsonc(&path)
}

/// Spawn `{gamedir}/prefabs/{name}.prefab` on the server world (host-only entry).
pub fn spawn_prefab_entity(
    world: &World,
    meshes: &mut MeshRegistry,
    gamedir: &Path,
    resources: &ResourceManager,
    name: &str,
    map: Option<&MapSpawnOpts>,
) -> Result<EntityId, SceneLoadError> {
    log::info!("[scene] spawning prefab '{name}'");
    let prefab = load_prefab_file(gamedir, name)?;
    if prefab.components.is_empty() {
        return Err(format!("prefab '{name}': no components").into());
    }
    let entity = world.spawn();
    for (comp_name, value) in &prefab.components {
        apply_prefab_component_value(world, meshes, gamedir, resources, entity, comp_name, value)
            .map_err(|e| format!("prefab '{name}' {comp_name}: {e}"))?;
    }
    stamp_network_prefab_name(world, entity, name);
    if let Some(map) = map {
        apply_map_spawn_tail(world, entity, map);
    }
    log::info!("[scene] spawned prefab '{name}' entity={entity:#x}");
    Ok(entity)
}

/// Spawn a single entity from `{gamedir}/prefabs/{name}.prefab`.
pub fn spawn_prefab(
    world: &World,
    meshes: &mut MeshRegistry,
    gamedir: &Path,
    resources: &ResourceManager,
    name: &str,
) -> Result<EntityId, SceneLoadError> {
    spawn_prefab_filtered(world, meshes, gamedir, resources, name, None)
}

/// Spawn prefab applying only components whose names are in `allow`.
/// `allow == None` → all components. Always stamps `NetworkEntity.prefab` when present.
pub fn spawn_prefab_filtered(
    world: &World,
    meshes: &mut MeshRegistry,
    gamedir: &Path,
    resources: &ResourceManager,
    name: &str,
    allow: Option<&[&str]>,
) -> Result<EntityId, SceneLoadError> {
    spawn_prefab_filtered_with_overrides(world, meshes, gamedir, resources, name, allow, &[])
}

/// Like [`spawn_prefab_filtered`], but applies field overrides before component apply
/// (e.g. `Worldspawn.map` from the server).
pub fn spawn_prefab_filtered_with_overrides(
    world: &World,
    meshes: &mut MeshRegistry,
    gamedir: &Path,
    resources: &ResourceManager,
    name: &str,
    allow: Option<&[&str]>,
    overrides: &[(NetFieldId, FieldValue)],
) -> Result<EntityId, SceneLoadError> {
    spawn_prefab_filtered_with_overrides_cached(
        world, meshes, gamedir, resources, name, allow, overrides, None,
    )
}

/// Spawn with server [`StreamIn`] component overrides (prefab JSON merge).
pub fn spawn_prefab_filtered_with_component_overrides_cached(
    world: &World,
    meshes: &mut MeshRegistry,
    gamedir: &Path,
    resources: &ResourceManager,
    name: &str,
    allow: Option<&[&str]>,
    component_overrides: &HashMap<String, Value>,
    prefab_cache: Option<&PrefabManager>,
) -> Result<EntityId, SceneLoadError> {
    let mut prefab = if let Some(cache) = prefab_cache {
        if let Some(def) = cache.get_cached(name) {
            PrefabFile {
                components: def.components.clone(),
            }
        } else {
            load_prefab_file(gamedir, name)?
        }
    } else {
        load_prefab_file(gamedir, name)?
    };
    if prefab.components.is_empty() {
        return Err(format!("prefab '{name}': no components").into());
    }
    for (k, v) in component_overrides {
        prefab.components.insert(k.clone(), v.clone());
    }
    let entity = world.spawn();
    for (comp_name, value) in &prefab.components {
        if let Some(allow) = allow {
            if !allow.iter().any(|a| *a == comp_name.as_str()) {
                continue;
            }
        }
        apply_prefab_component_value(world, meshes, gamedir, resources, entity, comp_name, value)
            .map_err(|e| format!("prefab '{name}' {comp_name}: {e}"))?;
    }
    stamp_network_prefab_name(world, entity, name);
    resolve_client_mesh_components(world, meshes, gamedir, resources, entity);
    log::info!(
        "[scene] spawned prefab '{name}' entity={entity:#x} stream_overrides={}",
        component_overrides.len()
    );
    Ok(entity)
}

fn resolve_client_mesh_components(
    world: &World,
    meshes: &mut MeshRegistry,
    gamedir: &Path,
    resources: &ResourceManager,
    entity: EntityId,
) {
    let mesh3d = world.component_init::<crate::components::Mesh3D>();
    if world.has(entity, mesh3d) {
        let _ = crate::scene::resolve_mesh3d_uri(world, meshes, gamedir, resources, entity);
    }
}

/// Like [`spawn_prefab_filtered_with_overrides`], but prefers server-pushed defs in `prefab_cache`.
pub fn spawn_prefab_filtered_with_overrides_cached(
    world: &World,
    meshes: &mut MeshRegistry,
    gamedir: &Path,
    resources: &ResourceManager,
    name: &str,
    allow: Option<&[&str]>,
    overrides: &[(NetFieldId, FieldValue)],
    prefab_cache: Option<&PrefabManager>,
) -> Result<EntityId, SceneLoadError> {
    let mut prefab = if let Some(cache) = prefab_cache {
        if let Some(def) = cache.get_cached(name) {
            PrefabFile {
                components: def.components.clone(),
            }
        } else {
            load_prefab_file(gamedir, name)?
        }
    } else {
        load_prefab_file(gamedir, name)?
    };
    if prefab.components.is_empty() {
        return Err(format!("prefab '{name}': no components").into());
    }
    apply_overrides_to_prefab(&mut prefab.components, overrides);
    let entity = world.spawn();
    for (comp_name, value) in &prefab.components {
        if let Some(allow) = allow {
            if !allow.iter().any(|a| *a == comp_name.as_str()) {
                continue;
            }
        }
        apply_prefab_component_value(world, meshes, gamedir, resources, entity, comp_name, value)
            .map_err(|e| format!("prefab '{name}' {comp_name}: {e}"))?;
    }
    stamp_network_prefab_name(world, entity, name);
    resolve_client_mesh_components(world, meshes, gamedir, resources, entity);
    log::info!(
        "[scene] spawned prefab '{name}' entity={entity:#x} filtered={}",
        allow.is_some()
    );
    Ok(entity)
}

fn apply_overrides_to_prefab(
    components: &mut HashMap<String, Value>,
    overrides: &[(NetFieldId, FieldValue)],
) {
    for (path, value) in overrides {
        match (*path, value) {
            (NetFieldId::WORLDSPAWN_MAP, FieldValue::Str(map)) => {
                let entry = components
                    .entry("Worldspawn".to_string())
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                if let Some(obj) = entry.as_object_mut() {
                    obj.insert("map".into(), Value::String(map.clone()));
                }
            }
            (NetFieldId::TRANSFORM_POSITION, FieldValue::Vec3(v)) => {
                patch_transform(components, "position", v);
            }
            (NetFieldId::TRANSFORM_SCALE, FieldValue::Vec3(v)) => {
                patch_transform(components, "scale", v);
            }
            _ => {}
        }
    }
}

fn patch_transform(components: &mut HashMap<String, Value>, key: &str, v: &[f32; 3]) {
    let entry = components
        .entry("TransformComponent".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
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

fn stamp_network_prefab_name(world: &World, entity: EntityId, name: &str) {
    world.set_entity_name(entity, name);
    let id = world.component_init::<NetworkEntity>();
    if !world.has(entity, id) {
        return;
    }
    let ptr = world.get_raw(entity, id) as *mut NetworkEntity;
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).set_prefab(name);
    }
}
