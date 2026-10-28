//! Replication (ECS) + wire re-exports from [`deerpunk_engine_network`].

pub use deerpunk_engine_network::{
    self, ComponentType, EntityState, FrameNumber, HostCommand, NetLoopback,
    ClientSessionPhase, SlotPlayGate, UserCmd, UserCmdAckRpc, USERCMD_RING_MASK,
    USERCMD_RING_SIZE, USERCMD_SIZE, USERCMD_VERSION,
    AXIS_COUNT, AXIS_FORWARD, AXIS_RESERVED, AXIS_SIDE, AXIS_UP, BUTTON_ATTACK,
    BUTTON_BACK, BUTTON_DUCK, BUTTON_FORWARD, BUTTON_JUMP, BUTTON_MOVELEFT, BUTTON_MOVERIGHT,
    BUTTON_ATTACK2, BUTTON_ATTACK3, BUTTON_DROP, BUTTON_SPEED, BUTTON_USE,
    decode_light_style, decode_map_name, decode_set_view, encode_light_style, encode_map_name,
    encode_set_view, UserMessageRpc, UM_DEST_BROADCAST,
    SoundPlayRpc, SoundStopRpc, SND_FLAG_2D, SND_FLAG_LOOP, SND_FLAG_NET_ENTITY,
};

pub mod field_apply;
pub mod field_path;
pub mod net_field;
pub mod packet_log;
pub mod prefab_rep;
pub mod stream_components;
pub mod stream_rep;

pub use stream_components::build_stream_in_components;
pub use field_path::{FieldPathId, FieldValue, NetFieldId};
pub use net_field::{NetFieldDescriptor, NetFieldKind, NetFieldRegistry};
pub use prefab_rep::{NetDeltaRpc, NetDestroyRpc, PrefabDefRpc, SpawnPrefabRpc};
pub use stream_rep::{StreamEntityUpdate, StreamInRpc, StreamOutRpc, StreamUpdateBatchRpc};
pub use packet_log::{
    clc_label, describe_svc_payload, format_clc_line, format_svc_line, should_log_clc,
    should_log_in, should_log_out, should_log_svc, svc_label,
};

// Flat re-exports (existing call sites use `common::network::protocol`, etc.)
pub use deerpunk_engine_network::lan;
pub use deerpunk_engine_network::loopback;
pub use deerpunk_engine_network::protocol;
pub use deerpunk_engine_network::replication;
pub use deerpunk_engine_network::session;
pub use deerpunk_engine_network::sound_event;
pub use deerpunk_engine_network::user_message;
pub use deerpunk_engine_network::usercmd;
