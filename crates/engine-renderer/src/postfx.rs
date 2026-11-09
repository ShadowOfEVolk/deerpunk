//! Tonemap of the HDR color target onto the surface.
//! Bloom and film grain are not part of the runtime frame.

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
