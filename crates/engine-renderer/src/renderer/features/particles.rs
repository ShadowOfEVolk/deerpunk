//! Native instanced particle pass: one static quad, one per-frame instance
//! buffer, one instanced draw per texture range ([`GpuParticle`]).
//!
//! The client sim packs the whole frame into
//! [`RenderSnapshot::particles`](crate::snapshot::RenderSnapshot::particles);
//! this pass uploads it verbatim and draws GPU billboards. No ECS entities,
//! no per-group meshes or materials.

use std::sync::Arc;
use std::sync::LazyLock;

use deerpunk_engine_common::frame::{frame_height, frame_width};
use deerpunk_engine_common::particle_gpu::GPU_PARTICLE_STRIDE;

use super::super::{MeshViewUniform, Renderer, DEPTH_FORMAT};
use crate::frame::context::FrameCtx;
use crate::render_profile::lap_ms;
use crate::snapshot::{MeshTexture, RenderSnapshot};

/// Embedded built-in shader + the dither include it pulls in.
const PARTICLE_SHADER_WGSL: &str = include_str!(
    "../../../../engine-resource/assets/internal/shaders/particles/billboard_instanced.wgsl"
);
const PARTICLE_DITHER_WGSL: &str =
    include_str!("../../../../engine-resource/assets/internal/shaders/inc/dither.wgsl");
const DITHER_INCLUDE_LINE: &str = "#include \"internal://shaders/inc/dither.wgsl\"";

/// Preprocessed built-in particle shader, computed once.
static PARTICLE_SHADER: LazyLock<String> = LazyLock::new(|| {
    let expanded = PARTICLE_SHADER_WGSL.replacen(DITHER_INCLUDE_LINE, PARTICLE_DITHER_WGSL, 1);
    deerpunk_engine_common::material::ShaderPreprocessor::process(&expanded, &[])
        .map(|p| p.wgsl)
        .expect("built-in particle shader must preprocess")
});

/// Static corner quad, vertex step: `corner float2` in [-1, 1] + `uv float2`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct QuadVertex {
    corner: [f32; 2],
    uv: [f32; 2],
}

pub(crate) const QUAD_VERTS: [QuadVertex; 4] = [
    QuadVertex {
        corner: [-1.0, -1.0],
        uv: [0.0, 1.0],
    },
    QuadVertex {
        corner: [1.0, -1.0],
        uv: [1.0, 1.0],
    },
    QuadVertex {
        corner: [-1.0, 1.0],
        uv: [0.0, 0.0],
    },
    QuadVertex {
        corner: [1.0, 1.0],
        uv: [1.0, 0.0],
    },
];

pub(crate) const QUAD_INDICES: [u16; 6] = [0, 1, 2, 2, 1, 3];

/// Instance capacity the renderer starts with; grows ×2 to fit the frame.
pub(crate) const PARTICLE_INSTANCES_INIT: usize = 4096;

