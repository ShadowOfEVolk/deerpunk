//! Apply `Mesh3D.mesh` URI strings via host [`ResourceManager`] (gamelib loaders).

use std::path::Path;

use crate::components::Mesh3D;
use crate::resource::mesh::Mesh;
use crate::resource::ResourceManager;
use crate::world::{EntityId, World};

/// Apply `Mesh3D` from a prefab-style mesh URI (`cube`, `file://…`, `file://…#brush:12`).
pub fn apply_mesh3d_uri(
    world: &World,
    gamedir: &Path,
    resources: &mut ResourceManager,
    entity: EntityId,
    uri: &str,
) -> Result<(), String> {
    let kind = uri.trim();
    if kind.is_empty() || kind == "cube" {
        let mesh_id = world.component_init::<Mesh3D>();
        world.set(entity, mesh_id, Mesh3D::cube());
        return Ok(());
    }

    let handle = resources.load::<Mesh>(kind);
    let mesh = handle
        .try_get()
        .ok_or_else(|| format!("Mesh3D load failed: {kind} (asset={:?})", handle.id()))?;
    let mesh_id = world.component_init::<Mesh3D>();
    world.set(entity, mesh_id, Mesh3D::with_mesh(mesh));
    Ok(())
}

// Gamelib registers `brush` loader via `v1:resources:register`; no engine hooks.
