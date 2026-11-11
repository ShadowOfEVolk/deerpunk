//! One simulated billboard particle in the shared pool.

use std::sync::Arc;

use glam::Vec3;

use crate::affector::Affector;
use crate::format::ColorDef;

/// Motion state. `Rest` particles sit on a surface (see `Affector::CollideWorld`)
/// and only age; they are not integrated further.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleState {
    /// Free-flying; affected by affectors and collision.
    Air,
    /// Parked on a surface. Velocity is zero; only `age` advances.
    Rest,
}

/// One billboard particle in the shared pool. Plain data — the pool
/// integrates it, the packer turns it into a [`GpuParticle`] instance
/// (`deerpunk_engine_common::particle_gpu`). Draw data (texture, UV rect,
/// color) is resolved at spawn time so packing is a straight copy.
#[derive(Clone, Debug)]
pub struct Particle {
    pub pos: Vec3,
    pub vel: Vec3,
    pub age: f32,
    /// Total lifetime in seconds; the particle dies at `age >= ttl`.
    pub ttl: f32,
    /// Half extent of the billboard quad (world units).
    pub size: f32,
    /// Angular velocity (rad/s); driven by `SpinWobble` affectors.
    pub spin: f32,
    /// Current rotation (rad).
    pub rot: f32,
    /// Resolved sprite rect `[u0, v0, u1, v1]` in the bound texture
    /// (payload override, atlas frame, or full texture).
    pub uv_rect: [f32; 4],
    /// Texture uri of the bound sprite; groups particles into draw ranges.
    pub tex_uri: Arc<str>,
    /// Flat spawn color rgb (payload override or the effect color start).
    pub tint_rgb: [f32; 3],
    /// Flat spawn alpha (before opacity and the fade-out ramp).
    pub tint_a: f32,
    /// Color gradient over lifetime; `Some` replaces the flat spawn tint
    /// (evaluated at pack time). `None` = flat.
    pub grad: Option<ColorDef>,
    /// Base opacity multiplier.
    pub opacity: f32,
    /// Fraction of the lifetime at the end used to fade alpha to zero.
    pub fade_out: f32,
    /// Effect-instance tag; the pool counts live particles per tag so each
    /// effect keeps its `quota` in the shared pool.
    pub tag: u32,
    pub state: ParticleState,
    /// Random phase seed for sway/flutter/spin_wobble affectors.
    pub phase: f32,
    /// Affectors resolved from the effect def, shared by every particle of
    /// the effect instance (the pool integrates per particle).
    pub affectors: Arc<[Affector]>,
}
