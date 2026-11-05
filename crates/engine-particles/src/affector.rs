use glam::Vec2;

use crate::particle::LiveParticle;

#[derive(Clone, Debug, PartialEq)]
pub enum Affector {
    Gravity { strength: f32 },
    Sway {
        amplitude: f32,
        frequency: f32,
        phase_speed: f32,
    },
    Flutter { amplitude: f32, frequency: f32 },
    Spin { wobble: f32, wobble_frequency: f32 },
    Drag { horizontal: f32, vertical: f32 },
    TerminalVelocity { max_down: f32, max_horizontal: f32 },
}

pub fn apply_affectors(affectors: &[Affector], p: &mut LiveParticle, time: f32, dt: f32) {
    let t = time + p.phase;
    for aff in affectors {
        match aff {
            Affector::Gravity { strength } => {
                p.vel.z -= strength * dt;
            }
            Affector::Sway {
                amplitude,
                frequency,
                phase_speed,
            } => {
                let w = frequency * std::f32::consts::TAU;
                let ph = p.phase + time * phase_speed;
                p.vel.x += amplitude * (w * t + ph).sin() * dt;
                p.vel.y += amplitude * (w * t * 0.87 + ph * 1.13).cos() * dt;
            }
            Affector::Flutter { amplitude, frequency } => {
                let w = frequency * std::f32::consts::TAU;
                p.vel.z += amplitude * (w * t * 1.7 + p.phase).sin() * dt;
            }
            Affector::Spin { wobble, wobble_frequency } => {
                let w = wobble_frequency * std::f32::consts::TAU;
                let spin = p.spin + wobble * (w * t + p.phase).sin();
                p.rot += spin * dt;
            }
            Affector::Drag { horizontal, vertical } => {
                let hxy = (-horizontal * dt).exp();
                let hz = (-vertical * dt).exp();
                p.vel.x *= hxy;
                p.vel.y *= hxy;
                p.vel.z *= hz;
            }
            Affector::TerminalVelocity {
                max_down,
                max_horizontal,
            } => {
                let max_h = *max_horizontal;
                let max_d = *max_down;
                let xy = Vec2::new(p.vel.x, p.vel.y);
                let len = xy.length();
                if len > max_h && len > 1e-5 {
                    let s = max_h / len;
                    p.vel.x *= s;
                    p.vel.y *= s;
                }
                if p.vel.z < -max_d {
                    p.vel.z = -max_d;
                }
            }
        }
    }
}
