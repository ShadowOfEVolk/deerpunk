use glam::{Quat, Vec3};

/// World transform. Engine convention is **Z-up** (yaw around Z).
#[derive(Clone, Copy, PartialEq)]
pub struct TransformComponent {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for TransformComponent {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

// Network replication support
impl crate::NetworkComponent for TransformComponent {
    fn declare_fields() -> &'static [crate::NetworkField] {
        static FIELDS: [crate::NetworkField; 10] = [
            crate::NetworkField { offset: 0, size: 4 },  // position.x
            crate::NetworkField { offset: 4, size: 4 },  // position.y
            crate::NetworkField { offset: 8, size: 4 },  // position.z
            crate::NetworkField { offset: 12, size: 4 },  // rotation.x
            crate::NetworkField { offset: 16, size: 4 },  // rotation.y
            crate::NetworkField { offset: 20, size: 4 },  // rotation.z
            crate::NetworkField { offset: 24, size: 4 },  // rotation.w
            crate::NetworkField { offset: 28, size: 4 },  // scale.x
            crate::NetworkField { offset: 32, size: 4 },  // scale.y
            crate::NetworkField { offset: 36, size: 4 },  // scale.z
        ];

        &FIELDS
    }
}
