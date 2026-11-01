use std::sync::Arc;

use glam::{Quat, Vec3};

use deerpunk_engine_common::components::render_layer::RENDER_LAYER_WORLD;
use deerpunk_engine_common::components::{
    ColorLoad, DepthAttachment, DepthLoad, DepthTargetId,
};
use deerpunk_engine_common::resource::mesh::Mesh;

use crate::aabb::Aabb;
use crate::cull::CullScratch;
use crate::snapshot::{
    cam_features, CameraDefinition, MeshDraw, RenderScene, RenderTarget, SceneGeometry, SkyDraw,
};

/// Overlay layer bit (game-defined convention; bit 1).
const OVERLAY_LAYER: u32 = 1 << 1;
const OVERLAY_DEPTH: DepthTargetId = DepthTargetId(1);

fn test_scene_two_cameras() -> RenderScene {
    let chunk = Aabb::from_min_max(Vec3::ZERO, Vec3::splat(16.0));
    let hand = Aabb::from_min_max(Vec3::new(-0.2, -0.2, -0.2), Vec3::splat(0.2));

    let mut geo = SceneGeometry::default();
    geo.mesh_draws.push(MeshDraw {
        mesh: Arc::new(Mesh::empty()),
        model_mat: glam::Mat4::IDENTITY.to_cols_array_2d(),
        layer_mask: RENDER_LAYER_WORLD,
        batchable: false,
        material_id: 1,
        texture: crate::MeshTexture::WHITE,
        normal: crate::MeshTexture::WHITE,
        orm: crate::MeshTexture::WHITE,
        sun_dir: [0.0, 0.0, 1.0, 0.0],
        ambient: [1.0; 4],
        shader_id: 1,
        lightmap_gpu: 0,
        first_index: 0,
        index_count: 0,
        world_bounds: chunk,
        entity_id: 1,
        uniform_aux: 0,
    });
    geo.mesh_draws.push(MeshDraw {
        mesh: Arc::new(Mesh::empty()),
        model_mat: glam::Mat4::IDENTITY.to_cols_array_2d(),
        layer_mask: OVERLAY_LAYER,
        batchable: false,
        material_id: 1,
        texture: crate::MeshTexture::WHITE,
        normal: crate::MeshTexture::WHITE,
        orm: crate::MeshTexture::WHITE,
        sun_dir: [0.0, 0.0, 1.0, 0.0],
        ambient: [1.0; 4],
        shader_id: 1,
        lightmap_gpu: 0,
        first_index: 0,
        index_count: 0,
        world_bounds: hand,
        entity_id: 2,
        uniform_aux: 0,
    });
    geo.sky = Some(SkyDraw {
        scene_sun: [0.0, 0.0, 1.0, 1.0],
        sun_dir: [0.0, 0.0, 1.0],
        camera_pos: [0.0; 3],
        params: [0.0; 32],
        sky_color: [0.5, 0.6, 0.7],
        zenith: [0.2, 0.4, 0.8],
    });
    geo.refresh_cull_serial();

    let eye = Vec3::new(8.0, -20.0, 10.0);
    let mut scene = RenderScene::new();
    scene.cull.enabled = true;
    scene.push_camera(CameraDefinition {
        position: eye.to_array(),
        rotation: Quat::IDENTITY,
        fov_y: 70.0_f32.to_radians(),
        near: 0.1,
        far: 512.0,
        target: RenderTarget::Surface,
        layer_mask: RENDER_LAYER_WORLD,
        order: 0,
        pass_mask: cam_features::WORLD_DEFAULT,
        color_load: ColorLoad::Clear,
        depth: DepthAttachment::owned(DepthTargetId(0)),
        dirty: true,
    });
    scene.push_camera(CameraDefinition {
        position: eye.to_array(),
        rotation: Quat::IDENTITY,
        fov_y: 70.0_f32.to_radians(),
        near: 0.02,
        far: 48.0,
        target: RenderTarget::Surface,
        layer_mask: OVERLAY_LAYER,
        order: 1,
        pass_mask: cam_features::DEFAULT_OVERLAY,
        color_load: ColorLoad::Load,
        depth: DepthAttachment::owned(OVERLAY_DEPTH),
        dirty: true,
    });
    scene.set_geometry(geo);
    scene
}

