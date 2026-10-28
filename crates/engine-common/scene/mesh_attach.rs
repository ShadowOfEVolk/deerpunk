//! Replace mesh-owning components.
//!
//! Mesh lifetime is owned by [`MeshRegistry`] and reclaimed by its per-frame
//! [`MeshRegistry::sweep`]; the helpers here only mutate the ECS component.

use crate::components::Mesh3D;
use crate::components::SkinnedMesh3D;
use crate::resource::mesh_handle::MeshHandle;
use crate::resource::mesh_registry::MeshRegistry;
use crate::world::{ComponentId, EntityId, World};

/// Remove the `Mesh3D` component from `entity` (registry untouched; sweep
/// reclaims the slot when no entity references it).
pub fn release_mesh3d_at(world: &World, _meshes: &mut MeshRegistry, entity: EntityId, comp: ComponentId) {
    world.remove(entity, comp);
}

/// Remove the `SkinnedMesh3D` component from `entity`.
pub fn release_skinned_at(world: &World, _meshes: &mut MeshRegistry, entity: EntityId, comp: ComponentId) {
    world.remove(entity, comp);
}

/// Attach `handle` to `entity` as a `Mesh3D`, preserving the prior URI.
pub fn attach_mesh3d(
    world: &World,
    _meshes: &mut MeshRegistry,
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

/// Attach a `SkinnedMesh3D` (overwrites any prior component on `entity`).
pub fn attach_skinned(
    world: &World,
    _meshes: &mut MeshRegistry,
    entity: EntityId,
    comp: ComponentId,
    skinned: SkinnedMesh3D,
) {
    world.set(entity, comp, skinned);
}
