//! Auto-split from `encode_frame` (frame graph pass).

use crate::frame::context::FrameCtx;
use crate::gpu::Renderer;
use crate::render_profile::lap_ms;
use deerpunk_engine_common::frame::{frame_height, frame_width};

impl Renderer {
    pub(crate) fn pass_tonemap_and_ui(
        &mut self,
        ctx: &mut FrameCtx<'_>, encoder: &mut wgpu::CommandEncoder,
    ) {
    // --- Bloom → tonemap → grain → surface ---
    if let Some(surface_view) = ctx.surface_view {
        self.ensure_tonemap_pipeline(ctx.surface_format);
        let hdr_view_ref = self.hdr_view.as_ref().expect("HDR texture not initialized");
        let w = frame_width();
        let h = frame_height();
        let exposure = self.exposure;
        let tonemap_mode = self.tonemap_mode;
        let game_time = ctx.snapshot.game_time;
        let pipeline = self.tonemap_pipeline.as_ref().unwrap().clone();
        self.postfx.encode_tonemap_chain(
            &self.device,
            &self.queue,
            encoder,
            hdr_view_ref,
            w,
            h,
            ctx.surface_format,
            surface_view,
            &pipeline,
            &self.tonemap_uniform_buf,
            exposure,
            tonemap_mode,
            game_time,
            Some(ctx.prof),
        );
        lap_ms(ctx.lap);

        // UI overlay (RmlUi geometry) on LDR surface
        if let Some(ui) = self.pending_ui.take() {
            let device = &self.device;
            let queue = &self.queue;
            let ui_pass = &mut self.ui_pass;
            ui_pass.encode(device, queue, encoder, surface_view, ctx.surface_format, &ui, Some(ctx.prof));
            lap_ms(ctx.lap);
        } else {
            lap_ms(ctx.lap);
        }
    }
    }
}
