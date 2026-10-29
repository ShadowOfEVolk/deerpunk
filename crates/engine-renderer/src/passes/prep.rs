//! Auto-split from `encode_frame` (frame graph pass).

use crate::frame::context::FrameCtx;
use crate::gpu::Renderer;
use crate::render_profile::lap_ms;

impl Renderer {
    pub(crate) fn pass_upload_prep(
        &mut self,
        ctx: &mut FrameCtx<'_>,
    ) {
    self.upload_sprites(ctx.snapshot);
        let _ = lap_ms(ctx.lap);
    self.material_allocator.flush(&self.queue);
    if self.material_allocator.generation != self.material_bg_gen {
        self.material_bg_gen = self.material_allocator.generation;
        self.invalidate_material_bgs();
    }
        let _ = lap_ms(ctx.lap);
    }
}
