//! Place a loaded BSP (`BspMap` / `BspWorldMesh`) into the ECS world.

use std::path::Path;

use crate::bsp30::BspWorldMesh;
use crate::components::{
    BspWorldDebug, LightstyleTable, Material, Mesh3D, PendingBspAtlas, PendingLightmap,
    TransformComponent, Worldspawn,
};
use crate::resource::ResourceManager;
use crate::world::{EntityId, World};

use super::spawner::spawn_map_entities;

/// Attach world geometry to `entity` and spawn map entities from the BSP lump.
pub fn place_bsp_map(
    world: &World,
    gamedir: &Path,
    resources: &mut ResourceManager,
    entity: EntityId,
    loaded: BspWorldMesh,
    map_name: &str,
    spawn_lump_entities: bool,
) -> Result<(), String> {
    let map = map_name.trim().trim_end_matches(".bsp");
    if map.is_empty() {
        return Err("empty map name".into());
    }

    let atlas_sz = loaded
        .pending
        .image
        .as_ref()
        .map(|i| (i.width, i.height))
        .unwrap_or((0, 0));

    let mesh_id = world.component_init::<Mesh3D>();
    world.set(entity, mesh_id, Mesh3D::with_mesh(loaded.mesh));

    let dbg_id = world.component_init::<BspWorldDebug>();
    world.set(entity, dbg_id, loaded.debug);

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

    let pending_id = world.component_init::<PendingBspAtlas>();
    world.set(entity, pending_id, loaded.pending);

    if let Some(lm) = loaded.lightmap {
        let lm_id = world.component_init::<PendingLightmap>();
        world.set(entity, lm_id, PendingLightmap::new(lm));
    }

    let ls_id = world.component_init::<LightstyleTable>();
    world.set(entity, ls_id, LightstyleTable::hl_defaults());

    log::info!(
        "[bsp] place map={map} entity={entity:#x} albedo_atlas={}x{} has_lm={}",
        atlas_sz.0,
        atlas_sz.1,
        world.has(entity, world.component_init::<PendingLightmap>())
    );

    if spawn_lump_entities {
        let (spawned, skipped) =
            spawn_map_entities(world, gamedir, resources, &loaded.entities, &loaded.submodels);
        log::info!("[bsp] map entities spawned={spawned} skipped={skipped}");
    }
    Ok(())
}
