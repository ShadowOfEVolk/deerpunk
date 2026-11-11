//! Shared dense particle pool: spawn, integrate, per-frame GPU packing.
//!
//! One pool holds the particles of every live effect instance (each
//! [`Particle`] carries its effect's affectors and resolved draw data).
//! Per frame the client spawns into the pool, advances it once with
//! [`ParticlePool::integrate`] and packs it into one
//! [`ParticleFrame`]; the renderer draws one instanced billboard quad
//! pass per texture range. The pool never reallocates past its capacity —
//! `spawn` refuses instead (`false`).

use std::collections::HashMap;
use std::sync::Arc;

use deerpunk_engine_common::particle_gpu::{
    GpuParticle, ParticleDrawRange, ParticleFrame, GPU_PARTICLE_STRIDE,
};

use crate::affector::{apply_affectors, collide_world, Affector, AffectorRegistry, ApplyCtx};
use crate::collider::WorldCollider;
use crate::particle::{Particle, ParticleState};

const _: () = assert!(
    std::mem::size_of::<GpuParticle>() == GPU_PARTICLE_STRIDE as usize,
    "GpuParticle must stay 40 bytes",
);

/// Dense pool of live particles shared by all effect instances.
pub struct ParticlePool {
    particles: Vec<Particle>,
    /// Hard capacity; preallocated in [`ParticlePool::new`].
    capacity: usize,
    /// Sim time accumulated by `integrate`; drives sway/flutter phases.
    time: f32,
    /// World hook for `collide_world` affectors (optional at runtime).
    collider: Option<Arc<dyn WorldCollider>>,
    /// Custom affectors, resolved per apply (keyed by `"ns:name"`).
    reg: AffectorRegistry,
    /// Live particles per effect tag (per-effect quota accounting).
    tag_counts: HashMap<u32, u32>,
}

impl ParticlePool {
    /// Preallocates `capacity` particle slots.
    pub fn new(capacity: usize) -> Self {
        Self {
            particles: Vec::with_capacity(capacity),
            capacity,
            time: 0.0,
            collider: None,
            reg: AffectorRegistry::new(),
            tag_counts: HashMap::new(),
        }
    }

    /// Register the world collider used by `collide_world` affectors.
    /// Particles from effects without a collider ignore this.
    pub fn set_collider(&mut self, collider: Option<Arc<dyn WorldCollider>>) {
        self.collider = collider;
    }

    /// Register the custom-affector registry (resolved at integrate time).
    pub fn set_registry(&mut self, reg: AffectorRegistry) {
        self.reg = reg;
    }

    /// Registry access for spawn paths that adopt their effect's registry.
    pub(crate) fn reg_is_empty(&self) -> bool {
        self.reg.is_empty()
    }

    /// Live particles spawned with `tag` (per-effect quota accounting).
    pub fn live_for_tag(&self, tag: u32) -> u32 {
        self.tag_counts.get(&tag).copied().unwrap_or(0)
    }

    /// All live particles (dense, unordered).
    pub fn particles(&self) -> &[Particle] {
        &self.particles
    }

    /// Push one particle. Returns `false` when the pool is full: the particle
    /// is dropped, the capacity is a hard clamp (never reallocates).
    pub fn spawn(&mut self, p: Particle) -> bool {
        if self.particles.len() >= self.capacity {
            return false;
        }
        *self.tag_counts.entry(p.tag).or_insert(0) += 1;
        self.particles.push(p);
        true
    }

    /// Advance `dt` seconds: age, integrate, affectors, collision; dead
    /// particles are swap-removed so the pool stays dense.
    pub fn integrate(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        self.time += dt;
        let time = self.time;
        let collider = self.collider.as_deref();
        let reg = &self.reg;
        let mut i = 0;
        while i < self.particles.len() {
            let p = &mut self.particles[i];
            p.age += dt;
            if p.age >= p.ttl {
                remove_at(&mut self.particles, &mut self.tag_counts, i);
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
            let affectors = Arc::clone(&p.affectors);
            apply_affectors(&affectors, reg, p, &mut ctx);
            p.pos += p.vel * dt;
            if ctx.want_collide {
                let (bounce, friction, rest) = affectors
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
                    bounce,
                    friction,
                    rest,
                    collider.expect("collide_world without collider"),
                );
                if settled {
                    remove_at(&mut self.particles, &mut self.tag_counts, i);
                    continue;
                }
            }
            i += 1;
        }
    }

    /// Pack all live particles into `frame` (cleared first): instances are
    /// grouped by texture in first-seen bucket order, one contiguous
    /// [`ParticleDrawRange`] per texture, ready for one buffer write.
    pub fn pack_into(&mut self, frame: &mut ParticleFrame) {
        frame.clear();
        let mut order: Vec<Arc<str>> = Vec::new();
        let mut groups: HashMap<Arc<str>, Vec<GpuParticle>> = HashMap::new();
        for p in &self.particles {
            match groups.get_mut(&p.tex_uri) {
                Some(g) => g.push(pack_one(p)),
                None => {
                    order.push(Arc::clone(&p.tex_uri));
                    groups.insert(Arc::clone(&p.tex_uri), vec![pack_one(p)]);
                }
            }
        }
        let mut offset = 0u32;
        for tex in order {
            let g = groups.remove(&tex).unwrap_or_default();
            let len = g.len() as u32;
            frame.instances.extend_from_slice(&g);
            frame.ranges.push(ParticleDrawRange {
                tex_uri: tex,
                offset,
                len,
            });
            offset += len;
        }
    }

    /// `(live, capacity)`.
    pub fn stats(&self) -> (usize, usize) {
        (self.particles.len(), self.capacity)
    }

    /// Drop every live particle (allocation is kept).
    pub fn clear(&mut self) {
        self.particles.clear();
        self.tag_counts.clear();
    }
}

/// Swap-remove the particle at `i` and keep the per-tag live count in sync.
/// A free function so the pool's other fields stay borrowed (split borrows
/// in `integrate`).
fn remove_at(particles: &mut Vec<Particle>, tag_counts: &mut HashMap<u32, u32>, i: usize) {
    let p = particles.swap_remove(i);
    match tag_counts.get_mut(&p.tag) {
        Some(c) => {
            *c -= 1;
            if *c == 0 {
                tag_counts.remove(&p.tag);
            }
        }
        // Particles only enter through `spawn`, which counts the tag.
        None => debug_assert!(false, "particle with uncounted tag {}", p.tag),
    }
}

/// Turn one particle into its instance payload: position/size/rotation ride
/// as-is, the color resolves the gradient over lifetime, opacity and the
/// trailing fade-out ramp into the 8-bit tint (alpha feeds the shader's
/// Bayer coverage).
fn pack_one(p: &Particle) -> GpuParticle {
    let t = (p.age / p.ttl).clamp(0.0, 1.0);
    let base = match &p.grad {
        Some(c) => c.eval(t),
        None => [p.tint_rgb[0], p.tint_rgb[1], p.tint_rgb[2], p.tint_a],
    };
    let mut alpha = base[3] * p.opacity;
    if p.fade_out > 0.0 {
        let fade_start = (1.0 - p.fade_out).max(0.0);
        if t > fade_start {
            let k = ((t - fade_start) / p.fade_out).clamp(0.0, 1.0);
            alpha *= 1.0 - k;
        }
    }
    let enc = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    GpuParticle {
        center: p.pos.to_array(),
        half: p.size,
        rot: p.rot,
        uv_rect: p.uv_rect,
        tint: [enc(base[0]), enc(base[1]), enc(base[2]), enc(alpha)],
    }
}
