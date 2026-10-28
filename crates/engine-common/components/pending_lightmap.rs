use crate::lightmap_atlas::LightmapAtlasLayers;
use crate::wad::RgbaImage;

/// Legacy CPU lightmap upload path (superseded by material `_Lightmap` URIs).
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

/// Legacy per-entity GPU lightmap id (superseded by material `lightmap_id` on brush materials).
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
