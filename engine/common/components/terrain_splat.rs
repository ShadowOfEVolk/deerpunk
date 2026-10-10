use super::mesh_renderer::MeshTexture;

/// Terrain splat material — 2-layer blending via splat map.
///
/// Splat map convention (per pixel):
///   R = index into `layers[0..4]` for first texture
///   G = index into `layers[0..4]` for second texture
///   B = blend factor (0.0 = all R-layer, 1.0 = all G-layer)
///
/// Only albedo for now; normal/roughness can be added later.
#[repr(C)]
#[derive(Clone)]
pub struct TerrainSplatMaterial {
    pub layers: [MeshTexture; 4],
    pub splat_map: MeshTexture,
    pub sun_dir: [f32; 4],
    pub ambient: [f32; 4],
}

impl Default for TerrainSplatMaterial {
    fn default() -> Self {
        Self {
            layers: [MeshTexture::WHITE; 4],
            splat_map: MeshTexture::WHITE,
            sun_dir: [0.3, 0.3, 1.0, 1.0],
            ambient: [0.15, 0.15, 0.2, 1.0],
        }
    }
}
