//! Spawn API: plain serializable spawn payload + cone sampling.
//!
//! [`SpawnPayload`] is wire-friendly data only (no borrows, no host handles) —
//! `None` fields mean "use the effect template default".

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::format::{EmitterDef, OneOrRange, ParticleDef, SpriteSource};

/// Per-call overrides for a burst. Plain serializable data; a wire codec can
/// encode it verbatim.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpawnPayload {
    /// Override particle count (burst emitters only).
    pub count: Option<u32>,
    /// Multiplier applied to `particle.size`.
    pub scale: Option<f32>,
    /// Multiplier applied to `emitter.speed`.
    pub speed: Option<f32>,
    /// Override the template color (flat; replaces gradient).
    pub tint: Option<[f32; 4]>,
    /// Override the emission direction (normalized when applied).
    pub dir: Option<[f32; 3]>,
    /// Sprite override (dynamic atlas rect or explicit frame).
    pub sprite: Option<SpriteOverride>,
}

/// Sprite override: game-resolved UV rect in the bound texture, or a frame
/// index into a tiled atlas.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpriteOverride {
    /// `[u0, v0, u1, v1]` in the bound texture, game-resolved.
    Rect { uv: [f32; 4] },
    /// Frame index of the effect's tiled atlas.
    Frame { index: u32 },
}

/// Deterministic RNG-free sampler: caller supplies uniforms in [0,1).
/// Kept explicit so tests can drive exact outcomes.
pub struct Sample<'a> {
    pub emitter: &'a EmitterDef,
    pub particle: &'a ParticleDef,
    pub payload: &'a SpawnPayload,
}

impl<'a> Sample<'a> {
    /// Count for a burst spawn (payload wins over template).
    pub fn count(&self, u: f32) -> u32 {
        let c = self.payload.count.map(|c| c as f32).unwrap_or_else(|| {
            let v = match self.emitter.count {
                OneOrRange::Single(v) => v,
                r @ OneOrRange::Range(_) => r.sample(u),
            };
            v
        });
        c.max(0.0) as u32
    }

    /// Speed along the emission direction.
    pub fn speed(&self, u: f32) -> f32 {
        self.emitter.speed.sample(u) * self.payload.speed.unwrap_or(1.0)
    }

    /// Emission direction: payload override or template `dir`, then cone +
    /// spread jitter. Returned unit vector.
    pub fn dir(&self, u_cone: f32, u_ang: f32, u_spread: f32) -> Vec3 {
        let base = match self.payload.dir {
            Some(d) => Vec3::new(d[0], d[1], d[2]).normalize_or_zero(),
            None => Vec3::new(self.emitter.dir[0], self.emitter.dir[1], self.emitter.dir[2])
                .normalize_or_zero(),
        };
        if base.length_squared() < 1e-8 {
            return Vec3::Z;
        }
        let base = base.normalize();
        let cone = self.emitter.cone_deg.to_radians() * 0.5;
        let _ = u_spread;
        if cone <= 0.0 {
            return base;
        }
        // Orthonormal basis around `base`.
        let up = if base.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
        let t1 = base.cross(up).normalize();
        let t2 = base.cross(t1);
        let ang = u_ang * std::f32::consts::TAU;
        let off = u_cone * cone; // uniform disc-ish
        (base + t1 * (off * ang.cos()) + t2 * (off * ang.sin())).normalize()
    }

    /// Velocity = dir * speed (+ spread jitter magnitude * speed).
    pub fn velocity(&self, dir: Vec3, speed: f32, u_spread: f32) -> Vec3 {
        let mut v = dir * speed;
        let s = self.emitter.spread.clamp(0.0, 1.0);
        if s > 0.0 {
            // Small radial kick from spread.
            let perp = if dir.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
            let perp = dir.cross(perp).normalize_or_zero();
            let j = (u_spread - 0.5) * 2.0 * s * speed;
            v += perp * j;
        }
        v
    }

    /// Lifetime.
    pub fn ttl(&self, u: f32) -> f32 {
        self.particle.ttl.sample(u)
    }

    /// Billboard half size with payload scale.
    pub fn size(&self, u: f32) -> f32 {
        self.particle.size.sample(u) * self.payload.scale.unwrap_or(1.0)
    }

    /// Start tint (template flat/gradient start, payload flat wins).
    pub fn start_tint(&self) -> [f32; 4] {
        match self.payload.tint {
            Some(t) => t,
            None => self.particle.color.start,
        }
    }

    /// Sprite frame info for the mesh builder: resolved UV rect for a given
    /// particle frame index, when the source is static or the payload overrides.
    pub fn uv_rect(&self, frame: usize, atlas: &impl Fn(usize) -> [f32; 4]) -> [f32; 4] {
        match self.payload.sprite.as_ref() {
            Some(SpriteOverride::Rect { uv }) => *uv,
            Some(SpriteOverride::Frame { index }) => atlas(*index as usize),
            None => atlas(frame),
        }
    }

    /// Max frames in a tiled atlas (1 for a static texture).
    pub fn tile_count(sprite: &SpriteSource) -> u32 {
        match sprite {
            SpriteSource::Texture { .. } => 1,
            SpriteSource::Atlas { tile, .. } => (*tile).max(1) * (*tile).max(1),
            SpriteSource::Procedural(_) => 1,
        }
    }
}

/// Uniform per-rect UV for a `tile×tile` atlas (row-major frames).
pub fn atlas_uv_rect(tile: u32, frame: usize) -> [f32; 4] {
    let t = tile.max(1);
    let f = frame as u32 % (t * t);
    let tx = (f % t) as f32;
    let ty = (f / t) as f32;
    let s = 1.0 / t as f32;
    [tx * s, ty * s, (tx + 1.0) * s, (ty + 1.0) * s]
}
