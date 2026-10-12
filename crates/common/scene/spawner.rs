use std::collections::HashMap;
use std::path::Path;

use crate::bsp_entities::{parse_vec3_str, EntityKv};
use super::map_submodel::BspSubmodel;
use crate::components::{MapKv, TransformComponent};
use crate::resource::ResourceManager;
use crate::world::{EntityId, World};
use glam::{EulerRot, Quat, Vec3};

use super::apply::apply_component;
use super::entity_def::EntityDef;
use super::map_light::apply_point_light_from_mapkv;
use super::prefab_manager::{PrefabManager, PrefabResolveError};

/// Single map-entity spawn request (darkdemo-style descriptor, no xash shell).
#[derive(Debug, Clone)]
pub struct EntitySpawnDescriptor {
    pub classname: String,
    pub origin: Option<Vec3>,
    pub angles: Option<Vec3>,
    pub kv: HashMap<String, String>,
}

impl EntitySpawnDescriptor {
    pub fn from_kv(classname: impl Into<String>, kv: &EntityKv) -> Self {
        let origin = kv
            .get("origin")
            .and_then(|s| parse_vec3_str(s))
            .map(|a| Vec3::from_array(a));
        let angles = kv
            .get("angles")
            .and_then(|s| parse_vec3_str(s))
            .map(|a| Vec3::from_array(a));
        Self {
            classname: classname.into(),
            origin,
            angles,
            kv: kv.clone(),
        }
    }
}

#[derive(Debug)]
pub enum SpawnError {
    Prefab(PrefabResolveError),
    EmptyDef,
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Prefab(e) => write!(f, "{e}"),
            Self::EmptyDef => write!(f, "entity def has no components"),
        }
    }
}

impl From<PrefabResolveError> for SpawnError {
    fn from(e: PrefabResolveError) -> Self {
        Self::Prefab(e)
    }
}

/// Prefab cache + spawn entry point for map entities.
#[derive(Default)]
pub struct EntitySpawner {
    prefabs: PrefabManager,
}

impl EntitySpawner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn(
        &mut self,
        world: &World,
        gamedir: &Path,
        resources: &mut ResourceManager,
        desc: EntitySpawnDescriptor,
        submodels: &[BspSubmodel],
    ) -> Result<EntityId, SpawnError> {
        let def = self.prefabs.resolve(gamedir, &desc.classname)?;
        if def.components.is_empty() {
            return Err(SpawnError::EmptyDef);
        }
        let id = spawn_prefab(world, gamedir, resources, &desc, &def, submodels);
        log::info!(
            "[spawn] {} (def={}) → entity={id:#x}",
            desc.classname,
            def.id
        );
        Ok(id)
    }
}

fn spawn_prefab(
    world: &World,
    gamedir: &Path,
    resources: &mut ResourceManager,
    desc: &EntitySpawnDescriptor,
    def: &EntityDef,
    submodels: &[BspSubmodel],
) -> EntityId {
    let entity = world.spawn();

    for (comp_name, value) in &def.components {
        if let Err(e) = apply_component(world, gamedir, resources, entity, comp_name, value) {
            log::warn!(
                "[spawn] {} attach '{}': {e}",
                desc.classname,
                comp_name
            );
        }
    }

    apply_transform_from_map(world, entity, desc, submodels);

    let kv_id = world.component_init::<MapKv>();
    world.set(entity, kv_id, MapKv::new(desc.kv.clone()));
    apply_point_light_from_mapkv(world, entity);

    entity
}

/// Minimal entity without a prefab: transform + MapKv only.
fn spawn_minimal(
    world: &World,
    desc: &EntitySpawnDescriptor,
    submodels: &[BspSubmodel],
) -> EntityId {
    let entity = world.spawn();
    apply_transform_from_map(world, entity, desc, submodels);
    let kv_id = world.component_init::<MapKv>();
    world.set(entity, kv_id, MapKv::new(desc.kv.clone()));
    entity
}

/// GoldSrc `angles` = pitch yaw roll (degrees) → Z-up Quat (yaw around Z).
fn angles_to_quat(angles: Vec3) -> Quat {
    let pitch = angles.x.to_radians();
    let yaw = angles.y.to_radians();
    let roll = angles.z.to_radians();
    Quat::from_euler(EulerRot::ZYX, yaw, pitch, roll)
}

fn parse_star_model_index(model: &str) -> Option<usize> {
    let s = model.trim();
    let num = s.strip_prefix('*')?;
    num.parse().ok()
}

fn origin_from_submodel(kv: &EntityKv, submodels: &[BspSubmodel]) -> Option<Vec3> {
    let model = kv.get("model")?;
    let idx = parse_star_model_index(model)?;
    let sm = *submodels.get(idx)?;
    // Prefer non-zero model origin; else AABB center (typical for HL brush ents).
    if sm.origin.iter().any(|v| *v != 0.0) {
        Some(Vec3::from_array(sm.origin))
    } else {
        Some(Vec3::from_array(sm.center()))
    }
}

fn apply_transform_from_map(
    world: &World,
    entity: EntityId,
    desc: &EntitySpawnDescriptor,
    submodels: &[BspSubmodel],
) {
    let brush_origin = origin_from_submodel(&desc.kv, submodels);
    let origin = desc.origin.or(brush_origin);
    if origin.is_none() && desc.angles.is_none() {
        return;
    }
    let t_id = world.component_init::<TransformComponent>();
    let mut t = world
        .get::<TransformComponent>(entity, t_id)
        .unwrap_or_default();
    if let Some(origin) = origin {
        t.position = origin;
    }
    if let Some(angles) = desc.angles {
        t.rotation = angles_to_quat(angles);
    }
    world.set(entity, t_id, t);
}

