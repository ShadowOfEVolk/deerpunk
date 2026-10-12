use crate::bsp_lightmap::LightmapAtlasLayers;
use crate::wad::RgbaImage;

/// CPU lightmap atlas (4 style layers) awaiting GPU upload.
#[derive(Clone, Default)]
pub struct PendingLightmap {
    pub atlas: Option<LightmapAtlasLayers>,
}

impl PendingLightmap {
    pub fn new(atlas: LightmapAtlasLayers) -> Self {
        Self { atlas: Some(atlas) }
    }

    /// Convenience: wrap four equal-sized images.
    pub fn from_layers(layers: [RgbaImage; 4]) -> Self {
        let width = layers[0].width;
        let height = layers[0].height;
        Self {
            atlas: Some(LightmapAtlasLayers {
                width,
                height,
                layers,
            }),
        }
    }
}

/// GPU lightmap atlas id (from renderer `upload_lightmap_layers`).
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct LightmapId {
    pub id: u32,
}

impl LightmapId {
    pub fn new(id: u32) -> Self {
        Self { id }
    }
}
