//! Runtime GPU asset inventory for the inspector `assets` collection.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    Texture2d,
    TextureArray,
    Mesh,
    Material,
}

impl AssetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Texture2d => "texture2d",
            Self::TextureArray => "texture_array",
            Self::Mesh => "mesh",
            Self::Material => "material",
        }
    }
}

#[derive(Clone, Debug)]
pub struct AssetEntry {
    pub id: u64,
    pub kind: AssetKind,
    pub name: String,
    /// Payload size in bytes (approx. GPU/CPU upload size).
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub layers: Option<u32>,
    pub verts: Option<u32>,
    pub indices: Option<u32>,
}

/// Inspector id namespaces (high bits) so kinds do not collide.
pub const ASSET_ID_MESH: u64 = 0x1_0000_0000;
pub const ASSET_ID_MATERIAL: u64 = 0x2_0000_0000;
pub const ASSET_ID_TEX_ARRAY: u64 = 0x8000_0000;
