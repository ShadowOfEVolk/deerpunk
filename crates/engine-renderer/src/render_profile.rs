//! Per-frame render breakdown for `r_speeds` (CPU command encoding).

/// One recorded render pass (matches `wgpu::RenderPassDescriptor::label` where set).
#[derive(Clone, Debug)]
pub struct PassTiming {
    pub name: String,
    pub ms: f32,
}

/// Render-graph node summary (when a [`crate::render_graph::RenderGraph`] drives the frame).
#[derive(Clone, Debug, Default)]
pub struct GraphNodeTiming {
    pub name: String,
    pub ms: f32,
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
}

/// Mesh path diagnostics (`r_speeds 3`).
#[derive(Clone, Debug, Default)]
pub struct MeshEncodeStats {
    pub draws_submitted: u32,
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
    /// Imperative engine passes in frame order (`label: Some("… pass")`).
    pub passes: Vec<PassTiming>,
    /// Future: per-node timings when encoding goes through `RenderGraph::run`.
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
    /// CPU time between lap checkpoints while recording commands (not GPU, not `encoder.finish`).
    pub fn record_pass(&mut self, name: &'static str, ms: f32) {
        if ms >= 0.01 {
            self.passes.push(PassTiming {
                name: name.to_string(),
                ms,
            });
        }
    }

    pub fn passes_cpu_ms(&self) -> f32 {
        self.passes.iter().map(|p| p.ms).sum()
    }
}

#[inline]
pub(crate) fn lap_ms(start: &mut std::time::Instant) -> f32 {
    let now = std::time::Instant::now();
    let ms = now.duration_since(*start).as_secs_f32() * 1000.0;
    *start = now;
    ms
}
