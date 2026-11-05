//! Raw FFI contract between the deerpunk host and gamelib DLLs.
//!
//! No engine logic, no Flecs, no serde — only `repr(C)` layouts and function pointers.

mod component;
mod init;
mod imgui_host;
mod gamelib_ui;

pub use component::{
    ComponentInspectDumpFn, ComponentRegisterDesc, PrefabComponentApplyFn, no_inspect_dump,
    no_prefab_apply,
};
pub use init::{ClientInitData, GamelibInitData, ResolverFn, ServerInitData, GAMELIB_API_VERSION};
pub use imgui_host::{ImGuiDrawHost, ImGuiVec2, IMGUI_DRAW_HOST_VERSION};
pub use gamelib_ui::{
    GamelibUiBuildArgs, GamelibUiBuildFn, GamelibUiRenderArgs, GamelibUiRenderFn,
    GAMELIB_SURFACE_BGRA8, GAMELIB_SURFACE_BGRA8_SRGB, GAMELIB_SURFACE_RGBA8,
    GAMELIB_SURFACE_RGBA8_SRGB, GAMELIB_UI_API_VERSION,
};

pub type EntityId = u64;
pub type ComponentId = u64;