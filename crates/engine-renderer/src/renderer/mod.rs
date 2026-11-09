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
use std::time::Duration;

use bytemuck::{Pod, Zeroable};
use deerpunk_engine_common::frame::{frame_height, frame_width};
use deerpunk_engine_common::material::ShaderId;

use crate::shader_registry::ShaderRegistry;
use tiny_skia::Pixmap;
use wgpu::util::DeviceExt;

use crate::asset_inventory::{AssetEntry, AssetKind, ASSET_ID_MATERIAL};
use crate::bone_gpu::GPU_BONE_BLOCK_SIZE;
use crate::font::{build_font_atlas, ATLAS_H, ATLAS_W};
use crate::frame::context::FrameCtx;
use crate::mesh_allocator::SKINNED_MESH_VERTEX_SIZE;
use crate::render_profile::{FrameDrawAccum, FrameSpeedsHud, RenderFrameProfile};
use crate::snapshot::{Instance, RenderSnapshot, Shape, TexSel};

mod features;

#[doc(hidden)]
pub use features::mesh::{
    bench_batch_fingerprint, bench_build_indirect, bench_mdi_cache_probes, StaticBatchPlanBench,
};
pub mod frame;
mod hdr;

/// Host Nuklear debug HUD (console, infobox, alloc bars).
/// When `false`, `host_ui_begin_frame` / the swapchain UI pass are skipped.
pub const HOST_UI_ENABLED: bool = true;

/// `vsync=true` → FIFO (ждёт vblank). `false` → без ожидания vblank, если драйвер умеет.
/// Prefer an sRGB swapchain so the display does the gamma encode. Tonemap writes linear LDR.
fn pick_surface_format(caps: &wgpu::SurfaceCapabilities) -> wgpu::TextureFormat {
    const ORDER: [wgpu::TextureFormat; 4] = [
        wgpu::TextureFormat::Bgra8UnormSrgb,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
    ];
    for want in ORDER {
        if caps.formats.contains(&want) {
            return want;
        }
    }
    caps.formats[0]
}

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
    modes.first().copied().unwrap_or(wgpu::PresentMode::Fifo)
}

