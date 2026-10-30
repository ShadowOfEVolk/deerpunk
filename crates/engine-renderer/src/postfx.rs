//! Basic bloom (HDR) + film grain (LDR) — darkdemo postfx subset.

use std::time::Instant;

use bytemuck::{Pod, Zeroable};

use crate::render_profile::{lap_ms, RenderFrameProfile};

const NOISE_SIZE: u32 = 256;
const GRAIN_FPS: f32 = 24.0;
const SOFT_KNEE: f32 = 0.5;
const MAX_BRIGHTNESS: f32 = 16.0;
const LDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BloomParamsGpu {
    threshold: f32,
    intensity: f32,
    /// soft_knee for threshold; horizontal flag (>0.5) for blur.
    soft_or_horizontal: f32,
    max_brightness: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GrainParamsGpu {
    screen: [f32; 2],
    frame: f32,
    noise_size: f32,
    strength: f32,
    scale: f32,
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TonemapGlobals {
    screen: [f32; 2],
    exposure: f32,
    tonemap_mode: f32,
    view_proj: [[f32; 4]; 4],
}

pub struct PostFxShaders {
    pub bloom_threshold: String,
    pub bloom_blur: String,
    pub bloom_composite: String,
    pub grain: String,
    pub tonemap: String,
}

/// Per-frame bind groups (invalidated on resize / bloom scale change).
struct FrameBindGroups {
    bloom_threshold: wgpu::BindGroup,
    bloom_blur_a: wgpu::BindGroup,
    bloom_blur_b: wgpu::BindGroup,
    bloom_composite: wgpu::BindGroup,
    tonemap_hdr: wgpu::BindGroup,
    tonemap_bloom: wgpu::BindGroup,
    grain: wgpu::BindGroup,
}

pub struct PostFx {
    pub bloom_enabled: bool,
    pub bloom_threshold: f32,
    pub bloom_intensity: f32,
    /// Bloom working resolution: `full / bloom_downscale` (2 = half, 4 = quarter).
    pub bloom_downscale: u32,
    pub grain_enabled: bool,
    pub grain_strength: f32,
    pub grain_downscale: f32,

    full_w: u32,
    full_h: u32,

    bloom_a: Option<wgpu::Texture>,
    bloom_a_view: Option<wgpu::TextureView>,
    bloom_b: Option<wgpu::Texture>,
    bloom_b_view: Option<wgpu::TextureView>,
    hdr_comp: Option<wgpu::Texture>,
    hdr_comp_view: Option<wgpu::TextureView>,
    ldr: Option<wgpu::Texture>,
    ldr_view: Option<wgpu::TextureView>,

    noise: wgpu::Texture,
    noise_view: wgpu::TextureView,

    linear_sampler: wgpu::Sampler,
    /// Noise tile: bilinear + repeat (smoother grain than nearest).
    noise_sampler: wgpu::Sampler,

    bloom_params_buf: wgpu::Buffer,
    grain_params_buf: wgpu::Buffer,

    fs_tex_bg_layout: wgpu::BindGroupLayout,
    composite_bg_layout: wgpu::BindGroupLayout,
    grain_bg_layout: wgpu::BindGroupLayout,
    tonemap_bg_layout: wgpu::BindGroupLayout,

    threshold_pipeline: Option<wgpu::RenderPipeline>,
    blur_pipeline: Option<wgpu::RenderPipeline>,
    composite_pipeline: Option<wgpu::RenderPipeline>,
    tonemap_ldr_pipeline: Option<wgpu::RenderPipeline>,
    grain_pipeline: Option<wgpu::RenderPipeline>,
    grain_surface_format: Option<wgpu::TextureFormat>,

    shaders: PostFxShaders,
    hdr_format: wgpu::TextureFormat,
    frame_binds: Option<FrameBindGroups>,
}

fn generate_noise_rgba(size: u32) -> Vec<u8> {
    let pixels = (size * size) as usize;
    let mut rgba = vec![0u8; pixels * 4];
    let mut state = 0xA341_316C_u32;
    for i in 0..pixels {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let n = (state >> 16) as u8;
        let dst = i * 4;
        rgba[dst] = n;
        rgba[dst + 1] = n;
        rgba[dst + 2] = n;
        rgba[dst + 3] = 255;
    }
    rgba
}

fn make_fs_pipeline(
    device: &wgpu::Device,
    label: &str,
    shader_src: &str,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
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
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl PostFx {
    /// Replace post-process WGSL sources and drop cached pipelines (hot-reload).
    pub fn reload_shader_sources(&mut self, shaders: PostFxShaders) {
        self.shaders = shaders;
        self.threshold_pipeline = None;
        self.blur_pipeline = None;
        self.composite_pipeline = None;
        self.tonemap_ldr_pipeline = None;
        self.grain_pipeline = None;
        self.grain_surface_format = None;
        self.frame_binds = None;
    }

    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hdr_format: wgpu::TextureFormat,
        tonemap_bg_layout: wgpu::BindGroupLayout,
        shaders: PostFxShaders,
    ) -> Self {
        let noise = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("postfx grain noise"),
            size: wgpu::Extent3d {
                width: NOISE_SIZE,
                height: NOISE_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let rgba = generate_noise_rgba(NOISE_SIZE);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &noise,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(NOISE_SIZE * 4),
                rows_per_image: Some(NOISE_SIZE),
            },
            wgpu::Extent3d {
                width: NOISE_SIZE,
                height: NOISE_SIZE,
                depth_or_array_layers: 1,
            },
        );
        let noise_view = noise.create_view(&wgpu::TextureViewDescriptor::default());

        let fs_tex_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("postfx fs tex"),
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

        let composite_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("postfx composite"),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
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

        let grain_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("postfx grain"),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
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

        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("postfx linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let noise_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("postfx grain noise bilinear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bloom_params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("postfx bloom params"),
            size: std::mem::size_of::<BloomParamsGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let grain_params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("postfx grain params"),
            size: std::mem::size_of::<GrainParamsGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            bloom_enabled: true,
            bloom_threshold: 0.7,
            bloom_intensity: 1.0,
            bloom_downscale: 2,
            grain_enabled: true,
            grain_strength: 0.025,
            grain_downscale: 3.0,
            full_w: 0,
            full_h: 0,
            bloom_a: None,
            bloom_a_view: None,
            bloom_b: None,
            bloom_b_view: None,
            hdr_comp: None,
            hdr_comp_view: None,
            ldr: None,
            ldr_view: None,
            noise,
            noise_view,
            linear_sampler,
            noise_sampler,
            bloom_params_buf,
            grain_params_buf,
            fs_tex_bg_layout,
            composite_bg_layout,
            grain_bg_layout,
            tonemap_bg_layout,
            threshold_pipeline: None,
            blur_pipeline: None,
            composite_pipeline: None,
            tonemap_ldr_pipeline: None,
            grain_pipeline: None,
            grain_surface_format: None,
            shaders,
            hdr_format,
            frame_binds: None,
        }
    }

    pub fn set_bloom_downscale(&mut self, scale: u32) {
        let s = scale.clamp(2, 4);
        if self.bloom_downscale != s {
            self.bloom_downscale = s;
            self.invalidate_size();
        }
    }

    /// Drop size-dependent GPU targets (call on window resize).
    pub fn invalidate_size(&mut self) {
        self.full_w = 0;
        self.full_h = 0;
        self.frame_binds = None;
        self.bloom_a = None;
        self.bloom_a_view = None;
        self.bloom_b = None;
        self.bloom_b_view = None;
        self.hdr_comp = None;
        self.hdr_comp_view = None;
        self.ldr = None;
        self.ldr_view = None;
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        let scale = self.bloom_downscale.max(2);
        let bw = (w / scale).max(1);
        let bh = (h / scale).max(1);
        if self.full_w == w && self.full_h == h && self.bloom_a.is_some() {
            return;
        }
        self.full_w = w;
        self.full_h = h;
        self.frame_binds = None;

        let mk_hdr = |device: &wgpu::Device, label: &str, tw: u32, th: u32| {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: tw,
                    height: th,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.hdr_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            (tex, view)
        };

        let (a, av) = mk_hdr(device, "postfx bloom_a", bw, bh);
        let (b, bv) = mk_hdr(device, "postfx bloom_b", bw, bh);
        let (c, cv) = mk_hdr(device, "postfx hdr_comp", w, h);
        let ldr = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("postfx ldr"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: LDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let ldr_view = ldr.create_view(&wgpu::TextureViewDescriptor::default());

        self.bloom_a = Some(a);
        self.bloom_a_view = Some(av);
        self.bloom_b = Some(b);
        self.bloom_b_view = Some(bv);
        self.hdr_comp = Some(c);
        self.hdr_comp_view = Some(cv);
        self.ldr = Some(ldr);
        self.ldr_view = Some(ldr_view);
    }

    fn ensure_pipelines(&mut self, device: &wgpu::Device, surface_format: wgpu::TextureFormat) {
        if self.threshold_pipeline.is_none() {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("postfx threshold layout"),
                bind_group_layouts: &[Some(&self.fs_tex_bg_layout)],
                immediate_size: 0,
            });
            self.threshold_pipeline = Some(make_fs_pipeline(
                device,
                "postfx bloom threshold",
                &self.shaders.bloom_threshold,
                &layout,
                self.hdr_format,
            ));
        }
        if self.blur_pipeline.is_none() {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("postfx blur layout"),
                bind_group_layouts: &[Some(&self.fs_tex_bg_layout)],
                immediate_size: 0,
            });
            self.blur_pipeline = Some(make_fs_pipeline(
                device,
                "postfx bloom blur",
                &self.shaders.bloom_blur,
                &layout,
                self.hdr_format,
            ));
        }
        if self.composite_pipeline.is_none() {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("postfx composite layout"),
                bind_group_layouts: &[Some(&self.composite_bg_layout)],
                immediate_size: 0,
            });
            self.composite_pipeline = Some(make_fs_pipeline(
                device,
                "postfx bloom composite",
                &self.shaders.bloom_composite,
                &layout,
                self.hdr_format,
            ));
        }
        if self.tonemap_ldr_pipeline.is_none() {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("postfx tonemap ldr layout"),
                bind_group_layouts: &[Some(&self.tonemap_bg_layout)],
                immediate_size: 0,
            });
            self.tonemap_ldr_pipeline = Some(make_fs_pipeline(
                device,
                "postfx tonemap ldr",
                &self.shaders.tonemap,
                &layout,
                LDR_FORMAT,
            ));
        }
        if self.grain_pipeline.is_none() || self.grain_surface_format != Some(surface_format) {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("postfx grain layout"),
                bind_group_layouts: &[Some(&self.grain_bg_layout)],
                immediate_size: 0,
            });
            self.grain_pipeline = Some(make_fs_pipeline(
                device,
                "postfx grain",
                &self.shaders.grain,
                &layout,
                surface_format,
            ));
            self.grain_surface_format = Some(surface_format);
        }
    }

    fn make_fs_tex_bg(
        &self,
        device: &wgpu::Device,
        label: &str,
        src: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.fs_tex_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.bloom_params_buf.as_entire_binding(),
                },
            ],
        })
    }

    fn ensure_frame_binds(
        &mut self,
        device: &wgpu::Device,
        hdr_view: &wgpu::TextureView,
        tonemap_uniform_buf: &wgpu::Buffer,
    ) {
        if self.frame_binds.is_some() {
            return;
        }
        let a = self.bloom_a_view.as_ref().expect("bloom_a");
        let b = self.bloom_b_view.as_ref().expect("bloom_b");
        let comp = self.hdr_comp_view.as_ref().expect("hdr_comp");
        let ldr = self.ldr_view.as_ref().expect("ldr");

        let bloom_threshold = self.make_fs_tex_bg(device, "bloom threshold bg", hdr_view);
        let bloom_blur_a = self.make_fs_tex_bg(device, "bloom blur a bg", a);
        let bloom_blur_b = self.make_fs_tex_bg(device, "bloom blur b bg", b);
        let bloom_composite = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom composite bg"),
            layout: &self.composite_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(a),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.bloom_params_buf.as_entire_binding(),
                },
            ],
        });

        let tonemap_hdr = Self::make_tonemap_bg(
            device,
            &self.tonemap_bg_layout,
            hdr_view,
            &self.linear_sampler,
            tonemap_uniform_buf,
            "tonemap hdr bg",
        );
        let tonemap_bloom = Self::make_tonemap_bg(
            device,
            &self.tonemap_bg_layout,
            comp,
            &self.linear_sampler,
            tonemap_uniform_buf,
            "tonemap bloom bg",
        );
        let grain = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grain bg"),
            layout: &self.grain_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(ldr),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.noise_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.noise_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.grain_params_buf.as_entire_binding(),
                },
            ],
        });

        self.frame_binds = Some(FrameBindGroups {
            bloom_threshold,
            bloom_blur_a,
            bloom_blur_b,
            bloom_composite,
            tonemap_hdr,
            tonemap_bloom,
            grain,
        });
    }

    fn make_tonemap_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        src: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        tonemap_uniform_buf: &wgpu::Buffer,
        label: &str,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tonemap_uniform_buf.as_entire_binding(),
                },
            ],
        })
    }

    fn blit_fs(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        pipeline: &wgpu::RenderPipeline,
        dst: &wgpu::TextureView,
        params: BloomParamsGpu,
        label: &str,
        bg: &wgpu::BindGroup,
    ) {
        queue.write_buffer(&self.bloom_params_buf, 0, bytemuck::bytes_of(&params));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: dst,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bg, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Bloom (optional) + tonemap + grain (optional) onto the surface.
    pub fn encode_tonemap_chain(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        hdr_view: &wgpu::TextureView,
        w: u32,
        h: u32,
        surface_format: wgpu::TextureFormat,
        surface_view: &wgpu::TextureView,
        tonemap_surface_pipeline: &wgpu::RenderPipeline,
        tonemap_uniform_buf: &wgpu::Buffer,
        exposure: f32,
        tonemap_mode: f32,
        game_time: f32,
        mut prof: Option<&mut RenderFrameProfile>,
    ) {
        let mut lap = Instant::now();
        self.ensure_targets(device, w, h);
        self.ensure_pipelines(device, surface_format);

        self.ensure_frame_binds(device, hdr_view, tonemap_uniform_buf);
        let binds = self.frame_binds.as_ref().unwrap();

        // POSTFX OFF: bloom + grain disabled (commented).
        let use_bloom = false;
        /*
        let use_bloom = self.bloom_enabled && self.bloom_intensity > 0.001;
        if use_bloom {
            let a = self.bloom_a_view.as_ref().unwrap().clone();
            let b = self.bloom_b_view.as_ref().unwrap().clone();
            let comp = self.hdr_comp_view.as_ref().unwrap().clone();
            let thr = self.threshold_pipeline.as_ref().unwrap().clone();
            let blur = self.blur_pipeline.as_ref().unwrap().clone();
            let composite = self.composite_pipeline.as_ref().unwrap().clone();

            let base = BloomParamsGpu {
                threshold: self.bloom_threshold,
                intensity: self.bloom_intensity,
                soft_or_horizontal: SOFT_KNEE,
                max_brightness: MAX_BRIGHTNESS,
            };
            self.blit_fs(
                encoder,
                queue,
                &thr,
                &a,
                base,
                "bloom threshold",
                &binds.bloom_threshold,
            );

            // Single H→V blur at reduced resolution (was 2× ping-pong).
            self.blit_fs(
                encoder,
                queue,
                &blur,
                &b,
                BloomParamsGpu {
                    threshold: 0.0,
                    intensity: 0.0,
                    soft_or_horizontal: 1.0,
                    max_brightness: 0.0,
                },
                "bloom blur H",
                &binds.bloom_blur_a,
            );
            self.blit_fs(
                encoder,
                queue,
                &blur,
                &a,
                BloomParamsGpu {
                    threshold: 0.0,
                    intensity: 0.0,
                    soft_or_horizontal: 0.0,
                    max_brightness: 0.0,
                },
                "bloom blur V",
                &binds.bloom_blur_b,
            );

            let params = BloomParamsGpu {
                threshold: self.bloom_threshold,
                intensity: self.bloom_intensity,
                soft_or_horizontal: SOFT_KNEE,
                max_brightness: MAX_BRIGHTNESS,
            };
            queue.write_buffer(&self.bloom_params_buf, 0, bytemuck::bytes_of(&params));
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bloom composite"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &comp,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&composite);
                pass.set_bind_group(0, &binds.bloom_composite, &[]);
                pass.draw(0..3, 0..1);
            }
            if let Some(p) = prof.as_mut() {
                lap_ms(&mut lap);
            } else {
                lap_ms(&mut lap);
            }
        } else {
            lap_ms(&mut lap);
        }
        */

        let globals = TonemapGlobals {
            screen: [w as f32, h as f32],
            exposure,
            tonemap_mode,
            view_proj: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        };
        queue.write_buffer(tonemap_uniform_buf, 0, bytemuck::bytes_of(&globals));

        let tonemap_bg = if use_bloom {
            &binds.tonemap_bloom
        } else {
            &binds.tonemap_hdr
        };

        /* POSTFX grain off
        if self.grain_enabled && self.grain_strength > 0.001 {
            let ldr_view = self.ldr_view.as_ref().unwrap().clone();
            let tonemap_ldr = self.tonemap_ldr_pipeline.as_ref().unwrap().clone();
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("tonemap to ldr"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &ldr_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&tonemap_ldr);
                pass.set_bind_group(0, tonemap_bg, &[]);
                pass.draw(0..3, 0..1);
            }
            if let Some(p) = prof.as_mut() {
                lap_ms(&mut lap);
            }

            let frame_idx = (game_time * GRAIN_FPS).floor();
            let gparams = GrainParamsGpu {
                screen: [w as f32, h as f32],
                frame: frame_idx,
                noise_size: NOISE_SIZE as f32,
                strength: self.grain_strength,
                scale: self.grain_downscale,
                _pad: [0.0; 2],
            };
            queue.write_buffer(&self.grain_params_buf, 0, bytemuck::bytes_of(&gparams));
            let grain_pipe = self.grain_pipeline.as_ref().unwrap().clone();
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("grain pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: surface_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&grain_pipe);
                pass.set_bind_group(0, &binds.grain, &[]);
                pass.draw(0..3, 0..1);
            }
            if let Some(p) = prof.as_mut() {
                lap_ms(&mut lap);
            }
        } */
        if false {
        } else {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tonemap pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: surface_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(tonemap_surface_pipeline);
            pass.set_bind_group(0, tonemap_bg, &[]);
            pass.draw(0..3, 0..1);
            if let Some(p) = prof.as_mut() {
                lap_ms(&mut lap);
            }
        }
    }
}
