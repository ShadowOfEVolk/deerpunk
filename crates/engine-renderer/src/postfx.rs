//! Tonemap of the HDR color target onto the surface.
//! Bloom and film grain are not part of the runtime frame.

use std::mem::size_of;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};

use crate::render_profile::{lap_ms, RenderFrameProfile};

fn sun_screen_uv(sun_dir: [f32; 4], camera_pos: [f32; 3], view_proj: [[f32; 4]; 4]) -> [f32; 2] {
    use glam::{Mat4, Vec3};
    if sun_dir[3] <= 0.001 {
        return [-1.0, -1.0];
    }
    let sd = Vec3::new(sun_dir[0], sun_dir[1], sun_dir[2]);
    if sd.length_squared() < 1e-8 {
        return [-1.0, -1.0];
    }
    let sd = sd.normalize();
    let eye = Vec3::from_array(camera_pos);
    let vp = Mat4::from_cols_array_2d(&view_proj);
    let wp = eye + sd * 8000.0;
    let clip = vp * wp.extend(1.0);
    if clip.w <= 0.001 {
        return [-1.0, -1.0];
    }
    let ndc = clip.truncate() / clip.w;
    let u = ndc.x * 0.5 + 0.5;
    let v = 1.0 - (ndc.y * 0.5 + 0.5);
    [u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)]
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TonemapGlobals {
    screen: [f32; 2],
    exposure: f32,
    tonemap_mode: f32,
    sun_screen: [f32; 2],
    sun_intensity: f32,
    godray_strength: f32,
    godray_decay: f32,
    godray_density: f32,
    /// World sun direction Z (up); low |Z| → golden hour boost in tonemap.
    sun_elevation: f32,
    /// Screen-space FXAA in tonemap pass (0..1, from `r_aa`).
    aa_strength: f32,
}

pub struct PostFx {
    pub godray_strength: f32,
    pub godray_decay: f32,
    pub godray_density: f32,
    pub aa_strength: f32,

    full_w: u32,
    full_h: u32,

    linear_sampler: wgpu::Sampler,
    tonemap_bg_layout: wgpu::BindGroupLayout,
    /// HDR → surface only.
    tonemap_bind: Option<wgpu::BindGroup>,
}

impl PostFx {
    pub fn new(device: &wgpu::Device, tonemap_bg_layout: wgpu::BindGroupLayout) -> Self {
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("postfx linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            godray_strength: 0.58,
            godray_decay: 0.96,
            godray_density: 1.05,
            aa_strength: 0.0,
            full_w: 0,
            full_h: 0,
            linear_sampler,
            tonemap_bg_layout,
            tonemap_bind: None,
        }
    }

    /// Drop the size-dependent tonemap bind group (call on window resize).
    pub fn invalidate_size(&mut self) {
        self.full_w = 0;
        self.full_h = 0;
        self.tonemap_bind = None;
    }

    fn make_tonemap_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        src: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        tonemap_uniform_buf: &wgpu::Buffer,
        label: &str,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tonemap_uniform_buf.as_entire_binding(),
                },
            ],
        })
    }

    /// Tonemap the HDR target onto the surface (exposure + curve).
    pub fn encode_tonemap_chain(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        hdr_view: &wgpu::TextureView,
        w: u32,
        h: u32,
        _surface_format: wgpu::TextureFormat,
        surface_view: &wgpu::TextureView,
        tonemap_surface_pipeline: &wgpu::RenderPipeline,
        tonemap_uniform_buf: &wgpu::Buffer,
        exposure: f32,
        tonemap_mode: f32,
        scene_sun_dir: [f32; 4],
        view_proj: [[f32; 4]; 4],
        camera_pos: [f32; 3],
        mut prof: Option<&mut RenderFrameProfile>,
    ) {
        let mut lap = Instant::now();
        if self.tonemap_bind.is_none() || self.full_w != w || self.full_h != h {
            self.full_w = w;
            self.full_h = h;
            self.tonemap_bind = Some(Self::make_tonemap_bg(
                device,
                &self.tonemap_bg_layout,
                hdr_view,
                &self.linear_sampler,
                tonemap_uniform_buf,
                "tonemap hdr bg",
            ));
        }

        let globals = TonemapGlobals {
            screen: [w as f32, h as f32],
            exposure,
            tonemap_mode,
            sun_screen: sun_screen_uv(scene_sun_dir, camera_pos, view_proj),
            sun_intensity: scene_sun_dir[3],
            godray_strength: self.godray_strength,
            godray_decay: self.godray_decay,
            godray_density: self.godray_density,
            sun_elevation: scene_sun_dir[2],
            aa_strength: self.aa_strength.clamp(0.0, 1.0),
        };
        queue.write_buffer(tonemap_uniform_buf, 0, bytemuck::bytes_of(&globals));

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
        pass.set_pipeline(tonemap_surface_pipeline);
        let tonemap_bg = self.tonemap_bind.as_ref().unwrap();
        pass.set_bind_group(0, tonemap_bg, &[]);
        pass.draw(0..3, 0..1);
        if let Some(_p) = prof.as_mut() {
            lap_ms(&mut lap);
        }
    }
}

