//! Auto-split from `encode_frame` (frame graph pass).

use deerpunk_engine_common::material::ShaderId;
use deerpunk_engine_common::resource::mesh_handle::MeshHandle;

use crate::draw_list::{MeshDrawListView, SkinnedDrawListView};
use crate::frame::context::FrameCtx;
use crate::render_profile::lap_ms;
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
    /// Chunk face splits repeat the same handle. The id list is deduped with epoch
    /// marks (no hashing, no per-frame allocation) into the caller-owned scratch.
    fn visible_mesh_ids_into(&mut self, ctx: &FrameCtx<'_>, ids: &mut Vec<(MeshHandle, ShaderId)>) {
        render_scope!("render/visible_mesh_ids");
        ids.clear();
        self.visible_seen_epoch += 1;
        let epoch = self.visible_seen_epoch;
        let marks = &mut self.visible_seen_marks;
        let meshes = MeshDrawListView::new(ctx.snapshot, ctx.geometry);
        for i in 0..meshes.len() {
            let md = meshes.get(i);
            let slot = md.mesh.idx() as usize;
            if marks.len() <= slot {
                marks.resize(slot + 1, 0);
            }
            if marks[slot] != epoch {
                marks[slot] = epoch;
                ids.push((md.mesh, md.shader_id));
            }
        }
        let skinned = SkinnedDrawListView::new(ctx.snapshot, ctx.geometry);
        for j in 0..skinned.len() {
            let sd = skinned.get(j);
            let slot = sd.mesh.idx() as usize;
            if marks.len() <= slot {
                marks.resize(slot + 1, 0);
            }
            if marks[slot] != epoch {
                marks[slot] = epoch;
                ids.push((sd.mesh, sd.shader_id));
            }
        }
    }

    /// Handles whose GPU slot is missing or whose `gpu_revision` moved. Fills
    /// the caller-owned scratch; the per-item `mesh_needs_gpu_upload` checks are
    /// spanned by the `render/get_meshes_to_upload` zone only.
    fn get_meshes_to_upload_into(
        &mut self,
        ctx: &FrameCtx<'_>,
        ids: &mut Vec<(MeshHandle, ShaderId)>,
        to_upload: &mut Vec<(MeshHandle, ShaderId)>,
    ) {
        render_scope!("render/get_meshes_to_upload");
        self.visible_mesh_ids_into(ctx, ids);
        to_upload.clear();
        for (mesh, shader_id) in ids.drain(..) {
            if self.mesh_needs_gpu_upload(mesh) {
                to_upload.push((mesh, shader_id));
            } else {
                self.frame_mesh_gpu_cached += 1;
            }
        }
    }

    /// Upload visible mesh bytes before the mesh pass. Re-uploads when `gpu_revision` moves.
    pub(in crate::renderer) fn prepare_mesh_uploads(&mut self, ctx: &mut FrameCtx<'_>) {
        let mut ids = std::mem::take(&mut self.visible_ids_scratch);
        let mut to_upload = std::mem::take(&mut self.upload_list_scratch);
        self.get_meshes_to_upload_into(ctx, &mut ids, &mut to_upload);
        for (mesh, shader_id) in to_upload.drain(..) {
            let Some(shader) = ctx.shaders.get(shader_id) else {
                continue;
            };
            let _ = self.mesh_for_draw(&mesh, shader.mesh_vertex_layout());
        }
        self.upload_list_scratch = to_upload;
        self.visible_ids_scratch = ids;
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
