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
use deerpunk_engine_common::material::ShaderId;

use crate::shader_registry::ShaderRegistry;
use tiny_skia::Pixmap;
use wgpu::util::DeviceExt;

use crate::asset_inventory::{AssetEntry, AssetKind, ASSET_ID_MATERIAL, ASSET_ID_MESH};
use crate::bone_gpu::{GpuBoneBlock, GPU_BONE_BLOCK_SIZE};
use crate::font::{build_font_atlas, ATLAS_H, ATLAS_W};
use crate::frame::context::FrameCtx;
use crate::mesh_allocator::{MESH_INDEX_SIZE, MESH_VERTEX_SIZE, SKINNED_MESH_VERTEX_SIZE};
use crate::render_profile::{FrameDrawAccum, FrameSpeedsHud, RenderFrameProfile};
use crate::snapshot::{Instance, MeshTexture, RenderSnapshot, Shape, TexSel};

#[path = "passes/mod.rs"]
mod passes;

/// `vsync=true` → FIFO (ждёт vblank). `false` → без ожидания vblank, если драйвер умеет.
fn select_present_mode(caps: &wgpu::SurfaceCapabilities, vsync: bool) -> wgpu::PresentMode {
    let modes = &caps.present_modes;
    if vsync {
        if modes.contains(&wgpu::PresentMode::Fifo) {
            return wgpu::PresentMode::Fifo;
        }
    } else if modes.contains(&wgpu::PresentMode::Immediate) {
        return wgpu::PresentMode::Immediate;
    } else if modes.contains(&wgpu::PresentMode::Mailbox) {
        // Не совсем «без vsync», но ниже задержка чем FIFO на части драйверов.
        return wgpu::PresentMode::Mailbox;
    }
    modes
        .first()
        .copied()
        .unwrap_or(wgpu::PresentMode::Fifo)
}

/// Mip count for a 2D texture (always ≥ 1).
#[inline]
fn texture_mip_levels(width: u32, height: u32) -> u32 {
    width.max(height).max(1).ilog2() + 1
}

fn downsample_rgba_half(src: &[u8], width: u32, height: u32) -> (u32, u32, Vec<u8>) {
    let out_w = (width / 2).max(1);
    let out_h = (height / 2).max(1);
    let mut out = vec![0u8; out_w as usize * out_h as usize * 4];
    for y in 0..out_h {
        for x in 0..out_w {
            let mut r = 0u32;
            let mut g = 0u32;
            let mut b = 0u32;
            let mut a = 0u32;
            let mut n = 0u32;
            for oy in 0..2u32 {
                for ox in 0..2u32 {
                    let sx = (x * 2 + ox).min(width - 1);
                    let sy = (y * 2 + oy).min(height - 1);
                    let i = ((sy * width + sx) * 4) as usize;
                    r += src[i] as u32;
                    g += src[i + 1] as u32;
                    b += src[i + 2] as u32;
                    a += src[i + 3] as u32;
                    n += 1;
                }
            }
            let o = ((y * out_w + x) * 4) as usize;
            out[o] = (r / n) as u8;
            out[o + 1] = (g / n) as u8;
            out[o + 2] = (b / n) as u8;
            out[o + 3] = (a / n) as u8;
        }
    }
    (out_w, out_h, out)
}

fn build_rgba_mipmap_chain(base: &[u8], width: u32, height: u32) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(width, height, base.to_vec())];
    let mut w = width;
    let mut h = height;
    while w > 1 || h > 1 {
        let (nw, nh, data) = downsample_rgba_half(&levels.last().expect("mip").2, w, h);
        levels.push((nw, nh, data));
        w = nw;
        h = nh;
    }
    levels
}

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
/// Dynamic uniform slots for batched `MeshUniform` (one per mesh draw). Was 256 → 64 KiB total.
const MESH_UNIFORM_SLOTS_INIT: u64 = 512;

/// Формат всех наших текстур и offscreen-таргета (без sRGB — байты как были
/// в CPU-пикспмапе, чтобы картинка совпадала с прежним рендерером).
const TEX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub struct ShaderSources {
    pub sprite: String,
    /// GPU skinned mesh pass (`mesh_skinned.wgsl`).
    pub mesh_skinned: String,
    pub terrain_splat: String,
    pub sky: String,
    pub cloud: String,
    pub debug: String,
    pub tonemap: String,
    pub luminance: String,
    pub storm: String,
    pub bloom_threshold: String,
    pub bloom_blur: String,
    pub bloom_composite: String,
    pub grain: String,
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
    /// Albedo atlas rect `[min_u, min_v, size_u, size_v]` (full tex → `[0,0,1,1]`).
    atlas_rect: [f32; 4],
    /// BSP lightmap UV (atlas). Unused meshes → [0,0].
    lm_uv: [f32; 2],
    /// Lightstyle indices (255 = unused slot).
    lm_styles: [f32; 4],
    _pad: [f32; 2],
}

const _: () = assert!(size_of::<MeshVertex>() == crate::mesh_allocator::MESH_VERTEX_SIZE as usize);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkinnedMeshVertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    joints: [u32; 4],
    weights: [f32; 4],
}

const _: () = assert!(size_of::<SkinnedMeshVertex>() == SKINNED_MESH_VERTEX_SIZE as usize);

pub struct GpuMesh {
    pub vertex_buf: wgpu::Buffer,
    pub index_buf: wgpu::Buffer,
    pub index_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct MeshUniform {
    view_proj: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    material_data: [u32; 4],
    /// World-space camera position (for correct view vector).
    camera_pos: [f32; 3],
    _pad_cam: f32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MeshBatchKey {
    shader_id: u32,
    lm_surface: bool,
    material_id: u32,
    texture: u32,
    normal: u32,
    orm: u32,
    lightmap_gpu: u32,
    buffer_id: u32,
}

pub(crate) const MAX_POINT_LIGHTS: usize = 32;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct GpuPointLight {
    position: [f32; 3],
    range: f32,
    color: [f32; 3],
    intensity: f32,
}

/// Layout must match WGSL `LightsUniform` (uniform address space).
/// Header is four `u32`s — do **not** use `vec3` in WGSL here (align-16 gap → 1056 vs 1040).
/// `_p0` = `scene_time` as `f32` bitcast (`bitcast<f32>(scene_time_bits)` in WGSL).
///
/// `sun_dir` / `ambient` are scene-level lighting (moved out of per-material SSBO).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct LightsUniform {
    count: u32,
    scene_time_bits: u32,
    _p1: u32,
    _p2: u32,
    /// xyz = sun direction, w = intensity (0 = sun off).
    sun_dir: [f32; 4],
    /// rgb ambient color (alpha unused).
    ambient: [f32; 4],
    lights: [GpuPointLight; MAX_POINT_LIGHTS],
}

const _: () = assert!(std::mem::size_of::<LightsUniform>() == 16 + 32 + 32 * MAX_POINT_LIGHTS);
const _: () = assert!(std::mem::size_of::<GpuPointLight>() == 32);

const MAX_LIGHTSTYLES: usize = 64;

/// Pack 64 f32 scales as 16×vec4 for WGSL uniform alignment.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct StylesUniform {
    values: [[f32; 4]; 16],
}

impl StylesUniform {
    fn from_scales(scales: &[f32; MAX_LIGHTSTYLES]) -> Self {
        let mut values = [[1.0f32; 4]; 16];
        for i in 0..MAX_LIGHTSTYLES {
            values[i / 4][i % 4] = scales[i];
        }
        Self { values }
    }

    fn all_ones() -> Self {
        Self {
            values: [[1.0; 4]; 16],
        }
    }
}

#[derive(Clone, Copy)]
struct SpriteSlot {
    cell: usize,
    uv: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct SkyUniform {
    inv_view_proj: [[f32; 4]; 4],
    view_proj: [[f32; 4]; 4],
    sun_dir: [f32; 4],
    camera_pos: [f32; 4],
    params: [[f32; 4]; 8],
    /// Linear horizon haze (rgb) + sun-side glow (w).
    sky_color: [f32; 4],
    /// Linear zenith rgb (w unused). Do not pack into `view_proj` (camera matrix).
    zenith: [f32; 4],
}

const _: () = {
    assert!(
        size_of::<SkyUniform>() == 320,
        "SkyUniform must be 320 bytes"
    );
};

/// Matches `gamedir/shaders/debug.wgsl` (view/proj only — not the sky dome).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct DebugViewUniform {
    inv_view_proj: [[f32; 4]; 4],
    view_proj: [[f32; 4]; 4],
    sun_dir: [f32; 4],
    camera_pos: [f32; 4],
    params: [[f32; 4]; 8],
}

