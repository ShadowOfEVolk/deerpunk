//! Audio API for gamelib (`v1:audio:*`).
//!
//! Sound requests are encoded as binary RPC packets ([`SVC::SoundEvent`]) and
//! queued on the host channel — the future network boundary. The host frame
//! drains the queue and applies them on the client.

use crate::engine_api::engine_api::EngineApiMethod;

use super::resources::read_str;
use deerpunk_engine_common::network::protocol::SVC;
use deerpunk_engine_common::network::sound_event::{
    SoundEventRpc, KIND_PLAY_2D, KIND_PLAY_3D, KIND_START_ATTACHED, KIND_STOP_ATTACHED,
};

fn queue_sound_rpc(engine: *mut u8, rpc: SoundEventRpc) -> i32 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let mut packet = Vec::with_capacity(rpc.event.len() + 48);
    packet.push(SVC::SoundEvent as u8);
    packet.extend_from_slice(&rpc.encode());
    host.channel.push_server(packet);
    0
}

/// `v1:audio:play_2d(event_ptr, event_len, volume_u8) -> 0`.
pub extern "C" fn tramp_audio_play_2d(
    engine: *mut u8,
    event_ptr: *const u8,
    event_len: usize,
    volume_u8: u32,
) -> i32 {
    queue_sound_rpc(
        engine,
        SoundEventRpc {
            kind: KIND_PLAY_2D,
            event: read_str(event_ptr, event_len),
            volume: volume_u8 as f32 / 255.0,
            ..Default::default()
        },
    )
}

/// `v1:audio:play_3d(event_ptr, event_len, x, y, z, volume_u8, param_name_ptr, param_name_len, param_value) -> 0`.
#[allow(clippy::too_many_arguments)]
pub extern "C" fn tramp_audio_play_3d(
    engine: *mut u8,
    event_ptr: *const u8,
    event_len: usize,
    x: f32,
    y: f32,
    z: f32,
    volume_u8: u32,
    param_name_ptr: *const u8,
    param_name_len: usize,
    param_value: f32,
) -> i32 {
    let name = read_str(param_name_ptr, param_name_len);
    let params = if name.is_empty() {
        Vec::new()
    } else {
        vec![(name, param_value)]
    };
    queue_sound_rpc(
        engine,
        SoundEventRpc {
            kind: KIND_PLAY_3D,
            event: read_str(event_ptr, event_len),
            position: [x, y, z],
            volume: volume_u8 as f32 / 255.0,
            params,
            ..Default::default()
        },
    )
}

/// `v1:audio:start_attached(key, event_ptr, event_len, x, y, z, volume_u8) -> 0`.
pub extern "C" fn tramp_audio_start_attached(
    engine: *mut u8,
    key: u64,
    event_ptr: *const u8,
    event_len: usize,
    x: f32,
    y: f32,
    z: f32,
    volume_u8: u32,
) -> i32 {
    queue_sound_rpc(
        engine,
        SoundEventRpc {
            kind: KIND_START_ATTACHED,
            key,
            event: read_str(event_ptr, event_len),
            position: [x, y, z],
            volume: volume_u8 as f32 / 255.0,
            ..Default::default()
        },
    )
}

/// `v1:audio:stop_attached(key) -> 0`.
pub extern "C" fn tramp_audio_stop_attached(engine: *mut u8, key: u64) -> i32 {
    queue_sound_rpc(engine, SoundEventRpc::stop_attached(key))
}

inventory::submit! { EngineApiMethod { name: "v1:audio:play_2d", callback: tramp_audio_play_2d as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:audio:play_3d", callback: tramp_audio_play_3d as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:audio:start_attached", callback: tramp_audio_start_attached as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:audio:stop_attached", callback: tramp_audio_stop_attached as *const (), flags: 0 } }
