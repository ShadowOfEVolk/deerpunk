//! Install / replace mesh-owning ECS components.
//!
//! Mesh lifetime is owned by [`MeshRegistry`] and freed by its per-frame
//! [`MeshRegistry::sweep`]; component installers here do **not** call
//! `release`. Replacing a `Mesh3D` simply overwrites the handle — the
//! previous mesh stays in the registry until no entity references it.

use std::ffi::c_void;
use std::mem::size_of;

use deerpunk_engine_common::components::mesh3d::Mesh3D;
use deerpunk_engine_common::components::SkinnedMesh3D;
use deerpunk_engine_common::resource::mesh_handle::MeshHandle;
use deerpunk_engine_common::resource::mesh_registry::MeshRegistry;
use deerpunk_engine_common::world::{ComponentId, EntityId, World};

/// Drop the `Mesh3D` component from `entity` without touching the registry
/// (the per-frame sweep frees the slot once nothing references it).
pub fn release_mesh3d(world: &World, _registry: &mut MeshRegistry, entity: u64, comp: u64) {
    world.remove(entity, comp);
}

/// Drop the `SkinnedMesh3D` component from `entity` without touching the registry.
pub fn release_skinned(world: &World, _registry: &mut MeshRegistry, entity: u64, comp: u64) {
    world.remove(entity, comp);
}

/// Install / replace a `Mesh3D` from raw FFI bytes. The previous handle, if
/// any, is left in the registry for the sweep to reclaim.
pub fn set_mesh3d_raw(
    world: &World,
    _registry: &mut MeshRegistry,
    entity: u64,
    comp: u64,
    data: *const c_void,
    size: usize,
) {
    if size != size_of::<Mesh3D>() || data.is_null() {
        return;
    }
    let new = unsafe { std::ptr::read_unaligned(data as *const Mesh3D) };
    world.set(entity, comp, new);
}

/// Install / replace a `SkinnedMesh3D` from raw FFI bytes.
pub fn set_skinned_raw(
    world: &World,
    _registry: &mut MeshRegistry,
    entity: u64,
    comp: u64,
    data: *const c_void,
    size: usize,
) {
    if size != size_of::<SkinnedMesh3D>() || data.is_null() {
        return;
    }
    let new = unsafe { std::ptr::read_unaligned(data as *const SkinnedMesh3D) };
    world.set(entity, comp, new);
}

/// Attach `handle` to `entity` as a `Mesh3D`, preserving the prior URI.
pub fn attach_mesh3d(
    world: &World,
    _registry: &mut MeshRegistry,
    entity: EntityId,
    comp: ComponentId,
    handle: MeshHandle,
) {
    let uri_owned = world
        .get::<Mesh3D>(entity, comp)
        .map(|m| m.uri_str().to_owned())
        .unwrap_or_default();
    let mut m = Mesh3D::with_handle(handle);
    if !uri_owned.is_empty() {
        m.set_uri(&uri_owned);
    }
    world.set(entity, comp, m);
}
