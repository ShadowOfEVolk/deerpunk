use glam::Quat;

/// Camera lens + optional client-authoritative look (yaw/pitch/roll not on pawn [`TransformComponent`]).
#[derive(Clone, Copy, Debug)]
pub struct CameraComponent {
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
    /// Local +Z eye offset from entity origin (feet → head for player pawns).
    pub eye_height: f32,
    /// Horizontal look (radians, Z-up). Used when [`client_view`] is set.
    pub view_yaw: f32,
    /// Vertical look (radians).
    pub view_pitch: f32,
    /// View roll / punch (radians).
    pub view_roll: f32,
    /// If true, [`world_rotation`] ignores body yaw and uses `view_*` (local mlook).
    pub client_view: bool,
}

impl Default for CameraComponent {
    fn default() -> Self {
        Self {
            fov_y: 60.0_f32.to_radians(),
            near: 0.1,
            far: 1024.0,
            eye_height: 0.0,
            view_yaw: 0.0,
            view_pitch: 0.0,
            view_roll: 0.0,
            client_view: false,
        }
    }
}

impl CameraComponent {
    pub fn aspect_ratio(&self, width: f32, height: f32) -> f32 {
        width / height
    }

    /// World view orientation (Z-up). Player cameras: client `view_*`; map cameras: body transform.
    #[inline]
    pub fn world_rotation(&self, body_rotation: Quat) -> Quat {
        if self.client_view {
            return Quat::from_rotation_z(-self.view_yaw)
                * Quat::from_rotation_x(self.view_pitch)
                * Quat::from_rotation_y(self.view_roll);
        }
        if self.view_pitch == 0.0 && self.view_roll == 0.0 {
            return body_rotation;
        }
        body_rotation
            * Quat::from_rotation_x(self.view_pitch)
            * Quat::from_rotation_y(self.view_roll)
    }
}
