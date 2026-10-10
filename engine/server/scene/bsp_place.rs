//! Place a loaded BSP (`BspMap` / `BspWorldMesh`) into the ECS world.

use std::path::Path;

use deerpunk_engine_common::bsp30::BspWorldMesh;
use deerpunk_engine_common::components::{
    Material, Mesh3D, PendingBspArrays, TransformComponent, Worldspawn,
};
use deerpunk_engine_common::resource::ResourceManager;
use deerpunk_engine_common::world::{EntityId, World};

use super::spawner::spawn_map_entities;

/// Attach world geometry to `entity` and spawn map entities from the BSP lump.
pub fn place_bsp_map(
    world: &World,
    gamedir: &Path,
    resources: &mut ResourceManager,
    entity: EntityId,
    loaded: BspWorldMesh,
    map_name: &str,
) -> Result<(), String> {
    let map = map_name.trim().trim_end_matches(".bsp");
    if map.is_empty() {
        return Err("empty map name".into());
    }

    let n_buckets = loaded.pending.buckets.len();
    let n_parts = loaded.pending.parts.len();

    let mesh_id = world.component_init::<Mesh3D>();
    world.set(entity, mesh_id, Mesh3D::with_mesh(loaded.mesh));

    let ws_id = world.component_init::<Worldspawn>();
    world.set(entity, ws_id, Worldspawn::new(map));

    let t_id = world.component_init::<TransformComponent>();
    if !world.has(entity, t_id) {
        world.set(entity, t_id, TransformComponent::default());
    }

    let m_id = world.component_init::<Material>();
    if !world.has(entity, m_id) {
        let mut mat = Material::default();
        mat.color = [1.0, 1.0, 1.0, 1.0];
        mat.roughness = 0.9;
        mat.ambient = [0.0, 0.0, 0.0, 0.0];
        mat.sun_dir = [0.0, 0.0, 1.0, 0.0];
        world.set(entity, m_id, mat);
    }

    let pending_id = world.component_init::<PendingBspArrays>();
    world.set(entity, pending_id, loaded.pending);

    log::info!(
        "[bsp] place map={map} entity={entity:#x} buckets={n_buckets} parts={n_parts}"
    );

    let (spawned, skipped) = spawn_map_entities(world, gamedir, resources, &loaded.entities);
    log::info!("[bsp] map entities spawned={spawned} skipped={skipped}");
    Ok(())
}
