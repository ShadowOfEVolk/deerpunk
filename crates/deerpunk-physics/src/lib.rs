use glam::{Quat, Vec3};

pub mod abi;
pub mod backend;
pub mod character;
pub mod collider;
pub mod ecs_sync;
pub mod rapier_backend;
pub mod run;
pub mod voxel_query;
pub mod world;

pub use abi::{
    decode_shape, encode_shape_ball, encode_shape_box, encode_shape_convex, encode_shape_halfspace,
    encode_shape_trimesh, encode_shape_voxel_volume, ColliderPart, PartsHandle,
    ShapeGeom, ShapeHandle, BODY_TYPE_DYNAMIC, BODY_TYPE_KINEMATIC, BODY_TYPE_STATIC,
    COLLIDER_FLAG_SENSOR, PARTS_HANDLE_INVALID, SHAPE_ABI_VERSION, SHAPE_HANDLE_INVALID,
    SHAPE_KIND_BALL, SHAPE_KIND_BOX, SHAPE_KIND_CONVEX, SHAPE_KIND_HALFSPACE, SHAPE_KIND_TRIMESH,
    SHAPE_KIND_VOXEL_VOLUME, VOXEL_VOLUME_BITSET_BYTES, VOXEL_VOLUME_SIZE,
};
/// Legacy ECS collider component (ABI flecs name is now "PhysCollider"; the
/// Rust type name is unchanged). The new prefab-facing POD `Collider` lives
/// below and is a different type.
pub use abi::Collider as PhysCollider;
pub use backend::{CapsuleCastHit, NullPhysics, PhysicsBackend};
pub use character::{
    CharacterController, CHAR_EYE_HEIGHT, CHAR_FLAG_MANUAL_MOVE, CHAR_FLAG_NOCLIP,
    CHAR_HALF_HEIGHT, CHAR_RADIUS, CHAR_STEP_HEIGHT,
};
pub use collider::Sensor;
pub use rapier_backend::RapierPhysicsBackend;
pub use run::physics_step;
pub use world::{
    categories, BodyType, ColliderDescriptor, ColliderShape, PhysicsWorld, RapierProfile, RayHit,
};

/// Shape description inside the prefab-facing [`Collider`] component.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ColliderShapeDesc {
    /// 1=box, 2=ball, 3=capsule.
    pub kind: u32,
    /// box=[hx,hy,hz,0], ball=[r,0,0,0], capsule=[half_height,radius,0,0].
    pub params: [f32; 4],
    /// Local position of the shape relative to the entity pose.
    pub local_pos: [f32; 3],
}

pub const SHAPE_DESC_KIND_BOX: u32 = 1;
pub const SHAPE_DESC_KIND_BALL: u32 = 2;
pub const SHAPE_DESC_KIND_CAPSULE: u32 = 3;

/// Layout-compatible mirror of `engine-common`'s `TransformComponent`.
/// The physics crate cannot depend on `engine-common` (it re-exports this
/// crate), so the bridge reads/writes the ECS `TransformComponent` through
/// `get_raw`/`set_raw` using this `repr(C)` mirror.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TransformView {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

/// Component: immutable Rapier rigid body handle. Set once by the bridge at
/// body registration, removed together with the body.
#[derive(Clone, Copy, Debug)]
pub struct PhysBodyHandle(pub u64);

impl PhysBodyHandle {
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

/// POD component: rigid body definition. Set by game code or a prefab
/// `"RigidBody"` JSON key on an entity with a Transform. The bridge's
/// prepare-phase manual system creates the matching Rapier body; an OnRemove
/// observer destroys it.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RigidBody {
    pub body_type: u32, // 0=static, 1=dynamic, 2=kinematic
    pub gravity_scale: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub ccd_enabled: u32,
    pub can_sleep: u32,
}

/// POD component: collider definition. Optional on a body — a body without a
/// `Collider` is legal (e.g. a trigger-less kinematic marker). One component
/// holds up to 4 shapes.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Collider {
    pub shapes: [ColliderShapeDesc; 4],
    pub shape_count: u32,
    pub category_bits: u32,
    pub mask_bits: u32,
    pub friction: f32,
    pub restitution: f32,
    pub density: f32,
    pub sensor: u32,
}

/// POD component: velocity target. Dynamic bodies copy `linear` → body linvel
/// every prepare (wakes the body); writeback copies body linvel → component.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Velocity {
    pub linear: [f32; 3],
    pub angular: [f32; 3],
}

impl Velocity {
    pub fn zero() -> Self {
        Self {
            linear: [0.0; 3],
            angular: [0.0; 3],
        }
    }
}
