//! Static mesh pass, then skinned mesh pass.


use std::collections::HashMap;
use std::cmp::Ordering;

use glam::Vec3;
use deerpunk_engine_common::frame::{frame_height, frame_width};
use deerpunk_engine_common::material::ShaderId;
use bytemuck::Zeroable;
use wgpu::util::DeviceExt;

use crate::frame::context::FrameCtx;
use crate::gpu::{
    GpuPointLight, LightsUniform, MeshBatchKey, MeshUniform, Renderer, DEPTH_FORMAT,
    MAX_POINT_LIGHTS,
};
use crate::render_profile::lap_ms;
use crate::shader_registry::ShaderRegistry;
use crate::snapshot::MeshTexture;

type MeshPipelineKey = (u32, bool, bool);

struct MeshMdiSegment {
    buffer_id: u32,
    byte_offset: u64,
    cmd_count: u32,
    index_count: u32,
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
        md.index_count
            .min(handle.idx_count.saturating_sub(start))
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

fn mesh_draw_is_transparent(shaders: &ShaderRegistry, md: &crate::snapshot::MeshDraw) -> bool {
    shaders
        .get(md.shader_id)
        .is_some_and(|d| d.mesh_transparent())
}

fn mesh_draw_material_sort_key(
    shaders: &ShaderRegistry,
    md: &crate::snapshot::MeshDraw,
) -> (u32, u32, u32, u32, u32, u32) {
    let queue = shaders
        .get(md.shader_id)
        .map(|d| d.mesh_queue())
        .unwrap_or(0);
    (
        queue,
        md.shader_id,
        md.material_id,
        md.texture.0,
        md.normal.0,
        md.orm.0,
    )
}

/// Squared distance from camera to draw bounds (center). Used for back-to-front transparent order.
fn mesh_draw_depth_sq(md: &crate::snapshot::MeshDraw, camera_pos: [f32; 3]) -> f32 {
    let cam = Vec3::from_array(camera_pos);
    if md.world_bounds.is_valid() {
        cam.distance_squared(md.world_bounds.center())
    } else {
        let m = md.model_mat;
        let origin = Vec3::new(m[3][0], m[3][1], m[3][2]);
        cam.distance_squared(origin)
    }
}

fn mesh_batch_fingerprint(
    meshes: &MeshDrawListView<'_>,
    shaders: &ShaderRegistry,
    camera: [f32; 3],
) -> u64 {
    let mut h = meshes.len() as u64;
    h ^= camera[0].to_bits() as u64;
    h ^= camera[1].to_bits() as u64;
    h ^= camera[2].to_bits() as u64;
    for i in 0..meshes.len() {
        let md = meshes.get(i);
        let transparent = mesh_draw_is_transparent(shaders, md) as u64;
        h = h.wrapping_mul(31).wrapping_add(md.shader_id as u64);
        h = h.wrapping_mul(31).wrapping_add(md.material_id as u64);
        h = h.wrapping_mul(31).wrapping_add(md.texture.0 as u64);
        h = h.wrapping_mul(31).wrapping_add(md.normal.0 as u64);
        h = h.wrapping_mul(31).wrapping_add(md.orm.0 as u64);
        h = h.wrapping_mul(31).wrapping_add(md.lightmap_gpu as u64);
        h = h.wrapping_mul(31).wrapping_add(md.batchable as u64);
        h = h.wrapping_mul(31).wrapping_add(transparent);
        let m = md.model_mat;
        h ^= m[3][0].to_bits() as u64;
        h ^= m[3][1].to_bits() as u64;
        h ^= m[3][2].to_bits() as u64;
    }
    h
}

fn mesh_draw_sort_cmp(
    shaders: &ShaderRegistry,
    a: &crate::snapshot::MeshDraw,
    b: &crate::snapshot::MeshDraw,
    camera_pos: [f32; 3],
) -> Ordering {
    let ta = mesh_draw_is_transparent(shaders, a);
    let tb = mesh_draw_is_transparent(shaders, b);
    match (ta, tb) {
        (false, true) => Ordering::Less,
        (true, false) => Ordering::Greater,
        (true, true) => {
            let da = mesh_draw_depth_sq(a, camera_pos);
            let db = mesh_draw_depth_sq(b, camera_pos);
            db.partial_cmp(&da)
                .unwrap_or(Ordering::Equal)
                .then_with(|| mesh_draw_material_sort_key(shaders, a).cmp(&mesh_draw_material_sort_key(shaders, b)))
        }
        (false, false) => mesh_draw_material_sort_key(shaders, a).cmp(&mesh_draw_material_sort_key(shaders, b)),
    }
}

use crate::draw_list::{MeshDrawListView, SkinnedDrawListView};

fn mesh_index_span(first_index: u32, index_count: u32, handle_idx_count: u32) -> u32 {
    if index_count == 0 {
        handle_idx_count
    } else {
        index_count.min(handle_idx_count.saturating_sub(first_index))
    }
}

fn snapshot_depth_load(
    load: deerpunk_engine_common::components::DepthLoad,
) -> wgpu::LoadOp<f32> {
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
    fn record(
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
            ctx.prof.mesh_stats.sample_shader_path =
                ctx.shaders.path_for_id(shader_id).unwrap_or("?").to_string();
            ctx.prof.mesh_stats.sample_lm_array = lm_surface;
            ctx.prof.mesh_stats.sample_albedo_slot = texture;
            ctx.prof.mesh_stats.sample_lightmap_gpu = lightmap_gpu;
            ctx.prof.mesh_stats.sample_material_id = material_id;
            ctx.prof.mesh_stats.sample_has_bsp_mesh = has_bsp;
        }
    }
}

