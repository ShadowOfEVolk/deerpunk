//! CPU lightmap atlas layers (shared with renderer upload path).

use crate::wad::RgbaImage;

pub const LM_STYLE_SLOTS: usize = 4;

/// Four atlas layers (same W×H), sampled as `texture_2d_array`.
#[derive(Clone)]
pub struct LightmapAtlasLayers {
    pub width: u32,
    pub height: u32,
    pub layers: [RgbaImage; LM_STYLE_SLOTS],
}
