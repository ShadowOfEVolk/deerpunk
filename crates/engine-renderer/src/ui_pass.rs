//! WGPU render pass for RmlUi geometry (`UiFrame`).

use std::collections::HashMap;
use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use deerpunk_engine_common::ui_draw::{UiFrame, UiVertex};

use crate::render_profile::{lap_ms, RenderFrameProfile};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UiGlobals {
    screen: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuUiVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

impl From<UiVertex> for GpuUiVertex {
    fn from(v: UiVertex) -> Self {
        Self {
            pos: v.pos,
            uv: v.uv,
            color: [
                v.color[0] as f32 / 255.0,
                v.color[1] as f32 / 255.0,
                v.color[2] as f32 / 255.0,
                v.color[3] as f32 / 255.0,
            ],
        }
    }
}

pub struct UiPass {
    pipeline: Option<wgpu::RenderPipeline>,
    pipeline_format: Option<wgpu::TextureFormat>,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    globals_layout: wgpu::BindGroupLayout,
    tex_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sampler_linear: wgpu::Sampler,
    sampler_nearest: wgpu::Sampler,
    vbo: wgpu::Buffer,
    ibo: wgpu::Buffer,
    vbo_cap: u64,
    ibo_cap: u64,
    textures: HashMap<u32, (wgpu::Texture, wgpu::BindGroup)>,
    shader: String,
    gpu_vert_scratch: Vec<GpuUiVertex>,
}

impl UiPass {
    pub fn new(device: &wgpu::Device, shader_source: String) -> Self {
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui globals layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<UiGlobals>() as u64),
                },
                count: None,
            }],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui tex layout"),
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui pipeline layout"),
            bind_group_layouts: &[Some(&globals_layout), Some(&tex_layout)],
            immediate_size: 0,
        });
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui globals"),
            size: std::mem::size_of::<UiGlobals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui globals bg"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });
        let sampler_linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui sampler linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let sampler_nearest = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui sampler nearest"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        const UI_VBO_INIT: u64 = 256 * 1024;
        let vbo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui vbo"),
            size: UI_VBO_INIT,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ibo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui ibo"),
            size: UI_VBO_INIT,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline: None,
            pipeline_format: None,
            globals_buf,
            globals_bg,
            globals_layout,
            tex_layout,
            pipeline_layout,
            sampler_linear,
            sampler_nearest,
            vbo,
            ibo,
            vbo_cap: UI_VBO_INIT,
            ibo_cap: UI_VBO_INIT,
            textures: HashMap::new(),
            shader: shader_source,
            gpu_vert_scratch: Vec::new(),
        }
    }

    fn ensure_pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        if self.pipeline_format == Some(format) && self.pipeline.is_some() {
            return;
        }
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui shader"),
            source: wgpu::ShaderSource::Wgsl(self.shader.clone().into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui pipeline"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuUiVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x4,
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.pipeline = Some(pipeline);
        self.pipeline_format = Some(format);
    }

    fn ensure_buffer(
        device: &wgpu::Device,
        buf: &mut wgpu::Buffer,
        cap: &mut u64,
        need: u64,
        usage: wgpu::BufferUsages,
        label: &str,
    ) {
        let have = buf.size();
        if need <= have {
            *cap = have.max(*cap);
            return;
        }
        // Headroom so a single heavy UI frame (console, inventory) does not resize every frame.
        let new_cap = (need * 3 / 2).next_power_of_two().max(256 * 1024);
        *buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: new_cap,
            usage,
            mapped_at_creation: false,
        });
        *cap = new_cap;
    }

    pub fn sync_textures(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &UiFrame) {
        for id in &frame.released {
            self.textures.remove(id);
        }
        // Guarantee white (id=1) exists even if the frame forgot to upload it.
        if !self.textures.contains_key(&1) {
            let upload = deerpunk_engine_common::ui_draw::UiTextureUpload {
                width: 1,
                height: 1,
                rgba: vec![255, 255, 255, 255],
                filter_nearest: false,
            };
            self.upload_texture(device, queue, 1, &upload);
        }
        for (id, upload) in &frame.textures {
            self.upload_texture(device, queue, *id, upload);
        }
    }

    fn write_texture_rgba(
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        upload: &deerpunk_engine_common::ui_draw::UiTextureUpload,
    ) {
        let w = upload.width.max(1);
        let h = upload.height.max(1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &upload.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }

    fn upload_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: u32,
        upload: &deerpunk_engine_common::ui_draw::UiTextureUpload,
    ) {
        let w = upload.width.max(1);
        let h = upload.height.max(1);
        if let Some((tex, _bg)) = self.textures.get(&id) {
            let sz = tex.size();
            if sz.width == w && sz.height == h {
                Self::write_texture_rgba(queue, tex, upload);
                return;
            }
            self.textures.remove(&id);
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui tex"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        Self::write_texture_rgba(queue, &texture, upload);
        let view = texture.create_view(&Default::default());
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui tex bg"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(if upload.filter_nearest {
                        &self.sampler_nearest
                    } else {
                        &self.sampler_linear
                    }),
                },
            ],
        });
        self.textures.insert(id, (texture, bg));
    }

    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        frame: &UiFrame,
        mut prof: Option<&mut RenderFrameProfile>,
    ) {
        if frame.is_empty() {
            return;
        }
        let mut lap = std::time::Instant::now();
        self.ensure_pipeline(device, format);
        self.sync_textures(device, queue, frame);
        if let Some(p) = prof.as_mut() {
                lap_ms(&mut lap);
        }

        self.gpu_vert_scratch.clear();
        self.gpu_vert_scratch.reserve(frame.vertices.len());
        for v in &frame.vertices {
            self.gpu_vert_scratch.push(GpuUiVertex::from(*v));
        }
        let gpu_verts = &self.gpu_vert_scratch;
        let v_bytes = bytemuck::cast_slice(&gpu_verts);
        let i_bytes = bytemuck::cast_slice(&frame.indices);
        Self::ensure_buffer(
            device,
            &mut self.vbo,
            &mut self.vbo_cap,
            v_bytes.len() as u64,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "ui vbo",
        );
        Self::ensure_buffer(
            device,
            &mut self.ibo,
            &mut self.ibo_cap,
            i_bytes.len() as u64,
            wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            "ui ibo",
        );
        queue.write_buffer(&self.vbo, 0, v_bytes);
        queue.write_buffer(&self.ibo, 0, i_bytes);
        queue.write_buffer(
            &self.globals_buf,
            0,
            bytemuck::bytes_of(&UiGlobals {
                screen: [frame.screen[0] as f32, frame.screen[1] as f32],
                _pad: [0.0, 0.0],
            }),
        );

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        pass.set_pipeline(self.pipeline.as_ref().unwrap());
        pass.set_bind_group(0, &self.globals_bg, &[]);
        pass.set_vertex_buffer(0, self.vbo.slice(..));
        pass.set_index_buffer(self.ibo.slice(..), wgpu::IndexFormat::Uint32);

        for batch in &frame.batches {
            if batch.index_count == 0 {
                continue;
            }
            let Some((_, bg)) = self.textures.get(&batch.texture_id) else {
                continue;
            };
            if let Some([x, y, w, h]) = batch.scissor {
                pass.set_scissor_rect(
                    x.max(0) as u32,
                    y.max(0) as u32,
                    w.max(0) as u32,
                    h.max(0) as u32,
                );
            } else {
                pass.set_scissor_rect(0, 0, frame.screen[0].max(1), frame.screen[1].max(1));
            }
            pass.set_bind_group(1, bg, &[]);
            pass.draw_indexed(
                batch.index_start..batch.index_start + batch.index_count,
                0,
                0..1,
            );
        }
        if let Some(p) = prof.as_mut() {
                lap_ms(&mut lap);
        }
    }
}
