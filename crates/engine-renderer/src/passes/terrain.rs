//! Auto-split from `encode_frame` (frame graph pass).

use crate::frame::context::FrameCtx;
use crate::gpu::{MeshUniform, Renderer, DEPTH_FORMAT};
use crate::render_profile::lap_ms;
use deerpunk_engine_common::frame::{frame_height, frame_width};

impl Renderer {
    pub(crate) fn pass_terrain_splat(
        &mut self,
        ctx: &mut FrameCtx<'_>, encoder: &mut wgpu::CommandEncoder,
    ) {
    // --- Terrain splat pass ---
    if !ctx.snapshot.terrain_splat_draws.is_empty() {
        if !self.terrain_splat_pipelines.contains_key(&ctx.hdr_format) {
            let p = self.build_terrain_splat_pipeline(ctx.hdr_format);
            self.terrain_splat_pipelines.insert(ctx.hdr_format, p);
        }

        let uniform_size = 128u64;
        let vp: [[f32; 4]; 4] = ctx.snapshot.view_proj;

        for (i, td) in ctx.snapshot.terrain_splat_draws.iter().enumerate() {
            let model = td.model_mat;
            let mvp =
                glam::Mat4::from_cols_array_2d(&vp) * glam::Mat4::from_cols_array_2d(&model);
            let uniform = MeshUniform {
                view_proj: mvp.to_cols_array_2d(),
                model,
                material_data: [0, 0, 0, 0],
                camera_pos: ctx.snapshot.camera_pos,
                _pad_cam: 0.0,
                _pad: [0.0; 24],
            };
            self.queue.write_buffer(
                &self.mesh_globals_buf,
                i as u64 * uniform_size,
                bytemuck::bytes_of(&uniform),
            );
        }

        let terrain_handles: Vec<_> = ctx.snapshot
            .terrain_splat_draws
            .iter()
            .map(|td| self.mesh_for_draw(&td.mesh))
            .collect();

        let depth_view = self.ensure_depth(
            ctx.snapshot.depth.target,
            frame_width(),
            frame_height(),
        );

        let pipeline = self.terrain_splat_pipelines.get(&ctx.hdr_format).unwrap();

        let mesh_wrote_depth = !ctx.snapshot.mesh_draws.is_empty()
            || !ctx.snapshot.skinned_mesh_draws.is_empty();
        let depth_load = if mesh_wrote_depth {
            wgpu::LoadOp::Load
        } else {
            wgpu::LoadOp::Clear(1.0)
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("terrain splat pass"),
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
                    load: depth_load,
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

        for (i, td) in ctx.snapshot.terrain_splat_draws.iter().enumerate() {
            let handle = terrain_handles[i];
            let offset = (i as u64 * uniform_size) as u32;
            pass.set_bind_group(0, &self.mesh_globals_bg, &[]);

            let layer_views: [&wgpu::TextureView; 4] = std::array::from_fn(|j| {
                self.texture_views
                    .get(&td.layers[j].0)
                    .unwrap_or(&self.white_tex_view)
            });
            let splat_view = self
                .texture_views
                .get(&td.splat_map.0)
                .unwrap_or(&self.white_tex_view);
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("terrain splat bg"),
                layout: &self.terrain_splat_bg_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(layer_views[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(layer_views[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(layer_views[2]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(layer_views[3]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(splat_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::Sampler(&self.repeat_sampler),
                    },
                ],
            });
            pass.set_bind_group(1, &bg, &[]);

            let vb = self.mesh_pool.vertex_buffer(handle.buffer_id);
            let ib = self.mesh_pool.index_buffer(handle.buffer_id);
            pass.set_vertex_buffer(0, vb.slice(self.mesh_pool.vertex_byte_range(handle)));
            pass.set_index_buffer(
                ib.slice(self.mesh_pool.index_byte_range(handle)),
                wgpu::IndexFormat::Uint32,
            );
            pass.draw_indexed(0..handle.idx_count, 0, 0..1);
            ctx.draw_acc.note_draw_indexed(handle.idx_count);
        }
    }
        let _ = lap_ms(ctx.lap);
    }
}
