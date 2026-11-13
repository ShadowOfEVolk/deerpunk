//! Surface offscreen passes: pending `UiFrame`s render into per-surface RTs
//! (`surface_<id>`, `Rgba8UnormSrgb`, ensure-pattern like the hud RT), then
//! screen sprites draw topmost above the UI composite.

use deerpunk_engine_common::frame::{frame_height, frame_width};

use crate::frame::context::FrameCtx;
use crate::renderer::Renderer;
use crate::renderer::SurfaceTexture;
use crate::ui_pass::UI_TEX_FORMAT;

impl Renderer {
    /// Renders each pending surface frame into its offscreen RT and keeps the
    /// RT registered in `surface_textures` for the gamelib. Texture content
    /// persists between frames — a surface is redrawn only when a new frame
    /// arrives.
    pub(crate) fn pass_surfaces(
        &mut self,
        _ctx: &mut FrameCtx<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        if self.pending_surfaces.is_empty() {
            return;
        }
        let ids: Vec<u32> = self.pending_surfaces.keys().copied().collect();
        for id in ids {
            let (w, h, upload) = {
                let Some(frame) = self.pending_surfaces.get(&id) else {
                    continue;
                };
                (
                    frame.screen[0].max(1),
                    frame.screen[1].max(1),
                    self.surface_upload.get(&id).copied().unwrap_or(true),
                )
            };
            self.ensure_surface_rt(id, w, h);
            let Some(view) = self.surface_textures.get(&id).map(|st| st.view.clone()) else {
                continue;
            };
            let frame = self
                .pending_surfaces
                .get(&id)
                .expect("pending surface frame");
            self.ui_pass.encode_surface(
                &self.gpu.device,
                &self.gpu.queue,
                encoder,
                id,
                &view,
                frame,
                upload,
            );
            // Frame consumed: the next `set_surface_frame` folds deltas only.
            self.surface_upload.insert(id, false);
        }
    }

    /// (Re)creates the surface RT when the surface appears or resizes
    /// (ensure-pattern like the hud RT). Bind group is sampled by the
    /// screen-sprite pass and handed to the gamelib.
    fn ensure_surface_rt(&mut self, id: u32, w: u32, h: u32) {
        if let Some(st) = self.surface_textures.get(&id) {
            if st.width == w && st.height == h {
                return;
            }
        }
        self.surface_textures.remove(&id);
        let label = format!("surface_{id}");
        let tex = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&label),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: UI_TEX_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self
            .ui_pass
            .create_texture_bind_group(&self.gpu.device, &view);
        self.surface_textures.insert(
            id,
            SurfaceTexture {
                texture: tex,
                view,
                bind_group,
                width: w,
                height: h,
            },
        );
    }

    /// Draws the accumulated screen sprites topmost onto the LDR surface,
    /// above the UI composite. Standard alpha blending; zero cost when the
    /// game pushed nothing this frame.
    pub(crate) fn pass_screen_sprites(
        &mut self,
        ctx: &mut FrameCtx<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let Some(surface_view) = ctx.surface_view else {
            return;
        };
        self.ui_pass.draw_screen_sprites(
            &self.gpu.device,
            &self.gpu.queue,
            encoder,
            surface_view,
            ctx.surface_format,
            (frame_width(), frame_height()),
            &self.surface_textures,
        );
    }
}
