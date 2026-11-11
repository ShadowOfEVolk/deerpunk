//! GPU features. Each module owns one subsystem's resources and its render pass.
//! Child of [`crate::gpu`] so passes can use private [`super::super::Renderer`] fields.

pub mod debug;
pub mod hud;
pub mod mesh;
pub mod particles;
pub mod post;
pub mod prep;
pub mod sky;
pub mod sprites;
