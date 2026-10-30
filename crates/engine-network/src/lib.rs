//! Network wire format and process-local session plumbing (no Flecs / replication apply).
//!
//! ECS field registry, `field_apply`, and stream prefab export live in `deerpunk-engine-common::network`.

pub mod component_types;
pub mod entity_state;
pub mod lan;
pub mod loopback;
pub mod protocol;
pub mod replication;
pub mod session;
pub mod sound_event;
pub mod user_message;
pub mod move_ack;
pub mod client_command;
pub mod usercmd;

pub use component_types::ComponentType;
pub use entity_state::EntityState;
pub use lan::{
    bind_reuse_broadcast, decode_info, encode_info, has_connectionless_header,
    is_connectionless_query, DEFAULT_GAME_PORT, HEADER, INFO_TAG, LanAdvertiseState,
    LanServerInfo, QUERY_TAG,
};
pub use loopback::{HostCommand, NetLoopback};
pub use replication::{NetworkComponent, NetworkField};
pub use session::{ClientSessionPhase, SlotPlayGate};
pub use sound_event::{SoundPlayRpc, SoundStopRpc, SND_FLAG_2D, SND_FLAG_LOOP, SND_FLAG_NET_ENTITY};
pub use user_message::{
    decode_light_style, decode_map_name, decode_set_view, encode_light_style, encode_map_name,
    encode_set_view, UserMessageRpc, UM_DEST_BROADCAST,
};
pub use move_ack::UserCmdAckRpc;
pub use client_command::{decode_payload as decode_client_command_payload, encode_clc_packet as encode_client_command_clc, MAX_CLIENT_COMMAND_LEN};
pub use usercmd::{
    UserCmd, AXIS_COUNT, AXIS_FORWARD, AXIS_RESERVED, AXIS_SIDE, AXIS_UP, BUTTON_ATTACK,
    BUTTON_BACK, BUTTON_DUCK, BUTTON_FORWARD, BUTTON_JUMP, BUTTON_MOVELEFT, BUTTON_MOVERIGHT,
    BUTTON_ATTACK2, BUTTON_ATTACK3, BUTTON_DROP, BUTTON_SPEED, BUTTON_USE, USERCMD_RING_MASK,
    USERCMD_RING_SIZE, USERCMD_SIZE, USERCMD_VERSION,
};

/// Frame number (wraps around)
pub type FrameNumber = u8;