pub(crate) const SKY_UNIFORM_RING: usize = 2;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct CloudUniform {
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
    skinned_mesh_globals_bg_layout: wgpu::BindGroupLayout,
    skinned_mesh_pipeline_layout: wgpu::PipelineLayout,
    skinned_mesh_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    bone_gpu_bufs: HashMap<u32, wgpu::Buffer>,
    skinned_mesh_globals_bgs: HashMap<u32, wgpu::BindGroup>,
    mesh_pipeline_layout: wgpu::PipelineLayout,
    mesh_white_bg: wgpu::BindGroup,
    mesh_user_bgs: HashMap<u32, wgpu::BindGroup>,
    /// 2D albedo + lightmap array (BSP world atlas path).
    tex_array_bg_layout: wgpu::BindGroupLayout,
    /// Layout when group 1 = 2D albedo + `texture_2d_array` (shader reflection).
    mesh_lm_surface_pipeline_layout: wgpu::PipelineLayout,
    /// Per-shader opaque mesh pipelines (`ShaderId` from registry).
    shader_mesh_pipelines: HashMap<(ShaderId, wgpu::TextureFormat), wgpu::RenderPipeline>,
    /// Key: (albedo_tex_slot, lightmap_gpu) — id 0 = white fallback.
    mesh_lm_surface_bgs: HashMap<(u32, u32), wgpu::BindGroup>,
    repeat_sampler: wgpu::Sampler,
    lightmap_white_tex: wgpu::Texture,
    lightmap_white_view: wgpu::TextureView,
    lightmap_textures: HashMap<u32, wgpu::Texture>,
    lightmap_views: HashMap<u32, wgpu::TextureView>,
    clamp_sampler: wgpu::Sampler,
    next_lightmap_slot: u32,
    /// `r_fullbright` — LM forced to white in mesh_brush shader.
    fullbright: bool,
    /// `r_lightmap` — debug: output LM only.
    lightmap_only: bool,
    wireframe: bool,
    lights_bg_layout: wgpu::BindGroupLayout,
    lights_buf: wgpu::Buffer,
    styles_buf: wgpu::Buffer,
    lights_bg: wgpu::BindGroup,

    // Terrain splat pipeline
    terrain_splat_bg_layout: wgpu::BindGroupLayout,
    terrain_splat_pipeline_layout: wgpu::PipelineLayout,
    terrain_splat_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,

    sky_globals_bg_layout: wgpu::BindGroupLayout,
    /// Ring so CPU can upload next frame while GPU may still read the previous slot.
    sky_globals_bufs: [wgpu::Buffer; SKY_UNIFORM_RING],
    sky_globals_bgs: [wgpu::BindGroup; SKY_UNIFORM_RING],
    sky_uniform_ring: u8,
    sky_pipeline_layout: wgpu::PipelineLayout,
    sky_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,

    debug_globals_buf: wgpu::Buffer,
    debug_globals_bg: wgpu::BindGroup,
    debug_pipeline_layout: wgpu::PipelineLayout,

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

    // Debug lines: visible (depth ≤) + occluded (depth >, dim fragment).
    debug_pipeline: Option<wgpu::RenderPipeline>,
    debug_pipeline_occluded: Option<wgpu::RenderPipeline>,
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
    /// `r_vsync` — when true, swapchain uses FIFO (vsync).
    vsync_enabled: bool,
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
    /// Staging buffer `map_async` in flight (auto-exposure readback).
    lum_readback_pending: bool,
    lum_log_avg: f32,
    tonemap_mode: f32,
    exposure: f32,
    target_exposure: f32,

    postfx: crate::postfx::PostFx,

    shaders: ShaderSources,

    // ImGui
    imgui_ctx: Option<imgui::Context>,
    imgui_wgpu: Option<imgui_wgpu::Renderer>,
    /// Mouse fed into ImGui IO each frame (set by client from `Input`).
    imgui_mouse_pos: [f32; 2],
    imgui_mouse_down: [bool; 5],
    imgui_mouse_wheel: f32,
    imgui_mouse_wheel_h: f32,

    // Mesh slab allocator + cache (Arc ptr → O(1); content hash fallback)
    mesh_pool: crate::mesh_allocator::MeshBufferPool,
    mesh_allocators: Vec<Box<dyn crate::mesh_allocator::MeshAllocator>>,
    mesh_gpu: crate::mesh_gpu_pool::MeshGpuPool,
    /// Staging for batched per-draw `MeshUniform` uploads (one `write_buffer` per frame).
    mesh_uniform_scratch: Vec<u8>,
    /// Indirect commands for world-space multi-draw batches.
    mesh_indirect_buf: wgpu::Buffer,
    /// Some adapters cannot execute indirect draw commands. Keep the normal
    /// per-draw path available on those devices.
    multi_draw_indirect_supported: bool,
    /// Reused draw order for mesh pass (sort by shader / material to cut pipeline binds).
    mesh_draw_order: Vec<usize>,
    /// Per-frame lazy GPU mesh handles (index = snapshot draw index).
    mesh_pass_static_handles: Vec<Option<crate::mesh_allocator::MeshHandle>>,
    mesh_pass_skinned_handles: Vec<Option<crate::mesh_allocator::MeshHandle>>,
    /// Reused indirect command list for multi-draw batches.
    mesh_indirect_cmd_scratch: Vec<wgpu::util::DrawIndexedIndirectArgs>,
    mesh_occlusion: crate::occlusion::MeshOcclusion,
    frame_occlusion_queries: bool,
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

    // Scene-level lighting (set via set_scene_lighting, was per-material before).
    scene_sun_dir: [f32; 4],
    scene_ambient: [f32; 4],
    /// Derived in `set_scene_lighting` — shared by sky pass + gamelib skylight.
    scene_sky_day_k: f32,
    scene_sky_zenith: [f32; 3],
    cloud_speed: f32,
    /// 0 = soft volumetric, 1 = dithered cheap (`r_cloud_mode`).
    cloud_mode: u32,
    /// Hosek-Wilkie atmosphere parameters (set from cvars `r_sky_*`).
    /// Turbidity 1..10, ground albedo RGB 0..1. Drives `SkyState` in the client
    /// engine so the procedural sky, scene ambient and fog share one model.
    scene_sky_turbidity: f32,
    scene_sky_albedo: [f32; 3],
    /// Log Hosek `SkyEnvironment` each extract when true (`r_sky_debug`).
    sky_debug_log: bool,

    /// Forward frame pass graph (HDR → surface).
    forward_graph: crate::render_graph::RenderGraph,

    /// Last frame CPU encode breakdown (`r_speeds`).
    last_profile: RenderFrameProfile,
    pending_extract_ms: f32,
    pending_prep_ms: f32,
    pending_prep_ui_ms: f32,
    pending_prep_snapshot_ms: f32,
    pending_prep_cull_ms: f32,
    pending_prep_cull_frustum_ms: f32,
    pending_prep_cull_assemble_ms: f32,
    pending_prep_cull_gather_ms: f32,
    pending_prep_audio_ms: f32,
    pending_extract_gpu: crate::render_profile::SceneExtractGpuStats,
    /// Per-encode counters (reset at start of mesh gpu cache).
    frame_mesh_gpu_uploads: u32,
    frame_mesh_gpu_cached: u32,
}

