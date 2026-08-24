use glam::{Mat4, Quat, Vec3};
use serde::Deserialize;

/// 3D-transform: Z-up (X=right, Y=forward, Z=up).
/// Rotation — around Z axis (yaw).
#[repr(C)]
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Transform {
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self {
            position: Vec3::new(x, y, z),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }

    pub fn from_2d(x: f32, y: f32, yaw: f32) -> Self {
        Self {
            position: Vec3::new(x, y, 0.0),
            rotation: Quat::from_rotation_z(yaw),
            scale: Vec3::ONE,
        }
    }

    #[inline]
    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::X
    }

    /// Извлекает yaw-angle из кватерниона.
    #[inline]
    pub fn yaw(&self) -> f32 {
        let f = self.forward();
        f.y.atan2(f.x)
    }

    /// Извлекает pitch-angle (вверх/вниз) из кватерниона.
    #[inline]
    pub fn pitch(&self) -> f32 {
        let f = self.forward();
        f.z.atan2((f.x * f.x + f.y * f.y).sqrt())
    }

    #[inline]
    pub fn set_yaw(&mut self, yaw: f32) {
        let p = self.pitch();
        self.rotation = Quat::from_rotation_z(yaw) * Quat::from_rotation_y(p);
    }

    #[inline]
    pub fn set_pitch(&mut self, pitch: f32) {
        let y = self.yaw();
        self.rotation = Quat::from_rotation_z(y) * Quat::from_rotation_y(pitch);
    }

    #[inline]
    pub fn set_yaw_pitch(&mut self, yaw: f32, pitch: f32) {
        self.rotation = Quat::from_rotation_z(yaw) * Quat::from_rotation_y(pitch);
    }

    /// 4x4 модель-матрица (column-major) для рендера меша.
    #[inline]
    pub fn model_mat(&self) -> [[f32; 4]; 4] {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position)
            .to_cols_array_2d()
    }
}
