//! Which forward-graph passes run for a given frame phase.
//!
//! Replaces the old `FrameGraphScope::{Primary,Stack,Full}` split. A camera now
//! declares its own `pass_mask` (which passes it runs); the graph runs exactly
//! those passes for it. HUD + tonemap run once per render target after all
//! cameras (`Phase::Post`).

use crate::render_graph::ForwardPass;
use deerpunk_engine_common::components::cam_pass;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// A single camera's passes. Only passes whose bit is set in `pass_mask` run.
    Camera { pass_mask: u32 },
    /// Once per render target after all cameras: HUD + tonemap + UI.
    Post,
}

pub fn node_enabled(pass: ForwardPass, phase: Phase) -> bool {
    match phase {
        Phase::Camera { pass_mask } => match pass {
            ForwardPass::UploadPrep => true,
            ForwardPass::Sky => pass_mask & cam_pass::SKY != 0,
            // clear_hdr is the no-sky fallback; runs only when this camera clears color
            // and sky did not run. The executor re-checks sky availability.
            ForwardPass::ClearHdr => true,
            ForwardPass::Sprites3d => pass_mask & cam_pass::SPRITES_3D != 0,
            ForwardPass::Mesh => pass_mask & cam_pass::MESH != 0,
            // Additive GPU billboards over the opaque scene: same world pass as
            // meshes (the old particles were mesh_transparent queue draws).
            ForwardPass::Particles => pass_mask & cam_pass::MESH != 0,
            ForwardPass::Debug => pass_mask & cam_pass::DEBUG != 0,
            ForwardPass::Hud | ForwardPass::TonemapUi | ForwardPass::PostFx => false,
        },
        Phase::Post => matches!(
            pass,
            ForwardPass::Hud | ForwardPass::TonemapUi | ForwardPass::PostFx
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_runs_only_masked_passes() {
        let world = Phase::Camera {
            pass_mask: cam_pass::DEFAULT_WORLD,
        };
        assert!(node_enabled(ForwardPass::Sky, world));
        assert!(node_enabled(ForwardPass::Mesh, world));
        assert!(node_enabled(ForwardPass::Debug, world));
        assert!(!node_enabled(ForwardPass::Hud, world));
        assert!(!node_enabled(ForwardPass::TonemapUi, world));
    }

    #[test]
    fn overlay_runs_mesh_only() {
        let overlay = Phase::Camera {
            pass_mask: cam_pass::DEFAULT_OVERLAY,
        };
        assert!(node_enabled(ForwardPass::Mesh, overlay));
        assert!(!node_enabled(ForwardPass::Sky, overlay));
        assert!(!node_enabled(ForwardPass::Debug, overlay));
        assert!(!node_enabled(ForwardPass::Hud, overlay));
    }

    #[test]
    fn post_runs_ui_pipeline() {
        assert!(node_enabled(ForwardPass::TonemapUi, Phase::Post));
        assert!(node_enabled(ForwardPass::Hud, Phase::Post));
        assert!(!node_enabled(ForwardPass::Mesh, Phase::Post));
    }
}
