use crate::wad::RgbaImage;

/// Worldspawn albedo atlas awaiting GPU upload → [`super::material::Material::albedo`].
#[derive(Clone, Default)]
pub struct PendingBspAtlas {
    pub image: Option<RgbaImage>,
}

impl PendingBspAtlas {
    pub fn new(image: RgbaImage) -> Self {
        Self { image: Some(image) }
    }
}
