//! Auto-split from `encode_frame` (frame graph pass).

use crate::frame::context::FrameCtx;
use crate::render_profile::lap_ms;
use crate::renderer::Renderer;
use deerpunk_engine_common::frame::{frame_height, frame_width};

impl Renderer {
    pub(crate) fn pass_tonemap_and_ui(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        // --- Bloom → tonemap → grain → surface ---
        if let Some(surface_view) = ctx.surface_view {
            self.ensure_tonemap_pipeline(ctx.surface_format);
            let hdr_view_ref = self.hdr.view.as_ref().expect("HDR texture not initialized");
            let w = frame_width();
            let h = frame_height();
            let exposure = self.hdr.exposure;
            let tonemap_mode = self.hdr.tonemap_mode;
            let pipeline = self.hdr.tonemap_pipeline.as_ref().unwrap().clone();
            self.postfx.encode_tonemap_chain(
                &self.gpu.device,
                &self.gpu.queue,
                encoder,
                hdr_view_ref,
                w,
                h,
                ctx.surface_format,
                surface_view,
                &pipeline,
                &self.hdr.tonemap_uniform_buf,
                exposure,
                tonemap_mode,
                self.scene_sun_dir,
                ctx.snapshot.view_proj,
                ctx.snapshot.camera_pos,
                Some(ctx.prof),
            );
            lap_ms(ctx.lap);

            // One-shot capture: the surface holds the tonemapped scene and no
            // UI has been drawn yet (this is the only tonemap path — the
            // deferred-submit branch reuses this same encoder).
            self.encode_capture_copy(encoder);

            // UI overlay (RmlUi geometry) on LDR surface
            if let Some(ui) = self.pending_ui.as_ref() {
                let upload = std::mem::take(&mut self.ui_upload);
                let device = &self.gpu.device;
                let queue = &self.gpu.queue;
                let ui_pass = &mut self.ui_pass;
                ui_pass.encode(
                    device,
                    queue,
                    encoder,
                    surface_view,
                    ctx.surface_format,
                    ui,
                    upload,
                    Some(ctx.prof),
                );
                lap_ms(ctx.lap);
            } else {
                lap_ms(ctx.lap);
            }
        }
    }
}
