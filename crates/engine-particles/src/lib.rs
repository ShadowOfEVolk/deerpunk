//! Generic engine-side particle backend.
//!
//! - [`format::EffectDef`] — universal JSON effect files loaded from
//!   `<gamedir>/particles/*.json` (engine-owned format, games author files).
//! - [`ParticlePool`] — one dense CPU pool shared by every effect instance;
//!   integrates once per frame and packs one `ParticleFrame` payload for the
//!   instanced GPU billboard pass (cylindrical billboarding, additive blend
//!   + Bayer cut in the shader).
//! - [`ParticleSim`] — per-effect spawn state (cone sampling, payload
//!   overrides, stream accumulators); builtin and custom affectors
//!   (registry, unknown types are hard load errors).
//! - [`WorldCollider`] — game-registered ray hook for `collide_world`; the
//!   engine never knows about voxels.
//! - [`SpawnPayload`] — plain serializable per-call overrides (wire-friendly).
//!
//! ECS wiring (the `ParticleEmitter` component, the client tick system and
//! the renderer's instanced particle feature) lives in `engine-client` /
//! `engine-renderer`.

pub mod affector;
pub mod collider;
pub mod emitter;
pub mod format;
pub mod particle;
pub mod pool;
pub mod sim;

pub use affector::{apply_affectors, Affector, AffectorRegistry, CustomAffector, Rest};
pub use collider::{ColliderHit, NoCollider, WorldCollider};
pub use emitter::{atlas_uv_rect, Sample, SpawnPayload, SpriteOverride};
pub use format::{
    parse_hex, AffectorDef, ColorDef, EffectDef, EmitterDef, EmitterKind, OneOrRange, ParticleDef,
    SpriteSource,
};
pub use particle::{Particle, ParticleState};
pub use pool::ParticlePool;
pub use sim::{ParticleSim, Rng};

#[cfg(test)]
mod tests;
