use glam::{Quat, Vec3};

use super::TransformComponent;

/// **World-space** pose after parent chain (written by [`crate::transform_hierarchy::sync_world_transforms`], read by render/audio/etc.).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Default)]
pub struct WorldTransformComponent {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl WorldTransformComponent {
    pub fn from_transform(t: &TransformComponent) -> Self {
        Self {
            position: t.position,
            rotation: t.rotation,
            scale: t.scale,
        }
    }
}

impl From<TransformComponent> for WorldTransformComponent {
    fn from(t: TransformComponent) -> Self {
        Self::from_transform(&t)
    }
}
