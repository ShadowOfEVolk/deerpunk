//! Deerpunk gamelib SDK — typed API over methoddb + FFI layouts.
//!
//! Gamelib crates should depend on **`deerpunk-engine-sdk`** (this crate) and
//! **`deerpunk-engine-ffi`** for `repr(C)` init/component registration only.

pub mod component;
pub mod engine;
pub mod inspect;
pub mod prefab;
pub mod registry;

pub use component::{ComponentInspect, InspectDump, PrefabFromValue};
pub use schemars::JsonSchema;
pub use registry::{register_inventory_components, GameComponentRegistrar};
pub use deerpunk_engine_api as api;
pub use deerpunk_engine_ffi as ffi;
pub use deerpunk_sdk_macro::{game_component, register_game_components};

pub use deerpunk_engine_api::*;
pub use deerpunk_engine_ffi::{
    ClientInitData, ComponentId, ComponentInspectDumpFn, ComponentRegisterDesc, EntityId,
    GAMELIB_API_VERSION, GamelibInitData, PrefabComponentApplyFn, ResolverFn, ServerInitData,
    no_inspect_dump, no_prefab_apply,
};
