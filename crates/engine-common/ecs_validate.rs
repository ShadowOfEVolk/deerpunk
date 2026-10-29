//! Fail-fast checks before Flecs FFI (invalid ids → UB / stack corruption otherwise).

use crate::world::{ComponentId, EntityId, World};

#[inline]
pub fn assert_component_id(comp: ComponentId, op: &str) {
    if comp == 0 {
        panic!(
            "ECS {op}: ComponentId is 0 — component_register failed or wrong id. \
             Zero-sized Rust types cannot be registered (use at least one byte, e.g. `_tag: u8`)"
        );
    }
}

#[inline]
pub fn assert_entity_id(entity: EntityId, op: &str) {
    if entity == 0 {
        panic!("ECS {op}: EntityId is 0");
    }
}

#[inline]
pub fn assert_entity_alive(world: &World, entity: EntityId, op: &str) {
    assert_entity_id(entity, op);
    if !world.is_alive(entity) {
        panic!("ECS {op}: entity {entity:#x} is not alive");
    }
}

#[inline]
pub fn assert_set_payload(size: usize, op: &str) {
    if size == 0 {
        panic!(
            "ECS {op}: set payload size is 0 — cannot write component (empty struct?)"
        );
    }
}