#[test]
fn build_snapshots_uses_frustum_for_all_cameras() {
    let scene = test_scene_two_cameras();
    let mut scratch = CullScratch::new();
    let snaps = scene.build_snapshots(16.0 / 9.0, &mut scratch);
    assert_eq!(snaps.len(), 2);
    assert!(snaps.iter().all(|s| s.frustum_culled));
}

#[test]
fn layer_mask_splits_world_and_stack_draws() {
    let scene = test_scene_two_cameras();
    let mut scratch = CullScratch::new();
    let snaps = scene.build_snapshots(16.0 / 9.0, &mut scratch);
    assert_eq!(snaps[0].visible_mesh, vec![0]);
    assert_eq!(snaps[1].visible_mesh, vec![1]);
}

#[test]
fn gather_visible_draws_filters_by_camera_layer() {
    let scene = test_scene_two_cameras();
    let mut scratch = CullScratch::new();
    let snaps = scene.build_snapshots(16.0 / 9.0, &mut scratch);
    let mut mesh = Vec::new();
    let mut skinned = Vec::new();
    scene.gather_visible_draws(&snaps[0], &mut mesh, &mut skinned);
    assert_eq!(mesh.len(), 1);
    assert_eq!(mesh[0].layer_mask, RENDER_LAYER_WORLD);
    mesh.clear();
    scene.gather_visible_draws(&snaps[1], &mut mesh, &mut skinned);
    assert_eq!(mesh.len(), 1);
    assert_eq!(mesh[0].layer_mask, OVERLAY_LAYER);
}

#[test]
fn camera_definition_carries_order_and_features() {
    let scene = test_scene_two_cameras();
    assert_eq!(scene.cameras[0].order, 0);
    assert_eq!(scene.cameras[1].order, 1);
    assert_eq!(scene.cameras[0].pass_mask, cam_features::WORLD_DEFAULT);
    assert_eq!(scene.cameras[1].pass_mask, cam_features::DEFAULT_OVERLAY);
}

#[test]
fn behind_camera_world_mesh_culled_even_with_matching_layer() {
    let mut scene = test_scene_two_cameras();
    scene.cameras[0].position = [8.0, 20.0, 10.0];
    let mut scratch = CullScratch::new();
    let snaps = scene.build_snapshots(16.0 / 9.0, &mut scratch);
    assert!(snaps[0].visible_mesh.is_empty());
}

#[test]
fn snapshot_camera_pos_matches_definition_not_remapped() {
    let scene = test_scene_two_cameras();
    let mut scratch = CullScratch::new();
    let snaps = scene.build_snapshots(16.0 / 9.0, &mut scratch);
    assert_eq!(snaps[0].camera_pos, scene.cameras[0].position);
    assert_eq!(snaps[1].camera_pos, scene.cameras[1].position);
}

#[test]
fn overlay_camera_has_own_depth_and_loads_color() {
    // Regression for the z-fight root cause: the overlay (viewmodel) camera must
    // (a) keep the world HDR color (Load) so it draws on top, and
    // (b) own a separate depth target so its different near plane never corrupts
    // the world depth buffer. The world camera clears color and owns the world depth.
    let scene = test_scene_two_cameras();
    let mut scratch = CullScratch::new();
    let snaps = scene.build_snapshots(16.0 / 9.0, &mut scratch);

    // World camera (order 0).
    assert_eq!(snaps[0].color_load, ColorLoad::Clear);
    assert_eq!(snaps[0].depth.target, DepthTargetId(0));
    assert_eq!(snaps[0].depth.load, DepthLoad::Clear(1.0));
    assert_eq!(snaps[0].pass_mask, cam_features::WORLD_DEFAULT);

    // Overlay camera (order 1): own depth, loads color, mesh-only.
    assert_eq!(snaps[1].color_load, ColorLoad::Load);
    assert_eq!(snaps[1].depth.target, OVERLAY_DEPTH);
    assert_ne!(snaps[1].depth.target, snaps[0].depth.target);
    assert_eq!(snaps[1].depth.load, DepthLoad::Clear(1.0));
    assert_eq!(snaps[1].pass_mask, cam_features::DEFAULT_OVERLAY);
}
