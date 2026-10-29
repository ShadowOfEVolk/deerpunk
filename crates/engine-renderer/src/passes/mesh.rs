//! Mesh + skinned + indirect draw pass.

use std::collections::HashMap;

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

/// Pipeline cache key for **static** mesh draws (`mesh_globals_bg`, never skinned group-0 layout).
fn static_mesh_pipeline_key(shaders: &ShaderRegistry, shader_id: u32) -> MeshPipelineKey {
    let Some(def) = shaders.get(shader_id) else {
        return (shader_id, false, false);
    };
    (
        shader_id,
        def.group1_uses_lightmap_array(),
        false,
    )
}

impl Renderer {
    pub(crate) fn pass_mesh(
        &mut self,
        ctx: &mut FrameCtx<'_>, encoder: &mut wgpu::CommandEncoder,
    ) {
    // --- Mesh draws (static + GPU-skinned, single depth pass) ---
    let static_count = ctx.snapshot.mesh_draws.len();
    let skinned_count = ctx.snapshot.skinned_mesh_draws.len();
    if static_count > 0 || skinned_count > 0 {
        let mesh_align = 256u64;
        let mesh_uniform_size = ((std::mem::size_of::<MeshUniform>() as u64 + mesh_align - 1)
            / mesh_align)
            * mesh_align;

        let resolved_static: Vec<u32> = ctx.snapshot
            .mesh_draws
            .iter()
            .map(|md| md.material_id)
            .collect();
        let resolved_skinned: Vec<u32> = ctx.snapshot
            .skinned_mesh_draws
            .iter()
            .map(|sd| sd.material_id)
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

        let occlusion_active = self.mesh_occlusion.filters_draws();
        let static_visible: Option<Vec<bool>> = if occlusion_active {
            Some(
                (0..static_count)
                    .map(|i| {
                        self.mesh_occlusion.draw_visible_static(
                            i,
                            &ctx.snapshot.mesh_draws[i],
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
                            &ctx.snapshot.skinned_mesh_draws[j],
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
        for (i, md) in ctx.snapshot.mesh_draws.iter().enumerate() {
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
            };
            let off = (i as u64 * mesh_uniform_size) as usize;
            let uniform_bytes = bytemuck::bytes_of(&uniform);
            self.mesh_uniform_scratch[off..off + uniform_bytes.len()]
                .copy_from_slice(uniform_bytes);
        }
        for (j, sd) in ctx.snapshot.skinned_mesh_draws.iter().enumerate() {
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
            };
            let off = (i as u64 * mesh_uniform_size) as usize;
            let uniform_bytes = bytemuck::bytes_of(&uniform);
            self.mesh_uniform_scratch[off..off + uniform_bytes.len()]
                .copy_from_slice(uniform_bytes);
        }
        if wrote_uniform {
            let start_byte = (min_uniform_slot * mesh_uniform_size) as usize;
            let end_byte = ((max_uniform_slot + 1) * mesh_uniform_size) as usize;
            self.queue.write_buffer(
                &self.mesh_globals_buf,
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
            self.queue
                .write_buffer(&self.lights_buf, 0, bytemuck::bytes_of(&lights));
        }

        self.ensure_depth(frame_width(), frame_height());

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

        let mut shader_bins: std::collections::HashMap<(u32, bool), (u32, u64)> =
            std::collections::HashMap::new();
        let mut sample_set = false;

        self.mesh_draw_order.clear();
        self.mesh_draw_order.extend(0..ctx.snapshot.mesh_draws.len());
        self.mesh_draw_order.sort_unstable_by_key(|&i| {
            let md = &ctx.snapshot.mesh_draws[i];
            let lm = ctx.shaders
                .get(md.shader_id)
                .map(|d| d.group1_uses_lightmap_array())
                .unwrap_or(false);
            (
                md.shader_id,
                lm as u8,
                md.material_id,
                md.texture.0,
                md.normal.0,
                md.orm.0,
                md.lightmap_gpu,
            )
        });

        let mut last_mesh_pipeline: Option<MeshPipelineKey> = None;

        let draw_order = std::mem::take(&mut self.mesh_draw_order);
        let mut batch_groups: HashMap<MeshBatchKey, Vec<usize>> = HashMap::new();
        let mut batch_for_index = vec![None; ctx.snapshot.mesh_draws.len()];
        for &i in &draw_order {
            let md = &ctx.snapshot.mesh_draws[i];
            if !self.multi_draw_indirect_supported
                || !md.batchable
                || !static_vis(i)
            {
                continue;
            }
            if ctx
                .shaders
                .get(md.shader_id)
                .is_some_and(|d| d.uses_skinned_mesh_vertex())
            {
                continue;
            }
            let handle = self.mesh_static_handle_at(i, &md.mesh);
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
                lightmap_gpu: md.lightmap_gpu,
                buffer_id: handle.buffer_id,
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
        let indirect_command_count = batch_groups
            .values()
            .filter(|group| group.len() >= 2)
            .map(|group| group.len())
            .sum::<usize>();
        if indirect_command_count > 0 {
            self.ensure_mesh_indirect_capacity(indirect_command_count);
        }
        let indirect_command_stride =
            std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>() as u64;
        let mut indirect_offset = 0u64;
        let mut emitted_batches = std::collections::HashSet::new();
        for i in draw_order {
            let md = &ctx.snapshot.mesh_draws[i];
            if !static_vis(i) {
                continue;
            }
            if let Some(key) = batch_for_index[i] {
                if emitted_batches.insert(key) {
                    let group = batch_groups
                        .get(&key)
                        .expect("batch key missing from mesh groups");
                    let representative = group[0];
                    let rep = &ctx.snapshot.mesh_draws[representative];
                    let offset = (representative as u64 * mesh_uniform_size) as u32;
                    pass.set_bind_group(0, &self.mesh_globals_bg, &[offset]);

                    let pipeline = self.ensure_shader_mesh_pipeline(
                        ctx.shaders,
                        key.shader_id,
                        ctx.format,
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
                    let vb = self.mesh_pool.vertex_buffer(key.buffer_id);
                    let ib = self.mesh_pool.index_buffer(key.buffer_id);
                    pass.set_vertex_buffer(0, vb.slice(..));
                    pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);

                    self.mesh_indirect_cmd_scratch.clear();
                    self.mesh_indirect_cmd_scratch.reserve(group.len());
                    let mut total_indices = 0u32;
                    for &group_index in group {
                        let group_md = &ctx.snapshot.mesh_draws[group_index];
                        let group_handle =
                            self.mesh_static_handle_at(group_index, &group_md.mesh);
                        let start = group_md.first_index;
                        let count = if group_md.index_count == 0 {
                            group_handle.idx_count
                        } else {
                            group_md
                                .index_count
                                .min(group_handle.idx_count.saturating_sub(start))
                        };
                        if count == 0 {
                            continue;
                        }
                        total_indices = total_indices.saturating_add(count);
                        self.mesh_indirect_cmd_scratch
                            .push(wgpu::util::DrawIndexedIndirectArgs {
                                index_count: count,
                                instance_count: 1,
                                first_index: group_handle.idx_offset.saturating_add(start),
                                base_vertex: group_handle.vert_offset as i32,
                                first_instance: 0,
                            });
                    }
                    if !self.mesh_indirect_cmd_scratch.is_empty() {
                        let cmd_count = self.mesh_indirect_cmd_scratch.len();
                        self.queue.write_buffer(
                            &self.mesh_indirect_buf,
                            indirect_offset,
                            bytemuck::cast_slice(&self.mesh_indirect_cmd_scratch),
                        );
                        pass.multi_draw_indexed_indirect(
                            &self.mesh_indirect_buf,
                            indirect_offset,
                            cmd_count as u32,
                        );
                        indirect_offset += cmd_count as u64 * indirect_command_stride;
                        ctx.draw_acc.note_draw_indexed(total_indices);
                        ctx.prof.mesh_index_count += total_indices as u64;
                        ctx.prof.mesh_stats.draws_submitted += cmd_count as u32;
                        ctx.prof.mesh_stats.tris_drawn += total_indices as u64;
                        if key.lm_surface {
                            ctx.prof.mesh_stats.lm_array_layout_draws += cmd_count as u32;
                            if key.lightmap_gpu == 0 {
                                ctx.prof.mesh_stats.lm_array_with_gpu_id_zero += cmd_count as u32;
                            }
                        } else {
                            ctx.prof.mesh_stats.albedo_2d_layout_draws += cmd_count as u32;
                        }
                        if key.texture == MeshTexture::WHITE.0 {
                            ctx.prof.mesh_stats.albedo_white_slot_draws += cmd_count as u32;
                        }
                        let bin = shader_bins
                            .entry((key.shader_id, key.lm_surface))
                            .or_insert((0, 0));
                        bin.0 += cmd_count as u32;
                        bin.1 += total_indices as u64;
                        if !sample_set {
                            sample_set = true;
                            ctx.prof.mesh_stats.sample_shader_id = key.shader_id;
                            ctx.prof.mesh_stats.sample_shader_path =
                                ctx.shaders.path_for_id(key.shader_id).unwrap_or("?").to_string();
                            ctx.prof.mesh_stats.sample_lm_array = key.lm_surface;
                            ctx.prof.mesh_stats.sample_albedo_slot = key.texture;
                            ctx.prof.mesh_stats.sample_lightmap_gpu = key.lightmap_gpu;
                            ctx.prof.mesh_stats.sample_material_id = key.material_id;
                            ctx.prof.mesh_stats.sample_has_bsp_mesh = Self::mesh_has_bsp_surface_data(&rep.mesh);
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
            let handle = self.mesh_static_handle_at(i, &md.mesh);
            let material_id = resolved_static[i];
            let offset = (i as u64 * mesh_uniform_size) as u32;
            pass.set_bind_group(0, &self.mesh_globals_bg, &[offset]);

            let lm_surface = ctx.shaders
                .get(md.shader_id)
                .map(|d| d.group1_uses_lightmap_array())
                .unwrap_or(false);
            let bsp_mesh = Self::mesh_has_bsp_surface_data(&md.mesh);

            let pipeline = self.ensure_shader_mesh_pipeline(
                ctx.shaders,
                md.shader_id,
                ctx.format,
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
                pass.draw_indexed(start..start + count, 0, 0..1);
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
        for (j, sd) in ctx.snapshot.skinned_mesh_draws.iter().enumerate() {
            if !skinned_vis(j) {
                continue;
            }
            if last_skinned_shader != Some(sd.shader_id) {
                let pipeline =
                    self.ensure_skinned_draw_pipeline(ctx.shaders, sd.shader_id, ctx.format);
                ctx.draw_acc.note_pipeline();
                pass.set_pipeline(&pipeline);
                last_skinned_shader = Some(sd.shader_id);
            }
            let handle = self.mesh_skinned_handle_at(j, &sd.mesh);
            let material_id = resolved_skinned[j];
            let i = static_count + j;
            let offset = (i as u64 * mesh_uniform_size) as u32;
            let bone_bg = self
                .skinned_mesh_globals_bgs
                .get(&sd.bone_buffer)
                .expect("bone palette missing — upload bone_palettes before draw");
            pass.set_bind_group(0, bone_bg, &[offset]);
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
                pass.draw_indexed(0..count, 0, 0..1);
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
        if let Some(depth_view) = self.depth_view.as_ref() {
            self.frame_occlusion_queries = self.mesh_occlusion.encode_queries(
                &self.device,
                &self.queue,
                encoder,
                ctx.snapshot,
                depth_view,
                ctx.hdr_view,
                ctx.format,
                DEPTH_FORMAT,
            );
        }
    }
        ctx.prof.mesh_stats.draw_encode_ms = lap_ms(ctx.lap);
    }
}
