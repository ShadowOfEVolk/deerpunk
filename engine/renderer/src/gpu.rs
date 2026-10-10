//! GPU-бэкенд рендерера (wgpu).
//!
//! Принимает `RenderSnapshot` (display list: какие спрайты/примитивы на экране
//! и в каком порядке) и превращает его в инстансированный draw:
//! - резолвит `Arc<Pixmap>` в атлас спрайтов (dedup по указателю, upload при первом
//!   появлении, evict при переполнении атласа),
//! - грузит font-atlas и карту мира отдельными текстурами,
//! - рисует в порядке списка (порядок = z-index), батча по текстуре,
//! - умеет два выхода: surface окна и offscreen + readback RGBA (headless).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bytemuck::{Pod, Zeroable};
use deerpunk_engine_common::frame::{frame_height, frame_width};
use tiny_skia::Pixmap;
use wgpu::util::DeviceExt;

use crate::asset_inventory::{
    AssetEntry, AssetKind, ASSET_ID_MATERIAL, ASSET_ID_MESH, ASSET_ID_TEX_ARRAY,
};
use crate::font::{build_font_atlas, ATLAS_H, ATLAS_W};
use crate::mesh_allocator::{MESH_INDEX_SIZE, MESH_VERTEX_SIZE};
use crate::snapshot::{Instance, MeshTexture, RenderSnapshot, Shape, TexSel};

/// Спрайт, уменьшенный до ячейки атласа (nearest — как старый CPU-рендерер).
/// Возвращает (w, h, raw RGBA). Исходник больше ячейки — уменьшаем с сохранением
/// пропорций, чтобы `write_texture` не вылезал за ячейку (и край атласа).
fn fit_sprite(sprite: &Pixmap) -> (u32, u32, Vec<u8>) {
    let w = sprite.width();
    let h = sprite.height();
    if w <= SPRITE_CELL && h <= SPRITE_CELL {
        return (w, h, sprite.data().to_vec());
    }
    let scale = (SPRITE_CELL as f32 / w.max(h) as f32).min(1.0);
    let nw = (w as f32 * scale).max(1.0).round() as u32;
    let nh = (h as f32 * scale).max(1.0).round() as u32;
    let src = sprite.data();
    let mut out = Vec::with_capacity(nw as usize * nh as usize * 4);
    for y in 0..nh {
        let sy = if nh > 1 {
            ((y as f32 * (h - 1) as f32 / (nh - 1) as f32).round() as usize).min(h as usize - 1)
        } else {
            0
        };
        for x in 0..nw {
            let sx = if nw > 1 {
                ((x as f32 * (w - 1) as f32 / (nw - 1) as f32).round() as usize).min(w as usize - 1)
            } else {
                0
            };
            let off = (sy * w as usize + sx) * 4;
            out.extend_from_slice(&src[off..off + 4]);
        }
    }
    (nw, nh, out)
}

/// Размер атласа спрайтов (квадрат) в пикселях.
const SPRITE_ATLAS_SIZE: u32 = 2048;
/// Сторона ячейки атласа. Танки 149×118 умещаются в 160×160.
const SPRITE_CELL: u32 = 160;
const SPRITE_COLS: u32 = SPRITE_ATLAS_SIZE / SPRITE_CELL;
const SPRITE_CELLS: usize = (SPRITE_COLS * SPRITE_COLS) as usize;

/// Максимум инстансов за кадр.
const MAX_INSTANCES: usize = 8192;

