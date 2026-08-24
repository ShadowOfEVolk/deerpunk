#[derive(Clone, Copy, Debug)]
pub struct CameraComponent {
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for CameraComponent {
    fn default() -> Self {
        Self { fov_y: 60.0_f32.to_radians(), near: 0.1, far: 1024.0 }
    }
}

impl CameraComponent {
    pub fn aspect_ratio(&self, width: f32, height: f32) -> f32 {
        width / height
    }
}
