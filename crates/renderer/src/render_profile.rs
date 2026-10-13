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

/// Last completed frame (updated in `Renderer::encode_frame`).
#[derive(Clone, Debug, Default)]
pub struct RenderFrameProfile {
    /// Client scene extract (host-measured), ms.
    pub extract_ms: f32,
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
}

impl RenderFrameProfile {
    pub fn record_pass(&mut self, name: &'static str, ms: f32) {
        if ms >= 0.000_5 {
            self.passes.push(PassTiming {
                name: name.to_string(),
                ms,
            });
        }
    }
}

#[inline]
pub(crate) fn lap_ms(start: &mut std::time::Instant) -> f32 {
    let now = std::time::Instant::now();
    let ms = now.duration_since(*start).as_secs_f32() * 1000.0;
    *start = now;
    ms
}
