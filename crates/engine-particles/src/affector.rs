//! Affector vocabulary + custom-affector registry.
//!
//! Builtins are resolved at effect-load time; custom affectors live in a
//! [`AffectorRegistry`] the game fills before the catalog loads. An unknown
//! custom type name is a hard error at load — never a silent fallback.

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use serde_json::Value as JsonValue;

use crate::collider::{ColliderHit, WorldCollider};
use crate::particle::{Particle, ParticleState};

/// What a resting particle does when it stops moving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rest {
    /// Particle despawns as soon as it settles.
    Stop,
    /// Particle parks on the surface (velocity zeroed) until its ttl expires.
    Sleep,
}

/// One affector applied to a particle every step. `time` is global sim time;
/// `p.phase` is the particle's random seed for periodic motion.
#[derive(Clone, Debug, PartialEq)]
pub enum Affector {
    /// Constant downward (−Z) acceleration.
    Gravity { g: f32 },
    /// Exponential velocity damping, uniform on all axes.
    Drag { k: f32 },
    /// Clamp downward and horizontal speed.
    TerminalVelocity { v: f32 },
    /// Horizontal sinusoidal drift (leaves, sparks).
    Sway { freq: f32, amp: f32 },
    /// Vertical sinusoidal bobbing.
    Flutter { amp: f32, freq: f32 },
    /// Spin oscillation around the base spin.
    SpinWobble { wobble: f32, freq: f32 },
    /// Constant acceleration in an arbitrary direction.
    LinearForce { f: [f32; 3] },
    /// Segment collision against a [`WorldCollider`]; needs one registered.
    CollideWorld {
        bounce: f32,
        friction: f32,
        rest: Rest,
    },
    /// Game-defined effect, resolved through [`AffectorRegistry`].
    /// Unknown names are rejected at load time.
    Custom {
        type_name: String,
        params: JsonValue,
    },
}

impl Affector {
    /// Map from an effect-file `"type"` name to a builtin; `None` falls through
    /// to the custom registry.
    pub fn builtin_type_name(&self) -> Option<&'static str> {
        match self {
            Self::Gravity { .. } => Some("gravity"),
            Self::Drag { .. } => Some("drag"),
            Self::TerminalVelocity { .. } => Some("terminal_velocity"),
            Self::Sway { .. } => Some("sway"),
            Self::Flutter { .. } => Some("flutter"),
            Self::SpinWobble { .. } => Some("spin_wobble"),
            Self::LinearForce { .. } => Some("linear_force"),
            Self::CollideWorld { .. } => Some("collide_world"),
            Self::Custom { .. } => None,
        }
    }
}

/// Game-implemented custom affector. Params come verbatim from the effect JSON.
pub trait CustomAffector: Send + Sync {
    fn apply(&self, p: &mut Particle, dt: f32, params: &JsonValue);
}

/// Registry of game custom affectors, keyed by `"ns:name"`.
#[derive(Default, Clone)]
pub struct AffectorRegistry {
    customs: HashMap<String, Arc<dyn CustomAffector>>,
}

impl AffectorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_custom(&mut self, type_name: impl Into<String>, f: Arc<dyn CustomAffector>) {
        self.customs.insert(type_name.into(), f);
    }

    pub fn resolve(&self, type_name: &str) -> Option<Arc<dyn CustomAffector>> {
        self.customs.get(type_name).cloned()
    }

    pub fn len(&self) -> usize {
        self.customs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.customs.is_empty()
    }
}

/// Result of applying all affectors for one step: whether the particle wants
/// a collision segment test this frame (set by `CollideWorld`).
pub struct ApplyCtx<'a> {
    pub time: f32,
    pub dt: f32,
    pub collider: Option<&'a dyn WorldCollider>,
    /// Output: segment to test when a `CollideWorld` affector is present.
    pub want_collide: bool,
}

/// Apply affectors to one particle for `dt` seconds.
pub fn apply_affectors(
    affectors: &[Affector],
    reg: &AffectorRegistry,
    p: &mut Particle,
    ctx: &mut ApplyCtx,
) {
    let dt = ctx.dt;
    for aff in affectors {
        match aff {
            Affector::Gravity { g } => {
                p.vel.z -= *g * dt;
            }
            Affector::Drag { k } => {
                let f = (-(*k) * dt).exp();
                p.vel *= f;
            }
            Affector::TerminalVelocity { v } => {
                let max_h = *v;
                let xy = Vec3::new(p.vel.x, p.vel.y, 0.0);
                let len = xy.length();
                if len > max_h && len > 1e-5 {
                    let s = max_h / len;
                    p.vel.x *= s;
                    p.vel.y *= s;
                }
                if p.vel.z < -*v {
                    p.vel.z = -*v;
                }
            }
            Affector::Sway { freq, amp } => {
                let w = *freq * std::f32::consts::TAU;
                let ph = p.phase;
                p.vel.x += *amp * (w * ctx.time + ph).sin() * dt;
                p.vel.y += *amp * (w * ctx.time * 0.87 + ph * 1.13).cos() * dt;
            }
            Affector::Flutter { amp, freq } => {
                let w = *freq * std::f32::consts::TAU;
                p.vel.z += *amp * (w * ctx.time * 1.7 + p.phase).sin() * dt;
            }
            Affector::SpinWobble { wobble, freq } => {
                let w = *freq * std::f32::consts::TAU;
                p.spin = wobble * (w * ctx.time + p.phase).sin();
            }
            Affector::LinearForce { f } => {
                p.vel += Vec3::new(f[0], f[1], f[2]) * dt;
            }
            Affector::CollideWorld { .. } => {
                if p.state == ParticleState::Air && ctx.collider.is_some() {
                    ctx.want_collide = true;
                }
            }
            Affector::Custom { type_name, params } => {
                if let Some(f) = reg.resolve(type_name) {
                    f.apply(p, dt, params);
                }
            }
        }
    }
}

/// Segment test + response for `CollideWorld`. Returns true if the particle settled.
pub fn collide_world(
    p: &mut Particle,
    prev: Vec3,
    bounce: f32,
    friction: f32,
    rest: Rest,
    collider: &dyn WorldCollider,
) -> bool {
    let dir = p.pos - prev;
    let dist = dir.length();
    if dist <= 1e-5 {
        return false;
    }
    let Some(hit) = collider.raycast(prev, dir / dist, dist) else {
        return false;
    };
    p.pos = hit.point;
    let n = hit.normal;
    let v = p.vel;
    let vn = v.dot(n);
    if vn < 0.0 {
        // Reflect the normal component with `bounce`, damp the tangential with `friction`.
        let reflected = v - n * (vn * (1.0 + bounce));
        p.vel = reflected * friction.max(0.0);
    }
    if p.vel.length() < 0.15 {
        match rest {
            Rest::Stop => return true, // caller removes the particle
            Rest::Sleep => {
                p.vel = Vec3::ZERO;
                p.state = ParticleState::Rest;
            }
        }
    }
    false
}

/// Hit returned by a game's [`WorldCollider::raycast`].
pub use crate::collider::ColliderHit as Hit;

#[allow(unused)]
fn _hit_smoke(h: Option<ColliderHit>) -> Option<ColliderHit> {
    h
}
