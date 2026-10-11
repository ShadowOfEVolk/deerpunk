pub mod replication;
pub mod protocol;
pub mod component_types;
pub mod entity_state;
pub mod sound_event;

pub use component_types::ComponentType;
pub use entity_state::EntityState;
pub use sound_event::SoundEventRpc;

/// Frame number (wraps around)
pub type FrameNumber = u8;

