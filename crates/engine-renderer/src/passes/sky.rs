//! Sky + volumetric cloud fullscreen passes (HDR target).

use bytemuck;

use super::super::{CloudUniform, Renderer, SkyUniform, SKY_UNIFORM_RING};
use crate::frame::context::FrameCtx;
use crate::render_profile::lap_ms;

impl Renderer {
    pub(crate) fn pass_sky_and_clouds(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let snapshot = ctx.snapshot;
        let view = ctx.hdr_view;
        let hdr_fmt = self.hdr_format;
        let Some(sky) = &snapshot.sky else {
            lap_ms(ctx.lap);
            return;
        };

        let mut params = [[0.0f32; 4]; 8];
        for (i, v) in sky.params.iter().enumerate() {
            params[i / 4][i % 4] = *v;
        }
        let sky_inv_vp = snapshot.inv_view_proj_sky;
        let uniform = SkyUniform {
            inv_view_proj: sky_inv_vp,
            view_proj: snapshot.view_proj,
            sun_dir: sky.scene_sun,
            camera_pos: [
                snapshot.camera_pos[0],
                snapshot.camera_pos[1],
                snapshot.camera_pos[2],
                1.0,
            ],
            params,
            sky_color: [
                sky.sky_color[0],
                sky.sky_color[1],
                sky.sky_color[2],
                self.scene_ambient[3],
            ],
            zenith: [sky.zenith[0], sky.zenith[1], sky.zenith[2], 0.0],
        };
        if self.sky_debug_log {
            println!(
                "[sky:gpu] sun=[{:.3},{:.3},{:.3}] horizon=[{:.3},{:.3},{:.3}] zenith=[{:.3},{:.3},{:.3}] glow={:.3} day_fade={:.3}",
                uniform.sun_dir[0],
                uniform.sun_dir[1],
                uniform.sun_dir[2],
                uniform.sky_color[0],
                uniform.sky_color[1],
                uniform.sky_color[2],
                uniform.zenith[0],
                uniform.zenith[1],
                uniform.zenith[2],
                uniform.sky_color[3],
                sky.params[31],
            );
        }
        let sky_slot = (self.sky_uniform_ring as usize + 1) % SKY_UNIFORM_RING;
        self.sky_uniform_ring = sky_slot as u8;
        self.queue.write_buffer(
            &self.sky_globals_bufs[sky_slot],
            0,
            bytemuck::bytes_of(&uniform),
        );

        if !self.sky_pipelines.contains_key(&hdr_fmt) {
            let p = self.build_sky_pipeline(hdr_fmt);
            self.sky_pipelines.insert(hdr_fmt, p);
        }
        let pipeline = self.sky_pipelines.get(&hdr_fmt).unwrap();

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sky pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: snapshot.clear[0] as f64,
                        g: snapshot.clear[1] as f64,
                        b: snapshot.clear[2] as f64,
                        a: snapshot.clear[3] as f64,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        ctx.draw_acc.note_pipeline();
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.sky_globals_bgs[sky_slot], &[]);
        pass.draw(0..3, 0..1);
        ctx.draw_acc.note_draw_verts(3);
        let _ = lap_ms(ctx.lap);
        drop(pass);

        let cloud_uniform = CloudUniform {
            inv_view_proj: snapshot.inv_view_proj_sky,
            camera_pos: [
                snapshot.camera_pos[0],
                snapshot.camera_pos[1],
                snapshot.camera_pos[2],
                0.0,
            ],
            sun_dir: sky.scene_sun,
            params: [
                0.38,
                self.cloud_mode as f32,
                580.0, // meters above camera eye (not world Z)
                self.cloud_speed,
            ],
            time: [snapshot.game_time, 0.0, 0.0, 0.0],
            sky_color: [
                sky.sky_color[0],
                sky.sky_color[1],
                sky.sky_color[2],
                self.scene_ambient[3],
            ],
        };
        self.queue.write_buffer(
            &self.cloud_uniform_buf,
            0,
            bytemuck::bytes_of(&cloud_uniform),
        );

        if !self.cloud_pipelines.contains_key(&hdr_fmt) {
            let p = self.build_cloud_pipeline(hdr_fmt);
            self.cloud_pipelines.insert(hdr_fmt, p);
        }
        let pipeline = self.cloud_pipelines.get(&hdr_fmt).unwrap();

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cloud pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
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
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.cloud_bg, &[]);
        pass.draw(0..3, 0..1);
        ctx.draw_acc.note_draw_verts(3);
        let _ = lap_ms(ctx.lap);
    }

    pub(crate) fn pass_clear_hdr_no_sky(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        if ctx.snapshot.sky.is_some() {
            lap_ms(ctx.lap);
            return;
        }
        let snapshot = ctx.snapshot;
        let view = ctx.hdr_view;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear hdr"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: snapshot.clear[0] as f64,
                        g: snapshot.clear[1] as f64,
                        b: snapshot.clear[2] as f64,
                        a: snapshot.clear[3] as f64,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        drop(pass);
        let _ = lap_ms(ctx.lap);
    }
}
