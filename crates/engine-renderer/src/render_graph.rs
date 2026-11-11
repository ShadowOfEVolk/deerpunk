//! Typed forward render graph: the frame's ordered pass list.
//!
//! The graph is a static list of [`ForwardPass`] values executed in declaration
//! order on one frame encoder (see `crate::frame::executor::run_frame_graph`).
//! There is no resource plumbing: depth is owned by the per-camera depth pool
//! (`Renderer::ensure_depth(target_id, w, h)`), and HDR / surface views are read
//! from `FrameCtx` by the passes that need them.

/// One pass in the forward frame. Dispatch is a typed match in
/// `crate::frame::executor`, not a boxed closure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForwardPass {
    UploadPrep,
    Sky,
    ClearHdr,
    Sprites3d,
    Mesh,
    Particles,
    Debug,
    Hud,
    TonemapUi,
}

impl ForwardPass {
    pub fn name(self) -> &'static str {
        match self {
            ForwardPass::UploadPrep => "upload_prep",
            ForwardPass::Sky => "sky",
            ForwardPass::ClearHdr => "clear_hdr",
            ForwardPass::Sprites3d => "sprites_3d",
            ForwardPass::Mesh => "mesh",
            ForwardPass::Particles => "particles",
            ForwardPass::Debug => "debug",
            ForwardPass::Hud => "hud",
            ForwardPass::TonemapUi => "tonemap_ui",
        }
    }
}

/// The forward frame (HDR → … → surface), in execution order. Static: every
/// pass reads frame state through `FrameCtx`, so there is nothing to build or
/// mutate per frame.
pub const FORWARD_PASSES: [ForwardPass; 9] = [
    ForwardPass::UploadPrep,
    ForwardPass::Sky,
    ForwardPass::ClearHdr,
    ForwardPass::Sprites3d,
    ForwardPass::Mesh,
    ForwardPass::Particles,
    ForwardPass::Debug,
    ForwardPass::Hud,
    ForwardPass::TonemapUi,
];
