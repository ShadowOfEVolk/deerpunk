//! Frustum culling — outputs draw indices into a reusable scratch buffer.

use std::collections::HashMap;

use glam::Mat4;

use crate::aabb::Aabb;
use crate::frustum::{frustum_world_aabb, Frustum};
use crate::snapshot::{MeshDraw, SkinnedMeshDraw};
use crate::spatial_grid::PersistentSpatialIndex;

/// Below this count, linear frustum tests beat hash bookkeeping.
pub const GRID_MIN_DRAWS: usize = 64;
/// Default cell size — fits 16³ voxel chunks (1 block = 1 unit).
pub const DEFAULT_CELL_SIZE: f32 = 32.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct CullStats {
    pub mesh_total: u32,
    pub mesh_visible: u32,
    pub skinned_total: u32,
    pub skinned_visible: u32,
}

/// Per-frame reuse: visible indices + persistent spatial indices (entity-keyed).
#[derive(Default)]
pub struct CullScratch {
    pub visible_mesh: Vec<u32>,
    pub visible_skinned: Vec<u32>,
    mesh_spatial: PersistentSpatialIndex,
    skinned_spatial: PersistentSpatialIndex,
    id_to_index: HashMap<u64, u32>,
    keys_scratch: Vec<(u64, Aabb)>,
}

impl CullScratch {
    pub fn new() -> Self {
        Self {
            visible_mesh: Vec::new(),
            visible_skinned: Vec::new(),
            mesh_spatial: PersistentSpatialIndex::new(DEFAULT_CELL_SIZE),
            skinned_spatial: PersistentSpatialIndex::new(DEFAULT_CELL_SIZE),
            id_to_index: HashMap::new(),
            keys_scratch: Vec::new(),
        }
    }
}

pub fn world_bounds(model: Mat4, mesh: &deerpunk_engine_common::resource::mesh::Mesh) -> Aabb {
    let local = if mesh.has_local_bounds() {
        Aabb::from_min_max(
            glam::Vec3::from_array(mesh.bounds_min),
            glam::Vec3::from_array(mesh.bounds_max),
        )
    } else {
        Aabb::from_positions(&mesh.positions)
    };
    local.transform(model)
}

pub struct CullCamera {
    pub view_proj: Mat4,
    pub view: Mat4,
    pub near: f32,
    pub far: f32,
    pub fov_y: f32,
    pub aspect: f32,
}

fn cull_draws_linear(
    draws_len: usize,
    bounds_at: impl Fn(usize) -> Aabb,
    frustum: &Frustum,
    out: &mut Vec<u32>,
) {
    out.clear();
    for i in 0..draws_len {
        if frustum.intersects_aabb(&bounds_at(i)) {
            out.push(i as u32);
        }
    }
}

fn cull_draws_spatial(
    keys_bounds: &[(u64, Aabb)],
    frustum: &Frustum,
    broad: Aabb,
    cell_size: f32,
    spatial: &mut PersistentSpatialIndex,
    id_to_index: &mut HashMap<u64, u32>,
    out: &mut Vec<u32>,
) {
    out.clear();
    id_to_index.clear();
    for (i, (id, _)) in keys_bounds.iter().enumerate() {
        id_to_index.insert(*id, i as u32);
    }

    spatial.sync_slice(keys_bounds, cell_size);
    let region = broad.inflate(cell_size);
    let candidates = spatial.query_region_ids(&region);
    for &id in candidates {
        let Some(ix) = id_to_index.get(&id) else {
            continue;
        };
        let bounds = keys_bounds[*ix as usize].1;
        if frustum.intersects_aabb(&bounds) {
            out.push(*ix);
        }
    }
}

fn mesh_cull_key(draw: &MeshDraw, index: usize) -> u64 {
    if draw.entity_id != 0 {
        draw.entity_id
    } else {
        // Legacy / tests: stable only within this frame's extract order.
        index as u64 | (1u64 << 63)
    }
}

fn skinned_cull_key(draw: &SkinnedMeshDraw, index: usize) -> u64 {
    if draw.entity_id != 0 {
        draw.entity_id
    } else {
        index as u64 | (1u64 << 63)
    }
}

pub fn cull_mesh_indices(
    draws: &[MeshDraw],
    cam: &CullCamera,
    cell_size: f32,
    scratch: &mut CullScratch,
) -> (u32, u32) {
    let frustum = Frustum::from_view_proj(cam.view_proj);
    let broad = frustum_world_aabb(cam.view, cam.near, cam.far, cam.fov_y, cam.aspect);
    let len = draws.len();
    if len == 0 {
        scratch.visible_mesh.clear();
        return (0, 0);
    }
    if len < GRID_MIN_DRAWS {
        cull_draws_linear(len, |i| draws[i].world_bounds, &frustum, &mut scratch.visible_mesh);
    } else {
        scratch.keys_scratch.clear();
        scratch.keys_scratch.reserve(len);
        for (i, d) in draws.iter().enumerate() {
            scratch
                .keys_scratch
                .push((mesh_cull_key(d, i), d.world_bounds));
        }
        cull_draws_spatial(
            &scratch.keys_scratch,
            &frustum,
            broad,
            cell_size,
            &mut scratch.mesh_spatial,
            &mut scratch.id_to_index,
            &mut scratch.visible_mesh,
        );
    }
    (len as u32, scratch.visible_mesh.len() as u32)
}

pub fn cull_skinned_indices(
    draws: &[SkinnedMeshDraw],
    cam: &CullCamera,
    cell_size: f32,
    scratch: &mut CullScratch,
) -> (u32, u32) {
    let frustum = Frustum::from_view_proj(cam.view_proj);
    let broad = frustum_world_aabb(cam.view, cam.near, cam.far, cam.fov_y, cam.aspect);
    let len = draws.len();
    if len == 0 {
        scratch.visible_skinned.clear();
        return (0, 0);
    }
    if len < GRID_MIN_DRAWS {
        cull_draws_linear(len, |i| draws[i].world_bounds, &frustum, &mut scratch.visible_skinned);
    } else {
        scratch.keys_scratch.clear();
        scratch.keys_scratch.reserve(len);
        for (i, d) in draws.iter().enumerate() {
            scratch
                .keys_scratch
                .push((skinned_cull_key(d, i), d.world_bounds));
        }
        cull_draws_spatial(
            &scratch.keys_scratch,
            &frustum,
            broad,
            cell_size,
            &mut scratch.skinned_spatial,
            &mut scratch.id_to_index,
            &mut scratch.visible_skinned,
        );
    }
    (len as u32, scratch.visible_skinned.len() as u32)
}
