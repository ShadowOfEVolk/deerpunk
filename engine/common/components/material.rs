use super::mesh_renderer::MeshTexture;

/// Material component — CPU-side data that gets uploaded to GPU SSBO.
///
/// PBR metallic-roughness workflow (Cook-Torrance BRDF):
/// - color: tint multiplied by albedo texture
/// - metallic/roughness: uniform fallback when no ORM texture
/// - albedo: base color texture (group 1)
/// - normal: tangent-space normal map (group 2)
/// - ao_roughness_metallic: ORM packed texture (R=AO, G=Roughness, B=Metallic, glTF convention)
/// - emission: self-illumination
/// - sun_dir.xyz: light direction, sun_dir.w: light intensity
/// - ambient: ambient light color
#[repr(C)]
#[derive(Clone)]
pub struct Material {
    pub color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub albedo: MeshTexture,
    pub normal: MeshTexture,
    pub ao_roughness_metallic: MeshTexture,
    pub emission: [f32; 4],
    pub sun_dir: [f32; 4],
    pub ambient: [f32; 4],
}

impl Material {
    pub fn new() -> Self {
        Self {
            color: [1.0, 1.0, 1.0, 1.0],
            metallic: 0.0,
            roughness: 0.5,
            albedo: MeshTexture::WHITE,
            normal: MeshTexture::WHITE,
            ao_roughness_metallic: MeshTexture::WHITE,
            emission: [0.0, 0.0, 0.0, 0.0],
            // xyz = direction, w = intensity (0 = sun off)
            sun_dir: [0.0, 0.0, 1.0, 0.0],
            ambient: [0.0, 0.0, 0.0, 0.0],
        }
    }

    pub fn with_color(mut self, color: [f32; 4]) -> Self {
        self.color = color;
        self
    }

    pub fn with_metallic(mut self, metallic: f32) -> Self {
        self.metallic = metallic;
        self
    }

    pub fn with_roughness(mut self, roughness: f32) -> Self {
        self.roughness = roughness;
        self
    }

    pub fn with_albedo(mut self, albedo: MeshTexture) -> Self {
        self.albedo = albedo;
        self
    }

    pub fn with_normal(mut self, normal: MeshTexture) -> Self {
        self.normal = normal;
        self
    }

    pub fn with_orm(mut self, orm: MeshTexture) -> Self {
        self.ao_roughness_metallic = orm;
        self
    }

    pub fn with_sun_dir(mut self, sun_dir: [f32; 4]) -> Self {
        self.sun_dir = sun_dir;
        self
    }

    pub fn with_ambient(mut self, ambient: [f32; 4]) -> Self {
        self.ambient = ambient;
        self
    }
}

impl Default for Material {
    fn default() -> Self {
        Self::new()
    }
}
