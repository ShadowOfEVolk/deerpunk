use crate::MeshTexture;

#[repr(C)]
#[derive(Clone, Debug)]
pub struct TerrainRenderer {
    pub color: [f32; 4],
    pub texture: MeshTexture,
    pub sun_dir: [f32; 4],
    pub ambient: [f32; 4],
}

impl Default for TerrainRenderer {
    fn default() -> Self {
        Self {
            color: [1.0, 1.0, 1.0, 1.0],
            texture: MeshTexture::WHITE,
            sun_dir: [0.0, 0.0, 1.0, 1.0],
            ambient: [0.2, 0.2, 0.25, 0.0],
        }
    }
}