impl Renderer {
    pub(crate) fn pass_particles(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let frame = &ctx.snapshot.particles;
        if frame.ranges.is_empty() || frame.instances.is_empty() {
            let _ = lap_ms(ctx.lap);
            return;
        }

        self.upload_particles(ctx.snapshot);
        // Clone (Arc-cheap): releases the `&mut self` borrow for ensure_depth.
        let pipeline = self.ensure_particle_pipeline(ctx.hdr_format).clone();
        let depth_view =
            self.ensure_depth(ctx.snapshot.depth.target, frame_width(), frame_height());

        ctx.draw_acc.note_pass();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("particles pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: ctx.hdr_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            // Depth test on, write off: particles respect the opaque mesh depth.
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
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
        ctx.draw_acc.note_pipeline();
        pass.set_pipeline(&pipeline);
        ctx.draw_acc.note_bind();
        pass.set_bind_group(0, &self.particle_view_bg, &[]);
        ctx.draw_acc.note_vertex_buffer();
        pass.set_vertex_buffer(0, self.particle_quad_vbuf.slice(..));
        // Instance-step vertex data (slot 1): the pipeline requires it on
        // every draw; binding once here covers all ranges.
        ctx.draw_acc.note_vertex_buffer();
        pass.set_vertex_buffer(1, self.particle_instance_buf.slice(..));
        ctx.draw_acc.note_index_buffer();
        pass.set_index_buffer(self.particle_quad_ibuf.slice(..), wgpu::IndexFormat::Uint16);

        let mut bound_uri: Option<Arc<str>> = None;
        for range in &frame.ranges {
            if range.len == 0 {
                continue;
            }
            let Some(bg) = self.particle_texture_bg(&range.tex_uri) else {
                continue;
            };
            if bound_uri.as_ref() != Some(&range.tex_uri) {
                ctx.draw_acc.note_bind();
                pass.set_bind_group(1, &bg, &[]);
                bound_uri = Some(Arc::clone(&range.tex_uri));
            }
            let lo = range.offset.min(frame.instances.len() as u32);
            let hi = range
                .offset
                .saturating_add(range.len)
                .min(frame.instances.len() as u32);
            if hi > lo {
                ctx.draw_acc.note_vertex_buffer();
                // Instance-stepped vertex attributes use the RELATIVE instance
                // index, so point slot 1 at the range start and draw from 0 —
                // an `lo..hi` instance range alone would read the wrong slab.
                let byte_lo = (lo as u64) * GPU_PARTICLE_STRIDE as u64;
                let byte_hi = (hi as u64) * GPU_PARTICLE_STRIDE as u64;
                pass.set_vertex_buffer(1, self.particle_instance_buf.slice(byte_lo..byte_hi));
                ctx.draw_acc.note_draw_indexed(6 * (hi - lo));
                pass.draw_indexed(0..6, 0, 0..(hi - lo));
            }
        }

        let _ = lap_ms(ctx.lap);
    }

    /// One `write_buffer` per frame over the used instance bytes.
    fn upload_particles(&mut self, snapshot: &RenderSnapshot) {
        let instances = &snapshot.particles.instances;
        let needed = instances.len();
        if needed > self.particle_instance_cap {
            // Grow ×2 from the current capacity (shrink-tolerant: never shrinks).
            let mut cap = self.particle_instance_cap.max(PARTICLE_INSTANCES_INIT);
            while cap < needed {
                cap *= 2;
            }
            self.particle_instance_buf = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("particle instances"),
                size: (cap * GPU_PARTICLE_STRIDE as usize) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.particle_instance_cap = cap;
        }
        let view = MeshViewUniform {
            view_proj: snapshot.view_proj,
            camera_pos: snapshot.camera_pos,
            _pad_cam: 0.0,
        };
        self.gpu
            .queue
            .write_buffer(&self.particle_view_buf, 0, bytemuck::bytes_of(&view));
        self.gpu.queue.write_buffer(
            &self.particle_instance_buf,
            0,
            bytemuck::cast_slice(instances),
        );
    }

    /// Bind group for a particle texture URI (cached slot lookup, shared
    /// texture bind groups). Unresolvable textures are remembered as failed
    /// so a bad URI skips its ranges without repeated warnings.
    fn particle_texture_bg(&mut self, uri: &Arc<str>) -> Option<wgpu::BindGroup> {
        if !self.particle_tex_slots.contains_key(uri) {
            let slot = resolve_particle_slot(uri.as_ref());
            if slot.is_none() {
                log::warn!(
                    "[particles] texture '{uri}' unknown to the renderer; skipping its ranges (call Renderer::register_particle_texture after upload)"
                );
            }
            self.particle_tex_slots.insert(Arc::clone(uri), slot);
        }
        self.particle_tex_slots
            .get(uri)
            .and_then(|slot| slot.as_ref())
            .map(|slot| self.mesh_bind_group(*slot).clone())
    }

    fn ensure_particle_pipeline(&mut self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        if !self.particle_pipelines.contains_key(&format) {
            let pipeline = self.build_particle_pipeline(format);
            self.particle_pipelines.insert(format, pipeline);
        }
        self.particle_pipelines.get(&format).unwrap()
    }

    fn build_particle_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let shader = self
            .gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("particle billboard shader"),
                source: wgpu::ShaderSource::Wgsl(PARTICLE_SHADER.as_str().into()),
            });
        let layout = self
            .gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("particle pipeline layout"),
                bind_group_layouts: &[
                    Some(&self.particle_view_bg_layout),
                    Some(&self.tex_bg_layout),
                ],
                immediate_size: 0,
            });
        let quad_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<QuadVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 8,
                    shader_location: 1,
                },
            ],
        };
        // Mirrors `GpuParticle` exactly: center @2 off 0, size @3 off 12,
        // rot @4 off 16, uv_rect @5 off 20, tint unorm8x4 @6 off 36.
        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: GPU_PARTICLE_STRIDE as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 12,
                    shader_location: 3,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 16,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 20,
                    shader_location: 5,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Unorm8x4,
                    offset: 36,
                    shader_location: 6,
                },
            ],
        };
        self.gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("particle pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(quad_layout), Some(instance_layout)],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    // Spinning billboards flip winding; culling would halve them.
                    cull_mode: None,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                // Depth test on, depth write off (same as transparent meshes).
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        // Cutout: no blending — the Bayer dither in the
                        // shader discards transparent pixels, the rest write
                        // opaquely (leaf-sprite style).
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }

    /// Register a particle texture so `tex_uri` ranges can bind it. The client
    /// calls this after uploading the texture via the normal texture pipeline.
    pub fn register_particle_texture(&mut self, uri: &str, tex: MeshTexture) {
        self.particle_tex_slots.insert(Arc::from(uri), Some(tex));
    }
}

/// `slot:N` URIs bind mesh texture slots directly; anything else must be
/// registered via [`Renderer::register_particle_texture`].
fn resolve_particle_slot(uri: &str) -> Option<MeshTexture> {
    uri.strip_prefix("slot:")
        .and_then(|rest| rest.parse::<u32>().ok())
        .map(MeshTexture)
}
