pub mod run;
pub mod world;

use glam::Vec3;
pub use run::{physics_step, physics_sync_in, physics_sync_out};
pub use world::{
    categories, BodyType, ColliderDescriptor, ColliderShape, PhysicsWorld, RapierProfile, RayHit,
    Sensor,
};

/// Компонент: immutable хендл rapier3D rigid body.
/// Устанавливается один раз при регистрации физ. тела, не меняется.
#[derive(Clone, Copy, Debug)]
pub struct RigidBodyHandle(pub u64);

impl RigidBodyHandle {
    pub fn new(handle: rapier3d::prelude::RigidBodyHandle) -> Self {
        Self(Self::encode(handle))
    }

    pub fn rapier_handle(&self) -> rapier3d::prelude::RigidBodyHandle {
        Self::decode(self.0)
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

/// Fixed-size physics body descriptor for the ECS observer pattern.
/// Game code adds this component to an entity with Transform.
/// An OnAdd observer fires, creates the rapier body, sets RigidBodyHandle + RigidBody.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PhysicsBodyDesc {
    pub body_type: u32,      // 0=static, 1=dynamic, 2=kinematic
    pub shape_type: u32,     // 0=box, 1=ball, 2=capsule
    pub shape_params: [f32; 4], // box=[hx,hy,hz,0], ball=[r,0,0,0], capsule=[hh,r,0,0]
    pub category_bits: u32,
    pub mask_bits: u32,
    pub friction: f32,
    pub restitution: f32,
    pub density: f32,
    pub is_sensor: u32,
    pub yaw: f32,
}

impl PhysicsBodyDesc {
    pub fn dynamic_box(half_extents: [f32; 3], density: f32) -> Self {
        Self {
            body_type: 1,
            shape_type: 0,
            shape_params: [half_extents[0], half_extents[1], half_extents[2], 0.0],
            category_bits: categories::TANK,
            mask_bits: 0xFFFF,
            friction: 0.5,
            restitution: 0.0,
            density,
            is_sensor: 0,
            yaw: 0.0,
        }
    }

    pub fn dynamic_box_active(half_extents: [f32; 3], density: f32) -> Self {
        let mut desc = Self::dynamic_box(half_extents, density);
        desc.body_type = 1;
        desc
    }

    pub fn static_box(half_extents: [f32; 3]) -> Self {
        Self {
            body_type: 0,
            shape_type: 0,
            shape_params: [half_extents[0], half_extents[1], half_extents[2], 0.0],
            category_bits: categories::TERRAIN,
            mask_bits: 0xFFFF,
            friction: 0.5,
            restitution: 0.0,
            density: 0.0,
            is_sensor: 0,
            yaw: 0.0,
        }
    }

    pub fn vehicle_chassis(half_extents: [f32; 3]) -> Self {
        Self {
            body_type: 1,
            shape_type: 0,
            shape_params: [half_extents[0], half_extents[1], half_extents[2], 0.0],
            category_bits: categories::TANK,
            mask_bits: 0xFFFF,
            friction: 0.5,
            restitution: 0.0,
            density: 500.0,
            is_sensor: 0,
            yaw: 0.0,
        }
    }

    pub fn to_collider_descriptor(&self) -> ColliderDescriptor {
        let shape = match self.shape_type {
            0 => ColliderShape::Box([self.shape_params[0], self.shape_params[1], self.shape_params[2]]),
            1 => ColliderShape::Ball(self.shape_params[0]),
            _ => ColliderShape::Box([0.5, 0.5, 0.5]),
        };
        let body_type = match self.body_type {
            0 => BodyType::Static,
            1 => BodyType::Dynamic,
            2 => BodyType::Kinematic,
            _ => BodyType::Dynamic,
        };
        ColliderDescriptor {
            shape,
            body_type,
            category_bits: self.category_bits,
            mask_bits: self.mask_bits,
            friction: self.friction,
            restitution: self.restitution,
            density: self.density,
            is_sensor: self.is_sensor != 0,
        }
    }
}

/// Mutable данные физ. тела.
/// `active` — тело управляется извне (танки): sync_in telefonирует + wake_up.
/// `velocity` / `angular_velocity` — целевые значения, применяются при sync_in.
#[derive(Clone, Copy, Debug)]
pub struct RigidBody {
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
    pub active: bool,
}

impl RigidBody {
    pub fn passive() -> Self {
        Self {
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            active: false,
        }
    }

    pub fn active() -> Self {
        Self {
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            active: true,
        }
    }
}
