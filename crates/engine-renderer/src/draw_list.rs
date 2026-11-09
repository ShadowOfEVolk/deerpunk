//! Zero-copy views into scene mesh draws (indices into [`SceneGeometry`] vs owned snapshot vecs).

use crate::snapshot::{MeshDraw, RenderSnapshot, SceneGeometry, SkinnedMeshDraw};

/// Resolved static mesh draws for one camera encode pass.
pub struct MeshDrawListView<'a> {
    geo: Option<&'a SceneGeometry>,
    owned: &'a [MeshDraw],
    indices: &'a [u32],
}

impl<'a> MeshDrawListView<'a> {
    pub fn new(snap: &'a RenderSnapshot, geo: Option<&'a SceneGeometry>) -> Self {
        // Geometry indices are authoritative whenever the encode side has geometry
        // (frustum-culled and non-culled pack both fill them). Owned vecs remain
        // the fallback for snapshots encoded without scene geometry.
        if let Some(geo) = geo {
            if !snap.mesh_geometry_indices.is_empty() {
                return Self {
                    geo: Some(geo),
                    owned: &[],
                    indices: &snap.mesh_geometry_indices,
                };
            }
        }
        Self {
            geo: None,
            owned: &snap.mesh_draws,
            indices: &[],
        }
    }

    pub fn len(&self) -> usize {
        if self.geo.is_some() {
            self.indices.len()
        } else {
            self.owned.len()
        }
    }

    pub fn get(&self, i: usize) -> &MeshDraw {
        if let Some(geo) = self.geo {
            let idx = self.indices[i] as usize;
            return &geo.mesh_draws[idx];
        }
        &self.owned[i]
    }
}

pub struct SkinnedDrawListView<'a> {
    geo: Option<&'a SceneGeometry>,
    owned: &'a [SkinnedMeshDraw],
    indices: &'a [u32],
}

impl<'a> SkinnedDrawListView<'a> {
    pub fn new(snap: &'a RenderSnapshot, geo: Option<&'a SceneGeometry>) -> Self {
        if let Some(geo) = geo {
            if !snap.skinned_geometry_indices.is_empty() {
                return Self {
                    geo: Some(geo),
                    owned: &[],
                    indices: &snap.skinned_geometry_indices,
                };
            }
        }
        Self {
            geo: None,
            owned: &snap.skinned_mesh_draws,
            indices: &[],
        }
    }

    pub fn len(&self) -> usize {
        if self.geo.is_some() {
            self.indices.len()
        } else {
            self.owned.len()
        }
    }

    pub fn get(&self, i: usize) -> &SkinnedMeshDraw {
        if let Some(geo) = self.geo {
            let idx = self.indices[i] as usize;
            return &geo.skinned_mesh_draws[idx];
        }
        &self.owned[i]
    }
}

pub fn snapshot_mesh_draw_count(snap: &RenderSnapshot, geo: Option<&SceneGeometry>) -> u32 {
    (MeshDrawListView::new(snap, geo).len() + SkinnedDrawListView::new(snap, geo).len()) as u32
}
