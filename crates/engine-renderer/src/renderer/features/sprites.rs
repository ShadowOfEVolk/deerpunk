//! Auto-split from `encode_frame` (frame graph pass).

use std::sync::Arc;

use deerpunk_engine_common::frame::{frame_height, frame_width};

use crate::frame::context::FrameCtx;
use crate::render_profile::lap_ms;
use crate::renderer::{
    fit_sprite, Globals, GpuInstance, Renderer, SpriteSlot, SPRITE_ATLAS_SIZE, SPRITE_CELL,
    SPRITE_CELLS, SPRITE_COLS,
};
use crate::snapshot::{Instance, RenderSnapshot, TexSel};

impl Renderer {
    pub(crate) fn pass_sprites_3d(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
    ) {
        if ctx.snapshot.instances.is_empty() {
            let _ = lap_ms(ctx.lap);
            return;
        }
        // --- Pass 1: 3D scene (clear + draw) ---
        let gpu_insts = self.build_instances_from(&ctx.snapshot.instances);
        self.write_buffers(&gpu_insts, ctx.snapshot.view_proj);
        {
            ctx.draw_acc.note_pass();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: ctx.hdr_view,
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
            ctx.draw_acc.note_pipeline();
            pass.set_pipeline(pipeline);
            ctx.draw_acc.note_bind();
            pass.set_bind_group(0, &self.globals_bg, &[]);
            Self::draw_batched(
                &mut pass,
                &ctx.snapshot.instances,
                &self.sprite_bg,
                &self.map_bg,
                &self.font_bg,
                ctx.draw_acc,
            );
        }
        let _ = lap_ms(ctx.lap);
    }
}

impl Renderer {
    pub(in crate::renderer) fn upload_sprites(&mut self, snapshot: &RenderSnapshot) {
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
            self.gpu.queue.write_texture(
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
            self.atlas.insert(ptr, SpriteSlot { uv });
        }
    }

    pub(in crate::renderer) fn clear_atlas(&mut self) {
        self.atlas.clear();
        self.free_cells = (0..SPRITE_CELLS).collect();
    }

    pub(in crate::renderer) fn build_instances_from(
        &self,
        instances: &[Instance],
    ) -> Vec<GpuInstance> {
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

    pub(in crate::renderer) fn write_buffers(
        &mut self,
        instances: &[GpuInstance],
        view_proj: [[f32; 4]; 4],
    ) {
        let globals = Globals {
            screen: [frame_width() as f32, frame_height() as f32],
            exposure: 1.0,
            tonemap_mode: 0.0,
            view_proj,
        };
        self.gpu
            .queue
            .write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));
        self.gpu
            .queue
            .write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(instances));
    }

    pub(in crate::renderer) fn write_hud_buffers(
        &mut self,
        instances: &[GpuInstance],
        view_proj: [[f32; 4]; 4],
    ) {
        let globals = Globals {
            screen: [frame_width() as f32, frame_height() as f32],
            exposure: 1.0,
            tonemap_mode: 0.0,
            view_proj,
        };
        self.gpu
            .queue
            .write_buffer(&self.hud_globals_buf, 0, bytemuck::bytes_of(&globals));
        self.gpu
            .queue
            .write_buffer(&self.hud_instance_buf, 0, bytemuck::cast_slice(instances));
    }

    pub(in crate::renderer) fn build_pipeline(
        &mut self,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let shader = self
            .gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("shader"),
                source: wgpu::ShaderSource::Wgsl(self.shaders.sprite.clone().into()),
            });
        self.gpu
            .device
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
}
