//! Static mesh pass, then skinned mesh pass.

use std::collections::HashMap;

use bytemuck::Zeroable;
use deerpunk_engine_common::frame::{frame_height, frame_width};
use deerpunk_engine_common::material::{ShaderId, VertexLayoutSpec};

use crate::asset_inventory::{AssetEntry, AssetKind, ASSET_ID_MESH};
use crate::bone_gpu::{GpuBoneBlock, GPU_BONE_BLOCK_SIZE};
use crate::frame::context::FrameCtx;
use crate::mesh_allocator::MESH_INDEX_SIZE;
use crate::mesh_vertex_pack;
use crate::render_profile::lap_ms;
use crate::renderer::{
    build_rgba_mipmap_chain, texture_mip_levels, GpuPointLight, LightsUniform, MeshBatchKey,
    MeshUniform, Renderer, SkinnedMeshVertex, ALBEDO_TEX_FORMAT, DEPTH_FORMAT, FRAME_UNIFORM_RING,
    MAX_POINT_LIGHTS, MESH_UNIFORM_SLOTS_INIT, TEX_FORMAT,
};
use crate::shader_registry::ShaderRegistry;
use crate::snapshot::MeshTexture;

type MeshPipelineKey = (u32, bool, bool);

struct MeshMdiSegment {
    buffer_id: u32,
    byte_offset: u64,
    cmd_count: u32,
    index_count: u32,
}

macro_rules! render_scope {
    ($label:literal) => {
        #[cfg(any(feature = "tracy", feature = "puffin"))]
        let _render_profile =
            crate::profile_active().then(|| crate::gamelib_profile::zone_scope($label));
    };
}

/// `face == 0` always draws. Cardinal faces drop when the camera is outside
/// the chunk on the far side of that normal (Sodium section-face cull).
fn cardinal_face_visible(md: &crate::snapshot::MeshDraw, cam: [f32; 3]) -> bool {
    let b = md.world_bounds;
    if !b.is_valid() {
        return true;
    }
    match md.face {
        1 => cam[0] <= b.max.x,
        2 => cam[0] >= b.min.x,
        3 => cam[1] <= b.max.y,
        4 => cam[1] >= b.min.y,
        5 => cam[2] <= b.max.z,
        6 => cam[2] >= b.min.z,
        _ => true,
    }
}

/// Same `mesh_revision` is not enough: draw slots are reused for other chunks that were never re-uploaded.
fn mdi_gpu_cache_hit(cached_mesh: usize, cached_rev: u64, mesh_id: usize, revision: u64) -> bool {
    revision != 0 && cached_mesh == mesh_id && cached_rev == revision
}

fn push_mesh_indirect_cmd(
    scratch: &mut Vec<wgpu::util::DrawIndexedIndirectArgs>,
    draw_index: usize,
    md: &crate::snapshot::MeshDraw,
    handle: crate::mesh_allocator::MeshHandle,
) -> u32 {
    let start = md.first_index;
    let count = if md.index_count == 0 {
        handle.idx_count
    } else {
        md.index_count.min(handle.idx_count.saturating_sub(start))
    };
    if count == 0 {
        return 0;
    }
    scratch.push(wgpu::util::DrawIndexedIndirectArgs {
        index_count: count,
        instance_count: 1,
        first_index: handle.idx_offset.saturating_add(start),
        base_vertex: handle.vert_offset as i32,
        first_instance: draw_index as u32,
    });
    count
}

/// Pipeline cache key for **static** mesh draws (`mesh_globals_bg`, never skinned group-0 layout).
fn static_mesh_pipeline_key(shaders: &ShaderRegistry, shader_id: u32) -> MeshPipelineKey {
    let Some(def) = shaders.get(shader_id) else {
        return (shader_id, false, false);
    };
    (
        shader_id,
        def.group1_uses_lightmap_array(),
        def.mesh_transparent(),
    )
}

fn hash_batch_draw(mut h: u64, md: &crate::snapshot::MeshDraw) -> u64 {
    h = h.wrapping_mul(31).wrapping_add(md.shader_id as u64);
    h = h.wrapping_mul(31).wrapping_add(md.material_id as u64);
    h = h.wrapping_mul(31).wrapping_add(md.texture.0 as u64);
    h = h.wrapping_mul(31).wrapping_add(md.normal.0 as u64);
    h = h.wrapping_mul(31).wrapping_add(md.orm.0 as u64);
    h = h.wrapping_mul(31).wrapping_add(md.lightmap_gpu as u64);
    h = h.wrapping_mul(31).wrapping_add(md.batchable as u64);
    h = h.wrapping_mul(31).wrapping_add(md.mesh.0 as u64);
    h
}

/// Fingerprint of a draw list. Bench entry for the batch-plan cache key.
#[doc(hidden)]
pub fn bench_batch_fingerprint(draws: &[crate::snapshot::MeshDraw]) -> u64 {
    let mut h = draws.len() as u64;
    for md in draws {
        h = hash_batch_draw(h, md);
    }
    h | 1
}

/// Cache probes for one frame. Each row is `(cached mesh id, cached revision, current mesh id, current revision)`.
#[doc(hidden)]
pub fn bench_mdi_cache_probes(rows: &[(usize, u64, usize, u64)]) -> u32 {
    let mut hits = 0u32;
    for &(cached_mesh, cached_rev, mesh_id, revision) in rows {
        if mdi_gpu_cache_hit(cached_mesh, cached_rev, mesh_id, revision) {
            hits += 1;
        }
    }
    hits
}

/// Group draws by GPU buffer and write indirect commands. Bench entry for `build_static_mdi`.
#[doc(hidden)]
pub fn bench_build_indirect(
    draws: &[crate::snapshot::MeshDraw],
    gpu: &[crate::mesh_allocator::MeshHandle],
    order: &mut Vec<(u32, usize)>,
    scratch: &mut Vec<wgpu::util::DrawIndexedIndirectArgs>,
) -> u32 {
    order.clear();
    scratch.clear();
    for (i, handle) in gpu.iter().enumerate() {
        order.push((handle.buffer_id, i));
    }
    order.sort_unstable_by_key(|entry| entry.0);
    let mut cmds = 0u32;
    for &(_, i) in order.iter() {
        cmds += push_mesh_indirect_cmd(scratch, i, &draws[i], gpu[i]);
    }
    cmds
}

/// Same scratch the renderer keeps across frames. The bench writes the plan back,
/// which is what `encode_static_mesh_pass` does after `static_batch_plan`.
#[doc(hidden)]
pub struct StaticBatchPlanBench {
    opaque_count: usize,
    order: Vec<usize>,
    groups: HashMap<MeshBatchKey, Vec<usize>>,
    for_index: Vec<Option<MeshBatchKey>>,
    pairs: Vec<(MeshBatchKey, u32)>,
}

impl StaticBatchPlanBench {
    pub fn new() -> Self {
        Self {
            opaque_count: 0,
            order: Vec::new(),
            groups: HashMap::new(),
            for_index: Vec::new(),
            pairs: Vec::new(),
        }
    }

    /// One `static_batch_plan` frame. Every draw is treated as opaque and batchable.
    pub fn frame(&mut self, draws: &[crate::snapshot::MeshDraw], camera: [f32; 3]) -> u64 {
        let _ = camera;
        let (order, groups, for_index) = run_static_batch_plan(
            &mut self.opaque_count,
            &mut self.order,
            &mut self.groups,
            &mut self.for_index,
            &mut self.pairs,
            draws.len(),
            |i| &draws[i],
            true,
            |_, _| true,
        );
        let groups_n = groups.len() as u64;
        self.order = order;
        self.groups = groups;
        self.for_index = for_index;
        groups_n
    }
}

impl Default for StaticBatchPlanBench {
    fn default() -> Self {
        Self::new()
    }
}

fn submission_key_ord(
    for_index: &[Option<MeshBatchKey>],
    draw: &crate::snapshot::MeshDraw,
    index: usize,
) -> (u32, u32, u32, u32, u32) {
    if let Some(key) = for_index[index] {
        return (
            key.shader_id,
            key.material_id,
            key.texture,
            key.normal,
            key.orm,
        );
    }
    (
        draw.shader_id,
        draw.material_id,
        draw.texture.0,
        draw.normal.0,
        draw.orm.0,
    )
}

