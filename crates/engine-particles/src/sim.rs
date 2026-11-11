//! Per-effect spawn state: emission sampling into the shared pool.
//!
//! Particles live in the one shared [`ParticlePool`]; a [`ParticleSim`]
//! holds the resolved effect template (affectors, emitter, sprite) plus the
//! stream accumulator, and spawns resolved [`Particle`]s into the pool.
//! Integration happens in the pool, once per frame for all instances.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use glam::Vec3;

use crate::affector::{Affector, AffectorRegistry};
use crate::emitter::{Sample, SpawnPayload};
use crate::format::{EffectDef, EmitterKind, SpriteSource};
use crate::particle::{Particle, ParticleState};
use crate::pool::ParticlePool;

/// Deterministic xorshift — reproducible spawns in tests, no rand dep.
#[derive(Clone)]
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_f32(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        let v = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        ((v >> 40) as f32) / ((1u64 << 24) as f32)
    }

    /// Uniform in [a, b).
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next_f32()
    }
}

/// Effect-instance tags for per-effect quota accounting in the shared pool.
static NEXT_TAG: AtomicU32 = AtomicU32::new(1);

/// Spawn state for one effect template. Owns no particles — spawns into the
/// shared [`ParticlePool`].
pub struct ParticleSim {
    pub def: EffectDef,
    pub affectors: Vec<Affector>,
    /// Affectors cloned into every spawned particle (per-particle
    /// integration in the shared pool).
    affectors_shared: Arc<[Affector]>,
    pub reg: AffectorRegistry,
    /// Pool tag of this instance (per-effect quota accounting).
    pub tag: u32,
    rng: Rng,
    /// Stream spawn accumulator (fractional particles owed).
    stream_acc: f32,
    /// Total spawned (test/debug).
    pub spawned: u64,
}

impl ParticleSim {
    pub fn new(def: EffectDef, reg: &AffectorRegistry, has_collider: bool) -> Result<Self, String> {
        let mut affectors = Vec::with_capacity(def.affectors.len());
        for a in def.affectors.iter().cloned() {
            affectors.push(a.resolve(reg)?);
        }
        let needs_collider = affectors
            .iter()
            .any(|a| matches!(a, Affector::CollideWorld { .. }));
        if needs_collider && !has_collider {
            return Err("effect uses collide_world but no WorldCollider is registered".into());
        }
        // Loadable effects always carry a texture uri (`procedural` sprites
        // fail at parse); reject anyway so the pool never sees a uri-less
        // particle.
        if def.sprite.texture_uri().is_none() {
            return Err("effect sprite has no texture uri".into());
        }
        let affectors_shared: Arc<[Affector]> = affectors.as_slice().into();
        Ok(Self {
            def,
            affectors,
            affectors_shared,
            reg: reg.clone(),
            tag: NEXT_TAG.fetch_add(1, Ordering::Relaxed),
            rng: Rng::new(0x9E37_79B9_7F4A_7C15),
            stream_acc: 0.0,
            spawned: 0,
        })
    }

    /// Spawn one burst at `pos` into `pool`. Payload `None` fields fall back
    /// to the template. Honors the effect quota.
    pub fn spawn_burst(
        &mut self,
        pool: &mut ParticlePool,
        pos: Vec3,
        base_vel: Vec3,
        payload: &SpawnPayload,
        time: f32,
    ) {
        if self.def.emitter.kind != EmitterKind::Burst {
            // Streams ignore explicit bursts; use stream_owed() to drive them.
            return;
        }
        let count = Sample {
            emitter: &self.def.emitter,
            particle: &self.def.particle,
            payload,
        }
        .count(self.rng.next_f32());
        self.spawn_n(pool, count, pos, base_vel, payload, time);
    }

