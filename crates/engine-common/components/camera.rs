use super::render_layer::RENDER_LAYER_WORLD_CAMERA;

/// Camera lens. View pose comes from [`TransformComponent`] (+ [`ParentComponent`] for pawns).
#[derive(Clone, Copy, Debug)]
pub struct CameraComponent {
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
    /// Draw mask on [`RenderLayer`] must overlap this field (bitwise AND).
    pub layer_mask: u32,
}

impl Default for CameraComponent {
    fn default() -> Self {
        Self {
            fov_y: 60.0_f32.to_radians(),
            near: 0.1,
            far: 1024.0,
            layer_mask: RENDER_LAYER_WORLD_CAMERA,
        }
    }
}

impl CameraComponent {
    pub fn aspect_ratio(&self, width: f32, height: f32) -> f32 {
        width / height
    }
}
