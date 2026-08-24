use crate::resource::mesh::Mesh;

/// Texture slot for 3D meshes. Game defines semantic constants (TEX_WHITE, etc).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MeshTexture(pub u32);

impl MeshTexture {
    pub const WHITE: Self = Self(0);
    pub const fn slot(n: u32) -> Self { Self(n) }
}

#[repr(C)]
#[derive(Clone)]
pub struct MeshRenderer {
    pub mesh: Option<Mesh>,
    pub color: [f32; 4],
    pub texture: MeshTexture,
    pub sun_dir: [f32; 4],
    pub ambient: [f32; 4],
}

impl MeshRenderer {
    pub fn new() -> Self {
        Self {
            mesh: None,
            color: [1.0, 1.0, 1.0, 1.0],
            texture: MeshTexture::WHITE,
            sun_dir: [0.0, 0.0, 1.0, 0.0],
            ambient: [1.0, 1.0, 1.0, 0.0],
        }
    }
}
