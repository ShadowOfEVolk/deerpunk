//! Sky [`RenderScene`] assembly (gamelib → `SkyDraw`). GPU pass: [`crate::gpu::passes::sky`].

use deerpunk_engine_renderer::snapshot::{RenderScene, SkyDraw};

/// In/out buffer for optional `client_compose_sky` in the gamelib DLL.
#[repr(C)]
pub struct SkySceneCompose {
    pub camera_pos: [f32; 3],
    pub scene_sun: [f32; 4],
    pub ambient: [f32; 4],
    pub zenith_default: [f32; 3],
    /// Set to 1 by gamelib when `geometry.sky` should be filled.
    pub wrote: u8,
    pub clear: [f32; 4],
    pub sun_dir: [f32; 3],
    pub params: [f32; 32],
    pub sky_color: [f32; 3],
    pub zenith: [f32; 3],
}

pub type ClientComposeSkyFn = unsafe extern "C" fn(ctx: *mut u8, io: *mut SkySceneCompose);

pub fn smoothstep_sky(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge0 >= edge1 {
        return if x >= edge1 { 1.0 } else { 0.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn apply_compose(scene: &mut RenderScene, io: &SkySceneCompose) {
    if io.wrote == 0 {
        return;
    }
    scene.geometry.sky = Some(SkyDraw {
        scene_sun: io.scene_sun,
        sun_dir: io.sun_dir,
        camera_pos: io.camera_pos,
        params: io.params,
        sky_color: io.sky_color,
        zenith: io.zenith,
    });
    scene.clear = io.clear;
}

/// Default engine path when gamelib does not export `client_compose_sky`.
pub fn fill_procedural_sky_default(
    scene: &mut RenderScene,
    camera_pos: [f32; 3],
    scene_sun: [f32; 4],
    ambient: [f32; 4],
    zenith_default: [f32; 3],
    gamelib_params: Option<[f32; 32]>,
    gamelib_horizon: Option<[f32; 3]>,
    gamelib_zenith: Option<[f32; 3]>,
    sky_debug_log: bool,
) {
    let sun = glam::Vec3::new(scene_sun[0], scene_sun[1], scene_sun[2]).normalize_or(glam::Vec3::Z);
    let mut params = gamelib_params.unwrap_or([0.0; 32]);
    let horizon = gamelib_horizon.unwrap_or([ambient[0], ambient[1], ambient[2]]);
    let zenith = gamelib_zenith.unwrap_or(zenith_default);
    if params[31] <= 0.0 {
        let z = scene_sun[2].clamp(-1.0, 1.0);
        params[31] = smoothstep_sky(-0.35, 0.12, z);
    }
    let mut io = SkySceneCompose {
        camera_pos,
        scene_sun,
        ambient,
        zenith_default,
        wrote: 1,
        clear: [0.0; 4],
        sun_dir: sun.to_array(),
        params,
        sky_color: horizon,
        zenith,
    };
    let display = horizon.map(|c| c.clamp(0.0, 1.0).powf(1.0 / 2.2));
    io.clear = [display[0], display[1], display[2], 1.0];
    if sky_debug_log {
        println!(
            "[sky:extract] sun=[{:.4}, {:.4}, {:.4}, {:.4}] horizon=[{:.4}, {:.4}, {:.4}] zenith=[{:.4}, {:.4}, {:.4}] glow={:.3} day_fade={:.3}",
            scene_sun[0],
            scene_sun[1],
            scene_sun[2],
            scene_sun[3],
            horizon[0],
            horizon[1],
            horizon[2],
            zenith[0],
            zenith[1],
            zenith[2],
            ambient[3],
            params[31],
        );
    }
    apply_compose(scene, &io);
}
