/// One submesh draw: index range into the entity's [`super::Mesh3D`] mesh.
#[derive(Clone, Copy, Debug)]
pub struct MeshPart {
    pub first_index: u32,
    pub index_count: u32,
    /// GPU texture-array handle (from renderer upload).
    pub array_id: u32,
    /// Layer inside that array.
    pub layer: u32,
}

/// Multi-material / multi-bucket parts for one mesh (Unity-style submeshes).
#[derive(Clone, Default)]
pub struct MeshParts {
    pub parts: Vec<MeshPart>,
}

impl MeshParts {
    pub fn new(parts: Vec<MeshPart>) -> Self {
        Self { parts }
    }
}
