//! Which forward-graph nodes run for a given frame phase.
//!
//! Replaces the old `FrameGraphScope::{Primary,Stack,Full}` split. A camera now
//! declares its own `pass_mask` (which passes it runs); the graph runs exactly
//! those nodes for it. HUD + tonemap run once per render target after all cameras
//! (`Phase::Post`).

use deerpunk_engine_common::components::cam_pass;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// A single camera's passes. Only nodes whose bit is set in `pass_mask` run.
    Camera { pass_mask: u32 },
    /// Once per render target after all cameras: HUD + tonemap + UI.
    Post,
}

pub fn node_enabled(name: &str, phase: Phase) -> bool {
    match phase {
        Phase::Camera { pass_mask } => match name {
            "upload_prep" => true,
            "sky" => pass_mask & cam_pass::SKY != 0,
            // clear_hdr is the no-sky fallback; runs only when this camera clears color
            // and sky did not run. The executor re-checks sky availability.
            "clear_hdr" => true,
            "sprites_3d" => pass_mask & cam_pass::SPRITES_3D != 0,
            "mesh" => pass_mask & cam_pass::MESH != 0,
            "terrain_splat" => pass_mask & cam_pass::TERRAIN != 0,
            "debug" => pass_mask & cam_pass::DEBUG != 0,
            _ => false,
        },
        Phase::Post => matches!(name, "hud" | "luminance" | "tonemap_ui"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_runs_only_masked_passes() {
        let world = Phase::Camera { pass_mask: cam_pass::DEFAULT_WORLD };
        assert!(node_enabled("sky", world));
        assert!(node_enabled("mesh", world));
        assert!(node_enabled("terrain_splat", world));
        assert!(node_enabled("debug", world));
        assert!(!node_enabled("hud", world));
        assert!(!node_enabled("tonemap_ui", world));
    }

    #[test]
    fn overlay_runs_mesh_only() {
        let overlay = Phase::Camera { pass_mask: cam_pass::DEFAULT_OVERLAY };
        assert!(node_enabled("mesh", overlay));
        assert!(!node_enabled("sky", overlay));
        assert!(!node_enabled("terrain_splat", overlay));
        assert!(!node_enabled("debug", overlay));
        assert!(!node_enabled("hud", overlay));
    }

    #[test]
    fn post_runs_ui_pipeline() {
        assert!(node_enabled("tonemap_ui", Phase::Post));
        assert!(node_enabled("hud", Phase::Post));
        assert!(!node_enabled("mesh", Phase::Post));
    }
}
