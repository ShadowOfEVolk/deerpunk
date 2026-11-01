use glam::{Mat4, Quat, Vec3};

use crate::aabb::Aabb;
use crate::frustum::{frustum_world_aabb, Frustum};
use crate::snapshot::CameraDefinition;

fn deerpunk_camera(eye: Vec3, yaw: f32) -> (Mat4, Mat4) {
    let cam = CameraDefinition {
        position: eye.to_array(),
        rotation: Quat::from_rotation_z(-yaw),
        fov_y: 70.0_f32.to_radians(),
        near: 0.1,
        far: 512.0,
        target: crate::snapshot::RenderTarget::Surface,
        layer_mask: deerpunk_engine_common::components::render_layer::RENDER_LAYER_WORLD,
        order: 0,
        pass_mask: crate::snapshot::cam_features::WORLD_DEFAULT,
        color_load: deerpunk_engine_common::components::ColorLoad::Clear,
        depth: deerpunk_engine_common::components::DepthAttachment::owned(deerpunk_engine_common::components::DepthTargetId(0)),
        dirty: true,
    };
    let aspect = 16.0 / 9.0;
    let view = cam.view_matrix();
    let proj = cam.projection_matrix(aspect);
    (proj * view, view)
}

#[test]
fn chunk_ahead_of_camera_not_culled() {
    let eye = Vec3::new(8.0, -20.0, 10.0);
    let (vp, _) = deerpunk_camera(eye, 0.0);
    let frustum = Frustum::from_view_proj(vp);
    // Chunk at world origin, 16³ mesh local [0,16]³.
    let chunk = Aabb::from_min_max(Vec3::ZERO, Vec3::splat(16.0));
    assert!(
        frustum.intersects_aabb(&chunk),
        "chunk in front of camera should pass cull"
    );
}

#[test]
fn chunk_behind_camera_culled() {
    let eye = Vec3::new(8.0, 20.0, 10.0);
    let (vp, _) = deerpunk_camera(eye, 0.0);
    let frustum = Frustum::from_view_proj(vp);
    let chunk = Aabb::from_min_max(Vec3::ZERO, Vec3::splat(16.0));
    assert!(
        !frustum.intersects_aabb(&chunk),
        "chunk behind camera should be culled"
    );
}

#[test]
fn frustum_broad_phase_covers_visible_chunk() {
    let eye = Vec3::new(8.0, -20.0, 10.0);
    let aspect = 16.0 / 9.0;
    let cam = CameraDefinition {
        position: eye.to_array(),
        rotation: Quat::from_rotation_z(0.0),
        fov_y: 70.0_f32.to_radians(),
        near: 0.1,
        far: 512.0,
        target: crate::snapshot::RenderTarget::Surface,
        layer_mask: deerpunk_engine_common::components::render_layer::RENDER_LAYER_WORLD,
        order: 0,
        pass_mask: crate::snapshot::cam_features::WORLD_DEFAULT,
        color_load: deerpunk_engine_common::components::ColorLoad::Clear,
        depth: deerpunk_engine_common::components::DepthAttachment::owned(deerpunk_engine_common::components::DepthTargetId(0)),
        dirty: true,
    };
    let view = cam.view_matrix();
    let broad = frustum_world_aabb(view, cam.near, cam.far, cam.fov_y, aspect);
    let chunk = Aabb::from_min_max(Vec3::ZERO, Vec3::splat(16.0));
    assert!(broad.contains_aabb(&chunk));
}
