use std::sync::Arc;

use deerpunk_engine_common::components::mesh3d::Mesh3D;
use deerpunk_engine_common::components::terrain_splat::TerrainSplatMaterial;
use deerpunk_engine_common::components::SkinnedMesh3D;
use deerpunk_engine_common::components::{
    CameraComponent, MeshDrawHidden, PointLight, TransformComponent,
    WorldTransformComponent,
};
use deerpunk_engine_common::transform_hierarchy::camera_world_pose;
use deerpunk_engine_common::material::DrawMaterial;
use deerpunk_engine_common::resource::mesh_registry::MeshRegistry;

use crate::camera_util::camera_entity_in_view;
use deerpunk_engine_common::world::{ComponentId, QueryId, World};
use deerpunk_engine_common::MeshTexture;
use deerpunk_engine_renderer::material_allocator::MaterialAllocator;
use deerpunk_engine_renderer::material_store::MaterialStore;
use deerpunk_engine_renderer::snapshot::{
    CameraDefinition, MeshDraw, PointLightDraw, RenderScene, RenderTarget, SkinnedMeshDraw,
    TerrainSplatDraw,
};
use deerpunk_engine_renderer::SceneExtractGpuStats;

pub struct RenderExtractor {
    q_cameras: QueryId,
    q_draw_materials: QueryId,
    q_terrain_splat: QueryId,
    q_lights: QueryId,
    q_skinned: QueryId,
    parent_id: ComponentId,
    mesh3d_id: ComponentId,
    skinned_id: ComponentId,
    world_transform_id: ComponentId,
    q_world_sync_roots: QueryId,
    q_world_sync_parented: QueryId,
    camera_list_scratch: Vec<(u64, TransformComponent, CameraComponent)>,
}

impl RenderExtractor {
    pub fn new(world: &mut World) -> Self {
        let _ = world.component_init::<TransformComponent>();
        let _ = world.component_init::<CameraComponent>();
        let parent_id = world.component_init::<deerpunk_engine_common::components::ParentComponent>();
        let mesh3d_id = world.component_init::<Mesh3D>();
        let _ = world.component_init::<DrawMaterial>();
        let skinned_id = world.component_init::<SkinnedMesh3D>();
        let _ = world.component_init::<TerrainSplatMaterial>();
        let _ = world.component_init::<PointLight>();
        let world_transform_id = world.component_init::<deerpunk_engine_common::components::WorldTransformComponent>();
        let _ = world.component_init::<MeshDrawHidden>();
        let q_world_sync_roots = world.query_prepare("TransformComponent, !ParentComponent");
        let q_world_sync_parented = world.query_prepare("TransformComponent, ParentComponent");
        Self {
            q_cameras: world.query_prepare("TransformComponent, CameraComponent"),
            q_draw_materials: world.query_prepare(
                "WorldTransformComponent, Mesh3D, DrawMaterial, !MeshDrawHidden",
            ),
            q_terrain_splat: world
                .query_prepare("TransformComponent, Mesh3D, TerrainSplatMaterial"),
            q_lights: world.query_prepare("TransformComponent, PointLight"),
            q_skinned: world.query_prepare(
                "WorldTransformComponent, SkinnedMesh3D, DrawMaterial, !MeshDrawHidden",
            ),
            parent_id,
            mesh3d_id,
            skinned_id,
            world_transform_id,
            q_world_sync_roots,
            q_world_sync_parented,
            camera_list_scratch: Vec::new(),
        }
    }

    /// [`TransformComponent`] → [`WorldTransformComponent`]; run before [`Self::extract`].
    pub fn sync_world_transforms(
        &self,
        world: &mut World,
    ) -> deerpunk_engine_common::transform_hierarchy::SyncWorldTransformStats {
        deerpunk_engine_common::transform_hierarchy::sync_world_transforms(
            world,
            self.q_world_sync_roots,
            self.q_world_sync_parented,
        )
    }

