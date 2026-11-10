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
use deerpunk_engine_common::resource::mesh_registry::MeshRegistry;
use deerpunk_engine_common::world::World;

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
