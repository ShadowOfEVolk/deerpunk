//! Proc-macros for deerpunk gamelib (`#[game_component]`).

mod game_component;
mod register_all;

use proc_macro::TokenStream;

/// Register an ECS component type on the server or client world.
///
/// ```ignore
/// #[game_component]
/// #[repr(C)]
/// struct Usable { _m: u8 }
///
/// #[game_component(client, name = "BspWorldDebug")]
/// struct BspWorldDebug { ... }
/// ```
///
/// - Default: `api.world.register_component(...)`.
/// - `client`: `api.client_world.register_component(...)`.
/// - `name = "..."`: override ECS type name (default: struct ident).
/// - `prefab`: generate CBOR prefab applier + `component_register` (no inspect).
/// - `inspect`: `InspectDump` + JSON Schema from `#[derive(JsonSchema)]` (or `schema = …` override).
#[proc_macro_attribute]
pub fn game_component(attr: TokenStream, item: TokenStream) -> TokenStream {
    match game_component::expand(attr.into(), item.into()) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Register multiple `#[game_component]` types: `register_game_components!(api, A, B, C)`.
#[proc_macro]
pub fn register_game_components(input: TokenStream) -> TokenStream {
    match register_all::expand(input.into()) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}
