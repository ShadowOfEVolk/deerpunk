//! Frustum culling — outputs draw indices into a reusable scratch buffer.

use std::collections::HashMap;

use glam::Mat4;

use crate::aabb::Aabb;
use crate::frustum::Frustum;
use crate::snapshot::{MeshDraw, SkinnedMeshDraw};
use crate::octree::PersistentOctree;

/// Below this count, linear frustum tests beat hash bookkeeping.
pub const GRID_MIN_DRAWS: usize = 64;
/// Default broad-phase cell hint (world units; legacy cvar, octree uses object bounds).
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
    mesh_octree: PersistentOctree,
    skinned_octree: PersistentOctree,
    id_to_index: HashMap<u64, u32>,
    keys_scratch: Vec<(u64, Aabb)>,
    cached_cull_serial: u64,
    mesh_octree_serial: u64,
    skinned_octree_serial: u64,
    last_view_proj: Mat4,
    /// Last [`RenderScene::build_snapshots`] frustum pass (ms).
    pub last_frustum_ms: f32,
    /// Matrix / clone work in `build_snapshots` excluding frustum (ms).
    pub last_assemble_ms: f32,
}

impl CullScratch {
    pub fn new() -> Self {
        Self {
            visible_mesh: Vec::new(),
            visible_skinned: Vec::new(),
            mesh_octree: PersistentOctree::new(),
            skinned_octree: PersistentOctree::new(),
            id_to_index: HashMap::new(),
            keys_scratch: Vec::new(),
            cached_cull_serial: 0,
            mesh_octree_serial: 0,
            skinned_octree_serial: 0,
            last_view_proj: Mat4::ZERO,
            last_frustum_ms: 0.0,
            last_assemble_ms: 0.0,
        }
    }

    pub fn reset_build_timing(&mut self) {
        self.last_frustum_ms = 0.0;
        self.last_assemble_ms = 0.0;
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

fn draws_geom_fingerprint<M>(
    draws: &[M],
    key_at: impl Fn(usize, &M) -> u64,
    bounds_at: impl Fn(&M) -> Aabb,
) -> u64 {
    let mut h = draws.len() as u64;
    for (i, d) in draws.iter().enumerate() {
        h = h.wrapping_mul(31).wrapping_add(key_at(i, d));
        let b = bounds_at(d);
        if b.is_valid() {
            h ^= b.min.x.to_bits() as u64;
            h ^= b.min.y.to_bits() as u64;
            h ^= b.min.z.to_bits() as u64;
            h ^= b.max.x.to_bits() as u64;
            h ^= b.max.y.to_bits() as u64;
            h ^= b.max.z.to_bits() as u64;
        }
    }
    h
}

fn rebuild_id_to_index<M>(draws: &[M], key_at: impl Fn(usize, &M) -> u64, out: &mut HashMap<u64, u32>) {
    out.clear();
    out.reserve(draws.len());
    for (i, d) in draws.iter().enumerate() {
        out.insert(key_at(i, d), i as u32);
    }
}

fn cull_octree_frustum_draws<M>(
    draws: &[M],
    frustum: &Frustum,
    octree: &mut PersistentOctree,
    id_to_index: &HashMap<u64, u32>,
    out: &mut Vec<u32>,
) {
    out.clear();
    for &id in octree.query_frustum_ids(frustum) {
        let Some(&ix) = id_to_index.get(&id) else {
            continue;
        };
        if (ix as usize) < draws.len() {
            out.push(ix);
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

pub fn scene_geometry_cull_serial(geometry: &crate::snapshot::SceneGeometry) -> u64 {
    let mesh = draws_geom_fingerprint(
        &geometry.mesh_draws,
        |i, d| mesh_cull_key(d, i),
        |d| d.world_bounds,
    );
    let skinned = draws_geom_fingerprint(
        &geometry.skinned_mesh_draws,
        |i, d| skinned_cull_key(d, i),
        |d| d.world_bounds,
    );
    mesh.wrapping_mul(31).wrapping_add(skinned)
}

pub fn cull_mesh_indices(
    draws: &[MeshDraw],
    cam: &CullCamera,
    cull_serial: u64,
    _cell_size: f32,
    scratch: &mut CullScratch,
) -> (u32, u32) {
    let frustum = Frustum::from_view_proj(cam.view_proj);
    let len = draws.len();
    if len == 0 {
        scratch.visible_mesh.clear();
        return (0, 0);
    }

    if scratch.last_view_proj == cam.view_proj
        && cull_serial == scratch.cached_cull_serial
        && !scratch.visible_mesh.is_empty()
    {
        return (len as u32, scratch.visible_mesh.len() as u32);
    }
    if scratch.last_view_proj != cam.view_proj {
        scratch.visible_mesh.clear();
        scratch.visible_skinned.clear();
    }
    scratch.last_view_proj = cam.view_proj;
    scratch.cached_cull_serial = cull_serial;

    if len < GRID_MIN_DRAWS {
        cull_draws_linear(len, |i| draws[i].world_bounds, &frustum, &mut scratch.visible_mesh);
    } else {
        if cull_serial != scratch.mesh_octree_serial {
            scratch.mesh_octree_serial = cull_serial;
            scratch.keys_scratch.clear();
            scratch.keys_scratch.reserve(len);
            for (i, d) in draws.iter().enumerate() {
                scratch
                    .keys_scratch
                    .push((mesh_cull_key(d, i), d.world_bounds));
            }
            scratch.mesh_octree.sync_slice(&scratch.keys_scratch);
        }
        rebuild_id_to_index(draws, |i, d| mesh_cull_key(d, i), &mut scratch.id_to_index);
        cull_octree_frustum_draws(
            draws,
            &frustum,
            &mut scratch.mesh_octree,
            &scratch.id_to_index,
            &mut scratch.visible_mesh,
        );
    }
    (len as u32, scratch.visible_mesh.len() as u32)
}

pub fn cull_skinned_indices(
    draws: &[SkinnedMeshDraw],
    cam: &CullCamera,
    cull_serial: u64,
    _cell_size: f32,
    scratch: &mut CullScratch,
) -> (u32, u32) {
    let frustum = Frustum::from_view_proj(cam.view_proj);
    let len = draws.len();
    if len == 0 {
        scratch.visible_skinned.clear();
        return (0, 0);
    }

    if scratch.last_view_proj == cam.view_proj
        && cull_serial == scratch.cached_cull_serial
        && !scratch.visible_skinned.is_empty()
    {
        return (len as u32, scratch.visible_skinned.len() as u32);
    }

    if len < GRID_MIN_DRAWS {
        cull_draws_linear(len, |i| draws[i].world_bounds, &frustum, &mut scratch.visible_skinned);
    } else {
        if cull_serial != scratch.skinned_octree_serial {
            scratch.skinned_octree_serial = cull_serial;
            scratch.keys_scratch.clear();
            scratch.keys_scratch.reserve(len);
            for (i, d) in draws.iter().enumerate() {
                scratch
                    .keys_scratch
                    .push((skinned_cull_key(d, i), d.world_bounds));
            }
            scratch.skinned_octree.sync_slice(&scratch.keys_scratch);
        }
        rebuild_id_to_index(draws, |i, d| skinned_cull_key(d, i), &mut scratch.id_to_index);
        cull_octree_frustum_draws(
            draws,
            &frustum,
            &mut scratch.skinned_octree,
            &scratch.id_to_index,
            &mut scratch.visible_skinned,
        );
    }
    (len as u32, scratch.visible_skinned.len() as u32)
}
