//! Server world helpers for MethodDB (`v1:world:*`).
//!
//! The server world lives on the sim thread (`ServerThreadCtx`); these
//! free functions are shared with the facade's bind methods.

use std::ffi::c_void;

use deerpunk_engine_common::world::World;

/// Remaining raw component writes (anything that is not a mesh/MapKv special
/// case — see [`crate::server_ctx::ServerThreadCtx::world_set_component_raw`]).
pub(crate) fn set_component_raw_fallback(
    world: &mut World,
    entity: u64,
    comp: u64,
    data: *const c_void,
    size: usize,
) {
    if size == std::mem::size_of::<deerpunk_engine_common::components::MapKv>() {
        let value =
            unsafe { std::ptr::read(data as *const deerpunk_engine_common::components::MapKv) };
        world.set(entity, comp, value);
        return;
    }
    world.set_raw(entity, comp, data, size);
}
