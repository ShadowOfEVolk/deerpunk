use std::sync::Arc;

use deerpunk_engine_common::components::material::Material;
use deerpunk_engine_common::components::mesh3d::Mesh3D;
use deerpunk_engine_common::components::pending_lightmap::LightmapId;
use deerpunk_engine_common::components::terrain_splat::TerrainSplatMaterial;
use deerpunk_engine_common::components::{CameraComponent, PointLight, TransformComponent};
use deerpunk_engine_common::physics::CharacterController;
use deerpunk_engine_common::resource::mesh::Mesh;
use deerpunk_engine_common::world::{ComponentId, QueryId, World};
use deerpunk_engine_renderer::snapshot::{
    CameraDefinition, MeshDraw, PointLightDraw, RenderScene, RenderTarget, TerrainSplatDraw,
};

pub struct RenderExtractor {
    q_cameras: QueryId,
    q_meshes: QueryId,
    q_terrain_splat: QueryId,
    q_lights: QueryId,
    lightmap_id: ComponentId,
    char_id: ComponentId,
}

impl RenderExtractor {
    pub fn new(world: &mut World) -> Self {
        let _ = world.component_init::<TransformComponent>();
        let _ = world.component_init::<CameraComponent>();
        let _ = world.component_init::<Mesh3D>();
        let _ = world.component_init::<Material>();
        let _ = world.component_init::<TerrainSplatMaterial>();
        let _ = world.component_init::<PointLight>();
        let lightmap_id = world.component_init::<LightmapId>();
        let char_id = world.component_init::<CharacterController>();
        Self {
            q_cameras: world.query_prepare("TransformComponent, CameraComponent"),
            q_meshes: world.query_prepare("TransformComponent, Mesh3D, Material"),
            q_terrain_splat: world
                .query_prepare("TransformComponent, Mesh3D, TerrainSplatMaterial"),
            q_lights: world.query_prepare("TransformComponent, PointLight"),
            lightmap_id,
            char_id,
        }
    }

    pub fn extract(&self, world: &World) -> RenderScene {
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
                    // Origin = feet for pawns: eye_height on camera, else CC default.
                    let mut eye = c.eye_height;
                    if eye <= 0.0 {
                        if let Some(cc) =
                            world.get::<CharacterController>(entities[i], self.char_id)
                        {
                            eye = cc.eye_height();
                        }
                    }
                    scene.push_camera(CameraDefinition {
                        position: [t.position.x, t.position.y, t.position.z + eye],
                        rotation: t.rotation,
                        fov_y: c.fov_y,
                        near: c.near,
                        far: c.far,
                        target: RenderTarget::Surface,
                        dirty: true,
                    });
                }
            }
        }

        if let Some(batches) = world.query_batches(self.q_meshes) {
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
                let materials = unsafe {
                    std::slice::from_raw_parts(batch.columns[2] as *const Material, batch.count)
                };
                for i in 0..batch.count {
                    let t = &transforms[i];
                    let m = &meshes[i];
                    let mat = &materials[i];
                    let ent = entities[i];
                    let model_mat = glam::Mat4::from_scale_rotation_translation(
                        t.scale, t.rotation, t.position,
                    )
                    .to_cols_array_2d();
                    let mesh = m
                        .mesh
                        .clone()
                        .unwrap_or_else(|| Arc::new(Mesh::cube()));

                    // LightmapId → BSP atlas + LM pipeline (id 0 = white LM fallback).
                    let lightmap = {
                        let ptr = world.get_raw(ent, self.lightmap_id) as *const LightmapId;
                        if ptr.is_null() {
                            None
                        } else {
                            Some(unsafe { (*ptr).id })
                        }
                    };

                    scene.geometry.mesh_draws.push(MeshDraw {
                        mesh,
                        model_mat,
                        material_id: 0,
                        texture: mat.albedo,
                        normal: mat.normal,
                        orm: mat.ao_roughness_metallic,
                        sun_dir: mat.sun_dir,
                        ambient: mat.ambient,
                        material: Some(mat.clone()),
                        lightmap,
                        first_index: 0,
                        index_count: 0,
                    });
                }
            }
        }

        if let Some(batches) = world.query_batches(self.q_terrain_splat) {
            for batch in batches {
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
                for ((t, m), s) in transforms.iter().zip(meshes.iter()).zip(splats.iter()) {
                    let model_mat = glam::Mat4::from_scale_rotation_translation(
                        t.scale, t.rotation, t.position,
                    );
                    let mesh = m
                        .mesh
                        .clone()
                        .unwrap_or_else(|| Arc::new(Mesh::cube()));
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