fn run_static_batch_plan<'a>(
    opaque_count: &mut usize,
    stored_order: &mut Vec<usize>,
    stored_groups: &mut HashMap<MeshBatchKey, Vec<usize>>,
    stored_for_index: &mut Vec<Option<MeshBatchKey>>,
    pairs: &mut Vec<(MeshBatchKey, u32)>,
    len: usize,
    draw: impl Fn(usize) -> &'a crate::snapshot::MeshDraw,
    batching: bool,
    visible: impl Fn(usize, &crate::snapshot::MeshDraw) -> bool,
) -> (
    Vec<usize>,
    HashMap<MeshBatchKey, Vec<usize>>,
    Vec<Option<MeshBatchKey>>,
) {
    let mut groups = std::mem::take(stored_groups);
    for bucket in groups.values_mut() {
        bucket.clear();
    }
    pairs.clear();
    let mut for_index = std::mem::take(stored_for_index);
    for_index.clear();
    for_index.resize(len, None);

    let mut order = std::mem::take(stored_order);
    order.clear();

    for i in 0..len {
        let md = draw(i);
        if !batching || !md.batchable || !visible(i, md) {
            order.push(i);
            continue;
        }
        pairs.push((
            MeshBatchKey {
                shader_id: md.shader_id,
                lm_surface: false,
                material_id: md.material_id,
                texture: md.texture.0,
                normal: md.normal.0,
                orm: md.orm.0,
                lightmap_gpu: 0,
            },
            i as u32,
        ));
    }
    pairs.sort_unstable_by(|a, b| {
        a.0.shader_id
            .cmp(&b.0.shader_id)
            .then(a.0.material_id.cmp(&b.0.material_id))
            .then(a.0.texture.cmp(&b.0.texture))
            .then(a.0.normal.cmp(&b.0.normal))
            .then(a.0.orm.cmp(&b.0.orm))
            .then(a.0.lightmap_gpu.cmp(&b.0.lightmap_gpu))
    });
    let mut run = 0usize;
    while run < pairs.len() {
        let key = pairs[run].0;
        let mut end = run + 1;
        while end < pairs.len() && pairs[end].0 == key {
            end += 1;
        }
        if end - run < 2 {
            order.push(pairs[run].1 as usize);
        } else {
            let bucket = groups.entry(key).or_default();
            bucket.clear();
            for pair in &pairs[run..end] {
                let i = pair.1 as usize;
                for_index[i] = Some(key);
                bucket.push(i);
            }
            order.push(pairs[run].1 as usize);
        }
        run = end;
    }
    pairs.clear();
    order.sort_unstable_by(|&a, &b| {
        submission_key_ord(&for_index, draw(a), a).cmp(&submission_key_ord(&for_index, draw(b), b))
    });
    *opaque_count = order.len();

    (order, groups, for_index)
}

use crate::draw_list::{MeshDrawListView, SkinnedDrawListView};

fn mesh_index_span(first_index: u32, index_count: u32, handle_idx_count: u32) -> u32 {
    if index_count == 0 {
        handle_idx_count
    } else {
        index_count.min(handle_idx_count.saturating_sub(first_index))
    }
}

fn snapshot_depth_load(load: deerpunk_engine_common::components::DepthLoad) -> wgpu::LoadOp<f32> {
    match load {
        deerpunk_engine_common::components::DepthLoad::Clear(v) => wgpu::LoadOp::Clear(v),
        deerpunk_engine_common::components::DepthLoad::Load => wgpu::LoadOp::Load,
        deerpunk_engine_common::components::DepthLoad::None => wgpu::LoadOp::Clear(1.0),
    }
}

fn begin_geometry_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &'static str,
    color: &'a wgpu::TextureView,
    depth: &'a wgpu::TextureView,
    depth_load: wgpu::LoadOp<f32>,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: color,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: depth_load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

struct StaticDrawStats<'a> {
    bins: &'a mut HashMap<(u32, bool), (u32, u64)>,
    sample_set: &'a mut bool,
}

impl StaticDrawStats<'_> {
    pub(in crate::renderer) fn record(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        shader_id: u32,
        lm_surface: bool,
        texture: u32,
        lightmap_gpu: u32,
        material_id: u32,
        has_bsp: bool,
        draws: u32,
        tris: u64,
        bind_batched: bool,
    ) {
        if draws == 0 {
            return;
        }
        ctx.prof.mesh_stats.draws_submitted += draws;
        ctx.prof.mesh_stats.tris_drawn += tris;
        if bind_batched {
            ctx.prof.mesh_stats.bind_batch_draws += draws;
        }
        if lm_surface {
            ctx.prof.mesh_stats.lm_array_layout_draws += draws;
            if lightmap_gpu == 0 {
                ctx.prof.mesh_stats.lm_array_with_gpu_id_zero += draws;
            }
        } else {
            ctx.prof.mesh_stats.albedo_2d_layout_draws += draws;
        }
        if texture == MeshTexture::WHITE.0 {
            ctx.prof.mesh_stats.albedo_white_slot_draws += draws;
        }
        let bin = self.bins.entry((shader_id, lm_surface)).or_insert((0, 0));
        bin.0 += draws;
        bin.1 += tris;
        if !*self.sample_set {
            *self.sample_set = true;
            ctx.prof.mesh_stats.sample_shader_id = shader_id;
            ctx.prof.mesh_stats.sample_shader_path = ctx
                .shaders
                .path_for_id(shader_id)
                .unwrap_or("?")
                .to_string();
            ctx.prof.mesh_stats.sample_lm_array = lm_surface;
            ctx.prof.mesh_stats.sample_albedo_slot = texture;
            ctx.prof.mesh_stats.sample_lightmap_gpu = lightmap_gpu;
            ctx.prof.mesh_stats.sample_material_id = material_id;
            ctx.prof.mesh_stats.sample_has_bsp_mesh = has_bsp;
        }
    }
}

impl Renderer {
    pub(crate) fn pass_mesh(&mut self, ctx: &mut FrameCtx<'_>, encoder: &mut wgpu::CommandEncoder) {
        // 1. Nothing to draw: still close the mesh timer.
        let static_count = MeshDrawListView::new(ctx.snapshot, ctx.geometry).len();
        let skinned_count = SkinnedDrawListView::new(ctx.snapshot, ctx.geometry).len();
        let owns_scene_occlusion =
            ctx.snapshot.depth.target == deerpunk_engine_common::components::DepthTargetId(0);
        if static_count == 0 && skinned_count == 0 {
            ctx.prof.mesh_stats.draw_encode_ms = lap_ms(ctx.lap);
            return;
        }

        // 2. Upload view, lights, and per-draw uniforms once for both passes.
        let occlusion_active = self.mesh_occlusion.filters_draws() && owns_scene_occlusion;
        self.prepare_mesh_frame(ctx, occlusion_active);

        // 3. Depth target. Static pass applies the camera load; skinned loads the result.
        let depth_view = {
            render_scope!("ensure_depth");
            self.ensure_depth(ctx.snapshot.depth.target, frame_width(), frame_height())
        };
        let depth_load = snapshot_depth_load(ctx.snapshot.depth.load);
        let mut shader_bins: HashMap<(u32, bool), (u32, u64)> = HashMap::new();
        let mut sample_set = false;

        // 4. Static geometry.
        if static_count > 0 {
            render_scope!("encode_static_mesh_pass");
            self.encode_static_mesh_pass(
                ctx,
                encoder,
                &depth_view,
                depth_load,
                occlusion_active,
                &mut shader_bins,
                &mut sample_set,
            );
        }

        // 5. Skinned geometry. Load color and depth when the static pass already ran.
        if skinned_count > 0 {
            let skinned_depth_load = if static_count > 0 {
                wgpu::LoadOp::Load
            } else {
                depth_load
            };
            self.encode_skinned_mesh_pass(
                ctx,
                encoder,
                &depth_view,
                skinned_depth_load,
                occlusion_active,
                static_count,
            );
        }

        ctx.prof.mesh_stats.mesh_for_draw_ms = lap_ms(ctx.lap);
        ctx.prof.mesh_gpu_uploads = self.frame_mesh_gpu_uploads;
        ctx.prof.mesh_gpu_cached = self.frame_mesh_gpu_cached;
        ctx.prof.mesh_stats.shader_bins = shader_bins
            .into_iter()
            .map(|((shader_id, lm_array_layout), (draws, tris))| {
                crate::render_profile::MeshShaderBin {
                    shader_id,
                    lm_array_layout,
                    draws,
                    tris,
                }
            })
            .collect();

        // 6. Occlusion queries read the depth both passes just wrote.
        if owns_scene_occlusion {
            render_scope!("encode_queries");
            self.frame_occlusion_queries |= self.mesh_occlusion.encode_queries(
                &self.gpu.device,
                &self.gpu.queue,
                encoder,
                ctx.snapshot,
                ctx.geometry,
                &depth_view,
                ctx.hdr_view,
                ctx.hdr_format,
                DEPTH_FORMAT,
            );
        }
        ctx.prof.mesh_stats.draw_encode_ms = lap_ms(ctx.lap);
    }

