mod aabb;
pub mod asset_inventory;
pub mod bind_log;
pub mod bone_gpu;
mod cull;
mod cvars;
pub mod debug_draw;
mod draw_list;
pub mod font;
mod frustum;
pub mod gamelib_profile;
pub mod gpu;
pub mod prepare;
pub mod renderer;
pub use renderer::frame;
pub mod host_ui;
pub mod material_allocator;
pub mod material_store;
pub mod mesh_alloc_debug;
pub mod mesh_allocator;
pub mod mesh_gpu_pool;
mod mesh_shader_instance;
mod mesh_vertex_pack;
mod occlusion;
mod octree;
pub mod postfx;
pub mod puffin_lifecycle;
pub mod render_graph;
pub mod render_profile;
mod shader_fallback;
pub mod shader_registry;
pub mod snapshot;
pub mod text_util;
pub mod tracy_lifecycle;
pub mod ui_pass;
pub mod upload_log;

#[doc(hidden)]
pub mod mesh_batch_bench {
    pub use crate::renderer::{
        bench_batch_fingerprint, bench_build_indirect, bench_mdi_cache_probes, StaticBatchPlanBench,
    };
}

pub use puffin_lifecycle::{puffin_active, puffin_begin_frame, puffin_ensure_started};
pub use tracy_lifecycle::{tracy_active, tracy_ensure_started, tracy_shutdown};

/// Whether host-side instrumentation is active (Tracy or Puffin).
pub fn profile_active() -> bool {
    #[cfg(feature = "tracy")]
    {
        return tracy_active();
    }
    #[cfg(feature = "puffin")]
    {
        return puffin_active();
    }
    false
}

pub fn profile_ensure_started() {
    #[cfg(feature = "tracy")]
    tracy_ensure_started();
    #[cfg(feature = "puffin")]
    puffin_ensure_started();
}

pub fn profile_begin_frame() {
    #[cfg(feature = "tracy")]
    gamelib_profile::frame_mark();
    #[cfg(feature = "puffin")]
    puffin_begin_frame();
}

pub use aabb::Aabb;
pub use asset_inventory::{AssetEntry, AssetKind};
pub use cull::{
    mesh_local_bounds, scene_geometry_cull_serial, world_bounds, CullScratch, CullStats,
    DEFAULT_CELL_SIZE, GRID_MIN_DRAWS,
};
pub use debug_draw::{DebugColor, DebugDraw, DebugPrimitive};
pub use font::{
    build_font_atlas, push_char, push_text, push_text_centered_shadow, push_text_shadow,
    text_width_f32, ATLAS_H, ATLAS_W, GLYPH_H, GLYPH_W,
};
pub use frustum::Frustum;
pub use gpu::Gpu;
pub use material_allocator::{MaterialAllocator, MaterialGpuEntry, MATERIAL_WHITE_ID};
pub use mesh_alloc_debug::{BufferView, MeshAllocReport, SideView};
pub use mesh_allocator::{
    MeshBufferPool, MeshHandle, SlabMeshAllocator, MESH_INDEX_SIZE, MESH_VERTEX_SIZE,
    SKINNED_MESH_VERTEX_SIZE,
};
pub use mesh_gpu_pool::MeshGpuPool;
pub use octree::PersistentOctree;
pub use prepare::{run_prepare_systems, PrepareDrawList};
pub use render_graph::{GraphError, Node, NodeContext, RenderGraph, ResId, Resource, TexturePool};
pub use render_profile::{
    FrameDrawAccum, FrameSpeedsHud, GraphNodeTiming, MeshEncodeStats, MeshShaderBin,
    RenderFrameProfile, SceneExtractGpuStats,
};
pub use renderer::{
    FrameError, PendingSurfacePresent, PendingSurfaceTail, Renderer, ShaderSources, HOST_UI_ENABLED,
};
pub use snapshot::{
    cam_features, rgba_u8, rgba_u8_4, CameraDefinition, CameraRenderGraph, CameraRenderSnapshot,
    CullOptions, Instance, MeshTexture, PointLightDraw, RenderScene, RenderSnapshot, RenderTarget,
    SceneGeometry, Shape, SkyDraw, TexSel,
};
pub use text_util::{
    color_f32, push_rect, push_text_centered, push_text_centered_f32, push_text_centered_parts,
};

#[cfg(test)]
mod camera_pipeline_tests;
