//! Auto-split from `encode_frame` (frame graph pass).

use crate::frame::context::FrameCtx;
use crate::gpu::Renderer;
use crate::render_profile::lap_ms;
use deerpunk_engine_common::frame::{frame_height, frame_width};

impl Renderer {
    pub(crate) fn pass_hud(
        &mut self,
        ctx: &mut FrameCtx<'_>, encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::RenderPipeline, hud_vp: [[f32; 4]; 4],
    ) {
    // --- Pass 2: HUD overlay (load, no clear) ---
    if !ctx.snapshot.hud.is_empty() {
        let gpu_hud = self.build_instances_from(&ctx.snapshot.hud);
        self.write_hud_buffers(&gpu_hud, hud_vp);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hud pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: ctx.hdr_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        ctx.draw_acc.note_pipeline();
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &self.hud_globals_bg, &[]);
        Self::draw_batched(
            &mut pass,
            &ctx.snapshot.hud,
            &self.sprite_bg,
            &self.map_bg,
            &self.font_bg,
            ctx.draw_acc,
        );
    }
        let _ = lap_ms(ctx.lap);
    }
}
