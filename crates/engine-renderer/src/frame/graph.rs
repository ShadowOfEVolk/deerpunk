//! Default forward [`RenderGraph`] (HDR → … → surface).

use crate::frame::schedule::res;
use crate::render_graph::{Node, NodeContext, NodeExecute, RenderGraph, Resource, ResId};

fn forward_node(name: &'static str, inputs: &[ResId], outputs: &[ResId]) -> Node {
    let execute: NodeExecute = Box::new(|nc: &mut NodeContext<'_>| {
        crate::frame::executor::dispatch_forward_node(nc);
    });
    Node {
        name,
        inputs: inputs.to_vec(),
        outputs: outputs.to_vec(),
        execute,
    }
}

/// Built once on [`crate::gpu::Renderer`] init. Passes bind via [`super::executor::GraphFrameUserData`].
pub fn build_forward_graph() -> RenderGraph {
    use res::{DEPTH, HDR, SURFACE};
    RenderGraph::new(HDR, SURFACE)
        .add_resource(HDR, Resource::External)
        .add_resource(DEPTH, Resource::External)
        .add_resource(SURFACE, Resource::External)
        .add_node(forward_node("upload_prep", &[], &[]))
        .add_node(forward_node("sky", &[], &[HDR]))
        .add_node(forward_node("clear_hdr", &[], &[HDR]))
        .add_node(forward_node("sprites_3d", &[HDR], &[HDR]))
        .add_node(forward_node("mesh", &[HDR], &[HDR, DEPTH]))
        .add_node(forward_node(
            "terrain_splat",
            &[HDR, DEPTH],
            &[HDR, DEPTH],
        ))
        .add_node(forward_node(
            "debug",
            &[HDR, DEPTH],
            &[HDR, DEPTH],
        ))
        .add_node(forward_node("hud", &[HDR], &[HDR]))
        .add_node(forward_node("luminance", &[HDR], &[HDR]))
        .add_node(forward_node("tonemap_ui", &[HDR], &[SURFACE]))
}
