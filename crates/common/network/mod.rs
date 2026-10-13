pub mod replication;
pub mod protocol;
pub mod component_types;
pub mod entity_state;
pub mod field_apply;
pub mod field_path;
pub mod prefab_rep;
pub mod stream_rep;
pub mod sound_event;
pub mod user_message;
pub mod usercmd;

pub use component_types::ComponentType;
pub use entity_state::EntityState;
pub use field_path::{FieldPathId, FieldValue};
pub use prefab_rep::{NetDeltaRpc, NetDestroyRpc, PrefabDefRpc, SpawnPrefabRpc};
pub use stream_rep::{StreamEntityUpdate, StreamInRpc, StreamOutRpc, StreamUpdateBatchRpc};
pub use sound_event::{SoundPlayRpc, SoundStopRpc, SND_FLAG_2D, SND_FLAG_LOOP, SND_FLAG_NET_ENTITY};
pub use user_message::{
    decode_light_style, decode_map_name, decode_set_view, encode_light_style, encode_map_name,
    encode_set_view, UserMessageRpc, UM_DEST_BROADCAST,
};
pub use usercmd::{
    UserCmd, AXIS_COUNT, AXIS_FORWARD, AXIS_RESERVED, AXIS_SIDE, AXIS_UP, BUTTON_ATTACK,
    BUTTON_BACK, BUTTON_DUCK, BUTTON_FORWARD, BUTTON_JUMP, BUTTON_MOVELEFT, BUTTON_MOVERIGHT,
    BUTTON_SPEED, BUTTON_USE, USERCMD_SIZE, USERCMD_VERSION,
};

/// Frame number (wraps around)
pub type FrameNumber = u8;

