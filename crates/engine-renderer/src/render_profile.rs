//! Per-frame render breakdown for `r_speeds` (CPU command encoding).

/// One render-graph node (forward frame registration order).
#[derive(Clone, Debug, Default)]
pub struct GraphNodeTiming {
    pub name: String,
    pub ms: f32,
    /// `true` when the node ran but did no GPU work (e.g. `clear_hdr` with procedural sky).
    pub skipped: bool,
}

/// Per-frame mesh draw routing (shader id, LM-array bind layout, draw count).
#[derive(Clone, Debug, Default)]
pub struct MeshShaderBin {
    pub shader_id: u32,
    pub lm_array_layout: bool,
    pub draws: u32,
    pub tris: u64,
}

/// Material GPU resolve during client scene extract (`r_loggpu` / `r_speeds`).
#[derive(Clone, Copy, Debug, Default)]
pub struct SceneExtractGpuStats {
    /// `MaterialStore::resolve_gpu` uploaded a new SSBO row this frame.
    pub material_uploads: u32,
    /// Material already had `gpu_index` (no SSBO write).
    pub material_cached: u32,
    pub mesh_draws: u32,
    pub skinned_draws: u32,
    pub terrain_draws: u32,
    /// Time in `resolve_gpu` only, ms.
    pub resolve_ms: f32,
    /// `RenderExtractor::extract` queries + `MeshDraw` build (excludes cull serial), ms.
    pub extractor_ms: f32,
    /// `SceneGeometry::refresh_cull_serial`, ms.
    pub cull_serial_ms: f32,
    /// `fill_bone_palettes` + `fill_procedural_sky`, ms.
    pub post_ms: f32,
    /// Per-entity draw cache: full static rebuild (mesh/material change).
    pub extract_static_rebuild: u32,
    /// Cache hit: only transform / bounds updated.
    pub extract_xform_only: u32,
    /// Cache hit: unchanged entity, reused as-is.
    pub extract_cache_hit: u32,
    /// Skipped before push (early frustum in extract).
    pub extract_frustum_skip: u32,
}

/// Mesh path diagnostics (`r_speeds 3`).
#[derive(Clone, Debug, Default)]
pub struct MeshEncodeStats {
    pub draws_submitted: u32,
    /// `multi_draw_indexed_indirect` batches (one GPU indirect call each).
    pub indirect_batches: u32,
    /// Draw records inside indirect batches (`sum(first_instance slots)`).
    pub indirect_commands: u32,
    /// Per-draw `draw_indexed` inside a bind batch when indirect is unavailable.
    pub bind_batch_draws: u32,
    pub lm_array_layout_draws: u32,
    pub albedo_2d_layout_draws: u32,
    pub lm_array_with_gpu_id_zero: u32,
    pub albedo_white_slot_draws: u32,
    pub tris_drawn: u64,
    pub sample_shader_id: u32,
    pub sample_shader_path: String,
    pub sample_lm_array: bool,
    pub sample_albedo_slot: u32,
    pub sample_lightmap_gpu: u32,
    pub sample_material_id: u32,
    pub sample_has_bsp_mesh: bool,
    pub shader_bins: Vec<MeshShaderBin>,
    /// CPU inside graph node `mesh` (`r_speeds 3`).
    pub uniforms_ms: f32,
    pub mesh_for_draw_ms: f32,
    pub draw_encode_ms: f32,
}

/// GoldSrc-style `r_speeds` block (DIP / binds / lights / cull).
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameSpeedsHud {
    pub dip_count: u32,
    pub shader_bind: u32,
    pub frame_total_tris: u32,
    pub visible_world_lights: u32,
    pub visible_dyn_lights: u32,
    pub visible_sun_lights: u32,
    pub visible_instances: u32,
    pub visible_occluders: u32,
    pub occluded_objects: u32,
}

/// CPU-side draw accounting during `encode_frame` (not GPU timestamps).
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameDrawAccum {
    pub dip_count: u32,
    pub shader_bind: u32,
    pub index_count: u64,
}

impl FrameDrawAccum {
    pub fn note_pipeline(&mut self) {
        self.shader_bind += 1;
    }