/// Mip count for a 2D texture (always ≥ 1).
#[inline]
pub(crate) fn texture_mip_levels(width: u32, height: u32) -> u32 {
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

pub(crate) fn build_rgba_mipmap_chain(
    base: &[u8],
    width: u32,
    height: u32,
) -> Vec<(u32, u32, Vec<u8>)> {
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
pub(crate) fn fit_sprite(sprite: &Pixmap) -> (u32, u32, Vec<u8>) {
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
pub(crate) const SPRITE_ATLAS_SIZE: u32 = 2048;
/// Сторона ячейки атласа. Танки 149×118 умещаются в 160×160.
pub(crate) const SPRITE_CELL: u32 = 160;
pub(crate) const SPRITE_COLS: u32 = SPRITE_ATLAS_SIZE / SPRITE_CELL;
pub(crate) const SPRITE_CELLS: usize = (SPRITE_COLS * SPRITE_COLS) as usize;

/// Максимум инстансов за кадр.
const MAX_INSTANCES: usize = 8192;
/// Dynamic uniform slots for batched `MeshUniform` (one per mesh draw). Was 256 → 64 KiB total.
pub(crate) const MESH_UNIFORM_SLOTS_INIT: u64 = 512;

/// Формат всех наших текстур и offscreen-таргета (без sRGB — байты как были
/// в CPU-пикспмапе, чтобы картинка совпадала с прежним рендерером).
pub(crate) const TEX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Block/item/albedo PNG atlases (sRGB storage, linear sample).
pub(crate) const ALBEDO_TEX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// One depth buffer in the per-target depth pool.
pub(crate) struct DepthTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

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
    pub bloom_threshold: String,
    pub bloom_blur: String,
    pub bloom_composite: String,
    pub grain: String,
    pub ui: String,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Globals {
    screen: [f32; 2],
    exposure: f32,
    tonemap_mode: f32,
    view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct GpuInstance {
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
pub(crate) struct MeshVertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    /// Albedo atlas rect `[min_u, min_v, size_u, size_v]` (full tex → `[0,0,1,1]`).
    atlas_rect: [f32; 4],
    /// BSP lightmap UV (atlas). Unused meshes → [0,0].
    lm_uv: [f32; 2],
    /// Per-vertex lightmap style indices (255 = unused slot). Empty → all unused.
    lm_styles: [f32; 4],
    _pad: [f32; 2],
}

const _: () = assert!(size_of::<MeshVertex>() == crate::mesh_allocator::MESH_VERTEX_SIZE as usize);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct SkinnedMeshVertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    joints: [u32; 4],
    weights: [f32; 4],
}

const _: () = assert!(size_of::<SkinnedMeshVertex>() == SKINNED_MESH_VERTEX_SIZE as usize);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
/// Per-draw object data. Camera matrix lives in [`MeshViewUniform`] (one upload per pass).
pub(crate) struct MeshUniform {
    model: [[f32; 4]; 4],
    material_data: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<MeshUniform>() == 80);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct MeshViewUniform {
    view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 3],
    _pad_cam: f32,
}

const _: () = assert!(std::mem::size_of::<MeshViewUniform>() == 80);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MeshBatchKey {
    shader_id: u32,
    lm_surface: bool,
    material_id: u32,
    texture: u32,
    normal: u32,
    orm: u32,
    lightmap_gpu: u32,
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
    _pad: [u32; 2],
    /// Hosek zenith (linear), same as sky dome — terrain outdoor_light matches sky.
    zenith: [f32; 4],
    /// xyz = sun direction, w = intensity (0 = sun off).
    sun_dir: [f32; 4],
    /// rgb = horizon haze (linear), w = sun-side glow for fog.
    ambient: [f32; 4],
    lights: [GpuPointLight; MAX_POINT_LIGHTS],
}

const _: () = assert!(std::mem::size_of::<LightsUniform>() == 64 + 32 * MAX_POINT_LIGHTS);
const _: () = assert!(std::mem::size_of::<GpuPointLight>() == 32);

#[derive(Clone, Copy)]
pub(crate) struct SpriteSlot {
    uv: [f32; 4],
}

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

/// Per-frame uniform / indirect copies. Slot N is written while the GPU may still read N-1.
pub(crate) const FRAME_UNIFORM_RING: usize = 2;

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
    pub gpu: crate::gpu::Gpu,
    /// Set by the client before encode. Draws store [`MeshHandle`], CPU bytes live in the registry.
    frame_meshes: *const deerpunk_engine_common::resource::MeshRegistry,

    ui_pass: crate::ui_pass::UiPass,
    pending_ui: Option<deerpunk_engine_common::ui_draw::UiFrame>,
    /// Upload `pending_ui` into GPU buffers on the next UI pass. Draw still runs after that.
    ui_upload: bool,

    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    pipeline_layout: wgpu::PipelineLayout,
    tex_bg_layout: wgpu::BindGroupLayout,

    mesh_globals_bg_layout: wgpu::BindGroupLayout,
    mesh_globals_bufs: [wgpu::Buffer; FRAME_UNIFORM_RING],
    mesh_view_bufs: [wgpu::Buffer; FRAME_UNIFORM_RING],
    mesh_globals_bgs: [wgpu::BindGroup; FRAME_UNIFORM_RING],
    frame_uniform_ring: u8,
    skinned_mesh_globals_bg_layout: wgpu::BindGroupLayout,
    skinned_mesh_pipeline_layout: wgpu::PipelineLayout,
    skinned_mesh_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    bone_gpu_bufs: HashMap<u32, wgpu::Buffer>,
    skinned_mesh_globals_bgs: HashMap<u32, [wgpu::BindGroup; FRAME_UNIFORM_RING]>,
    mesh_pipeline_layout: wgpu::PipelineLayout,
    mesh_white_bg: wgpu::BindGroup,
    mesh_user_bgs: HashMap<u32, wgpu::BindGroup>,
    /// Per-shader opaque mesh pipelines (`ShaderId` from registry).
    /// `(shader_id, format, skinned_bind_group_layout)` — static draws use `false`, skinned draws `true`.
    shader_mesh_pipelines:
        HashMap<(ShaderId, wgpu::TextureFormat, bool, bool), wgpu::RenderPipeline>,
    repeat_sampler: wgpu::Sampler,
    /// `r_fullbright` — mesh lighting forced off in the shader.
    fullbright: bool,
    wireframe: bool,
    lights_bufs: [wgpu::Buffer; FRAME_UNIFORM_RING],
    lights_bgs: [wgpu::BindGroup; FRAME_UNIFORM_RING],

    sky: features::sky::Sky,

    debug_globals_buf: wgpu::Buffer,
    debug_globals_bg: wgpu::BindGroup,
    debug_pipeline_layout: wgpu::PipelineLayout,

    // Debug lines: visible (depth ≤) + occluded (depth >, dim fragment).
    debug_pipeline: Option<wgpu::RenderPipeline>,
    debug_pipeline_occluded: Option<wgpu::RenderPipeline>,
    debug_vertex_buf: wgpu::Buffer,

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

    /// Per-target depth pool, keyed by `DepthTargetId`. Cameras sharing a target id
    /// share one depth buffer; different ids get separate depth buffers (so an overlay
    /// camera with its own near plane never corrupts the world depth).
    depth_pool:
        std::collections::HashMap<deerpunk_engine_common::components::DepthTargetId, DepthTexture>,

    hdr: hdr::Hdr,
    postfx: crate::postfx::PostFx,

    shaders: ShaderSources,

    host_ui: Option<crate::host_ui::HostUi>,
    host_ui_format: Option<wgpu::TextureFormat>,
    pending_ui_input: crate::host_ui::UiFrameInput,

    // Mesh slab allocator + cache (Arc ptr → O(1); content hash fallback)
    mesh_pool: crate::mesh_allocator::MeshBufferPool,
    mesh_allocators: Vec<Box<dyn crate::mesh_allocator::MeshAllocator>>,
    mesh_gpu: crate::mesh_gpu_pool::MeshGpuPool,
    /// Staging for batched per-draw `MeshUniform` uploads (one `write_buffer` per frame).
    mesh_uniform_scratch: Vec<u8>,
    /// Indirect commands for world-space multi-draw batches.
    mesh_indirect_bufs: [wgpu::Buffer; FRAME_UNIFORM_RING],
    /// Length of `mesh_batch_order`. Static plan is opaque only.
    mesh_batch_opaque_count: usize,
    mesh_batch_order: Vec<usize>,
    mesh_batch_for_index: Vec<Option<MeshBatchKey>>,
    mesh_batch_groups: HashMap<MeshBatchKey, Vec<usize>>,
    /// Batchable draws, sorted by key. Reused so a changed visible set does not allocate.
    mesh_batch_pairs: Vec<(MeshBatchKey, u32)>,
    /// `upload_prep` already ran for this frame (multi-camera).
    upload_prepped: bool,
    /// Per-frame lazy GPU mesh handles (index = snapshot draw index).
    mesh_pass_static_handles: Vec<Option<crate::mesh_allocator::MeshHandle>>,
    mesh_pass_skinned_handles: Vec<Option<crate::mesh_allocator::MeshHandle>>,
    /// Reused indirect command list for multi-draw batches.
    mesh_indirect_cmd_scratch: Vec<wgpu::util::DrawIndexedIndirectArgs>,
    /// GPU sub-allocation last used for a static draw index.
    /// Identity is `(registry handle, mesh_revision)`, not revision alone:
    /// many chunks share revision 1, and the draw index is reused as the view moves.
    mdi_draw_gpu: Vec<(usize, u64, crate::mesh_allocator::MeshHandle)>,
    /// Draw indices grouped by GPU buffer while building one MDI batch.
    mdi_by_buffer: Vec<(u32, usize)>,
    /// `mesh_shader_supports_draw_slot` per shader. WGSL is not scanned again.
    mdi_shader_ok: HashMap<u32, bool>,
    mesh_occlusion: crate::occlusion::MeshOcclusion,
    /// `r_mesh_batch` — group pooled static draws for shared pipeline/texture binds.
    mesh_draw_batching: bool,
    /// `r_drawtransparent` — alpha-blended mesh queue.
    draw_transparent: bool,
    /// `r_mesh_batch_mdi` — multi-draw indirect for batch groups (needs draw-slot in WGSL).
    mesh_batch_mdi: bool,
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
    /// Voxel distance fog (gamelib `set_scene_fog` → `LightsUniform._pad`).
    scene_fog: [f32; 2],
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
    pending_game_frame_ui_ms: Option<f32>,
    /// Per-encode counters (reset at start of mesh gpu cache).
    frame_mesh_gpu_uploads: u32,
    frame_mesh_gpu_cached: u32,
    /// `MeshRegistry::mesh_revisions` value last uploaded, by handle index.
    uploaded_mesh_revs: Vec<u64>,
    defer_surface_submit: bool,
    pending_surface_present: Option<PendingSurfacePresent>,
}

/// Command buffer + swapchain image waiting for async `submit` / `present`.
pub struct PendingSurfacePresent {
    pub cmds: Vec<wgpu::CommandBuffer>,
    pub surface: wgpu::SurfaceTexture,
    pub lum_copied: bool,
    pub occlusion_queries: bool,
    pub tail: PendingSurfaceTail,
}

/// CPU-side timings applied after async submit completes.
#[derive(Clone)]
pub struct PendingSurfaceTail {
    pub after_host_ui_ms: f32,
    pub encoder_finish_ms: f32,
    pub game_frame_ui_ms: Option<f32>,
    pub after_encode_ms: f32,
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
        let gpu = crate::gpu::Gpu::new(instance, compatible_surface);
        let device = &gpu.device;
        let queue = &gpu.queue;

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

        let _mesh_uniform_bytes = std::mem::size_of::<MeshUniform>() as u64;
        let mesh_view_layout_entry = wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let mesh_globals_bg_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("mesh globals bg layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    mesh_view_layout_entry,
                ],
            });
        // Storage-array stride (16). Camera lives in `mesh_view`, not in each slot.
        let mesh_uniform_size = Self::mesh_uniform_stride();
        let mut mesh_globals_bufs = Vec::with_capacity(FRAME_UNIFORM_RING);
        let mut mesh_view_bufs = Vec::with_capacity(FRAME_UNIFORM_RING);
        let mut mesh_globals_bgs = Vec::with_capacity(FRAME_UNIFORM_RING);
        let mut mesh_indirect_bufs = Vec::with_capacity(FRAME_UNIFORM_RING);
        for i in 0..FRAME_UNIFORM_RING {
            let mesh_globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("mesh globals {i}")),
                size: mesh_uniform_size * MESH_UNIFORM_SLOTS_INIT,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mesh_view_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("mesh view {i}")),
                size: 256,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mesh_globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("mesh globals bg {i}")),
                layout: &mesh_globals_bg_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &mesh_globals_buf,
                            offset: 0,
                            size: None,
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: mesh_view_buf.as_entire_binding(),
                    },
                ],
            });
            mesh_globals_bufs.push(mesh_globals_buf);
            mesh_view_bufs.push(mesh_view_buf);
            mesh_globals_bgs.push(mesh_globals_bg);
            mesh_indirect_bufs.push(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("mesh multi-draw indirect {i}")),
                size: (std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>() * 4096) as u64,
                usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let mesh_globals_bufs: [wgpu::Buffer; FRAME_UNIFORM_RING] =
            mesh_globals_bufs.try_into().expect("mesh globals ring");
        let mesh_view_bufs: [wgpu::Buffer; FRAME_UNIFORM_RING] =
            mesh_view_bufs.try_into().expect("mesh view ring");
        let mesh_globals_bgs: [wgpu::BindGroup; FRAME_UNIFORM_RING] =
            mesh_globals_bgs.try_into().expect("mesh globals bg ring");
        let mesh_indirect_bufs: [wgpu::Buffer; FRAME_UNIFORM_RING] =
            mesh_indirect_bufs.try_into().expect("mesh indirect ring");
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
        let material_allocator = crate::material_allocator::MaterialAllocator::new(device);
        // Scene-level lighting defaults (no sun, no ambient — gamelib sets via set_scene_lighting).
        let scene_sun_dir: [f32; 4] = [0.0, 0.0, 1.0, 0.0];
        let scene_ambient: [f32; 4] = [0.0, 0.0, 0.0, 0.0];
        let scene_fog: [f32; 2] = [0.0, 0.0];
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
            label: Some("point lights bg layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
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
            }],
        });
        let mut lights_bufs = Vec::with_capacity(FRAME_UNIFORM_RING);
        let mut lights_bgs = Vec::with_capacity(FRAME_UNIFORM_RING);
        for i in 0..FRAME_UNIFORM_RING {
            let lights_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("point lights {i}")),
                size: std::mem::size_of::<LightsUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let lights_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("point lights bg {i}")),
                layout: &lights_bg_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: lights_buf.as_entire_binding(),
                }],
            });
            lights_bufs.push(lights_buf);
            lights_bgs.push(lights_bg);
        }
        let lights_bufs: [wgpu::Buffer; FRAME_UNIFORM_RING] =
            lights_bufs.try_into().expect("lights ring");
        let lights_bgs: [wgpu::BindGroup; FRAME_UNIFORM_RING] =
            lights_bgs.try_into().expect("lights bg ring");

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
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
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
                    mesh_view_layout_entry,
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
        let sky = features::sky::Sky::new(device);

        let debug_globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("debug view globals"),
            size: size_of::<DebugViewUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let debug_globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("debug view globals bg"),
            layout: sky.globals_layout(),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &debug_globals_buf,
                    offset: 0,
                    size: None,
                }),
            }],
        });
        let debug_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("debug pipeline layout"),
                bind_group_layouts: &[Some(sky.globals_layout())],
                immediate_size: 0,
            });

        let hdr = hdr::Hdr::new(device);

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

        let mesh_pool = crate::mesh_allocator::MeshBufferPool::new(256 * 1024, 512 * 1024);
        let mut mesh_allocators: Vec<Box<dyn crate::mesh_allocator::MeshAllocator>> = Vec::new();
        // Allocator id 0 = default built-in slab allocator.
        mesh_allocators.push(Box::new(crate::mesh_allocator::SlabMeshAllocator::new()));
        let ui_pass = crate::ui_pass::UiPass::new(device, shaders.ui.clone());
        let postfx = crate::postfx::PostFx::new(device, hdr.tonemap_bg_layout.clone());

        Self {
            gpu,
            frame_meshes: std::ptr::null(),
            ui_pass,
            pending_ui: None,
            ui_upload: false,
            pipelines: HashMap::new(),
            pipeline_layout,
            tex_bg_layout,

            mesh_globals_bg_layout,
            mesh_globals_bufs,
            mesh_view_bufs,
            mesh_globals_bgs,
            frame_uniform_ring: 0,
            skinned_mesh_globals_bg_layout,
            skinned_mesh_pipeline_layout,
            skinned_mesh_pipelines: HashMap::new(),
            bone_gpu_bufs: HashMap::new(),
            skinned_mesh_globals_bgs: HashMap::new(),
            mesh_pipeline_layout,
            mesh_white_bg,
            mesh_user_bgs: HashMap::new(),
            shader_mesh_pipelines: HashMap::new(),
            repeat_sampler,
            fullbright: false,
            wireframe: false,
            lights_bufs,
            lights_bgs,

            sky,

            debug_globals_buf,
            debug_globals_bg,
            debug_pipeline_layout,

            debug_pipeline: None,
            debug_pipeline_occluded: None,
            debug_vertex_buf: debug_vb,

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
            depth_pool: std::collections::HashMap::new(),
            hdr,
            postfx,

            shaders,

            host_ui: None,
            host_ui_format: None,
            pending_ui_input: crate::host_ui::UiFrameInput::default(),

            mesh_pool,
            mesh_allocators,
            mesh_gpu: crate::mesh_gpu_pool::MeshGpuPool::default(),
            mesh_uniform_scratch: Vec::new(),
            mesh_indirect_bufs,
            mesh_batch_opaque_count: 0,
            mesh_batch_order: Vec::new(),
            mesh_batch_for_index: Vec::new(),
            mesh_batch_groups: HashMap::new(),
            mesh_batch_pairs: Vec::new(),
            upload_prepped: false,
            mesh_pass_static_handles: Vec::new(),
            mesh_pass_skinned_handles: Vec::new(),
            mesh_indirect_cmd_scratch: Vec::new(),
            mdi_draw_gpu: Vec::new(),
            mdi_by_buffer: Vec::new(),
            mdi_shader_ok: HashMap::new(),
            mesh_occlusion: crate::occlusion::MeshOcclusion::new(),
            mesh_draw_batching: true,
            draw_transparent: true,
            mesh_batch_mdi: true,
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
            scene_fog,
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
            pending_game_frame_ui_ms: None,
            frame_mesh_gpu_uploads: 0,
            frame_mesh_gpu_cached: 0,
            uploaded_mesh_revs: Vec::new(),
            defer_surface_submit: false,
            pending_surface_present: None,
        }
    }

    pub fn set_defer_surface_submit(&mut self, defer: bool) {
        self.defer_surface_submit = defer;
    }

    pub fn defer_surface_submit(&self) -> bool {
        self.defer_surface_submit
    }

    pub fn take_pending_surface_present(&mut self) -> Option<PendingSurfacePresent> {
        self.pending_surface_present.take()
    }

    /// Present on the calling thread. The submit worker must not call this:
    /// it shares the window-server lock with event polling.
    pub fn present_surface(&self, surface: wgpu::SurfaceTexture) -> f32 {
        let start = std::time::Instant::now();
        self.gpu.queue.present(surface);
        start.elapsed().as_secs_f32() * 1000.0
    }

    pub fn finish_deferred_surface_submit(
        &mut self,
        lum_copied: bool,
        occlusion_queries: bool,
        tail: PendingSurfaceTail,
        submit_ms: f32,
        present_ms: f32,
    ) {
        self.queue_lum_readback_after_submit(lum_copied);
        if occlusion_queries {
            self.mesh_occlusion.queue_readback_after_submit();
        }
        if let Some(ui) = tail.game_frame_ui_ms {
            self.last_profile.game_frame_ui_ms = ui;
        }
        self.last_profile.after_host_ui_ms = tail.after_host_ui_ms;
        self.last_profile.encoder_finish_ms = tail.encoder_finish_ms;
        self.last_profile.queue_submit_ms = submit_ms;
        self.last_profile.after_submit_ms = tail.encoder_finish_ms + submit_ms;
        self.last_profile.after_present_ms = present_ms;
        self.last_profile.after_encode_ms = tail.after_encode_ms;
    }

    /// CPU scene extract time for the upcoming draw (`r_speeds`).
    pub fn note_extract_ms(&mut self, ms: f32) {
        self.pending_extract_ms = ms;
    }

    pub fn note_game_frame_ui_ms(&mut self, ms: f32) {
        self.pending_game_frame_ui_ms = Some(ms);
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

    /// Ensure (and return a clone of) the depth view for a given target id, sized to the
    /// frame. First use of a target id allocates its depth buffer; subsequent cameras with
    /// the same id share it. Different ids get separate depth buffers (so an overlay
    /// camera with its own near plane never corrupts the world depth). Returns a
    /// cloned `TextureView` (Arc-cheap) so callers avoid `&self` borrow conflicts.
    pub(crate) fn ensure_depth(
        &mut self,
        target: deerpunk_engine_common::components::DepthTargetId,
        w: u32,
        h: u32,
    ) -> wgpu::TextureView {
        let needs = match self.depth_pool.get(&target) {
            Some(d) => d.texture.width() != w || d.texture.height() != h,
            None => true,
        };
        if needs {
            let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
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
            self.depth_pool
                .insert(target, DepthTexture { texture, view });
        }
        self.depth_pool.get(&target).unwrap().view.clone()
    }

    pub(crate) fn forward_graph_mut(&mut self) -> &mut crate::render_graph::RenderGraph {
        &mut self.forward_graph
    }

    /// Forward graph topology for `r_speeds` (node order = GPU queue order).
    pub fn forward_graph_describe(
        &self,
    ) -> Vec<(&'static str, Vec<&'static str>, Vec<&'static str>)> {
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
        self.sky.scene_sky_day_k = smoothstep_gpu(-0.18, 0.32, z);
    }

    pub fn set_scene_fog(&mut self, start: f32, falloff: f32) {
        self.scene_fog = [start.max(0.0), falloff.max(1.0)];
    }

    /// Current scene sun (xyz direction, w = intensity).
    pub fn scene_sun_dir(&self) -> [f32; 4] {
        self.scene_sun_dir
    }

    pub fn scene_ambient(&self) -> [f32; 4] {
        self.scene_ambient
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
        let caps = surface.get_capabilities(&self.gpu.adapter);
        let mode = select_present_mode(&caps, self.vsync_enabled);
        if let Some(config) = &mut self.surface_config {
            if config.present_mode == mode {
                return;
            }
            config.present_mode = mode;
            surface.configure(&self.gpu.device, config);
        }
    }

    /// Подключает поверхность окна.
    pub fn set_surface(&mut self, surface: wgpu::Surface<'static>, width: u32, height: u32) {
        let caps = surface.get_capabilities(&self.gpu.adapter);
        let format = pick_surface_format(&caps);
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
        surface.configure(&self.gpu.device, &config);
        self.surface_config = Some(config);
        self.surface = Some(surface);

        self.ensure_hdr_texture(width, height);
        self.ensure_tonemap_pipeline(format);
    }

    pub fn set_host_ui_input(&mut self, input: crate::host_ui::UiFrameInput) {
        self.pending_ui_input = input;
    }

    pub fn host_ui_begin_frame(&mut self, width: f32, height: f32) {
        if !HOST_UI_ENABLED {
            return;
        }
        let format = self
            .surface_config
            .as_ref()
            .map(|c| c.format)
            .unwrap_or(TEX_FORMAT);
        if self.host_ui_format != Some(format) {
            self.host_ui = Some(crate::host_ui::HostUi::new(
                &self.gpu.device,
                &self.gpu.queue,
                format,
            ));
            self.host_ui_format = Some(format);
        }
        if let Some(ui) = self.host_ui.as_mut() {
            let input = std::mem::take(&mut self.pending_ui_input);
            ui.set_input(input);
            ui.begin_frame(width, height);
        }
    }

    /// Пересобрать swapchain и все ресурсы кадра после resize окна.
    pub fn resize_framebuffer(&mut self, width: u32, height: u32) {
        let w = width.max(1);
        let h = height.max(1);
        if let Some(config) = &mut self.surface_config {
            config.width = w;
            config.height = h;
            if let Some(surface) = &self.surface {
                surface.configure(&self.gpu.device, config);
            }
        }
        self.ensure_hdr_texture(w, h);
        // Drop all per-target depth textures so they recreate at the new size on next use.
        self.depth_pool.clear();
        self.forward_graph_mut().pool_mut().clear();
        self.postfx.invalidate_size();
        self.offscreen = None;
        self.skinned_mesh_globals_bgs.clear();
        self.mesh_pass_static_handles.clear();
        self.mesh_pass_skinned_handles.clear();
    }

    /// Alias for [`Self::resize_framebuffer`].
    pub fn resize_surface(&mut self, width: u32, height: u32) {
        self.resize_framebuffer(width, height);
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
            let bg = self
                .gpu
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
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
        let bpr = (4 * w + 255) & !255;
        let buf_size = (bpr as u64) * (*h as u64);
        let buf = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map readback debug"),
            size: buf_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .gpu
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
        self.gpu.queue.submit([encoder.finish()]);
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll map readback");
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        self.gpu
            .device
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
        self.sky.clear_pipelines();
        self.debug_pipeline = None;
        self.debug_pipeline_occluded = None;
        self.hdr.tonemap_pipeline = None;
        self.hdr.tonemap_pipeline_format = None;
        self.invalidate_material_bgs();
    }

    pub fn set_godrays(&mut self, enabled: bool, strength: f32, decay: f32, density: f32) {
        self.postfx.godray_strength = if enabled { strength.max(0.0) } else { 0.0 };
        self.postfx.godray_decay = decay.clamp(0.5, 0.99);
        self.postfx.godray_density = density.clamp(0.2, 2.0);
    }

    pub fn set_aa_strength(&mut self, strength: f32) {
        self.postfx.aa_strength = strength.clamp(0.0, 1.0);
    }

    // ── Nuklear ──────────────────────────────────────────────────

    pub fn swapchain_layout(&self) -> Option<(u32, u32, wgpu::TextureFormat)> {
        self.surface_config
            .as_ref()
            .map(|c| (c.width, c.height, c.format))
    }

    pub fn host_ui_end_frame(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) {
        if !HOST_UI_ENABLED {
            return;
        }
        if let Some(ui) = self.host_ui.as_mut() {
            ui.end_frame(&self.gpu.device, &self.gpu.queue, encoder, target);
        }
    }

    pub fn render_to_surface(
        &mut self,
        snapshot: &RenderSnapshot,
        shaders: &ShaderRegistry,
    ) -> Result<(), FrameError> {
        // 1. Read back previous frame's luminance (if ready)
        self.process_lum_readback();
        self.mesh_occlusion.process_readback(&self.gpu.device);

        let Some(surface) = &self.surface else {
            return Ok(());
        };

        let acquire_start = std::time::Instant::now();
        crate::gamelib_profile::zone_begin("render.acquire");
        let acquired = surface.get_current_texture();
        crate::gamelib_profile::zone_end();
        let current = match acquired {
            wgpu::CurrentSurfaceTexture::Success(st) => st,
            wgpu::CurrentSurfaceTexture::Suboptimal(st) => st,
            wgpu::CurrentSurfaceTexture::Timeout => return Err(FrameError::SurfaceTimeout),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(FrameError::SurfaceOccluded),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(FrameError::SurfaceOutdated),
            wgpu::CurrentSurfaceTexture::Lost => return Err(FrameError::SurfaceLost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(FrameError::SurfaceValidation),
        };
        self.last_profile.surface_acquire_ms = acquire_start.elapsed().as_secs_f32() * 1000.0;
        let surface_view = current
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let format = self
            .surface_config
            .as_ref()
            .map(|c| c.format)
            .unwrap_or(TEX_FORMAT);

        let hdr_view = self.hdr.view.clone().expect("HDR texture not initialized");

        let (sw, sh) = self
            .surface_config
            .as_ref()
            .map(|c| (c.width as f32, c.height as f32))
            .unwrap_or((800.0, 600.0));
        self.host_ui_begin_frame(sw, sh);

        let encoder = self.encode_compose(
            std::slice::from_ref(snapshot),
            snapshot,
            None,
            &hdr_view,
            format,
            Some(&surface_view),
            shaders,
        );

        self.finish_surface_encoder(encoder, current, &surface_view, None, None);
        Ok(())
    }

    /// Multi-camera frame: cameras by `order` → HUD/tonemap (Bevy-style stack).
    pub fn render_to_surface_compose(
        &mut self,
        cameras: &[RenderSnapshot],
        post: &RenderSnapshot,
        geometry: Option<&crate::snapshot::SceneGeometry>,
        shaders: &ShaderRegistry,
    ) -> Result<(), FrameError> {
        self.process_lum_readback();
        self.mesh_occlusion.process_readback(&self.gpu.device);

        let Some(surface) = &self.surface else {
            return Ok(());
        };

        let acquire_start = std::time::Instant::now();
        crate::gamelib_profile::zone_begin("render.acquire");
        let acquired = surface.get_current_texture();
        crate::gamelib_profile::zone_end();
        let current = match acquired {
            wgpu::CurrentSurfaceTexture::Success(st) => st,
            wgpu::CurrentSurfaceTexture::Suboptimal(st) => st,
            wgpu::CurrentSurfaceTexture::Timeout => return Err(FrameError::SurfaceTimeout),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(FrameError::SurfaceOccluded),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(FrameError::SurfaceOutdated),
            wgpu::CurrentSurfaceTexture::Lost => return Err(FrameError::SurfaceLost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(FrameError::SurfaceValidation),
        };
        self.last_profile.surface_acquire_ms = acquire_start.elapsed().as_secs_f32() * 1000.0;
        let surface_view = current
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let format = self
            .surface_config
            .as_ref()
            .map(|c| c.format)
            .unwrap_or(TEX_FORMAT);

        let hdr_view = self.hdr.view.clone().expect("HDR texture not initialized");

        let (sw, sh) = self
            .surface_config
            .as_ref()
            .map(|c| (c.width as f32, c.height as f32))
            .unwrap_or((800.0, 600.0));
        // Match `render_to_surface_compose_with_game_frame` / `render_to_surface_with_hdr`:
        // `finish_surface_encoder` ends the host Nuklear frame, so begin must run
        // here or the next frame has no open context.
        self.host_ui_begin_frame(sw, sh);

        let encoder = self.encode_compose(
            cameras,
            post,
            geometry,
            &hdr_view,
            format,
            Some(&surface_view),
            shaders,
        );

        self.finish_surface_encoder(encoder, current, &surface_view, None, None);
        Ok(())
    }

    pub fn render_to_surface_compose_with_game_frame(
        &mut self,
        cameras: &[RenderSnapshot],
        post: &RenderSnapshot,
        geometry: Option<&crate::snapshot::SceneGeometry>,
        game_frame_fn: Option<extern "C" fn(*mut std::ffi::c_void)>,
        game_frame_ctx: *mut std::ffi::c_void,
        gamelib_ui_render: Option<deerpunk_engine_ffi::GamelibUiRenderFn>,
        shaders: &ShaderRegistry,
    ) -> Result<(), FrameError> {
        crate::gamelib_profile::zone_begin("render.readback");
        self.process_lum_readback();
        self.mesh_occlusion.process_readback(&self.gpu.device);
        crate::gamelib_profile::zone_end();

        let Some(surface) = &self.surface else {
            return Ok(());
        };

        let acquire_start = std::time::Instant::now();
        crate::gamelib_profile::zone_begin("render.acquire");
        let acquired = surface.get_current_texture();
        crate::gamelib_profile::zone_end();
        let current = match acquired {
            wgpu::CurrentSurfaceTexture::Success(st) => st,
            wgpu::CurrentSurfaceTexture::Suboptimal(st) => st,
            wgpu::CurrentSurfaceTexture::Timeout => return Err(FrameError::SurfaceTimeout),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(FrameError::SurfaceOccluded),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(FrameError::SurfaceOutdated),
            wgpu::CurrentSurfaceTexture::Lost => return Err(FrameError::SurfaceLost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(FrameError::SurfaceValidation),
        };
        self.last_profile.surface_acquire_ms = acquire_start.elapsed().as_secs_f32() * 1000.0;
        let surface_view = current
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let format = self
            .surface_config
            .as_ref()
            .map(|c| c.format)
            .unwrap_or(TEX_FORMAT);

        let hdr_view = self.hdr.view.clone().expect("HDR texture not initialized");

        let (sw, sh) = self
            .surface_config
            .as_ref()
            .map(|c| (c.width as f32, c.height as f32))
            .unwrap_or((800.0, 600.0));
        self.host_ui_begin_frame(sw, sh);

        let game_ui_start = std::time::Instant::now();
        crate::gamelib_profile::zone_begin("render.game_ui");
        if let Some(f) = game_frame_fn {
            f(game_frame_ctx);
        }
        crate::gamelib_profile::zone_end();
        let game_frame_ui_ms = game_ui_start.elapsed().as_secs_f32() * 1000.0;

        crate::gamelib_profile::zone_begin("render.encode");
        let encoder = self.encode_compose(
            cameras,
            post,
            geometry,
            &hdr_view,
            format,
            Some(&surface_view),
            shaders,
        );
        crate::gamelib_profile::zone_end();
        self.finish_surface_encoder(
            encoder,
            current,
            &surface_view,
            Some(game_frame_ui_ms),
            gamelib_ui_render,
        );
        Ok(())
    }

    fn encode_compose(
        &mut self,
        cameras: &[RenderSnapshot],
        post: &RenderSnapshot,
        geometry: Option<&crate::snapshot::SceneGeometry>,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        surface_view: Option<&wgpu::TextureView>,
        shaders: &ShaderRegistry,
    ) -> wgpu::CommandEncoder {
        use crate::frame::scope::Phase;

        // `write_buffer` goes through wgpu's staging belt and is ordered before this
        // submit. The uniform ring keeps the previous frame's slot alive. Do not
        // `poll(wait_indefinitely)` here — that stalls the GPU for a full frame.
        self.upload_prepped = false;
        self.frame_uniform_ring =
            self.frame_uniform_ring.wrapping_add(1) % FRAME_UNIFORM_RING as u8;
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
        prof.graph_nodes.clear();

        let primary = cameras.first().unwrap_or(post);
        prof.sprite_instances = primary.instances.len() as u32;
        prof.hud_instances = post.hud.len() as u32;
        prof.mesh_draws = cameras
            .iter()
            .map(|c| crate::draw_list::snapshot_mesh_draw_count(c, geometry))
            .sum();
        prof.terrain_draws = primary.terrain_splat_draws.len() as u32;
        prof.point_lights = primary.point_lights.len() as u32;
        prof.width = frame_width();
        prof.height = frame_height();

        let pipeline = match self.pipelines.get(&self.hdr.format) {
            Some(p) => p.clone(),
            None => {
                let p = self.build_pipeline(self.hdr.format);
                self.pipelines.insert(self.hdr.format, p.clone());
                p
            }
        };

        let fw = frame_width() as f32;
        let fh = frame_height() as f32;
        let hud_vp: [[f32; 4]; 4] = [
            [2.0 / fw, 0.0, 0.0, 0.0],
            [0.0, -2.0 / fh, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0, 1.0],
        ];

        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });

        for cam in cameras {
            let phase = Phase::Camera {
                pass_mask: cam.pass_mask,
            };
            let mut frame_ctx = FrameCtx {
                snapshot: cam,
                geometry,
                hdr_view: view,
                hdr_format: self.hdr.format,
                surface_format: format,
                surface_view,
                shaders,
                prof: &mut prof,
                draw_acc: &mut draw_acc,
                lap: &mut lap,
            };
            crate::frame::executor::run_frame_graph(
                self,
                &mut frame_ctx,
                &mut encoder,
                &pipeline,
                hud_vp,
                phase,
            );
        }
        {
            let mut frame_ctx = FrameCtx {
                snapshot: post,
                geometry,
                hdr_view: view,
                hdr_format: self.hdr.format,
                surface_format: format,
                surface_view,
                shaders,
                prof: &mut prof,
                draw_acc: &mut draw_acc,
                lap: &mut lap,
            };
            crate::frame::executor::run_frame_graph(
                self,
                &mut frame_ctx,
                &mut encoder,
                &pipeline,
                hud_vp,
                Phase::Post,
            );
        }

        prof.total_encode_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
        let frustum_occluded = if primary.frustum_culled {
            let cs = primary.cull_stats;
            (cs.mesh_total - cs.mesh_visible) + (cs.skinned_total - cs.skinned_visible)
        } else {
            0
        };
        let (_, gpu_occluded) = self.mesh_occlusion.last_stats();
        let occluded = frustum_occluded + gpu_occluded;
        let visible_instances = primary.instances.len() as u32
            + prof.mesh_draws
            + primary.terrain_splat_draws.len() as u32;
        prof.speeds_hud = FrameSpeedsHud {
            dip_count: draw_acc.dip_count,
            shader_bind: draw_acc.shader_bind,
            render_passes: draw_acc.render_passes,
            bind_groups: draw_acc.bind_groups,
            vertex_binds: draw_acc.vertex_binds,
            index_binds: draw_acc.index_binds,
            indirect_draws: draw_acc.indirect_draws,
            frame_total_tris: (draw_acc.index_count / 3).min(u32::MAX as u64) as u32,
            visible_world_lights: post.visible_world_lights,
            visible_dyn_lights: primary.point_lights.len() as u32,
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

    pub fn render_to_surface_with_game_frame(
        &mut self,
        snapshot: &RenderSnapshot,
        game_frame_fn: Option<extern "C" fn(*mut std::ffi::c_void)>,
        game_frame_ctx: *mut std::ffi::c_void,
        gamelib_ui_render: Option<deerpunk_engine_ffi::GamelibUiRenderFn>,
        shaders: &ShaderRegistry,
    ) -> Result<(), FrameError> {
        self.render_to_surface_compose_with_game_frame(
            std::slice::from_ref(snapshot),
            snapshot,
            None,
            game_frame_fn,
            game_frame_ctx,
            gamelib_ui_render,
            shaders,
        )
    }

    fn finish_surface_encoder(
        &mut self,
        mut encoder: wgpu::CommandEncoder,
        current: wgpu::SurfaceTexture,
        surface_view: &wgpu::TextureView,
        game_frame_ui_ms: Option<f32>,
        gamelib_ui_render: Option<deerpunk_engine_ffi::GamelibUiRenderFn>,
    ) {
        crate::gamelib_profile::zone_begin("render.poll");
        let _ = self.gpu.device.poll(wgpu::PollType::Poll);
        crate::gamelib_profile::zone_end();
        let tail_start = std::time::Instant::now();

        crate::gamelib_profile::zone_begin("render.host_ui");
        if let Some(f) = gamelib_ui_render {
            let (sw, sh) = self
                .surface_config
                .as_ref()
                .map(|c| (c.width as f32, c.height as f32))
                .unwrap_or((800.0, 600.0));
            let args = deerpunk_engine_ffi::GamelibUiRenderArgs {
                version: deerpunk_engine_ffi::GAMELIB_UI_API_VERSION,
                encoder: (&mut encoder as *mut wgpu::CommandEncoder).cast::<std::ffi::c_void>(),
                color_view: surface_view as *const wgpu::TextureView as *mut std::ffi::c_void,
                width: sw,
                height: sh,
            };
            unsafe {
                let _ = f(&args);
            }
        }

        self.host_ui_end_frame(&mut encoder, surface_view);
        crate::gamelib_profile::zone_end();
        let after_host_ui_ms = tail_start.elapsed().as_secs_f32() * 1000.0;

        let lum_copied = self.append_lum_copy_to_encoder(&mut encoder);

        let finish_start = std::time::Instant::now();
        crate::gamelib_profile::zone_begin("render.finish");
        let cmd_buf = encoder.finish();
        crate::gamelib_profile::zone_end();
        let encoder_finish_ms = finish_start.elapsed().as_secs_f32() * 1000.0;

        let tail = PendingSurfaceTail {
            after_host_ui_ms,
            encoder_finish_ms,
            game_frame_ui_ms,
            after_encode_ms: tail_start.elapsed().as_secs_f32() * 1000.0,
        };

        if self.defer_surface_submit {
            self.pending_surface_present = Some(PendingSurfacePresent {
                cmds: vec![cmd_buf],
                surface: current,
                lum_copied,
                occlusion_queries: self.frame_occlusion_queries,
                tail,
            });
            return;
        }

        let submit_start = std::time::Instant::now();
        crate::gamelib_profile::zone_begin("render.submit");
        self.gpu.queue.submit([cmd_buf]);
        crate::gamelib_profile::zone_end();
        let queue_submit_ms = submit_start.elapsed().as_secs_f32() * 1000.0;
        self.queue_lum_readback_after_submit(lum_copied);
        if self.frame_occlusion_queries {
            self.mesh_occlusion.queue_readback_after_submit();
        }
        let after_submit_ms = encoder_finish_ms + queue_submit_ms;

        let present_start = std::time::Instant::now();
        crate::gamelib_profile::zone_begin("render.present");
        self.gpu.queue.present(current);
        crate::gamelib_profile::zone_end();
        let after_present_ms = present_start.elapsed().as_secs_f32() * 1000.0;

        if let Some(ui) = game_frame_ui_ms {
            self.last_profile.game_frame_ui_ms = ui;
        }
        self.last_profile.after_host_ui_ms = after_host_ui_ms;
        self.last_profile.encoder_finish_ms = encoder_finish_ms;
        self.last_profile.queue_submit_ms = queue_submit_ms;
        self.last_profile.after_submit_ms = after_submit_ms;
        self.last_profile.after_present_ms = after_present_ms;
        self.last_profile.after_encode_ms = tail.after_encode_ms;
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
        let _ = self.gpu.device.poll(wgpu::PollType::wait_indefinitely());

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
        self.gpu.queue.submit([encoder.finish()]);

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
            let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
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
                self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("offscreen readback 0"),
                    size: (w * h * 4) as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
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

        let hal_device = unsafe { self.gpu.device.as_hal::<wgpu::hal::api::Dx12>() }?;
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

            let texture = self
                .gpu
                .device
                .create_texture_from_hal::<wgpu::hal::api::Dx12>(
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

    /// D3D12 HANDLE export — not available on macOS/Linux (Metal/Vulkan).
    #[cfg(not(target_os = "windows"))]
    #[allow(dead_code)]
    pub fn create_shared_texture(&mut self, _w: u32, _h: u32) -> Option<SharedTexture> {
        None
    }

    /// Create a double-buffered pair of shared GPU textures for zero-copy viewport.
    #[cfg(target_os = "windows")]
    pub fn create_shared_texture_pair(&mut self, w: u32, h: u32) -> Option<SharedTexturePair> {
        let a = self.create_shared_texture(w, h)?;
        let b = self.create_shared_texture(w, h)?;
        Some(SharedTexturePair {
            buffers: [a, b],
            read_idx: 0,
        })
    }

    #[cfg(not(target_os = "windows"))]
    #[allow(dead_code)]
    pub fn create_shared_texture_pair(&mut self, _w: u32, _h: u32) -> Option<SharedTexturePair> {
        None
    }

    /// Render the scene directly into a shared GPU texture (zero-copy viewport).
    /// The texture was created with D3D12_SHARED so the editor can import it without CPU readback.
    #[cfg(target_os = "windows")]
    pub fn render_into_shared(
        &mut self,
        snapshot: &RenderSnapshot,
        shared: &SharedTexture,
        shaders: &ShaderRegistry,
    ) {
        self.ensure_depth(snapshot.depth.target, shared.width, shared.height);
        let mut encoder = self.encode_frame(snapshot, &shared.view, TEX_FORMAT, None, shaders);
        self.gpu.queue.submit([encoder.finish()]);
        self.gpu.device.poll(wgpu::PollType::wait_indefinitely());
    }

    #[cfg(not(target_os = "windows"))]
    #[allow(dead_code)]
    pub fn render_into_shared(
        &mut self,
        _snapshot: &RenderSnapshot,
        _shared: &SharedTexture,
        _shaders: &ShaderRegistry,
    ) {
    }

    fn encode_frame(
        &mut self,
        snapshot: &RenderSnapshot,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        surface_view: Option<&wgpu::TextureView>,
        shaders: &ShaderRegistry,
    ) -> wgpu::CommandEncoder {
        self.upload_prepped = false;
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

        let pipeline = match self.pipelines.get(&self.hdr.format) {
            Some(p) => p.clone(),
            None => {
                let p = self.build_pipeline(self.hdr.format);
                self.pipelines.insert(self.hdr.format, p.clone());
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
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });

        let mut frame_ctx = FrameCtx {
            snapshot,
            geometry: None,
            hdr_view: view,
            hdr_format: self.hdr.format,
            surface_format: format,
            surface_view,
            shaders,
            prof: &mut prof,
            draw_acc: &mut draw_acc,
            lap: &mut lap,
        };

        crate::frame::executor::run_frame_graph(
            self,
            &mut frame_ctx,
            &mut encoder,
            &pipeline,
            hud_vp,
            crate::frame::scope::Phase::Camera {
                pass_mask: snapshot.pass_mask,
            },
        );

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
            render_passes: draw_acc.render_passes,
            bind_groups: draw_acc.bind_groups,
            vertex_binds: draw_acc.vertex_binds,
            index_binds: draw_acc.index_binds,
            indirect_draws: draw_acc.indirect_draws,
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
        self.ui_upload = frame.is_some();
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
            .gpu
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
        self.gpu.queue.submit([encoder.finish()]);
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
