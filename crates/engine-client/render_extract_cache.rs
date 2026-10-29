//! Cached scene extract: static draw data per entity, transform-only updates, optional early frustum.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use deerpunk_engine_common::components::{Mesh3D, TransformComponent};
use deerpunk_engine_common::material::DrawMaterial;
use deerpunk_engine_common::resource::mesh_handle::MeshHandle;
use deerpunk_engine_renderer::Frustum;
use deerpunk_engine_renderer::snapshot::{CameraDefinition, MeshDraw, SkinnedMeshDraw};
use deerpunk_engine_renderer::world_bounds;
use glam::{Mat4, Quat, Vec3};

#[derive(Clone, Copy, PartialEq)]
struct XformKey {
    position: [f32; 3],
    rotation: [f32; 4],
    scale: [f32; 3],
}

impl XformKey {
    fn from_transform(t: &TransformComponent) -> Self {
        Self {
            position: t.position.to_array(),
            rotation: t.rotation.to_array(),
            scale: t.scale.to_array(),
        }
    }

    fn model_mat(self) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            Vec3::from_array(self.scale),
            Quat::from_array(self.rotation),
            Vec3::from_array(self.position),
        )
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct StaticMeshKey {
    mesh_handle: usize,
    material_handle: u32,
    uniform_aux: u32,
}

#[derive(Clone)]
struct CachedMeshEntry {
    static_key: StaticMeshKey,
    xform: XformKey,
    draw: MeshDraw,
}

#[derive(Clone, Copy, PartialEq)]
pub struct StaticSkinnedKey {
    mesh_handle: usize,
    material_handle: u32,
    bone_buffer: u32,
}

#[derive(Clone)]
struct CachedSkinnedEntry {
    static_key: StaticSkinnedKey,
    xform: XformKey,
    draw: SkinnedMeshDraw,
}

pub struct MeshDrawExtractCache {
    mesh: HashMap<u64, CachedMeshEntry>,
    skinned: HashMap<u64, CachedSkinnedEntry>,
    alive: HashSet<u64>,
    mesh_out: Vec<MeshDraw>,
    skinned_out: Vec<SkinnedMeshDraw>,
    pub static_rebuild: u32,
    pub xform_only: u32,
    pub full_hit: u32,
    pub frustum_skip: u32,
}

impl MeshDrawExtractCache {
    pub fn new() -> Self {
        Self {
            mesh: HashMap::new(),
            skinned: HashMap::new(),
            alive: HashSet::new(),
            mesh_out: Vec::new(),
            skinned_out: Vec::new(),
            static_rebuild: 0,
            xform_only: 0,
            full_hit: 0,
            frustum_skip: 0,
        }
    }

    pub fn clear(&mut self) {
        self.mesh.clear();
        self.skinned.clear();
        self.alive.clear();
        self.mesh_out.clear();
        self.skinned_out.clear();
    }

    pub fn begin_frame(&mut self) {
        self.alive.clear();
        self.mesh_out.clear();
        self.skinned_out.clear();
        self.static_rebuild = 0;
        self.xform_only = 0;
        self.full_hit = 0;
        self.frustum_skip = 0;
    }

    pub fn finish_frame(&mut self) {
        self.mesh.retain(|ent, _| self.alive.contains(ent));
        self.skinned.retain(|ent, _| self.alive.contains(ent));
    }

    pub fn frustum_from_camera(cam: &CameraDefinition, aspect: f32) -> Frustum {
        let view = cam.view_matrix();
        let projection = cam.projection_matrix(aspect);
        Frustum::from_view_proj(projection * view)
    }

    fn update_mesh_transform(draw: &mut MeshDraw, xform: XformKey) {
        let mesh = draw.mesh.clone();
        let model = xform.model_mat();
        draw.model_mat = model.to_cols_array_2d();
        draw.world_bounds = world_bounds(model, &mesh);
        draw.batchable = mesh.allocator_id != 0
            && xform.position == [0.0; 3]
            && xform.rotation == [0.0, 0.0, 0.0, 1.0]
            && xform.scale == [1.0, 1.0, 1.0]
            && draw.uniform_aux == 0;
    }

    fn update_skinned_transform(draw: &mut SkinnedMeshDraw, xform: XformKey) {
        let mesh = draw.mesh.clone();
        let model = xform.model_mat();
        draw.model_mat = model.to_cols_array_2d();
        draw.world_bounds = world_bounds(model, &mesh);
    }