    /// Per-draw `MeshUniform` slots, camera view, and scene lights.
    pub(in crate::renderer) fn prepare_mesh_frame(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        occlusion_active: bool,
    ) {
        render_scope!("prepare_mesh_frame");

        let meshes = MeshDrawListView::new(ctx.snapshot, ctx.geometry);
        let skinned_meshes = SkinnedDrawListView::new(ctx.snapshot, ctx.geometry);
        let static_count = meshes.len();
        let skinned_count = skinned_meshes.len();
        let mesh_uniform_size = Self::mesh_uniform_stride();
        let draw_count = static_count + skinned_count;
        let fullbright_flag = u32::from(self.fullbright);
        let lightmap_flag = 0u32;

        self.ensure_mesh_uniform_slots(draw_count as u64);
        for palette in ctx.snapshot.bone_palettes.iter() {
            self.ensure_skinned_bone_bind_group(palette.handle, &palette.matrices);
        }

        let ring = self.frame_uniform_ring as usize;
        let slab_bytes = (mesh_uniform_size * draw_count as u64) as usize;
        assert!(
            slab_bytes as u64 <= self.mesh_globals_bufs[ring].size(),
            "mesh uniform upload {slab_bytes} bytes > mesh_globals_buf {} ({draw_count} draws)",
            self.mesh_globals_bufs[ring].size(),
        );
        self.mesh_uniform_scratch.resize(slab_bytes, 0);
        self.gpu.queue.write_buffer(
            &self.mesh_view_bufs[ring],
            0,
            bytemuck::bytes_of(&crate::renderer::MeshViewUniform {
                view_proj: ctx.snapshot.view_proj,
                camera_pos: ctx.snapshot.camera_pos,
                _pad_cam: 0.0,
            }),
        );

        let mut min_slot = draw_count as u64;
        let mut max_slot = 0u64;
        let mut wrote_uniform = false;
        for i in 0..static_count {
            let md = meshes.get(i);
            if !self.static_draw_visible(occlusion_active, i, md) {
                continue;
            }
            wrote_uniform = true;
            min_slot = min_slot.min(i as u64);
            max_slot = max_slot.max(i as u64);
            self.write_mesh_uniform_slot(
                i as u64,
                mesh_uniform_size,
                md.model_mat,
                [
                    md.material_id,
                    md.uniform_aux,
                    fullbright_flag,
                    lightmap_flag,
                ],
            );
        }
        for j in 0..skinned_count {
            let sd = skinned_meshes.get(j);
            if !self.skinned_draw_visible(occlusion_active, j, sd) {
                continue;
            }
            let slot = (static_count + j) as u64;
            wrote_uniform = true;
            min_slot = min_slot.min(slot);
            max_slot = max_slot.max(slot);
            self.write_mesh_uniform_slot(
                slot,
                mesh_uniform_size,
                sd.model_mat,
                [sd.material_id, 0, fullbright_flag, lightmap_flag],
            );
        }
        if wrote_uniform {
            let start_byte = (min_slot * mesh_uniform_size) as usize;
            let end_byte = ((max_slot + 1) * mesh_uniform_size) as usize;
            self.gpu.queue.write_buffer(
                &self.mesh_globals_bufs[ring],
                min_slot * mesh_uniform_size,
                &self.mesh_uniform_scratch[start_byte..end_byte],
            );
        }
        ctx.prof.mesh_stats.uniforms_ms = lap_ms(ctx.lap);

        self.mesh_pass_static_handles.clear();
        self.mesh_pass_static_handles.resize(static_count, None);
        self.mesh_pass_skinned_handles.clear();
        self.mesh_pass_skinned_handles.resize(skinned_count, None);

        let mut lights = LightsUniform::zeroed();
        let n = ctx.snapshot.point_lights.len().min(MAX_POINT_LIGHTS);
        lights.count = n as u32;
        lights.scene_time_bits = ctx.snapshot.game_time.to_bits();
        lights._pad[0] = self.scene_fog[0].to_bits();
        lights._pad[1] = self.scene_fog[1].to_bits();
        lights.zenith = [
            self.sky.scene_sky_zenith[0],
            self.sky.scene_sky_zenith[1],
            self.sky.scene_sky_zenith[2],
            0.0,
        ];
        lights.sun_dir = self.scene_sun_dir;
        lights.ambient = self.scene_ambient;
        for (i, pl) in ctx.snapshot.point_lights.iter().take(n).enumerate() {
            lights.lights[i] = GpuPointLight {
                position: pl.position,
                range: pl.range.max(1.0),
                color: pl.color,
                intensity: pl.intensity,
            };
        }
        self.gpu
            .queue
            .write_buffer(&self.lights_bufs[ring], 0, bytemuck::bytes_of(&lights));
    }

    pub(in crate::renderer) fn write_mesh_uniform_slot(
        &mut self,
        slot: u64,
        stride: u64,
        model: [[f32; 4]; 4],
        material_data: [u32; 4],
    ) {
        let uniform = MeshUniform {
            model,
            material_data,
        };
        let off = (slot * stride) as usize;
        let bytes = bytemuck::bytes_of(&uniform);
        self.mesh_uniform_scratch[off..off + bytes.len()].copy_from_slice(bytes);
    }

    pub(in crate::renderer) fn static_draw_visible(
        &self,
        occlusion_active: bool,
        index: usize,
        md: &crate::snapshot::MeshDraw,
    ) -> bool {
        !occlusion_active || self.mesh_occlusion.draw_visible_static(index, md)
    }

    pub(in crate::renderer) fn skinned_draw_visible(
        &self,
        occlusion_active: bool,
        index: usize,
        sd: &crate::snapshot::SkinnedMeshDraw,
    ) -> bool {
        !occlusion_active || self.mesh_occlusion.draw_visible_skinned(index, sd)
    }

    /// Opaque static draws. Skinned meshes are a separate pass. Lightmap arrays are not bound.
    pub(in crate::renderer) fn encode_static_mesh_pass(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        encoder: &mut wgpu::CommandEncoder,
        depth_view: &wgpu::TextureView,
        depth_load: wgpu::LoadOp<f32>,
        occlusion_active: bool,
        shader_bins: &mut HashMap<(u32, bool), (u32, u64)>,
        sample_set: &mut bool,
    ) {
        let (draw_order, batch_groups, batch_for_index) =
            self.static_batch_plan(ctx, occlusion_active);
        let batch_mdi = self.build_static_mdi(ctx, &batch_groups, occlusion_active);
        let meshes = MeshDrawListView::new(ctx.snapshot, ctx.geometry);

        ctx.draw_acc.note_pass();
        crate::bind_log::begin_pass();
        let mut pass = begin_geometry_pass(
            encoder,
            "static mesh pass",
            ctx.hdr_view,
            depth_view,
            depth_load,
        );
        let ring = self.frame_uniform_ring as usize;
        ctx.draw_acc.note_bind();
        pass.set_bind_group(3, &self.lights_bgs[ring], &[]);
        crate::bind_log::group(3, "lights");

        let mut last_pipeline: Option<MeshPipelineKey> = None;
        let mut globals_bound = false;
        let mut last_g1: Option<u32> = None;
        let mut last_g2: Option<(u32, u32, u32)> = None;
        let mut emitted = std::collections::HashSet::new();
        let mut stats = StaticDrawStats {
            bins: shader_bins,
            sample_set,
        };

        {
            render_scope!("work");
            for &i in &draw_order {
                let md = meshes.get(i);
                if !self.static_draw_visible(occlusion_active, i, md) {
                    continue;
                }
                if let Some(key) = batch_for_index[i] {
                    if emitted.insert(key) {
                        let group = batch_groups.get(&key).expect("batch key missing").clone();
                        self.draw_static_batch(
                            ctx,
                            &mut pass,
                            &group,
                            &batch_mdi,
                            key,
                            occlusion_active,
                            &mut last_pipeline,
                            &mut globals_bound,
                            &mut last_g1,
                            &mut last_g2,
                            &mut stats,
                        );
                    }
                    continue;
                }
                self.draw_static_single(
                    ctx,
                    &mut pass,
                    i,
                    &mut last_pipeline,
                    &mut globals_bound,
                    &mut last_g1,
                    &mut last_g2,
                    &mut stats,
                );
            }
        }

        drop(pass);
        self.mesh_batch_order = draw_order;
        self.mesh_batch_groups = batch_groups;
        self.mesh_batch_for_index = batch_for_index;
        crate::bind_log::end_pass();
    }

