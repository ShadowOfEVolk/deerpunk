//! Cached scene extract: static draw data per entity, transform-only updates, optional early frustum.

use std::collections::HashMap;
use std::sync::Arc;

use deerpunk_engine_common::components::{Mesh3D, TransformComponent, WorldTransformComponent};
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
    fn from_world(w: &WorldTransformComponent) -> Self {
        Self {
            position: w.position.to_array(),
            rotation: w.rotation.to_array(),
            scale: w.scale.to_array(),
        }
    }

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
    touch_gen: u32,
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
    touch_gen: u32,
}

pub struct MeshDrawExtractCache {
    mesh: HashMap<u64, CachedMeshEntry>,
    skinned: HashMap<u64, CachedSkinnedEntry>,
    frame_gen: u32,
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
            frame_gen: 0,
            static_rebuild: 0,
            xform_only: 0,
            full_hit: 0,
            frustum_skip: 0,
        }
    }

    pub fn clear(&mut self) {
        self.mesh.clear();
        self.skinned.clear();
        self.frame_gen = 0;
    }

    pub fn begin_frame(&mut self) {
        self.frame_gen = self.frame_gen.wrapping_add(1);
        if self.frame_gen == 0 {
            self.frame_gen = 1;
        }
        self.static_rebuild = 0;
        self.xform_only = 0;
        self.full_hit = 0;
        self.frustum_skip = 0;
    }

    pub fn finish_frame(&mut self) {
        let gen = self.frame_gen;
        self.mesh.retain(|_, e| e.touch_gen == gen);
        self.skinned.retain(|_, e| e.touch_gen == gen);
    }

    pub fn frustum_from_camera(cam: &CameraDefinition, aspect: f32) -> Frustum {
        let view = cam.view_matrix();
        let projection = cam.projection_matrix(aspect);
        Frustum::from_view_proj(projection * view)
    }

    fn update_mesh_transform(draw: &mut MeshDraw, xform: XformKey) {
        let model = xform.model_mat();
        draw.model_mat = model.to_cols_array_2d();
        draw.world_bounds = world_bounds(model, &draw.mesh);
        draw.batchable = mesh_draw_batchable(&draw.mesh, draw.uniform_aux);
    }

    fn update_skinned_transform(draw: &mut SkinnedMeshDraw, xform: XformKey) {
        let model = xform.model_mat();
        draw.model_mat = model.to_cols_array_2d();
        draw.world_bounds = world_bounds(model, &draw.mesh);
    }

    fn push_visible_mesh(out: &mut Vec<MeshDraw>, draw: &MeshDraw) {
        out.push(MeshDraw {
            batchable: mesh_draw_batchable(&draw.mesh, draw.uniform_aux),
            ..draw.clone()
        });
    }

    pub fn push_mesh(
        &mut self,
        ent: u64,
        world: &WorldTransformComponent,
        m: &Mesh3D,
        static_key: StaticMeshKey,
        build_static: impl FnOnce() -> Option<MeshDraw>,
        frustum: Option<&Frustum>,
        out: &mut Vec<MeshDraw>,
    ) {
        let xform = XformKey::from_world(world);
        let gen = self.frame_gen;

        if let Some(entry) = self.mesh.get_mut(&ent) {
            if entry.static_key == static_key && entry.xform == xform {
                entry.touch_gen = gen;
                self.full_hit += 1;
                if let Some(f) = frustum {
                    if !f.intersects_aabb(&entry.draw.world_bounds) {
                        self.frustum_skip += 1;
                        return;
                    }
                }
                Self::push_visible_mesh(out, &entry.draw);
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
                    touch_gen: gen,
                },
            );
            self.static_rebuild += 1;
        } else {
            let entry = self.mesh.get_mut(&ent).expect("cache entry");
            entry.touch_gen = gen;
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
        Self::push_visible_mesh(out, draw);
    }

    /// Full cache hit: only frustum + clone (world pose already in [`WorldTransformComponent`]).
    pub fn try_mesh_world_hit(
        &mut self,
        ent: u64,
        world: &WorldTransformComponent,
        static_key: StaticMeshKey,
        frustum: Option<&Frustum>,
        out: &mut Vec<MeshDraw>,
    ) -> bool {
        let xform = XformKey::from_world(world);
        let gen = self.frame_gen;
        let Some(entry) = self.mesh.get_mut(&ent) else {
            return false;
        };
        if entry.static_key != static_key || entry.xform != xform {
            return false;
        }
        entry.touch_gen = gen;
        self.full_hit += 1;
        if let Some(f) = frustum {
            if !f.intersects_aabb(&entry.draw.world_bounds) {
                self.frustum_skip += 1;
                return true;
            }
        }
        Self::push_visible_mesh(out, &entry.draw);
        true
    }

    pub fn push_skinned(
        &mut self,
        ent: u64,
        world: &WorldTransformComponent,
        static_key: StaticSkinnedKey,
        build_static: impl FnOnce() -> Option<SkinnedMeshDraw>,
        frustum: Option<&Frustum>,
        out: &mut Vec<SkinnedMeshDraw>,
    ) {
        let xform = XformKey::from_world(world);
        let gen = self.frame_gen;

        if let Some(entry) = self.skinned.get_mut(&ent) {
            if entry.static_key == static_key && entry.xform == xform {
                entry.touch_gen = gen;
                self.full_hit += 1;
                if let Some(f) = frustum {
                    if !f.intersects_aabb(&entry.draw.world_bounds) {
                        self.frustum_skip += 1;
                        return;
                    }
                }
                out.push(entry.draw.clone());
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
                    touch_gen: gen,
                },
            );
            self.static_rebuild += 1;
        } else {
            let entry = self.skinned.get_mut(&ent).expect("cache entry");
            entry.touch_gen = gen;
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
        out.push(draw.clone());
    }
}

/// Pooled meshes (`allocator_id != 0`) may use multi-draw indirect; each draw keeps its own
/// `MeshUniform` slot via `first_instance` / `@builtin(instance_index)` (chunk world origin in `model`).
/// Pooled meshes may share pipeline/texture binds; each draw keeps its own uniform slot.
pub fn mesh_draw_batchable(mesh: &std::sync::Arc<deerpunk_engine_common::resource::mesh::Mesh>, uniform_aux: u32) -> bool {
    mesh.allocator_id != 0 && uniform_aux == 0
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

#[cfg(test)]
mod tests {
    use super::mesh_draw_batchable;
    use deerpunk_engine_common::resource::mesh::Mesh;
    use std::sync::Arc;

    #[test]
    fn batchable_pooled_without_aux() {
        let mut mesh = Mesh::empty();
        mesh.allocator_id = 1;
        let mesh = Arc::new(mesh);
        assert!(mesh_draw_batchable(&mesh, 0));
        assert!(!mesh_draw_batchable(&mesh, 1));
    }

    #[test]
    fn batchable_not_without_pool() {
        let mesh = Arc::new(Mesh::empty());
        assert!(!mesh_draw_batchable(&mesh, 0));
    }
}
