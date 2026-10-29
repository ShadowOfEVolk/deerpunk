//! Auto-split from `encode_frame` (frame graph pass).

use crate::frame::context::FrameCtx;
use crate::gpu::{DebugViewUniform, Renderer};
use crate::render_profile::lap_ms;
use deerpunk_engine_common::frame::{frame_height, frame_width};

impl Renderer {
    pub(crate) fn pass_debug_lines(
        &mut self,
        ctx: &mut FrameCtx<'_>, encoder: &mut wgpu::CommandEncoder,
    ) {
    // --- Debug lines pass ---
    {
        let inv_vp = deerpunk_engine_common::coord::mat4_inverse(ctx.snapshot.view_proj);
        let debug_view = DebugViewUniform {
            inv_view_proj: inv_vp,
            view_proj: ctx.snapshot.view_proj,
            sun_dir: [0.0; 4],
            camera_pos: [
                ctx.snapshot.camera_pos[0],
                ctx.snapshot.camera_pos[1],
                ctx.snapshot.camera_pos[2],
                1.0,
            ],
            params: [[0.0; 4]; 8],
        };
        self.queue.write_buffer(
            &self.debug_globals_buf,
            0,
            bytemuck::bytes_of(&debug_view),
        );

        let verts = crate::debug_draw::collect_vertices(ctx.snapshot, 1.5, 120000);
        if !verts.is_empty() {
            self.ensure_depth(frame_width(), frame_height());
            self.ensure_debug_pipelines(ctx.hdr_format);

            self.queue
                .write_buffer(&self.debug_vertex_buf, 0, bytemuck::cast_slice(&verts));
            let vcount = verts.len() as u32;
            let depth_view = self.depth_view.as_ref().unwrap();

            // 1) Behind geometry — dim (depth test: farther than scene).
            if let Some(ref pipeline) = self.debug_pipeline_occluded {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("debug occluded"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: ctx.hdr_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                ctx.draw_acc.note_pipeline();
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &self.debug_globals_bg, &[]);
                pass.set_vertex_buffer(0, self.debug_vertex_buf.slice(..));
                pass.draw(0..vcount, 0..1);
                ctx.draw_acc.note_draw_verts(vcount);
            }

            // 2) In front / on surface — full color.
            if let Some(ref pipeline) = self.debug_pipeline {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("debug visible"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: ctx.hdr_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                ctx.draw_acc.note_pipeline();
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &self.debug_globals_bg, &[]);
                pass.set_vertex_buffer(0, self.debug_vertex_buf.slice(..));
                pass.draw(0..vcount, 0..1);
                ctx.draw_acc.note_draw_verts(vcount);
            }
            ctx.prof.debug_verts = vcount;
        }
    }
        let _ = lap_ms(ctx.lap);
    }
}
