//! Frame orchestration: forward [`crate::render_graph::RenderGraph`] on [`crate::renderer::Renderer`].

pub mod context;
pub mod executor;
pub mod graph;
pub mod schedule;
pub mod scope;

pub use executor::format_graph_speeds_lines;
pub use graph::build_forward_graph;