/// Uniform shared by every LDR post-chain pass (`post_fx/*.wgsl`), same shape
/// as the WGSL block the chain shaders declare.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct PostFxGlobals {
    screen: [f32; 2],
    /// Seconds since renderer start (riteg's iTime).
    time: f32,
    /// Frame counter, passes through untouched (riteg authored at 30 fps).
    frame: f32,
    // WGSL rounds the struct size up to its 8-byte alignment → 24 bytes total.
    _pad: [f32; 2],
}

/// Ordered WGSL sources for the LDR post chain, loaded by the client from
/// `gamedir/shaders/post_fx/chain.toml`. Each entry is `(name, wgsl)` in chain
/// order; the last pass writes the surface, the rest ping-pong between the
/// two full-res `Rgba16Float` intermediates.
#[derive(Clone)]
pub struct PostChainSources {
    pub passes: Vec<(String, String)>,
}

struct PostChainPass {
    /// Manifest entry name; kept for diagnostics.
    #[allow(dead_code)]
    name: String,
    pipeline: wgpu::RenderPipeline,
}

/// The runtime post chain: pipelines for each manifest pass plus the two
/// ping-pong intermediates. Absent (`passes` empty) when no manifest exists —
/// a designed config state where tonemap writes straight to the surface.
/// Pipelines are built lazily: the surface format is only known at first frame.
pub(crate) struct PostFxChain {
    linear_sampler: wgpu::Sampler,
    /// Same shape as the tonemap layout: texture, sampler, uniform.
    bg_layout: wgpu::BindGroupLayout,
    uniform_buf: wgpu::Buffer,
    /// Manifest sources (kept so a surface-format change can rebuild).
    sources: Option<PostChainSources>,
    loaded_format: Option<wgpu::TextureFormat>,
    passes: Vec<PostChainPass>,
    /// One bind group per pass, bound to that pass's source view.
    bind_groups: Vec<wgpu::BindGroup>,
    ping: Option<(wgpu::Texture, wgpu::TextureView)>,
    pong: Option<(wgpu::Texture, wgpu::TextureView)>,
    chain_w: u32,
    chain_h: u32,
}

