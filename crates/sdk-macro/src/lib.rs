//! Proc-macros for deerpunk gamelib (`#[game_component]`, `#[game_system]`).

mod game_component;
mod game_system;
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

/// Register an ECS system (`api.world` / `api.client_world.system_register`) with an FFI glue.
///
/// ```ignore
/// #[game_system(client, name = "client.chunk_draw", stage = "renderer.prepare", without(ChunkCulled))]
/// fn chunk_draw(
///     api: &EngineApi,                 // optional special parameter
///     chunks: &[ChunkRenderer],        // SLICES mode: one `&[C]` per query term, in order
///     xforms: &[WorldTransformComponent],
///     entities: &[EntityId],           // optional special parameter
///     dt: f32,                         // optional special parameter
/// ) { ... }
///
/// // ROWS mode: exactly one rows parameter instead of component slices.
/// #[game_system(server, name = "sim.gravity")]
/// fn gravity(dt: f32, rows: impl Iterator<Item = (EntityId, &Velocity, &Transform)>) { ... }
///
/// #[game_system(server, name = "sim.gravity")]
/// fn gravity(dt: f32, rows: Rows<'_, (EntityId, &Velocity, &Transform)>) { ... }
/// ```
///
/// - First argument: `client` or `server` (which `WorldApi` the system runs on).
/// - `name = "..."`: system name (required).
/// - `stage = "..."`: move the system onto a named phase via `system_set_stage`
///   (`renderer.prepare` runs after scene extract).
/// - `interval = 0.0`: Flecs `ecs_system_desc_t::interval` seconds (0 = every progress step).
/// - `without(A, B)`: `!A, !B` query filters (components are registered too, names only).
/// - The query and the `fields[]` order come from the component parameters in declaration
///   order (without the `without` types); component names are the last path segments.
/// - Generates `pub fn register(api: &EngineApi)` (idempotent via `Once`) next to the
///   annotated function; component columns are read-only.
#[proc_macro_attribute]
pub fn game_system(attr: TokenStream, item: TokenStream) -> TokenStream {
    match game_system::expand(attr.into(), item.into()) {
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
