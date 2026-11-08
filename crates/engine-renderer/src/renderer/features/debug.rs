//! Auto-split from `encode_frame` (frame graph pass).

use crate::frame::context::FrameCtx;
use crate::render_profile::lap_ms;
use crate::renderer::{DebugViewUniform, Renderer, DEPTH_FORMAT};
use deerpunk_engine_common::frame::{frame_height, frame_width};

impl Renderer {
    pub(crate) fn pass_debug_lines(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        encoder: &mut wgpu::CommandEncoder,
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
            self.gpu.queue.write_buffer(
                &self.debug_globals_buf,
                0,
                bytemuck::bytes_of(&debug_view),
            );

            let verts = crate::debug_draw::collect_vertices(ctx.snapshot, 1.5, 120000);
            if !verts.is_empty() {
                self.ensure_debug_pipelines(ctx.hdr_format);

                self.gpu.queue.write_buffer(
                    &self.debug_vertex_buf,
                    0,
                    bytemuck::cast_slice(&verts),
                );
                let vcount = verts.len() as u32;
                let depth_view =
                    self.ensure_depth(ctx.snapshot.depth.target, frame_width(), frame_height());

                // 1) Behind geometry — dim (depth test: farther than scene).
                if let Some(ref pipeline) = self.debug_pipeline_occluded {
                    ctx.draw_acc.note_pass();
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
                            view: &depth_view,
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
                    ctx.draw_acc.note_bind();
                    pass.set_bind_group(0, &self.debug_globals_bg, &[]);
                    ctx.draw_acc.note_vertex_buffer();
                    pass.set_vertex_buffer(0, self.debug_vertex_buf.slice(..));
                    pass.draw(0..vcount, 0..1);
                    ctx.draw_acc.note_draw_verts(vcount);
                }

                // 2) In front / on surface — full color.
                if let Some(ref pipeline) = self.debug_pipeline {
                    ctx.draw_acc.note_pass();
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
                            view: &depth_view,
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
                    ctx.draw_acc.note_bind();
                    pass.set_bind_group(0, &self.debug_globals_bg, &[]);
                    ctx.draw_acc.note_vertex_buffer();
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

impl Renderer {
    pub(in crate::renderer) fn ensure_debug_pipelines(&mut self, format: wgpu::TextureFormat) {
        if self.debug_pipeline.is_some() && self.debug_pipeline_occluded.is_some() {
            return;
        }
        const ATTRS: [wgpu::VertexAttribute; 2] = [
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 12,
                shader_location: 1,
            },
        ];
        let shader = self
            .gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("debug shader"),
                source: wgpu::ShaderSource::Wgsl(self.shaders.debug.clone().into()),
            });
        let make = |device: &wgpu::Device,
                    layout: &wgpu::PipelineLayout,
                    shader: &wgpu::ShaderModule,
                    label: &str,
                    fs: &str,
                    depth_compare: wgpu::CompareFunction,
                    bias_const: i32|
         -> wgpu::RenderPipeline {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<crate::debug_draw::DebugVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &ATTRS,
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(depth_compare),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState {
                        constant: bias_const,
                        slope_scale: if bias_const != 0 { -1.0 } else { 0.0 },
                        clamp: 0.0,
                    },
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        if self.debug_pipeline_occluded.is_none() {
            self.debug_pipeline_occluded = Some(make(
                &self.gpu.device,
                &self.debug_pipeline_layout,
                &shader,
                "debug occluded",
                "fs_occluded",
                wgpu::CompareFunction::Greater,
                0,
            ));
        }
        if self.debug_pipeline.is_none() {
            // Slight negative bias so coplanar floor edges win as "visible".
            self.debug_pipeline = Some(make(
                &self.gpu.device,
                &self.debug_pipeline_layout,
                &shader,
                "debug visible",
                "fs_main",
                wgpu::CompareFunction::LessEqual,
                -1,
            ));
        }
    }
}
