use std::sync::Arc;

use deerpunk_engine_common::components::mesh3d::Mesh3D;
use deerpunk_engine_common::components::terrain_splat::TerrainSplatMaterial;
use deerpunk_engine_common::components::SkinnedMesh3D;
use deerpunk_engine_common::components::{
    CameraComponent, MeshDrawAux, MeshDrawHidden, PointLight, RenderLayer, TransformComponent,
    WorldTransformComponent,
};
use deerpunk_engine_common::components::render_layer::{
    render_layer_visible, RENDER_LAYER_DEFAULT_DRAW, RENDER_LAYER_WORLD,
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
use deerpunk_engine_renderer::world_bounds;
use deerpunk_engine_renderer::SceneExtractGpuStats;
use glam::Vec3;

use crate::render_extract_cache::{
    static_mesh_key, static_skinned_key, MeshDrawExtractCache,
};

pub struct RenderExtractor {
    q_cameras: QueryId,
    q_draw_materials: QueryId,
    q_draw_stream: QueryId,
    q_terrain_splat: QueryId,
    q_lights: QueryId,
    q_skinned: QueryId,
    parent_id: ComponentId,
    render_layer_id: ComponentId,
    mesh_draw_aux_id: ComponentId,
    mesh_draw_hidden_id: ComponentId,
    world_transform_id: ComponentId,
    q_world_sync_roots: QueryId,
    q_world_sync_parented: QueryId,
    mesh_cache: MeshDrawExtractCache,
    camera_list_scratch: Vec<(u64, TransformComponent, CameraComponent)>,
}

impl RenderExtractor {
    pub fn new(world: &mut World) -> Self {
        let _ = world.component_init::<TransformComponent>();
        let _ = world.component_init::<CameraComponent>();
        let parent_id = world.component_init::<deerpunk_engine_common::components::ParentComponent>();
        let _ = world.component_init::<Mesh3D>();
        let _ = world.component_init::<DrawMaterial>();
        let _ = world.component_init::<SkinnedMesh3D>();
        let _ = world.component_init::<TerrainSplatMaterial>();
        let _ = world.component_init::<PointLight>();
        let mesh_draw_aux_id = world.component_init::<MeshDrawAux>();
        let world_transform_id = world.component_init::<deerpunk_engine_common::components::WorldTransformComponent>();
        let mesh_draw_hidden_id = world.component_init::<MeshDrawHidden>();
        let render_layer_id = world.component_init::<RenderLayer>();
        let q_world_sync_roots = world.query_prepare("TransformComponent, !ParentComponent");
        let q_world_sync_parented = world.query_prepare("TransformComponent, ParentComponent");
        Self {
            q_cameras: world.query_prepare("TransformComponent, CameraComponent"),
            q_draw_materials: world.query_prepare(
                "WorldTransformComponent, Mesh3D, DrawMaterial, !MeshDrawHidden",
            ),
            q_draw_stream: world.query_prepare("WorldTransformComponent, Mesh3D, DrawMaterial"),
            q_terrain_splat: world
                .query_prepare("TransformComponent, Mesh3D, TerrainSplatMaterial"),
            q_lights: world.query_prepare("TransformComponent, PointLight"),
            q_skinned: world.query_prepare(
                "WorldTransformComponent, SkinnedMesh3D, DrawMaterial, !MeshDrawHidden",
            ),
            parent_id,
            render_layer_id,
            mesh_draw_aux_id,
            mesh_draw_hidden_id,
            world_transform_id,
            q_world_sync_roots,
            q_world_sync_parented,
            mesh_cache: MeshDrawExtractCache::new(),
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

    /// Toggle [`MeshDrawHidden`] on streamed mesh entities outside `draw_distance` (0 = off).
    /// Gamelib should still `destroy` unloaded chunks; this trims extract query work.
    pub fn apply_draw_stream_distance(
        &self,
        world: &mut World,
        view_entity: Option<u64>,
        draw_distance: f32,
    ) {
        if draw_distance <= 0.0 {
            return;
        }
        let Some(cam_pos) = self.active_camera_position(world, view_entity) else {
            return;
        };
        let dist_sq = draw_distance * draw_distance;
        let Some(batches) = world.query_batches(self.q_draw_stream) else {
            return;
        };
        for batch in batches {
            let entities = unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
            let world_xforms = unsafe {
                std::slice::from_raw_parts(
                    batch.columns[0] as *const WorldTransformComponent,
                    batch.count,
                )
            };
            for i in 0..batch.count {
                let ent = entities[i];
                let p = world_xforms[i].position;
                let dsq = (p - cam_pos).length_squared();
                if dsq > dist_sq {
                    world.set(ent, self.mesh_draw_hidden_id, MeshDrawHidden::marker());
                } else if world
                    .get::<MeshDrawHidden>(ent, self.mesh_draw_hidden_id)
                    .is_some()
                {
                    world.remove(ent, self.mesh_draw_hidden_id);
                }
            }
        }
    }

    fn active_camera_position(&self, world: &World, view_entity: Option<u64>) -> Option<Vec3> {
        if let Some(ve) = view_entity {
            if let Some(wt) = world.get::<WorldTransformComponent>(ve, self.world_transform_id) {
                return Some(wt.position);
            }
        }
        let mut best: Option<(u8, u64, Vec3)> = None;
        let Some(batches) = world.query_batches(self.q_cameras) else {
            return None;
        };
        for batch in batches {
            let entities = unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
            let transforms = unsafe {
                std::slice::from_raw_parts(
                    batch.columns[0] as *const TransformComponent,
                    batch.count,
                )
            };
            let cameras = unsafe {
                std::slice::from_raw_parts(batch.columns[1] as *const CameraComponent, batch.count)
            };
            for i in 0..batch.count {
                let ent = entities[i];
                if !camera_entity_in_view(world, view_entity, ent) {
                    continue;
                }
                let c = &cameras[i];
                let pos = world
                    .get::<WorldTransformComponent>(ent, self.world_transform_id)
                    .map(|w| w.position)
                    .unwrap_or_else(|| {
                        let (p, _) = camera_world_pose(world, ent, &transforms[i], self.parent_id);
                        p
                    });
                match &best {
                    None => best = Some((c.order, ent, pos)),
                    Some((ord, e, _)) if (c.order, ent) < (*ord, *e) => {
                        best = Some((c.order, ent, pos));
                    }
                    _ => {}
                }
            }
        }
        best.map(|(_, _, p)| p)
    }

    fn draw_layer_mask(world: &World, ent: u64, render_layer_id: ComponentId) -> u32 {
        world
            .get::<RenderLayer>(ent, render_layer_id)
            .map(|l| l.mask)
            .unwrap_or(RENDER_LAYER_DEFAULT_DRAW)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_mesh_draw(
        ent_copy: u64,
        m: &Mesh3D,
        dm_handle: deerpunk_engine_common::material::MaterialHandle,
        uniform_aux: u32,
        layer_mask: u32,
        resolve: &dyn Fn(u64, deerpunk_engine_common::resource::mesh_handle::MeshHandle) -> Arc<deerpunk_engine_common::resource::mesh::Mesh>,
        material_store: &mut MaterialStore,
        material_allocator: &mut MaterialAllocator,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu_stats: &mut SceneExtractGpuStats,
    ) -> Option<MeshDraw> {
        let mesh = resolve(ent_copy, m.mesh);
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
            mesh,
            model_mat: [[0.0; 4]; 4],
            batchable: false,
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
        meshes: &MeshRegistry,
        view_entity: Option<u64>,
        material_store: &mut MaterialStore,
        material_allocator: &mut MaterialAllocator,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _fallback_mesh: &Arc<deerpunk_engine_common::resource::mesh::Mesh>,
        gpu_stats: &mut SceneExtractGpuStats,
        aspect: f32,
        early_cull: bool,
    ) -> RenderScene {
        let fallback_mesh = _fallback_mesh.clone();
        let resolve = |ent: u64,
                       h: deerpunk_engine_common::resource::mesh_handle::MeshHandle|
         -> Arc<deerpunk_engine_common::resource::mesh::Mesh> {
            if let Some(m) = meshes.get(h) {
                return m.clone();
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
            fallback_mesh.clone()
        };
        let mut scene = RenderScene::new();
        scene.geometry.mesh_draws.clear();
        scene.geometry.skinned_mesh_draws.clear();
        let extract_body_start = std::time::Instant::now();
        self.mesh_cache.begin_frame();

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

        let extract_frustum = if early_cull {
            scene
                .cameras
                .first()
                .map(|c| MeshDrawExtractCache::frustum_from_camera(c, aspect))
        } else {
            None
        };
        let frustum_ref = extract_frustum.as_ref();
        let mesh_out = &mut scene.geometry.mesh_draws;
        let skinned_out = &mut scene.geometry.skinned_mesh_draws;

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

                    let uniform_aux = if self.mesh_draw_aux_id != 0 {
                        let ptr = world.get_raw(ent, self.mesh_draw_aux_id);
                        if ptr.is_null() {
                            0
                        } else {
                            unsafe { (*(ptr as *const MeshDrawAux)).crack_stage }
                        }
                    } else {
                        0
                    };
                    let static_key = static_mesh_key(m, dm, uniform_aux);
                    if self.mesh_cache.try_mesh_world_hit(
                        ent,
                        wt,
                        static_key,
                        frustum_ref,
                        mesh_out,
                    ) {
                        continue;
                    }
                    let draw_layers = Self::draw_layer_mask(world, ent, self.render_layer_id);
                    if draw_layers == 0 {
                        continue;
                    }
                    let ent_copy = ent;
                    let dm_handle = dm.handle;
                    let build = || {
                        Self::build_mesh_draw(
                            ent_copy,
                            m,
                            dm_handle,
                            uniform_aux,
                            draw_layers,
                            &resolve,
                            material_store,
                            material_allocator,
                            device,
                            queue,
                            gpu_stats,
                        )
                    };
                    self.mesh_cache.push_mesh(
                        ent,
                        wt,
                        m,
                        static_key,
                        build,
                        frustum_ref,
                        mesh_out,
                    );
                }
            }
        }

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
                    let mesh = resolve(ent, m.mesh);
                    gpu_stats.terrain_draws += 1;
                    scene.geometry.terrain_splat_draws.push(TerrainSplatDraw {
                        mesh,
                        model_mat: model_mat.to_cols_array_2d(),
                        layers: s.layers,
                        splat_map: s.splat_map,
                        sun_dir: s.sun_dir,
                        ambient: s.ambient,
                    });
                }
            }
        }

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
                    let static_key = static_skinned_key(sm.mesh, dm, sm.bone_buffer);
                    let ent_copy = ent;
                    let dm_handle = dm.handle;
                    let bone_buffer = sm.bone_buffer;
                    self.mesh_cache.push_skinned(
                        ent,
                        wt,
                        static_key,
                        || {
                            let mesh = resolve(ent_copy, sm.mesh);
                            let (texture, normal, orm) = match material_store.get(dm_handle) {
                                Some(e) => (e.albedo, e.normal, e.orm),
                                None => {
                                    (MeshTexture::WHITE, MeshTexture::WHITE, MeshTexture::WHITE)
                                }
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
                            let shader_id =
                                material_store.get(dm_handle).map(|e| e.shader).unwrap_or(0);
                            let draw_layers = Self::draw_layer_mask(world, ent, self.render_layer_id);
                            Some(SkinnedMeshDraw {
                                mesh,
                                model_mat: [[0.0; 4]; 4],
                                layer_mask: draw_layers,
                                material_id,
                                texture,
                                normal,
                                orm,
                                shader_id,
                                bone_buffer,
                                world_bounds: deerpunk_engine_renderer::Aabb::default(),
                                entity_id: ent_copy,
                            })
                        },
                        frustum_ref,
                        skinned_out,
                    );
                }
            }
        }

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

        gpu_stats.extractor_ms = extract_body_start.elapsed().as_secs_f32() * 1000.0;
        gpu_stats.extract_static_rebuild = self.mesh_cache.static_rebuild;
        gpu_stats.extract_xform_only = self.mesh_cache.xform_only;
        gpu_stats.extract_cache_hit = self.mesh_cache.full_hit;
        gpu_stats.extract_frustum_skip = self.mesh_cache.frustum_skip;

        self.mesh_cache.finish_frame();

        gpu_stats.mesh_draws = scene.geometry.mesh_draws.len() as u32;
        gpu_stats.skinned_draws = scene.geometry.skinned_mesh_draws.len() as u32;

        let serial_start = std::time::Instant::now();
        scene.geometry.refresh_cull_serial();
        gpu_stats.cull_serial_ms = serial_start.elapsed().as_secs_f32() * 1000.0;

        scene.extract_early_cull = early_cull && frustum_ref.is_some();

        scene
    }
}
