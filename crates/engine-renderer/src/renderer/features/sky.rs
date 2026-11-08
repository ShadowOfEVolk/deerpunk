//! Sky dome and volumetric clouds (one fullscreen feature, HDR target).

use std::collections::HashMap;
use std::mem::size_of;

use bytemuck::{Pod, Zeroable};

use super::super::Renderer;
use crate::frame::context::FrameCtx;
use crate::render_profile::lap_ms;

pub(crate) const SKY_UNIFORM_RING: usize = 2;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct SkyUniform {
    pub(crate) inv_view_proj: [[f32; 4]; 4],
    pub(crate) view_proj: [[f32; 4]; 4],
    pub(crate) sun_dir: [f32; 4],
    pub(crate) camera_pos: [f32; 4],
    pub(crate) params: [[f32; 4]; 8],
    /// Linear horizon haze (rgb) + sun-side glow (w).
    pub(crate) sky_color: [f32; 4],
    /// Linear zenith rgb (w unused). Do not pack into `view_proj` (camera matrix).
    pub(crate) zenith: [f32; 4],
}

const _: () = {
    assert!(
        size_of::<SkyUniform>() == 320,
        "SkyUniform must be 320 bytes"
    );
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct CloudUniform {
    pub(crate) inv_view_proj: [[f32; 4]; 4],
    pub(crate) camera_pos: [f32; 4],
    pub(crate) sun_dir: [f32; 4],
    pub(crate) params: [f32; 4],
    pub(crate) time: [f32; 4],
    pub(crate) sky_color: [f32; 4],
}

/// Sky dome + cloud layer GPU state.
pub(crate) struct Sky {
    pub(crate) sky_globals_bg_layout: wgpu::BindGroupLayout,
    sky_globals_bufs: [wgpu::Buffer; SKY_UNIFORM_RING],
    sky_globals_bgs: [wgpu::BindGroup; SKY_UNIFORM_RING],
    sky_uniform_ring: u8,
    sky_pipeline_layout: wgpu::PipelineLayout,
    sky_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,

    cloud_bg: wgpu::BindGroup,
    cloud_uniform_buf: wgpu::Buffer,
    cloud_pipeline_layout: wgpu::PipelineLayout,
    cloud_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,

    pub(crate) scene_sky_day_k: f32,
    pub(crate) scene_sky_zenith: [f32; 3],
    cloud_speed: f32,
    cloud_mode: u32,
    cloud_scale: f32,
    scene_sky_turbidity: f32,
    scene_sky_albedo: [f32; 3],
    sky_debug_log: bool,
    draw_sky: bool,
    draw_clouds: bool,
}

impl Sky {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let sky_globals_bg_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("sky globals bg layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let mut sky_globals_bufs = Vec::with_capacity(SKY_UNIFORM_RING);
        let mut sky_globals_bgs = Vec::with_capacity(SKY_UNIFORM_RING);
        for i in 0..SKY_UNIFORM_RING {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("sky globals {i}")),
                size: size_of::<SkyUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("sky globals bg {i}")),
                layout: &sky_globals_bg_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buf,
                        offset: 0,
                        size: None,
                    }),
                }],
            });
            sky_globals_bufs.push(buf);
            sky_globals_bgs.push(bg);
        }
        let sky_globals_bufs = sky_globals_bufs.try_into().ok().expect("sky uniform ring");
        let sky_globals_bgs = sky_globals_bgs
            .try_into()
            .ok()
            .expect("sky uniform ring bg");

        let sky_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky pipeline layout"),
            bind_group_layouts: &[Some(&sky_globals_bg_layout)],
            immediate_size: 0,
        });

        let cloud_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cloud bg layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let cloud_uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cloud uniform"),
            size: size_of::<CloudUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cloud_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cloud bg"),
            layout: &cloud_bg_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &cloud_uniform_buf,
                    offset: 0,
                    size: None,
                }),
            }],
        });
        let cloud_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cloud pipeline layout"),
                bind_group_layouts: &[Some(&cloud_bg_layout)],
                immediate_size: 0,
            });

        Self {
            sky_globals_bg_layout,
            sky_globals_bufs,
            sky_globals_bgs,
            sky_uniform_ring: 0,
            sky_pipeline_layout,
            sky_pipelines: HashMap::new(),
            cloud_bg,
            cloud_uniform_buf,
            cloud_pipeline_layout,
            cloud_pipelines: HashMap::new(),
            scene_sky_day_k: 1.0,
            scene_sky_zenith: [0.28, 0.55, 0.92],
            cloud_speed: 0.04,
            cloud_mode: 0,
            cloud_scale: 1.0,
            scene_sky_turbidity: 3.0,
            scene_sky_albedo: [0.2, 0.25, 0.3],
            sky_debug_log: false,
            draw_sky: true,
            draw_clouds: true,
        }
    }

    pub(crate) fn globals_layout(&self) -> &wgpu::BindGroupLayout {
        &self.sky_globals_bg_layout
    }

    pub(crate) fn clear_pipelines(&mut self) {
        self.sky_pipelines.clear();
        self.cloud_pipelines.clear();
    }
}

