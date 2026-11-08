//! Auto-split from `encode_frame` (frame graph pass).

use std::collections::HashSet;

use deerpunk_engine_common::material::ShaderId;
use deerpunk_engine_common::resource::mesh_handle::MeshHandle;

use crate::draw_list::{MeshDrawListView, SkinnedDrawListView};
use crate::frame::context::FrameCtx;
use crate::render_profile::{self, lap_ms};
use crate::renderer::Renderer;

macro_rules! render_scope {
    ($label:literal) => {
        #[cfg(any(feature = "tracy", feature = "puffin"))]
        let _render_profile =
            crate::profile_active().then(|| crate::gamelib_profile::zone_scope($label));
    };
}

impl Renderer {
    /// Unique registry handles referenced by this camera's static and skinned draws.
    ///
    /// Chunk face splits repeat the same handle. The id list is deduped here so the
    /// upload loop never sees those repeats.
    fn visible_mesh_ids(&self, ctx: &FrameCtx<'_>) -> Vec<(MeshHandle, ShaderId)> {
        render_scope!("render/visible_mesh_ids");
        let mut seen: HashSet<usize> = HashSet::new();
        let mut ids = Vec::new();
        let meshes = MeshDrawListView::new(ctx.snapshot, ctx.geometry);
        for i in 0..meshes.len() {
            let md = meshes.get(i);
            if seen.insert(md.mesh.0) {
                ids.push((md.mesh, md.shader_id));
            }
        }
        let skinned = SkinnedDrawListView::new(ctx.snapshot, ctx.geometry);
        for j in 0..skinned.len() {
            let sd = skinned.get(j);
            if seen.insert(sd.mesh.0) {
                ids.push((sd.mesh, sd.shader_id));
            }
        }
        ids
    }

    /// Handles whose GPU slot is missing or whose `gpu_revision` moved.
    fn get_meshes_to_upload(&mut self, ctx: &FrameCtx<'_>) -> Vec<(MeshHandle, ShaderId)> {
        render_scope!("render/get_meshes_to_upload");
        let ids = self.visible_mesh_ids(ctx);
        let mut to_upload = Vec::new();
        for (mesh, shader_id) in ids {
            if self.mesh_needs_gpu_upload(mesh) {
                to_upload.push((mesh, shader_id));
            } else {
                self.frame_mesh_gpu_cached += 1;
            }
        }
        to_upload
    }

    /// Upload visible mesh bytes before the mesh pass. Re-uploads when `gpu_revision` moves.
    pub(in crate::renderer) fn prepare_mesh_uploads(&mut self, ctx: &mut FrameCtx<'_>) {
        let to_upload = self.get_meshes_to_upload(ctx);
        for (mesh, shader_id) in to_upload {
            let Some(shader) = ctx.shaders.get(shader_id) else {
                continue;
            };
            let _ = self.mesh_for_draw(&mesh, shader.mesh_vertex_layout());
        }
        ctx.prof.mesh_gpu_uploads = self.frame_mesh_gpu_uploads;
    }

    pub(crate) fn pass_upload_prep(&mut self, ctx: &mut FrameCtx<'_>) {
        self.prepare_mesh_uploads(ctx);
        if self.upload_prepped {
            let _ = lap_ms(ctx.lap);
            return;
        }
        self.upload_prepped = true;
        self.upload_sprites(ctx.snapshot);
        let _ = lap_ms(ctx.lap);
        self.material_allocator.flush(&self.gpu.queue);
        if self.material_allocator.generation != self.material_bg_gen {
            self.material_bg_gen = self.material_allocator.generation;
            self.invalidate_material_bgs();
        }
        let _ = lap_ms(ctx.lap);
    }
}
