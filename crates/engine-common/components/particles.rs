//! Streaming `ParticleEmitter` component (engine-side).
//!
//! Emitter state: the effect name + stream config live here; the particle
//! pool and mesh upload are engine-client runtime (`crates/engine-client/src/particles.rs`).
//! Bursts don't use this component — they go through the spawn API
//! (`ClientEngine::spawn_particles_burst`, also reachable via `v1:particles:spawn_burst`).

/// Fixed-size effect name: NUL-padded UTF-8, max 63 chars + NUL.
pub const PARTICLE_EFFECT_MAX: usize = 64;

/// Per-entity stream emitter. Position from `TransformComponent`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ParticleEmitter {
    /// Effect name in the client catalog (file stem of `<gamedir>/particles/<name>.json`).
    pub effect: [u8; PARTICLE_EFFECT_MAX],
    /// Stream rate override (particles/second). `0` = use the effect default.
    pub rate: f32,
    /// Fraction of the emitter's velocity added to spawned particles.
    /// Negative = use the effect default.
    pub inherit_velocity: f32,
    /// When false the emitter is paused (no spawning, existing particles live on).
    pub enabled: bool,
    /// Client-side stream accumulator (fractional particles owed). Not replicated.
    pub acc: f32,
}

impl Default for ParticleEmitter {
    fn default() -> Self {
        Self::new("")
    }
}

impl ParticleEmitter {
    pub fn new(effect: &str) -> Self {
        let mut buf = [0u8; PARTICLE_EFFECT_MAX];
        let bytes = effect.as_bytes();
        let n = bytes.len().min(PARTICLE_EFFECT_MAX - 1);
        buf[..n].copy_from_slice(&bytes[..n]);
        Self {
            effect: buf,
            rate: 0.0,
            inherit_velocity: -1.0,
            enabled: true,
            acc: 0.0,
        }
    }

    /// Effect name as `&str` (up to the first NUL).
    pub fn effect_str(&self) -> &str {
        let end = self.effect.iter().position(|&b| b == 0).unwrap_or(0);
        std::str::from_utf8(&self.effect[..end]).unwrap_or("")
    }
}
