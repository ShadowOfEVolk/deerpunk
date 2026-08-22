pub mod debug_draw;
pub mod font;
pub mod gpu;
pub mod snapshot;
pub mod text_util;

pub use gpu::{FrameError, Renderer, ShaderSources};
pub use snapshot::{
    Instance, MeshTexture, RenderSnapshot, Shape, SkyDraw, StormDraw, TexSel, rgba_u8, rgba_u8_4,
};
pub use font::{
    build_font_atlas, push_char, push_text, push_text_centered_shadow, push_text_shadow,
    text_width_f32, ATLAS_H, ATLAS_W, GLYPH_H, GLYPH_W,
};
pub use debug_draw::{DebugColor, DebugDraw, DebugPrimitive};
pub use text_util::{
    color_f32, push_rect, push_text_centered, push_text_centered_f32, push_text_centered_parts,
};