    pub fn push_mesh(
        &mut self,
        ent: u64,
        t: &TransformComponent,
        m: &Mesh3D,
        static_key: StaticMeshKey,
        build_static: impl FnOnce() -> Option<MeshDraw>,
        frustum: Option<&Frustum>,
    ) {
        self.alive.insert(ent);
        let xform = XformKey::from_transform(t);

        if let Some(entry) = self.mesh.get(&ent) {
            if entry.static_key == static_key && entry.xform == xform {
                self.full_hit += 1;
                if let Some(f) = frustum {
                    if !f.intersects_aabb(&entry.draw.world_bounds) {
                        self.frustum_skip += 1;
                        return;
                    }
                }
                self.mesh_out.push(entry.draw.clone());
                return;
            }
        }

        let needs_static = self
            .mesh
            .get(&ent)
            .map(|e| e.static_key != static_key)
            .unwrap_or(true);

        if needs_static {
            let Some(mut draw) = build_static() else {
                self.mesh.remove(&ent);
                return;
            };
            Self::update_mesh_transform(&mut draw, xform);
            self.mesh.insert(
                ent,
                CachedMeshEntry {
                    static_key,
                    xform,
                    draw,
                },
            );
            self.static_rebuild += 1;
        } else {
            let entry = self.mesh.get_mut(&ent).expect("cache entry");
            if entry.xform != xform {
                Self::update_mesh_transform(&mut entry.draw, xform);
                entry.xform = xform;
                self.xform_only += 1;
            } else {
                self.full_hit += 1;
            }
        }

        let draw = &self.mesh.get(&ent).expect("cache entry").draw;
        if let Some(f) = frustum {
            if !f.intersects_aabb(&draw.world_bounds) {
                self.frustum_skip += 1;
                return;
            }
        }
        self.mesh_out.push(draw.clone());
    }

    pub fn push_skinned(
        &mut self,
        ent: u64,
        t: &TransformComponent,
        static_key: StaticSkinnedKey,
        build_static: impl FnOnce() -> Option<SkinnedMeshDraw>,
        frustum: Option<&Frustum>,
    ) {
        self.alive.insert(ent);
        let xform = XformKey::from_transform(t);

        if let Some(entry) = self.skinned.get(&ent) {
            if entry.static_key == static_key && entry.xform == xform {
                self.full_hit += 1;
                if let Some(f) = frustum {
                    if !f.intersects_aabb(&entry.draw.world_bounds) {
                        self.frustum_skip += 1;
                        return;
                    }
                }
                self.skinned_out.push(entry.draw.clone());
                return;
            }
        }

        let needs_static = self
            .skinned
            .get(&ent)
            .map(|e| e.static_key != static_key)
            .unwrap_or(true);

        if needs_static {
            let Some(mut draw) = build_static() else {
                self.skinned.remove(&ent);
                return;
            };
            Self::update_skinned_transform(&mut draw, xform);
            self.skinned.insert(
                ent,
                CachedSkinnedEntry {
                    static_key,
                    xform,
                    draw,
                },
            );
            self.static_rebuild += 1;
        } else {
            let entry = self.skinned.get_mut(&ent).expect("cache entry");
            if entry.xform != xform {
                Self::update_skinned_transform(&mut entry.draw, xform);
                entry.xform = xform;
                self.xform_only += 1;
            } else {
                self.full_hit += 1;
            }
        }

        let draw = &self.skinned.get(&ent).expect("cache entry").draw;
        if let Some(f) = frustum {
            if !f.intersects_aabb(&draw.world_bounds) {
                self.frustum_skip += 1;
                return;
            }
        }
        self.skinned_out.push(draw.clone());
    }

    pub fn take_mesh_out(&mut self) -> Vec<MeshDraw> {
        std::mem::take(&mut self.mesh_out)
    }

    pub fn take_skinned_out(&mut self) -> Vec<SkinnedMeshDraw> {
        std::mem::take(&mut self.skinned_out)
    }
}

pub fn static_mesh_key(
    m: &Mesh3D,
    dm: &DrawMaterial,
    uniform_aux: u32,
) -> StaticMeshKey {
    StaticMeshKey {
        mesh_handle: m.mesh.0,
        material_handle: dm.handle.0,
        uniform_aux,
    }
}

pub fn static_skinned_key(
    mesh: MeshHandle,
    dm: &DrawMaterial,
    bone_buffer: u32,
) -> StaticSkinnedKey {
    StaticSkinnedKey {
        mesh_handle: mesh.0,
        material_handle: dm.handle.0,
        bone_buffer,
    }
}