impl PostFxChain {
    pub fn new(device: &wgpu::Device, bg_layout: wgpu::BindGroupLayout) -> Self {
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post chain linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post chain uniform"),
            size: size_of::<PostFxGlobals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            linear_sampler,
            bg_layout,
            uniform_buf,
            sources: None,
            loaded_format: None,
            passes: Vec::new(),
            bind_groups: Vec::new(),
            ping: None,
            pong: None,
            chain_w: 0,
            chain_h: 0,
        }
    }

    pub fn is_loaded(&self) -> bool {
        !self.passes.is_empty()
    }

    /// Replace the manifest sources; pipelines rebuild on the next
    /// [`PostFxChain::ensure_loaded`].
    pub fn set_sources(&mut self, sources: Option<PostChainSources>) {
        self.sources = sources;
        self.clear();
        self.loaded_format = None;
    }

    /// Drop pipelines (hot reload) and size-dependent resources.
    pub fn clear(&mut self) {
        self.passes.clear();
        self.bind_groups.clear();
        self.ping = None;
        self.pong = None;
        self.chain_w = 0;
        self.chain_h = 0;
    }

    pub fn invalidate_size(&mut self) {
        self.bind_groups.clear();
        self.ping = None;
        self.pong = None;
        self.chain_w = 0;
        self.chain_h = 0;
    }

    /// Build pipelines when the sources or the surface format changed.
    pub fn ensure_loaded(&mut self, device: &wgpu::Device, surface_format: wgpu::TextureFormat) {
        // Clone the (small) manifest so `clear` can take `&mut self` below.
        let Some(sources) = self.sources.clone() else {
            return;
        };
        if self.loaded_format == Some(surface_format) && !self.passes.is_empty() {
            return;
        }
        self.clear();
        self.loaded_format = Some(surface_format);
        let last = sources.passes.len().saturating_sub(1);
        for (i, (name, wgsl)) in sources.passes.iter().enumerate() {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&format!("post chain {name}")),
                source: wgpu::ShaderSource::Wgsl(wgsl.clone().into()),
            });
            let target_format = if i == last {
                surface_format
            } else {
                wgpu::TextureFormat::Rgba16Float
            };
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&format!("post chain {name} layout")),
                bind_group_layouts: &[Some(&self.bg_layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&format!("post chain {name} pipeline")),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
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
                    module: &module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
            self.passes.push(PostChainPass {
                name: name.clone(),
                pipeline,
            });
        }
    }

    fn make_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        src: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        uniform_buf: &wgpu::Buffer,
        label: &str,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buf.as_entire_binding(),
                },
            ],
        })
    }

    /// Encode the whole chain: pass 0 samples `ldr_view`, intermediate passes
    /// ping-pong between the two full-res `Rgba16Float` targets, the last pass
    /// writes `surface_view`.
    #[allow(clippy::too_many_arguments)] // mirrors `encode_tonemap_chain`
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        ldr_view: &wgpu::TextureView,
        w: u32,
        h: u32,
        surface_view: &wgpu::TextureView,
        time: f32,
        frame: f32,
    ) {
        if self.passes.is_empty() {
            return;
        }
        if self.ping.is_none() || self.chain_w != w || self.chain_h != h {
            self.chain_w = w;
            self.chain_h = h;
            let usage =
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            let mk = |label: &str| {
                let tex = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage,
                    view_formats: &[],
                });
                let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
                (tex, view)
            };
            self.ping = Some(mk("post chain ping"));
            self.pong = Some(mk("post chain pong"));
            // Pass sources: 0 = LDR tonemap target, odd = ping, even > 0 = pong
            // (mirrors the destination alternation below).
            self.bind_groups = self
                .passes
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let label = format!("post chain bg {i}");
                    if i == 0 {
                        Self::make_bind_group(
                            device,
                            &self.bg_layout,
                            ldr_view,
                            &self.linear_sampler,
                            &self.uniform_buf,
                            &label,
                        )
                    } else {
                        let (tex, _) = if i % 2 == 1 {
                            self.ping.as_ref().unwrap()
                        } else {
                            self.pong.as_ref().unwrap()
                        };
                        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
                        Self::make_bind_group(
                            device,
                            &self.bg_layout,
                            &view,
                            &self.linear_sampler,
                            &self.uniform_buf,
                            &label,
                        )
                    }
                })
                .collect();
        }

        let globals = PostFxGlobals {
            screen: [w as f32, h as f32],
            time,
            frame,
            _pad: [0.0; 2],
        };
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&globals));

        let last = self.passes.len() - 1;
        for (i, pass) in self.passes.iter().enumerate() {
            let dst_view: &wgpu::TextureView = if i == last {
                surface_view
            } else if i % 2 == 0 {
                &self.ping.as_ref().unwrap().1
            } else {
                &self.pong.as_ref().unwrap().1
            };
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&format!("post chain pass {i}")),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: dst_view,
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
            rp.set_pipeline(&pass.pipeline);
            rp.set_bind_group(0, &self.bind_groups[i], &[]);
            rp.draw(0..3, 0..1);
        }
    }
}
