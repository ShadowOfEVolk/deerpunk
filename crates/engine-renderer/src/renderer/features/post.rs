//! Auto-split from `encode_frame` (frame graph pass).

use crate::frame::context::FrameCtx;
use crate::render_profile::lap_ms;
use crate::renderer::Renderer;
use deerpunk_engine_common::frame::{frame_height, frame_width};

macro_rules! render_scope {
    ($label:literal) => {
        #[cfg(any(feature = "tracy", feature = "puffin"))]
        let _render_profile =
            crate::profile_active().then(|| crate::gamelib_profile::zone_scope($label));
    };
}

impl Renderer {
    /// Tonemap HDR onto the post-chain LDR intermediate when a
    /// `post_fx/chain.toml` manifest is loaded, otherwise straight onto the
    /// surface (no-chain config state).
    pub(crate) fn pass_tonemap(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        if ctx.surface_view.is_none() {
            return;
        }
        // Sub-timings for `r_speeds`; multiple cameras overwrite (last run wins).
        ctx.prof.tonemap_ui = Default::default();
        // Build chain pipelines before deciding the tonemap target: TonemapUi
        // runs before PostFx in the pass list, so frame 1 must already know.
        self.post_chain
            .ensure_loaded(&self.gpu.device, ctx.surface_format);
        let chain_loaded = self.post_chain.is_loaded();
        let target_format = if chain_loaded {
            wgpu::TextureFormat::Rgba16Float
        } else {
            ctx.surface_format
        };
        self.ensure_tonemap_pipeline(target_format);
        render_scope!("tonemap_ui/chain");
        let hdr_view_ref = self.hdr.view.as_ref().expect("HDR texture not initialized");
        let w = frame_width();
        let h = frame_height();
        let exposure = self.hdr.exposure;
        let tonemap_mode = self.hdr.tonemap_mode;
        let pipeline = self.hdr.tonemap_pipeline.as_ref().unwrap().clone();
        let dst_view: &wgpu::TextureView = if chain_loaded {
            self.hdr
                .ldr_view
                .as_ref()
                .expect("LDR texture not initialized")
        } else {
            ctx.surface_view.unwrap()
        };
        self.postfx.encode_tonemap_chain(
            &self.gpu.device,
            &self.gpu.queue,
            encoder,
            hdr_view_ref,
            w,
            h,
            ctx.surface_format,
            dst_view,
            &pipeline,
            &self.hdr.tonemap_uniform_buf,
            exposure,
            tonemap_mode,
            self.scene_sun_dir,
            ctx.snapshot.view_proj,
            ctx.snapshot.camera_pos,
            Some(ctx.prof),
        );
        ctx.prof.tonemap_ui.chain_ms = lap_ms(ctx.lap);
    }

    /// Run the LDR post chain (ping-pong `post_fx/*.wgsl` passes) onto the
    /// surface, then the one-shot capture copy and the UI composite. The VHS
    /// look applies to the scene only — RmlUi draws on top, crisp.
    pub(crate) fn pass_postfx(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let Some(surface_view) = ctx.surface_view else {
            return;
        };
        render_scope!("tonemap_ui/postfx");

        if self.post_chain.is_loaded() {
            let w = frame_width();
            let h = frame_height();
            let ldr_view = self
                .hdr
                .ldr_view
                .as_ref()
                .expect("LDR texture not initialized");
            let time = self.post_chain_start.elapsed().as_secs_f32();
            // riteg's iFrame advances at the 30 fps authoring rate regardless
            // of render fps, so the tape model plays at real-time speed.
            let frame = (time * 30.0).floor();
            self.post_chain.encode(
                &self.gpu.device,
                &self.gpu.queue,
                encoder,
                ldr_view,
                w,
                h,
                surface_view,
                time,
                frame,
            );
        }
        ctx.prof.tonemap_ui.post_ms = lap_ms(ctx.lap);

        // One-shot capture: the surface holds the (post-processed) scene and no
        // UI has been drawn yet.
        render_scope!("tonemap_ui/capture");
        let capture_at = std::time::Instant::now();
        self.encode_capture_copy(encoder);
        ctx.prof.tonemap_ui.capture_ms = capture_at.elapsed().as_secs_f32() * 1000.0;

        // UI overlay (RmlUi geometry) on LDR surface
        render_scope!("tonemap_ui/ui");
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