    pub(in crate::renderer) fn static_batch_plan(
        &mut self,
        ctx: &FrameCtx<'_>,
        occlusion_active: bool,
    ) -> (
        Vec<usize>,
        HashMap<MeshBatchKey, Vec<usize>>,
        Vec<Option<MeshBatchKey>>,
    ) {
        render_scope!("static_batch_plan");
        let meshes = MeshDrawListView::new(ctx.snapshot, ctx.geometry);
        let len = meshes.len();
        let visible: Vec<bool> = if occlusion_active {
            (0..len)
                .map(|i| {
                    let md = meshes.get(i);
                    self.static_draw_visible(true, i, md)
                })
                .collect()
        } else {
            Vec::new()
        };
        let batching = self.mesh_draw_batching;
        run_static_batch_plan(
            &mut self.mesh_batch_opaque_count,
            &mut self.mesh_batch_order,
            &mut self.mesh_batch_groups,
            &mut self.mesh_batch_for_index,
            &mut self.mesh_batch_pairs,
            len,
            |i| meshes.get(i),
            batching,
            |i, _| visible.is_empty() || visible[i],
        )
    }

    pub(in crate::renderer) fn build_static_mdi(
        &mut self,
        ctx: &FrameCtx<'_>,
        batch_groups: &HashMap<MeshBatchKey, Vec<usize>>,
        occlusion_active: bool,
    ) -> HashMap<MeshBatchKey, Vec<MeshMdiSegment>> {
        render_scope!("build_static_mdi");
        let mut out = HashMap::new();
        if !(self.mesh_draw_batching
            && self.mesh_batch_mdi
            && self.gpu.multi_draw_indirect
            && self.gpu.indirect_first_instance)
        {
            return out;
        }
        let meshes = MeshDrawListView::new(ctx.snapshot, ctx.geometry);
        let stride = std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>() as u64;
        let camera_pos = ctx.snapshot.camera_pos;
        self.mesh_indirect_cmd_scratch.clear();
        for (key, group) in batch_groups {
            if group.len() < 2 {
                continue;
            }
            if !self.mdi_shader_supports(ctx, key.shader_id) {
                continue;
            }
            let vtx = ctx
                .shaders
                .get(key.shader_id)
                .expect("mdi shader")
                .mesh_vertex_layout();
            self.mdi_by_buffer.clear();
            for &gi in group {
                let md = meshes.get(gi);
                if !self.static_draw_visible(occlusion_active, gi, md) {
                    continue;
                }
                if !cardinal_face_visible(md, camera_pos) {
                    continue;
                }
                let handle = self.static_mdi_handle(gi, &md.mesh, vtx);
                self.mdi_by_buffer.push((handle.buffer_id, gi));
            }
            if self.mdi_by_buffer.is_empty() {
                continue;
            }
            self.mdi_by_buffer.sort_unstable_by_key(|entry| entry.0);
            let mut run_start = 0;
            while run_start < self.mdi_by_buffer.len() {
                let buffer_id = self.mdi_by_buffer[run_start].0;
                let mut run_end = run_start + 1;
                while run_end < self.mdi_by_buffer.len()
                    && self.mdi_by_buffer[run_end].0 == buffer_id
                {
                    run_end += 1;
                }
                let scratch_start = self.mesh_indirect_cmd_scratch.len();
                let mut seg_indices = 0u32;
                let run_len = run_end - run_start;
                for k in 0..run_len {
                    let gi = self.mdi_by_buffer[run_start + k].1;
                    let md = meshes.get(gi);
                    let handle = self.static_mdi_handle(gi, &md.mesh, vtx);
                    seg_indices +=
                        push_mesh_indirect_cmd(&mut self.mesh_indirect_cmd_scratch, gi, md, handle);
                }
                let cmd_count = self.mesh_indirect_cmd_scratch.len() - scratch_start;
                if cmd_count > 0 {
                    out.entry(*key).or_default().push(MeshMdiSegment {
                        buffer_id,
                        byte_offset: scratch_start as u64 * stride,
                        cmd_count: cmd_count as u32,
                        index_count: seg_indices,
                    });
                }
                run_start = run_end;
            }
        }
        if !self.mesh_indirect_cmd_scratch.is_empty() {
            self.ensure_mesh_indirect_capacity(self.mesh_indirect_cmd_scratch.len());
            self.gpu.queue.write_buffer(
                &self.mesh_indirect_bufs[self.frame_uniform_ring as usize],
                0,
                bytemuck::cast_slice(&self.mesh_indirect_cmd_scratch),
            );
        }
        out
    }