/// Формат всех наших текстур и offscreen-таргета (без sRGB — байты как были
/// в CPU-пикспмапе, чтобы картинка совпадала с прежним рендерером).
const TEX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub struct ShaderSources {
    pub sprite: String,
    pub mesh: String,
    pub mesh_array: String,
    pub terrain_splat: String,
    pub sky: String,
    pub cloud: String,
    pub debug: String,
    pub tonemap: String,
    pub luminance: String,
    pub storm: String,
    pub ui: String,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    screen: [f32; 2],
    exposure: f32,
    tonemap_mode: f32,
    view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuInstance {
    x: f32,
    y: f32,
    rot: f32,
    hw: f32,
    hh: f32,
    // WGSL: vec4 выравнивается на 16 байт → после hh (offset 16) идёт
    // неявный паддинг до 32. Поля ниже повторяют layout шейдера 1:1.
    _pad0: [f32; 3],
    uv: [f32; 4],
    color: [f32; 4],
    shape: u32,
    _pad: [u32; 3],
}

const _: () = {
    assert!(
        size_of::<GpuInstance>() == 80,
        "GpuInstance must be 80 bytes"
    );
    assert!(core::mem::offset_of!(GpuInstance, uv) == 32);
    assert!(core::mem::offset_of!(GpuInstance, color) == 48);
    assert!(core::mem::offset_of!(GpuInstance, shape) == 64);
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MeshVertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    /// texture_2d_array layer (0 for regular 2D albedo draws).
    layer: f32,
}

const _: () = assert!(size_of::<MeshVertex>() == crate::mesh_allocator::MESH_VERTEX_SIZE as usize);

pub struct GpuMesh {
    pub vertex_buf: wgpu::Buffer,
    pub index_buf: wgpu::Buffer,
    pub index_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MeshUniform {
    view_proj: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    material_data: [u32; 4],
    /// World-space camera position (for correct view vector).
    camera_pos: [f32; 3],
    _pad_cam: f32,
}

const MAX_POINT_LIGHTS: usize = 32;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuPointLight {
    position: [f32; 3],
    range: f32,
    color: [f32; 3],
    intensity: f32,
}

/// Layout must match WGSL `LightsUniform` (uniform address space).
/// Header is four `u32`s — do **not** use `vec3` in WGSL here (align-16 gap → 1056 vs 1040).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightsUniform {
    count: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
    lights: [GpuPointLight; MAX_POINT_LIGHTS],
}

const _: () = assert!(std::mem::size_of::<LightsUniform>() == 16 + 32 * MAX_POINT_LIGHTS);
const _: () = assert!(std::mem::size_of::<GpuPointLight>() == 32);

#[derive(Clone, Copy)]
struct SpriteSlot {
    cell: usize,
    uv: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkyUniform {
    inv_view_proj: [[f32; 4]; 4],
    view_proj: [[f32; 4]; 4],
    sun_dir: [f32; 4],
    camera_pos: [f32; 4],
    params: [[f32; 4]; 8],
}

const _: () = {
    assert!(
        size_of::<SkyUniform>() == 288,
        "SkyUniform must be 288 bytes"
    );
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CloudUniform {
    inv_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    sun_dir: [f32; 4],
    params: [f32; 4],
    time: [f32; 4],
    sky_color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct StormUniform {
    inv_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    params: [f32; 4], // map_size, tile_meters, time, unused
}

/// Ошибка отрисовки кадра в окно.
#[derive(Debug)]
pub enum FrameError {
    /// Окно свернуто/закрыто или в очереди нет кадра — пропускаем кадр.
    SurfaceTimeout,
    SurfaceOccluded,
    /// Конфигурация поверхности устарела — нужно переconfigure.
    SurfaceOutdated,
    /// Поверхность потеряна — нужно пересоздать/переconfigure.
    SurfaceLost,
    SurfaceValidation,
}

struct Offscreen {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: [wgpu::Buffer; 2],
    width: u32,
    height: u32,
}

pub struct SharedTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub handle: i64,
    pub width: u32,
    pub height: u32,
}

pub struct SharedTexturePair {
    pub buffers: [SharedTexture; 2],
    pub read_idx: usize,
}

impl SharedTexturePair {
    pub fn read(&self) -> &SharedTexture {
        &self.buffers[self.read_idx]
    }

    pub fn write(&self) -> &SharedTexture {
        &self.buffers[1 - self.read_idx]
    }

    pub fn flip(&mut self) {
        self.read_idx = 1 - self.read_idx;
    }
}

/// GPU-бэкенд рендерера.
pub struct Renderer {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,

    ui_pass: crate::ui_pass::UiPass,
    pending_ui: Option<deerpunk_engine_common::ui_draw::UiFrame>,

    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    pipeline_layout: wgpu::PipelineLayout,
    globals_bg_layout: wgpu::BindGroupLayout,
    tex_bg_layout: wgpu::BindGroupLayout,

    mesh_globals_bg_layout: wgpu::BindGroupLayout,
    mesh_globals_buf: wgpu::Buffer,
    mesh_globals_bg: wgpu::BindGroup,
    mesh_pipeline_layout: wgpu::PipelineLayout,
    mesh_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    mesh_white_bg: wgpu::BindGroup,
    mesh_user_bgs: HashMap<u32, wgpu::BindGroup>,
    /// texture_2d_array albedo path (BSP world buckets).
    tex_array_bg_layout: wgpu::BindGroupLayout,
    mesh_array_pipeline_layout: wgpu::PipelineLayout,
    mesh_array_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    mesh_array_bgs: HashMap<u32, wgpu::BindGroup>,
    repeat_sampler: wgpu::Sampler,
    next_array_slot: u32,
    wireframe: bool,
    lights_bg_layout: wgpu::BindGroupLayout,
    lights_buf: wgpu::Buffer,
    lights_bg: wgpu::BindGroup,

    // Terrain splat pipeline
    terrain_splat_bg_layout: wgpu::BindGroupLayout,
    terrain_splat_pipeline_layout: wgpu::PipelineLayout,
    terrain_splat_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,

    sky_globals_bg_layout: wgpu::BindGroupLayout,
    sky_globals_buf: wgpu::Buffer,
    sky_globals_bg: wgpu::BindGroup,
    sky_pipeline_layout: wgpu::PipelineLayout,
    sky_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,

    cloud_bg_layout: wgpu::BindGroupLayout,
    cloud_bg: wgpu::BindGroup,
    cloud_uniform_buf: wgpu::Buffer,
    cloud_pipeline_layout: wgpu::PipelineLayout,
    cloud_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,

    storm_bg_layout: wgpu::BindGroupLayout,
    storm_bg: wgpu::BindGroup,
    storm_uniform_buf: wgpu::Buffer,
    storm_texture: wgpu::Texture,
    storm_view: wgpu::TextureView,
    storm_sampler: wgpu::Sampler,
    storm_pipeline_layout: wgpu::PipelineLayout,
    storm_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,

    // Debug line pipeline (TriangleList, view_proj uniform)
    debug_pipeline: Option<wgpu::RenderPipeline>,
    debug_vertex_buf: wgpu::Buffer,

    sampler: wgpu::Sampler,
    globals_buf: wgpu::Buffer,
    instance_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,

    hud_globals_buf: wgpu::Buffer,
    hud_instance_buf: wgpu::Buffer,
    hud_globals_bg: wgpu::BindGroup,

    font_bg: wgpu::BindGroup,
    sprite_bg: wgpu::BindGroup,
    map_bg: wgpu::BindGroup,
    map_texture: Option<(wgpu::Texture, u32, u32)>,

    sprite_texture: wgpu::Texture,
    atlas: HashMap<usize, SpriteSlot>,
    free_cells: Vec<usize>,

    surface: Option<wgpu::Surface<'static>>,
    surface_config: Option<wgpu::SurfaceConfiguration>,
    offscreen: Option<Offscreen>,

    frame_buf: Vec<u8>,
    readback_idx: usize,
    readback_rx: Option<std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    readback_mapped: [bool; 2],

    depth_texture: Option<wgpu::Texture>,
    depth_view: Option<wgpu::TextureView>,

    // HDR intermediate target + tonemap
    hdr_format: wgpu::TextureFormat,
    hdr_texture: Option<wgpu::Texture>,
    hdr_view: Option<wgpu::TextureView>,
    tonemap_bg_layout: wgpu::BindGroupLayout,
    tonemap_bg: Option<wgpu::BindGroup>,
    tonemap_pipeline: Option<wgpu::RenderPipeline>,
    tonemap_uniform_buf: wgpu::Buffer,

    // Auto-exposure
    exposure_override: Option<f32>,
    lum_bg_layout: wgpu::BindGroupLayout,
    lum_texture: Option<wgpu::Texture>,
    lum_view: Option<wgpu::TextureView>,
    lum_pipeline: Option<wgpu::RenderPipeline>,
    lum_staging_buf: wgpu::Buffer,
    lum_readback_rx: Option<std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    lum_mapped: bool,
    lum_log_avg: f32,
    tonemap_mode: f32,
    exposure: f32,
    target_exposure: f32,

    shaders: ShaderSources,

    // ImGui
    imgui_ctx: Option<imgui::Context>,
    imgui_wgpu: Option<imgui_wgpu::Renderer>,

    // Mesh slab allocator + cache (Arc ptr → O(1); content hash fallback)
    mesh_allocator: crate::mesh_allocator::MeshAllocator,
    mesh_cache: HashMap<u64, crate::mesh_allocator::MeshHandle>,
    mesh_arc_cache: HashMap<usize, crate::mesh_allocator::MeshHandle>,
    /// Inspector asset inventory (textures / arrays / meshes). Materials rebuilt on query.
    asset_meta: HashMap<u64, AssetEntry>,
    next_mesh_asset_id: u32,

    // Material SSBO
    material_bg_layout: wgpu::BindGroupLayout,
    material_bgs: Vec<Option<wgpu::BindGroup>>,
    texture_views: HashMap<u32, wgpu::TextureView>,
    next_texture_slot: u32,
    material_bg_gen: u64,
    white_tex_view: wgpu::TextureView,
    /// Flat tangent-space normal (0.5, 0.5, 1.0) — white is NOT a valid default normal.
    flat_normal_tex_view: wgpu::TextureView,
    pub material_allocator: crate::material_allocator::MaterialAllocator,
}

impl Renderer {
    /// Создаёт устройство и пайплайны. Если surface передан, адаптер выбирается совместимый с ним.
    pub fn new(
        shaders: ShaderSources,
        instance: &wgpu::Instance,
        compatible_surface: Option<&wgpu::Surface<'_>>,
    ) -> Self {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface,
            ..Default::default()
        }))
        .expect("no adapter for wgpu");
        let mut required_features = wgpu::Features::empty();
        required_features |= wgpu::Features::POLYGON_MODE_LINE;
        let desc = wgpu::DeviceDescriptor {
            required_features,
            ..Default::default()
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&desc)).expect("no device for wgpu");

        let globals_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globals"),
            contents: bytemuck::bytes_of(&Globals {
                screen: [1.0, 1.0],
                exposure: 1.0,
                tonemap_mode: 0.0,
                view_proj: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: (MAX_INSTANCES * size_of::<GpuInstance>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let globals_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals bg layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let tex_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex bg layout"),
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

        let mesh_uniform_bytes = std::mem::size_of::<MeshUniform>() as u64;
        let mesh_globals_bg_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("mesh globals bg layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(mesh_uniform_bytes),
                    },
                    count: None,
                }],
            });
        // Mesh uniform buffer: 256 entries, each aligned to 256 bytes
        let mesh_align = 256u64; // min_uniform_buffer_offset_alignment
        let mesh_uniform_size = ((std::mem::size_of::<MeshUniform>() as u64 + mesh_align - 1)
            / mesh_align)
            * mesh_align;
        let mesh_globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh globals"),
            size: mesh_uniform_size * 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mesh_globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh globals bg"),
            layout: &mesh_globals_bg_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &mesh_globals_buf,
                    offset: 0,
                    size: wgpu::BufferSize::new(mesh_uniform_bytes),
                }),
            }],
        });
        // Material SSBO bind group (now with PBR texture slots)
        let material_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material bg layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let material_allocator = crate::material_allocator::MaterialAllocator::new(&device);
        // White 1x1 texture view for default PBR textures (normal=up, ORM=full white)
        let white_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("white 1x1"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEX_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &white_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &[255u8, 255, 255, 255],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let white_tex_view = white_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // Flat normal map: RGB(128,128,255) → tangent normal (0,0,1). White(255,255,255)
        // decodes to ~(0.57,0.57,0.57) and biases lighting ~45°.
        let flat_n_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("flat normal 1x1"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEX_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &flat_n_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[128u8, 128, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let flat_normal_tex_view = flat_n_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let lights_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point lights bg layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(
                        std::mem::size_of::<LightsUniform>() as u64,
                    ),
                },
                count: None,
            }],
        });
        let lights_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point lights"),
            size: std::mem::size_of::<LightsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let lights_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point lights bg"),
            layout: &lights_bg_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: lights_buf.as_entire_binding(),
            }],
        });

        let mesh_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh pipeline layout"),
            bind_group_layouts: &[
                Some(&mesh_globals_bg_layout),
                Some(&tex_bg_layout),
                Some(&material_bg_layout),
                Some(&lights_bg_layout),
            ],
            immediate_size: 0,
        });

        let tex_array_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex array bg layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
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
        let mesh_array_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh array pipeline layout"),
            bind_group_layouts: &[
                Some(&mesh_globals_bg_layout),
                Some(&tex_array_bg_layout),
                Some(&material_bg_layout),
                Some(&lights_bg_layout),
            ],
            immediate_size: 0,
        });
        let repeat_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("repeat linear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        // Terrain splat bind group layout: globals(0) + [layer0, layer1, layer2, layer3, splat_map, sampler](1)
        let terrain_splat_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain splat bg layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let terrain_splat_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain splat pipeline layout"),
            bind_group_layouts: &[Some(&mesh_globals_bg_layout), Some(&terrain_splat_bg_layout)],
            immediate_size: 0,
        });

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
        let sky_globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky globals"),
            size: size_of::<SkyUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sky_globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky globals bg"),
            layout: &sky_globals_bg_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &sky_globals_buf,
                    offset: 0,
                    size: None,
                }),
            }],
        });
        let sky_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky pipeline layout"),
            bind_group_layouts: &[Some(&sky_globals_bg_layout)],
            immediate_size: 0,
        });

        // --- Cloud: 3D noise texture (64³) + uniform + bind group ---
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

        // --- Storm: uniform + 64x64 infected texture + sampler ---
        let storm_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("storm bg layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        });

        let storm_uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("storm uniform"),
            size: size_of::<StormUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // 64x64 R8Unorm texture for storm infected data
        let storm_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("storm texture"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let storm_view = storm_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let storm_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("storm sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let mut storm_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("storm bg"),
            layout: &storm_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &storm_uniform_buf,
                        offset: 0,
                        size: None,
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&storm_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&storm_sampler),
                },
            ],
        });

        let storm_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("storm pipeline layout"),
                bind_group_layouts: &[Some(&storm_bg_layout)],
                immediate_size: 0,
            });

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

        let lum_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("luminance bg layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });

        let debug_vb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("debug vertices"),
            size: 4 * 1024 * 1024, // 4MB — enough for circles on many wheels
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline layout"),
            bind_group_layouts: &[Some(&globals_bg_layout), Some(&tex_bg_layout)],
            immediate_size: 0,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nearest"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals bg"),
            layout: &globals_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buf,
                        offset: 0,
                        size: None,
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &instance_buf,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
        });

        let hud_globals_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hud_globals"),
            contents: bytemuck::bytes_of(&Globals {
                screen: [1.0, 1.0],
                exposure: 1.0,
                tonemap_mode: 0.0,
                view_proj: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let hud_instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hud_instances"),
            size: (MAX_INSTANCES * size_of::<GpuInstance>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hud_globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hud globals bg"),
            layout: &globals_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &hud_globals_buf,
                        offset: 0,
                        size: None,
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &hud_instance_buf,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
        });

        let font_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("font atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_W,
                height: ATLAS_H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEX_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let font_data = build_font_atlas();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &font_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &font_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * ATLAS_W),
                rows_per_image: Some(ATLAS_H),
            },
            wgpu::Extent3d {
                width: ATLAS_W,
                height: ATLAS_H,
                depth_or_array_layers: 1,
            },
        );
        let font_view = font_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let font_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("font bg"),
            layout: &tex_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&font_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let sprite_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sprite atlas"),
            size: wgpu::Extent3d {
                width: SPRITE_ATLAS_SIZE,
                height: SPRITE_ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEX_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sprite_view = sprite_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sprite_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite bg"),
            layout: &tex_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&sprite_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        // Заглушка map-текстуры (1×1) до первого upload.
        let map_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("map placeholder"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEX_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let map_view = map_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let map_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("map bg"),
            layout: &tex_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&map_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let mesh_white_bg = {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("white 1x1"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TEX_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let white: [u8; 4] = [255, 255, 255, 255];
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &white,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("white bg"),
                layout: &tex_bg_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        };

        let lum_staging_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lum_staging"),
            size: (64 * 64 * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mesh_allocator = crate::mesh_allocator::MeshAllocator::new(&device, 256 * 1024, 512 * 1024);
        let ui_pass = crate::ui_pass::UiPass::new(&device, shaders.ui.clone());

        Self {
            adapter,
            device,
            queue,
            ui_pass,
            pending_ui: None,
            pipelines: HashMap::new(),
            pipeline_layout,
            globals_bg_layout,
            tex_bg_layout,

            mesh_globals_bg_layout,
            mesh_globals_buf,
            mesh_globals_bg,
            mesh_pipeline_layout,
            mesh_pipelines: HashMap::new(),
            mesh_white_bg,
            mesh_user_bgs: HashMap::new(),
            tex_array_bg_layout,
            mesh_array_pipeline_layout,
            mesh_array_pipelines: HashMap::new(),
            mesh_array_bgs: HashMap::new(),
            repeat_sampler,
            next_array_slot: 1,
            wireframe: false,
            lights_bg_layout,
            lights_buf,
            lights_bg,

            terrain_splat_bg_layout,
            terrain_splat_pipeline_layout,
            terrain_splat_pipelines: HashMap::new(),

            sky_globals_bg_layout,
            sky_globals_buf,
            sky_globals_bg,
            sky_pipeline_layout,
            sky_pipelines: HashMap::new(),

            cloud_bg_layout,
            cloud_bg,
            cloud_uniform_buf,
            cloud_pipeline_layout,
            cloud_pipelines: HashMap::new(),

            storm_bg_layout,
            storm_bg,
            storm_uniform_buf,
            storm_texture,
            storm_view,
            storm_sampler,
            storm_pipeline_layout,
            storm_pipelines: HashMap::new(),

            debug_pipeline: None,
            debug_vertex_buf: debug_vb,

            sampler,
            globals_buf,
            instance_buf,
            globals_bg,
            hud_globals_buf,
            hud_instance_buf,
            hud_globals_bg,
            font_bg,
            sprite_bg,
            map_bg,
            map_texture: None,
            sprite_texture,
            atlas: HashMap::new(),
            free_cells: (0..SPRITE_CELLS).collect(),
            surface: None,
            surface_config: None,
            offscreen: None,
            frame_buf: Vec::new(),
            readback_idx: 0,
            readback_rx: None,
            readback_mapped: [false; 2],
            depth_texture: None,
            depth_view: None,
            hdr_format: wgpu::TextureFormat::Rgba16Float,
            hdr_texture: None,
            hdr_view: None,
            tonemap_bg_layout,
            tonemap_bg: None,
            tonemap_pipeline: None,
            tonemap_uniform_buf,

            lum_bg_layout,
            lum_texture: None,
            lum_view: None,
            lum_pipeline: None,
            lum_staging_buf,
            lum_readback_rx: None,
            lum_mapped: false,
            lum_log_avg: 0.0,
            // 1.0 = Reinhard (softer than ACES for dark maps + point lights)
            tonemap_mode: 1.0,
            exposure_override: None,
            exposure: 1.0,
            target_exposure: 1.0,

            shaders,

            imgui_ctx: {
                let mut ctx = imgui::Context::create();
                ctx.set_ini_filename(None);
                Some(ctx)
            },
            imgui_wgpu: None,

            mesh_allocator,
            mesh_cache: HashMap::new(),
            mesh_arc_cache: HashMap::new(),
            asset_meta: HashMap::new(),
            next_mesh_asset_id: 1,

            material_bg_layout,
            material_bgs: Vec::new(),
            texture_views: HashMap::new(),
            next_texture_slot: 1,
            material_bg_gen: 0,
            white_tex_view,
            flat_normal_tex_view,
            material_allocator,
        }
    }

    /// Snapshot of uploaded GPU assets for the inspector `assets` collection.
    pub fn asset_inventory(&self) -> Vec<AssetEntry> {
        let mut out: Vec<AssetEntry> = self.asset_meta.values().cloned().collect();
        let mat_size = std::mem::size_of::<crate::material_allocator::MaterialGpuEntry>() as u64;
        for i in 0..self.material_allocator.len() {
            let id = ASSET_ID_MATERIAL | i as u64;
            out.push(AssetEntry {
                id,
                kind: AssetKind::Material,
                name: format!("material#{i}"),
                size: mat_size,
                width: None,
                height: None,
                layers: None,
                verts: None,
                indices: None,
            });
        }
        out.sort_by_key(|e| e.id);
        out
    }

    /// Подключает поверхность окна.
    pub fn set_surface(&mut self, surface: wgpu::Surface<'static>, width: u32, height: u32) {
        let caps = surface.get_capabilities(&self.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| *f == wgpu::TextureFormat::Rgba16Float)
            .or_else(|| {
                caps.formats
                    .iter()
                    .copied()
                    .find(|f| *f == wgpu::TextureFormat::Bgra8Unorm)
            })
            .or_else(|| {
                caps.formats
                    .iter()
                    .copied()
                    .find(|f| *f == wgpu::TextureFormat::Rgba8Unorm)
            })
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&self.device, &config);
        self.surface_config = Some(config);
        self.surface = Some(surface);

        self.ensure_hdr_texture(width, height);
        self.ensure_tonemap_pipeline(format);

        // Initialize imgui-wgpu renderer with the surface format
        if let Some(ctx) = &mut self.imgui_ctx {
            let config = imgui_wgpu::RendererConfig {
                texture_format: format,
                depth_format: None,
                sample_count: 1,
                ..Default::default()
            };
            self.imgui_wgpu = Some(imgui_wgpu::Renderer::new(
                ctx,
                &self.device,
                &self.queue,
                config,
            ));
        }
    }

    fn ensure_hdr_texture(&mut self, width: u32, height: u32) {
        let recreate = match &self.hdr_texture {
            Some(tex) => tex.width() != width || tex.height() != height,
            None => true,
        };
        if !recreate {
            return;
        }
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr intermediate"),
            size: wgpu::Extent3d {
                width,
                height,
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
        self.hdr_texture = Some(tex);
        self.hdr_view = Some(view);

        // Create 64x64 luminance texture (for auto-exposure readback)
        let lum_tex = self.device.create_texture(&wgpu::TextureDescriptor {
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
        self.lum_view = Some(lum_tex.create_view(&wgpu::TextureViewDescriptor::default()));
        self.lum_texture = Some(lum_tex);
    }

    fn ensure_tonemap_pipeline(&mut self, surface_format: wgpu::TextureFormat) {
        if self.tonemap_pipeline.is_some() {
            return;
        }
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("tonemap shader"),
                source: wgpu::ShaderSource::Wgsl(self.shaders.tonemap.clone().into()),
            });
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("tonemap pipeline layout"),
                bind_group_layouts: &[Some(&self.tonemap_bg_layout)],
                immediate_size: 0,
            });
        let pipeline = self
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
        self.tonemap_pipeline = Some(pipeline);
    }

    fn ensure_lum_pipeline(&mut self) {
        if self.lum_pipeline.is_some() {
            return;
        }
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("luminance shader"),
                source: wgpu::ShaderSource::Wgsl(self.shaders.luminance.clone().into()),
            });
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("luminance pipeline layout"),
                bind_group_layouts: &[Some(&self.lum_bg_layout)],
                immediate_size: 0,
            });
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("luminance pipeline"),
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
                        format: wgpu::TextureFormat::R32Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        self.lum_pipeline = Some(pipeline);
    }

    /// Пересобрать swapchain окна после resize.
    pub fn resize_surface(&mut self, width: u32, height: u32) {
        if let Some(config) = &mut self.surface_config {
            config.width = width.max(1);
            config.height = height.max(1);
            if let Some(surface) = &self.surface {
                surface.configure(&self.device, config);
            }
        }
        self.ensure_hdr_texture(width.max(1), height.max(1));
    }

    /// Загрузить свежеиспечённую зону карты в GPU-текстуру.
    pub fn upload_map(&mut self, pixmap: &Pixmap) {
        let w = pixmap.width();
        let h = pixmap.height();
        let recreate = match &self.map_texture {
            Some((_, ow, oh)) => *ow != w || *oh != h,
            None => true,
        };
        if recreate {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("map"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TEX_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("map bg"),
                layout: &self.tex_bg_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.map_bg = bg;
            self.map_texture = Some((tex, w, h));
        }
        let (tex, _, _) = self.map_texture.as_ref().expect("map texture");
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixmap.data(),
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

    /// Readback GPU map-текстуры (отладка: что реально залито на GPU).
    pub fn dump_map_texture(&mut self) -> Option<Vec<u8>> {
        let (tex, w, h) = self.map_texture.as_ref()?;
        let bpr = ((4 * w + 255) & !255) as u32;
        let buf_size = (bpr as u64) * (*h as u64);
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map readback debug"),
            size: buf_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("map readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bpr),
                    rows_per_image: Some(*h),
                },
            },
            wgpu::Extent3d {
                width: *w,
                height: *h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll map readback");
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll map map");
        let map_res = match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(r) => r,
            Err(_) => return None,
        };
        if map_res.is_err() {
            return None;
        }
        let data = slice.get_mapped_range().expect("mapped");
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..*h {
            let start = (row as u64 * bpr as u64) as usize;
            out.extend_from_slice(&data[start..start + (w * 4) as usize]);
        }
        drop(data);
        buf.unmap();
        Some(out)
    }

    /// Включает/выключает wireframe (пересоздаёт map mesh pipeline).
    pub fn set_wireframe(&mut self, wireframe: bool) {
        if self.wireframe != wireframe {
            self.wireframe = wireframe;
            self.mesh_pipelines.clear();
        }
    }

    /// Нарисовать снапшот в окно и показать.
    pub fn auto_exposure_stats(&self) -> (f32, f32, f32) {
        (self.exposure, self.target_exposure, self.lum_log_avg)
    }

    pub fn set_exposure_override(&mut self, v: Option<f32>) {
        self.exposure_override = v;
        if let Some(e) = v {
            self.exposure = e;
        }
    }

    pub fn set_tonemap_mode(&mut self, mode: f32) {
        self.tonemap_mode = mode;
    }

    // ── ImGui ────────────────────────────────────────────────────

    pub fn imgui_context_ptr(&self) -> *mut std::ffi::c_void {
        self.imgui_ctx.as_ref().map_or(std::ptr::null_mut(), |ctx| {
            ctx as *const imgui::Context as *mut std::ffi::c_void
        })
    }

    pub fn imgui_begin_frame(&mut self, width: f32, height: f32) {
        if let Some(ctx) = &mut self.imgui_ctx {
            ctx.io_mut().display_size = [width, height];
            unsafe {
                imgui::sys::igNewFrame();
            }
        }
    }

    pub fn imgui_end_frame_and_render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) {
        if self.imgui_ctx.is_none() || self.imgui_wgpu.is_none() {
            return;
        }

        unsafe {
            imgui::sys::igRender();
        }

        let draw_data_ptr = unsafe { imgui::sys::igGetDrawData() };
        if draw_data_ptr.is_null() {
            return;
        }
        let draw_data = unsafe { &*(draw_data_ptr as *const imgui::DrawData) };

        if let Some(renderer) = &mut self.imgui_wgpu {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("imgui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            let _ = renderer.render(draw_data, &self.queue, &self.device, &mut rpass);
        }
    }

    pub fn render_to_surface(&mut self, snapshot: &RenderSnapshot) -> Result<(), FrameError> {
        // 1. Read back previous frame's luminance (if ready)
        self.process_lum_readback();

        let Some(surface) = &self.surface else {
            return Ok(());
        };

        let current = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(st) => st,
            wgpu::CurrentSurfaceTexture::Suboptimal(st) => st,
            wgpu::CurrentSurfaceTexture::Timeout => return Err(FrameError::SurfaceTimeout),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(FrameError::SurfaceOccluded),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(FrameError::SurfaceOutdated),
            wgpu::CurrentSurfaceTexture::Lost => return Err(FrameError::SurfaceLost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(FrameError::SurfaceValidation),
        };
        let surface_view = current
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let format = self
            .surface_config
            .as_ref()
            .map(|c| c.format)
            .unwrap_or(TEX_FORMAT);

        let hdr_view = self.hdr_view.clone().expect("HDR texture not initialized");

        // Begin imgui frame before encode_frame (needs display_size set)
        let (sw, sh) = self.surface_config.as_ref().map(|c| (c.width as f32, c.height as f32)).unwrap_or((800.0, 600.0));
        self.imgui_begin_frame(sw, sh);

        let mut encoder = self.encode_frame(snapshot, &hdr_view, format, Some(&surface_view));

        // ImGui render pass (after all 3D passes, before present)
        self.imgui_end_frame_and_render(&mut encoder, &surface_view);

        // 2. Copy luminance texture to staging buffer for next frame's readback
        if let (Some(lum_tex), false) = (&self.lum_texture, self.lum_mapped) {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: lum_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &self.lum_staging_buf,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(64 * 4),
                        rows_per_image: Some(64),
                    },
                },
                wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
            );
        }

        self.queue.submit([encoder.finish()]);

        // 3. Wait for GPU to finish the luminance copy, then map the staging buffer
        if !self.lum_mapped {
            self.device.poll(wgpu::PollType::wait_indefinitely());
            self.lum_mapped = true;
            let buf = self.lum_staging_buf.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            self.lum_readback_rx = Some(rx);
            buf.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        }

        self.queue.present(current);
        Ok(())
    }

    pub fn render_to_surface_with_game_frame(
        &mut self,
        snapshot: &RenderSnapshot,
        game_frame_fn: Option<extern "C" fn(*mut std::ffi::c_void)>,
        game_frame_ctx: *mut std::ffi::c_void,
    ) -> Result<(), FrameError> {
        self.process_lum_readback();

        let Some(surface) = &self.surface else {
            return Ok(());
        };

        let current = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(st) => st,
            wgpu::CurrentSurfaceTexture::Suboptimal(st) => st,
            wgpu::CurrentSurfaceTexture::Timeout => return Err(FrameError::SurfaceTimeout),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(FrameError::SurfaceOccluded),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(FrameError::SurfaceOutdated),
            wgpu::CurrentSurfaceTexture::Lost => return Err(FrameError::SurfaceLost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(FrameError::SurfaceValidation),
        };
        let surface_view = current
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let format = self
            .surface_config
            .as_ref()
            .map(|c| c.format)
            .unwrap_or(TEX_FORMAT);

        let hdr_view = self.hdr_view.clone().expect("HDR texture not initialized");

        let (sw, sh) = self.surface_config.as_ref().map(|c| (c.width as f32, c.height as f32)).unwrap_or((800.0, 600.0));
        self.imgui_begin_frame(sw, sh);

        if let Some(f) = game_frame_fn {
            f(game_frame_ctx);
        }

        let mut encoder = self.encode_frame(snapshot, &hdr_view, format, Some(&surface_view));

        self.imgui_end_frame_and_render(&mut encoder, &surface_view);

        if let (Some(lum_tex), false) = (&self.lum_texture, self.lum_mapped) {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: lum_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &self.lum_staging_buf,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(64 * 4),
                        rows_per_image: Some(64),
                    },
                },
                wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
            );
        }

        self.queue.submit([encoder.finish()]);

        if !self.lum_mapped {
            self.device.poll(wgpu::PollType::wait_indefinitely());
            self.lum_mapped = true;
            let buf = self.lum_staging_buf.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            self.lum_readback_rx = Some(rx);
            buf.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        }

        self.queue.present(current);
        Ok(())
    }

    fn process_lum_readback(&mut self) {
        if !self.lum_mapped {
            return;
        }
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let done = self
            .lum_readback_rx
            .as_ref()
            .map(|rx| rx.try_recv().is_ok())
            .unwrap_or(false);
        if !done {
            return;
        }
        let slice = self.lum_staging_buf.slice(..);
        let data = slice.get_mapped_range().expect("lum readback mapped");
        let values: &[f32] = bytemuck::cast_slice(&data);
        let mut sum = 0.0f32;
        let count = values.len().min(64 * 64) as f32;
        for &v in values.iter().take(64 * 64) {
            sum += v;
        }
        let log_avg = if count > 0.0 { sum / count } else { 0.0 };
        drop(data);
        self.lum_staging_buf.unmap();
        self.lum_mapped = false;
        self.lum_readback_rx = None;

        let avg_lum = log_avg.exp();
        self.target_exposure = 0.18 / (avg_lum + 0.001);
        self.target_exposure = self.target_exposure.clamp(0.001, 100.0);
        if self.exposure_override.is_none() {
            let adaptation_speed = 0.03;
            self.exposure += (self.target_exposure - self.exposure) * adaptation_speed;
        }
    }

    /// Нарисовать снапшот в offscreen-текстуру и вернуть raw RGBA кадр.
    pub fn render_offscreen(&mut self, snapshot: &RenderSnapshot) -> &[u8] {
        let w = frame_width();
        let h = frame_height();
        let size = (w * h * 4) as usize;
        let buf_idx = self.readback_idx;

        // Step 1: Pump GPU callbacks so map_async completions fire
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());

        // Step 2: Try to collect previous frame's readback (non-blocking)
        if let Some(rx) = &self.readback_rx {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    if let Some(off) = &self.offscreen {
                        let prev_idx = 1 - buf_idx;
                        let slice = off.readback[prev_idx].slice(..);
                        let data = slice.get_mapped_range().expect("mapped range");
                        self.frame_buf.clear();
                        self.frame_buf.extend_from_slice(&data[..size]);
                        drop(data);
                        off.readback[prev_idx].unmap();
                    }
                    self.readback_mapped[1 - buf_idx] = false;
                    self.readback_rx = None;
                }
                Ok(Err(e)) => {
                    eprintln!("[gpu] readback map error: {e:?}");
                    if let Some(off) = &self.offscreen {
                        off.readback[buf_idx].unmap();
                    }
                    self.readback_mapped[buf_idx] = false;
                    self.readback_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Still pending — return previous frame_buf
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("[gpu] readback channel disconnected");
                    self.readback_rx = None;
                }
            }
        }

        // Step 2: Ensure current buffer is unmapped before reuse
        if self.readback_mapped[buf_idx] {
            if let Some(off) = &self.offscreen {
                off.readback[buf_idx].unmap();
            }
            self.readback_mapped[buf_idx] = false;
        }

        // Step 3: Ensure offscreen resources exist, then submit new frame
        self.ensure_offscreen(w, h);

        // Take what we need from offscreen to avoid borrow conflicts
        let (view_clone, texture_clone, readback_clone) = {
            let off = self.offscreen.as_ref().unwrap();
            (
                off.view.clone(),
                off.texture.clone(),
                off.readback[buf_idx].clone(),
            )
        };

        let mut encoder = self.encode_frame(snapshot, &view_clone, TEX_FORMAT, None);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture_clone,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback_clone,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * w),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        // Step 5: Start async map (callback fires next time poll() is called)
        let slice = readback_clone.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        self.readback_rx = Some(rx);
        self.readback_mapped[buf_idx] = true;

        // Step 5: Swap to other buffer for next frame
        self.readback_idx = 1 - buf_idx;

        &self.frame_buf
    }

    fn ensure_offscreen(&mut self, w: u32, h: u32) -> &mut Offscreen {
        let needs = match &self.offscreen {
            Some(off) => off.width != w || off.height != h,
            None => true,
        };
        if needs {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("offscreen"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TEX_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let readback = [
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("offscreen readback 0"),
                    size: (w * h * 4) as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("offscreen readback 1"),
                    size: (w * h * 4) as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
            ];
            self.readback_idx = 0;
            self.readback_rx = None;
            self.readback_mapped = [false; 2];
            self.offscreen = Some(Offscreen {
                texture,
                view,
                readback,
                width: w,
                height: h,
            });
        }
        self.offscreen.as_mut().expect("offscreen")
    }

    /// Create a D3D12 shared texture that can be imported by another process.
    /// Returns the wgpu Texture, its TextureView, and the raw HANDLE as i64.
    #[cfg(target_os = "windows")]
    pub fn create_shared_texture(&mut self, w: u32, h: u32) -> Option<SharedTexture> {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Graphics::Direct3D12::*;
        use windows::Win32::Graphics::Dxgi::Common::*;

        let hal_device = unsafe { self.device.as_hal::<wgpu::hal::api::Dx12>() }?;
        let raw_device = hal_device.raw_device();

        let resource_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Alignment: 0,
            Width: w as u64,
            Height: h,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            Flags: D3D12_RESOURCE_FLAG_NONE,
        };

        let heap_props = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            ..Default::default()
        };

        let mut resource: Option<ID3D12Resource> = None;
        unsafe {
            raw_device
                .CreateCommittedResource(
                    &heap_props,
                    D3D12_HEAP_FLAG_SHARED,
                    &resource_desc,
                    D3D12_RESOURCE_STATE_COMMON,
                    None,
                    &mut resource,
                )
                .ok()?;

            let shared_handle = raw_device
                .CreateSharedHandle(resource.as_ref().unwrap(), None, 0, None)
                .ok()?;

            let handle_val = shared_handle.0 as i64;

            let hal_texture = wgpu::hal::dx12::Device::texture_from_raw(
                resource.unwrap(),
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureDimension::D2,
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                1,
                1,
            );

            let texture = self.device.create_texture_from_hal::<wgpu::hal::api::Dx12>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("shared_texture"),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: TEX_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::TextureUses::COLOR_TARGET | wgpu::TextureUses::RESOURCE,
            );

            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            Some(SharedTexture {
                texture,
                view,
                handle: handle_val,
                width: w,
                height: h,
            })
        }
    }

    /// Create a double-buffered pair of shared GPU textures for zero-copy viewport.
    pub fn create_shared_texture_pair(&mut self, w: u32, h: u32) -> Option<SharedTexturePair> {
        let a = self.create_shared_texture(w, h)?;
        let b = self.create_shared_texture(w, h)?;
        Some(SharedTexturePair {
            buffers: [a, b],
            read_idx: 0,
        })
    }

    /// Render the scene directly into a shared GPU texture (zero-copy viewport).
    /// The texture was created with D3D12_SHARED so the editor can import it without CPU readback.
    pub fn render_into_shared(&mut self, snapshot: &RenderSnapshot, shared: &SharedTexture) {
        self.ensure_depth(shared.width, shared.height);
        let mut encoder = self.encode_frame(snapshot, &shared.view, TEX_FORMAT, None);
        self.queue.submit([encoder.finish()]);
        self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    fn ensure_depth(&mut self, w: u32, h: u32) {
        let needs = match &self.depth_texture {
            Some(tex) => tex.width() != w || tex.height() != h,
            None => true,
        };
        if needs {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d {
                    width: w.max(1),
                    height: h.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.depth_texture = Some(texture);
            self.depth_view = Some(view);
        }
    }

    /// Загрузка новых спрайтов из снапшота в атлас (dedup по указателю).
    fn upload_sprites(&mut self, snapshot: &RenderSnapshot) {
        for inst in snapshot.instances.iter().chain(snapshot.hud.iter()) {
            let Some(sprite) = &inst.sprite else {
                continue;
            };
            let ptr = Arc::as_ptr(sprite) as usize;
            if self.atlas.contains_key(&ptr) {
                continue;
            }
            let cell = match self.free_cells.pop() {
                Some(cell) => cell,
                None => {
                    self.clear_atlas();
                    let Some(cell) = self.free_cells.pop() else {
                        // Атлас всё ещё полон — рисуем спрайт прозрачным в этом кадре.
                        continue;
                    };
                    cell
                }
            };
            let (w, h, data) = fit_sprite(sprite);
            let cell_x = (cell as u32 % SPRITE_COLS) * SPRITE_CELL;
            let cell_y = (cell as u32 / SPRITE_COLS) * SPRITE_CELL;
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.sprite_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: cell_x,
                        y: cell_y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
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
            let uv = [
                cell_x as f32 / SPRITE_ATLAS_SIZE as f32,
                cell_y as f32 / SPRITE_ATLAS_SIZE as f32,
                (cell_x + w) as f32 / SPRITE_ATLAS_SIZE as f32,
                (cell_y + h) as f32 / SPRITE_ATLAS_SIZE as f32,
            ];
            self.atlas.insert(ptr, SpriteSlot { cell, uv });
        }
    }

    fn clear_atlas(&mut self) {
        self.atlas.clear();
        self.free_cells = (0..SPRITE_CELLS).collect();
    }

    fn build_instances(&self, snapshot: &RenderSnapshot) -> Vec<GpuInstance> {
        self.build_instances_from(&snapshot.instances)
    }

    fn build_instances_from(&self, instances: &[Instance]) -> Vec<GpuInstance> {
        instances
            .iter()
            .map(|inst| {
                let uv = match (inst.tex, &inst.sprite) {
                    (TexSel::Sprite, Some(sprite)) => {
                        let ptr = Arc::as_ptr(sprite) as usize;
                        self.atlas
                            .get(&ptr)
                            .map(|s| s.uv)
                            .unwrap_or([0.0, 0.0, 1.0, 1.0])
                    }
                    _ => inst.uv,
                };
                GpuInstance {
                    x: inst.x,
                    y: inst.y,
                    rot: inst.rot,
                    hw: inst.hw,
                    hh: inst.hh,
                    _pad0: [0.0; 3],
                    uv,
                    color: inst.color,
                    shape: inst.shape as u32,
                    _pad: [0; 3],
                }
            })
            .collect()
    }

    fn build_hud_instances(&self, snapshot: &RenderSnapshot) -> Vec<GpuInstance> {
        snapshot
            .hud
            .iter()
            .map(|inst| {
                let uv = match (inst.tex, &inst.sprite) {
                    (TexSel::Sprite, Some(sprite)) => {
                        let ptr = Arc::as_ptr(sprite) as usize;
                        self.atlas
                            .get(&ptr)
                            .map(|s| s.uv)
                            .unwrap_or([0.0, 0.0, 1.0, 1.0])
                    }
                    _ => inst.uv,
                };
                GpuInstance {
                    x: inst.x,
                    y: inst.y,
                    rot: inst.rot,
                    hw: inst.hw,
                    hh: inst.hh,
                    _pad0: [0.0; 3],
                    uv,
                    color: inst.color,
                    shape: inst.shape as u32,
                    _pad: [0; 3],
                }
            })
            .collect()
    }

    fn write_buffers(&mut self, instances: &[GpuInstance], view_proj: [[f32; 4]; 4]) {
        let globals = Globals {
            screen: [frame_width() as f32, frame_height() as f32],
            exposure: 1.0,
            tonemap_mode: 0.0,
            view_proj,
        };
        self.queue
            .write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));
        self.queue
            .write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(instances));
    }

    fn write_hud_buffers(&mut self, instances: &[GpuInstance], view_proj: [[f32; 4]; 4]) {
        let globals = Globals {
            screen: [frame_width() as f32, frame_height() as f32],
            exposure: 1.0,
            tonemap_mode: 0.0,
            view_proj,
        };
        self.queue
            .write_buffer(&self.hud_globals_buf, 0, bytemuck::bytes_of(&globals));
        self.queue
            .write_buffer(&self.hud_instance_buf, 0, bytemuck::cast_slice(instances));
    }

    fn build_pipeline(&mut self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("shader"),
                source: wgpu::ShaderSource::Wgsl(self.shaders.sprite.clone().into()),
            });
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("main pipeline"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
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
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }

    fn build_sky_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("sky shader"),
                source: wgpu::ShaderSource::Wgsl(self.shaders.sky.clone().into()),
            });
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sky pipeline"),
                layout: Some(&self.sky_pipeline_layout),
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

    fn build_cloud_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("cloud shader"),
                source: wgpu::ShaderSource::Wgsl(self.shaders.cloud.clone().into()),
            });
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("cloud pipeline"),
                layout: Some(&self.cloud_pipeline_layout),
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

    fn build_storm_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("storm shader"),
                source: wgpu::ShaderSource::Wgsl(self.shaders.storm.clone().into()),
            });
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("storm pipeline"),
                layout: Some(&self.storm_pipeline_layout),
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

    fn build_mesh_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("mesh shader"),
                source: wgpu::ShaderSource::Wgsl(self.shaders.mesh.clone().into()),
            });
        let vertex_buf_layout = wgpu::VertexBufferLayout {
            array_stride: size_of::<MeshVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    format: wgpu::VertexFormat::Float32x3,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    offset: 12,
                    format: wgpu::VertexFormat::Float32x3,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    offset: 24,
                    format: wgpu::VertexFormat::Float32x2,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    format: wgpu::VertexFormat::Float32,
                    shader_location: 3,
                },
            ],
        };
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mesh pipeline"),
                layout: Some(&self.mesh_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Some(vertex_buf_layout)],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    polygon_mode: if self.wireframe {
                        wgpu::PolygonMode::Line
                    } else {
                        wgpu::PolygonMode::Fill
                    },
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
        pipeline
    }

    fn build_mesh_array_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("mesh array shader"),
                source: wgpu::ShaderSource::Wgsl(self.shaders.mesh_array.clone().into()),
            });
        let vertex_buf_layout = wgpu::VertexBufferLayout {
            array_stride: size_of::<MeshVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    format: wgpu::VertexFormat::Float32x3,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    offset: 12,
                    format: wgpu::VertexFormat::Float32x3,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    offset: 24,
                    format: wgpu::VertexFormat::Float32x2,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    format: wgpu::VertexFormat::Float32,
                    shader_location: 3,
                },
            ],
        };
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mesh array pipeline"),
                layout: Some(&self.mesh_array_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Some(vertex_buf_layout)],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    polygon_mode: if self.wireframe {
                        wgpu::PolygonMode::Line
                    } else {
                        wgpu::PolygonMode::Fill
                    },
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
    }

    fn build_terrain_splat_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("terrain splat shader"),
                source: wgpu::ShaderSource::Wgsl(self.shaders.terrain_splat.clone().into()),
            });
        let vertex_buf_layout = wgpu::VertexBufferLayout {
            array_stride: size_of::<MeshVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute { offset: 0, format: wgpu::VertexFormat::Float32x3, shader_location: 0 },
                wgpu::VertexAttribute { offset: 12, format: wgpu::VertexFormat::Float32x3, shader_location: 1 },
                wgpu::VertexAttribute { offset: 24, format: wgpu::VertexFormat::Float32x2, shader_location: 2 },
                wgpu::VertexAttribute { offset: 32, format: wgpu::VertexFormat::Float32, shader_location: 3 },
            ],
        };
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain splat pipeline"),
            layout: Some(&self.terrain_splat_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(vertex_buf_layout)],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: if self.wireframe { wgpu::PolygonMode::Line } else { wgpu::PolygonMode::Fill },
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    pub fn upload_mesh(&self, mesh: &deerpunk_engine_common::resource::mesh::Mesh) -> GpuMesh {
        let mut vertices = Vec::with_capacity(mesh.vertex_count());
        for i in 0..mesh.vertex_count() {
            vertices.push(MeshVertex {
                position: mesh.positions[i],
                normal: mesh
                    .normals
                    .get(i)
                    .copied()
                    .unwrap_or(deerpunk_engine_common::coord::UP),
                uv: mesh.uvs.get(i).copied().unwrap_or([0.0, 0.0]),
                layer: mesh.tex_layers.get(i).copied().unwrap_or(0.0),
            });
        }
        let vertex_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let index_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh indices"),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        GpuMesh {
            vertex_buf,
            index_buf,
            index_count: mesh.indices.len() as u32,
        }
    }

    fn mesh_hash(mesh: &deerpunk_engine_common::resource::mesh::Mesh) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for p in &mesh.positions {
            bytemuck::cast_slice::<f32, u8>(p).hash(&mut hasher);
        }
        for n in &mesh.normals {
            bytemuck::cast_slice::<f32, u8>(n).hash(&mut hasher);
        }
        for u in &mesh.uvs {
            bytemuck::cast_slice::<f32, u8>(u).hash(&mut hasher);
        }
        for l in &mesh.tex_layers {
            l.to_bits().hash(&mut hasher);
        }
        mesh.indices.hash(&mut hasher);
        hasher.finish()
    }

    /// Fast path: cache by `Arc` identity (no per-frame vertex hashing).
    pub fn ensure_mesh_uploaded_arc(
        &mut self,
        mesh: &Arc<deerpunk_engine_common::resource::mesh::Mesh>,
    ) -> crate::mesh_allocator::MeshHandle {
        let key = Arc::as_ptr(mesh) as usize;
        if let Some(&handle) = self.mesh_arc_cache.get(&key) {
            return handle;
        }
        let handle = self.ensure_mesh_uploaded(mesh.as_ref());
        self.mesh_arc_cache.insert(key, handle);
        handle
    }

    pub fn ensure_mesh_uploaded(
        &mut self,
        mesh: &deerpunk_engine_common::resource::mesh::Mesh,
    ) -> crate::mesh_allocator::MeshHandle {
        let hash = Self::mesh_hash(mesh);
        if let Some(&handle) = self.mesh_cache.get(&hash) {
            return handle;
        }

        let vertex_size = std::mem::size_of::<MeshVertex>();
        let mut vertices = Vec::with_capacity(mesh.vertex_count() * vertex_size);
        for i in 0..mesh.vertex_count() {
            vertices.extend_from_slice(bytemuck::bytes_of(&MeshVertex {
                position: mesh.positions[i],
                normal: mesh
                    .normals
                    .get(i)
                    .copied()
                    .unwrap_or(deerpunk_engine_common::coord::UP),
                uv: mesh.uvs.get(i).copied().unwrap_or([0.0, 0.0]),
                layer: mesh.tex_layers.get(i).copied().unwrap_or(0.0),
            }));
        }

        let handle = self.mesh_allocator.allocate(
            &self.device,
            &self.queue,
            &vertices,
            &mesh.indices,
        );
        self.mesh_cache.insert(hash, handle);
        let verts = handle.vertex_count;
        let indices = handle.index_count;
        let mesh_id = ASSET_ID_MESH | self.next_mesh_asset_id as u64;
        self.next_mesh_asset_id = self.next_mesh_asset_id.wrapping_add(1);
        let size = verts as u64 * MESH_VERTEX_SIZE as u64 + indices as u64 * MESH_INDEX_SIZE as u64;
        self.asset_meta.insert(
            mesh_id,
            AssetEntry {
                id: mesh_id,
                kind: AssetKind::Mesh,
                name: format!("mesh#{hash:x}"),
                size,
                width: None,
                height: None,
                layers: None,
                verts: Some(verts),
                indices: Some(indices),
            },
        );
        handle
    }

    /// Upload a square texture array (each layer = `size×size` RGBA). Returns array id for MeshDraw.tex_array.
    pub fn upload_texture_array(&mut self, size: u32, layers: &[Vec<u8>]) -> u32 {
        let size = size.max(1);
        let n = layers.len().max(1) as u32;
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mesh tex array"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: n,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let expected = (size * size * 4) as usize;
        for (i, layer) in layers.iter().enumerate() {
            let bytes = if layer.len() >= expected {
                &layer[..expected]
            } else {
                continue;
            };
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: i as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * size),
                    rows_per_image: Some(size),
                },
                wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = tex.create_view(&wgpu::TextureViewDescriptor {
            label: Some("mesh tex array view"),
            format: None,
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: 0,
            array_layer_count: Some(n),
            usage: None,
        });
        let slot = self.next_array_slot;
        self.next_array_slot += 1;
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh array albedo bg"),
            layout: &self.tex_array_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.repeat_sampler),
                },
            ],
        });
        self.mesh_array_bgs.insert(slot, bg);
        // Keep view alive via texture_views under high ids — store as dummy; BG holds view.
        // Actually BindGroup holds references... view must be owned. Put in texture_views with offset.
        let view_id = 0x8000_0000 | slot;
        self.texture_views.insert(view_id, view);
        let asset_id = ASSET_ID_TEX_ARRAY | slot as u64;
        let bytes = size as u64 * size as u64 * 4 * n as u64;
        self.asset_meta.insert(
            asset_id,
            AssetEntry {
                id: asset_id,
                kind: AssetKind::TextureArray,
                name: format!("tex_array#{slot}"),
                size: bytes,
                width: Some(size),
                height: Some(size),
                layers: Some(n),
                verts: None,
                indices: None,
            },
        );
        slot
    }

    pub fn upload_texture(&mut self, pixels: &[u8], w: u32, h: u32) -> MeshTexture {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pbr tex"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let slot = self.next_texture_slot;
        self.next_texture_slot += 1;

        // Group 1 bind group for albedo texture (same as mesh_user_bgs pattern)
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pbr albedo bg"),
            layout: &self.tex_bg_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.mesh_user_bgs.insert(slot, bg);

        self.texture_views.insert(slot, view);
        let asset_id = slot as u64;
        self.asset_meta.insert(
            asset_id,
            AssetEntry {
                id: asset_id,
                kind: AssetKind::Texture2d,
                name: format!("tex#{slot}"),
                size: w as u64 * h as u64 * 4,
                width: Some(w),
                height: Some(h),
                layers: Some(1),
                verts: None,
                indices: None,
            },
        );
        MeshTexture(slot)
    }

    fn ensure_material_bg(&mut self, material_id: u32, normal_slot: MeshTexture, orm_slot: MeshTexture) -> &wgpu::BindGroup {
        let idx = material_id as usize;
        while self.material_bgs.len() <= idx {
            self.material_bgs.push(None);
        }
        if self.material_bgs[idx].is_none() {
            let normal_view = if normal_slot == MeshTexture::WHITE {
                &self.flat_normal_tex_view
            } else {
                self.texture_views
                    .get(&normal_slot.0)
                    .unwrap_or(&self.flat_normal_tex_view)
            };
            let orm_view = self.texture_views.get(&orm_slot.0).unwrap_or(&self.white_tex_view);
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("material bg {}", material_id)),
                layout: &self.material_bg_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.material_allocator.gpu_buffer().as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(normal_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(orm_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.material_bgs[idx] = Some(bg);
        }
        self.material_bgs[idx].as_ref().unwrap()
    }

    fn invalidate_material_bgs(&mut self) {
        for bg in &mut self.material_bgs {
            *bg = None;
        }
    }

    pub fn draw_mesh(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        gpu_mesh: &GpuMesh,
        model_mat: [[f32; 4]; 4],
        view_proj: [[f32; 4]; 4],
        material_id: u32,
        texture: MeshTexture,
        format: wgpu::TextureFormat,
    ) {
        let uniform = MeshUniform {
            view_proj,
            model: model_mat,
            material_data: [material_id, 0, 0, 0],
            camera_pos: [0.0, 0.0, 0.0],
            _pad_cam: 0.0,
        };
        self.queue
            .write_buffer(&self.mesh_globals_buf, 0, bytemuck::bytes_of(&uniform));

        if !self.mesh_pipelines.contains_key(&format) {
            let p = self.build_mesh_pipeline(format);
            self.mesh_pipelines.insert(format, p);
        }

        self.ensure_depth(frame_width(), frame_height());

        let pipeline = self.mesh_pipelines.get(&format).unwrap();

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mesh pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: self.depth_view.as_ref().unwrap(),
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.mesh_globals_bg, &[]);
        {
            let bg = self.mesh_bind_group(texture);
            pass.set_bind_group(1, bg, &[]);
        }
        {
            let mat_bg = self.ensure_material_bg(material_id, MeshTexture::WHITE, MeshTexture::WHITE);
            pass.set_bind_group(2, mat_bg, &[]);
        }
        pass.set_vertex_buffer(0, gpu_mesh.vertex_buf.slice(..));
        pass.set_index_buffer(gpu_mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..gpu_mesh.index_count, 0, 0..1);
    }

    /// Bind-group с текстурой для меша (WHITE → 1x1 белая).
    fn mesh_bind_group(&self, texture: MeshTexture) -> &wgpu::BindGroup {
        if texture == MeshTexture::WHITE {
            return &self.mesh_white_bg;
        }
        self.mesh_user_bgs
            .get(&texture.0)
            .unwrap_or(&self.mesh_white_bg)
    }

    /// Загружает атлас текстур карты на GPU.
    pub fn upload_map_atlas(&mut self, pixels: &[u8], w: u32, h: u32) {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("map atlas"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEX_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
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
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("map atlas bg"),
            layout: &self.tex_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.mesh_user_bgs.insert(MeshTexture::slot(1).0, bg);
    }

    /// Загружает текстуру для меша на GPU (slot — номер слота текстуры).
    pub fn upload_mesh_texture(&mut self, slot: MeshTexture, pixels: &[u8], w: u32, h: u32) {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mesh texture"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEX_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
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
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh texture bg"),
            layout: &self.tex_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.mesh_user_bgs.insert(slot.0, bg);
    }

    fn encode_frame(
        &mut self,
        snapshot: &RenderSnapshot,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        surface_view: Option<&wgpu::TextureView>,
    ) -> wgpu::CommandEncoder {
        self.upload_sprites(snapshot);
        self.material_allocator.flush(&self.queue);
        if self.material_allocator.generation != self.material_bg_gen {
            self.material_bg_gen = self.material_allocator.generation;
            self.invalidate_material_bgs();
        }
        let pipeline = match self.pipelines.get(&format) {
            Some(p) => p.clone(),
            None => {
                let p = self.build_pipeline(format);
                self.pipelines.insert(format, p.clone());
                p
            }
        };

        let w = frame_width() as f32;
        let h = frame_height() as f32;
        let hud_vp: [[f32; 4]; 4] = [
            [2.0 / w, 0.0, 0.0, 0.0],
            [0.0, -2.0 / h, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0, 1.0],
        ];

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });

        // --- Pass 0: sky background (clear + fullscreen triangle) ---
        if let Some(sky) = &snapshot.sky {
            let inv_vp = deerpunk_engine_common::coord::mat4_inverse(snapshot.view_proj);
            let mut params = [[0.0f32; 4]; 8];
            for i in 0..30 {
                params[i / 4][i % 4] = sky.params[i];
            }
            let uniform = SkyUniform {
                inv_view_proj: inv_vp,
                view_proj: snapshot.view_proj,
                sun_dir: [sky.sun_dir[0], sky.sun_dir[1], sky.sun_dir[2], 0.0],
                camera_pos: [sky.camera_pos[0], sky.camera_pos[1], sky.camera_pos[2], 1.0],
                params,
            };
            self.queue
                .write_buffer(&self.sky_globals_buf, 0, bytemuck::bytes_of(&uniform));

            if !self.sky_pipelines.contains_key(&format) {
                let p = self.build_sky_pipeline(format);
                self.sky_pipelines.insert(format, p);
            }
            let pipeline = self.sky_pipelines.get(&format).unwrap();

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
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.sky_globals_bg, &[]);
            pass.draw(0..3, 0..1);
        } else {
            // No sky — just clear to background color
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear pass"),
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
        }

        // --- Pass 0.5: volumetric clouds (after sky, before 3D) ---
        if let Some(sky) = &snapshot.sky {
            let inv_vp = deerpunk_engine_common::coord::mat4_inverse(snapshot.view_proj);
            let cloud_uniform = CloudUniform {
                inv_view_proj: inv_vp,
                camera_pos: [sky.camera_pos[0], sky.camera_pos[1], sky.camera_pos[2], 0.0],
                sun_dir: [sky.sun_dir[0], sky.sun_dir[1], sky.sun_dir[2], 0.0],
                params: [0.45, 1.0, 200.0, 0.015],
                time: [snapshot.game_time, 0.0, 0.0, 0.0],
                sky_color: [sky.sky_color[0], sky.sky_color[1], sky.sky_color[2], 0.0],
            };
            self.queue.write_buffer(
                &self.cloud_uniform_buf,
                0,
                bytemuck::bytes_of(&cloud_uniform),
            );

            if !self.cloud_pipelines.contains_key(&format) {
                let p = self.build_cloud_pipeline(format);
                self.cloud_pipelines.insert(format, p);
            }
            let pipeline = self.cloud_pipelines.get(&format).unwrap();

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
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.cloud_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // --- Pass 1: 3D scene (clear + draw) ---
        let gpu_insts = self.build_instances_from(&snapshot.instances);
        self.write_buffers(&gpu_insts, snapshot.view_proj);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d pass"),
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
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            Self::draw_batched(
                &mut pass,
                &snapshot.instances,
                &self.sprite_bg,
                &self.map_bg,
                &self.font_bg,
            );
        }

        // --- Mesh draws (load, no clear, single pass with dynamic offsets) ---
        if !snapshot.mesh_draws.is_empty() {
            let mesh_align = 256u64;
            let mesh_uniform_size = ((std::mem::size_of::<MeshUniform>() as u64 + mesh_align - 1)
                / mesh_align)
                * mesh_align;

            // Resolve material IDs first (needs &mut material_allocator).
            let resolved_ids: Vec<u32> = snapshot.mesh_draws.iter().map(|md| {
                if let Some(ref mat) = md.material {
                    self.material_allocator.ensure_uploaded(&self.device, &self.queue, mat)
                } else {
                    md.material_id
                }
            }).collect();

            // Pre-fill all uniforms into the buffer at aligned offsets
            for (i, md) in snapshot.mesh_draws.iter().enumerate() {
                let uniform = MeshUniform {
                    view_proj: snapshot.view_proj,
                    model: md.model_mat,
                    material_data: [resolved_ids[i], md.tex_layer, 0, 0],
                    camera_pos: snapshot.camera_pos,
                    _pad_cam: 0.0,
                };
                self.queue.write_buffer(
                    &self.mesh_globals_buf,
                    i as u64 * mesh_uniform_size,
                    bytemuck::bytes_of(&uniform),
                );
            }

            // Upload meshes + material BGs before the pass (needs &mut self).
            let mesh_handles: Vec<_> = snapshot
                .mesh_draws
                .iter()
                .map(|md| self.ensure_mesh_uploaded_arc(&md.mesh))
                .collect();
            for (i, md) in snapshot.mesh_draws.iter().enumerate() {
                let _ = self.ensure_material_bg(resolved_ids[i], md.normal, md.orm);
            }

            {
                let mut lights = LightsUniform::zeroed();
                let n = snapshot.point_lights.len().min(MAX_POINT_LIGHTS);
                lights.count = n as u32;
                for (i, pl) in snapshot.point_lights.iter().take(n).enumerate() {
                    lights.lights[i] = GpuPointLight {
                        position: pl.position,
                        range: pl.range.max(1.0),
                        color: pl.color,
                        intensity: pl.intensity,
                    };
                }
                self.queue
                    .write_buffer(&self.lights_buf, 0, bytemuck::bytes_of(&lights));
            }

            self.ensure_depth(frame_width(), frame_height());

            if !self.mesh_pipelines.contains_key(&format) {
                let p = self.build_mesh_pipeline(format);
                self.mesh_pipelines.insert(format, p);
            }
            if !self.mesh_array_pipelines.contains_key(&format) {
                let p = self.build_mesh_array_pipeline(format);
                self.mesh_array_pipelines.insert(format, p);
            }

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mesh pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.depth_view.as_ref().unwrap(),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(3, &self.lights_bg, &[]);

            for (i, md) in snapshot.mesh_draws.iter().enumerate() {
                let handle = mesh_handles[i];
                let material_id = resolved_ids[i];
                let offset = (i as u64 * mesh_uniform_size) as u32;
                pass.set_bind_group(0, &self.mesh_globals_bg, &[offset]);

                if let Some(array_id) = md.tex_array {
                    let pipeline = self.mesh_array_pipelines.get(&format).unwrap();
                    pass.set_pipeline(pipeline);
                    if let Some(bg) = self.mesh_array_bgs.get(&array_id) {
                        pass.set_bind_group(1, bg, &[]);
                    } else {
                        continue;
                    }
                } else {
                    let pipeline = self.mesh_pipelines.get(&format).unwrap();
                    pass.set_pipeline(pipeline);
                    let bg = self.mesh_bind_group(md.texture);
                    pass.set_bind_group(1, bg, &[]);
                }
                if let Some(mat_bg) = self.material_bgs.get(material_id as usize).and_then(|s| s.as_ref()) {
                    pass.set_bind_group(2, mat_bg, &[]);
                }
                let vb = self.mesh_allocator.vertex_buffer(handle);
                let ib = self.mesh_allocator.index_buffer(handle);
                pass.set_vertex_buffer(0, vb.slice(self.mesh_allocator.vertex_byte_range(handle)));
                pass.set_index_buffer(ib.slice(self.mesh_allocator.index_byte_range(handle)), wgpu::IndexFormat::Uint32);
                let start = md.first_index;
                let count = if md.index_count == 0 {
                    handle.index_count
                } else {
                    md.index_count.min(handle.index_count.saturating_sub(start))
                };
                if count > 0 {
                    pass.draw_indexed(start..start + count, 0, 0..1);
                }
            }
        }

        // --- Terrain splat pass ---
        if !snapshot.terrain_splat_draws.is_empty() {
            if !self.terrain_splat_pipelines.contains_key(&format) {
                let p = self.build_terrain_splat_pipeline(format);
                self.terrain_splat_pipelines.insert(format, p);
            }

            let uniform_size = 128u64;
            let vp: [[f32; 4]; 4] = snapshot.view_proj;

            for (i, td) in snapshot.terrain_splat_draws.iter().enumerate() {
                let model = td.model_mat;
                let mvp = glam::Mat4::from_cols_array_2d(&vp) * glam::Mat4::from_cols_array_2d(&model);
                let uniform = MeshUniform {
                    view_proj: mvp.to_cols_array_2d(),
                    model,
                    material_data: [0, 0, 0, 0],
                    camera_pos: snapshot.camera_pos,
                    _pad_cam: 0.0,
                };
                self.queue.write_buffer(
                    &self.mesh_globals_buf,
                    i as u64 * uniform_size,
                    bytemuck::bytes_of(&uniform),
                );
            }

            let terrain_handles: Vec<_> = snapshot
                .terrain_splat_draws
                .iter()
                .map(|td| self.ensure_mesh_uploaded_arc(&td.mesh))
                .collect();

            let pipeline = self.terrain_splat_pipelines.get(&format).unwrap();

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terrain splat pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.depth_view.as_ref().unwrap(),
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
            pass.set_pipeline(pipeline);

            for (i, td) in snapshot.terrain_splat_draws.iter().enumerate() {
                let handle = terrain_handles[i];
                let offset = (i as u64 * uniform_size) as u32;
                pass.set_bind_group(0, &self.mesh_globals_bg, &[offset]);

                let layer_views: [&wgpu::TextureView; 4] = std::array::from_fn(|j| {
                    self.texture_views.get(&td.layers[j].0).unwrap_or(&self.white_tex_view)
                });
                let splat_view = self.texture_views.get(&td.splat_map.0).unwrap_or(&self.white_tex_view);
                let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("terrain splat bg"),
                    layout: &self.terrain_splat_bg_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(layer_views[0]) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(layer_views[1]) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(layer_views[2]) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(layer_views[3]) },
                        wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(splat_view) },
                        wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    ],
                });
                pass.set_bind_group(1, &bg, &[]);

                let vb = self.mesh_allocator.vertex_buffer(handle);
                let ib = self.mesh_allocator.index_buffer(handle);
                pass.set_vertex_buffer(0, vb.slice(self.mesh_allocator.vertex_byte_range(handle)));
                pass.set_index_buffer(ib.slice(self.mesh_allocator.index_byte_range(handle)), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..handle.index_count, 0, 0..1);
            }
        }

        // --- Debug lines pass ---
        {
            let verts = crate::debug_draw::collect_vertices(snapshot, 0.04, 120000);
            if !verts.is_empty() {
                // Ленивая инициализация debug pipeline
                if self.debug_pipeline.is_none() {
                    let shader = self
                        .device
                        .create_shader_module(wgpu::ShaderModuleDescriptor {
                            label: Some("debug shader"),
                            source: wgpu::ShaderSource::Wgsl(self.shaders.debug.clone().into()),
                        });
                    self.debug_pipeline = Some(
                        self.device
                            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                                label: Some("debug pipeline"),
                                layout: Some(&self.sky_pipeline_layout),
                                vertex: wgpu::VertexState {
                                    module: &shader,
                                    entry_point: Some("vs_main"),
                                    compilation_options: Default::default(),
                                    buffers: &[Some(wgpu::VertexBufferLayout {
                                        array_stride: std::mem::size_of::<
                                            crate::debug_draw::DebugVertex,
                                        >()
                                            as u64,
                                        step_mode: wgpu::VertexStepMode::Vertex,
                                        attributes: &[
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
                                        ],
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
                                depth_stencil: None,
                                multisample: wgpu::MultisampleState::default(),
                                fragment: Some(wgpu::FragmentState {
                                    module: &shader,
                                    entry_point: Some("fs_main"),
                                    compilation_options: Default::default(),
                                    targets: &[Some(wgpu::ColorTargetState {
                                        format,
                                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                                        write_mask: wgpu::ColorWrites::ALL,
                                    })],
                                }),
                                multiview_mask: None,
                                cache: None,
                            }),
                    );
                }

                if let Some(ref pipeline) = self.debug_pipeline {
                    self.queue.write_buffer(
                        &self.debug_vertex_buf,
                        0,
                        bytemuck::cast_slice(&verts),
                    );
                    // view_proj уже записан в sky_globals_buf на earlier pass
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("debug pass"),
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
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &self.sky_globals_bg, &[]);
                    pass.set_vertex_buffer(0, self.debug_vertex_buf.slice(..));
                    pass.draw(0..verts.len() as u32, 0..1);
                }
            }
        }

        // --- Storm pass: DISABLED (pipeline format mismatch: hdr_format vs surface format) ---
        // if let Some(ref storm_data) = snapshot.storm {
        // if !storm_data.infected.is_empty() {
        //     // Upload infected bitmap to GPU texture
        //     self.queue.write_texture(
        //         wgpu::TexelCopyTextureInfo {
        //             texture: &self.storm_texture,
        //             mip_level: 0,
        //             origin: wgpu::Origin3d::ZERO,
        //             aspect: wgpu::TextureAspect::All,
        //         },
        //         &storm_data.infected,
        //         wgpu::TexelCopyBufferLayout {
        //             offset: 0,
        //             bytes_per_row: Some(64),
        //             rows_per_image: Some(64),
        //         },
        //         wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 },
        //     );
        //
        //     // Upload storm uniform
        //     let uniform = StormUniform {
        //         inv_view_proj: snapshot.inv_view_proj,
        //         camera_pos: [0.0; 4],
        //         params: [64.0, 4.0, snapshot.game_time, 0.0], // map_size=64, tile_meters=4.0
        //     };
        //     self.queue.write_buffer(&self.storm_uniform_buf, 0, bytemuck::bytes_of(&uniform));
        //
        //     // Rebuild bind group (texture view may have changed)
        //     self.storm_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
        //         label: Some("storm bg"),
        //         layout: &self.storm_bg_layout,
        //         entries: &[
        //             wgpu::BindGroupEntry {
        //                 binding: 0,
        //                 resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
        //                     buffer: &self.storm_uniform_buf,
        //                     offset: 0,
        //                     size: None,
        //                 }),
        //             },
        //             wgpu::BindGroupEntry {
        //                 binding: 1,
        //                 resource: wgpu::BindingResource::TextureView(&self.storm_view),
        //             },
        //             wgpu::BindGroupEntry {
        //                 binding: 2,
        //                 resource: wgpu::BindingResource::Sampler(&self.storm_sampler),
        //             },
        //         ],
        //     });
        //
        //     let format = self.hdr_format;
        //     if !self.storm_pipelines.contains_key(&format) {
        //         let p = self.build_storm_pipeline(format);
        //         self.storm_pipelines.insert(format, p);
        //     }
        //     let pipeline = self.storm_pipelines.get(&format).unwrap();
        //
        //     let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        //         label: Some("storm pass"),
        //         color_attachments: &[Some(wgpu::RenderPassColorAttachment {
        //             view,
        //             depth_slice: None,
        //             resolve_target: None,
        //             ops: wgpu::Operations {
        //                 load: wgpu::LoadOp::Load,
        //                 store: wgpu::StoreOp::Store,
        //             },
        //         })],
        //         depth_stencil_attachment: None,
        //         timestamp_writes: None,
        //         occlusion_query_set: None,
        //         multiview_mask: None,
        //     });
        //     pass.set_pipeline(pipeline);
        //     pass.set_bind_group(0, &self.storm_bg, &[]);
        //     pass.draw(0..3, 0..1);
        // }
        // }

        // --- Pass 2: HUD overlay (load, no clear) ---
        if !snapshot.hud.is_empty() {
            let gpu_hud = self.build_instances_from(&snapshot.hud);
            self.write_hud_buffers(&gpu_hud, hud_vp);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hud pass"),
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
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &self.hud_globals_bg, &[]);
            Self::draw_batched(
                &mut pass,
                &snapshot.hud,
                &self.sprite_bg,
                &self.map_bg,
                &self.font_bg,
            );
        }

        // --- Luminance pass: HDR → 64x64 log-lum (for auto-exposure) ---
        if let (Some(lum_view), Some(hdr_view)) = (self.lum_view.clone(), self.hdr_view.clone()) {
            self.ensure_lum_pipeline();
            let pipeline = self.lum_pipeline.as_ref().unwrap();
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("luminance bg"),
                layout: &self.lum_bg_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&hdr_view),
                }],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("luminance pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &lum_view,
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
            pass.set_bind_group(0, &bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // --- Tonemap pass: HDR → surface (only when rendering to window) ---
        if let Some(surface_view) = surface_view {
            self.ensure_tonemap_pipeline(format);
            let hdr_view_ref = self.hdr_view.as_ref().expect("HDR texture not initialized");
            let pipeline = self.tonemap_pipeline.as_ref().unwrap();

            let w = frame_width();
            let h = frame_height();
            let globals = Globals {
                screen: [w as f32, h as f32],
                exposure: self.exposure,
                tonemap_mode: self.tonemap_mode,
                view_proj: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
            };
            self.queue
                .write_buffer(&self.tonemap_uniform_buf, 0, bytemuck::bytes_of(&globals));

            let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("tonemap linear sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });

            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tonemap bg"),
                layout: &self.tonemap_bg_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(hdr_view_ref),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.tonemap_uniform_buf,
                            offset: 0,
                            size: None,
                        }),
                    },
                ],
            });
            self.tonemap_bg = Some(bg);

            {
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
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, self.tonemap_bg.as_ref().unwrap(), &[]);
                pass.draw(0..3, 0..1);
            }

            // UI overlay (RmlUi geometry) on LDR surface
            if let Some(ui) = self.pending_ui.take() {
                let device = &self.device;
                let queue = &self.queue;
                let ui_pass = &mut self.ui_pass;
                ui_pass.encode(device, queue, &mut encoder, surface_view, format, &ui);
            }
        }

        encoder
    }

    /// Queue a UI frame to be drawn after tonemap on the next `encode_frame`.
    pub fn set_ui_frame(&mut self, frame: Option<deerpunk_engine_common::ui_draw::UiFrame>) {
        self.pending_ui = frame;
    }

    fn draw_batched(
        pass: &mut wgpu::RenderPass<'_>,
        instances: &[Instance],
        sprite_bg: &wgpu::BindGroup,
        map_bg: &wgpu::BindGroup,
        font_bg: &wgpu::BindGroup,
    ) {
        let mut i = 0usize;
        while i < instances.len() {
            let tex = instances[i].tex;
            let start = i;
            while i < instances.len() && instances[i].tex == tex {
                i += 1;
            }
            let bg = match tex {
                TexSel::Sprite => sprite_bg,
                TexSel::Map => map_bg,
                _ => font_bg,
            };
            pass.set_bind_group(1, bg, &[]);
            pass.draw(0..4, start as u32..i as u32);
        }
    }

    pub fn render_clear(&self, view: &wgpu::TextureView, _format: wgpu::TextureFormat) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("clear_encoder"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1,
                            g: 0.2,
                            b: 0.3,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        self.queue.submit([encoder.finish()]);
    }
}

impl Default for Renderer {
    fn default() -> Self {
        panic!("Renderer::default() is not supported — use Renderer::new(shaders) instead");
    }
}

// `Shape` кастается в u32; завязка на номера вариантов в шейдере.
const _: () = {
    assert!(Shape::Texture as u32 == 0);
    assert!(Shape::Solid as u32 == 1);
    assert!(Shape::Circle as u32 == 2);
};