    pub fn note_draw_indexed(&mut self, indices: u32) {
        if indices == 0 {
            return;
        }
        self.dip_count += 1;
        self.index_count += indices as u64;
    }

    /// Non-indexed triangle-list draws (`draw(0..n, …)`).
    pub fn note_draw_verts(&mut self, verts: u32) {
        if verts == 0 {
            return;
        }
        self.dip_count += 1;
        self.index_count += verts as u64;
    }
}

/// Last completed frame (updated in `Renderer::encode_frame`).
#[derive(Clone, Debug, Default)]
pub struct RenderFrameProfile {
    /// Client scene extract (host-measured), ms.
    pub extract_ms: f32,
    /// `tick_inner` before `render_to_surface*` (RmlUi, snapshot build, audio), ms.
    pub prep_ms: f32,
    /// RmlUi `frame()` path inside [`prep_ms`], ms.
    pub prep_ui_ms: f32,
    /// `build_snapshots` + audio listener inside [`prep_ms`], ms.
    pub prep_snapshot_ms: f32,
    /// [`RenderScene::build_snapshots`] only (frustum cull), ms.
    pub prep_cull_ms: f32,
    /// Frustum index pass inside `build_snapshots`, ms.
    pub prep_cull_frustum_ms: f32,
    /// Matrix inverse + snapshot clones in `build_snapshots`, ms.
    pub prep_cull_assemble_ms: f32,
    /// `gather_visible_draws` into GPU snapshot, ms.
    pub prep_cull_gather_ms: f32,
    /// [`ClientEngine::audio_tick`] inside snap bucket, ms.
    pub prep_audio_ms: f32,
    /// Host `frame_ui` + ImGui NewFrame callback (inside present path), ms.
    pub game_frame_ui_ms: f32,
    /// `get_current_texture` (swapchain acquire; may block), ms.
    pub surface_acquire_ms: f32,
    /// CPU: ImGui render pass encoding after `encode_frame`, ms.
    pub after_imgui_ms: f32,
    /// CPU: `CommandEncoder::finish` (assemble/validate the full frame CB), ms.
    pub encoder_finish_ms: f32,
    /// CPU/driver: `queue.submit` only (may block if GPU queue is full), ms.
    pub queue_submit_ms: f32,
    /// `encoder_finish` + `queue_submit` (legacy bucket name).
    pub after_submit_ms: f32,
    /// Wall: `queue.present` (often waits for GPU + swapchain), ms.
    pub after_present_ms: f32,
    /// `after_imgui` + `after_submit` + `after_present` (tail of the frame).
    pub after_encode_ms: f32,
    /// Material uploads during the last scene extract (see [`SceneExtractGpuStats`]).
    pub extract_gpu: SceneExtractGpuStats,
    /// New vertex/index buffer uploads in the last encode (`mesh_for_draw`).
    pub mesh_gpu_uploads: u32,
    /// Draws that reused an already-uploaded mesh.
    pub mesh_gpu_cached: u32,
    /// Forward [`crate::render_graph::RenderGraph`] node timings (registration order).
    pub graph_nodes: Vec<GraphNodeTiming>,
    pub total_encode_ms: f32,

    pub width: u32,
    pub height: u32,
    pub mesh_draws: u32,
    pub mesh_index_count: u64,
    pub sprite_instances: u32,
    pub hud_instances: u32,
    pub terrain_draws: u32,
    pub point_lights: u32,
    pub debug_verts: u32,

    pub mesh_stats: MeshEncodeStats,
    pub speeds_hud: FrameSpeedsHud,
}

impl RenderFrameProfile {
    pub fn graph_nodes_cpu_ms(&self) -> f32 {
        self.graph_nodes.iter().map(|n| n.ms).sum()
    }

    pub fn graph_node_ms(&self, name: &str) -> f32 {
        self.graph_nodes
            .iter()
            .find(|n| n.name == name)
            .map(|n| n.ms)
            .unwrap_or(0.0)
    }
}

#[inline]
pub(crate) fn lap_ms(start: &mut std::time::Instant) -> f32 {
    let now = std::time::Instant::now();
    let ms = now.duration_since(*start).as_secs_f32() * 1000.0;
    *start = now;
    ms
}