    pub(in crate::renderer) fn draw_static_batch(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        pass: &mut wgpu::RenderPass<'_>,
        group: &[usize],
        batch_mdi: &HashMap<MeshBatchKey, Vec<MeshMdiSegment>>,
        key: MeshBatchKey,
        occlusion_active: bool,
        last_pipeline: &mut Option<MeshPipelineKey>,
        globals_bound: &mut bool,
        last_g1: &mut Option<u32>,
        last_g2: &mut Option<(u32, u32, u32)>,
        stats: &mut StaticDrawStats<'_>,
    ) {
        let meshes = MeshDrawListView::new(ctx.snapshot, ctx.geometry);
        let has_bsp = self.mesh_handle_has_bsp(&meshes.get(group[0]).mesh);
        self.bind_static_state(
            ctx,
            pass,
            key.shader_id,
            key.texture,
            key.material_id,
            key.normal,
            key.orm,
            last_pipeline,
            globals_bound,
            last_g1,
            last_g2,
            false,
        );

        if let Some(segments) = batch_mdi.get(&key) {
            let mut cmds = 0u32;
            let mut tris = 0u64;
            for seg in segments {
                ctx.draw_acc.note_vertex_buffer();
                pass.set_vertex_buffer(0, self.mesh_pool.vertex_buffer(seg.buffer_id).slice(..));
                ctx.draw_acc.note_index_buffer();
                pass.set_index_buffer(
                    self.mesh_pool.index_buffer(seg.buffer_id).slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                ctx.draw_acc.note_indirect();
                pass.multi_draw_indexed_indirect(
                    &self.mesh_indirect_bufs[self.frame_uniform_ring as usize],
                    seg.byte_offset,
                    seg.cmd_count,
                );
                ctx.draw_acc.note_draw_indexed(seg.index_count);
                cmds += seg.cmd_count;
                tris += seg.index_count as u64;
                ctx.prof.mesh_index_count += seg.index_count as u64;
                ctx.prof.mesh_stats.indirect_batches += 1;
                ctx.prof.mesh_stats.indirect_commands += seg.cmd_count;
            }
            if cmds > 0 {
                stats.record(
                    ctx,
                    key.shader_id,
                    false,
                    key.texture,
                    0,
                    key.material_id,
                    has_bsp,
                    cmds,
                    tris,
                    false,
                );
                return;
            }
        }

        let vtx = ctx
            .shaders
            .get(key.shader_id)
            .expect("mesh batch shader")
            .mesh_vertex_layout();
        let mut draws = 0u32;
        let mut tris = 0u64;
        for &gi in group {
            let md = meshes.get(gi);
            if !self.static_draw_visible(occlusion_active, gi, md) {
                continue;
            }
            if !cardinal_face_visible(md, ctx.snapshot.camera_pos) {
                continue;
            }
            let handle = self.mesh_static_handle_at(gi, &md.mesh, vtx);
            let count = mesh_index_span(md.first_index, md.index_count, handle.idx_count);
            if count == 0 {
                continue;
            }
            self.draw_mesh_range(ctx, pass, handle, md.first_index, count, gi as u32);
            draws += 1;
            tris += count as u64;
        }
        stats.record(
            ctx,
            key.shader_id,
            false,
            key.texture,
            0,
            key.material_id,
            has_bsp,
            draws,
            tris,
            true,
        );
    }

    pub(in crate::renderer) fn draw_static_single(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        pass: &mut wgpu::RenderPass<'_>,
        index: usize,
        last_pipeline: &mut Option<MeshPipelineKey>,
        globals_bound: &mut bool,
        last_g1: &mut Option<u32>,
        last_g2: &mut Option<(u32, u32, u32)>,
        stats: &mut StaticDrawStats<'_>,
    ) {
        let md = MeshDrawListView::new(ctx.snapshot, ctx.geometry)
            .get(index)
            .clone();
        if !cardinal_face_visible(&md, ctx.snapshot.camera_pos) {
            return;
        }
        assert!(
            md.shader_id != 0,
            "mesh draw {index}: shader_id=0 (material_id={})",
            md.material_id
        );
        let vtx = ctx
            .shaders
            .get(md.shader_id)
            .expect("mesh draw shader")
            .mesh_vertex_layout();
        let handle = self.mesh_static_handle_at(index, &md.mesh, vtx);
        let has_bsp = self.mesh_handle_has_bsp(&md.mesh);
        self.bind_static_state(
            ctx,
            pass,
            md.shader_id,
            md.texture.0,
            md.material_id,
            md.normal.0,
            md.orm.0,
            last_pipeline,
            globals_bound,
            last_g1,
            last_g2,
            true,
        );
        let count = mesh_index_span(md.first_index, md.index_count, handle.idx_count);
        if count == 0 {
            return;
        }
        self.draw_mesh_range(ctx, pass, handle, md.first_index, count, index as u32);
        stats.record(
            ctx,
            md.shader_id,
            false,
            md.texture.0,
            0,
            md.material_id,
            has_bsp,
            1,
            count as u64,
            false,
        );
    }

    pub(in crate::renderer) fn bind_static_state(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        pass: &mut wgpu::RenderPass<'_>,
        shader_id: u32,
        texture: u32,
        material_id: u32,
        normal: u32,
        orm: u32,
        last_pipeline: &mut Option<MeshPipelineKey>,
        globals_bound: &mut bool,
        last_g1: &mut Option<u32>,
        last_g2: &mut Option<(u32, u32, u32)>,
        skip_if_same: bool,
    ) {
        let pipe_key = static_mesh_pipeline_key(ctx.shaders, shader_id);
        if *last_pipeline != Some(pipe_key) {
            let pipeline =
                self.ensure_shader_mesh_pipeline(ctx.shaders, shader_id, ctx.hdr_format, false);
            ctx.draw_acc.note_pipeline();
            pass.set_pipeline(pipeline);
            *last_pipeline = Some(pipe_key);
        }
        if !*globals_bound {
            ctx.draw_acc.note_bind();
            pass.set_bind_group(
                0,
                &self.mesh_globals_bgs[self.frame_uniform_ring as usize],
                &[],
            );
            crate::bind_log::group(0, "mesh_globals");
            *globals_bound = true;
        }
        if !skip_if_same || *last_g1 != Some(texture) {
            ctx.draw_acc.note_bind();
            pass.set_bind_group(1, self.mesh_bind_group(MeshTexture::slot(texture)), &[]);
            crate::bind_log::group(1, &format!("shader={shader_id} tex={texture}"));
            *last_g1 = Some(texture);
        }
        let g2 = (material_id, normal, orm);
        if !skip_if_same || *last_g2 != Some(g2) {
            ctx.draw_acc.note_bind();
            pass.set_bind_group(
                2,
                self.ensure_material_bg(
                    material_id,
                    MeshTexture::slot(normal),
                    MeshTexture::slot(orm),
                ),
                &[],
            );
            crate::bind_log::group(2, &format!("mat={material_id} n={normal} orm={orm}"));
            *last_g2 = Some(g2);
        }
    }

    pub(in crate::renderer) fn draw_mesh_range(
        &self,
        ctx: &mut FrameCtx<'_>,
        pass: &mut wgpu::RenderPass<'_>,
        handle: crate::mesh_allocator::MeshHandle,
        first_index: u32,
        count: u32,
        instance: u32,
    ) {
        ctx.draw_acc.note_vertex_buffer();
        pass.set_vertex_buffer(
            0,
            self.mesh_pool
                .vertex_buffer(handle.buffer_id)
                .slice(self.mesh_pool.vertex_byte_range(handle)),
        );
        ctx.draw_acc.note_index_buffer();
        pass.set_index_buffer(
            self.mesh_pool
                .index_buffer(handle.buffer_id)
                .slice(self.mesh_pool.index_byte_range(handle)),
            wgpu::IndexFormat::Uint32,
        );
        pass.draw_indexed(first_index..first_index + count, 0, instance..instance + 1);
        ctx.draw_acc.note_draw_indexed(count);
        ctx.prof.mesh_index_count += count as u64;
    }

    /// Skinned draws in their own pass so group 0 can be the bone palette.
    pub(in crate::renderer) fn encode_skinned_mesh_pass(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        encoder: &mut wgpu::CommandEncoder,
        depth_view: &wgpu::TextureView,
        depth_load: wgpu::LoadOp<f32>,
        occlusion_active: bool,
        static_count: usize,
    ) {
        ctx.draw_acc.note_pass();
        crate::bind_log::begin_pass();
        let mut pass = begin_geometry_pass(
            encoder,
            "skinned mesh pass",
            ctx.hdr_view,
            depth_view,
            depth_load,
        );
        ctx.draw_acc.note_bind();
        pass.set_bind_group(3, &self.lights_bgs[self.frame_uniform_ring as usize], &[]);
        crate::bind_log::group(3, "lights");

        // One view for the whole pass; draws are read by reference, not cloned.
        let skinned = SkinnedDrawListView::new(ctx.snapshot, ctx.geometry);
        let count = skinned.len();
        let mut last_shader: Option<ShaderId> = None;
        for j in 0..count {
            let sd = skinned.get(j);
            if !self.skinned_draw_visible(occlusion_active, j, sd) {
                continue;
            }
            if !self.draw_transparent
                && ctx
                    .shaders
                    .get(sd.shader_id)
                    .is_some_and(|d| d.mesh_transparent())
            {
                continue;
            }
            if last_shader != Some(sd.shader_id) {
                let pipeline =
                    self.ensure_skinned_draw_pipeline(ctx.shaders, sd.shader_id, ctx.hdr_format);
                ctx.draw_acc.note_pipeline();
                pass.set_pipeline(&pipeline);
                last_shader = Some(sd.shader_id);
            }
            let vtx = ctx
                .shaders
                .get(sd.shader_id)
                .expect("skinned mesh shader")
                .mesh_vertex_layout();
            let handle = self.mesh_skinned_handle_at(j, &sd.mesh, vtx);
            if handle.idx_count == 0 {
                continue;
            }
            let bone_bg = &self
                .skinned_mesh_globals_bgs
                .get(&sd.bone_buffer)
                .expect("bone palette missing — upload bone_palettes before draw")
                [self.frame_uniform_ring as usize];
            ctx.draw_acc.note_bind();
            pass.set_bind_group(0, bone_bg, &[]);
            ctx.draw_acc.note_bind();
            pass.set_bind_group(1, self.mesh_bind_group(sd.texture), &[]);
            ctx.draw_acc.note_bind();
            pass.set_bind_group(
                2,
                self.ensure_material_bg(sd.material_id, sd.normal, sd.orm),
                &[],
            );
            self.draw_mesh_range(
                ctx,
                &mut pass,
                handle,
                0,
                handle.idx_count,
                (static_count + j) as u32,
            );
            ctx.prof.mesh_stats.tris_drawn += handle.idx_count as u64;
            ctx.prof.mesh_stats.draws_submitted += 1;
            ctx.prof.mesh_stats.albedo_2d_layout_draws += 1;
        }
        crate::bind_log::end_pass();
    }
}

impl Renderer {
    pub(in crate::renderer) fn skinned_mesh_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
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