impl Renderer {
    pub(crate) fn pass_mesh(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        // 1. Nothing to draw: still close the mesh timer.
        let static_count = MeshDrawListView::new(ctx.snapshot, ctx.geometry).len();
        let skinned_count = SkinnedDrawListView::new(ctx.snapshot, ctx.geometry).len();
        let owns_scene_occlusion = ctx.snapshot.depth.target
            == deerpunk_engine_common::components::DepthTargetId(0);
        if static_count == 0 && skinned_count == 0 {
            ctx.prof.mesh_stats.draw_encode_ms = lap_ms(ctx.lap);
            return;
        }

        // 2. Upload view, lights, and per-draw uniforms once for both passes.
        let occlusion_active = self.mesh_occlusion.filters_draws() && owns_scene_occlusion;
        self.prepare_mesh_frame(ctx, occlusion_active);

        // 3. Depth target. Static pass applies the camera load; skinned loads the result.
        let depth_view = self.ensure_depth(
            ctx.snapshot.depth.target,
            frame_width(),
            frame_height(),
        );
        let depth_load = snapshot_depth_load(ctx.snapshot.depth.load);
        let mut shader_bins: HashMap<(u32, bool), (u32, u64)> = HashMap::new();
        let mut sample_set = false;

        // 4. Static geometry.
        if static_count > 0 {
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
            self.frame_occlusion_queries |= self.mesh_occlusion.encode_queries(
                &self.device,
                &self.queue,
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
    fn prepare_mesh_frame(&mut self, ctx: &mut FrameCtx<'_>, occlusion_active: bool) {
        let meshes = MeshDrawListView::new(ctx.snapshot, ctx.geometry);
        let skinned_meshes = SkinnedDrawListView::new(ctx.snapshot, ctx.geometry);
        let static_count = meshes.len();
        let skinned_count = skinned_meshes.len();
        let mesh_uniform_size = Self::mesh_uniform_stride();
        let draw_count = static_count + skinned_count;
        let fullbright_flag = u32::from(self.fullbright);
        let lightmap_flag = 0u32;

        self.ensure_mesh_uniform_slots(draw_count as u64);
        for palette in &ctx.snapshot.bone_palettes {
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
        self.queue.write_buffer(
            &self.mesh_view_bufs[ring],
            0,
            bytemuck::bytes_of(&crate::gpu::MeshViewUniform {
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
                [md.material_id, md.uniform_aux, fullbright_flag, lightmap_flag],
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
            self.queue.write_buffer(
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
            self.scene_sky_zenith[0],
            self.scene_sky_zenith[1],
            self.scene_sky_zenith[2],
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
        self.queue.write_buffer(&self.lights_bufs[ring], 0, bytemuck::bytes_of(&lights));
    }

    fn write_mesh_uniform_slot(
        &mut self,
        slot: u64,
        stride: u64,
        model: [[f32; 4]; 4],
        material_data: [u32; 4],
    ) {
        let uniform = MeshUniform { model, material_data };
        let off = (slot * stride) as usize;
        let bytes = bytemuck::bytes_of(&uniform);
        self.mesh_uniform_scratch[off..off + bytes.len()].copy_from_slice(bytes);
    }

    fn static_draw_visible(
        &self,
        occlusion_active: bool,
        index: usize,
        md: &crate::snapshot::MeshDraw,
    ) -> bool {
        !occlusion_active || self.mesh_occlusion.draw_visible_static(index, md)
    }

    fn skinned_draw_visible(
        &self,
        occlusion_active: bool,
        index: usize,
        sd: &crate::snapshot::SkinnedMeshDraw,
    ) -> bool {
        !occlusion_active || self.mesh_occlusion.draw_visible_skinned(index, sd)
    }

    /// Opaque then transparent static draws. Lightmap arrays are not bound.
    fn encode_static_mesh_pass(
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
        let mut stats = StaticDrawStats { bins: shader_bins, sample_set };

        for &i in &draw_order {
            let md = meshes.get(i);
            if !self.static_draw_visible(occlusion_active, i, md) {
                continue;
            }
            if !self.draw_transparent && mesh_draw_is_transparent(ctx.shaders, md) {
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

        drop(pass);
        self.mesh_batch_order = draw_order;
        self.mesh_batch_groups = batch_groups;
        self.mesh_batch_for_index = batch_for_index;
        crate::bind_log::end_pass();
    }

    fn static_batch_plan(
        &mut self,
        ctx: &FrameCtx<'_>,
        occlusion_active: bool,
    ) -> (Vec<usize>, HashMap<MeshBatchKey, Vec<usize>>, Vec<Option<MeshBatchKey>>) {
        let meshes = MeshDrawListView::new(ctx.snapshot, ctx.geometry);
        let batch_fp = if occlusion_active {
            0
        } else {
            mesh_batch_fingerprint(&meshes, ctx.shaders, ctx.snapshot.camera_pos)
        };
        if batch_fp != 0
            && batch_fp == self.mesh_batch_fp
            && self.mesh_batch_for_index.len() == meshes.len()
        {
            return (
                std::mem::take(&mut self.mesh_batch_order),
                std::mem::take(&mut self.mesh_batch_groups),
                std::mem::take(&mut self.mesh_batch_for_index),
            );
        }

        self.mesh_draw_order.clear();
        self.mesh_draw_order.extend(0..meshes.len());
        let camera_pos = ctx.snapshot.camera_pos;
        self.mesh_draw_order.sort_unstable_by(|&a, &b| {
            mesh_draw_sort_cmp(ctx.shaders, meshes.get(a), meshes.get(b), camera_pos)
                .then_with(|| meshes.get(a).texture.0.cmp(&meshes.get(b).texture.0))
        });
        let draw_order = std::mem::take(&mut self.mesh_draw_order);
        let mut batch_groups: HashMap<MeshBatchKey, Vec<usize>> = HashMap::new();
        let mut batch_for_index = vec![None; meshes.len()];
        for &i in &draw_order {
            let md = meshes.get(i);
            if !self.mesh_draw_batching
                || !md.batchable
                || !self.static_draw_visible(occlusion_active, i, md)
                || mesh_draw_is_transparent(ctx.shaders, md)
            {
                continue;
            }
            let Some(def) = ctx.shaders.get(md.shader_id) else {
                continue;
            };
            if def.uses_skinned_mesh_vertex() {
                continue;
            }
            self.mesh_static_handle_at(i, &md.mesh, def.mesh_vertex_layout());
            let key = MeshBatchKey {
                shader_id: md.shader_id,
                lm_surface: false,
                material_id: md.material_id,
                texture: md.texture.0,
                normal: md.normal.0,
                orm: md.orm.0,
                lightmap_gpu: 0,
            };
            batch_for_index[i] = Some(key);
            batch_groups.entry(key).or_default().push(i);
        }
        for group in batch_groups.values() {
            if group.len() < 2 {
                for &i in group {
                    batch_for_index[i] = None;
                }
            }
        }
        self.mesh_batch_fp = batch_fp;
        (draw_order, batch_groups, batch_for_index)
    }

    fn build_static_mdi(
        &mut self,
        ctx: &FrameCtx<'_>,
        batch_groups: &HashMap<MeshBatchKey, Vec<usize>>,
        occlusion_active: bool,
    ) -> HashMap<MeshBatchKey, Vec<MeshMdiSegment>> {
        let mut out = HashMap::new();
        if !(self.mesh_draw_batching
            && self.mesh_batch_mdi
            && self.multi_draw_indirect_supported
            && self.indirect_first_instance_supported)
        {
            return out;
        }
        let meshes = MeshDrawListView::new(ctx.snapshot, ctx.geometry);
        let stride = std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>() as u64;
        self.mesh_indirect_cmd_scratch.clear();
        for (key, group) in batch_groups {
            if group.len() < 2 {
                continue;
            }
            let Some(def) = ctx.shaders.get(key.shader_id) else {
                continue;
            };
            if !crate::mesh_shader_instance::mesh_shader_supports_draw_slot(&def.wgsl) {
                continue;
            }
            let vtx = def.mesh_vertex_layout();
            let mut by_buffer: HashMap<u32, Vec<usize>> = HashMap::new();
            for &gi in group {
                let md = meshes.get(gi);
                if !self.static_draw_visible(occlusion_active, gi, md) {
                    continue;
                }
                let handle = self.mesh_static_handle_at(gi, &md.mesh, vtx);
                by_buffer.entry(handle.buffer_id).or_default().push(gi);
            }
            for (buffer_id, indices) in by_buffer {
                let scratch_start = self.mesh_indirect_cmd_scratch.len();
                let mut seg_indices = 0u32;
                for &gi in &indices {
                    let md = meshes.get(gi);
                    let handle = self.mesh_static_handle_at(gi, &md.mesh, vtx);
                    seg_indices += push_mesh_indirect_cmd(
                        &mut self.mesh_indirect_cmd_scratch,
                        gi,
                        md,
                        handle,
                    );
                }
                let cmd_count = self.mesh_indirect_cmd_scratch.len() - scratch_start;
                if cmd_count == 0 {
                    continue;
                }
                out.entry(*key).or_default().push(MeshMdiSegment {
                    buffer_id,
                    byte_offset: scratch_start as u64 * stride,
                    cmd_count: cmd_count as u32,
                    index_count: seg_indices,
                });
            }
        }
        if !self.mesh_indirect_cmd_scratch.is_empty() {
            self.ensure_mesh_indirect_capacity(self.mesh_indirect_cmd_scratch.len());
            self.queue.write_buffer(
                &self.mesh_indirect_bufs[self.frame_uniform_ring as usize],
                0,
                bytemuck::cast_slice(&self.mesh_indirect_cmd_scratch),
            );
        }
        out
    }

    fn draw_static_batch(
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
        self.bind_static_state(ctx, pass, key.shader_id, key.texture, key.material_id, key.normal, key.orm, last_pipeline, globals_bound, last_g1, last_g2, false);

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
                stats.record(ctx, key.shader_id, false, key.texture, 0, key.material_id, has_bsp, cmds, tris, false);
                return;
            }
        }

        let vtx = ctx.shaders.get(key.shader_id).expect("mesh batch shader").mesh_vertex_layout();
        let mut draws = 0u32;
        let mut tris = 0u64;
        for &gi in group {
            let md = meshes.get(gi);
            if !self.static_draw_visible(occlusion_active, gi, md) {
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
        stats.record(ctx, key.shader_id, false, key.texture, 0, key.material_id, has_bsp, draws, tris, true);
    }

    fn draw_static_single(
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
        let md = MeshDrawListView::new(ctx.snapshot, ctx.geometry).get(index).clone();
        assert!(md.shader_id != 0, "mesh draw {index}: shader_id=0 (material_id={})", md.material_id);
        let vtx = ctx.shaders.get(md.shader_id).expect("mesh draw shader").mesh_vertex_layout();
        let handle = self.mesh_static_handle_at(index, &md.mesh, vtx);
        let has_bsp = self.mesh_handle_has_bsp(&md.mesh);
        self.bind_static_state(
            ctx, pass, md.shader_id, md.texture.0, md.material_id, md.normal.0, md.orm.0,
            last_pipeline, globals_bound, last_g1, last_g2, true,
        );
        let count = mesh_index_span(md.first_index, md.index_count, handle.idx_count);
        if count == 0 {
            return;
        }
        self.draw_mesh_range(ctx, pass, handle, md.first_index, count, index as u32);
        stats.record(ctx, md.shader_id, false, md.texture.0, 0, md.material_id, has_bsp, 1, count as u64, false);
    }

    fn bind_static_state(
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
            let pipeline = self.ensure_shader_mesh_pipeline(ctx.shaders, shader_id, ctx.hdr_format, false);
            ctx.draw_acc.note_pipeline();
            pass.set_pipeline(pipeline);
            *last_pipeline = Some(pipe_key);
        }
        if !*globals_bound {
            ctx.draw_acc.note_bind();
            pass.set_bind_group(0, &self.mesh_globals_bgs[self.frame_uniform_ring as usize], &[]);
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
                self.ensure_material_bg(material_id, MeshTexture::slot(normal), MeshTexture::slot(orm)),
                &[],
            );
            crate::bind_log::group(2, &format!("mat={material_id} n={normal} orm={orm}"));
            *last_g2 = Some(g2);
        }
    }

    fn draw_mesh_range(
        &self,
        ctx: &mut FrameCtx<'_>,
        pass: &mut wgpu::RenderPass<'_>,
        handle: crate::mesh_allocator::MeshHandle,
        first_index: u32,
        count: u32,
        instance: u32,
    ) {
        ctx.draw_acc.note_vertex_buffer();
        pass.set_vertex_buffer(0, self.mesh_pool.vertex_buffer(handle.buffer_id).slice(self.mesh_pool.vertex_byte_range(handle)));
        ctx.draw_acc.note_index_buffer();
        pass.set_index_buffer(
            self.mesh_pool.index_buffer(handle.buffer_id).slice(self.mesh_pool.index_byte_range(handle)),
            wgpu::IndexFormat::Uint32,
        );
        pass.draw_indexed(first_index..first_index + count, 0, instance..instance + 1);
        ctx.draw_acc.note_draw_indexed(count);
        ctx.prof.mesh_index_count += count as u64;
    }

    /// Skinned draws in their own pass so group 0 can be the bone palette.
    fn encode_skinned_mesh_pass(
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
        let mut pass = begin_geometry_pass(encoder, "skinned mesh pass", ctx.hdr_view, depth_view, depth_load);
        ctx.draw_acc.note_bind();
        pass.set_bind_group(3, &self.lights_bgs[self.frame_uniform_ring as usize], &[]);
        crate::bind_log::group(3, "lights");

        let count = SkinnedDrawListView::new(ctx.snapshot, ctx.geometry).len();
        let mut last_shader: Option<ShaderId> = None;
        for j in 0..count {
            let sd = SkinnedDrawListView::new(ctx.snapshot, ctx.geometry).get(j).clone();
            if !self.skinned_draw_visible(occlusion_active, j, &sd) {
                continue;
            }
            if !self.draw_transparent && ctx.shaders.get(sd.shader_id).is_some_and(|d| d.mesh_transparent()) {
                continue;
            }
            if last_shader != Some(sd.shader_id) {
                let pipeline = self.ensure_skinned_draw_pipeline(ctx.shaders, sd.shader_id, ctx.hdr_format);
                ctx.draw_acc.note_pipeline();
                pass.set_pipeline(&pipeline);
                last_shader = Some(sd.shader_id);
            }
            let vtx = ctx.shaders.get(sd.shader_id).expect("skinned mesh shader").mesh_vertex_layout();
            let handle = self.mesh_skinned_handle_at(j, &sd.mesh, vtx);
            if handle.idx_count == 0 {
                continue;
            }
            let bone_bg = &self.skinned_mesh_globals_bgs
                .get(&sd.bone_buffer)
                .expect("bone palette missing — upload bone_palettes before draw")
                [self.frame_uniform_ring as usize];
            ctx.draw_acc.note_bind();
            pass.set_bind_group(0, bone_bg, &[]);
            ctx.draw_acc.note_bind();
            pass.set_bind_group(1, self.mesh_bind_group(sd.texture), &[]);
            ctx.draw_acc.note_bind();
            pass.set_bind_group(2, self.ensure_material_bg(sd.material_id, sd.normal, sd.orm), &[]);
            self.draw_mesh_range(ctx, &mut pass, handle, 0, handle.idx_count, (static_count + j) as u32);
            ctx.prof.mesh_stats.tris_drawn += handle.idx_count as u64;
            ctx.prof.mesh_stats.draws_submitted += 1;
            ctx.prof.mesh_stats.albedo_2d_layout_draws += 1;
        }
        crate::bind_log::end_pass();
    }
}
