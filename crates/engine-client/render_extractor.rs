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

pub struct RenderExtractor {
    q_cameras: QueryId,
    q_draw_materials: QueryId,
    q_terrain_splat: QueryId,
    q_lights: QueryId,
    q_skinned: QueryId,
    char_id: ComponentId,
    mesh_draw_aux_id: ComponentId,
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
        Self {
            q_cameras: world.query_prepare("TransformComponent, CameraComponent"),
            q_draw_materials: world.query_prepare("TransformComponent, Mesh3D, DrawMaterial"),
            q_terrain_splat: world
                .query_prepare("TransformComponent, Mesh3D, TerrainSplatMaterial"),
            q_lights: world.query_prepare("TransformComponent, PointLight"),
            q_skinned: world.query_prepare("TransformComponent, SkinnedMesh3D, DrawMaterial"),
            char_id,
            mesh_draw_aux_id,
        }
    }

    /// `view_entity`: if set, only that pawn's camera is used (server SetView).
    ///
    /// `material_store` — for resolving `DrawMaterial` handles to GPU indices.
    /// `material_allocator` — shared GPU SSBO (same buffer the renderer binds).
    /// `device` / `queue` — for lazy GPU upload of new materials.
    pub fn extract(
        &self,
        world: &World,
        meshes: &MeshRegistry,
        view_entity: Option<u64>,
        material_store: &mut MaterialStore,
        material_allocator: &mut MaterialAllocator,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _fallback_mesh: &Arc<deerpunk_engine_common::resource::mesh::Mesh>,
        gpu_stats: &mut SceneExtractGpuStats,
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

                    let model = glam::Mat4::from_scale_rotation_translation(
                        t.scale, t.rotation, t.position,
                    );
                    let model_mat = model.to_cols_array_2d();
                    let mesh = resolve(ent, m.mesh);
                    let world_bounds = world_bounds(model, &mesh);

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

                    let (shader_id, lightmap_gpu) = material_store
                        .get(dm.handle)
                        .map(|e| (e.shader, e.lightmap_id))
                        .unwrap_or((0, 0));

                    if shader_id == 0 {
                        panic!(
                        "DrawMaterial entity {ent:#x} handle {:?}: shader not loaded (shader_id=0)",
                        dm.handle
                    );
                        continue;
                    }

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
                    let batchable = m.mesh.is_valid()
                        && mesh.allocator_id != 0
                        && t.position == glam::Vec3::ZERO
                        && t.rotation == glam::Quat::IDENTITY
                        && t.scale == glam::Vec3::ONE
                        && uniform_aux == 0;

                    gpu_stats.mesh_draws += 1;
                    scene.geometry.mesh_draws.push(MeshDraw {
                        mesh,
                        model_mat,
                        batchable,
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
                        world_bounds,
                        entity_id: ent,
                        uniform_aux,
                    });
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
                    let model = glam::Mat4::from_scale_rotation_translation(
                        t.scale, t.rotation, t.position,
                    );
                    let model_mat = model.to_cols_array_2d();
                    let mesh = resolve(ent, sm.mesh);
                    let world_bounds = world_bounds(model, &mesh);
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
                    gpu_stats.skinned_draws += 1;
                    scene.geometry.skinned_mesh_draws.push(SkinnedMeshDraw {
                        mesh,
                        model_mat,
                        material_id,
                        texture,
                        normal,
                        orm,
                        shader_id,
                        bone_buffer: sm.bone_buffer,
                        world_bounds,
                        entity_id: ent,
                    });
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

        scene
    }
}
