//! World-query hook for `collide_world` affectors.
//!
//! The engine never knows about voxels (or any other world shape). Games
//! implement [`WorldCollider`] and register it on the shared
//! [`crate::ParticlePool`] via [`crate::ParticlePool::set_collider`].

use glam::Vec3;

/// Single raycast hit.
#[derive(Clone, Copy, Debug)]
pub struct ColliderHit {
    /// World-space hit position (the particle is placed here).
    pub point: Vec3,
    /// Surface normal at the hit.
    pub normal: Vec3,
    /// Distance along the ray.
    pub dist: f32,
}

/// Minimal ray query the game registers next to its custom affectors.
pub trait WorldCollider {
    /// Cast a unit `dir` ray from `origin`, up to `max_dist`. `None` = clear.
    fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<ColliderHit>;
}

/// A collider with no surface at all (tests / air worlds).
pub struct NoCollider;

impl WorldCollider for NoCollider {
    fn raycast(&self, _origin: Vec3, _dir: Vec3, _max_dist: f32) -> Option<ColliderHit> {
        None
    }
}
