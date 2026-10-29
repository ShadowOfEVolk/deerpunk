mod aabb;
mod cull;
mod cvars;
mod occlusion;
mod frustum;
mod octree;
mod spatial_grid;
pub mod asset_inventory;
pub mod bone_gpu;
pub mod debug_draw;
pub mod font;
pub mod frame;
pub mod gpu;
pub mod material_allocator;
pub mod material_store;
pub mod mesh_alloc_debug;
pub mod mesh_allocator;
pub mod mesh_gpu_pool;
pub mod postfx;
pub mod render_graph;
pub mod render_profile;
pub mod shader_registry;
pub mod snapshot;
pub mod text_util;
pub mod ui_pass;

pub use asset_inventory::{AssetEntry, AssetKind};
pub use debug_draw::{DebugColor, DebugDraw, DebugPrimitive};
pub use font::{
    build_font_atlas, push_char, push_text, push_text_centered_shadow, push_text_shadow,
    text_width_f32, ATLAS_H, ATLAS_W, GLYPH_H, GLYPH_W,
};
pub use gpu::{FrameError, Renderer, ShaderSources};
pub use material_allocator::{MaterialAllocator, MaterialGpuEntry, MATERIAL_WHITE_ID};
pub use mesh_alloc_debug::{BufferView, MeshAllocReport, SideView};
pub use mesh_allocator::{
    MeshBufferPool, MeshHandle, MESH_INDEX_SIZE, MESH_VERTEX_SIZE, SKINNED_MESH_VERTEX_SIZE,
    SlabMeshAllocator,
};
pub use mesh_gpu_pool::MeshGpuPool;
pub use render_graph::{GraphError, Node, NodeContext, RenderGraph, ResId, Resource, TexturePool};
pub use render_profile::{
    FrameDrawAccum, FrameSpeedsHud, GraphNodeTiming, MeshEncodeStats, MeshShaderBin,
    RenderFrameProfile,
    SceneExtractGpuStats,
};
pub use aabb::Aabb;
pub use frustum::Frustum;
pub use octree::PersistentOctree;
pub use cull::{
    scene_geometry_cull_serial, world_bounds, CullScratch, CullStats, DEFAULT_CELL_SIZE,
    GRID_MIN_DRAWS,
};
pub use snapshot::{
    rgba_u8, rgba_u8_4, CameraDefinition, CameraRenderGraph, CameraRenderSnapshot, CullOptions,
    Instance, MeshTexture, PointLightDraw, RenderScene, RenderSnapshot, RenderTarget, SceneGeometry,
    Shape, SkyDraw, StormDraw, TexSel,
};
pub use text_util::{
    color_f32, push_rect, push_text_centered, push_text_centered_f32, push_text_centered_parts,
};
