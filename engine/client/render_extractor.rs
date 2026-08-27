use deerpunk_engine_common::components::{CameraComponent, MeshRenderer, TransformComponent};
use deerpunk_engine_common::resource::mesh::Mesh;
use deerpunk_engine_common::world::{QueryId, World};
use deerpunk_engine_renderer::snapshot::{CameraDefinition, MeshDraw, RenderScene, RenderTarget};

pub struct RenderExtractor {
    q_cameras: QueryId,
    q_meshes: QueryId,
}

impl RenderExtractor {
    pub fn new(world: &mut World) -> Self {
        Self {
            q_cameras: world.query_prepare("TransformComponent, CameraComponent"),
            q_meshes: world.query_prepare("TransformComponent, MeshRenderer"),
        }
    }

    pub fn extract(&self, world: &World) -> RenderScene {
        let mut scene = RenderScene::new();

        if let Some(batches) = world.query_batches(self.q_cameras) {
            for batch in batches {
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
                for (t, c) in transforms.iter().zip(cameras.iter()) {
                    scene.push_camera(CameraDefinition {
                        position: [t.position.x, t.position.y, t.position.z],
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
                let transforms = unsafe {
                    std::slice::from_raw_parts(
                        batch.columns[0] as *const TransformComponent,
                        batch.count,
                    )
                };
                let meshes = unsafe {
                    std::slice::from_raw_parts(batch.columns[1] as *const MeshRenderer, batch.count)
                };
                for (t, m) in transforms.iter().zip(meshes.iter()) {
                    let model_mat = glam::Mat4::from_scale_rotation_translation(
                        t.scale, t.rotation, t.position,
                    );
                    scene.geometry.mesh_draws.push(MeshDraw {
                        mesh: m.mesh.clone().unwrap_or_else(Mesh::cube),
                        model_mat: model_mat.to_cols_array_2d(),
                        color: m.color,
                        texture: m.texture,
                        sun_dir: m.sun_dir,
                        ambient: m.ambient,
                    });
                }
            }
        }

        scene
    }
}
