//! Auto-split from `encode_frame` (frame graph pass).

use crate::frame::context::FrameCtx;
use crate::gpu::Renderer;
use crate::render_profile::lap_ms;

impl Renderer {
    pub(crate) fn pass_sprites_3d(
        &mut self,
        ctx: &mut FrameCtx<'_>, encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::RenderPipeline,
    ) {
    // --- Pass 1: 3D scene (clear + draw) ---
    let gpu_insts = self.build_instances_from(&ctx.snapshot.instances);
    self.write_buffers(&gpu_insts, ctx.snapshot.view_proj);
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3d pass"),
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
        pass.set_bind_group(0, &self.globals_bg, &[]);
        Self::draw_batched(
            &mut pass,
            &ctx.snapshot.instances,
            &self.sprite_bg,
            &self.map_bg,
            &self.font_bg,
            ctx.draw_acc,
        );
    }
        let _ = lap_ms(ctx.lap);
    }
}