    fn spawn_n(
        &mut self,
        pool: &mut ParticlePool,
        count: u32,
        pos: Vec3,
        base_vel: Vec3,
        payload: &SpawnPayload,
        _time: f32,
    ) {
        // The pool resolves custom affectors at integrate time; adopt this
        // effect's registry if the pool has none yet.
        if pool.reg_is_empty() && !self.reg.is_empty() {
            pool.set_registry(self.reg.clone());
        }
        let inherit = self.def.emitter.inherit_velocity;
        let tile_count = crate::emitter::Sample::tile_count(&self.def.sprite);
        let def_tile = match self.def.sprite {
            SpriteSource::Atlas { tile, .. } => tile,
            _ => 1,
        };
        // Payload sprite override → fixed UV rect for every particle of this
        // burst (Rect) or an explicit atlas frame (Frame).
        let uv_override = payload.sprite.as_ref().map(|s| match s {
            crate::emitter::SpriteOverride::Rect { uv } => *uv,
            crate::emitter::SpriteOverride::Frame { index } => {
                crate::emitter::atlas_uv_rect(def_tile.max(1), *index as usize)
            }
        });
        // Payload texture override replaces the effect's bound sprite texture
        // (see [`SpawnPayload::texture`]): the sprite UVs then address the
        // override, so the effect texture's atlas-frame slicing is off.
        let texture_override = payload.texture.is_some();
        let tex_uri: Arc<str> = match payload
            .texture
            .as_deref()
            .map(Arc::from)
            .or_else(|| self.def.sprite.texture_uri().map(Arc::from))
        {
            Some(uri) => uri,
            // Hard-errored in `new`; unreachable for a constructed sim.
            None => return,
        };
        let use_gradient = self.def.particle.color.start != self.def.particle.color.end;
        for _ in 0..count {
            if pool.live_for_tag(self.tag) as usize >= self.def.quota {
                log::debug!(
                    "particles: quota {} reached, dropping spawn",
                    self.def.quota
                );
                return;
            }
            let mut r = Rng(u64::from(self.rng.next_f32().to_bits()).max(1) | 1);
            let u = self.rng.next_f32();
            let u2 = self.rng.next_f32();
            let u3 = self.rng.next_f32();
            let s = Sample {
                emitter: &self.def.emitter,
                particle: &self.def.particle,
                payload,
            };
            let dir = s.dir(u, u2, u3);
            let speed = s.speed(u);
            let vel = s.velocity(dir, speed, u3) + base_vel * inherit;
            let tint = s.start_tint();
            let ttl = s.ttl(r.next_f32());
            let size = s.size(r.next_f32());
            let rot = r.range(0.0, std::f32::consts::TAU);
            let frame = r.range(0.0, tile_count as f32);
            let phase = r.range(0.0, std::f32::consts::TAU);
            // UV rect resolved at spawn: override, atlas frame, or full
            // texture. With a texture override the UVs address the override
            // texture — no atlas slicing (full rect unless the caller
            // supplies an explicit rect override).
            let uv_rect = uv_override.unwrap_or_else(|| {
                if texture_override {
                    [0.0, 0.0, 1.0, 1.0]
                } else {
                    match self.def.sprite {
                        SpriteSource::Atlas { tile, .. } => {
                            crate::emitter::atlas_uv_rect(tile.max(1), frame as usize)
                        }
                        _ => [0.0, 0.0, 1.0, 1.0],
                    }
                }
            });
            let pushed = pool.spawn(Particle {
                pos,
                vel,
                age: 0.0,
                ttl,
                size,
                spin: self.def.particle.spin,
                rot,
                uv_rect,
                tex_uri: Arc::clone(&tex_uri),
                tint_rgb: [tint[0], tint[1], tint[2]],
                tint_a: tint[3],
                grad: use_gradient.then_some(self.def.particle.color),
                opacity: self.def.particle.opacity,
                fade_out: self.def.particle.fade_out,
                tag: self.tag,
                state: ParticleState::Air,
                phase,
                affectors: Arc::clone(&self.affectors_shared),
            });
            if !pushed {
                log::debug!("particles: pool full, dropping spawn");
                return;
            }
            self.spawned += 1;
        }
    }

    /// Stream spawn from an external position (the emitter entity's transform)
    /// into `pool`. Called by the engine system per frame with the accumulated
    /// count from [`ParticleSim::stream_owed`].
    pub fn spawn_stream_at(
        &mut self,
        pool: &mut ParticlePool,
        pos: Vec3,
        base_vel: Vec3,
        count: u32,
        payload: &SpawnPayload,
        time: f32,
    ) {
        if self.def.emitter.kind != EmitterKind::Stream {
            return;
        }
        self.spawn_n(pool, count, pos, base_vel, payload, time);
    }

    /// How many particles a stream emitter owes for `dt` (for the system tick).
    pub fn stream_owed(&mut self, dt: f32) -> u32 {
        if self.def.emitter.kind != EmitterKind::Stream {
            return 0;
        }
        self.stream_acc += self.def.emitter.rate * dt;
        let owed = self.stream_acc as u32;
        self.stream_acc -= owed as f32;
        owed.min(self.def.quota as u32)
    }

    /// Live particles of this effect instance in `pool`.
    pub fn live(&self, pool: &ParticlePool) -> usize {
        pool.live_for_tag(self.tag) as usize
    }
}