    fn static_world_bounds(
        &self,
        world: &World,
        meshes: &MeshRegistry,
        ent: u64,
        m: &Mesh3D,
        wt: &WorldTransformComponent,
    ) -> Option<deerpunk_engine_renderer::Aabb> {
        let pos = wt.position.to_array();
        let rot = wt.rotation.to_array();
        let scale = wt.scale.to_array();
        if let Some((min, max)) = m.fresh_world(pos, rot, scale) {
            return Some(deerpunk_engine_renderer::Aabb::from_min_max(
                glam::Vec3::from_array(min),
                glam::Vec3::from_array(max),
            ));
        }
        let mut updated = *m;
        if !updated.has_local_bounds() {
            let cpu = meshes.get(m.mesh)?;
            updated = updated.set_local_bounds(cpu.bounds_min, cpu.bounds_max);
        }
        let local = deerpunk_engine_renderer::Aabb::from_min_max(
            glam::Vec3::from_array(updated.bounds_min),
            glam::Vec3::from_array(updated.bounds_max),
        );
        let baked = local.transformed_trs(wt.position, wt.rotation, wt.scale);
        updated.bake_world(pos, rot, scale, baked.min.to_array(), baked.max.to_array());
        world.set(ent, self.mesh3d_id, updated);
        Some(baked)
    }

