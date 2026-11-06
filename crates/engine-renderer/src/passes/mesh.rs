//! Mesh + skinned + indirect draw pass.


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

impl Renderer {
    pub(crate) fn pass_mesh(
        &mut self,
        ctx: &mut FrameCtx<'_>, encoder: &mut wgpu::CommandEncoder,
    ) {
    // --- Mesh draws (static + GPU-skinned, single depth pass) ---
    let meshes = MeshDrawListView::new(ctx.snapshot, ctx.geometry);
    let skinned_meshes = SkinnedDrawListView::new(ctx.snapshot, ctx.geometry);
    let static_count = meshes.len();
    let skinned_count = skinned_meshes.len();
    // GPU occlusion is tied to the primary scene depth buffer (convention: `DepthTargetId(0)`).
    let owns_scene_occlusion = ctx.snapshot.depth.target
        == deerpunk_engine_common::components::DepthTargetId(0);
    if static_count > 0 || skinned_count > 0 {
        let mesh_align = 256u64;
        let mesh_uniform_size = ((std::mem::size_of::<MeshUniform>() as u64 + mesh_align - 1)
            / mesh_align)
            * mesh_align;

        let resolved_static: Vec<u32> = (0..static_count)
            .map(|i| meshes.get(i).material_id)
            .collect();
        let resolved_skinned: Vec<u32> = (0..skinned_count)
            .map(|j| skinned_meshes.get(j).material_id)
            .collect();

        let fullbright_flag = if self.fullbright { 1u32 } else { 0u32 };
        let lightmap_flag = if self.lightmap_only { 1u32 } else { 0u32 };
        let draw_count = static_count + skinned_count;
        self.ensure_mesh_uniform_slots(draw_count as u64);
        for palette in &ctx.snapshot.bone_palettes {
            self.ensure_skinned_bone_bind_group(palette.handle, &palette.matrices);
        }
        let slab_bytes = (mesh_uniform_size * draw_count as u64) as usize;
        assert!(
            slab_bytes as u64 <= self.mesh_globals_buf.size(),
            "mesh uniform upload {} bytes > mesh_globals_buf {} ({} draws)",
            slab_bytes,
            self.mesh_globals_buf.size(),
            draw_count
        );
        self.mesh_uniform_scratch.resize(slab_bytes, 0);

        let occlusion_active =
            self.mesh_occlusion.filters_draws() && owns_scene_occlusion;
        let static_visible: Option<Vec<bool>> = if occlusion_active {
            Some(
                (0..static_count)
                    .map(|i| {
                        self.mesh_occlusion.draw_visible_static(
                            i,
                            &meshes.get(i),
                        )
                    })
                    .collect(),
            )
        } else {
            None
        };
        let skinned_visible: Option<Vec<bool>> = if occlusion_active {
            Some(
                (0..skinned_count)
                    .map(|j| {
                        self.mesh_occlusion.draw_visible_skinned(
                            j,
                            &skinned_meshes.get(j),
                        )
                    })
                    .collect(),
            )
        } else {
            None
        };
        let static_vis = |i: usize| static_visible.as_ref().is_none_or(|v| v[i]);
        let skinned_vis = |j: usize| skinned_visible.as_ref().is_none_or(|v| v[j]);

        let mut min_uniform_slot = draw_count as u64;
        let mut max_uniform_slot = 0u64;
        let mut wrote_uniform = false;
        for (i, md) in (0..static_count).map(|i| meshes.get(i)).enumerate() {
            if !static_vis(i) {
                continue;
            }
            wrote_uniform = true;
            min_uniform_slot = min_uniform_slot.min(i as u64);
            max_uniform_slot = max_uniform_slot.max(i as u64);
            let uniform = MeshUniform {
                view_proj: ctx.snapshot.view_proj,
                model: md.model_mat,
                material_data: [
                    resolved_static[i],
                    md.uniform_aux,
                    fullbright_flag,
                    lightmap_flag,
                ],
                camera_pos: ctx.snapshot.camera_pos,
                _pad_cam: 0.0,
                _pad: [0.0; 24],
            };
            let off = (i as u64 * mesh_uniform_size) as usize;
            let uniform_bytes = bytemuck::bytes_of(&uniform);
            self.mesh_uniform_scratch[off..off + uniform_bytes.len()]
                .copy_from_slice(uniform_bytes);
        }
        for (j, sd) in (0..skinned_count).map(|j| skinned_meshes.get(j)).enumerate() {
            if !skinned_vis(j) {
                continue;
            }
            let i = static_count + j;
            wrote_uniform = true;
            min_uniform_slot = min_uniform_slot.min(i as u64);
            max_uniform_slot = max_uniform_slot.max(i as u64);
            let uniform = MeshUniform {
                view_proj: ctx.snapshot.view_proj,
                model: sd.model_mat,
                material_data: [resolved_skinned[j], 0, fullbright_flag, lightmap_flag],
                camera_pos: ctx.snapshot.camera_pos,
                _pad_cam: 0.0,
                _pad: [0.0; 24],
            };
            let off = (i as u64 * mesh_uniform_size) as usize;
            let uniform_bytes = bytemuck::bytes_of(&uniform);
            self.mesh_uniform_scratch[off..off + uniform_bytes.len()]
                .copy_from_slice(uniform_bytes);
        }
        if wrote_uniform {
            let start_byte = (min_uniform_slot * mesh_uniform_size) as usize;
            let end_byte = ((max_uniform_slot + 1) * mesh_uniform_size) as usize;
            self.queue.write_buffer(&self.mesh_globals_buf,
                min_uniform_slot * mesh_uniform_size,
                &self.mesh_uniform_scratch[start_byte..end_byte],
            );
        }
        ctx.prof.mesh_stats.uniforms_ms = lap_ms(ctx.lap);

        self.mesh_pass_static_handles.clear();
        self.mesh_pass_static_handles.resize(static_count, None);
        self.mesh_pass_skinned_handles.clear();
        self.mesh_pass_skinned_handles.resize(skinned_count, None);

        {
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
            // Scene-level lighting (set via set_scene_lighting).
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
            self.queue.write_buffer(&self.lights_buf, 0, bytemuck::bytes_of(&lights));
        }

        let depth_view = self.ensure_depth(
            ctx.snapshot.depth.target,
            frame_width(),
            frame_height(),
        );
        // `DepthLoad::None` falls back to Clear: the mesh pipelines carry a depth-stencil
        // state, so wgpu requires a depth attachment. True "no depth" would need a
        // separate depthless pipeline (not yet warranted — no camera uses None today).
        let depth_load = match ctx.snapshot.depth.load {
            deerpunk_engine_common::components::DepthLoad::Clear(v) => wgpu::LoadOp::Clear(v),
            deerpunk_engine_common::components::DepthLoad::Load => wgpu::LoadOp::Load,
            deerpunk_engine_common::components::DepthLoad::None => wgpu::LoadOp::Clear(1.0),
        };
        let mut shader_bins: std::collections::HashMap<(u32, bool), (u32, u64)> =
            std::collections::HashMap::new();
        let mut sample_set = false;

        self.mesh_draw_order.clear();
        self.mesh_draw_order.extend(0..meshes.len());
        let camera_pos = ctx.snapshot.camera_pos;
        self.mesh_draw_order.sort_unstable_by(|&a, &b| {
            let ma = meshes.get(a);
            let mb = meshes.get(b);
            mesh_draw_sort_cmp(ctx.shaders, ma, mb, camera_pos)
                .then_with(|| ma.lightmap_gpu.cmp(&mb.lightmap_gpu))
        });

        let draw_order = std::mem::take(&mut self.mesh_draw_order);
        let mut batch_groups: HashMap<MeshBatchKey, Vec<usize>> = HashMap::new();
        let mut batch_for_index = vec![None; meshes.len()];
        for &i in &draw_order {
            let md = &meshes.get(i);
            if !self.mesh_draw_batching || !md.batchable || !static_vis(i) {
                continue;
            }
            if mesh_draw_is_transparent(ctx.shaders, md) {
                continue;
            }
            if ctx
                .shaders
                .get(md.shader_id)
                .is_some_and(|d| d.uses_skinned_mesh_vertex())
            {
                continue;
            }
            let vtx = ctx
                .shaders
                .get(md.shader_id)
                .expect("mesh batch shader")
                .mesh_vertex_layout();
            self.mesh_static_handle_at(i, &md.mesh, vtx);
            let lm_surface = ctx.shaders
                .get(md.shader_id)
                .map(|d| d.group1_uses_lightmap_array())
                .unwrap_or(false);
            let key = MeshBatchKey {
                shader_id: md.shader_id,
                lm_surface,
                material_id: resolved_static[i],
                texture: md.texture.0,
                normal: md.normal.0,
                orm: md.orm.0,
                lightmap_gpu: if lm_surface { md.lightmap_gpu } else { 0 },
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

        let indirect_command_stride =
            std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>() as u64;
        let mut batch_mdi: HashMap<MeshBatchKey, Vec<MeshMdiSegment>> = HashMap::new();
        let mdi_ok = self.mesh_draw_batching
            && self.mesh_batch_mdi
            && self.multi_draw_indirect_supported
            && self.indirect_first_instance_supported;
        if mdi_ok {
            self.mesh_indirect_cmd_scratch.clear();
            for (key, group) in &batch_groups {
                if group.len() < 2 {
                    continue;
                }
                let shader_ok = ctx
                    .shaders
                    .get(key.shader_id)
                    .is_some_and(|d| crate::mesh_shader_instance::mesh_shader_supports_draw_slot(&d.wgsl));
                if !shader_ok {
                    continue;
                }
                let vtx = ctx
                    .shaders
                    .get(key.shader_id)
                    .expect("mesh batch shader")
                    .mesh_vertex_layout();
                let mut by_buffer: HashMap<u32, Vec<usize>> = HashMap::new();
                for &gi in group {
                    if !static_vis(gi) {
                        continue;
                    }
                    let md = &meshes.get(gi);
                    let handle = self.mesh_static_handle_at(gi, &md.mesh, vtx);
                    by_buffer.entry(handle.buffer_id).or_default().push(gi);
                }
                for (buffer_id, indices) in by_buffer {
                    if indices.is_empty() {
                        continue;
                    }
                    let scratch_start = self.mesh_indirect_cmd_scratch.len();
                    let mut seg_indices = 0u32;
                    for &gi in &indices {
                        let md = &meshes.get(gi);
                        let handle = self.mesh_static_handle_at(gi, &md.mesh, vtx);
                        seg_indices = seg_indices.saturating_add(push_mesh_indirect_cmd(
                            &mut self.mesh_indirect_cmd_scratch,
                            gi,
                            md,
                            handle,
                        ));
                    }
                    let cmd_count = self.mesh_indirect_cmd_scratch.len() - scratch_start;
                    if cmd_count == 0 {
                        continue;
                    }
                    batch_mdi.entry(*key).or_default().push(MeshMdiSegment {
                        buffer_id,
                        byte_offset: scratch_start as u64 * indirect_command_stride,
                        cmd_count: cmd_count as u32,
                        index_count: seg_indices,
                    });
                }
            }
            if !self.mesh_indirect_cmd_scratch.is_empty() {
                self.ensure_mesh_indirect_capacity(self.mesh_indirect_cmd_scratch.len());
                self.queue.write_buffer(&self.mesh_indirect_buf,
                    0,
                    bytemuck::cast_slice(&self.mesh_indirect_cmd_scratch),
                );
            }
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mesh pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: ctx.hdr_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: depth_load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(3, &self.lights_bg, &[]);

        let mut last_mesh_pipeline: Option<MeshPipelineKey> = None;

        let mut emitted_batches = std::collections::HashSet::new();
        for i in draw_order {
            let md = &meshes.get(i);
            if !static_vis(i) {
                continue;
            }
            if let Some(key) = batch_for_index[i] {
                if emitted_batches.insert(key) {
                    let group = batch_groups
                        .get(&key)
                        .expect("batch key missing from mesh groups");
                    let representative = group[0];
                    let rep = &meshes.get(representative);

                    let pipeline = self.ensure_shader_mesh_pipeline(
                        ctx.shaders,
                        key.shader_id,
                        ctx.hdr_format,
                        false,
                    );
                    let pipe_key = static_mesh_pipeline_key(ctx.shaders, key.shader_id);
                    if last_mesh_pipeline != Some(pipe_key) {
                        ctx.draw_acc.note_pipeline();
                        pass.set_pipeline(pipeline);
                        last_mesh_pipeline = Some(pipe_key);
                    }
                    if key.lm_surface {
                        let bind1 =
                            self.ensure_albedo_lightmap_bg(key.texture, key.lightmap_gpu);
                        pass.set_bind_group(1, &bind1, &[]);
                    } else {
                        pass.set_bind_group(
                            1,
                            self.mesh_bind_group(MeshTexture::slot(key.texture)),
                            &[],
                        );
                    }
                    pass.set_bind_group(
                        2,
                        self.ensure_material_bg(
                            key.material_id,
                            MeshTexture::slot(key.normal),
                            MeshTexture::slot(key.orm),
                        ),
                        &[],
                    );
                    pass.set_bind_group(0, &self.mesh_globals_bg, &[]);

                    let mut batch_done = false;
                    if let Some(segments) = batch_mdi.get(&key) {
                        let mut batch_cmds = 0u32;
                        let mut batch_tris = 0u64;
                        for seg in segments {
                            let vb = self.mesh_pool.vertex_buffer(seg.buffer_id);
                            let ib = self.mesh_pool.index_buffer(seg.buffer_id);
                            pass.set_vertex_buffer(0, vb.slice(..));
                            pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                            pass.multi_draw_indexed_indirect(
                                &self.mesh_indirect_buf,
                                seg.byte_offset,
                                seg.cmd_count,
                            );
                            ctx.draw_acc.note_draw_indexed(seg.index_count);
                            batch_cmds += seg.cmd_count;
                            batch_tris += seg.index_count as u64;
                            ctx.prof.mesh_index_count += seg.index_count as u64;
                            ctx.prof.mesh_stats.indirect_batches += 1;
                            ctx.prof.mesh_stats.indirect_commands += seg.cmd_count;
                        }
                        if batch_cmds > 0 {
                            ctx.prof.mesh_stats.draws_submitted += batch_cmds;
                            ctx.prof.mesh_stats.tris_drawn += batch_tris;
                            if key.lm_surface {
                                ctx.prof.mesh_stats.lm_array_layout_draws += batch_cmds;
                                if key.lightmap_gpu == 0 {
                                    ctx.prof.mesh_stats.lm_array_with_gpu_id_zero += batch_cmds;
                                }
                            } else {
                                ctx.prof.mesh_stats.albedo_2d_layout_draws += batch_cmds;
                            }
                            if key.texture == MeshTexture::WHITE.0 {
                                ctx.prof.mesh_stats.albedo_white_slot_draws += batch_cmds;
                            }
                            let bin = shader_bins
                                .entry((key.shader_id, key.lm_surface))
                                .or_insert((0, 0));
                            bin.0 += batch_cmds;
                            bin.1 += batch_tris;
                            if !sample_set {
                                sample_set = true;
                                ctx.prof.mesh_stats.sample_shader_id = key.shader_id;
                                ctx.prof.mesh_stats.sample_shader_path = ctx
                                    .shaders
                                    .path_for_id(key.shader_id)
                                    .unwrap_or("?")
                                    .to_string();
                                ctx.prof.mesh_stats.sample_lm_array = key.lm_surface;
                                ctx.prof.mesh_stats.sample_albedo_slot = key.texture;
                                ctx.prof.mesh_stats.sample_lightmap_gpu = key.lightmap_gpu;
                                ctx.prof.mesh_stats.sample_material_id = key.material_id;
                                ctx.prof.mesh_stats.sample_has_bsp_mesh =
                                    Self::mesh_has_bsp_surface_data(&rep.mesh);
                            }
                            batch_done = true;
                        }
                    }
                    if !batch_done {
                    let mut batch_draws = 0u32;
                    let mut batch_tris = 0u64;
                    for &group_index in group {
                        if !static_vis(group_index) {
                            continue;
                        }
                        let group_md = &meshes.get(group_index);
                        let vtx = ctx
                            .shaders
                            .get(group_md.shader_id)
                            .expect("mesh batch group shader")
                            .mesh_vertex_layout();
                        let handle =
                            self.mesh_static_handle_at(group_index, &group_md.mesh, vtx);
                        let vb = self.mesh_pool.vertex_buffer(handle.buffer_id);
                        let ib = self.mesh_pool.index_buffer(handle.buffer_id);
                        pass.set_vertex_buffer(
                            0,
                            vb.slice(self.mesh_pool.vertex_byte_range(handle)),
                        );
                        pass.set_index_buffer(
                            ib.slice(self.mesh_pool.index_byte_range(handle)),
                            wgpu::IndexFormat::Uint32,
                        );
                        let start = group_md.first_index;
                        let count = if group_md.index_count == 0 {
                            handle.idx_count
                        } else {
                            group_md
                                .index_count
                                .min(handle.idx_count.saturating_sub(start))
                        };
                        if count == 0 {
                            continue;
                        }
                        pass.draw_indexed(
                            start..start + count,
                            0,
                            group_index as u32..group_index as u32 + 1,
                        );
                        ctx.draw_acc.note_draw_indexed(count);
                        batch_draws += 1;
                        batch_tris += count as u64;
                        ctx.prof.mesh_index_count += count as u64;
                    }
                    if batch_draws > 0 {
                        ctx.prof.mesh_stats.draws_submitted += batch_draws;
                        ctx.prof.mesh_stats.bind_batch_draws += batch_draws;
                        ctx.prof.mesh_stats.tris_drawn += batch_tris;
                        if key.lm_surface {
                            ctx.prof.mesh_stats.lm_array_layout_draws += batch_draws;
                            if key.lightmap_gpu == 0 {
                                ctx.prof.mesh_stats.lm_array_with_gpu_id_zero += batch_draws;
                            }
                        } else {
                            ctx.prof.mesh_stats.albedo_2d_layout_draws += batch_draws;
                        }
                        if key.texture == MeshTexture::WHITE.0 {
                            ctx.prof.mesh_stats.albedo_white_slot_draws += batch_draws;
                        }
                        let bin = shader_bins
                            .entry((key.shader_id, key.lm_surface))
                            .or_insert((0, 0));
                        bin.0 += batch_draws;
                        bin.1 += batch_tris;
                        if !sample_set {
                            sample_set = true;
                            ctx.prof.mesh_stats.sample_shader_id = key.shader_id;
                            ctx.prof.mesh_stats.sample_shader_path = ctx
                                .shaders
                                .path_for_id(key.shader_id)
                                .unwrap_or("?")
                                .to_string();
                            ctx.prof.mesh_stats.sample_lm_array = key.lm_surface;
                            ctx.prof.mesh_stats.sample_albedo_slot = key.texture;
                            ctx.prof.mesh_stats.sample_lightmap_gpu = key.lightmap_gpu;
                            ctx.prof.mesh_stats.sample_material_id = key.material_id;
                            ctx.prof.mesh_stats.sample_has_bsp_mesh =
                                Self::mesh_has_bsp_surface_data(&rep.mesh);
                        }
                    }
                    }
                }
                continue;
            }
            assert!(
                md.shader_id != 0,
                "mesh draw {i}: shader_id=0 (material_id={})",
                md.material_id
            );
            let vtx = ctx
                .shaders
                .get(md.shader_id)
                .expect("mesh draw shader")
                .mesh_vertex_layout();
            let handle = self.mesh_static_handle_at(i, &md.mesh, vtx);
            let material_id = resolved_static[i];
            pass.set_bind_group(0, &self.mesh_globals_bg, &[]);

            let lm_surface = ctx.shaders
                .get(md.shader_id)
                .map(|d| d.group1_uses_lightmap_array())
                .unwrap_or(false);
            let bsp_mesh = Self::mesh_has_bsp_surface_data(&md.mesh);

            let pipeline = self.ensure_shader_mesh_pipeline(
                ctx.shaders,
                md.shader_id,
                ctx.hdr_format,
                false,
            );
            let pipe_key = static_mesh_pipeline_key(ctx.shaders, md.shader_id);
            if last_mesh_pipeline != Some(pipe_key) {
                ctx.draw_acc.note_pipeline();
                pass.set_pipeline(pipeline);
                last_mesh_pipeline = Some(pipe_key);
            }
            if lm_surface {
                let bind1 = self.ensure_albedo_lightmap_bg(md.texture.0, md.lightmap_gpu);
                pass.set_bind_group(1, &bind1, &[]);
            } else {
                pass.set_bind_group(1, self.mesh_bind_group(md.texture), &[]);
            }
            pass.set_bind_group(2, self.ensure_material_bg(material_id, md.normal, md.orm), &[]);
            let vb = self.mesh_pool.vertex_buffer(handle.buffer_id);
            let ib = self.mesh_pool.index_buffer(handle.buffer_id);
            pass.set_vertex_buffer(0, vb.slice(self.mesh_pool.vertex_byte_range(handle)));
            pass.set_index_buffer(
                ib.slice(self.mesh_pool.index_byte_range(handle)),
                wgpu::IndexFormat::Uint32,
            );
            let start = md.first_index;
            let count = if md.index_count == 0 {
                handle.idx_count
            } else {
                md.index_count.min(handle.idx_count.saturating_sub(start))
            };
            if count > 0 {
                ctx.prof.mesh_index_count += count as u64;
                pass.draw_indexed(start..start + count, 0, i as u32..i as u32 + 1);
                ctx.draw_acc.note_draw_indexed(count);

                ctx.prof.mesh_stats.tris_drawn += count as u64;
                ctx.prof.mesh_stats.draws_submitted += 1;
                if lm_surface {
                    ctx.prof.mesh_stats.lm_array_layout_draws += 1;
                    if md.lightmap_gpu == 0 {
                        ctx.prof.mesh_stats.lm_array_with_gpu_id_zero += 1;
                    }
                } else {
                    ctx.prof.mesh_stats.albedo_2d_layout_draws += 1;
                }
                if md.texture.0 == MeshTexture::WHITE.0 {
                    ctx.prof.mesh_stats.albedo_white_slot_draws += 1;
                }
                let bin = shader_bins
                    .entry((md.shader_id, lm_surface))
                    .or_insert((0, 0));
                bin.0 += 1;
                bin.1 += count as u64;

                if !sample_set {
                    sample_set = true;
                    ctx.prof.mesh_stats.sample_shader_id = md.shader_id;
                    ctx.prof.mesh_stats.sample_shader_path =
                        ctx.shaders.path_for_id(md.shader_id).unwrap_or("?").to_string();
                    ctx.prof.mesh_stats.sample_lm_array = lm_surface;
                    ctx.prof.mesh_stats.sample_albedo_slot = md.texture.0;
                    ctx.prof.mesh_stats.sample_lightmap_gpu = md.lightmap_gpu;
                    ctx.prof.mesh_stats.sample_material_id = material_id;
                    ctx.prof.mesh_stats.sample_has_bsp_mesh = bsp_mesh;
                }
            }
        }

        let mut last_skinned_shader: Option<ShaderId> = None;
        for (j, sd) in (0..skinned_count).map(|j| skinned_meshes.get(j)).enumerate() {
            if !skinned_vis(j) {
                continue;
            }
            if last_skinned_shader != Some(sd.shader_id) {
                let pipeline =
                    self.ensure_skinned_draw_pipeline(ctx.shaders, sd.shader_id, ctx.hdr_format);
                ctx.draw_acc.note_pipeline();
                pass.set_pipeline(&pipeline);
                last_skinned_shader = Some(sd.shader_id);
            }
            let vtx = ctx
                .shaders
                .get(sd.shader_id)
                .expect("skinned mesh shader")
                .mesh_vertex_layout();
            let handle = self.mesh_skinned_handle_at(j, &sd.mesh, vtx);
            let material_id = resolved_skinned[j];
            let i = static_count + j;
            let offset = (i as u64 * mesh_uniform_size) as u32;
            let bone_bg = self
                .skinned_mesh_globals_bgs
                .get(&sd.bone_buffer)
                .expect("bone palette missing — upload bone_palettes before draw");
            pass.set_bind_group(0, bone_bg, &[]);
            pass.set_bind_group(1, self.mesh_bind_group(sd.texture), &[]);
            pass.set_bind_group(2, self.ensure_material_bg(material_id, sd.normal, sd.orm), &[]);
            let vb = self.mesh_pool.vertex_buffer(handle.buffer_id);
            let ib = self.mesh_pool.index_buffer(handle.buffer_id);
            pass.set_vertex_buffer(0, vb.slice(self.mesh_pool.vertex_byte_range(handle)));
            pass.set_index_buffer(
                ib.slice(self.mesh_pool.index_byte_range(handle)),
                wgpu::IndexFormat::Uint32,
            );
            let count = handle.idx_count;
            if count > 0 {
                ctx.prof.mesh_index_count += count as u64;
                pass.draw_indexed(0..count, 0, (i as u32)..(i as u32 + 1));
                ctx.draw_acc.note_draw_indexed(count);
                ctx.prof.mesh_stats.tris_drawn += count as u64;
                ctx.prof.mesh_stats.draws_submitted += 1;
                ctx.prof.mesh_stats.albedo_2d_layout_draws += 1;
            }
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
    }
    if static_count > 0 || skinned_count > 0 {
        let depth_view = self.ensure_depth(
            ctx.snapshot.depth.target,
            frame_width(),
            frame_height(),
        );
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
    }
        ctx.prof.mesh_stats.draw_encode_ms = lap_ms(ctx.lap);
    }
}
