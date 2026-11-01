//! CPU-side camera stack: cull, gather, compose packing.

use deerpunk_engine_renderer::snapshot::{
    cam_features, CameraRenderSnapshot, RenderScene, RenderSnapshot, SkyDraw,
};

/// Build one surface [`RenderSnapshot`] from a culled [`CameraRenderSnapshot`].
pub fn pack_surface_camera(
    scene: &RenderScene,
    cam: &CameraRenderSnapshot,
    out: &mut RenderSnapshot,
) {
    out.clear = cam.clear;
    out.color_load = cam.color_load;
    out.depth = cam.depth;
    out.view_proj = cam.view_proj.to_cols_array_2d();
    out.inv_view_proj = cam.inv_view_proj.to_cols_array_2d();
    out.inv_view_proj_sky = cam.inv_view_proj_sky.to_cols_array_2d();
    out.camera_pos = cam.camera_pos;
    out.game_time = cam.game_time;
    out.pass_mask = cam.pass_mask;
    out.sky = if cam.pass_mask & cam_features::SKY != 0 {
        cam.sky
    } else {
        None
    };
    out.frustum_culled = cam.frustum_culled;
    out.cull_stats = cam.cull_stats;
    out.bone_palettes.clone_from(&cam.bone_palettes);
    out.point_lights.clone_from(&cam.point_lights);
    out.instances.clear();
    out.hud.clear();
    out.mesh_draws.clear();
    out.skinned_mesh_draws.clear();
    out.terrain_splat_draws.clear();
    out.storm = None;
    scene.gather_visible_draws(cam, &mut out.mesh_draws, &mut out.skinned_mesh_draws);
    if cam.pass_mask & cam_features::TERRAIN != 0 {
        out.terrain_splat_draws
            .clone_from(&scene.geometry.terrain_splat_draws);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deerpunk_engine_common::components::{ColorLoad, DepthAttachment, DepthTargetId};
    use deerpunk_engine_renderer::snapshot::{
        CameraDefinition, CullOptions, RenderTarget, SceneGeometry,
    };
    use deerpunk_engine_renderer::{Aabb, CullScratch};
    use glam::{Quat, Vec3};

    /// Overlay layer bit (game-defined convention; bit 1). The engine only knows bit 0
    /// as the default — games pick the rest.
    const OVERLAY_LAYER: u32 = 1 << 1;
    const OVERLAY_DEPTH: DepthTargetId = DepthTargetId(1);

    fn scene_with_two_layered_draws() -> RenderScene {
        use deerpunk_engine_common::components::render_layer::RENDER_LAYER_WORLD;
        use deerpunk_engine_common::resource::mesh::Mesh;
        use deerpunk_engine_renderer::snapshot::SkyDraw;
        use deerpunk_engine_renderer::snapshot::MeshDraw;
        use std::sync::Arc;

        let chunk = Aabb::from_min_max(Vec3::ZERO, Vec3::splat(16.0));
        let hand = Aabb::from_min_max(Vec3::new(-0.2, -0.2, -0.2), Vec3::splat(0.2));

        let mut geo = SceneGeometry::default();
        geo.mesh_draws.push(MeshDraw {
            mesh: Arc::new(Mesh::empty()),
            model_mat: glam::Mat4::IDENTITY.to_cols_array_2d(),
            layer_mask: RENDER_LAYER_WORLD,
            batchable: false,
            material_id: 1,
            texture: deerpunk_engine_renderer::MeshTexture::WHITE,
            normal: deerpunk_engine_renderer::MeshTexture::WHITE,
            orm: deerpunk_engine_renderer::MeshTexture::WHITE,
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
            texture: deerpunk_engine_renderer::MeshTexture::WHITE,
            normal: deerpunk_engine_renderer::MeshTexture::WHITE,
            orm: deerpunk_engine_renderer::MeshTexture::WHITE,
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
        scene.cull = CullOptions {
            enabled: true,
            ..CullOptions::default()
        };
        scene.push_camera(CameraDefinition {
            position: eye.to_array(),
            rotation: Quat::from_rotation_z(0.0),
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
            rotation: Quat::from_rotation_z(0.0),
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
    fn build_snapshots_frustum_and_layer_per_camera() {
        let scene = scene_with_two_layered_draws();
        let mut scratch = CullScratch::new();
        let snaps = scene.build_snapshots(16.0 / 9.0, &mut scratch);
        assert_eq!(snaps.len(), 2);
        assert!(snaps[0].frustum_culled);
        assert!(snaps[1].frustum_culled);
        assert_eq!(snaps[0].visible_mesh.len(), 1);
        assert_eq!(snaps[1].visible_mesh.len(), 1);
        assert_eq!(snaps[0].layer_mask, scene.cameras[0].layer_mask);
        assert_eq!(snaps[1].order, 1);
    }

    #[test]
    fn gather_and_pack_respect_layer_and_pass_features() {
        let scene = scene_with_two_layered_draws();
        let mut scratch = CullScratch::new();
        let snaps = scene.build_snapshots(16.0 / 9.0, &mut scratch);

        let mut world_out = RenderSnapshot::new(ColorLoad::Clear);
        pack_surface_camera(&scene, &snaps[0], &mut world_out);
        assert_eq!(world_out.mesh_draws.len(), 1);
        assert!(world_out.sky.is_some() || snaps[0].sky.is_some());
        assert_eq!(world_out.pass_mask, cam_features::WORLD_DEFAULT);

        let mut stack_out = RenderSnapshot::new(ColorLoad::Load);
        pack_surface_camera(&scene, &snaps[1], &mut stack_out);
        assert_eq!(stack_out.mesh_draws.len(), 1);
        assert!(stack_out.sky.is_none());
        assert_eq!(stack_out.pass_mask, cam_features::DEFAULT_OVERLAY);
        assert!(stack_out.terrain_splat_draws.is_empty());
    }

    #[test]
    fn snapshot_camera_pos_matches_definition() {
        let scene = scene_with_two_layered_draws();
        let mut scratch = CullScratch::new();
        let snaps = scene.build_snapshots(16.0 / 9.0, &mut scratch);
        assert_eq!(snaps[0].camera_pos, scene.cameras[0].position);
        assert_eq!(snaps[1].camera_pos, scene.cameras[1].position);
    }

    #[test]
    fn pack_propagates_per_camera_depth_and_color_load() {
        // Regression for the z-fight root cause: the overlay camera must pack its
        // own depth target + Load color, while the world camera packs Clear color
        // + the world depth target. Different depth targets => no shared depth
        // buffer => no z-fight between different near planes.
        use deerpunk_engine_common::components::{DepthLoad, DepthTargetId};
        let scene = scene_with_two_layered_draws();
        let mut scratch = CullScratch::new();
        let snaps = scene.build_snapshots(16.0 / 9.0, &mut scratch);

        let mut world_out = RenderSnapshot::new(ColorLoad::Clear);
        pack_surface_camera(&scene, &snaps[0], &mut world_out);
        assert_eq!(world_out.depth.target, DepthTargetId(0));
        assert_eq!(world_out.depth.load, DepthLoad::Clear(1.0));
        assert_eq!(world_out.color_load, ColorLoad::Clear);

        let mut overlay_out = RenderSnapshot::new(ColorLoad::Load);
        pack_surface_camera(&scene, &snaps[1], &mut overlay_out);
        assert_eq!(overlay_out.depth.target, OVERLAY_DEPTH);
        assert_eq!(overlay_out.depth.load, DepthLoad::Clear(1.0));
        assert_eq!(overlay_out.color_load, ColorLoad::Load);
        assert_ne!(overlay_out.depth.target, world_out.depth.target);
    }
}
