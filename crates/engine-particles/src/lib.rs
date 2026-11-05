//! Generic CPU particles: integration helpers, affectors, billboard mesh build.
//!
//! ECS upload stays in gamelib (see ohmyblock `ParticleFx` + [`BillboardParticleHost`] pattern).

mod affector;
mod billboard;
mod particle;
mod sim;

pub use affector::{apply_affectors, Affector};
pub use billboard::{push_billboard_quad, AtlasRects};
pub use particle::LiveParticle;
pub use sim::{integrate_live, IntegrateCtx};
