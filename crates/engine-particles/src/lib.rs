//! Generic engine-side particle backend.
//!
//! - [`format::EffectDef`] — universal JSON effect files loaded from
//!   `<gamedir>/particles/*.json` (engine-owned format, games author files).
//! - [`ParticleSim`] — CPU pool + spawn + integration; builtin and custom
//!   affectors (registry, unknown types are hard load errors).
//! - [`WorldCollider`] — game-registered ray hook for `collide_world`; the
//!   engine never knows about voxels.
//! - [`SpawnPayload`] — plain serializable per-call overrides (wire-friendly).
//! - [`push_billboard_vertex`] — packed billboard quads for the engine
//!   particle shader (`pass=mesh_transparent`, additive blend + Bayer cut).
//!
//! ECS wiring (the `ParticleEmitter` component, the client tick system and the
//! shared mesh upload) lives in `engine-client` / `engine-common`.

pub mod affector;
pub mod billboard;
pub mod collider;
pub mod emitter;
pub mod format;
pub mod particle;
pub mod sim;

pub use affector::{
    apply_affectors, Affector, AffectorRegistry, CustomAffector, Rest,
};
pub use billboard::{
    encode_particle_vertex, push_billboard_quad, push_billboard_vertex, AtlasRects,
    PARTICLE_VERTEX_STRIDE,
};
pub use collider::{ColliderHit, NoCollider, WorldCollider};
pub use emitter::{atlas_uv_rect, Sample, SpawnPayload, SpriteOverride};
pub use format::{
    parse_hex, AffectorDef, ColorDef, EmitterDef, EmitterKind, EffectDef, OneOrRange,
    ParticleDef, SpriteSource,
};
pub use particle::{Particle, ParticleState};
pub use sim::{ParticleSim, Rng};

#[cfg(test)]
mod tests;
