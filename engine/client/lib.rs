// Client-side engine components
pub mod network;
pub mod render;
pub mod systems;

pub mod common {
    pub use drifter_engine_common::*;
}

pub use render::WindowRenderer;
pub use systems::simple_entity::simple_entity_system;
pub use network::replication::{client_replication, ClientReplication};
pub use network::netchan::{NetChan, IncomingMessage};
