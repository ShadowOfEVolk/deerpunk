//! Resolve [`DrawMaterial::uri`] into client [`MaterialStore`] handles.

use deerpunk_engine_common::material::{DrawMaterial, MaterialAsset};
use deerpunk_engine_common::resource::ResourceManager;
use deerpunk_engine_common::world::EntityId;

use crate::client_engine::ClientEngine;

pub fn resolve_draw_material_uri(
    client: &mut ClientEngine,
    resources: &ResourceManager,
    entity: EntityId,
) -> Result<(), String> {
    let id = client.world.component_init::<DrawMaterial>();
    if !client.world.has(entity, id) {
        return Ok(());
    }
    let mut dm = client
        .world
        .get::<DrawMaterial>(entity, id)
        .ok_or_else(|| "DrawMaterial missing".to_string())?;
    if dm.handle.0 != 0 {
        return Ok(());
    }
    let uri = dm.uri_str();
    if uri.is_empty() {
        return Ok(());
    }
    let handle = resources.load::<MaterialAsset>(uri);
    let asset = handle
        .try_get()
        .ok_or_else(|| format!("DrawMaterial load failed: {uri} (asset={:?})", handle.id()))?;
    let mat = client.load_material_from_asset(uri, &asset, resources)?;
    dm.handle = mat;
    client.world.set(entity, id, dm);
    Ok(())
}
