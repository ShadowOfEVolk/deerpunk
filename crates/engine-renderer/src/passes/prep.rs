//! Auto-split from `encode_frame` (frame graph pass).

use crate::draw_list::{MeshDrawListView, SkinnedDrawListView};
use crate::frame::context::FrameCtx;
use crate::gpu::Renderer;
use crate::render_profile::lap_ms;

impl Renderer {
    /// Upload visible mesh bytes before the mesh pass. Re-uploads when `gpu_revision` moves.
    fn prepare_mesh_uploads(&mut self, ctx: &FrameCtx<'_>) {
        let meshes = MeshDrawListView::new(ctx.snapshot, ctx.geometry);
        for i in 0..meshes.len() {
            let md = meshes.get(i);
            let Some(shader) = ctx.shaders.get(md.shader_id) else {
                continue;
            };
            let cpu = self.cpu_mesh(md.mesh);
            let _ = self.mesh_for_draw(&cpu, shader.mesh_vertex_layout());
        }
        let skinned = SkinnedDrawListView::new(ctx.snapshot, ctx.geometry);
        for j in 0..skinned.len() {
            let sd = skinned.get(j);
            let Some(shader) = ctx.shaders.get(sd.shader_id) else {
                continue;
            };
            let cpu = self.cpu_mesh(sd.mesh);
            let _ = self.mesh_for_draw(&cpu, shader.mesh_vertex_layout());
        }
    }

    pub(crate) fn pass_upload_prep(
        &mut self,
        ctx: &mut FrameCtx<'_>,
    ) {
    self.prepare_mesh_uploads(ctx);
    if self.upload_prepped {
        let _ = lap_ms(ctx.lap);
        return;
    }
    self.upload_prepped = true;
    self.upload_sprites(ctx.snapshot);
        let _ = lap_ms(ctx.lap);
    self.material_allocator.flush(&self.queue);
    if self.material_allocator.generation != self.material_bg_gen {
        self.material_bg_gen = self.material_allocator.generation;
        self.invalidate_material_bgs();
    }
        let _ = lap_ms(ctx.lap);
    }
}
