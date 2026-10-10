/// Forward point light (position from [`super::TransformComponent`]).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PointLight {
    /// Linear RGB tint (0..1+).
    pub color: [f32; 3],
    pub intensity: f32,
    /// Falloff radius (world units). Outside → no contribution.
    pub range: f32,
    pub _pad: [f32; 3],
}

impl Default for PointLight {
    fn default() -> Self {
        Self {
            color: [1.0, 0.85, 0.55],
            intensity: 0.28,
            range: 550.0,
            _pad: [0.0; 3],
        }
    }
}

impl PointLight {
    pub fn new(color: [f32; 3], intensity: f32, range: f32) -> Self {
        Self {
            color,
            intensity,
            range,
            _pad: [0.0; 3],
        }
    }
}
