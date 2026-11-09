//! Frame orchestration: the forward pass list on [`crate::renderer::Renderer`].

pub mod context;
pub mod executor;
pub mod scope;

pub use executor::format_graph_speeds_lines;
