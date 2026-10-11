pub mod replication;
pub mod protocol;
pub mod component_types;
pub mod entity_state;
pub mod sound_event;
pub mod usercmd;

pub use component_types::ComponentType;
pub use entity_state::EntityState;
pub use sound_event::SoundEventRpc;
pub use usercmd::{
    UserCmd, AXIS_COUNT, AXIS_FORWARD, AXIS_RESERVED, AXIS_SIDE, AXIS_UP, BUTTON_ATTACK,
    BUTTON_BACK, BUTTON_DUCK, BUTTON_FORWARD, BUTTON_JUMP, BUTTON_MOVELEFT, BUTTON_MOVERIGHT,
    BUTTON_SPEED, BUTTON_USE, USERCMD_SIZE, USERCMD_VERSION,
};

/// Frame number (wraps around)
pub type FrameNumber = u8;

