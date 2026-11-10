//! Per-effect runtime: particle pool, spawn, integration.
//!
//! One `ParticleSim` per live effect instance group is overkill for v1; the
//! pool stores particles from any effect (each carries the data the mesh
//! builder needs). The engine client keeps one sim per texture group.

use glam::Vec3;

use crate::affector::{apply_affectors, collide_world, Affector, AffectorRegistry, ApplyCtx};
use crate::collider::WorldCollider;
use crate::format::{EffectDef, EmitterKind};
use crate::particle::{Particle, ParticleState};
use crate::emitter::{Sample, SpawnPayload};

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

/// CPU particle pool + spawn state for one effect template.
pub struct ParticleSim {
    pub def: EffectDef,
    pub affectors: Vec<Affector>,
    pub reg: AffectorRegistry,
    pub particles: Vec<Particle>,
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
        Ok(Self {
            def,
            affectors,
            reg: reg.clone(),
            particles: Vec::new(),
            rng: Rng::new(0x9E37_79B9_7F4A_7C15),
            stream_acc: 0.0,
            spawned: 0,
        })
    }

    /// Spawn one burst at `pos`. Payload `None` fields fall back to the
    /// template. Honors `quota`.
    pub fn spawn_burst(&mut self, pos: Vec3, base_vel: Vec3, payload: &SpawnPayload, time: f32) {
        if self.def.emitter.kind != EmitterKind::Burst {
            // Streams ignore explicit bursts; use tick() with enabled stream.
            return;
        }
        let count = Sample {
            emitter: &self.def.emitter,
            particle: &self.def.particle,
            payload,
        }
        .count(self.rng.next_f32());
        self.spawn_n(count, pos, base_vel, payload, time);
    }

    fn spawn_n(&mut self, count: u32, pos: Vec3, base_vel: Vec3, payload: &SpawnPayload, _time: f32) {
        let inherit = self.def.emitter.inherit_velocity;
        let tile_count = crate::emitter::Sample::tile_count(&self.def.sprite);
        let def_tile = match self.def.sprite {
            crate::format::SpriteSource::Atlas { tile, .. } => tile,
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
        for _ in 0..count {
            if self.particles.len() >= self.def.quota {
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
            self.particles.push(Particle {
                pos,
                vel,
                age: 0.0,
                ttl,
                size,
                tint,
                spin: self.def.particle.spin,
                rot: r.range(0.0, std::f32::consts::TAU),
                frame: r.range(0.0, tile_count as f32),
                uv: uv_override,
                state: ParticleState::Air,
                phase: r.range(0.0, std::f32::consts::TAU),
            });
            self.spawned += 1;
        }
    }

    /// Advance `dt` seconds: age, integrate, remove dead. Stream spawning is
    /// driven by the caller (`stream_owed` + `spawn_stream_at`) since the
    /// emitter position lives on the entity component.
    pub fn tick(&mut self, dt: f32, time: f32, collider: Option<&dyn WorldCollider>) {
        if dt <= 0.0 {
            return;
        }
        self.integrate(dt, time, collider);
    }

    /// Stream spawn from an external position (the emitter entity's transform).
    /// Called by the engine system per frame with the accumulated count.
    pub fn spawn_stream_at(
        &mut self,
        pos: Vec3,
        base_vel: Vec3,
        count: u32,
        payload: &SpawnPayload,
        time: f32,
    ) {
        if self.def.emitter.kind != EmitterKind::Stream {
            return;
        }
        self.spawn_n(count, pos, base_vel, payload, time);
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

    /// Age + integrate all particles. Remove dead. `collide_world` runs a
    /// segment test from the pre-step position.
    fn integrate(&mut self, dt: f32, time: f32, collider: Option<&dyn WorldCollider>) {
        let mut i = 0;
        while i < self.particles.len() {
            let p = &mut self.particles[i];
            p.age += dt;
            if p.age >= p.ttl {
                self.particles.swap_remove(i);
                continue;
            }
            if p.state == ParticleState::Rest {
                i += 1;
                continue;
            }
            p.rot += p.spin * dt;
            let prev = p.pos;
            let mut ctx = ApplyCtx {
                time,
                dt,
                collider,
                want_collide: false,
            };
            apply_affectors(&self.affectors, &self.reg, p, &mut ctx);
            p.pos += p.vel * dt;
            if ctx.want_collide {
                let bounce = self
                    .affectors
                    .iter()
                    .find_map(|a| match a {
                        Affector::CollideWorld {
                            bounce,
                            friction,
                            rest,
                        } => Some((*bounce, *friction, *rest)),
                        _ => None,
                    })
                    .unwrap();
                let settled = collide_world(
                    p,
                    prev,
                    bounce.0,
                    bounce.1,
                    bounce.2,
                    collider.expect("collide_world without collider"),
                );
                if settled {
                    self.particles.swap_remove(i);
                    continue;
                }
            }
            i += 1;
        }
    }

    pub fn live(&self) -> usize {
        self.particles.len()
    }
}
