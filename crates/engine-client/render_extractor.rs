use std::sync::Arc;

use deerpunk_engine_common::components::mesh3d::Mesh3D;
use deerpunk_engine_common::components::terrain_splat::TerrainSplatMaterial;
use deerpunk_engine_common::components::SkinnedMesh3D;
use deerpunk_engine_common::components::{CameraComponent, MeshDrawAux, PointLight, TransformComponent};
use deerpunk_engine_common::material::DrawMaterial;
use deerpunk_engine_common::physics::CharacterController;
use deerpunk_engine_common::resource::mesh_registry::MeshRegistry;
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

use crate::render_extract_cache::{
    static_mesh_key, static_skinned_key, MeshDrawExtractCache,
};

pub struct RenderExtractor {
    q_cameras: QueryId,
    q_draw_materials: QueryId,
    q_terrain_splat: QueryId,
    q_lights: QueryId,
    q_skinned: QueryId,
    char_id: ComponentId,
    mesh_draw_aux_id: ComponentId,
    mesh_cache: MeshDrawExtractCache,
}

impl RenderExtractor {
    pub fn new(world: &mut World) -> Self {
        let _ = world.component_init::<TransformComponent>();
        let _ = world.component_init::<CameraComponent>();
        let _ = world.component_init::<Mesh3D>();
        let _ = world.component_init::<DrawMaterial>();
        let _ = world.component_init::<SkinnedMesh3D>();
        let _ = world.component_init::<TerrainSplatMaterial>();
        let _ = world.component_init::<PointLight>();
        let char_id = world.component_init::<CharacterController>();
        let mesh_draw_aux_id = world.component_init::<MeshDrawAux>();
        let _ = world.component_init::<deerpunk_engine_common::components::MeshDrawHidden>();
        Self {
            q_cameras: world.query_prepare("TransformComponent, CameraComponent"),
            q_draw_materials: world
                .query_prepare("TransformComponent, Mesh3D, DrawMaterial, !MeshDrawHidden"),
            q_terrain_splat: world
                .query_prepare("TransformComponent, Mesh3D, TerrainSplatMaterial"),
            q_lights: world.query_prepare("TransformComponent, PointLight"),
            q_skinned: world.query_prepare(
                "TransformComponent, SkinnedMesh3D, DrawMaterial, !MeshDrawHidden",
            ),
            char_id,
            mesh_draw_aux_id,
            mesh_cache: MeshDrawExtractCache::new(),
        }
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
        let extract_body_start = std::time::Instant::now();
        self.mesh_cache.begin_frame();

        if let Some(batches) = world.query_batches(self.q_cameras) {
            for batch in batches {
                let entities = unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
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
                    if let Some(view) = view_entity {
                        if ent != view {
                            continue;
                        }
                    } else if !scene.cameras.is_empty() {
                        // No SetView yet — keep a single camera (avoid flicker).
                        break;
                    }
                    // Origin = feet for pawns: eye_height on camera, else CC default.
                    let mut eye = c.eye_height;
                    if eye <= 0.0 {
                        if let Some(cc) = world.get::<CharacterController>(ent, self.char_id) {
                            eye = cc.eye_height();
                        }
                    }
                    scene.push_camera(CameraDefinition {
                        position: [t.position.x, t.position.y, t.position.z + eye],
                        rotation: c.world_rotation(t.rotation),
                        fov_y: c.fov_y,
                        near: c.near,
                        far: c.far,
                        target: RenderTarget::Surface,
                        dirty: true,
                    });
                }
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

        if let Some(batches) = world.query_batches(self.q_draw_materials) {
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
                let draw_mats = unsafe {
                    std::slice::from_raw_parts(batch.columns[2] as *const DrawMaterial, batch.count)
                };
                for i in 0..batch.count {
                    let ent = entities[i];
                    let t = &transforms[i];
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
                    let ent_copy = ent;
                    let dm_handle = dm.handle;
                    self.mesh_cache.push_mesh(
                        ent,
                        t,
                        m,
                        static_key,
                        || {
                            let mesh = resolve(ent_copy, m.mesh);
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
                            })
                        },
                        frustum_ref,
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
                let transforms = unsafe {
                    std::slice::from_raw_parts(
                        batch.columns[0] as *const TransformComponent,
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
                    let t = &transforms[i];
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
                        t,
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
                            Some(SkinnedMeshDraw {
                                mesh,
                                model_mat: [[0.0; 4]; 4],
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

        scene.geometry.mesh_draws = self.mesh_cache.take_mesh_out();
        scene.geometry.skinned_mesh_draws = self.mesh_cache.take_skinned_out();
        gpu_stats.mesh_draws = scene.geometry.mesh_draws.len() as u32;
        gpu_stats.skinned_draws = scene.geometry.skinned_mesh_draws.len() as u32;
        self.mesh_cache.finish_frame();

        let serial_start = std::time::Instant::now();
        scene.geometry.refresh_cull_serial();
        gpu_stats.cull_serial_ms = serial_start.elapsed().as_secs_f32() * 1000.0;

        scene
    }
}
