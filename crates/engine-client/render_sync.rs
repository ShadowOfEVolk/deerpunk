//! Apply host cvars to the client renderer + debug overlays before encode.

use deerpunk_engine_common::cvar::CvarSystem;
use deerpunk_engine_common::physics::PhysicsBackend;
use deerpunk_engine_common::resource::mesh_registry::MeshRegistry;
use deerpunk_engine_common::world::World;
use deerpunk_engine_renderer::snapshot::CullOptions;
use deerpunk_engine_renderer::DEFAULT_CELL_SIZE;

use crate::client_engine::{ClientEngine, DebugDrawBuf};
use crate::{entity_debug, phys_debug};

pub fn frustum_cull_options(cvar: &CvarSystem) -> CullOptions {
    CullOptions {
        enabled: cvar.get_bool_or("r_frustum_cull", true),
        cell_size: cvar.get_f32_or("r_cull_cell_size", DEFAULT_CELL_SIZE).max(1.0),
    }
}

pub fn apply_renderer_cvars(cvar: &CvarSystem, client: &mut ClientEngine) {
    let r = client.renderer_mut();
    r.set_fullbright(cvar.get_bool_or("r_fullbright", false));
    r.set_vsync(cvar.get_bool_or("r_vsync", true));
    r.set_lightmap_only(cvar.get_bool_or("r_lightmap", false));
    let e = cvar.get_f32_or("r_exposure", 1.0);
    r.set_exposure_override(if e < 0.0 { None } else { Some(e) });
    r.set_tonemap_mode(cvar.get_f32_or("r_tonemap", -1.0));
    r.set_bloom(
        cvar.get_f32_or("r_bloom", 1.0) >= 0.5,
        cvar.get_f32_or("r_bloom_threshold", 0.7),
        cvar.get_f32_or("r_bloom_intensity", 1.0),
    );
    r.set_bloom_downscale(cvar.get_f32_or("r_bloom_downscale", 2.0).round() as u32);
    r.set_grain(
        cvar.get_f32_or("r_grain", 1.0) >= 0.5,
        cvar.get_f32_or("r_grain_strength", 0.025),
        cvar.get_f32_or("r_grain_downscale", 3.0).clamp(1.0, 8.0),
    );
    r.set_cloud_speed(cvar.get_f32_or("r_cloud_speed", 0.04));
    r.set_cloud_mode(cvar.get_f32_or("r_cloud_mode", 0.0).round() as u32);
    r.set_sky_turbidity(cvar.get_f32_or("r_sky_turbidity", 3.0));
    let a = cvar.get_f32_or("r_sky_albedo", 0.25).clamp(0.0, 1.0);
    r.set_sky_albedo([a, a, a]);
    r.set_sky_debug_log(cvar.get_f32_or("r_sky_debug", 0.0) >= 0.5);
    r.set_occlusion_cull(cvar.get_bool_or("r_occlusion_cull", false));
}

pub fn build_debug_draw(
    cvar: &CvarSystem,
    client_world: &World,
    server_world: &World,
    physics: &dyn PhysicsBackend,
    meshes: &MeshRegistry,
) -> DebugDrawBuf {
    let mut draw = DebugDrawBuf::new();
    phys_debug::tick(cvar, client_world, server_world, physics, &mut draw);
    entity_debug::tick(cvar, meshes, client_world, &mut draw);
    draw
}
