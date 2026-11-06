//! Resolve [`DrawMaterial::uri`] / [`DrawMaterial::asset_id`] into client [`MaterialStore`] handles.

use deerpunk_engine_common::material::{DrawMaterial, MaterialAsset, MaterialHandle};
use deerpunk_engine_common::resource::ResourceManager;
use deerpunk_engine_common::world::EntityId;

use crate::client_engine::ClientEngine;

pub fn resolve_draw_material_uri(
    client: &mut ClientEngine,
    resources: &ResourceManager,
    entity: EntityId,
    force: bool,
) -> Result<(), String> {
    let id = client.world.component_init::<DrawMaterial>();
    if !client.world.has(entity, id) {
        return Ok(());
    }
    let mut dm = client
        .world
        .get::<DrawMaterial>(entity, id)
        .ok_or_else(|| "DrawMaterial missing".to_string())?;
    if dm.handle.0 != 0 && !force {
        return Ok(());
    }

    let asset_id_raw = if dm.asset_id != 0 {
        dm.asset_id
    } else {
        let uri = dm.uri_str();
        if uri.is_empty() {
            return Ok(());
        }
        let handle = resources.load::<MaterialAsset>(uri);
        dm.asset_id = handle.id().0;
        handle.id().0
    };

    let rh = resources
        .handle::<MaterialAsset>(deerpunk_engine_common::resource::AssetId(asset_id_raw))
        .ok_or_else(|| format!("DrawMaterial unknown asset {asset_id_raw}"))?;
    let asset = rh
        .try_get()
        .ok_or_else(|| format!("DrawMaterial asset {asset_id_raw} not loaded"))?;
    let path = resources
        .uri_for(deerpunk_engine_common::resource::AssetId(asset_id_raw))
        .unwrap_or_else(|| format!("asset:{asset_id_raw}"));
    let mat = client.load_material_from_asset(&path, &asset, resources)?;
    dm.handle = mat;
    client.world.set(entity, id, dm);
    Ok(())
}

pub fn invalidate_all_draw_material_handles(client: &mut ClientEngine) {
    let id = client.world.component_init::<DrawMaterial>();
    for ent in client.world.inspect_entities() {
        if !client.world.has(ent.id, id) {
            continue;
        }
        let Some(mut dm) = client.world.get::<DrawMaterial>(ent.id, id) else {
            continue;
        };
        dm.handle = MaterialHandle::NONE;
        client.world.set(ent.id, id, dm);
    }
}

/// Resolve entities whose [`DrawMaterial::handle`] is still unset (after flush or spawn).
pub fn resolve_all_draw_materials(client: &mut ClientEngine, resources: &ResourceManager) {
    resolve_all_draw_materials_force(client, resources, false);
}

/// After scene flush: handles were invalidated — same as `force: false` (all handles are 0).
/// `force: true` only if you must rebuild GPU materials while keeping handles (hot reload).
pub fn resolve_all_draw_materials_force(
    client: &mut ClientEngine,
    resources: &ResourceManager,
    force: bool,
) {
    let id = client.world.component_init::<DrawMaterial>();
    let entities: Vec<EntityId> = client
        .world
        .inspect_entities()
        .into_iter()
        .filter(|e| client.world.has(e.id, id))
        .map(|e| e.id)
        .collect();
    for entity in entities {
        let _ = resolve_draw_material_uri(client, resources, entity, force);
    }
}
