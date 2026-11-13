//! WGPU render pass for RmlUi geometry (`UiFrame`).
//!
//! Owns the shared UI pipeline/bind-group machinery. The main HUD renders
//! through an intermediate RT (`hud`) and blits onto the LDR surface; UI
//! *surfaces* render straight into their own offscreen RTs (`surface_<id>`)
//! with per-surface mesh/texture caches (`SurfaceCache`) so surface mesh ids
//! never collide with the main UI's.

use std::collections::HashMap;
use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use deerpunk_engine_common::ui_draw::{UiDraw, UiFrame, UiMesh, UiTextureUpload, UiVertex};

macro_rules! render_scope {
    ($label:literal) => {
        #[cfg(any(feature = "tracy", feature = "puffin"))]
        let _render_profile =
            crate::profile_active().then(|| crate::gamelib_profile::zone_scope($label));
    };
}

use crate::render_profile::{lap_ms, RenderFrameProfile};

pub(crate) const UI_TEX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

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

/// Screen-space sprite drawn topmost after the UI composite. Coordinates are
/// framebuffer pixels, origin top-left; `texture` is a handle into the
/// renderer's `surface_textures` registry (a surface id).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenSprite {
    pub texture: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Per-surface GPU state: mesh buffers + textures + draws buffer + globals.
/// Mesh ids are per-cache — surface meshes never collide with the main UI's.
struct SurfaceCache {
    meshes: HashMap<u32, CachedMesh>,
    textures: HashMap<u32, (wgpu::Texture, wgpu::BindGroup)>,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    draws_buf: wgpu::Buffer,
    draws_bg: wgpu::BindGroup,
    /// Packed per-draw translations for one `write_buffer` per flush.
    draw_scratch: Vec<[f32; 2]>,
}

impl SurfaceCache {
    fn new(
        device: &wgpu::Device,
        globals_layout: &wgpu::BindGroupLayout,
        draws_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("surface globals"),
            size: std::mem::size_of::<UiGlobals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("surface globals bg"),
            layout: globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });
        // Element 0 stays zero (reserved for the blit quad); surface
        // translations are written from offset 8 on flush.
        const SURFACE_DRAWS_INIT: u64 = 256 * TRANSLATION_BYTES;
        let draws_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("surface draws"),
            size: SURFACE_DRAWS_INIT,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let draws_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("surface draws bg"),
            layout: draws_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: draws_buf.as_entire_binding(),
            }],
        });
        Self {
            meshes: HashMap::new(),
            textures: HashMap::new(),
            globals_buf,
            globals_bg,
            draws_buf,
            draws_bg,
            draw_scratch: Vec::new(),
        }
    }
}

pub struct UiPass {
    /// Geometry pass writes the HUD into [`Self::hud`] (`Rgba8UnormSrgb`).
    geom_pipeline: Option<wgpu::RenderPipeline>,
    /// Fullscreen blit of that texture onto the LDR surface.
    composite_pipeline: Option<wgpu::RenderPipeline>,
    composite_format: Option<wgpu::TextureFormat>,
    globals_layout: wgpu::BindGroupLayout,
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
    /// Per-surface caches keyed by surface id. Separate mesh-id space from
    /// the main UI (`meshes`/`textures` above).
    surfaces: HashMap<u32, SurfaceCache>,
    /// Screen sprites accumulated per frame, drawn topmost after the UI
    /// composite.
    screen_sprites: Vec<ScreenSprite>,
    /// Screen-sprite quad vertices (6 per sprite), grown on demand.
    sprite_vbo: wgpu::Buffer,
}