    fn skinned_world_bounds(
        &self,
        world: &World,
        meshes: &MeshRegistry,
        ent: u64,
        sm: &SkinnedMesh3D,
        wt: &WorldTransformComponent,
    ) -> Option<deerpunk_engine_renderer::Aabb> {
        let pos = wt.position.to_array();
        let rot = wt.rotation.to_array();
        let scale = wt.scale.to_array();
        if let Some((min, max)) = sm.fresh_world(pos, rot, scale) {
            return Some(deerpunk_engine_renderer::Aabb::from_min_max(
                glam::Vec3::from_array(min),
                glam::Vec3::from_array(max),
            ));
        }
        let mut updated = *sm;
        if !updated.has_local_bounds() {
            let cpu = meshes.get(sm.mesh)?;
            updated = updated.set_local_bounds(cpu.bounds_min, cpu.bounds_max);
        }
        let local = deerpunk_engine_renderer::Aabb::from_min_max(
            glam::Vec3::from_array(updated.bounds_min),
            glam::Vec3::from_array(updated.bounds_max),
        );
        let baked = local.transformed_trs(wt.position, wt.rotation, wt.scale);
        updated.bake_world(pos, rot, scale, baked.min.to_array(), baked.max.to_array());
        world.set(ent, self.skinned_id, updated);
        Some(baked)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_mesh_draw(
        ent_copy: u64,
        dm_handle: deerpunk_engine_common::material::MaterialHandle,
        uniform_aux: u32,
        layer_mask: u32,
        mesh_handle: deerpunk_engine_common::resource::mesh_handle::MeshHandle,
        mesh: &deerpunk_engine_common::resource::mesh::Mesh,
        material_store: &mut MaterialStore,
        material_allocator: &mut MaterialAllocator,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu_stats: &mut SceneExtractGpuStats,
    ) -> Option<MeshDraw> {
        let (texture, normal, orm) = match material_store.get(dm_handle) {
            Some(e) => (e.albedo, e.normal, e.orm),
            None => (MeshTexture::WHITE, MeshTexture::WHITE, MeshTexture::WHITE),
        };
        let t0 = std::time::Instant::now();
        let material_id = material_store.resolve_gpu_tracked(
            dm_handle,
            material_allocator,
            device,
            queue,
            Some(gpu_stats),
        );
        gpu_stats.resolve_ms += t0.elapsed().as_secs_f32() * 1000.0;

        let (shader_id, lightmap_gpu) = material_store
            .get(dm_handle)
            .map(|e| (e.shader, e.lightmap_id))
            .unwrap_or((0, 0));

        if shader_id == 0 {
            panic!(
                "DrawMaterial entity {ent_copy:#x} handle {:?}: shader not loaded (shader_id=0)",
                dm_handle
            );
        }

        Some(MeshDraw {
            mesh: mesh_handle,
            model_mat: [[0.0; 4]; 4],
            batchable: mesh_draw_batchable(mesh, uniform_aux),
            material_id,
            texture,
            normal,
            orm,
            sun_dir: [0.0, 0.0, 1.0, 0.0],
            ambient: [0.0; 4],
            shader_id,
            lightmap_gpu,
            first_index: 0,
            index_count: 0,
            world_bounds: deerpunk_engine_renderer::Aabb::default(),
            entity_id: ent_copy,
            uniform_aux,
            layer_mask,
        })
    }

    /// `view_entity`: if set, only that pawn's camera is used (server SetView).
    ///
    /// `material_store` — for resolving `DrawMaterial` handles to GPU indices.
    /// `material_allocator` — shared GPU SSBO (same buffer the renderer binds).
    /// `device` / `queue` — for lazy GPU upload of new materials.
    pub fn extract(
        &mut self,
        world: &World,
        registry: &MeshRegistry,
        view_entity: Option<u64>,
        material_store: &mut MaterialStore,
        material_allocator: &mut MaterialAllocator,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _fallback_mesh: &Arc<deerpunk_engine_common::resource::mesh::Mesh>,
        gpu_stats: &mut SceneExtractGpuStats,
        aspect: f32,
        early_cull: bool,
        draw_world: bool,
    ) -> RenderScene {
        let lookup = |ent: u64,
                      h: deerpunk_engine_common::resource::mesh_handle::MeshHandle|
         -> Option<&std::sync::Arc<deerpunk_engine_common::resource::mesh::Mesh>> {
            if let Some(m) = registry.get(h) {
                return Some(m);
            }
            if h.is_valid() {
                log::error!(
                    "entity {ent:#x}: stale MeshHandle {:?} — skipping draw (refresh Mesh3D on entity)",
                    h
                );
            } else {
                log::warn!(
                    "entity {ent:#x}: invalid MeshHandle on Mesh3D — skipping draw",
                );
            }
            None
        };
        let mut scene = RenderScene::new();
        scene.geometry.mesh_draws.clear();
        scene.geometry.skinned_mesh_draws.clear();
        let extract_body_start = std::time::Instant::now();
        let mut frustum_skip = 0u32;

        let _z_cam = deerpunk_engine_renderer::gamelib_profile::zone_scope("extract.cameras");
        if let Some(batches) = world.query_batches(self.q_cameras) {
            self.camera_list_scratch.clear();
            for batch in batches {
                let entities =
                    unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
                let transforms = unsafe {
                    std::slice::from_raw_parts(
                        batch.columns[0] as *const TransformComponent,
                        batch.count,
                    )
                };
                let cameras = unsafe {
                    std::slice::from_raw_parts(
                        batch.columns[1] as *const CameraComponent,
                        batch.count,
                    )
                };
                for (i, (t, c)) in transforms.iter().zip(cameras.iter()).enumerate() {
                    let ent = entities[i];
                    if !camera_entity_in_view(world, view_entity, ent) {
                        continue;
                    }
                    self.camera_list_scratch.push((ent, *t, *c));
                }
            }
            self.camera_list_scratch
                .sort_by_key(|(e, _, c)| (c.order, *e));
            for (ent, t, c) in self.camera_list_scratch.drain(..) {
                let (world_pos, world_rot) = world
                    .get::<WorldTransformComponent>(ent, self.world_transform_id)
                    .map(|w| (w.position, w.rotation))
                    .unwrap_or_else(|| camera_world_pose(world, ent, &t, self.parent_id));
                scene.push_camera(CameraDefinition {
                    position: world_pos.to_array(),
                    rotation: world_rot,
                    fov_y: c.fov_y,
                    near: c.near,
                    far: c.far,
                    target: RenderTarget::Surface,
                    layer_mask: c.layer_mask,
                    order: c.order,
                    pass_mask: c.pass_mask,
                    color_load: c.color_load,
                    depth: c.depth,
                    dirty: true,
                });
            }
        }
        drop(_z_cam);

        let extract_frustum = if early_cull {
            scene.cameras.first().map(|c| {
                let view = c.view_matrix();
                let projection = c.projection_matrix(aspect);
                deerpunk_engine_renderer::Frustum::from_view_proj(projection * view)
            })
        } else {
            None
        };
        let frustum_ref = extract_frustum.as_ref();
        let mesh_out = &mut scene.geometry.mesh_draws;
        let skinned_out = &mut scene.geometry.skinned_mesh_draws;

        if draw_world {
        let _z_static = deerpunk_engine_renderer::gamelib_profile::zone_scope("extract.static");
        if let Some(batches) = world.query_batches(self.q_draw_materials) {
            for batch in batches {
                let entities = unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
                let world_xforms = unsafe {
                    std::slice::from_raw_parts(
                        batch.columns[0] as *const WorldTransformComponent,
                        batch.count,
                    )
                };
                let meshes = unsafe {
                    std::slice::from_raw_parts(batch.columns[1] as *const Mesh3D, batch.count)
                };
                let draw_mats = unsafe {
                    std::slice::from_raw_parts(batch.columns[2] as *const DrawMaterial, batch.count)
                };
                for i in 0..batch.count {
                    let ent = entities[i];
                    let wt = &world_xforms[i];
                    let m = &meshes[i];
                    let dm = &draw_mats[i];

                    let uniform_aux = dm.material_data_y();
                    let draw_layers = m.layer_mask;
                    if draw_layers == 0 {
                        continue;
                    }
                    let Some(world_bounds) = self.static_world_bounds(world, registry, ent, m, wt)
                    else {
                        continue;
                    };
                    if let Some(f) = frustum_ref {
                        if !f.intersects_aabb(&world_bounds) {
                            frustum_skip += 1;
                            continue;
                        }
                    }
                    let Some(cpu) = lookup(ent, m.mesh) else {
                        continue;
                    };
                    let model = glam::Mat4::from_scale_rotation_translation(
                        wt.scale,
                        wt.rotation,
                        wt.position,
                    );
                    let Some(mut draw) = Self::build_mesh_draw(
                        ent,
                        dm.handle,
                        uniform_aux,
                        draw_layers,
                        m.mesh,
                        cpu.as_ref(),
                        material_store,
                        material_allocator,
                        device,
                        queue,
                        gpu_stats,
                    ) else {
                        continue;
                    };
                    draw.model_mat = model.to_cols_array_2d();
                    draw.world_bounds = world_bounds;
                    mesh_out.push(draw);
                }
            }
        }
        drop(_z_static);

        let _z_terrain = deerpunk_engine_renderer::gamelib_profile::zone_scope("extract.terrain");
        if let Some(batches) = world.query_batches(self.q_terrain_splat) {
            for batch in batches {
                let entities = unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
                let transforms = unsafe {
                    std::slice::from_raw_parts(
                        batch.columns[0] as *const TransformComponent,
                        batch.count,
                    )
                };
                let meshes = unsafe {
                    std::slice::from_raw_parts(batch.columns[1] as *const Mesh3D, batch.count)
                };
                let splats = unsafe {
                    std::slice::from_raw_parts(
                        batch.columns[2] as *const TerrainSplatMaterial,
                        batch.count,
                    )
                };
                for i in 0..batch.count {
                    let ent = entities[i];
                    let t = &transforms[i];
                    let m = &meshes[i];
                    let s = &splats[i];
                    let model_mat = glam::Mat4::from_scale_rotation_translation(
                        t.scale, t.rotation, t.position,
                    );
                    let Some(cpu) = lookup(ent, m.mesh) else {
                        continue;
                    };
                    let _ = cpu;
                    gpu_stats.terrain_draws += 1;
                    scene.geometry.terrain_splat_draws.push(TerrainSplatDraw {
                        mesh: m.mesh,
                        model_mat: model_mat.to_cols_array_2d(),
                        layers: s.layers,
                        splat_map: s.splat_map,
                        sun_dir: s.sun_dir,
                        ambient: s.ambient,
                    });
                }
            }
        }
        drop(_z_terrain);

        let _z_skinned = deerpunk_engine_renderer::gamelib_profile::zone_scope("extract.skinned");
        if let Some(batches) = world.query_batches(self.q_skinned) {
            for batch in batches {
                let entities = unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
                let world_xforms = unsafe {
                    std::slice::from_raw_parts(
                        batch.columns[0] as *const WorldTransformComponent,
                        batch.count,
                    )
                };
                let meshes = unsafe {
                    std::slice::from_raw_parts(
                        batch.columns[1] as *const SkinnedMesh3D,
                        batch.count,
                    )
                };
                let draw_mats = unsafe {
                    std::slice::from_raw_parts(batch.columns[2] as *const DrawMaterial, batch.count)
                };
                for i in 0..batch.count {
                    let ent = entities[i];
                    let wt = &world_xforms[i];
                    let sm = &meshes[i];
                    let dm = &draw_mats[i];
                    if sm.bone_buffer == 0 {
                        continue;
                    }
                    let Some(bounds) = self.skinned_world_bounds(world, registry, ent, sm, wt) else {
                        continue;
                    };
                    if let Some(f) = frustum_ref {
                        if !f.intersects_aabb(&bounds) {
                            frustum_skip += 1;
                            continue;
                        }
                    }
                    let draw_layers = sm.layer_mask;
                    if draw_layers == 0 {
                        continue;
                    }
                    let (texture, normal, orm) = match material_store.get(dm.handle) {
                        Some(e) => (e.albedo, e.normal, e.orm),
                        None => (MeshTexture::WHITE, MeshTexture::WHITE, MeshTexture::WHITE),
                    };
                    let t0 = std::time::Instant::now();
                    let material_id = material_store.resolve_gpu_tracked(
                        dm.handle,
                        material_allocator,
                        device,
                        queue,
                        Some(gpu_stats),
                    );
                    gpu_stats.resolve_ms += t0.elapsed().as_secs_f32() * 1000.0;
                    let shader_id = material_store.get(dm.handle).map(|e| e.shader).unwrap_or(0);
                    let model = glam::Mat4::from_scale_rotation_translation(
                        wt.scale,
                        wt.rotation,
                        wt.position,
                    );
                    skinned_out.push(SkinnedMeshDraw {
                        mesh: sm.mesh,
                        model_mat: model.to_cols_array_2d(),
                        layer_mask: draw_layers,
                        material_id,
                        texture,
                        normal,
                        orm,
                        shader_id,
                        bone_buffer: sm.bone_buffer,
                        world_bounds: bounds,
                        entity_id: ent,
                    });
                }
            }
        }
        drop(_z_skinned);
        }

        let _z_lights = deerpunk_engine_renderer::gamelib_profile::zone_scope("extract.lights");
        if let Some(batches) = world.query_batches(self.q_lights) {
            for batch in batches {
                let transforms = unsafe {
                    std::slice::from_raw_parts(
                        batch.columns[0] as *const TransformComponent,
                        batch.count,
                    )
                };
                let lights = unsafe {
                    std::slice::from_raw_parts(batch.columns[1] as *const PointLight, batch.count)
                };
                for (t, l) in transforms.iter().zip(lights.iter()) {
                    scene.geometry.point_lights.push(PointLightDraw {
                        position: [t.position.x, t.position.y, t.position.z],
                        color: l.color,
                        intensity: l.intensity,
                        range: l.range,
                    });
                }
            }
        }
        drop(_z_lights);

        gpu_stats.extractor_ms = extract_body_start.elapsed().as_secs_f32() * 1000.0;
        gpu_stats.extract_frustum_skip = frustum_skip;

        gpu_stats.mesh_draws = scene.geometry.mesh_draws.len() as u32;
        gpu_stats.skinned_draws = scene.geometry.skinned_mesh_draws.len() as u32;

        let serial_start = std::time::Instant::now();
        {
            let _z = deerpunk_engine_renderer::gamelib_profile::zone_scope("extract.cull_serial");
            scene.geometry.refresh_cull_serial();
        }
        gpu_stats.cull_serial_ms = serial_start.elapsed().as_secs_f32() * 1000.0;

        scene.extract_early_cull = early_cull && frustum_ref.is_some();

        scene
    }
}

/// Bind groups do not depend on the per-draw uniform slot (`uniform_aux`, mining crack).
/// That slot is selected by `first_instance`, so these draws can share one bind.
fn mesh_draw_batchable(
    _mesh: &deerpunk_engine_common::resource::mesh::Mesh,
    _uniform_aux: u32,
) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::mesh_draw_batchable;
    use deerpunk_engine_common::resource::mesh::Mesh;

    #[test]
    fn batchable_ignores_pool_and_aux() {
        let mesh = Mesh::empty();
        assert!(mesh_draw_batchable(&mesh, 0));
        assert!(mesh_draw_batchable(&mesh, 1));
        let mut pooled = Mesh::empty();
        pooled.allocator_id = 1;
        assert!(mesh_draw_batchable(&pooled, 4));
    }
}
