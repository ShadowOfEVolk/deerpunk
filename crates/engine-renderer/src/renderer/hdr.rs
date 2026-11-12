//! HDR color target, tonemap pipeline, and auto-exposure readback.

use std::mem::size_of;

use super::Globals;

/// Intermediate RGBA16Float target plus the luminance probe used to drive exposure.
pub(crate) struct Hdr {
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) texture: Option<wgpu::Texture>,
    pub(crate) view: Option<wgpu::TextureView>,
    /// Linear LDR tonemap target (`Rgba16Float`); the post chain samples it
    /// when a `post_fx/chain.toml` manifest is present.
    pub(crate) ldr_texture: Option<wgpu::Texture>,
    pub(crate) ldr_view: Option<wgpu::TextureView>,
    pub(crate) tonemap_bg_layout: wgpu::BindGroupLayout,
    pub(crate) tonemap_pipeline: Option<wgpu::RenderPipeline>,
    pub(crate) tonemap_pipeline_format: Option<wgpu::TextureFormat>,
    pub(crate) tonemap_uniform_buf: wgpu::Buffer,
    pub(crate) exposure_override: Option<f32>,
    pub(crate) lum_texture: Option<wgpu::Texture>,
    pub(crate) lum_view: Option<wgpu::TextureView>,
    pub(crate) lum_staging_buf: wgpu::Buffer,
    pub(crate) lum_readback_rx:
        Option<std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    /// Staging buffer `map_async` in flight.
    pub(crate) lum_readback_pending: bool,
    pub(crate) lum_log_avg: f32,
    /// 1.0 = Reinhard.
    pub(crate) tonemap_mode: f32,
    pub(crate) exposure: f32,
    pub(crate) target_exposure: f32,
}

impl Hdr {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let tonemap_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tonemap bg layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let tonemap_uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tonemap uniform"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let lum_staging_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lum_staging"),
            size: (64 * 64 * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            format: wgpu::TextureFormat::Rgba16Float,
            texture: None,
            view: None,
            ldr_texture: None,
            ldr_view: None,
            tonemap_bg_layout,
            tonemap_pipeline: None,
            tonemap_pipeline_format: None,
            tonemap_uniform_buf,
            exposure_override: None,
            lum_texture: None,
            lum_view: None,
            lum_staging_buf,
            lum_readback_rx: None,
            lum_readback_pending: false,
            lum_log_avg: 0.0,
            tonemap_mode: 1.0,
            exposure: 1.0,
            target_exposure: 1.0,
        }
    }
}

impl super::Renderer {
    pub(in crate::renderer) fn ensure_hdr_texture(&mut self, width: u32, height: u32) {
        let recreate = match &self.hdr.texture {
            Some(tex) => tex.width() != width || tex.height() != height,
            None => true,
        };
        if !recreate {
            return;
        }
        let tex = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr intermediate"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.hdr.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.hdr.texture = Some(tex);
        self.hdr.view = Some(view);

        // Linear LDR tonemap target for the post chain (same size as HDR).
        let ldr_tex = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ldr intermediate"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        self.hdr.ldr_view = Some(ldr_tex.create_view(&wgpu::TextureViewDescriptor::default()));
        self.hdr.ldr_texture = Some(ldr_tex);

        // Create 64x64 luminance texture (for auto-exposure readback)
        let lum_tex = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("luminance 64x64"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.hdr.lum_view = Some(lum_tex.create_view(&wgpu::TextureViewDescriptor::default()));
        self.hdr.lum_texture = Some(lum_tex);
    }

    pub(in crate::renderer) fn ensure_tonemap_pipeline(
        &mut self,
        surface_format: wgpu::TextureFormat,
    ) {
        if self.hdr.tonemap_pipeline.is_some()
            && self.hdr.tonemap_pipeline_format == Some(surface_format)
        {
            return;
        }
        let shader = self
            .gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("tonemap shader"),
                source: wgpu::ShaderSource::Wgsl(self.shaders.tonemap.clone().into()),
            });
        let layout = self
            .gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("tonemap pipeline layout"),
                bind_group_layouts: &[Some(&self.hdr.tonemap_bg_layout)],
                immediate_size: 0,
            });
        let pipeline = self
            .gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("tonemap pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_fullscreen"),
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
                        format: surface_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        self.hdr.tonemap_pipeline = Some(pipeline);
        self.hdr.tonemap_pipeline_format = Some(surface_format);
    }

    pub fn auto_exposure_stats(&self) -> (f32, f32, f32) {
        (
            self.hdr.exposure,
            self.hdr.target_exposure,
            self.hdr.lum_log_avg,
        )
    }

    pub fn set_exposure_override(&mut self, v: Option<f32>) {
        self.hdr.exposure_override = v;
        if let Some(e) = v {
            self.hdr.exposure = e;
        }
    }

    pub fn set_tonemap_mode(&mut self, mode: f32) {
        self.hdr.tonemap_mode = mode;
    }

    /// Luminance readback is not part of the runtime frame.
    pub(in crate::renderer) fn append_lum_copy_to_encoder(
        &self,
        _encoder: &mut wgpu::CommandEncoder,
    ) -> bool {
        false
    }

    /// Call immediately after `queue.submit` when this frame's encoder included a lum copy.
    pub(in crate::renderer) fn queue_lum_readback_after_submit(&mut self, copied: bool) {
        if !copied || self.hdr.lum_readback_pending {
            return;
        }
        let buf = self.hdr.lum_staging_buf.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.hdr.lum_readback_rx = Some(rx);
        buf.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.hdr.lum_readback_pending = true;
    }

    pub(in crate::renderer) fn process_lum_readback(&mut self) {
        if !self.hdr.lum_readback_pending {
            return;
        }
        let _ = self.gpu.device.poll(wgpu::PollType::Poll);
        let done = self
            .hdr
            .lum_readback_rx
            .as_ref()
            .map(|rx| rx.try_recv().is_ok())
            .unwrap_or(false);
        if !done {
            return;
        }
        let slice = self.hdr.lum_staging_buf.slice(..);
        let data = slice.get_mapped_range().expect("lum readback mapped");
        let values: &[f32] = bytemuck::cast_slice(&data);
        let mut sum = 0.0f32;
        let count = values.len().min(64 * 64) as f32;
        for &v in values.iter().take(64 * 64) {
            sum += v;
        }
        let log_avg = if count > 0.0 { sum / count } else { 0.0 };
        self.hdr.lum_log_avg = log_avg;
        drop(data);
        self.hdr.lum_staging_buf.unmap();
        self.hdr.lum_readback_pending = false;
        self.hdr.lum_readback_rx = None;

        let avg_lum = log_avg.exp();
        self.hdr.target_exposure = 0.18 / (avg_lum + 0.001);
        self.hdr.target_exposure = self.hdr.target_exposure.clamp(0.001, 100.0);
        if self.hdr.exposure_override.is_none() {
            let adaptation_speed = 0.03;
            self.hdr.exposure += (self.hdr.target_exposure - self.hdr.exposure) * adaptation_speed;
        }
    }
}
