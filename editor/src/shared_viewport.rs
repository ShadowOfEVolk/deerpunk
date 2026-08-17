use iced::Rectangle;
use iced::Event;
use iced::widget::shader::{self, Pipeline, Primitive, Program};
use iced::widget::shader::Action;
use iced::mouse;

#[derive(Debug, Clone)]
pub struct SharedViewportPrimitive {
    pub handle: i64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct SharedViewportProgram {
    pub handle: i64,
    pub width: u32,
    pub height: u32,
}

impl<Message> Program<Message> for SharedViewportProgram {
    type State = ();
    type Primitive = SharedViewportPrimitive;

    fn update(
        &self,
        _state: &mut Self::State,
        _event: &Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        None
    }

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> SharedViewportPrimitive {
        SharedViewportPrimitive {
            handle: self.handle,
            width: self.width,
            height: self.height,
        }
    }
}

#[derive(Debug)]
pub struct SharedViewportPipeline {
    imported_view: Option<wgpu::TextureView>,
    bind_group: Option<wgpu::BindGroup>,
    last_handle: i64,
    last_size: (u32, u32),
    bind_group_layout: wgpu::BindGroupLayout,
    render_pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

const QUAD_SHADER: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var p = array<vec2<f32>,6>(
        vec2(-1.,-1.), vec2(1.,-1.), vec2(-1.,1.),
        vec2(-1.,1.),  vec2(1.,-1.), vec2(1.,1.),
    );
    var o: VsOut;
    o.pos = vec4(p[i], 0., 1.);
    o.uv = (p[i]+1.)*.5;
    o.uv.y = 1.-o.uv.y;
    return o;
}
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@fragment fn fs_main(v: VsOut) -> @location(0) vec4<f32> {
    return textureSample(tex, smp, v.uv);
}
"#;

impl Pipeline for SharedViewportPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shared_vp bgl"),
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
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shared_vp pl"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shared_vp shader"),
            source: wgpu::ShaderSource::Wgsl(QUAD_SHADER.into()),
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shared_vp pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shared_vp sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            imported_view: None,
            bind_group: None,
            last_handle: 0,
            last_size: (0, 0),
            bind_group_layout,
            render_pipeline,
            sampler,
        }
    }
}

impl Primitive for SharedViewportPrimitive {
    type Pipeline = SharedViewportPipeline;

    fn prepare(
        &self,
        pipeline: &mut SharedViewportPipeline,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _bounds: &Rectangle,
        _viewport: &shader::Viewport,
    ) {
        if self.handle == 0 {
            return;
        }
        if pipeline.last_handle == self.handle && pipeline.last_size == (self.width, self.height) {
            return;
        }

        let view = import_shared_handle(device, self.handle, self.width, self.height);
        if let Some(view) = view {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shared_vp bg"),
                layout: &pipeline.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&pipeline.sampler),
                    },
                ],
            });

            pipeline.imported_view = Some(view);
            pipeline.bind_group = Some(bind_group);
            pipeline.last_handle = self.handle;
            pipeline.last_size = (self.width, self.height);
        }
    }

    fn draw(
        &self,
        pipeline: &SharedViewportPipeline,
        render_pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let Some(ref bg) = pipeline.bind_group else {
            return false;
        };
        render_pass.set_pipeline(&pipeline.render_pipeline);
        render_pass.set_bind_group(0, bg, &[]);
        render_pass.draw(0..6, 0..1);
        true
    }
}

fn import_shared_handle(
    device: &wgpu::Device,
    handle: i64,
    width: u32,
    height: u32,
) -> Option<wgpu::TextureView> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Graphics::Direct3D12::*;

        let hal_device = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }?;
        let raw_device = hal_device.raw_device();

        let handle = HANDLE(handle as *mut _);

        let mut resource: Option<ID3D12Resource> = None;
        unsafe {
            raw_device.OpenSharedHandle(handle, &mut resource).ok()?;
        }
        let resource = resource?;

        let hal_texture = unsafe {
            wgpu::hal::dx12::Device::texture_from_raw(
                resource,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureDimension::D2,
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                1,
                1,
            )
        };

        let texture = unsafe {
            device.create_texture_from_hal::<wgpu::hal::api::Dx12>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("imported shared"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };

        Some(texture.create_view(&wgpu::TextureViewDescriptor::default()))
    }

    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}
