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
            ForwardPass::Debug => "debug",
            ForwardPass::Hud => "hud",
            ForwardPass::TonemapUi => "tonemap_ui",
        }
    }

    /// Static `(reads, writes)` target names. Display metadata for `r_speeds`
    /// only — execution takes its views from `FrameCtx` directly.
    pub fn io(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            ForwardPass::UploadPrep => (&[], &[]),
            ForwardPass::Sky => (&[], &["hdr_color"]),
            ForwardPass::ClearHdr => (&[], &["hdr_color"]),
            ForwardPass::Sprites3d => (&["hdr_color"], &["hdr_color"]),
            ForwardPass::Mesh => (&["hdr_color"], &["hdr_color"]),
            ForwardPass::Debug => (&["hdr_color"], &["hdr_color"]),
            ForwardPass::Hud => (&["hdr_color"], &["hdr_color"]),
            ForwardPass::TonemapUi => (&["hdr_color"], &["surface"]),
        }
    }
}

/// The forward frame (HDR → … → surface), in execution order. Static: every
/// pass reads frame state through `FrameCtx`, so there is nothing to build or
/// mutate per frame.
pub const FORWARD_PASSES: [ForwardPass; 8] = [
    ForwardPass::UploadPrep,
    ForwardPass::Sky,
    ForwardPass::ClearHdr,
    ForwardPass::Sprites3d,
    ForwardPass::Mesh,
    ForwardPass::Debug,
    ForwardPass::Hud,
    ForwardPass::TonemapUi,
];

/// Static pass structure for `r_speeds` / inspector: `name -> (reads, writes)`.
pub fn describe_nodes() -> Vec<(&'static str, Vec<&'static str>, Vec<&'static str>)> {
    FORWARD_PASSES
        .iter()
        .map(|p| {
            let (reads, writes) = p.io();
            (p.name(), reads.to_vec(), writes.to_vec())
        })
        .collect()
}
