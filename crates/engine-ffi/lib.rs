//! Raw FFI contract between the deerpunk host and gamelib DLLs.
//!
//! No engine logic, no Flecs, no serde — only `repr(C)` layouts and function pointers.

mod component;
mod init;

pub use component::{
    ComponentInspectDumpFn, ComponentRegisterDesc, PrefabComponentApplyFn, no_inspect_dump,
    no_prefab_apply,
};
pub use init::{ClientInitData, GamelibInitData, ResolverFn, ServerInitData, GAMELIB_API_VERSION};

pub type EntityId = u64;
pub type ComponentId = u64;
