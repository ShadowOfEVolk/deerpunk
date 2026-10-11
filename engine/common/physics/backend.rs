//! Pluggable physics backend (server now; client predict later).
//!
//! Same idea as client `AudioBackend`: engine owns ECS observers; backend owns
//! Rapier (or Null) + shape/parts arenas.
//!
//! Frame phases (names match darkdemo-lib):
//! `physics.prepare` → `physics.sim.step` → `physics.writeback`.

use super::abi::{Collider, ColliderPart, PartsHandle, ShapeGeom, ShapeHandle};

/// Result of [`PhysicsBackend::cast_capsule`] (world-space).
#[derive(Clone, Copy, Debug)]
pub struct CapsuleCastHit {
    /// Distance along unit `dir` to first hit.
    pub toi: f32,
    pub normal: [f32; 3],
    pub point: [f32; 3],
    /// Already overlapping at the start pose.
    pub start_solid: bool,
}

/// Pluggable physics. Single-threaded (server sim thread).
pub trait PhysicsBackend: Send {
    fn init(&mut self) -> bool;

    fn shutdown(&mut self);

    /// Register geometry from an ABI blob ([`super::abi::encode_shape_*`]).
    /// Returns a non-zero handle, or [`super::abi::SHAPE_HANDLE_INVALID`] on failure.
    fn create_shape(&mut self, abi: &[u8]) -> ShapeHandle;

    fn destroy_shape(&mut self, handle: ShapeHandle);

    /// Register a parts list (compound = N parts; boxes are just N box shapes).
    fn create_parts(&mut self, parts: &[ColliderPart]) -> PartsHandle;

    fn destroy_parts(&mut self, handle: PartsHandle);

    /// Debug: clone shape geom from the arena (`None` if missing / null backend).
    fn debug_shape(&self, handle: ShapeHandle) -> Option<ShapeGeom> {
        let _ = handle;
        None
    }

    /// Debug: clone parts list from the arena.
    fn debug_parts(&self, handle: PartsHandle) -> Option<Vec<ColliderPart>> {
        let _ = handle;
        None
    }

    /// Swept capsule (centered `capsule_z`) along unit `dir` up to `max_dist`.
    /// `exclude_entity` skips that rigid body (0 = none).
    fn cast_capsule(
        &mut self,
        center: [f32; 3],
        half_height: f32,
        radius: f32,
        dir: [f32; 3],
        max_dist: f32,
        exclude_entity: u64,
    ) -> Option<CapsuleCastHit> {
        let _ = (center, half_height, radius, dir, max_dist, exclude_entity);
        None
    }

    /// Create or rebuild the body for `entity` from [`Collider`] + world transform.
    fn upsert_body(
        &mut self,
        entity: u64,
        position: [f32; 3],
        rotation_xyzw: [f32; 4],
        collider: &Collider,
    );

    fn remove_body(&mut self, entity: u64);

    fn set_body_transform(&mut self, entity: u64, position: [f32; 3], rotation_xyzw: [f32; 4]);

    /// `physics.prepare` — CharacterController.velocity → KCC desired move; sync rigid bodies.
    fn prepare(&mut self, world: &crate::world::World, dt: f32);

    /// `physics.sim.step` — Rapier world step.
    fn step(&mut self, dt: f32);

    /// `physics.writeback` — Rapier → Transform + CharacterController.grounded.
    fn writeback(&mut self, world: &crate::world::World);

    /// Wire Flecs OnSet/OnRemove for [`Collider`] / [`super::character::CharacterController`].
    fn attach_ecs(&mut self, _world: &crate::world::World) {}
}

/// No-op backend until a real one is installed.
#[derive(Default)]
pub struct NullPhysics;

impl PhysicsBackend for NullPhysics {
    fn init(&mut self) -> bool {
        false
    }

    fn shutdown(&mut self) {}

    fn create_shape(&mut self, _abi: &[u8]) -> ShapeHandle {
        0
    }

    fn destroy_shape(&mut self, _handle: ShapeHandle) {}

    fn create_parts(&mut self, _parts: &[ColliderPart]) -> PartsHandle {
        0
    }

    fn destroy_parts(&mut self, _handle: PartsHandle) {}

    fn upsert_body(
        &mut self,
        _entity: u64,
        _position: [f32; 3],
        _rotation_xyzw: [f32; 4],
        _collider: &Collider,
    ) {
    }

    fn remove_body(&mut self, _entity: u64) {}

    fn set_body_transform(
        &mut self,
        _entity: u64,
        _position: [f32; 3],
        _rotation_xyzw: [f32; 4],
    ) {
    }

    fn prepare(&mut self, _world: &crate::world::World, _dt: f32) {}

    fn step(&mut self, _dt: f32) {}

    fn writeback(&mut self, _world: &crate::world::World) {}
}
