//! Terrain splat is not part of the runtime frame.

use crate::frame::context::FrameCtx;
use crate::gpu::Renderer;

impl Renderer {
    pub(crate) fn pass_terrain_splat(
        &mut self,
        _ctx: &mut FrameCtx<'_>,
        _encoder: &mut wgpu::CommandEncoder,
    ) {
    }
}
