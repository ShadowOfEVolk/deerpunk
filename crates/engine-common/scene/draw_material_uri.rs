//! [`DrawMaterial::uri`] helpers (resolve into `MaterialStore` is client-only).

use crate::material::DrawMaterial;
use crate::world::{EntityId, World};

pub fn draw_material_uri_of(world: &World, entity: EntityId) -> Option<String> {
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

pub fn set_draw_material_uri(world: &World, entity: EntityId, uri: &str) {
    let id = world.component_init::<DrawMaterial>();
    let mut m = world.get::<DrawMaterial>(entity, id).unwrap_or_default();
    m.set_uri(uri);
    world.set(entity, id, m);
}
