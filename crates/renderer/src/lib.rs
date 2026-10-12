pub mod asset_inventory;
pub mod debug_draw;
pub mod font;
pub mod gpu;
pub mod material_allocator;
pub mod mesh_allocator;
pub mod postfx;
pub mod render_graph;
pub mod snapshot;
pub mod text_util;
pub mod ui_pass;

pub use asset_inventory::{AssetEntry, AssetKind};
pub use gpu::{FrameError, Renderer, ShaderSources};
pub use material_allocator::{MaterialAllocator, MaterialGpuEntry, MATERIAL_WHITE_ID};
pub use mesh_allocator::{MeshAllocator, MeshHandle, MESH_INDEX_SIZE, MESH_VERTEX_SIZE};
pub use render_graph::{GraphError, Node, NodeContext, RenderGraph, ResId, Resource, TexturePool};
pub use snapshot::{
    CameraDefinition, CameraRenderGraph, CameraRenderSnapshot, Instance, MeshTexture, PointLightDraw,
    RenderScene,
    RenderSnapshot, RenderTarget, SceneGeometry, Shape, SkyDraw, StormDraw, TexSel, rgba_u8,
    rgba_u8_4,
};
pub use font::{
    build_font_atlas, push_char, push_text, push_text_centered_shadow, push_text_shadow,
    text_width_f32, ATLAS_H, ATLAS_W, GLYPH_H, GLYPH_W,
};
pub use debug_draw::{DebugColor, DebugDraw, DebugPrimitive};
pub use text_util::{
    color_f32, push_rect, push_text_centered, push_text_centered_f32, push_text_centered_parts,
};
