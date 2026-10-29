//! GPU mesh occlusion culling (1-frame latency, `entity_id`-keyed).

use std::collections::HashMap;
use std::sync::mpsc::Receiver;

use glam::{Mat4, Quat, Vec3};
use wgpu::util::DeviceExt;

use crate::aabb::Aabb;
use crate::snapshot::{MeshDraw, RenderSnapshot, SkinnedMeshDraw};

/// wgpu caps a single `QuerySet` at 4096 occlusion queries.
pub const MAX_OCCLUSION_QUERIES: u32 = 4096;

/// Consecutive occlusion-query misses before we stop drawing (reduces edge flicker).
const HIDE_AFTER_MISSES: u8 = 4;

#[derive(Clone, Copy, Default)]
struct OccEntityState {
    visible: bool,
    miss_streak: u8,
}

pub struct MeshOcclusion {
    enabled: bool,
    states: HashMap<u64, OccEntityState>,
    pending_keys: Vec<u64>,
    readback_pending: bool,
    readback_rx: Option<Receiver<Result<(), wgpu::BufferAsyncError>>>,
    warmup_frames: u8,
    last_occluded: u32,
    last_queried: u32,
    gpu: Option<OcclusionGpu>,
}

struct OcclusionGpu {
    query_set: wgpu::QuerySet,
    resolve_buf: wgpu::Buffer,
    readback: wgpu::Buffer,
    pipeline: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    bbox_vb: wgpu::Buffer,
    bbox_ib: wgpu::Buffer,
    index_count: u32,
    uniform_stride: u64,
    uniform_scratch: Vec<u8>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OccUniform {
    view_proj: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
}

impl MeshOcclusion {
    pub fn new() -> Self {
        Self {
            enabled: false,
            states: HashMap::new(),
            pending_keys: Vec::new(),
            readback_pending: false,
            readback_rx: None,
            warmup_frames: 2,
            last_occluded: 0,
            last_queried: 0,
            gpu: None,
        }
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.states.clear();
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// GPU occlusion hides draws after warmup; when false, all draws pass.
    pub fn filters_draws(&self) -> bool {
        self.enabled && self.warmup_frames == 0
    }

    pub fn last_stats(&self) -> (u32, u32) {
        (self.last_queried, self.last_occluded)
    }

    fn key_static(i: usize, entity_id: u64) -> u64 {
        if entity_id != 0 {
            entity_id
        } else {
            (i as u64) | (1u64 << 63)
        }
    }

    fn key_skinned(j: usize, entity_id: u64) -> u64 {
        if entity_id != 0 {
            entity_id
        } else {
            (j as u64) | (1u64 << 62)
        }
    }

    pub fn draw_visible_static(&self, i: usize, md: &MeshDraw) -> bool {
        if !self.enabled || self.warmup_frames > 0 {
            return true;
        }
        let key = Self::key_static(i, md.entity_id);
        Self::is_drawn(&self.states, key)
    }

    pub fn draw_visible_skinned(&self, j: usize, sd: &SkinnedMeshDraw) -> bool {
        if !self.enabled || self.warmup_frames > 0 {
            return true;
        }
        let key = Self::key_skinned(j, sd.entity_id);
        Self::is_drawn(&self.states, key)
    }

    fn apply_sample(key: u64, samples: u64, states: &mut HashMap<u64, OccEntityState>) {
        let st = states.entry(key).or_insert(OccEntityState {
            visible: true,
            miss_streak: 0,
        });
        if samples > 0 {
            st.visible = true;
            st.miss_streak = 0;
        } else {
            st.miss_streak = st.miss_streak.saturating_add(1);
            if st.miss_streak >= HIDE_AFTER_MISSES {
                st.visible = false;
            }
        }
    }

    fn is_drawn(states: &HashMap<u64, OccEntityState>, key: u64) -> bool {
        states.get(&key).map(|s| s.visible).unwrap_or(true)
    }

    pub fn process_readback(&mut self, device: &wgpu::Device) {
        if !self.readback_pending {
            return;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        let done = self
            .readback_rx
            .as_ref()
            .map(|rx| rx.try_recv().is_ok())
            .unwrap_or(false);
        if !done {
            return;
        }
        let gpu = self.gpu.as_ref().expect("occlusion gpu");
        let slice = gpu.readback.slice(..);
        let data = slice.get_mapped_range().expect("occlusion readback map");
        let words: &[u64] = bytemuck::cast_slice(&data);
        let mut occluded = 0u32;
        for (qi, key) in self.pending_keys.iter().enumerate() {
            // Missing slot → assume visible so we do not flash-hide on readback glitches.
            let samples = words.get(qi).copied().unwrap_or(1);
            Self::apply_sample(*key, samples, &mut self.states);
            if !Self::is_drawn(&self.states, *key) {
                occluded += 1;
            }
        }
        self.last_occluded = occluded;
        drop(data);
        gpu.readback.unmap();
        self.readback_pending = false;
        self.readback_rx = None;
        if self.warmup_frames > 0 {
            self.warmup_frames -= 1;
        }
    }

    pub fn queue_readback_after_submit(&mut self) {
        if !self.enabled || self.pending_keys.is_empty() || self.readback_pending {
            return;
        }
        let gpu = self.gpu.as_ref().expect("occlusion gpu");
        let (tx, rx) = std::sync::mpsc::channel();
        gpu.readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.readback_rx = Some(rx);
        self.readback_pending = true;
    }

    /// Record bbox occlusion queries against the current depth buffer. Returns true if queries were written.
    pub fn encode_queries(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        snapshot: &RenderSnapshot,
        depth_view: &wgpu::TextureView,
        color_view: &wgpu::TextureView,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> bool {
        if !self.enabled || self.readback_pending {
            return false;
        }

        self.pending_keys.clear();
        for (i, md) in snapshot.mesh_draws.iter().enumerate() {
            if self.pending_keys.len() >= MAX_OCCLUSION_QUERIES as usize {
                break;
            }
            self.pending_keys.push(Self::key_static(i, md.entity_id));
        }
        let static_n = self.pending_keys.len();
        for (j, sd) in snapshot.skinned_mesh_draws.iter().enumerate() {
            if self.pending_keys.len() >= MAX_OCCLUSION_QUERIES as usize {
                break;
            }
            self.pending_keys.push(Self::key_skinned(j, sd.entity_id));
        }
        let query_count = self.pending_keys.len();
        if query_count == 0 {
            return false;
        }
        self.last_queried = query_count as u32;

        self.ensure_gpu(device, color_format, depth_format);
        let gpu = self.gpu.as_mut().expect("occlusion gpu");

        let stride = gpu.uniform_stride;
        let slab = (stride * query_count as u64) as usize;
        gpu.uniform_scratch.resize(slab, 0);
        let view_proj = snapshot.view_proj;
        for (qi, md) in snapshot.mesh_draws.iter().take(static_n).enumerate() {
            write_uniform(&mut gpu.uniform_scratch, qi, stride, view_proj, &md.world_bounds);
        }
        for (j, sd) in snapshot.skinned_mesh_draws.iter().take(query_count - static_n).enumerate() {
            write_uniform(
                &mut gpu.uniform_scratch,
                static_n + j,
                stride,
                view_proj,
                &sd.world_bounds,
            );
        }
        queue.write_buffer(&gpu.globals_buf, 0, &gpu.uniform_scratch[..slab]);

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("occlusion query"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: Some(&gpu.query_set),
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.pipeline);
            pass.set_vertex_buffer(0, gpu.bbox_vb.slice(..));
            pass.set_index_buffer(gpu.bbox_ib.slice(..), wgpu::IndexFormat::Uint16);

            for qi in 0..query_count {
                pass.set_bind_group(0, &gpu.globals_bg, &[(qi as u64 * stride) as u32]);
                pass.begin_occlusion_query(qi as u32);
                pass.draw_indexed(0..gpu.index_count, 0, 0..1);
                pass.end_occlusion_query();
            }
        }

        let q = query_count as u32;
        encoder.resolve_query_set(&gpu.query_set, 0..q, &gpu.resolve_buf, 0);
        encoder.copy_buffer_to_buffer(&gpu.resolve_buf, 0, &gpu.readback, 0, q as u64 * 8);
        true
    }

    fn ensure_gpu(
        &mut self,
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) {
        if self.gpu.is_some() {
            return;
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("mesh occlusion"),
            ty: wgpu::QueryType::Occlusion,
            count: MAX_OCCLUSION_QUERIES,
        });
        let resolve_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occlusion resolve"),
            size: (MAX_OCCLUSION_QUERIES as u64) * 8,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occlusion readback"),
            size: (MAX_OCCLUSION_QUERIES as u64) * 8,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let align = 256u64;
        let uniform_stride =
            ((std::mem::size_of::<OccUniform>() as u64 + align - 1) / align) * align;
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occlusion uniforms"),
            size: uniform_stride * MAX_OCCLUSION_QUERIES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("occlusion layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: Some(
                        wgpu::BufferSize::new(std::mem::size_of::<OccUniform>() as u64).unwrap(),
                    ),
                },
                count: None,
            }],
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("occlusion globals"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &globals_buf,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<OccUniform>() as u64),
                }),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("occlusion pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("occlusion bbox"),
            source: wgpu::ShaderSource::Wgsl(OCCLUSION_WGSL.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("occlusion bbox"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        const VERTS: [[f32; 3]; 8] = [
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [0.5, 0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [0.5, 0.5, 0.5],
            [-0.5, 0.5, 0.5],
        ];
        const INDICES: [u16; 36] = [
            0, 1, 2, 0, 2, 3, 1, 5, 6, 1, 6, 2, 5, 4, 7, 5, 7, 6, 4, 0, 3, 4, 3, 7, 3, 2, 6, 3,
            6, 7, 4, 5, 1, 4, 1, 0,
        ];
        let bbox_vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("occlusion bbox vb"),
            contents: bytemuck::cast_slice(&VERTS),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let bbox_ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("occlusion bbox ib"),
            contents: bytemuck::cast_slice(&INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        self.gpu = Some(OcclusionGpu {
            query_set,
            resolve_buf,
            readback,
            pipeline,
            globals_buf,
            globals_bg,
            bbox_vb,
            bbox_ib,
            index_count: 36,
            uniform_stride,
            uniform_scratch: Vec::new(),
        });
    }
}

fn write_uniform(scratch: &mut [u8], qi: usize, stride: u64, view_proj: [[f32; 4]; 4], b: &Aabb) {
    let model = model_from_aabb(b);
    let u = OccUniform { view_proj, model };
    let off = (qi as u64 * stride) as usize;
    scratch[off..off + std::mem::size_of::<OccUniform>()].copy_from_slice(bytemuck::bytes_of(&u));
}

fn model_from_aabb(b: &Aabb) -> [[f32; 4]; 4] {
    if !b.is_valid() {
        return Mat4::IDENTITY.to_cols_array_2d();
    }
    let c = b.center();
    let e = b.extent().max(Vec3::splat(0.01));
    // Slightly fatten proxy vs. tight mesh AABB — fewer grazing false occlusions.
    let pad = (e * 0.04).max(Vec3::splat(0.12));
    let e = e + pad;
    Mat4::from_scale_rotation_translation(e, Quat::IDENTITY, c).to_cols_array_2d()
}

const OCCLUSION_WGSL: &str = r"
struct U {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return u.view_proj * u.model * vec4(pos, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4(0.0);
}
";
