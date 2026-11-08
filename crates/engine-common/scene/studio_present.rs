//! [`StudioModel`] presentation: mesh URI from [`MapKv`] `model`, optional [`DrawMaterial`].
//!
//! `StudioModel` only holds body/skin (replication). The `.mdl` path lives in `MapKv`.
//! Flecs observers call [`sync_studio_presentation`] on add/set so prefab/stream/map paths
//! do not each duplicate this logic.

use std::ffi::c_void;

use crate::components::{MapKv, StudioModel};
use crate::network::field_apply::MAPKV_STUDIO_MODEL_KEY;
use crate::scene::set_mesh3d_uri;
use crate::world::{ComponentId, EntityId, World};

struct StudioPresentCtx {
    world: *const World,
    studio_id: ComponentId,
}

/// Wire OnAdd `StudioModel` and OnSet `MapKv` → mesh/material URIs on the entity.
pub fn register_studio_present_observers(world: &World) {
    let studio_id = world.component_init::<StudioModel>();
    let ctx = Box::leak(Box::new(StudioPresentCtx { world, studio_id }));
    let ptr = ctx as *mut StudioPresentCtx as *mut c_void;
    if world
        .register_observer(
            "StudioModel",
            &[crate::world::WatchEvent::OnAdd],
            ptr,
            observer_studio_model as crate::world::EcsCallback,
        )
        .is_err()
    {
        log::error!("[studio] OnAdd StudioModel observer failed");
    }
    if world
        .register_observer(
            "MapKv",
            &[crate::world::WatchEvent::OnSet],
            ptr,
            observer_map_kv as crate::world::EcsCallback,
        )
        .is_err()
    {
        log::error!("[studio] OnSet MapKv observer failed");
    }
    log::info!("[studio] observers: StudioModel(OnAdd) + MapKv(OnSet) → Mesh3D.uri");
}

/// Push `MapKv.model` into `Mesh3D.uri` when `StudioModel` is present.
///
/// **Material** is always [`DrawMaterial::uri`] (prefab / `materials.load`), not on `StudioModel`.
pub fn sync_studio_presentation(world: &World, entity: EntityId) {
    let studio_id = world.component_init::<StudioModel>();
    if !world.has(entity, studio_id) {
        return;
    }
    let Some(model) = studio_model_path_from_map_kv(world, entity) else {
        return;
    };
    ensure_studio_model(world, entity);
    set_mesh3d_uri(world, entity, &model);
}

fn studio_model_path_from_map_kv(world: &World, entity: EntityId) -> Option<String> {
    let kv_id = world.component_init::<MapKv>();
    if !world.has(entity, kv_id) {
        return None;
    }
    let ptr = world.get_raw(entity, kv_id) as *const MapKv;
    if ptr.is_null() {
        return None;
    }
    let kv = unsafe { &*ptr };
    let model = kv.get(MAPKV_STUDIO_MODEL_KEY)?.trim();
    if model.is_empty() || model.starts_with('*') {
        return None;
    }
    Some(model.to_string())
}

fn ensure_studio_model(world: &World, entity: EntityId) {
    let id = world.component_init::<StudioModel>();
    if !world.has(entity, id) {
        world.set(entity, id, StudioModel { body: 0, skin: 0 });
    }
}

extern "C-unwind" fn observer_studio_model(
    entities: *const EntityId,
    count: u32,
    _dt: f32,
    ctx: *mut c_void,
) {
    if entities.is_null() || ctx.is_null() || count == 0 {
        return;
    }
    let ctx = unsafe { &*(ctx as *const StudioPresentCtx) };
    let world = unsafe { &*ctx.world };
    for i in 0..count as usize {
        let e = unsafe { *entities.add(i) };
        sync_studio_presentation(world, e);
    }
}

extern "C-unwind" fn observer_map_kv(
    entities: *const EntityId,
    count: u32,
    _dt: f32,
    ctx: *mut c_void,
) {
    if entities.is_null() || ctx.is_null() || count == 0 {
        return;
    }
    let ctx = unsafe { &*(ctx as *const StudioPresentCtx) };
    let world = unsafe { &*ctx.world };
    for i in 0..count as usize {
        let e = unsafe { *entities.add(i) };
        if !world.has(e, ctx.studio_id) {
            continue;
        }
        sync_studio_presentation(world, e);
    }
}