/// Uploads one compiled mesh into its own GPU buffers (write-once at compile
/// time; later draws reuse them). Empty meshes are dropped so draws
/// referencing them skip. Shared by the main UI pass and the surface passes.
fn upload_mesh_into(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    meshes: &mut HashMap<u32, CachedMesh>,
    mesh: &UiMesh,
) {
    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
        meshes.remove(&mesh.id);
        return;
    }
    let verts: Vec<GpuUiVertex> = mesh.vertices.iter().map(|v| (*v).into()).collect();
    let vbytes = (verts.len() * std::mem::size_of::<GpuUiVertex>()) as u64;
    let ibytes = (mesh.indices.len() * std::mem::size_of::<u32>()) as u64;
    // Reuse the cached buffers when capacity allows: dirty text meshes
    // change every frame and per-frame buffer reallocation dominated
    // `tonemap_ui/ui_meshes` (allocate/grow only, never shrink).
    let (vbo, ibo) = match meshes.remove(&mesh.id) {
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
    meshes.insert(
        mesh.id,
        CachedMesh {
            vbo,
            ibo,
            index_count: mesh.indices.len() as u32,
        },
    );
}

fn write_texture_rgba(queue: &wgpu::Queue, texture: &wgpu::Texture, upload: &UiTextureUpload) {
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

fn upload_texture_into(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tex_layout: &wgpu::BindGroupLayout,
    sampler_linear: &wgpu::Sampler,
    sampler_nearest: &wgpu::Sampler,
    textures: &mut HashMap<u32, (wgpu::Texture, wgpu::BindGroup)>,
    id: u32,
    upload: &UiTextureUpload,
) {
    let w = upload.width.max(1);
    let h = upload.height.max(1);
    if let Some((tex, _bg)) = textures.get(&id) {
        let sz = tex.size();
        if sz.width == w && sz.height == h {
            write_texture_rgba(queue, tex, upload);
            return;
        }
        textures.remove(&id);
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
    write_texture_rgba(queue, &texture, upload);
    let view = texture.create_view(&Default::default());
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ui tex bg"),
        layout: tex_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(if upload.filter_nearest {
                    sampler_nearest
                } else {
                    sampler_linear
                }),
            },
        ],
    });
    textures.insert(id, (texture, bg));
}

/// Syncs a frame's texture uploads/releases into `textures`. Shared by the
/// main UI pass and the surface passes; guarantees white (id=1) exists.
fn sync_textures_into(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tex_layout: &wgpu::BindGroupLayout,
    sampler_linear: &wgpu::Sampler,
    sampler_nearest: &wgpu::Sampler,
    textures: &mut HashMap<u32, (wgpu::Texture, wgpu::BindGroup)>,
    frame: &UiFrame,
) {
    for id in &frame.released {
        textures.remove(id);
    }
    // Guarantee white (id=1) exists even if the frame forgot to upload it.
    if !textures.contains_key(&1) {
        let upload = UiTextureUpload {
            width: 1,
            height: 1,
            rgba: vec![255, 255, 255, 255],
            filter_nearest: false,
        };
        upload_texture_into(
            device,
            queue,
            tex_layout,
            sampler_linear,
            sampler_nearest,
            textures,
            1,
            &upload,
        );
    }
    for (id, upload) in &frame.textures {
        upload_texture_into(
            device,
            queue,
            tex_layout,
            sampler_linear,
            sampler_nearest,
            textures,
            *id,
            upload,
        );
    }
}

fn ensure_draws_capacity_into(
    device: &wgpu::Device,
    draws_layout: &wgpu::BindGroupLayout,
    draws_buf: &mut wgpu::Buffer,
    draws_bg: &mut wgpu::BindGroup,
    need_bytes: u64,
) {
    if need_bytes <= draws_buf.size() {
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
        layout: draws_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buf.as_entire_binding(),
        }],
    });
    *draws_buf = buf;
    *draws_bg = bg;
}