    pub(in crate::renderer) fn ensure_skinned_mesh_pipeline(
        &mut self,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        if !self.skinned_mesh_pipelines.contains_key(&format) {
            assert!(
                !self.shaders.mesh_skinned.is_empty(),
                "ShaderSources.mesh_skinned missing (load mesh_skinned.wgsl)"
            );
            let shader = self
                .gpu
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("mesh_skinned"),
                    source: wgpu::ShaderSource::Wgsl(self.shaders.mesh_skinned.clone().into()),
                });
            let p = self
                .gpu
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

    pub(in crate::renderer) fn ensure_skinned_bone_bind_group(
        &mut self,
        handle: u32,
        palette: &[[[f32; 4]; 4]],
    ) -> &wgpu::BindGroup {
        if !self.bone_gpu_bufs.contains_key(&handle) {
            let buf = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bone palette"),
                size: GPU_BONE_BLOCK_SIZE,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.bone_gpu_bufs.insert(handle, buf);
        }
        if !self.skinned_mesh_globals_bgs.contains_key(&handle) {
            let bone_buf = self.bone_gpu_bufs.get(&handle).expect("bone palette buf");
            let mut bgs = Vec::with_capacity(FRAME_UNIFORM_RING);
            for slot in 0..FRAME_UNIFORM_RING {
                bgs.push(
                    self.gpu
                        .device
                        .create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("skinned mesh globals bg"),
                            layout: &self.skinned_mesh_globals_bg_layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                        buffer: &self.mesh_globals_bufs[slot],
                                        offset: 0,
                                        size: None,
                                    }),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: bone_buf.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: self.mesh_view_bufs[slot].as_entire_binding(),
                                },
                            ],
                        }),
                );
            }
            let bgs: [wgpu::BindGroup; FRAME_UNIFORM_RING] =
                bgs.try_into().expect("skinned globals ring");
            self.skinned_mesh_globals_bgs.insert(handle, bgs);
        }
        let block = GpuBoneBlock::from_palette(palette);
        let buf = self.bone_gpu_bufs.get(&handle).unwrap();
        self.gpu
            .queue
            .write_buffer(buf, 0, bytemuck::bytes_of(&block));
        &self.skinned_mesh_globals_bgs.get(&handle).unwrap()[self.frame_uniform_ring as usize]
    }

    pub(in crate::renderer) fn build_shader_mesh_pipeline(
        &self,
        wgsl: &str,
        format: wgpu::TextureFormat,
        vertex_layout: &VertexLayoutSpec,
        skinned_vertex: bool,
        transparent: bool,
        additive: bool,
    ) -> wgpu::RenderPipeline {
        let shader = self
            .gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("registry mesh shader"),
                source: wgpu::ShaderSource::Wgsl(wgsl.into()),
            });
        let wgpu_attributes: Vec<wgpu::VertexAttribute> = vertex_layout
            .attrs
            .iter()
            .map(|a| wgpu::VertexAttribute {
                offset: a.offset as u64,
                format: mesh_vertex_pack::wgpu_vertex_format(a.ty),
                shader_location: a.location,
            })
            .collect();
        let vertex_buf_layout = wgpu::VertexBufferLayout {
            array_stride: vertex_layout.stride as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu_attributes,
        };
        let layout = if skinned_vertex {
            &self.skinned_mesh_pipeline_layout
        } else {
            &self.mesh_pipeline_layout
        };
        self.gpu
            .device
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
                        blend: if transparent {
                            Some(if additive {
                                wgpu::BlendState {
                                    color: wgpu::BlendComponent {
                                        src_factor: wgpu::BlendFactor::One,
                                        dst_factor: wgpu::BlendFactor::One,
                                        operation: wgpu::BlendOperation::Add,
                                    },
                                    alpha: wgpu::BlendComponent {
                                        src_factor: wgpu::BlendFactor::One,
                                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                        operation: wgpu::BlendOperation::Add,
                                    },
                                }
                            } else {
                                wgpu::BlendState::ALPHA_BLENDING
                            })
                        } else {
                            None
                        },
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
                    depth_write_enabled: Some(!transparent),
                    depth_compare: Some(if transparent {
                        wgpu::CompareFunction::LessEqual
                    } else {
                        wgpu::CompareFunction::Less
                    }),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
    }

    pub(in crate::renderer) fn ensure_shader_mesh_pipeline(
        &mut self,
        shaders: &ShaderRegistry,
        shader_id: ShaderId,
        format: wgpu::TextureFormat,
        skinned_layout: bool,
    ) -> &wgpu::RenderPipeline {
        let transparent = shaders
            .get(shader_id)
            .map(|d| d.mesh_transparent())
            .unwrap_or(false);
        let key = (shader_id, format, skinned_layout, transparent);
        if !self.shader_mesh_pipelines.contains_key(&key) {
            let def = shaders
                .get(shader_id)
                .expect("MeshDraw shader_id not in ShaderRegistry");
            let skinned = skinned_layout && def.uses_skinned_mesh_vertex();
            let p = self.build_shader_mesh_pipeline(
                &def.wgsl,
                format,
                def.mesh_vertex_layout(),
                skinned,
                transparent,
                def.additive_blend(),
            );
            self.shader_mesh_pipelines.insert(key, p);
        }
        self.shader_mesh_pipelines.get(&key).unwrap()
    }

    /// Skinned draws: material shader when registered as skinned, else built-in `mesh_skinned.wgsl`.
    pub(in crate::renderer) fn ensure_skinned_draw_pipeline(
        &mut self,
        shaders: &ShaderRegistry,
        shader_id: ShaderId,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        if shader_id != 0 {
            if let Some(def) = shaders.get(shader_id) {
                if def.uses_skinned_mesh_vertex() {
                    return self
                        .ensure_shader_mesh_pipeline(shaders, shader_id, format, true)
                        .clone();
                }
            }
        }
        self.ensure_skinned_mesh_pipeline(format).clone()
    }

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

    /// Renderer-only: drop GPU regions whose CPU [`Mesh::gpu_revision`] moved.
    pub(in crate::renderer) fn free_stale_mesh_uploads(&mut self, content_id: u64, revision: u64) {
        let Some((allocator_id, handles)) = self.mesh_gpu.take_stale_uploads(content_id, revision)
        else {
            return;
        };
        if let Some(alloc) = self.mesh_allocators.get_mut(allocator_id as usize) {
            for handle in handles {
                alloc.free(handle);
            }
        }
    }

    pub fn mesh_release(&mut self, content_id: u64) {
        if let Some((allocator_id, handles)) = self.mesh_gpu.release(content_id) {
            if let Some(alloc) = self.mesh_allocators.get_mut(allocator_id as usize) {
                for handle in handles {
                    alloc.free(handle);
                }
            }
        }
    }

    /// Drop all refcount slots (e.g. client disconnect). Frees GPU regions via
    /// each allocator's `free`.
    pub fn reset_mesh_gpu(&mut self) {
        self.uploaded_mesh_revs.clear();
        self.mdi_draw_gpu.clear();
        self.mdi_shader_ok.clear();
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
    ///
    /// Looks the registry slot up by handle. Vertex bytes are copied only when this
    /// frame actually uploads; a cache hit reads `content_id` and `gpu_revision`.
    pub fn mesh_for_draw(
        &mut self,
        mesh: &deerpunk_engine_common::resource::mesh_handle::MeshHandle,
        vertex_layout: &VertexLayoutSpec,
    ) -> crate::mesh_allocator::MeshHandle {
        render_scope!("render/mesh_for_draw");

        let (content_id, revision, allocator_id) = self.mesh_upload_key(*mesh);
        assert_ne!(content_id, 0, "Mesh.content_id unset");
        assert!(
            self.mesh_gpu.is_retained(content_id),
            "mesh content_id={content_id} not retained (entity lifecycle)"
        );
        let layout_key = vertex_layout.layout_key();
        self.free_stale_mesh_uploads(content_id, revision);
        if self.mesh_gpu.needs_upload(content_id, layout_key, revision) {
            let (vertices, indices) = self.copy_mesh_upload(*mesh, vertex_layout);
            let handle =
                self.allocate_and_upload(allocator_id, &vertices, &indices, vertex_layout.stride);
            self.record_mesh_asset_upload(handle, content_id);
            self.mesh_gpu
                .set_uploaded(content_id, layout_key, handle, revision);
            self.frame_mesh_gpu_uploads += 1;
        } else {
            self.frame_mesh_gpu_cached += 1;
        }
        let revision_key = self.mesh_revision(*mesh);
        self.note_mesh_revision_uploaded(*mesh, revision_key);
        self.mesh_gpu
            .handle(content_id, layout_key)
            .expect("mesh upload missing after needs_upload")
    }

    /// GPU handle prepared by [`Self::mesh_for_draw`]. Does not upload.
    pub(crate) fn mesh_resolve_for_draw(
        &self,
        mesh: &deerpunk_engine_common::resource::mesh_handle::MeshHandle,
        vertex_layout: &VertexLayoutSpec,
    ) -> crate::mesh_allocator::MeshHandle {
        let (content_id, revision, _) = self.mesh_upload_key(*mesh);
        let layout_key = vertex_layout.layout_key();
        assert!(
            !self.mesh_gpu.needs_upload(content_id, layout_key, revision),
            "mesh content_id={content_id} rev {revision} was not uploaded in prepare"
        );
        self.mesh_gpu
            .handle(content_id, layout_key)
            .expect("mesh upload missing after prepare")
    }

    /// `mesh_revisions[idx]` differs from the revision last sent to the GPU.
    /// Does not touch the mesh allocation or the GPU-slot map.
    pub(in crate::renderer) fn mesh_needs_gpu_upload(
        &self,
        mesh: deerpunk_engine_common::resource::mesh_handle::MeshHandle,
    ) -> bool {
        render_scope!("render/mesh_needs_gpu_upload");
        let revision = self.mesh_revision(mesh);
        revision != 0 && self.uploaded_mesh_rev(mesh.idx() as usize) != revision
    }

    fn mesh_revision(
        &self,
        handle: deerpunk_engine_common::resource::mesh_handle::MeshHandle,
    ) -> u64 {
        assert!(
            !self.frame_meshes.is_null(),
            "bind_mesh_registry before drawing meshes"
        );
        let reg = unsafe { &*self.frame_meshes };
        reg.mesh_revision(handle)
    }

    fn uploaded_mesh_rev(&self, idx: usize) -> u64 {
        self.uploaded_mesh_revs.get(idx).copied().unwrap_or(0)
    }

    fn note_mesh_revision_uploaded(
        &mut self,
        handle: deerpunk_engine_common::resource::mesh_handle::MeshHandle,
        revision: u64,
    ) {
        let idx = handle.idx() as usize;
        if self.uploaded_mesh_revs.len() <= idx {
            self.uploaded_mesh_revs.resize(idx + 1, 0);
        }
        self.uploaded_mesh_revs[idx] = revision;
    }

    /// `content_id`, `gpu_revision`, `allocator_id`. Does not keep the registry borrow.
    fn mesh_upload_key(
        &self,
        handle: deerpunk_engine_common::resource::mesh_handle::MeshHandle,
    ) -> (u64, u64, u32) {
        let mesh = self.registry_mesh(handle);
        (mesh.content_id(), mesh.gpu_revision, mesh.allocator_id)
    }

    /// Owned vertex and index bytes for one GPU upload.
    fn copy_mesh_upload(
        &self,
        handle: deerpunk_engine_common::resource::mesh_handle::MeshHandle,
        layout: &VertexLayoutSpec,
    ) -> (Vec<u8>, Vec<u32>) {
        let mesh = self.registry_mesh(handle);
        let vertices = crate::mesh_vertex_pack::pack_mesh_vertices(mesh, layout);
        (vertices, mesh.indices.clone())
    }

    fn registry_mesh(
        &self,
        handle: deerpunk_engine_common::resource::mesh_handle::MeshHandle,
    ) -> &deerpunk_engine_common::resource::mesh::Mesh {
        assert!(
            !self.frame_meshes.is_null(),
            "bind_mesh_registry before drawing meshes"
        );
        let reg = unsafe { &*self.frame_meshes };
        reg.get(handle)
            .map(|mesh| mesh.as_ref())
            .unwrap_or_else(|| panic!("mesh {handle:?} missing from registry"))
    }

    /// CPU mesh registry for this frame. Pointer is only read during encode on this thread.
    pub fn bind_mesh_registry(&mut self, meshes: &deerpunk_engine_common::resource::MeshRegistry) {
        self.frame_meshes = meshes;
    }

    fn mdi_shader_supports(&mut self, ctx: &FrameCtx<'_>, shader_id: u32) -> bool {
        if let Some(ok) = self.mdi_shader_ok.get(&shader_id) {
            return *ok;
        }
        let ok = ctx.shaders.get(shader_id).is_some_and(|def| {
            crate::mesh_shader_instance::mesh_shader_supports_draw_slot(&def.wgsl)
        });
        self.mdi_shader_ok.insert(shader_id, ok);
        ok
    }

    /// GPU handle for an MDI draw. Repeat frames reuse it only for the same registry mesh and revision.
    fn static_mdi_handle(
        &mut self,
        index: usize,
        mesh: &deerpunk_engine_common::resource::mesh_handle::MeshHandle,
        vertex_layout: &VertexLayoutSpec,
    ) -> crate::mesh_allocator::MeshHandle {
        let revision = self.mesh_revision(*mesh);
        if let Some((cached_mesh, cached_rev, handle)) = self.mdi_draw_gpu.get(index).copied() {
            if mdi_gpu_cache_hit(cached_mesh, cached_rev, mesh.0, revision) && handle.is_valid() {
                self.mesh_pass_static_handles[index] = Some(handle);
                return handle;
            }
        }
        let handle = self.mesh_static_handle_at(index, mesh, vertex_layout);
        if self.mdi_draw_gpu.len() <= index {
            self.mdi_draw_gpu.resize(
                index + 1,
                (0, 0, crate::mesh_allocator::MeshHandle::INVALID),
            );
        }
        self.mdi_draw_gpu[index] = (mesh.0, revision, handle);
        handle
    }

    pub(crate) fn mesh_static_handle_at(
        &mut self,
        i: usize,
        mesh: &deerpunk_engine_common::resource::mesh_handle::MeshHandle,
        vertex_layout: &VertexLayoutSpec,
    ) -> crate::mesh_allocator::MeshHandle {
        if self.mesh_pass_static_handles[i].is_none() {
            self.mesh_pass_static_handles[i] =
                Some(self.mesh_resolve_for_draw(mesh, vertex_layout));
        }
        self.mesh_pass_static_handles[i].expect("mesh_static_handle_at")
    }

    pub(crate) fn mesh_skinned_handle_at(
        &mut self,
        j: usize,
        mesh: &deerpunk_engine_common::resource::mesh_handle::MeshHandle,
        vertex_layout: &VertexLayoutSpec,
    ) -> crate::mesh_allocator::MeshHandle {
        if self.mesh_pass_skinned_handles[j].is_none() {
            self.mesh_pass_skinned_handles[j] =
                Some(self.mesh_resolve_for_draw(mesh, vertex_layout));
        }
        self.mesh_pass_skinned_handles[j].expect("mesh_skinned_handle_at")
    }

    pub(in crate::renderer) fn mesh_handle_has_bsp(
        &self,
        mesh: &deerpunk_engine_common::resource::mesh_handle::MeshHandle,
    ) -> bool {
        Self::mesh_has_bsp_surface_data(self.registry_mesh(*mesh))
    }

    pub(in crate::renderer) fn mesh_uniform_stride() -> u64 {
        let mesh_align = 16u64;
        (std::mem::size_of::<MeshUniform>() as u64).div_ceil(mesh_align) * mesh_align
    }

    /// Grow `mesh_globals_buf` when static+skinned draw count exceeds capacity.
    pub(in crate::renderer) fn ensure_mesh_uniform_slots(&mut self, draw_count: u64) {
        let stride = Self::mesh_uniform_stride();
        let need_slots = draw_count.max(1);
        let cap_slots = self.mesh_globals_bufs[0].size() / stride;
        if need_slots <= cap_slots {
            return;
        }
        let new_slots = need_slots.next_power_of_two().max(MESH_UNIFORM_SLOTS_INIT);
        for i in 0..FRAME_UNIFORM_RING {
            let buf = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mesh globals"),
                size: stride * new_slots,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.mesh_globals_bgs[i] =
                self.gpu
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("mesh globals bg"),
                        layout: &self.mesh_globals_bg_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &buf,
                                    offset: 0,
                                    size: None,
                                }),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: self.mesh_view_bufs[i].as_entire_binding(),
                            },
                        ],
                    });
            self.mesh_globals_bufs[i] = buf;
        }
        // Skinned bind groups embed `mesh_globals_buf`; drop stale entries after resize.
        self.skinned_mesh_globals_bgs.clear();
        eprintln!(
            "[renderer] mesh_globals_buf grew to {} draws ({} bytes); had {} draws",
            new_slots,
            stride * new_slots,
            draw_count
        );
    }

    pub(in crate::renderer) fn ensure_mesh_indirect_capacity(&mut self, command_count: usize) {
        let command_bytes =
            (command_count * std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>()) as u64;
        if command_bytes <= self.mesh_indirect_bufs[0].size() {
            return;
        }
        let size = command_bytes.next_power_of_two();
        for i in 0..FRAME_UNIFORM_RING {
            self.mesh_indirect_bufs[i] = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mesh multi-draw indirect"),
                size,
                usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
    }

    /// Allocate a GPU region for `mesh` via its allocator, growing the pool
    /// and retrying if the allocator is full. Then upload vertex/index bytes.
    pub(in crate::renderer) fn allocate_and_upload(
        &mut self,
        allocator_id: u32,
        vertices: &[u8],
        indices: &[u32],
        stride: u32,
    ) -> crate::mesh_allocator::MeshHandle {
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
            }
            grow_attempts += 1;
            assert!(
                grow_attempts <= 8,
                "mesh pool growth failed (vert_count={vert_count} idx_count={idx_count})"
            );
            let (vert_cap, idx_cap) = self.mesh_pool.slab_grow_caps(vert_count, idx_count, stride);
            let buffer_id = self
                .mesh_pool
                .add(&self.gpu.device, vert_cap, idx_cap, stride);
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
        let ib_bytes =
            (handle.idx_count as usize) * crate::mesh_allocator::MESH_INDEX_SIZE as usize;
        assert_eq!(
            ib_bytes,
            std::mem::size_of_val(indices),
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
        self.gpu
            .queue
            .write_buffer(vb, v_off, &vertices[..vert_bytes]);
        self.gpu.queue.write_buffer(
            ib,
            i_off,
            bytemuck::cast_slice(&indices[..handle.idx_count as usize]),
        );
        handle
    }

    pub(in crate::renderer) fn record_mesh_asset_upload(
        &mut self,
        handle: crate::mesh_allocator::MeshHandle,
        content_id: u64,
    ) {
        let verts = handle.vert_count;
        let indices = handle.idx_count;
        let stride = handle.stride;
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

    /// `r_fullbright` — mesh shader skips lighting when the uniform flag is set.
    pub fn set_fullbright(&mut self, on: bool) {
        self.fullbright = on;
    }

    /// `r_occlusion_cull` — skip mesh draws occluded in the previous frame (GPU queries).
    pub fn set_occlusion_cull(&mut self, on: bool) {
        self.mesh_occlusion.set_enabled(on);
    }

    /// `r_mesh_batch` — static mesh bind-state batching in the mesh pass.
    pub fn set_mesh_draw_batching(&mut self, on: bool) {
        self.mesh_draw_batching = on;
    }

    /// `r_drawtransparent` — skip the alpha-blended mesh queue when off.
    pub fn set_draw_transparent(&mut self, on: bool) {
        self.draw_transparent = on;
    }

    /// `r_mesh_batch_mdi` — use `multi_draw_indexed_indirect` for batch groups (shader must use `instance_index` slot).
    pub fn set_mesh_batch_mdi(&mut self, on: bool) {
        self.mesh_batch_mdi = on;
    }

    pub(in crate::renderer) fn mesh_has_bsp_surface_data(
        mesh: &deerpunk_engine_common::resource::mesh::Mesh,
    ) -> bool {
        !mesh.lightmap_uvs.is_empty() || !mesh.atlas_rects.is_empty()
    }

    pub(in crate::renderer) fn create_sampled_texture_2d(
        &self,
        label: &'static str,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let mips = texture_mip_levels(width, height);
        let tex = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
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
            view_formats: if format == ALBEDO_TEX_FORMAT {
                &[ALBEDO_TEX_FORMAT][..]
            } else {
                &[]
            },
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        (tex, view)
    }

    pub(in crate::renderer) fn upload_rgba_with_mip_chain(
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
            self.gpu.queue.write_texture(
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
        let (tex, view) = self.create_sampled_texture_2d("pbr tex", w, h, ALBEDO_TEX_FORMAT);
        self.upload_rgba_with_mip_chain(&tex, pixels, w, h);
        let slot = self.next_texture_slot;
        self.next_texture_slot += 1;

        // Group 1 bind group for albedo texture (same as mesh_user_bgs pattern)
        let bg = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
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

    pub(in crate::renderer) fn ensure_material_bg(
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
            let bg = self
                .gpu
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
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

    pub(in crate::renderer) fn invalidate_material_bgs(&mut self) {
        for bg in &mut self.material_bgs {
            *bg = None;
        }
    }

    /// Bind-group с текстурой для меша (WHITE → 1x1 белая).
    pub(in crate::renderer) fn mesh_bind_group(&self, texture: MeshTexture) -> &wgpu::BindGroup {
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
        let bg = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
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
        let (tex, view) = self.create_sampled_texture_2d("mesh texture", w, h, ALBEDO_TEX_FORMAT);
        self.upload_rgba_with_mip_chain(&tex, pixels, w, h);
        let bg = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
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
}

#[cfg(test)]
mod batch_tests {
    use super::{mdi_gpu_cache_hit, push_mesh_indirect_cmd};
    use crate::aabb::Aabb;
    use crate::mesh_allocator::MeshHandle as GpuMesh;
    use crate::snapshot::MeshDraw;
    use deerpunk_engine_common::resource::mesh_handle::MeshHandle;
    use deerpunk_engine_common::MeshTexture;

    fn draw(mesh: MeshHandle, first_index: u32, index_count: u32) -> MeshDraw {
        MeshDraw {
            mesh,
            model_mat: [[0.0; 4]; 4],
            layer_mask: 1,
            batchable: true,
            material_id: 3,
            texture: MeshTexture(1),
            normal: MeshTexture(0),
            orm: MeshTexture(0),
            sun_dir: [0.0; 4],
            ambient: [0.0; 4],
            shader_id: 1,
            lightmap_gpu: 0,
            first_index,
            index_count,
            face: 0,
            world_bounds: Aabb::EMPTY,
            entity_id: 0,
            skip_vis: false,
            uniform_aux: 0,
        }
    }

    #[test]
    fn reused_draw_slot_does_not_keep_the_previous_mesh() {
        let revision = 1u64;
        let previous = MeshHandle::from_parts(4, 1).0;
        let current = MeshHandle::from_parts(9, 1).0;
        assert!(
            !mdi_gpu_cache_hit(previous, revision, current, revision),
            "same revision on a different registry mesh must miss"
        );
        assert!(mdi_gpu_cache_hit(current, revision, current, revision));
        assert!(!mdi_gpu_cache_hit(current, revision, current, revision + 1));
        assert!(!mdi_gpu_cache_hit(current, 0, current, 0));
    }

    #[test]
    fn indirect_commands_keep_each_draws_gpu_range() {
        let mut scratch = Vec::new();
        let chunk_a = draw(MeshHandle::from_parts(4, 1), 0, 12);
        let chunk_b = draw(MeshHandle::from_parts(9, 1), 6, 30);
        let gpu_a = GpuMesh {
            buffer_id: 1,
            vert_offset: 100,
            vert_count: 8,
            idx_offset: 200,
            idx_count: 12,
            stride: 32,
        };
        let gpu_b = GpuMesh {
            buffer_id: 1,
            vert_offset: 400,
            vert_count: 20,
            idx_offset: 800,
            idx_count: 90,
            stride: 32,
        };
        push_mesh_indirect_cmd(&mut scratch, 0, &chunk_a, gpu_a);
        push_mesh_indirect_cmd(&mut scratch, 7, &chunk_b, gpu_b);
        assert_eq!(scratch.len(), 2);
        assert_eq!(scratch[0].base_vertex, 100);
        assert_eq!(scratch[0].first_index, 200);
        assert_eq!(scratch[0].index_count, 12);
        assert_eq!(scratch[0].first_instance, 0);
        assert_eq!(scratch[1].base_vertex, 400);
        assert_eq!(scratch[1].first_index, 806);
        assert_eq!(scratch[1].index_count, 30);
        assert_eq!(scratch[1].first_instance, 7);
    }
}
