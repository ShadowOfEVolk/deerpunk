//! Graph resource name constants.

pub mod res {
    use crate::render_graph::ResId;

    pub const HDR: ResId = ResId("hdr_color");
    pub const SURFACE: ResId = ResId("surface");
    // Depth is NOT a graph resource: it is owned by the per-camera depth pool
    // (`Renderer::ensure_depth(target_id, w, h)`), keyed by `DepthTargetId`.
    // Cameras sharing a target id share one depth buffer; different ids get their own.
}
