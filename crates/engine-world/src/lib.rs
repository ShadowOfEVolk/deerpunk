//! ECS world. `flecs_ecs_sys` is a private dependency of this crate.

pub mod ecs_validate;
mod world;

pub mod component_inspect;
pub mod layout_meta;
pub mod prefab;
pub mod prefab_layout_apply;

pub use world::*;
pub use ecs_validate::{assert_component_id, assert_entity_alive, assert_entity_id, assert_set_payload};
