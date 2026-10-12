//! Audio API for gamelib (`v1:audio:*`).
//!
//! Server assigns `sound_id`; S2C [`SVC::SoundPlay`] / [`SVC::SoundStop`].

use crate::engine_api::engine_api::EngineApiMethod;

use super::resources::read_str;
use deerpunk_engine_common::network::protocol::SVC;
use deerpunk_engine_common::network::sound_event::{SoundPlayRpc, SoundStopRpc};

fn queue_play(host: &mut crate::Host, play: SoundPlayRpc) {
    let mut packet = Vec::with_capacity(play.event.len() + 48);
    packet.push(SVC::SoundPlay as u8);
    packet.extend_from_slice(&play.encode());
    host.queue_sound_packet(packet);
}

fn queue_stop(host: &mut crate::Host, sound_id: u32) {
    let pkt = SoundStopRpc::new(sound_id).to_packet();
    host.queue_sound_packet(pkt);
}

/// `v1:audio:play_2d` → server `sound_id` (0 on failure).
pub extern "C" fn tramp_audio_play_2d(
    engine: *mut u8,
    event_ptr: *const u8,
    event_len: usize,
    volume_u8: u32,
) -> u32 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let id = host.alloc_sound_id();
    queue_play(
        host,
        SoundPlayRpc::play_2d(
            id,
            &read_str(event_ptr, event_len),
            volume_u8 as f32 / 255.0,
            Vec::new(),
        ),
    );
    id
}

/// `v1:audio:play_3d` → `sound_id`.
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
) -> u32 {
    let name = read_str(param_name_ptr, param_name_len);
    let params = if name.is_empty() {
        Vec::new()
    } else {
        vec![(name, param_value)]
    };
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let id = host.alloc_sound_id();
    queue_play(
        host,
        SoundPlayRpc::play_3d(
            id,
            &read_str(event_ptr, event_len),
            [x, y, z],
            volume_u8 as f32 / 255.0,
            params,
        ),
    );
    id
}

/// `v1:audio:start_attached(entity, …)` → looping `sound_id` (follows `net_id` when replicated).
#[allow(clippy::too_many_arguments)]
pub extern "C" fn tramp_audio_start_attached(
    engine: *mut u8,
    entity: u64,
    event_ptr: *const u8,
    event_len: usize,
    x: f32,
    y: f32,
    z: f32,
    volume_u8: u32,
) -> u32 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let id = host.alloc_sound_id();
    let net_id = {
        #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
        {
            host.server_rep.net_id_of(entity).unwrap_or(0)
        }
        #[cfg(not(any(feature = "dedicated-server", feature = "client", feature = "full")))]
        {
            0u16
        }
    };
    queue_play(
        host,
        SoundPlayRpc::play_loop(
            id,
            &read_str(event_ptr, event_len),
            [x, y, z],
            volume_u8 as f32 / 255.0,
            net_id,
        ),
    );
    id
}

/// `v1:audio:stop_sound(sound_id)`.
pub extern "C" fn tramp_audio_stop_sound(engine: *mut u8, sound_id: u32) -> i32 {
    if sound_id == 0 {
        return -1;
    }
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    queue_stop(host, sound_id);
    0
}

inventory::submit! { EngineApiMethod { name: "v1:audio:play_2d", callback: tramp_audio_play_2d as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:audio:play_3d", callback: tramp_audio_play_3d as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:audio:start_attached", callback: tramp_audio_start_attached as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:audio:stop_sound", callback: tramp_audio_stop_sound as *const (), flags: 0 } }
