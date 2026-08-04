// Client-side engine components
// pub mod network;
pub mod render;
pub mod systems;

pub mod common {
    pub use deerpunk_engine_common::*;
}

// pub use network::netchan::{IncomingMessage, NetChan};
// pub use network::replication::{client_replication, ClientReplication};
pub use render::WindowRenderer;
pub use systems::simple_entity::simple_entity_system;
