//! Systems that append mesh draws after extract, before camera snapshots.
//!
//! Appended draws go through the same cull and mesh batching as extracted ones.
//! Set [`MeshDraw::batchable`] so a draw joins a [`crate::renderer::MeshBatchKey`] group.

use crate::snapshot::{MeshDraw, RenderScene, SceneGeometry, SkinnedMeshDraw};

/// Mutable draw list for one prepare pass.
pub struct PrepareDrawList<'a> {
    geometry: &'a mut SceneGeometry,
    added: bool,
}

impl<'a> PrepareDrawList<'a> {
    pub fn push_mesh(&mut self, draw: MeshDraw) {
        self.geometry.mesh_draws.push(draw);
        self.added = true;
    }

    pub fn push_skinned(&mut self, draw: SkinnedMeshDraw) {
        self.geometry.skinned_mesh_draws.push(draw);
        self.added = true;
    }
}

/// Run registered prepare systems, then refresh the cull serial if anything was added.
pub fn run_prepare_systems(
    scene: &mut RenderScene,
    systems: &mut [Box<dyn for<'a> FnMut(&mut PrepareDrawList<'a>) + Send>],
) {
    if systems.is_empty() {
        return;
    }
    let added = {
        let mut draws = PrepareDrawList {
            geometry: &mut scene.geometry,
            added: false,
        };
        for system in systems.iter_mut() {
            system(&mut draws);
        }
        draws.added
    };
    if added {
        scene.geometry.refresh_cull_serial();
    }
}