impl Renderer {
    pub(crate) fn pass_sky_and_clouds(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let snapshot = ctx.snapshot;
        let view = ctx.hdr_view;
        let hdr_fmt = self.hdr.format;
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
                snapshot.game_time,
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
        if self.sky.sky_debug_log {
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
        let sky_slot = (self.sky.sky_uniform_ring as usize + 1) % SKY_UNIFORM_RING;
        self.sky.sky_uniform_ring = sky_slot as u8;
        self.gpu.queue.write_buffer(
            &self.sky.sky_globals_bufs[sky_slot],
            0,
            bytemuck::bytes_of(&uniform),
        );

        if !self.sky.sky_pipelines.contains_key(&hdr_fmt) {
            let p = self.build_sky_pipeline(hdr_fmt);
            self.sky.sky_pipelines.insert(hdr_fmt, p);
        }
        let pipeline = self.sky.sky_pipelines.get(&hdr_fmt).unwrap().clone();

        ctx.draw_acc.note_pass();
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
        pass.set_pipeline(&pipeline);
        ctx.draw_acc.note_bind();
        pass.set_bind_group(0, &self.sky.sky_globals_bgs[sky_slot], &[]);
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
                self.sky.cloud_mode as f32,
                580.0, // meters above camera eye (not world Z)
                self.sky.cloud_speed,
            ],
            time: [snapshot.game_time, self.sky.cloud_scale, 0.0, 0.0],
            sky_color: [
                sky.sky_color[0],
                sky.sky_color[1],
                sky.sky_color[2],
                self.scene_ambient[3],
            ],
        };
        self.gpu.queue.write_buffer(
            &self.sky.cloud_uniform_buf,
            0,
            bytemuck::bytes_of(&cloud_uniform),
        );

        if !self.draws_clouds() {
            let _ = lap_ms(ctx.lap);
            return;
        }

        if !self.sky.cloud_pipelines.contains_key(&hdr_fmt) {
            let p = self.build_cloud_pipeline(hdr_fmt);
            self.sky.cloud_pipelines.insert(hdr_fmt, p);
        }
        let pipeline = self.sky.cloud_pipelines.get(&hdr_fmt).unwrap().clone();

        ctx.draw_acc.note_pass();
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
        pass.set_pipeline(&pipeline);
        ctx.draw_acc.note_bind();
        pass.set_bind_group(0, &self.sky.cloud_bg, &[]);
        pass.draw(0..3, 0..1);
        ctx.draw_acc.note_draw_verts(3);
        let _ = lap_ms(ctx.lap);
    }

    pub(crate) fn pass_clear_hdr_no_sky(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let snapshot = ctx.snapshot;
        let view = ctx.hdr_view;
        ctx.draw_acc.note_pass();
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

    pub(crate) fn build_sky_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let shader = self
            .gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("sky shader"),
                source: wgpu::ShaderSource::Wgsl(self.shaders.sky.clone().into()),
            });
        self.gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sky pipeline"),
                layout: Some(&self.sky.sky_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
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
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: 1,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }

    pub(crate) fn build_cloud_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let shader = self
            .gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("cloud shader"),
                source: wgpu::ShaderSource::Wgsl(self.shaders.cloud.clone().into()),
            });
        self.gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("cloud pipeline"),
                layout: Some(&self.sky.cloud_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
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
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }

    pub fn set_sky_palette(&mut self, day_k: f32, zenith: [f32; 3]) {
        self.sky.scene_sky_day_k = day_k.clamp(0.0, 1.0);
        self.sky.scene_sky_zenith = zenith;
    }

    pub fn scene_sky_day_k(&self) -> f32 {
        self.sky.scene_sky_day_k
    }

    pub fn scene_sky_zenith(&self) -> [f32; 3] {
        self.sky.scene_sky_zenith
    }

    pub fn set_cloud_speed(&mut self, speed: f32) {
        self.sky.cloud_speed = speed.clamp(0.0, 0.5);
    }

    pub fn set_cloud_mode(&mut self, mode: u32) {
        self.sky.cloud_mode = mode.min(1);
    }

    pub fn set_cloud_scale(&mut self, scale: f32) {
        self.sky.cloud_scale = scale.clamp(0.25, 8.0);
    }

    /// Hosek-Wilkie turbidity (1..10). Higher = hazier/more orange sky.
    pub fn set_sky_turbidity(&mut self, turbidity: f32) {
        self.sky.scene_sky_turbidity = turbidity.clamp(1.0, 10.0);
    }

    /// Hosek-Wilkie ground albedo (RGB 0..1).
    pub fn set_sky_albedo(&mut self, albedo: [f32; 3]) {
        self.sky.scene_sky_albedo = [
            albedo[0].clamp(0.0, 1.0),
            albedo[1].clamp(0.0, 1.0),
            albedo[2].clamp(0.0, 1.0),
        ];
    }

    pub fn set_sky_debug_log(&mut self, on: bool) {
        self.sky.sky_debug_log = on;
    }

    pub fn sky_debug_log(&self) -> bool {
        self.sky.sky_debug_log
    }

    pub fn scene_sky_turbidity(&self) -> f32 {
        self.sky.scene_sky_turbidity
    }

    pub fn scene_sky_albedo(&self) -> [f32; 3] {
        self.sky.scene_sky_albedo
    }

    pub fn set_sky_environment(&mut self, horizon_linear: [f32; 3], sun_glow: f32) {
        self.scene_ambient = [
            horizon_linear[0],
            horizon_linear[1],
            horizon_linear[2],
            sun_glow,
        ];
    }

    /// `r_drawsky` — skip the sky and cloud passes when off (HDR still clears).
    pub fn set_draw_sky(&mut self, on: bool) {
        self.sky.draw_sky = on;
    }

    pub(crate) fn draws_sky(&self) -> bool {
        self.sky.draw_sky
    }

    /// `r_drawclouds` — skip the cloud pass when off. Sky still draws.
    pub fn set_draw_clouds(&mut self, on: bool) {
        self.sky.draw_clouds = on;
    }

    pub(crate) fn draws_clouds(&self) -> bool {
        self.sky.draw_clouds
    }
}
