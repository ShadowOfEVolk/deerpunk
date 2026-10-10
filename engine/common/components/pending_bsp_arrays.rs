use crate::wad::RgbaImage;

/// CPU texture-array bucket awaiting GPU upload.
#[derive(Clone)]
pub struct PendingTexBucket {
    /// Edge size of each layer (square).
    pub size: u32,
    pub layers: Vec<RgbaImage>,
}

/// Submesh before array ids are known (bucket index + layer).
#[derive(Clone, Copy, Debug)]
pub struct PendingMeshPart {
    pub first_index: u32,
    pub index_count: u32,
    pub bucket: u32,
    pub layer: u32,
}

/// Worldspawn BSP materials: buckets + submesh table. Host uploads → [`super::MeshParts`].
#[derive(Clone, Default)]
pub struct PendingBspArrays {
    pub buckets: Vec<PendingTexBucket>,
    pub parts: Vec<PendingMeshPart>,
}
