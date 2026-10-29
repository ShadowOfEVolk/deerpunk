//! Per-frame encode context (snapshot + profiling + draw counters).

use wgpu::TextureFormat;

use crate::render_profile::{FrameDrawAccum, RenderFrameProfile};
use crate::shader_registry::ShaderRegistry;
use crate::snapshot::RenderSnapshot;

/// Borrowed inputs for one `encode_frame` invocation.
pub struct FrameCtx<'a> {
    pub snapshot: &'a RenderSnapshot,
    pub hdr_view: &'a wgpu::TextureView,
    /// HDR intermediate (`Rgba16Float`) — mesh / sky / sprites.
    pub hdr_format: TextureFormat,
    /// Swapchain / tonemap + UI target.
    pub surface_format: TextureFormat,
    pub surface_view: Option<&'a wgpu::TextureView>,
    pub shaders: &'a ShaderRegistry,
    pub prof: &'a mut RenderFrameProfile,
    pub draw_acc: &'a mut FrameDrawAccum,
    pub lap: &'a mut std::time::Instant,
}
