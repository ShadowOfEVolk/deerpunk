//! Resolve [`Mesh3D`] from its `uri` field via host [`ResourceManager`].

use std::path::Path;

use crate::components::Mesh3D;
use crate::resource::mesh_handle::MeshHandle;
use crate::resource::mesh::Mesh;
use crate::resource::mesh_registry::MeshRegistry;
use crate::resource::ResourceManager;
use crate::world::EntityId;
use crate::world::World;

/// Read replicated / prefab URI from an entity's `Mesh3D` component.
pub fn mesh3d_uri_of(world: &World, entity: EntityId) -> Option<String> {
    let mesh_id = world.component_init::<Mesh3D>();
    if !world.has(entity, mesh_id) {
        return None;
    }
    let m = world.get::<Mesh3D>(entity, mesh_id)?;
    let s = m.uri_str();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Store URI on `Mesh3D` (does not load geometry).
pub fn set_mesh3d_uri(world: &World, entity: EntityId, uri: &str) {
    let mesh_id = world.component_init::<Mesh3D>();
    let mut m = world.get::<Mesh3D>(entity, mesh_id).unwrap_or_default();
    if m.uri_str() != uri {
        if m.mesh.is_valid() {
            m.mesh = MeshHandle::INVALID;
        }
    }
    m.set_uri(uri);
    world.set(entity, mesh_id, m);
}

/// Load mesh for the entity's current `Mesh3D.uri` and install a client [`MeshHandle`].
pub fn resolve_mesh3d_uri(
    world: &World,
    meshes: &mut MeshRegistry,
    gamedir: &Path,
    resources: &ResourceManager,
    entity: EntityId,
) -> Result<(), String> {
    let mesh_id = world.component_init::<Mesh3D>();
    if !world.has(entity, mesh_id) {
        return Ok(());
    }
    let mut m = world
        .get::<Mesh3D>(entity, mesh_id)
        .ok_or_else(|| "Mesh3D missing".to_string())?;
    let kind = m.uri_str().to_string();
    resolve_mesh3d_uri_str(meshes, resources, &mut m, &kind)?;
    world.set(entity, mesh_id, m);
    Ok(())
}

/// Apply URI then resolve (net `stream.in`, prefab override).
pub fn apply_mesh3d_uri(
    world: &World,
    meshes: &mut MeshRegistry,
    gamedir: &Path,
    resources: &ResourceManager,
    entity: EntityId,
    uri: &str,
) -> Result<(), String> {
    let mesh_id = world.component_init::<Mesh3D>();
    let mut m = world.get::<Mesh3D>(entity, mesh_id).unwrap_or_default();
    m.set_uri(uri);
    resolve_mesh3d_uri_str(meshes, resources, &mut m, uri)?;
    world.set(entity, mesh_id, m);
    Ok(())
}

fn resolve_mesh3d_uri_str(
    meshes: &mut MeshRegistry,
    resources: &ResourceManager,
    m: &mut Mesh3D,
    kind: &str,
) -> Result<(), String> {
    let kind = kind.trim();
    if kind.is_empty() || kind == "cube" {
        m.mesh = meshes.insert(Mesh::cube());
        return Ok(());
    }

    let handle = resources.load::<Mesh>(kind);
    let mesh = handle
        .try_get()
        .ok_or_else(|| format!("Mesh3D load failed: {kind} (asset={:?})", handle.id()))?;
    let verts = mesh.positions.len();
    let indices = mesh.indices.len();
    // Overwrite the handle; the previous slot is reclaimed by the per-frame sweep.
    m.mesh = meshes.insert(mesh);
    log::info!(
        "[mesh] resolved uri='{kind}' -> handle={} verts={} indices={}",
        m.mesh.0,
        verts,
        indices
    );
    Ok(())
}
