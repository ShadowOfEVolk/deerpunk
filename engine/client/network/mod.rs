pub mod replication;
pub mod netchan;
pub mod component_registry;
pub mod frames;

pub use netchan::{NetChan, IncomingMessage};
pub use component_registry::{CLIENT_COMPONENT_REGISTRY, get_component_type, get_component_id};