/// Renders one UI draw list into an open render pass. Shared by the main UI
/// pass and the surface passes. `base_instance` skips reserved translation
/// slots (element 0 stays zero for the blit quad).
fn draw_ui_list(
    pass: &mut wgpu::RenderPass<'_>,
    meshes: &HashMap<u32, CachedMesh>,
    textures: &HashMap<u32, (wgpu::Texture, wgpu::BindGroup)>,
    draws: &[UiDraw],
    screen: (u32, u32),
    base_instance: u32,
) {
    let (w, h) = screen;
    let mut bound_texture: Option<u32> = None;
    let mut bound_scissor: Option<[i32; 4]> = None;
    let mut clip_all = false;
    for (di, draw) in draws.iter().enumerate() {
        let Some(mesh) = meshes.get(&draw.mesh_id) else {
            continue;
        };
        if mesh.index_count == 0 {
            continue;
        }
        let Some((_, bg)) = textures.get(&draw.texture_id) else {
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
        let instance = base_instance + di as u32;
        pass.draw_indexed(0..mesh.index_count, 0, instance..instance + 1);
    }
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
        let sprite_vbo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui sprite quads"),
            size: (std::mem::size_of::<GpuUiVertex>() * 6 * 16) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            geom_pipeline: None,
            composite_pipeline: None,
            composite_format: None,
            globals_layout,
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
            surfaces: HashMap::new(),
            screen_sprites: Vec::new(),
            sprite_vbo,
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
        sync_textures_into(
            device,
            queue,
            &self.tex_layout,
            &self.sampler_linear,
            &self.sampler_nearest,
            &mut self.textures,
            frame,
        );
    }

    /// Sampling bind group for a texture view — used for surface RTs so the
    /// screen-sprite pass (and later the gamelib) can sample them.
    pub fn create_texture_bind_group(
        &self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("surface tex bg"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                },
            ],
        })
    }

    /// Frees the surface's mesh/texture/draws cache (RT itself lives in the
    /// renderer's `surface_textures` registry).
    pub fn release_surface(&mut self, surface_id: u32) {
        self.surfaces.remove(&surface_id);
    }

    /// Replaces the screen-sprite list drawn topmost after the UI composite.
    pub fn set_screen_sprites(&mut self, sprites: Vec<ScreenSprite>) {
        self.screen_sprites = sprites;
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
                    upload_mesh_into(device, queue, &mut self.meshes, mesh);
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
            ensure_draws_capacity_into(
                device,
                &self.draws_layout,
                &mut self.draws_buf,
                &mut self.draws_bg,
                translation_bytes_len + TRANSLATION_BYTES,
            );
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
            draw_ui_list(
                &mut pass,
                &self.meshes,
                &self.textures,
                &frame.draws,
                (w, h),
                1,
            );
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

    /// Renders one surface's `UiFrame` into its offscreen RT (`target`). The
    /// RT is created and sized by the renderer (`surface_<id>`); this pass
    /// uploads the surface's meshes/textures into a per-surface cache
    /// (separate mesh-id space from the main UI) and draws the list straight
    /// into the RT — no composite blit, content replaces the previous frame.
    pub fn encode_surface(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        surface_id: u32,
        target: &wgpu::TextureView,
        frame: &UiFrame,
        upload: bool,
    ) {
        if upload && frame.is_empty() {
            return;
        }
        if !upload && frame.draws.is_empty() {
            return;
        }
        let w = frame.screen[0].max(1);
        let h = frame.screen[1].max(1);
        {
            render_scope!("surface/ui_sync");
            self.ensure_geom_pipeline(device);
            if !self.surfaces.contains_key(&surface_id) {
                let cache = SurfaceCache::new(device, &self.globals_layout, &self.draws_layout);
                self.surfaces.insert(surface_id, cache);
            }
            let cache = self.surfaces.get_mut(&surface_id).unwrap();
            if upload {
                sync_textures_into(
                    device,
                    queue,
                    &self.tex_layout,
                    &self.sampler_linear,
                    &self.sampler_nearest,
                    &mut cache.textures,
                    frame,
                );
                for id in &frame.released_meshes {
                    cache.meshes.remove(id);
                }
                for mesh in &frame.meshes {
                    upload_mesh_into(device, queue, &mut cache.meshes, mesh);
                }
            }
            if frame.draws.is_empty() {
                return;
            }
            render_scope!("surface/ui_draw");
            cache.draw_scratch.clear();
            cache
                .draw_scratch
                .extend(frame.draws.iter().map(|d| d.translation));
            let translation_bytes_len = cache.draw_scratch.len() as u64 * TRANSLATION_BYTES;
            ensure_draws_capacity_into(
                device,
                &self.draws_layout,
                &mut cache.draws_buf,
                &mut cache.draws_bg,
                translation_bytes_len + TRANSLATION_BYTES,
            );
            let translation_bytes = bytemuck::cast_slice(&cache.draw_scratch) as &[u8];
            queue.write_buffer(&cache.draws_buf, TRANSLATION_BYTES, translation_bytes);
            queue.write_buffer(
                &cache.globals_buf,
                0,
                bytemuck::bytes_of(&UiGlobals {
                    screen: [w as f32, h as f32],
                    _pad: [0.0, 0.0],
                }),
            );
            let geom = self.geom_pipeline.as_ref().unwrap();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("surface render"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
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
            pass.set_bind_group(0, &cache.globals_bg, &[]);
            pass.set_bind_group(2, &cache.draws_bg, &[]);
            draw_ui_list(
                &mut pass,
                &cache.meshes,
                &cache.textures,
                &frame.draws,
                (w, h),
                1,
            );
        }
    }

    /// Draws the accumulated screen sprites topmost onto `target` (the LDR
    /// surface) after the UI composite. Coordinates are framebuffer pixels,
    /// origin top-left; standard alpha blending. The list is consumed
    /// (cleared) — a game that pushes nothing next frame pays zero cost.
    pub fn draw_screen_sprites(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        screen: (u32, u32),
        surface_textures: &HashMap<u32, crate::renderer::SurfaceTexture>,
    ) {
        if self.screen_sprites.is_empty() {
            return;
        }
        self.ensure_composite_pipeline(device, format);
        // One quad (6 verts) per sprite; translation stays 0 — positions are
        // absolute framebuffer pixels, origin top-left.
        let white = [1.0f32, 1.0, 1.0, 1.0];
        let v = |x: f32, y: f32, u: f32, vv: f32| GpuUiVertex {
            pos: [x, y],
            uv: [u, vv],
            color: white,
        };
        let mut quads: Vec<GpuUiVertex> = Vec::with_capacity(self.screen_sprites.len() * 6);
        for s in &self.screen_sprites {
            let (x, y, w, h) = (s.x, s.y, s.w, s.h);
            quads.extend([
                v(x, y, 0.0, 0.0),
                v(x + w, y, 1.0, 0.0),
                v(x + w, y + h, 1.0, 1.0),
                v(x, y, 0.0, 0.0),
                v(x + w, y + h, 1.0, 1.0),
                v(x, y + h, 0.0, 1.0),
            ]);
        }
        let quad_bytes: &[u8] = bytemuck::cast_slice(&quads);
        if self.sprite_vbo.size() < quad_bytes.len() as u64 {
            self.sprite_vbo = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui sprite quads"),
                size: (quad_bytes.len() as u64).next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(&self.sprite_vbo, 0, quad_bytes);
        queue.write_buffer(
            &self.globals_buf,
            0,
            bytemuck::bytes_of(&UiGlobals {
                screen: [screen.0 as f32, screen.1 as f32],
                _pad: [0.0, 0.0],
            }),
        );
        let pipeline = self.composite_pipeline.as_ref().unwrap();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("screen sprites"),
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
        pass.set_bind_group(2, &self.draws_bg, &[]);
        pass.set_vertex_buffer(0, self.sprite_vbo.slice(..));
        pass.set_scissor_rect(0, 0, screen.0, screen.1);
        let mut offset = 0u32;
        for s in &self.screen_sprites {
            let Some(st) = surface_textures.get(&s.texture) else {
                continue;
            };
            pass.set_bind_group(1, &st.bind_group, &[]);
            pass.draw(offset..offset + 6, 0..1);
            offset += 6;
        }
        drop(pass);
        self.screen_sprites.clear();
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
