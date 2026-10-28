//! [`StudioModel`] presentation: mesh URI from [`MapKv`] `model`, optional [`DrawMaterial`].
//!
//! `StudioModel` only holds body/skin (replication). The `.mdl` path lives in `MapKv`.
//! Flecs observers call [`sync_studio_presentation`] on add/set so prefab/stream/map paths
//! do not each duplicate this logic.

use std::ffi::c_void;
use std::ffi::CString;

use flecs_ecs_sys::{self as sys};

use crate::components::{MapKv, StudioModel};
use crate::network::field_apply::MAPKV_STUDIO_MODEL_KEY;
use crate::scene::set_mesh3d_uri;
use crate::world::{ComponentId, EntityId, World};

struct StudioPresentCtx {
    world: *const World,
    studio_id: ComponentId,
    _expr_studio: CString,
    _expr_map_kv: CString,
}

/// Wire OnAdd `StudioModel` and OnSet `MapKv` → mesh/material URIs on the entity.
pub fn register_studio_present_observers(world: &World) {
    let studio_id = world.component_init::<StudioModel>();
    let expr_studio = CString::new("StudioModel").unwrap();
    let expr_map_kv = CString::new("MapKv").unwrap();
    let ctx = Box::leak(Box::new(StudioPresentCtx {
        world,
        studio_id,
        _expr_studio: expr_studio,
        _expr_map_kv: expr_map_kv,
    }));

    unsafe {
        let mut desc: sys::ecs_observer_desc_t = std::mem::zeroed();
        desc.query.expr = ctx._expr_studio.as_ptr();
        desc.events[0] = sys::EcsOnAdd;
        desc.callback = Some(observer_studio_model);
        desc.ctx = ctx as *mut StudioPresentCtx as *mut c_void;
        if sys::ecs_observer_init(world.as_ptr(), &desc) == 0 {
            log::error!("[studio] OnAdd StudioModel observer failed");
        }

        let mut desc_kv: sys::ecs_observer_desc_t = std::mem::zeroed();
        desc_kv.query.expr = ctx._expr_map_kv.as_ptr();
        desc_kv.events[0] = sys::EcsOnSet;
        desc_kv.callback = Some(observer_map_kv);
        desc_kv.ctx = ctx as *mut StudioPresentCtx as *mut c_void;
        if sys::ecs_observer_init(world.as_ptr(), &desc_kv) == 0 {
            log::error!("[studio] OnSet MapKv observer failed");
        }
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

unsafe extern "C-unwind" fn observer_studio_model(it: *mut sys::ecs_iter_t) {
    unsafe {
        if it.is_null() {
            return;
        }
        let it_ref = &*it;
        if it_ref.ctx.is_null() || it_ref.count <= 0 {
            return;
        }
        let ctx = &*(it_ref.ctx as *const StudioPresentCtx);
        let world = &*ctx.world;
        for i in 0..it_ref.count as usize {
            let e = *it_ref.entities.add(i);
            sync_studio_presentation(world, e);
        }
    }
}

unsafe extern "C-unwind" fn observer_map_kv(it: *mut sys::ecs_iter_t) {
    unsafe {
        if it.is_null() {
            return;
        }
        let it_ref = &*it;
        if it_ref.ctx.is_null() || it_ref.count <= 0 {
            return;
        }
        let ctx = &*(it_ref.ctx as *const StudioPresentCtx);
        let world = &*ctx.world;
        for i in 0..it_ref.count as usize {
            let e = *it_ref.entities.add(i);
            if !world.has(e, ctx.studio_id) {
                continue;
            }
            sync_studio_presentation(world, e);
        }
    }
}
