//! [`r_loggpu`] — log mesh/material GPU uploads during extract + encode.

use deerpunk_engine_renderer::RenderFrameProfile;

use deerpunk_engine_common::cvar::CvarSystem;

#[inline]
fn level(cvar: &CvarSystem) -> usize {
    cvar.get_usize_or("r_loggpu", 0)
}

/// Call once per presented frame after the renderer has encoded the scene.
pub fn log_frame(cvar: &CvarSystem, profile: &RenderFrameProfile) {
    let lvl = level(cvar);
    if lvl == 0 {
        return;
    }

    let eg = &profile.extract_gpu;
    if lvl == 1 {
        let any_upload = eg.material_uploads > 0 || profile.mesh_gpu_uploads > 0;
        if !any_upload {
            return;
        }
    }

    let mesh_node_ms = profile.graph_node_ms("mesh");
    log::info!(
        "[render] gpu extract: mat up={} hit={} draws={}+{} terr={} resolve {:.2} ms (scene extract {:.2} ms) | mesh up={} hit={} mesh node {:.2} ms",
        eg.material_uploads,
        eg.material_cached,
        eg.mesh_draws,
        eg.skinned_draws,
        eg.terrain_draws,
        eg.resolve_ms,
        profile.extract_ms,
        profile.mesh_gpu_uploads,
        profile.mesh_gpu_cached,
        mesh_node_ms,
    );
}
