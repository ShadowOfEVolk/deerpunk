use crate::wad::RgbaImage;

/// Albedo atlas awaiting GPU upload → [`super::material::Material::albedo`].
#[derive(Clone, Default)]
pub struct PendingAlbedoAtlas {
    pub image: Option<RgbaImage>,
}

impl PendingAlbedoAtlas {
    pub fn new(image: RgbaImage) -> Self {
        Self { image: Some(image) }
    }
}

