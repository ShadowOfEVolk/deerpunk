pub mod replication;
pub mod protocol;
pub mod component_types;
pub mod entity_state;

pub use component_types::ComponentType;
pub use entity_state::EntityState;

/// Frame number (wraps around)
pub type FrameNumber = u8;

