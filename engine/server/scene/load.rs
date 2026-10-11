use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use deerpunk_engine_common::resource::ResourceManager;
use deerpunk_engine_common::world::{EntityId, World};
use json_comments::StripComments;
use serde_json::Value;

use super::apply::apply_component;
use super::format::{PrefabFile, SceneFile};

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

fn scene_path(gamedir: &Path, name: &str) -> PathBuf {
    let name = name.trim_end_matches(".scene");
    gamedir.join("scenes").join(format!("{name}.scene"))
}

pub fn load_prefab_file(gamedir: &Path, name: &str) -> Result<PrefabFile, SceneLoadError> {
    let path = prefab_path(gamedir, name);
    read_jsonc(&path)
}

/// Load `{gamedir}/scenes/{name}.scene`, resolve prefabs, spawn entities into `world`.
pub fn load_scene(
    world: &World,
    gamedir: &Path,
    resources: &mut ResourceManager,
    name: &str,
) -> Result<Vec<EntityId>, SceneLoadError> {
    let path = scene_path(gamedir, name);
    let scene: SceneFile = read_jsonc(&path)?;
    log::info!(
        "[scene] load {} ({} entities)",
        path.display(),
        scene.entities.len()
    );

    let mut prefab_cache: HashMap<String, PrefabFile> = HashMap::new();
    let mut spawned = Vec::with_capacity(scene.entities.len());

    for (i, ent) in scene.entities.iter().enumerate() {
        let mut components: HashMap<String, Value> = HashMap::new();

        if let Some(prefab_name) = ent.prefab.as_deref() {
            if !prefab_cache.contains_key(prefab_name) {
                let prefab = load_prefab_file(gamedir, prefab_name)?;
                prefab_cache.insert(prefab_name.to_string(), prefab);
            }
            if let Some(prefab) = prefab_cache.get(prefab_name) {
                for (k, v) in &prefab.components {
                    components.insert(k.clone(), v.clone());
                }
            }
        }

        for (k, v) in &ent.components {
            components.insert(k.clone(), v.clone());
        }

        if components.is_empty() {
            return Err(format!("scene entity[{i}]: no components (prefab/inline)").into());
        }

        let entity = world.spawn();
        for (comp_name, value) in &components {
            apply_component(world, gamedir, resources, entity, comp_name, value)
                .map_err(|e| format!("entity[{i}] {comp_name}: {e}"))?;
        }
        log::info!("[scene] spawned entity={entity:#x} prefab={:?}", ent.prefab);
        spawned.push(entity);
    }

    Ok(spawned)
}

/// Spawn a single entity from `{gamedir}/prefabs/{name}.prefab`.
pub fn spawn_prefab(
    world: &World,
    gamedir: &Path,
    resources: &mut ResourceManager,
    name: &str,
) -> Result<EntityId, SceneLoadError> {
    let prefab = load_prefab_file(gamedir, name)?;
    if prefab.components.is_empty() {
        return Err(format!("prefab '{name}': no components").into());
    }
    let entity = world.spawn();
    for (comp_name, value) in &prefab.components {
        apply_component(world, gamedir, resources, entity, comp_name, value)
            .map_err(|e| format!("prefab '{name}' {comp_name}: {e}"))?;
    }
    log::info!("[scene] spawned prefab '{name}' entity={entity:#x}");
    Ok(entity)
}
