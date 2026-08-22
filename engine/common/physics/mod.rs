pub mod run;
pub mod world;

use glam::Vec3;
pub use run::{physics_step, physics_sync_in, physics_sync_out};
pub use world::{
    categories, BodyType, ColliderDescriptor, ColliderShape, PhysicsWorld, RapierProfile, RayHit,
    Sensor,
};

/// Компонент: объект с физическим телом в rapier.
/// Хендл тела хранится прямо в компоненте — прямой доступ без HashMap.
/// `active` — тело управляется извне (танки): sync_in телефонирует + wake_up.
/// `velocity` / `angular_velocity` — целевые значения, применяются при sync_in.
#[derive(Clone, Copy, Debug)]
pub struct RigidBody {
    pub handle: u64,
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
    pub active: bool,
}

impl RigidBody {
    pub fn new(handle: rapier3d::prelude::RigidBodyHandle) -> Self {
        Self {
            handle: Self::encode(handle),
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            active: false,
        }
    }

    pub fn active(handle: rapier3d::prelude::RigidBodyHandle) -> Self {
        Self {
            handle: Self::encode(handle),
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            active: true,
        }
    }

    pub fn rapier_handle(&self) -> rapier3d::prelude::RigidBodyHandle {
        Self::decode(self.handle)
    }

    pub fn encode(h: rapier3d::prelude::RigidBodyHandle) -> u64 {
        let (id, gen_) = h.into_raw_parts();
        (id as u64) | ((gen_ as u64) << 32)
    }

    pub fn decode(v: u64) -> rapier3d::prelude::RigidBodyHandle {
        let id = v as u32;
        let gen_ = (v >> 32) as u32;
        rapier3d::prelude::RigidBodyHandle::from_raw_parts(id, gen_)
    }
}
