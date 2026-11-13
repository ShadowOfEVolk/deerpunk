//! Deerpunk gamelib SDK — typed API over methoddb + FFI layouts.
//!
//! Gamelib crates should depend on **`deerpunk-sdk-common`** (crate `deerpunk_engine_sdk`) and
//! **`deerpunk-engine-ffi`** for `repr(C)` init/component registration only.

pub mod component;
pub mod engine;
pub mod gamelib_logger;
pub mod inspect;
pub mod prefab;
pub mod registry;
pub mod runtime;

pub use component::{ComponentInspect, InspectDump, PrefabFromValue};
pub use deerpunk_engine_api as api;
pub use deerpunk_engine_ffi as ffi;
pub use deerpunk_sdk_macro::{game_component, game_system, register_game_components};
pub use registry::{
    register_client_inventory_components, register_inventory_components,
    register_server_inventory_components, GameComponentRegistrar, GameComponentWorlds,
};
pub use schemars::JsonSchema;

pub use deerpunk_engine_api::*;
pub use deerpunk_engine_ffi::{
    no_inspect_dump, no_prefab_apply, ClientInitData, ComponentId, ComponentInspectDumpFn,
    ComponentRegisterDesc, EntityId, GamelibInitData, NuklearDrawHost, PrefabComponentApplyFn,
    ResolverFn, ServerInitData, GAMELIB_API_VERSION,
};
pub use gamelib_logger::install_gamelib_logger;
pub use runtime::{engine, init_engine, install_engine, raw_ctx, try_engine};