#[inline]
fn smoothstep_gpu(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge0 >= edge1 {
        return if x >= edge1 { 1.0 } else { 0.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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
        let multi_draw_indirect_supported = adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION);
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
        // Mesh uniform buffer: one 256-byte-aligned slot per mesh draw (grows at runtime).
        let mesh_align = 256u64; // min_uniform_buffer_offset_alignment
        let mesh_uniform_size = ((std::mem::size_of::<MeshUniform>() as u64 + mesh_align - 1)
            / mesh_align)
            * mesh_align;
        let mesh_globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh globals"),
            size: mesh_uniform_size * MESH_UNIFORM_SLOTS_INIT,
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
        let mesh_indirect_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh multi-draw indirect"),
            size: (std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>() * 4096) as u64,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Material SSBO bind group (now with PBR texture slots)
        let material_bg_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
        // Scene-level lighting defaults (no sun, no ambient — gamelib sets via set_scene_lighting).
        let scene_sun_dir: [f32; 4] = [0.0, 0.0, 1.0, 0.0];
        let scene_ambient: [f32; 4] = [0.0, 0.0, 0.0, 0.0];
        let scene_sky_day_k = 1.0;
        let scene_sky_zenith = [0.28, 0.55, 0.92];
        let cloud_speed = 0.04;
        let cloud_mode = 0u32;
        let scene_sky_turbidity = 3.0;
        let scene_sky_albedo = [0.2, 0.25, 0.3];
        let sky_debug_log = false;
        // White 1x1 texture view for default PBR textures (normal=up, ORM=full white)
        let white_tex = device.create_texture(&wgpu::TextureDescriptor {
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
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &white_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255u8, 255, 255, 255],
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
            label: Some("point lights + styles bg layout"),
            entries: &[
                // Binding 0: scene lights + `scene_time_bits` (games may read time in VS for wind).
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<LightsUniform>() as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<StylesUniform>() as u64,
                        ),
                    },
                    count: None,
                },
            ],
        });
        let lights_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point lights"),
            size: std::mem::size_of::<LightsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let styles_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lightstyles"),
            size: std::mem::size_of::<StylesUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &styles_buf,
            0,
            bytemuck::bytes_of(&StylesUniform::all_ones()),
        );
        let lights_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point lights + styles bg"),
            layout: &lights_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: lights_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: styles_buf.as_entire_binding(),
                },
            ],
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

        let skinned_mesh_globals_bg_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("skinned mesh globals bg layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: wgpu::BufferSize::new(mesh_uniform_bytes),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(GPU_BONE_BLOCK_SIZE),
                        },
                        count: None,
                    },
                ],
            });
        let skinned_mesh_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("skinned mesh pipeline layout"),
                bind_group_layouts: &[
                    Some(&skinned_mesh_globals_bg_layout),
                    Some(&tex_bg_layout),
                    Some(&material_bg_layout),
                    Some(&lights_bg_layout),
                ],
                immediate_size: 0,
            });

        // Group 1: 2D albedo atlas + lightmap array (device limit = 4 bind groups).
        let tex_array_bg_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("albedo+lightmap bg layout"),
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
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
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
        let mesh_lm_surface_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("mesh lm-surface pipeline layout"),
                bind_group_layouts: &[
                    Some(&mesh_globals_bg_layout),
                    Some(&tex_array_bg_layout),
                    Some(&material_bg_layout),
                    Some(&lights_bg_layout),
                ],
                immediate_size: 0,
            });
        // Nearest texels (pixel atlases / voxels); mip chain still used for LOD (linear between mips).
        let repeat_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("repeat nearest + mip linear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let clamp_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("clamp linear lightmap"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        // 1×1×4 white lightmap array fallback (4 style layers)
        let lm_white_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lightmap white array"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 4,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEX_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for z in 0..4u32 {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &lm_white_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: 0, z },
                    aspect: wgpu::TextureAspect::All,
                },
                &[255u8, 255, 255, 255],
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
        }
        let lightmap_white_view = lm_white_tex.create_view(&wgpu::TextureViewDescriptor {
            label: Some("lightmap white array view"),
            format: None,
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: 0,
            array_layer_count: Some(4),
            usage: None,
        });

        // Terrain splat bind group layout: globals(0) + [layer0, layer1, layer2, layer3, splat_map, sampler](1)
        let terrain_splat_bg_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("terrain splat bg layout"),
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
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let terrain_splat_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("terrain splat pipeline layout"),
                bind_group_layouts: &[
                    Some(&mesh_globals_bg_layout),
                    Some(&terrain_splat_bg_layout),
                ],
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
        let sky_globals_bufs = sky_globals_bufs
            .try_into()
            .ok()
            .expect("sky uniform ring");
        let sky_globals_bgs = sky_globals_bgs
            .try_into()
            .ok()
            .expect("sky uniform ring bg");

        let sky_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky pipeline layout"),
            bind_group_layouts: &[Some(&sky_globals_bg_layout)],
            immediate_size: 0,
        });

        let debug_globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("debug view globals"),
            size: size_of::<DebugViewUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let debug_globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("debug view globals bg"),
            layout: &sky_globals_bg_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &debug_globals_buf,
                    offset: 0,
                    size: None,
                }),
            }],
        });
        let debug_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("debug pipeline layout"),
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

        let mesh_pool =
            crate::mesh_allocator::MeshBufferPool::new(256 * 1024, 512 * 1024);
        let mut mesh_allocators: Vec<Box<dyn crate::mesh_allocator::MeshAllocator>> =
            Vec::new();
        // Allocator id 0 = default built-in slab allocator.
        mesh_allocators.push(Box::new(crate::mesh_allocator::SlabMeshAllocator::new()));
        let ui_pass = crate::ui_pass::UiPass::new(&device, shaders.ui.clone());
        let postfx = crate::postfx::PostFx::new(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba16Float,
            tonemap_bg_layout.clone(),
            crate::postfx::PostFxShaders {
                bloom_threshold: shaders.bloom_threshold.clone(),
                bloom_blur: shaders.bloom_blur.clone(),
                bloom_composite: shaders.bloom_composite.clone(),
                grain: shaders.grain.clone(),
                tonemap: shaders.tonemap.clone(),
            },
        );

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
            skinned_mesh_globals_bg_layout,
            skinned_mesh_pipeline_layout,
            skinned_mesh_pipelines: HashMap::new(),
            bone_gpu_bufs: HashMap::new(),
            skinned_mesh_globals_bgs: HashMap::new(),
            mesh_pipeline_layout,
            mesh_white_bg,
            mesh_user_bgs: HashMap::new(),
            tex_array_bg_layout,
            mesh_lm_surface_pipeline_layout,
            shader_mesh_pipelines: HashMap::new(),
            mesh_lm_surface_bgs: HashMap::new(),
            repeat_sampler,
            lightmap_white_tex: lm_white_tex,
            lightmap_white_view,
            lightmap_textures: HashMap::new(),
            lightmap_views: HashMap::new(),
            clamp_sampler,
            next_lightmap_slot: 1,
            fullbright: false,
            lightmap_only: false,
            wireframe: false,
            lights_bg_layout,
            lights_buf,
            styles_buf,
            lights_bg,

            terrain_splat_bg_layout,
            terrain_splat_pipeline_layout,
            terrain_splat_pipelines: HashMap::new(),

            sky_globals_bg_layout,
            sky_globals_bufs,
            sky_globals_bgs,
            sky_uniform_ring: 0,
            sky_pipeline_layout,
            sky_pipelines: HashMap::new(),

            debug_globals_buf,
            debug_globals_bg,
            debug_pipeline_layout,

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
            debug_pipeline_occluded: None,
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
            vsync_enabled: true,
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
            lum_readback_pending: false,
            lum_log_avg: 0.0,
            // 1.0 = Reinhard (softer than ACES for dark maps + point lights)
            tonemap_mode: 1.0,
            exposure_override: None,
            exposure: 1.0,
            target_exposure: 1.0,

            postfx,

            shaders,

            imgui_ctx: {
                let mut ctx = imgui::Context::create();
                ctx.set_ini_filename(None);
                Some(ctx)
            },
            imgui_wgpu: None,
            imgui_mouse_pos: [-1.0e6, -1.0e6],
            imgui_mouse_down: [false; 5],
            imgui_mouse_wheel: 0.0,
            imgui_mouse_wheel_h: 0.0,

            mesh_pool,
            mesh_allocators,
            mesh_gpu: crate::mesh_gpu_pool::MeshGpuPool::default(),
            mesh_uniform_scratch: Vec::new(),
            mesh_indirect_buf,
            multi_draw_indirect_supported,
            mesh_draw_order: Vec::new(),
            mesh_pass_static_handles: Vec::new(),
            mesh_pass_skinned_handles: Vec::new(),
            mesh_indirect_cmd_scratch: Vec::new(),
            mesh_occlusion: crate::occlusion::MeshOcclusion::new(),
            frame_occlusion_queries: false,
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
            scene_sun_dir,
            scene_ambient,
            scene_sky_day_k,
            scene_sky_zenith,
            cloud_speed,
            cloud_mode,
            scene_sky_turbidity,
            scene_sky_albedo,
            sky_debug_log,
            forward_graph: crate::frame::graph::build_forward_graph(),
            last_profile: RenderFrameProfile::default(),
            pending_extract_ms: 0.0,
            pending_prep_ms: 0.0,
            pending_prep_ui_ms: 0.0,
            pending_prep_snapshot_ms: 0.0,
            pending_prep_cull_ms: 0.0,
            pending_prep_cull_frustum_ms: 0.0,
            pending_prep_cull_assemble_ms: 0.0,
            pending_prep_cull_gather_ms: 0.0,
            pending_prep_audio_ms: 0.0,
            pending_extract_gpu: crate::render_profile::SceneExtractGpuStats::default(),
            frame_mesh_gpu_uploads: 0,
            frame_mesh_gpu_cached: 0,
        }
    }

    /// CPU scene extract time for the upcoming draw (`r_speeds`).
    pub fn note_extract_ms(&mut self, ms: f32) {
        self.pending_extract_ms = ms;
    }

    pub fn note_prep_ms(&mut self, ms: f32) {
        self.pending_prep_ms = ms;
    }

    pub fn note_prep_breakdown(&mut self, ui_ms: f32, cull_ms: f32, audio_ms: f32) {
        self.pending_prep_ui_ms = ui_ms;
        self.pending_prep_cull_ms = cull_ms;
        self.pending_prep_audio_ms = audio_ms;
        self.pending_prep_snapshot_ms = cull_ms + audio_ms;
        self.pending_prep_ms = ui_ms + cull_ms + audio_ms;
    }

    pub fn note_cull_detail(&mut self, frustum_ms: f32, assemble_ms: f32, gather_ms: f32) {
        self.pending_prep_cull_frustum_ms = frustum_ms;
        self.pending_prep_cull_assemble_ms = assemble_ms;
        self.pending_prep_cull_gather_ms = gather_ms;
    }

    /// Material GPU resolve stats from [`RenderExtractor`] (`r_loggpu`).
    pub fn note_extract_gpu_stats(&mut self, stats: crate::render_profile::SceneExtractGpuStats) {
        self.pending_extract_gpu = stats;
    }

    /// Last frame render breakdown (updated each `encode_frame`).
    pub fn frame_profile(&self) -> &RenderFrameProfile {
        &self.last_profile
    }

    pub(crate) fn depth_view_ref(&self) -> Option<&wgpu::TextureView> {
        self.depth_view.as_ref()
    }

    pub(crate) fn forward_graph_mut(&mut self) -> &mut crate::render_graph::RenderGraph {
        &mut self.forward_graph
    }

    /// Forward graph topology for `r_speeds` (node order = GPU queue order).
    pub fn forward_graph_describe(&self) -> Vec<(&'static str, Vec<&'static str>, Vec<&'static str>)> {
        self.forward_graph.describe_nodes()
    }

    /// Set scene-level directional + ambient lighting.
    ///
    /// `sun_dir` — xyz = normalized direction, w = intensity (0 = sun off).
    /// `ambient` — rgb ambient color (alpha unused).
    pub fn set_scene_lighting(&mut self, sun_dir: [f32; 4], ambient: [f32; 4]) {
        self.scene_sun_dir = sun_dir;
        self.scene_ambient = ambient;
        let z = sun_dir[2].clamp(-1.0, 1.0);
        self.scene_sky_day_k = smoothstep_gpu(-0.18, 0.32, z);
    }

    /// Authoritative sky palette from gamelib (`daytime.rs`), overrides derived day_k/zenith.
    pub fn set_sky_palette(&mut self, day_k: f32, zenith: [f32; 3]) {
        self.scene_sky_day_k = day_k.clamp(0.0, 1.0);
        self.scene_sky_zenith = zenith;
    }

    /// Current scene sun (xyz direction, w = intensity).
    pub fn scene_sun_dir(&self) -> [f32; 4] {
        self.scene_sun_dir
    }

    pub fn scene_ambient(&self) -> [f32; 4] {
        self.scene_ambient
    }

    pub fn scene_sky_day_k(&self) -> f32 {
        self.scene_sky_day_k
    }

    pub fn scene_sky_zenith(&self) -> [f32; 3] {
        self.scene_sky_zenith
    }

    pub fn set_cloud_speed(&mut self, speed: f32) {
        self.cloud_speed = speed.clamp(0.0, 0.5);
    }

    pub fn set_cloud_mode(&mut self, mode: u32) {
        self.cloud_mode = mode.min(1);
    }

    /// Hosek-Wilkie turbidity (1..10). Higher = hazier/more orange sky.
    pub fn set_sky_turbidity(&mut self, turbidity: f32) {
        self.scene_sky_turbidity = turbidity.clamp(1.0, 10.0);
    }

    /// Hosek-Wilkie ground albedo (RGB 0..1).
    pub fn set_sky_albedo(&mut self, albedo: [f32; 3]) {
        self.scene_sky_albedo = [
            albedo[0].clamp(0.0, 1.0),
            albedo[1].clamp(0.0, 1.0),
            albedo[2].clamp(0.0, 1.0),
        ];
    }

    pub fn set_sky_debug_log(&mut self, on: bool) {
        self.sky_debug_log = on;
    }

    pub fn sky_debug_log(&self) -> bool {
        self.sky_debug_log
    }

    pub fn scene_sky_turbidity(&self) -> f32 {
        self.scene_sky_turbidity
    }

    pub fn scene_sky_albedo(&self) -> [f32; 3] {
        self.scene_sky_albedo
    }

    /// Procedural-sky override of scene ambient: `rgb` = linear horizon haze,
    /// `w` = sun-side horizon glow. Lit shaders read it as `lights_u.ambient`.
    pub fn set_sky_environment(&mut self, horizon_linear: [f32; 3], sun_glow: f32) {
        self.scene_ambient = [horizon_linear[0], horizon_linear[1], horizon_linear[2], sun_glow];
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

    /// VSync on (`Fifo`) vs uncapped (`Immediate` when supported).
    pub fn set_vsync(&mut self, on: bool) {
        if self.vsync_enabled == on {
            return;
        }
        self.vsync_enabled = on;
        self.reconfigure_surface_present_mode();
    }

    pub fn vsync_enabled(&self) -> bool {
        self.vsync_enabled
    }

    fn reconfigure_surface_present_mode(&mut self) {
        let Some(surface) = &self.surface else {
            return;
        };
        let caps = surface.get_capabilities(&self.adapter);
        let mode = select_present_mode(&caps, self.vsync_enabled);
        if let Some(config) = &mut self.surface_config {
            if config.present_mode == mode {
                return;
            }
            config.present_mode = mode;
            surface.configure(&self.device, config);
        }
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
            present_mode: select_present_mode(&caps, self.vsync_enabled),
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
            let (tex, view) = self.create_sampled_texture_2d("map", w, h, TEX_FORMAT);
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
                        resource: wgpu::BindingResource::Sampler(&self.repeat_sampler),
                    },
                ],
            });
            self.map_bg = bg;
            self.map_texture = Some((tex, w, h));
        }
        let (tex, _, _) = self.map_texture.as_ref().expect("map texture");
        self.upload_rgba_with_mip_chain(tex, pixmap.data(), w, h);
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
            self.shader_mesh_pipelines.clear();
        }
    }

    /// Hot-reload built-in WGSL (sky, postfx, terrain, …) and drop cached pipelines.
    pub fn reload_shader_sources(&mut self, sources: ShaderSources) {
        self.shaders = sources;
        self.pipelines.clear();
        self.skinned_mesh_pipelines.clear();
        self.shader_mesh_pipelines.clear();
        self.terrain_splat_pipelines.clear();
        self.sky_pipelines.clear();
        self.cloud_pipelines.clear();
        self.storm_pipelines.clear();
        self.debug_pipeline = None;
        self.debug_pipeline_occluded = None;
        self.tonemap_pipeline = None;
        self.lum_pipeline = None;
        self.postfx.reload_shader_sources(crate::postfx::PostFxShaders {
            bloom_threshold: self.shaders.bloom_threshold.clone(),
            bloom_blur: self.shaders.bloom_blur.clone(),
            bloom_composite: self.shaders.bloom_composite.clone(),
            grain: self.shaders.grain.clone(),
            tonemap: self.shaders.tonemap.clone(),
        });
        self.invalidate_material_bgs();
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

    pub fn set_bloom(&mut self, enabled: bool, threshold: f32, intensity: f32) {
        self.postfx.bloom_enabled = enabled;
        self.postfx.bloom_threshold = threshold;
        self.postfx.bloom_intensity = intensity;
    }

    pub fn set_bloom_downscale(&mut self, scale: u32) {
        self.postfx.set_bloom_downscale(scale);
    }

    pub fn set_grain(&mut self, enabled: bool, strength: f32, downscale: f32) {
        self.postfx.grain_enabled = enabled;
        self.postfx.grain_strength = strength;
        self.postfx.grain_downscale = downscale;
    }

    // ── ImGui ────────────────────────────────────────────────────

    pub fn imgui_context_ptr(&self) -> *mut std::ffi::c_void {
        self.imgui_ctx.as_ref().map_or(std::ptr::null_mut(), |ctx| {
            ctx as *const imgui::Context as *mut std::ffi::c_void
        })
    }

    /// Feed OS mouse into ImGui IO (call once per frame before render).
    pub fn set_imgui_input(&mut self, pos: [f32; 2], down: [bool; 5], wheel_y: f32, wheel_h: f32) {
        self.imgui_mouse_pos = pos;
        self.imgui_mouse_down = down;
        self.imgui_mouse_wheel = wheel_y;
        self.imgui_mouse_wheel_h = wheel_h;
    }

    pub fn imgui_begin_frame(&mut self, width: f32, height: f32) {
        if let Some(ctx) = &mut self.imgui_ctx {
            ctx.io_mut().display_size = [width, height];
            ctx.io_mut().mouse_pos = self.imgui_mouse_pos;
            ctx.io_mut().mouse_down = self.imgui_mouse_down;
            ctx.io_mut().mouse_wheel = self.imgui_mouse_wheel;
            ctx.io_mut().mouse_wheel_h = self.imgui_mouse_wheel_h;
            ctx.new_frame();
            self.imgui_mouse_wheel = 0.0;
            self.imgui_mouse_wheel_h = 0.0;
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

        let draw_data = self.imgui_ctx.as_mut().unwrap().render();

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

    pub fn render_to_surface(
        &mut self,
        snapshot: &RenderSnapshot,
        shaders: &ShaderRegistry,
    ) -> Result<(), FrameError> {
        // 1. Read back previous frame's luminance (if ready)
        self.process_lum_readback();
        self.mesh_occlusion.process_readback(&self.device);

        let Some(surface) = &self.surface else {
            return Ok(());
        };

        let acquire_start = std::time::Instant::now();
        let current = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(st) => st,
            wgpu::CurrentSurfaceTexture::Suboptimal(st) => st,
            wgpu::CurrentSurfaceTexture::Timeout => return Err(FrameError::SurfaceTimeout),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(FrameError::SurfaceOccluded),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(FrameError::SurfaceOutdated),
            wgpu::CurrentSurfaceTexture::Lost => return Err(FrameError::SurfaceLost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(FrameError::SurfaceValidation),
        };
        self.last_profile.surface_acquire_ms =
            acquire_start.elapsed().as_secs_f32() * 1000.0;
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
        let (sw, sh) = self
            .surface_config
            .as_ref()
            .map(|c| (c.width as f32, c.height as f32))
            .unwrap_or((800.0, 600.0));
        self.imgui_begin_frame(sw, sh);

        let encoder =
            self.encode_frame(snapshot, &hdr_view, format, Some(&surface_view), shaders);

        self.finish_surface_encoder(encoder, current, &surface_view, None);
        Ok(())
    }

    pub fn render_to_surface_with_game_frame(
        &mut self,
        snapshot: &RenderSnapshot,
        game_frame_fn: Option<extern "C" fn(*mut std::ffi::c_void)>,
        game_frame_ctx: *mut std::ffi::c_void,
        shaders: &ShaderRegistry,
    ) -> Result<(), FrameError> {
        self.process_lum_readback();
        self.mesh_occlusion.process_readback(&self.device);

        let Some(surface) = &self.surface else {
            return Ok(());
        };

        let acquire_start = std::time::Instant::now();
        let current = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(st) => st,
            wgpu::CurrentSurfaceTexture::Suboptimal(st) => st,
            wgpu::CurrentSurfaceTexture::Timeout => return Err(FrameError::SurfaceTimeout),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(FrameError::SurfaceOccluded),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(FrameError::SurfaceOutdated),
            wgpu::CurrentSurfaceTexture::Lost => return Err(FrameError::SurfaceLost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(FrameError::SurfaceValidation),
        };
        self.last_profile.surface_acquire_ms =
            acquire_start.elapsed().as_secs_f32() * 1000.0;
        let surface_view = current
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let format = self
            .surface_config
            .as_ref()
            .map(|c| c.format)
            .unwrap_or(TEX_FORMAT);

        let hdr_view = self.hdr_view.clone().expect("HDR texture not initialized");

        let (sw, sh) = self
            .surface_config
            .as_ref()
            .map(|c| (c.width as f32, c.height as f32))
            .unwrap_or((800.0, 600.0));
        self.imgui_begin_frame(sw, sh);

        let game_ui_start = std::time::Instant::now();
        if let Some(f) = game_frame_fn {
            f(game_frame_ctx);
        }
        let game_frame_ui_ms = game_ui_start.elapsed().as_secs_f32() * 1000.0;

        let encoder =
            self.encode_frame(snapshot, &hdr_view, format, Some(&surface_view), shaders);
        self.finish_surface_encoder(encoder, current, &surface_view, Some(game_frame_ui_ms));
        Ok(())
    }

    fn finish_surface_encoder(
        &mut self,
        mut encoder: wgpu::CommandEncoder,
        current: wgpu::SurfaceTexture,
        surface_view: &wgpu::TextureView,
        game_frame_ui_ms: Option<f32>,
    ) {
        let _ = self.device.poll(wgpu::PollType::Poll);
        let tail_start = std::time::Instant::now();
        self.imgui_end_frame_and_render(&mut encoder, surface_view);
        let after_imgui_ms = tail_start.elapsed().as_secs_f32() * 1000.0;

        let lum_copied = self.append_lum_copy_to_encoder(&mut encoder);

        let finish_start = std::time::Instant::now();
        let cmd_buf = encoder.finish();
        let encoder_finish_ms = finish_start.elapsed().as_secs_f32() * 1000.0;

        let submit_start = std::time::Instant::now();
        self.queue.submit([cmd_buf]);
        let queue_submit_ms = submit_start.elapsed().as_secs_f32() * 1000.0;
        self.queue_lum_readback_after_submit(lum_copied);
        if self.frame_occlusion_queries {
            self.mesh_occlusion.queue_readback_after_submit();
        }
        let after_submit_ms = encoder_finish_ms + queue_submit_ms;

        let present_start = std::time::Instant::now();
        self.queue.present(current);
        let after_present_ms = present_start.elapsed().as_secs_f32() * 1000.0;

        if let Some(ui) = game_frame_ui_ms {
            self.last_profile.game_frame_ui_ms = ui;
        }
        self.last_profile.after_imgui_ms = after_imgui_ms;
        self.last_profile.encoder_finish_ms = encoder_finish_ms;
        self.last_profile.queue_submit_ms = queue_submit_ms;
        self.last_profile.after_submit_ms = after_submit_ms;
        self.last_profile.after_present_ms = after_present_ms;
        self.last_profile.after_encode_ms = tail_start.elapsed().as_secs_f32() * 1000.0;
    }

    fn auto_exposure_enabled(&self) -> bool {
        self.exposure_override.is_none()
    }

    /// GPU luminance pass lives in `encode_frame`; this only copies 64×64 → staging (async).
    fn append_lum_copy_to_encoder(&self, encoder: &mut wgpu::CommandEncoder) -> bool {
        if !self.auto_exposure_enabled() || self.lum_readback_pending {
            return false;
        }
        let Some(lum_tex) = &self.lum_texture else {
            return false;
        };
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
        true
    }

    /// Call immediately after `queue.submit` when this frame's encoder included a lum copy.
    fn queue_lum_readback_after_submit(&mut self, copied: bool) {
        if !copied || self.lum_readback_pending {
            return;
        }
        let buf = self.lum_staging_buf.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.lum_readback_rx = Some(rx);
        buf.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.lum_readback_pending = true;
    }

    fn process_lum_readback(&mut self) {
        if !self.lum_readback_pending {
            return;
        }
        let _ = self.device.poll(wgpu::PollType::Poll);
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
        self.lum_log_avg = log_avg;
        drop(data);
        self.lum_staging_buf.unmap();
        self.lum_readback_pending = false;
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
    pub fn render_offscreen(
        &mut self,
        snapshot: &RenderSnapshot,
        shaders: &ShaderRegistry,
    ) -> &[u8] {
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

        let mut encoder = self.encode_frame(snapshot, &view_clone, TEX_FORMAT, None, shaders);
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
    pub fn render_into_shared(
        &mut self,
        snapshot: &RenderSnapshot,
        shared: &SharedTexture,
        shaders: &ShaderRegistry,
    ) {
        self.ensure_depth(shared.width, shared.height);
        let mut encoder = self.encode_frame(snapshot, &shared.view, TEX_FORMAT, None, shaders);
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

    /// Depth target for mesh / terrain / debug (required by forward graph `ResId::DEPTH`).
    pub(crate) fn ensure_depth_for_frame(&mut self) {
        self.ensure_depth(frame_width(), frame_height());
    }

    fn ensure_debug_pipelines(&mut self, format: wgpu::TextureFormat) {
        if self.debug_pipeline.is_some() && self.debug_pipeline_occluded.is_some() {
            return;
        }
        const ATTRS: [wgpu::VertexAttribute; 2] = [
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
        ];
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("debug shader"),
                source: wgpu::ShaderSource::Wgsl(self.shaders.debug.clone().into()),
            });
        let make = |device: &wgpu::Device,
                    layout: &wgpu::PipelineLayout,
                    shader: &wgpu::ShaderModule,
                    label: &str,
                    fs: &str,
                    depth_compare: wgpu::CompareFunction,
                    bias_const: i32|
         -> wgpu::RenderPipeline {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<crate::debug_draw::DebugVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &ATTRS,
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
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(depth_compare),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState {
                        constant: bias_const,
                        slope_scale: if bias_const != 0 { -1.0 } else { 0.0 },
                        clamp: 0.0,
                    },
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(fs),
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
        };
        if self.debug_pipeline_occluded.is_none() {
            self.debug_pipeline_occluded = Some(make(
                &self.device,
                &self.debug_pipeline_layout,
                &shader,
                "debug occluded",
                "fs_occluded",
                wgpu::CompareFunction::Greater,
                0,
            ));
        }
        if self.debug_pipeline.is_none() {
            // Slight negative bias so coplanar floor edges win as "visible".
            self.debug_pipeline = Some(make(
                &self.device,
                &self.debug_pipeline_layout,
                &shader,
                "debug visible",
                "fs_main",
                wgpu::CompareFunction::LessEqual,
                -1,
            ));
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

    pub(crate) fn build_sky_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
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

    pub(crate) fn build_cloud_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
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

    fn skinned_mesh_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<SkinnedMeshVertex>() as u64,
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
                    format: wgpu::VertexFormat::Uint32x4,
                    shader_location: 3,
                },
                wgpu::VertexAttribute {
                    offset: 48,
                    format: wgpu::VertexFormat::Float32x4,
                    shader_location: 4,
                },
            ],
        }
    }

    fn ensure_skinned_mesh_pipeline(
        &mut self,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        if !self.skinned_mesh_pipelines.contains_key(&format) {
            assert!(
                !self.shaders.mesh_skinned.is_empty(),
                "ShaderSources.mesh_skinned missing (load mesh_skinned.wgsl)"
            );
            let shader = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("mesh_skinned"),
                    source: wgpu::ShaderSource::Wgsl(self.shaders.mesh_skinned.clone().into()),
                });
            let p = self
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("skinned mesh pipeline"),
                    layout: Some(&self.skinned_mesh_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        buffers: &[Some(Self::skinned_mesh_vertex_layout())],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_main"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
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
                        depth_compare: Some(wgpu::CompareFunction::Less),
                        stencil: wgpu::StencilState::default(),
                        bias: wgpu::DepthBiasState::default(),
                    }),
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                });
            self.skinned_mesh_pipelines.insert(format, p);
        }
        self.skinned_mesh_pipelines.get(&format).unwrap()
    }

    fn ensure_skinned_bone_bind_group(
        &mut self,
        handle: u32,
        palette: &[[[f32; 4]; 4]],
    ) -> &wgpu::BindGroup {
        if !self.bone_gpu_bufs.contains_key(&handle) {
            let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bone palette"),
                size: GPU_BONE_BLOCK_SIZE,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.bone_gpu_bufs.insert(handle, buf);
        }
        if !self.skinned_mesh_globals_bgs.contains_key(&handle) {
            let mesh_uniform_bytes = std::mem::size_of::<MeshUniform>() as u64;
            let bone_buf = self.bone_gpu_bufs.get(&handle).expect("bone palette buf");
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("skinned mesh globals bg"),
                layout: &self.skinned_mesh_globals_bg_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.mesh_globals_buf,
                            offset: 0,
                            size: wgpu::BufferSize::new(mesh_uniform_bytes),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: bone_buf.as_entire_binding(),
                    },
                ],
            });
            self.skinned_mesh_globals_bgs.insert(handle, bg);
        }
        let block = GpuBoneBlock::from_palette(palette);
        let buf = self.bone_gpu_bufs.get(&handle).unwrap();
        self.queue.write_buffer(buf, 0, bytemuck::bytes_of(&block));
        self.skinned_mesh_globals_bgs.get(&handle).unwrap()
    }

    fn mesh_vertex_layout_extended() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
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
                    format: wgpu::VertexFormat::Float32x4,
                    shader_location: 3,
                },
                wgpu::VertexAttribute {
                    offset: 48,
                    format: wgpu::VertexFormat::Float32x2,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    offset: 56,
                    format: wgpu::VertexFormat::Float32x4,
                    shader_location: 5,
                },
            ],
        }
    }

    fn build_shader_mesh_pipeline(
        &self,
        wgsl: &str,
        format: wgpu::TextureFormat,
        lm_surface: bool,
        extended_vertex: bool,
        skinned_vertex: bool,
    ) -> wgpu::RenderPipeline {
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("registry mesh shader"),
                source: wgpu::ShaderSource::Wgsl(wgsl.into()),
            });
        let vertex_buf_layout = if skinned_vertex {
            Self::skinned_mesh_vertex_layout()
        } else if extended_vertex {
            Self::mesh_vertex_layout_extended()
        } else {
            wgpu::VertexBufferLayout {
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
                ],
            }
        };
        let layout = if skinned_vertex {
            &self.skinned_mesh_pipeline_layout
        } else if lm_surface {
            &self.mesh_lm_surface_pipeline_layout
        } else {
            &self.mesh_pipeline_layout
        };
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("shader mesh pipeline"),
                layout: Some(layout),
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
                        blend: None,
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
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
    }

    fn ensure_shader_mesh_pipeline(
        &mut self,
        shaders: &ShaderRegistry,
        shader_id: ShaderId,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        let key = (shader_id, format);
        if !self.shader_mesh_pipelines.contains_key(&key) {
            let def = shaders
                .get(shader_id)
                .expect("MeshDraw shader_id not in ShaderRegistry");
            let skinned = def.uses_skinned_mesh_vertex();
            let lm_surface = !skinned && def.group1_uses_lightmap_array();
            let extended = !skinned && def.uses_extended_mesh_vertex();
            let p = self.build_shader_mesh_pipeline(
                &def.wgsl,
                format,
                lm_surface,
                extended,
                skinned,
            );
            self.shader_mesh_pipelines.insert(key, p);
        }
        self.shader_mesh_pipelines.get(&key).unwrap()
    }

    /// Skinned draws: material shader when registered as skinned, else built-in `mesh_skinned.wgsl`.
    fn ensure_skinned_draw_pipeline(
        &mut self,
        shaders: &ShaderRegistry,
        shader_id: ShaderId,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        if shader_id != 0 {
            if let Some(def) = shaders.get(shader_id) {
                if def.uses_skinned_mesh_vertex() {
                    return self
                        .ensure_shader_mesh_pipeline(shaders, shader_id, format)
                        .clone();
                }
            }
        }
        self.ensure_skinned_mesh_pipeline(format).clone()
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
            ],
        };
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                atlas_rect: mesh
                    .atlas_rects
                    .get(i)
                    .copied()
                    .unwrap_or([0.0, 0.0, 1.0, 1.0]),
                lm_uv: mesh.lightmap_uvs.get(i).copied().unwrap_or([0.0, 0.0]),
                lm_styles: mesh
                    .lightmap_styles
                    .get(i)
                    .copied()
                    .unwrap_or([255.0, 255.0, 255.0, 255.0]),
                _pad: [0.0, 0.0],
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

    /// Increment owners. `allocator_id` selects which registered allocator owns the
    /// GPU region once uploaded; `0` is the default slab allocator. The
    /// caller (mesh lifecycle) reads it from `Mesh::allocator_id`.
    pub fn mesh_retain(&mut self, content_id: u64, allocator_id: u32) {
        self.mesh_gpu.retain(content_id, allocator_id);
    }

    /// Register a custom mesh allocator. Returns its id (stable for the
    /// renderer's lifetime). Id `0` is reserved for the built-in slab
    /// allocator and is never handed out here. The engine owns the GPU
    /// buffers; the allocator is an offset-only bookkeeper.
    pub fn register_mesh_allocator(
        &mut self,
        allocator: Box<dyn crate::mesh_allocator::MeshAllocator>,
    ) -> u32 {
        let id = self.mesh_allocators.len() as u32;
        self.mesh_allocators.push(allocator);
        id
    }

    pub fn mesh_release(&mut self, content_id: u64) {
        if let Some((allocator_id, handle)) = self.mesh_gpu.release(content_id) {
            if let Some(alloc) = self.mesh_allocators.get_mut(allocator_id as usize) {
                alloc.free(handle);
            }
        }
    }

    /// Drop all refcount slots (e.g. client disconnect). Frees GPU regions via
    /// each allocator's `free`.
    pub fn reset_mesh_gpu(&mut self) {
        let freed = self.mesh_gpu.drain_for_free();
        for (allocator_id, handle) in freed {
            if let Some(alloc) = self.mesh_allocators.get_mut(allocator_id as usize) {
                alloc.free(handle);
            }
        }
    }

    pub fn mesh_gpu_live_count(&self) -> u32 {
        self.mesh_gpu.live_count()
    }

    pub fn mesh_buffer_pool(&self) -> &crate::mesh_allocator::MeshBufferPool {
        &self.mesh_pool
    }

    pub fn mesh_gpu_pool(&self) -> &crate::mesh_gpu_pool::MeshGpuPool {
        &self.mesh_gpu
    }

    /// Resolve GPU handle for draw. Mesh must be [`Self::mesh_retain`]'d by an owner.
    pub fn mesh_for_draw(
        &mut self,
        mesh: &Arc<deerpunk_engine_common::resource::mesh::Mesh>,
    ) -> crate::mesh_allocator::MeshHandle {
        let content_id = mesh.content_id();
        assert_ne!(content_id, 0, "Mesh.content_id unset");
        assert!(
            self.mesh_gpu.is_retained(content_id),
            "mesh content_id={content_id} not retained (entity lifecycle)"
        );
        let uploaded = self.mesh_gpu.needs_upload(content_id);
        if uploaded {
            let handle = self.upload_mesh_content(mesh.as_ref(), content_id);
            self.mesh_gpu.set_uploaded(content_id, handle);
            self.frame_mesh_gpu_uploads += 1;
        } else {
            self.frame_mesh_gpu_cached += 1;
        }
        self.mesh_gpu
            .handle(content_id)
            .expect("mesh upload missing after needs_upload")
    }

    /// Hot path when GPU data is already resident (no upload bookkeeping beyond stats).
    pub(crate) fn mesh_resolve_for_draw(
        &mut self,
        mesh: &std::sync::Arc<deerpunk_engine_common::resource::mesh::Mesh>,
    ) -> crate::mesh_allocator::MeshHandle {
        let content_id = mesh.content_id();
        assert_ne!(content_id, 0, "Mesh.content_id unset");
        assert!(
            self.mesh_gpu.is_retained(content_id),
            "mesh content_id={content_id} not retained (entity lifecycle)"
        );
        if !self.mesh_gpu.needs_upload(content_id) {
            if let Some(h) = self.mesh_gpu.handle(content_id) {
                self.frame_mesh_gpu_cached += 1;
                return h;
            }
        }
        self.mesh_for_draw(mesh)
    }

    pub(crate) fn mesh_static_handle_at(
        &mut self,
        i: usize,
        mesh: &std::sync::Arc<deerpunk_engine_common::resource::mesh::Mesh>,
    ) -> crate::mesh_allocator::MeshHandle {
        if self.mesh_pass_static_handles[i].is_none() {
            self.mesh_pass_static_handles[i] = Some(self.mesh_resolve_for_draw(mesh));
        }
        self.mesh_pass_static_handles[i].expect("mesh_static_handle_at")
    }

    pub(crate) fn mesh_skinned_handle_at(
        &mut self,
        j: usize,
        mesh: &std::sync::Arc<deerpunk_engine_common::resource::mesh::Mesh>,
    ) -> crate::mesh_allocator::MeshHandle {
        if self.mesh_pass_skinned_handles[j].is_none() {
            self.mesh_pass_skinned_handles[j] = Some(self.mesh_resolve_for_draw(mesh));
        }
        self.mesh_pass_skinned_handles[j].expect("mesh_skinned_handle_at")
    }

    pub(crate) fn mesh_static_handle_cached(&self, i: usize) -> crate::mesh_allocator::MeshHandle {
        self.mesh_pass_static_handles[i].expect("static mesh handle not resolved")
    }

    pub(crate) fn mesh_skinned_handle_cached(&self, j: usize) -> crate::mesh_allocator::MeshHandle {
        self.mesh_pass_skinned_handles[j].expect("skinned mesh handle not resolved")
    }

    fn allocator_for_mesh(
        &mut self,
        mesh: &deerpunk_engine_common::resource::mesh::Mesh,
    ) -> u32 {
        mesh.allocator_id
    }

    fn mesh_uniform_stride() -> u64 {
        let mesh_align = 256u64;
        ((std::mem::size_of::<MeshUniform>() as u64 + mesh_align - 1) / mesh_align) * mesh_align
    }

    /// Grow `mesh_globals_buf` when static+skinned draw count exceeds capacity.
    fn ensure_mesh_uniform_slots(&mut self, draw_count: u64) {
        let stride = Self::mesh_uniform_stride();
        let need_slots = draw_count.max(1);
        let cap_slots = self.mesh_globals_buf.size() / stride;
        if need_slots <= cap_slots {
            return;
        }
        let new_slots = need_slots.next_power_of_two().max(MESH_UNIFORM_SLOTS_INIT);
        let mesh_uniform_bytes = std::mem::size_of::<MeshUniform>() as u64;
        self.mesh_globals_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh globals"),
            size: stride * new_slots,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.mesh_globals_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh globals bg"),
            layout: &self.mesh_globals_bg_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &self.mesh_globals_buf,
                    offset: 0,
                    size: wgpu::BufferSize::new(mesh_uniform_bytes),
                }),
            }],
        });
        // Skinned bind groups embed `mesh_globals_buf`; drop stale entries after resize.
        self.skinned_mesh_globals_bgs.clear();
        eprintln!(
            "[renderer] mesh_globals_buf grew to {} draws ({} bytes); had {} draws",
            new_slots,
            stride * new_slots,
            draw_count
        );
    }

    fn ensure_mesh_indirect_capacity(&mut self, command_count: usize) {
        let command_bytes =
            (command_count * std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>()) as u64;
        if command_bytes <= self.mesh_indirect_buf.size() {
            return;
        }
        self.mesh_indirect_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh multi-draw indirect"),
            size: command_bytes.next_power_of_two(),
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }

    /// Allocate a GPU region for `mesh` via its allocator, growing the pool
    /// and retrying if the allocator is full. Then upload vertex/index bytes.
    fn allocate_and_upload(
        &mut self,
        mesh: &deerpunk_engine_common::resource::mesh::Mesh,
        vertices: &[u8],
        indices: &[u32],
        stride: u32,
    ) -> crate::mesh_allocator::MeshHandle {
        let allocator_id = self.allocator_for_mesh(mesh);
        let stride = stride.max(1);
        assert_eq!(
            vertices.len() % stride as usize,
            0,
            "mesh vertex blob size {} not aligned to stride {}",
            vertices.len(),
            stride
        );
        let vert_count = (vertices.len() as u32) / stride;
        let idx_count = indices.len() as u32;
        let alloc = self
            .mesh_allocators
            .get_mut(allocator_id as usize)
            .expect("mesh allocator id out of range");
        let mut handle = alloc.allocate(vert_count, idx_count, stride);
        let mut grow_attempts = 0u32;
        while !handle.is_valid() || !self.mesh_pool.handle_fits(handle) {
            if handle.is_valid() {
                alloc.free(handle);
                handle = crate::mesh_allocator::MeshHandle::INVALID;
            }
            grow_attempts += 1;
            assert!(
                grow_attempts <= 8,
                "mesh pool growth failed (vert_count={vert_count} idx_count={idx_count})"
            );
            let vert_cap = vert_count.saturating_add(vert_count / 4).max(256 * 1024);
            let idx_cap = idx_count.saturating_add(idx_count / 4).max(512 * 1024);
            let buffer_id = self.mesh_pool.add(&self.device, vert_cap, idx_cap, stride);
            alloc.add_buffer(buffer_id, vert_cap, idx_cap, stride);
            handle = alloc.allocate(vert_count, idx_count, stride);
        }
        let vert_bytes = (handle.vert_count as usize) * (handle.stride as usize);
        assert_eq!(
            vert_bytes,
            vertices.len(),
            "mesh upload vert_count/stride mismatch blob={} handle_count={} stride={}",
            vertices.len(),
            handle.vert_count,
            handle.stride
        );
        let ib_bytes = (handle.idx_count as usize)
            * crate::mesh_allocator::MESH_INDEX_SIZE as usize;
        assert_eq!(
            ib_bytes,
            indices.len() * std::mem::size_of::<u32>(),
            "mesh upload idx_count mismatch"
        );
        assert!(
            self.mesh_pool.handle_fits(handle),
            "mesh upload handle out of pool bounds (buffer_id={} v={}..{} i={}..{})",
            handle.buffer_id,
            handle.vert_offset,
            handle.vert_count,
            handle.idx_offset,
            handle.idx_count
        );
        let vb = self.mesh_pool.vertex_buffer(handle.buffer_id);
        let ib = self.mesh_pool.index_buffer(handle.buffer_id);
        let v_off = (handle.vert_offset * stride) as u64;
        let i_off = (handle.idx_offset * crate::mesh_allocator::MESH_INDEX_SIZE) as u64;
        assert!(
            v_off + vert_bytes as u64 <= vb.size() && i_off + ib_bytes as u64 <= ib.size(),
            "mesh upload would overrun GPU buffer (buf={} v_off={} v_len={} vb_size={} i_off={} i_len={} ib_size={})",
            handle.buffer_id,
            v_off,
            vert_bytes,
            vb.size(),
            i_off,
            ib_bytes,
            ib.size(),
        );
        self.queue.write_buffer(vb, v_off, &vertices[..vert_bytes]);
        self.queue.write_buffer(
            ib,
            i_off,
            bytemuck::cast_slice(&indices[..handle.idx_count as usize]),
        );
        handle
    }

    fn upload_mesh_content(
        &mut self,
        mesh: &deerpunk_engine_common::resource::mesh::Mesh,
        content_id: u64,
    ) -> crate::mesh_allocator::MeshHandle {
        if mesh.is_skinned() {
            return self.upload_skinned_mesh_content(mesh, content_id);
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
                atlas_rect: mesh
                    .atlas_rects
                    .get(i)
                    .copied()
                    .unwrap_or([0.0, 0.0, 1.0, 1.0]),
                lm_uv: mesh.lightmap_uvs.get(i).copied().unwrap_or([0.0, 0.0]),
                lm_styles: mesh
                    .lightmap_styles
                    .get(i)
                    .copied()
                    .unwrap_or([255.0, 255.0, 255.0, 255.0]),
                _pad: [0.0, 0.0],
            }));
        }

        let handle = self.allocate_and_upload(
            mesh,
            &vertices,
            &mesh.indices,
            MESH_VERTEX_SIZE,
        );
        self.record_mesh_asset_upload(handle, content_id);
        handle
    }

    fn upload_skinned_mesh_content(
        &mut self,
        mesh: &deerpunk_engine_common::resource::mesh::Mesh,
        content_id: u64,
    ) -> crate::mesh_allocator::MeshHandle {
        let mut vertices =
            Vec::with_capacity(mesh.vertex_count() * SKINNED_MESH_VERTEX_SIZE as usize);
        for i in 0..mesh.vertex_count() {
            let (ji, jw) = if let (Some(ji), Some(jw)) =
                (mesh.joint_indices.get(i), mesh.joint_weights.get(i))
            {
                (
                    [ji[0] as u32, ji[1] as u32, ji[2] as u32, ji[3] as u32],
                    *jw,
                )
            } else {
                ([0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0])
            };
            vertices.extend_from_slice(bytemuck::bytes_of(&SkinnedMeshVertex {
                position: mesh.positions[i],
                normal: mesh
                    .normals
                    .get(i)
                    .copied()
                    .unwrap_or(deerpunk_engine_common::coord::UP),
                uv: mesh.uvs.get(i).copied().unwrap_or([0.0, 0.0]),
                joints: ji,
                weights: jw,
            }));
        }
        let handle = self.allocate_and_upload(
            mesh,
            &vertices,
            &mesh.indices,
            SKINNED_MESH_VERTEX_SIZE,
        );
        self.record_mesh_asset_upload(handle, content_id);
        handle
    }

    fn record_mesh_asset_upload(
        &mut self,
        handle: crate::mesh_allocator::MeshHandle,
        content_id: u64,
    ) {
        let verts = handle.vert_count;
        let indices = handle.idx_count;
        let stride = handle.stride.max(MESH_VERTEX_SIZE);
        let mesh_id = ASSET_ID_MESH | self.next_mesh_asset_id as u64;
        self.next_mesh_asset_id = self.next_mesh_asset_id.wrapping_add(1);
        let size = verts as u64 * stride as u64 + indices as u64 * MESH_INDEX_SIZE as u64;
        self.asset_meta.insert(
            mesh_id,
            AssetEntry {
                id: mesh_id,
                kind: AssetKind::Mesh,
                name: format!("mesh#content_{content_id}"),
                size,
                width: None,
                height: None,
                layers: None,
                verts: Some(verts),
                indices: Some(indices),
            },
        );
    }

    /// Upload a single-layer lightmap as a 4-layer array (same pixels on all layers).
    pub fn upload_lightmap(&mut self, pixels: &[u8], w: u32, h: u32) -> u32 {
        self.upload_lightmap_layers(w, h, [pixels, pixels, pixels, pixels])
    }

    /// Upload a 4-layer lightmap atlas (RGBA8 each). Returns id for [`MeshDraw::lightmap`].
    pub fn upload_lightmap_layers(&mut self, w: u32, h: u32, layers: [&[u8]; 4]) -> u32 {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lightmap atlas array"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 4,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEX_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let expected = (w as usize) * (h as usize) * 4;
        for (z, pixels) in layers.iter().enumerate() {
            let bytes = if pixels.len() >= expected {
                &pixels[..expected]
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
                        z: z as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
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
        let view = tex.create_view(&wgpu::TextureViewDescriptor {
            label: Some("lightmap atlas array view"),
            format: None,
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: 0,
            array_layer_count: Some(4),
            usage: None,
        });
        let slot = self.next_lightmap_slot;
        self.next_lightmap_slot += 1;
        self.lightmap_textures.insert(slot, tex);
        self.lightmap_views.insert(slot, view);
        // Invalidate any BG cached earlier with the white fallback for this id.
        self.mesh_lm_surface_bgs.retain(|(_, lm), _| *lm != slot);
        slot
    }

    /// Upload HL lightstyle scales (`'a'`..`'z'` → 0..1) for the mesh_brush shader.
    pub fn set_lightstyles(&mut self, scales: &[f32; MAX_LIGHTSTYLES]) {
        self.queue.write_buffer(
            &self.styles_buf,
            0,
            bytemuck::bytes_of(&StylesUniform::from_scales(scales)),
        );
    }

    /// HL-style `r_fullbright` — albedo only, lightmap ignored in mesh_brush shader.
    pub fn set_fullbright(&mut self, on: bool) {
        self.fullbright = on;
    }

    /// `r_occlusion_cull` — skip mesh draws occluded in the previous frame (GPU queries).
    pub fn set_occlusion_cull(&mut self, on: bool) {
        self.mesh_occlusion.set_enabled(on);
    }

    /// Debug: show lightmap contribution only.
    pub fn set_lightmap_only(&mut self, on: bool) {
        self.lightmap_only = on;
    }

    /// Bind group for 2D albedo + lightmap array (lm_id `0` = white fallback).
    fn ensure_albedo_lightmap_bg(&mut self, albedo_slot: u32, lm_id: u32) -> wgpu::BindGroup {
        let key = (albedo_slot, lm_id);
        let albedo_ready =
            albedo_slot == MeshTexture::WHITE.0 || self.texture_views.contains_key(&albedo_slot);
        let lm_ready = lm_id == 0 || self.lightmap_views.contains_key(&lm_id);
        if self.mesh_lm_surface_bgs.contains_key(&key) {
            if albedo_ready && lm_ready {
                return self.mesh_lm_surface_bgs.get(&key).unwrap().clone();
            }
            self.mesh_lm_surface_bgs.remove(&key);
        }
        let albedo_view = if albedo_slot == MeshTexture::WHITE.0 {
            &self.white_tex_view
        } else {
            self.texture_views
                .get(&albedo_slot)
                .unwrap_or(&self.white_tex_view)
        };
        let lm_view = if lm_id == 0 {
            &self.lightmap_white_view
        } else {
            self.lightmap_views
                .get(&lm_id)
                .unwrap_or(&self.lightmap_white_view)
        };
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh albedo+lightmap bg"),
            layout: &self.tex_array_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(albedo_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.clamp_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(lm_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.clamp_sampler),
                },
            ],
        });
        self.mesh_lm_surface_bgs.insert(key, bg);
        self.mesh_lm_surface_bgs.get(&key).unwrap().clone()
    }

    fn invalidate_mesh_lm_surface_bgs_for_albedo(&mut self, slot: u32) {
        self.mesh_lm_surface_bgs
            .retain(|(albedo, _), _| *albedo != slot);
    }

    fn mesh_has_bsp_surface_data(mesh: &deerpunk_engine_common::resource::mesh::Mesh) -> bool {
        !mesh.lightmap_uvs.is_empty() || !mesh.atlas_rects.is_empty()
    }

    fn create_sampled_texture_2d(
        &self,
        label: &'static str,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let mips = texture_mip_levels(width, height);
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        (tex, view)
    }

    fn upload_rgba_with_mip_chain(
        &self,
        texture: &wgpu::Texture,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) {
        for (level, (w, h, data)) in build_rgba_mipmap_chain(pixels, width, height)
            .iter()
            .enumerate()
        {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * w),
                    rows_per_image: Some(*h),
                },
                wgpu::Extent3d {
                    width: *w,
                    height: *h,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    pub fn upload_texture(&mut self, pixels: &[u8], w: u32, h: u32) -> MeshTexture {
        let (tex, view) = self.create_sampled_texture_2d(
            "pbr tex",
            w,
            h,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        self.upload_rgba_with_mip_chain(&tex, pixels, w, h);
        let slot = self.next_texture_slot;
        self.next_texture_slot += 1;

        // Group 1 bind group for albedo texture (same as mesh_user_bgs pattern)
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pbr albedo bg"),
            layout: &self.tex_bg_layout,
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
        self.mesh_user_bgs.insert(slot, bg);

        self.texture_views.insert(slot, view);
        self.invalidate_mesh_lm_surface_bgs_for_albedo(slot);
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

    fn ensure_material_bg(
        &mut self,
        material_id: u32,
        normal_slot: MeshTexture,
        orm_slot: MeshTexture,
    ) -> &wgpu::BindGroup {
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
            let orm_view = self
                .texture_views
                .get(&orm_slot.0)
                .unwrap_or(&self.white_tex_view);
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
                        resource: wgpu::BindingResource::Sampler(&self.repeat_sampler),
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
        let (tex, view) = self.create_sampled_texture_2d("map atlas", w, h, TEX_FORMAT);
        self.upload_rgba_with_mip_chain(&tex, pixels, w, h);
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
                    resource: wgpu::BindingResource::Sampler(&self.repeat_sampler),
                },
            ],
        });
        self.mesh_user_bgs.insert(MeshTexture::slot(1).0, bg);
    }

    /// Загружает текстуру для меша на GPU (slot — номер слота текстуры).
    pub fn upload_mesh_texture(&mut self, slot: MeshTexture, pixels: &[u8], w: u32, h: u32) {
        let (tex, view) = self.create_sampled_texture_2d("mesh texture", w, h, TEX_FORMAT);
        self.upload_rgba_with_mip_chain(&tex, pixels, w, h);
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
                    resource: wgpu::BindingResource::Sampler(&self.repeat_sampler),
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
        shaders: &ShaderRegistry,
    ) -> wgpu::CommandEncoder {
        let frame_start = std::time::Instant::now();
        let mut lap = frame_start;
        let mut prof = RenderFrameProfile::default();
        let mut draw_acc = FrameDrawAccum::default();
        self.frame_occlusion_queries = false;
        prof.extract_ms = self.pending_extract_ms;
        prof.prep_ms = self.pending_prep_ms;
        prof.prep_ui_ms = self.pending_prep_ui_ms;
        prof.prep_snapshot_ms = self.pending_prep_snapshot_ms;
        prof.prep_cull_ms = self.pending_prep_cull_ms;
        prof.prep_cull_frustum_ms = self.pending_prep_cull_frustum_ms;
        prof.prep_cull_assemble_ms = self.pending_prep_cull_assemble_ms;
        prof.prep_cull_gather_ms = self.pending_prep_cull_gather_ms;
        prof.prep_audio_ms = self.pending_prep_audio_ms;
        prof.extract_gpu = self.pending_extract_gpu;
        self.frame_mesh_gpu_uploads = 0;
        self.frame_mesh_gpu_cached = 0;
        prof.sprite_instances = snapshot.instances.len() as u32;
        prof.hud_instances = snapshot.hud.len() as u32;
        prof.mesh_draws = (snapshot.mesh_draws.len() + snapshot.skinned_mesh_draws.len()) as u32;
        prof.terrain_draws = snapshot.terrain_splat_draws.len() as u32;
        prof.point_lights = snapshot.point_lights.len() as u32;
        prof.width = frame_width();
        prof.height = frame_height();

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

        let mut frame_ctx = FrameCtx {
            snapshot,
            hdr_view: view,
            format,
            surface_view,
            shaders,
            prof: &mut prof,
            draw_acc: &mut draw_acc,
            lap: &mut lap,
        };

        crate::frame::executor::run_frame_graph(self, &mut frame_ctx, &mut encoder, &pipeline, hud_vp);

        prof.total_encode_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
        let frustum_occluded = if snapshot.frustum_culled {
            let cs = snapshot.cull_stats;
            (cs.mesh_total - cs.mesh_visible) + (cs.skinned_total - cs.skinned_visible)
        } else {
            0
        };
        let (_, gpu_occluded) = self.mesh_occlusion.last_stats();
        let occluded = frustum_occluded + gpu_occluded;
        let visible_instances = snapshot.instances.len() as u32
            + snapshot.mesh_draws.len() as u32
            + snapshot.skinned_mesh_draws.len() as u32
            + snapshot.terrain_splat_draws.len() as u32;
        prof.speeds_hud = FrameSpeedsHud {
            dip_count: draw_acc.dip_count,
            shader_bind: draw_acc.shader_bind,
            frame_total_tris: (draw_acc.index_count / 3).min(u32::MAX as u64) as u32,
            visible_world_lights: snapshot.visible_world_lights,
            visible_dyn_lights: snapshot.point_lights.len() as u32,
            visible_sun_lights: if self.scene_sun_dir[3] > 0.001 { 1 } else { 0 },
            visible_instances,
            visible_occluders: {
                let (queried, occ) = self.mesh_occlusion.last_stats();
                queried.saturating_sub(occ)
            },
            occluded_objects: occluded,
        };
        self.last_profile = prof;

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
        draw_acc: &mut FrameDrawAccum,
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
            let batch = (i - start) as u32;
            pass.draw(0..4, start as u32..i as u32);
            draw_acc.note_draw_verts(4 * batch);
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
