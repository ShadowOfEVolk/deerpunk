//! One simulated billboard particle.

use glam::Vec3;

/// Motion state. `Rest` particles sit on a surface (see `Affector::CollideWorld`)
/// and only age; they are not integrated further.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleState {
    /// Free-flying; affected by affectors and collision.
    Air,
    /// Parked on a surface. Velocity is zero; only `age` advances.
    Rest,
}

/// One billboard particle in the shared pool. Plain data — systems copy it,
/// the mesh builder reads it.
#[derive(Clone, Debug)]
pub struct Particle {
    pub pos: Vec3,
    pub vel: Vec3,
    pub age: f32,
    pub ttl: f32,
    /// Half extent of the billboard quad (world units).
    pub size: f32,
    /// RGBA tint (alpha used for fade / additive intensity).
    pub tint: [f32; 4],
    /// Angular velocity (rad/s).
    pub spin: f32,
    /// Current rotation (rad).
    pub rot: f32,
    /// Atlas frame index (integer part) for tiled sprite sources.
    pub frame: f32,
    /// Explicit UV rect override (`[u0, v0, u1, v1]`) from a spawn payload
    /// `SpriteOverride`; `None` = derive from the effect's sprite source.
    pub uv: Option<[f32; 4]>,
    pub state: ParticleState,
    /// Random phase seed for sway/flutter affectors.
    pub phase: f32,
}
