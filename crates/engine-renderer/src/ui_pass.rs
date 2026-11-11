//! WGPU render pass for RmlUi geometry (`UiFrame`).

use std::collections::HashMap;
use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use deerpunk_engine_common::ui_draw::{UiFrame, UiMesh, UiVertex};

macro_rules! render_scope {
    ($label:literal) => {
        #[cfg(any(feature = "tracy", feature = "puffin"))]
        let _render_profile =
            crate::profile_active().then(|| crate::gamelib_profile::zone_scope($label));
    };
}

use crate::render_profile::{lap_ms, RenderFrameProfile};

const UI_TEX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// One `vec2<f32>` translation in the per-draw storage buffer.
const TRANSLATION_BYTES: u64 = 8;

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

struct CachedMesh {
    vbo: wgpu::Buffer,
    ibo: wgpu::Buffer,
    index_count: u32,
}

struct HudTarget {
    _tex: wgpu::Texture,
    view: wgpu::TextureView,
    bg: wgpu::BindGroup,
    size: (u32, u32),
}

pub struct UiPass {
    /// Geometry pass writes the HUD into [`Self::hud`] (`Rgba8UnormSrgb`).
    geom_pipeline: Option<wgpu::RenderPipeline>,
    /// Fullscreen blit of that texture onto the LDR surface.
    composite_pipeline: Option<wgpu::RenderPipeline>,
    composite_format: Option<wgpu::TextureFormat>,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    tex_layout: wgpu::BindGroupLayout,
    /// Read-only per-draw translations, bound as group 2.
    draws_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sampler_linear: wgpu::Sampler,
    sampler_nearest: wgpu::Sampler,
    /// Per-draw translations. Element 0 is reserved: it stays zero and the
    /// hud blit quad draws with instance 0. UI draws use `instance_index`.
    draws_buf: wgpu::Buffer,
    draws_bg: wgpu::BindGroup,
    textures: HashMap<u32, (wgpu::Texture, wgpu::BindGroup)>,
    shader: String,
    /// Packed per-draw translations for one `write_buffer` per flush.
    draw_scratch: Vec<[f32; 2]>,
    /// Compiled RmlUi meshes with write-once GPU buffers; draws add the
    /// translation from `draws_buf` in the vertex shader.
    meshes: HashMap<u32, CachedMesh>,
    hud: Option<HudTarget>,
    blit_vbo: wgpu::Buffer,
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
        let draws_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui draws layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(TRANSLATION_BYTES),
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui pipeline layout"),
            bind_group_layouts: &[
                Some(&globals_layout),
                Some(&tex_layout),
                Some(&draws_layout),
            ],
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
        // Element 0 stays zero (reserved for the blit quad); UI translations
        // are written from offset 8 on flush.
        const UI_DRAWS_INIT: u64 = 256 * TRANSLATION_BYTES;
        let draws_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui draws"),
            size: UI_DRAWS_INIT,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let draws_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui draws bg"),
            layout: &draws_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: draws_buf.as_entire_binding(),
            }],
        });
        let blit_vbo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui hud blit"),
            size: (std::mem::size_of::<GpuUiVertex>() * 6) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            geom_pipeline: None,
            composite_pipeline: None,
            composite_format: None,
            globals_buf,
            globals_bg,
            tex_layout,
            draws_layout,
            pipeline_layout,
            sampler_linear,
            sampler_nearest,
            draws_buf,
            draws_bg,
            textures: HashMap::new(),
            shader: shader_source,
            draw_scratch: Vec::new(),
            meshes: HashMap::new(),
            hud: None,
            blit_vbo,
        }
    }

    fn make_pipeline(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui shader"),
            source: wgpu::ShaderSource::Wgsl(self.shader.clone().into()),
        });

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
        })
    }

    fn ensure_geom_pipeline(&mut self, device: &wgpu::Device) {
        if self.geom_pipeline.is_none() {
            self.geom_pipeline = Some(self.make_pipeline(device, UI_TEX_FORMAT));
        }
    }

    fn ensure_composite_pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        if self.composite_format == Some(format) && self.composite_pipeline.is_some() {
            return;
        }
        self.composite_pipeline = Some(self.make_pipeline(device, format));
        self.composite_format = Some(format);
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
            format: UI_TEX_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[UI_TEX_FORMAT],
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

    /// Upload one compiled mesh into its own GPU buffers (write-once at
    /// compile time; later draws reuse them). Empty meshes are dropped so
    /// draws referencing them skip.
    fn upload_mesh(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, mesh: &UiMesh) {
        if mesh.vertices.is_empty() || mesh.indices.is_empty() {
            self.meshes.remove(&mesh.id);
            return;
        }
        let verts: Vec<GpuUiVertex> = mesh.vertices.iter().map(|v| (*v).into()).collect();
        let vbytes = (verts.len() * std::mem::size_of::<GpuUiVertex>()) as u64;
        let ibytes = (mesh.indices.len() * std::mem::size_of::<u32>()) as u64;
        // Reuse the cached buffers when capacity allows: dirty text meshes
        // change every frame and per-frame buffer reallocation dominated
        // `tonemap_ui/ui_meshes` (allocate/grow only, never shrink).
        let (vbo, ibo) = match self.meshes.remove(&mesh.id) {
            Some(old) if old.vbo.size() >= vbytes && old.ibo.size() >= ibytes => (old.vbo, old.ibo),
            _ => (
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("ui mesh vbo"),
                    size: vbytes.max(64),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("ui mesh ibo"),
                    size: ibytes.max(64),
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
            ),
        };
        queue.write_buffer(&vbo, 0, bytemuck::cast_slice(&verts));
        queue.write_buffer(&ibo, 0, bytemuck::cast_slice(&mesh.indices));
        self.meshes.insert(
            mesh.id,
            CachedMesh {
                vbo,
                ibo,
                index_count: mesh.indices.len() as u32,
            },
        );
    }

    fn ensure_draws_capacity(&mut self, device: &wgpu::Device, need_bytes: u64) {
        if need_bytes <= self.draws_buf.size() {
            return;
        }
        let size = need_bytes.next_power_of_two();
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui draws"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui draws bg"),
            layout: &self.draws_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buf.as_entire_binding(),
            }],
        });
        self.draws_buf = buf;
        self.draws_bg = bg;
    }

    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        frame: &UiFrame,
        upload: bool,
        mut prof: Option<&mut RenderFrameProfile>,
    ) {
        if frame.is_empty() && self.hud.is_none() {
            return;
        }
        let mut lap = std::time::Instant::now();
        let w = frame.screen[0].max(1);
        let h = frame.screen[1].max(1);
        if upload && !frame.is_empty() {
            {
                render_scope!("tonemap_ui/ui_sync");
                self.ensure_geom_pipeline(device);
                render_scope!("tonemap_ui/ui_textures");
                self.sync_textures(device, queue, frame);
                for id in &frame.released_meshes {
                    self.meshes.remove(id);
                }
                render_scope!("tonemap_ui/ui_meshes");
                for mesh in &frame.meshes {
                    self.upload_mesh(device, queue, mesh);
                }
                if let Some(p) = prof.as_mut() {
                    p.tonemap_ui.ui_sync_ms = lap_ms(&mut lap);
                }
            }
            render_scope!("tonemap_ui/ui_draw");
            if frame.draws.is_empty() {
                render_scope!("tonemap_ui/ui_blit");
                if let Some(p) = prof.as_mut() {
                    p.tonemap_ui.ui_draw_ms = lap_ms(&mut lap);
                }
                let blit_at = std::time::Instant::now();
                self.blit(device, encoder, target, format, w, h);
                if let Some(p) = prof.as_mut() {
                    p.tonemap_ui.ui_blit_ms = blit_at.elapsed().as_secs_f32() * 1000.0;
                }
                return;
            }
            self.ensure_hud(device, w, h);
            // Per-draw translations only: geometry lives in per-mesh GPU
            // buffers and the vertex shader adds `draws[instance_index]`.
            // Element 0 stays zero for the blit quad, so writes start at 8.
            self.draw_scratch.clear();
            self.draw_scratch
                .extend(frame.draws.iter().map(|d| d.translation));
            let translation_bytes_len = self.draw_scratch.len() as u64 * TRANSLATION_BYTES;
            self.ensure_draws_capacity(device, translation_bytes_len + TRANSLATION_BYTES);
            let translation_bytes = bytemuck::cast_slice(&self.draw_scratch) as &[u8];
            queue.write_buffer(&self.draws_buf, TRANSLATION_BYTES, translation_bytes);
            queue.write_buffer(
                &self.globals_buf,
                0,
                bytemuck::bytes_of(&UiGlobals {
                    screen: [w as f32, h as f32],
                    _pad: [0.0, 0.0],
                }),
            );
            let view = self.hud.as_ref().unwrap().view.clone();
            let geom = self.geom_pipeline.as_ref().unwrap();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui hud"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(geom);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            pass.set_bind_group(2, &self.draws_bg, &[]);
            let mut bound_texture: Option<u32> = None;
            let mut bound_scissor: Option<[i32; 4]> = None;
            let mut clip_all = false;
            for (di, draw) in frame.draws.iter().enumerate() {
                let Some(mesh) = self.meshes.get(&draw.mesh_id) else {
                    continue;
                };
                if mesh.index_count == 0 {
                    continue;
                }
                let Some((_, bg)) = self.textures.get(&draw.texture_id) else {
                    continue;
                };
                if bound_texture != Some(draw.texture_id) {
                    pass.set_bind_group(1, bg, &[]);
                    bound_texture = Some(draw.texture_id);
                }
                if bound_scissor != draw.scissor {
                    bound_scissor = draw.scissor;
                    clip_all = false;
                    match draw.scissor {
                        Some([x, y, sw, sh]) => {
                            let x0 = x.clamp(0, w as i32) as u32;
                            let y0 = y.clamp(0, h as i32) as u32;
                            let x1 = (x + sw).clamp(0, w as i32) as u32;
                            let y1 = (y + sh).clamp(0, h as i32) as u32;
                            if x1 > x0 && y1 > y0 {
                                pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
                            } else {
                                clip_all = true;
                            }
                        }
                        None => pass.set_scissor_rect(0, 0, w, h),
                    }
                }
                if clip_all {
                    continue;
                }
                pass.set_vertex_buffer(0, mesh.vbo.slice(..));
                pass.set_index_buffer(mesh.ibo.slice(..), wgpu::IndexFormat::Uint32);
                let instance = di as u32 + 1; // element 0 is the blit translation
                pass.draw_indexed(0..mesh.index_count, 0, instance..instance + 1);
            }
            drop(pass);
            self.write_blit(queue, w, h);
        }
        if let Some(p) = prof.as_mut() {
            p.tonemap_ui.ui_draw_ms = lap_ms(&mut lap);
        }
        render_scope!("tonemap_ui/ui_blit");
        let blit_at = std::time::Instant::now();
        self.blit(device, encoder, target, format, w, h);
        if let Some(p) = prof.as_mut() {
            p.tonemap_ui.ui_blit_ms = blit_at.elapsed().as_secs_f32() * 1000.0;
        }
    }

    fn ensure_hud(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        if self.hud.as_ref().is_some_and(|t| t.size == (w, h)) {
            return;
        }
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui hud"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: UI_TEX_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui hud bg"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                },
            ],
        });
        self.hud = Some(HudTarget {
            _tex: tex,
            view,
            bg,
            size: (w, h),
        });
    }

    fn write_blit(&self, queue: &wgpu::Queue, w: u32, h: u32) {
        let wf = w as f32;
        let hf = h as f32;
        let white = [1.0, 1.0, 1.0, 1.0];
        let v = |x, y, u, vv| GpuUiVertex {
            pos: [x, y],
            uv: [u, vv],
            color: white,
        };
        let verts = [
            v(0.0, 0.0, 0.0, 0.0),
            v(wf, 0.0, 1.0, 0.0),
            v(wf, hf, 1.0, 1.0),
            v(0.0, 0.0, 0.0, 0.0),
            v(wf, hf, 1.0, 1.0),
            v(0.0, hf, 0.0, 1.0),
        ];
        queue.write_buffer(&self.blit_vbo, 0, bytemuck::cast_slice(&verts));
    }

    fn blit(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        w: u32,
        h: u32,
    ) {
        if self.hud.is_none() {
            return;
        }
        self.ensure_composite_pipeline(device, format);
        let pipeline = self.composite_pipeline.as_ref().unwrap();
        let hud = self.hud.as_ref().unwrap();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui hud blit"),
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
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.globals_bg, &[]);
        pass.set_bind_group(1, &hud.bg, &[]);
        pass.set_bind_group(2, &self.draws_bg, &[]);
        pass.set_vertex_buffer(0, self.blit_vbo.slice(..));
        pass.set_scissor_rect(0, 0, w, h);
        pass.draw(0..6, 0..1);
    }
}
