//! JSON ↔ CBOR for gamelib prefab apply (host passes CBOR across FFI).

use deerpunk_engine_api::{ComponentId, EngineApi, EntityId};

pub use deerpunk_engine_common::prefab::{value_from_cbor, value_to_cbor};

/// Deprecated: use [`crate::init_engine`].
#[inline]
pub fn set_prefab_engine_api(api: &'static EngineApi) {
    crate::init_engine(api);
}

/// Write a component during prefab apply when the type is registered on both worlds.
#[inline]
pub fn set_component_copy<T: Copy>(
    api: &EngineApi,
    entity: EntityId,
    component_id: ComponentId,
    value: T,
) {
    if api.client_world.is_alive(entity) {
        api.client_world.set(entity, component_id, value);
    } else {
        api.world.set(entity, component_id, value);
    }
}

/// Read a component for inspect dump when the type is registered on both worlds.
#[inline]
pub fn get_component_copy<T: Copy>(
    api: &EngineApi,
    entity: EntityId,
    component_id: ComponentId,
) -> Option<T> {
    if api.client_world.is_alive(entity) {
        api.client_world.get(entity, component_id)
    } else {
        api.world.get(entity, component_id)
    }
}
