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

impl LightmapAtlasLayers {
    /// Neutral full-white atlas (shader fallback when a map has no brush LM pack).
    pub fn neutral_white() -> Self {
        let layer = RgbaImage {
            width: 1,
            height: 1,
            pixels: vec![255, 255, 255, 255],
        };
        Self {
            width: 1,
            height: 1,
            layers: [
                layer.clone(),
                layer.clone(),
                layer.clone(),
                layer,
            ],
        }
    }
}
