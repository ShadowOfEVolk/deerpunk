//! In-process net channel + server enable (`v1:net:*`, `v1:server:*`, `v1:um:*`).

use deerpunk_engine_common::network::{UserCmd, USERCMD_SIZE};

use crate::channel::ClientCmd;
use crate::engine_api::engine_api::EngineApiMethod;
use crate::move_state;
use crate::user_messages::UmHookFn;

fn read_str(ptr: *const u8, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    unsafe {
        let bytes = std::slice::from_raw_parts(ptr, len);
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// `v1:net:client_cmd` — queue a client→server intent (`map <name>`, `disconnect`).
pub extern "C" fn tramp_net_client_cmd(
    engine: *mut u8,
    line_ptr: *const u8,
    line_len: usize,
) -> i32 {
    let line = read_str(line_ptr, line_len);
    let line = line.trim();
    if line.is_empty() {
        return -1;
    }
    let host = unsafe { &*(engine as *const crate::Host) };
    let mut parts = line.split_whitespace();
    let Some(cmd) = parts.next() else {
        return -1;
    };
    match cmd {
        "map" => {
            let name = parts.next().unwrap_or("").to_string();
            if name.is_empty() {
                return -1;
            }
            host.channel.push_client(ClientCmd::Map(name));
            0
        }
        "disconnect" => {
            host.channel.push_client(ClientCmd::Disconnect);
            0
        }
        _ => {
            log::warn!("[net] unknown client_cmd: {line}");
            -1
        }
    }
}

/// `v1:server:set_enabled` — turn server world simulation on/off.
pub extern "C" fn tramp_server_set_enabled(engine: *mut u8, enabled: i32) -> i32 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    host.set_server_enabled(enabled != 0);
    0
}

/// `v1:server:activated` — map/load done, server sim ready; host connects local client.
pub extern "C" fn tramp_server_activated(engine: *mut u8) -> i32 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    host.on_server_activated();
    0
}

/// `v1:server:is_enabled` → 1/0.
pub extern "C" fn tramp_server_is_enabled(engine: *mut u8) -> i32 {
    let host = unsafe { &*(engine as *const crate::Host) };
    if host.server_engine.enabled {
        1
    } else {
        0
    }
}

/// `v1:net:in_game` → client in **play** phase (`InitDone`), listen or UDP.
pub extern "C" fn tramp_net_in_game(engine: *mut u8) -> i32 {
    let host = unsafe { &*(engine as *const crate::Host) };
    if host.client_in_play() {
        1
    } else {
        0
    }
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:net:client_cmd",
        callback: tramp_net_client_cmd as *const (),
        flags: 0,
    }
}
inventory::submit! {
    EngineApiMethod {
        name: "v1:server:set_enabled",
        callback: tramp_server_set_enabled as *const (),
        flags: 0,
    }
}
inventory::submit! {
    EngineApiMethod {
        name: "v1:server:is_enabled",
        callback: tramp_server_is_enabled as *const (),
        flags: 0,
    }
}
inventory::submit! {
    EngineApiMethod {
        name: "v1:server:activated",
        callback: tramp_server_activated as *const (),
        flags: 0,
    }
}
inventory::submit! {
    EngineApiMethod {
        name: "v1:net:in_game",
        callback: tramp_net_in_game as *const (),
        flags: 0,
    }
}

/// `v1:players:set_entity(slot, entity)` — bind pawn entity to a connection.
pub extern "C" fn tramp_players_set_entity(engine: *mut u8, slot: u32, entity: u64) -> i32 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    host.players.set_entity(slot, entity);
    0
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:players:set_entity",
        callback: tramp_players_set_entity as *const (),
        flags: 0,
    }
}

/// `v1:net:send_usercmd` — queue CLC::UserCmd packet (payload = 64-byte UserCmd).
pub extern "C" fn tramp_net_send_usercmd(engine: *mut u8, ptr: *const u8, len: usize) -> i32 {
    if ptr.is_null() || len < USERCMD_SIZE {
        return -1;
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, USERCMD_SIZE) };
    let Some(cmd) = UserCmd::decode(bytes) else {
        return -1;
    };
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let packet = cmd.encode_clc_packet().to_vec();
    if host.net.is_client() {
        if !host.client_in_play() {
            return 0;
        }
        host.net.send_c2s(packet);
    } else {
        host.channel.push_client_bytes(packet);
    }
    0
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:net:send_usercmd",
        callback: tramp_net_send_usercmd as *const (),
        flags: 0,
    }
}

/// `v1:players:get_usercmd(slot, out_ptr, out_len)` — copy latest UserCmd (64 bytes). 0=ok.
pub extern "C" fn tramp_players_get_usercmd(
    engine: *mut u8,
    slot: u32,
    out_ptr: *mut u8,
    out_len: usize,
) -> i32 {
    if out_ptr.is_null() || out_len < USERCMD_SIZE {
        return -1;
    }
    let host = unsafe { &*(engine as *const crate::Host) };
    let Some(cmd) = host.players.usercmd(slot) else {
        return -1;
    };
    let bytes = cmd.encode();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_ptr, USERCMD_SIZE);
    }
    0
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:players:get_usercmd",
        callback: tramp_players_get_usercmd as *const (),
        flags: 0,
    }
}

/// `v1:move:snapshot(out_axes_ptr, axis_count, out_buttons_ptr)` — current +forward state.
pub extern "C" fn tramp_move_snapshot(
    _engine: *mut u8,
    axes_ptr: *mut f32,
    axis_count: usize,
    buttons_ptr: *mut u64,
) -> i32 {
    if axes_ptr.is_null() || buttons_ptr.is_null() || axis_count == 0 {
        return -1;
    }
    let st = move_state::global();
    let axes = st.axes();
    let n = axis_count.min(axes.len());
    unsafe {
        std::ptr::copy_nonoverlapping(axes.as_ptr(), axes_ptr, n);
        *buttons_ptr = st.buttons;
    }
    0
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:move:snapshot",
        callback: tramp_move_snapshot as *const (),
        flags: 0,
    }
}

/// `v1:players:apply_local_view(yaw, pitch, roll, eye_height, fov_y)` — client view stamp.
pub extern "C" fn tramp_players_apply_local_view(
    engine: *mut u8,
    yaw: f32,
    pitch: f32,
    roll: f32,
    eye_height: f32,
    fov_y: f32,
) -> i32 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    host.apply_local_view(yaw, pitch, roll, eye_height, fov_y);
    0
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:players:apply_local_view",
        callback: tramp_players_apply_local_view as *const (),
        flags: 0,
    }
}

/// `v1:players:local_entity` → pawn entity id, or 0 if unbound.
pub extern "C" fn tramp_players_local_entity(engine: *mut u8) -> u64 {
    let host = unsafe { &*(engine as *const crate::Host) };
    let Some(slot) = host.players.local_slot() else {
        return 0;
    };
    host.players
        .get(slot)
        .and_then(|p| p.entity)
        .unwrap_or(0)
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:players:local_entity",
        callback: tramp_players_local_entity as *const (),
        flags: 0,
    }
}

/// `v1:um:register(name, size)` → msgid (>0) or -1.
pub extern "C" fn tramp_um_register(
    engine: *mut u8,
    name_ptr: *const u8,
    name_len: usize,
    size: i16,
) -> i32 {
    let name = read_str(name_ptr, name_len);
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    match host.um.register(&name, size) {
        Ok(id) => id as i32,
        Err(e) => {
            log::warn!("[um] register: {e}");
            -1
        }
    }
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:um:register",
        callback: tramp_um_register as *const (),
        flags: 0,
    }
}

/// `v1:um:send(dest, name, data)` — dest 0 = broadcast; dest = slot+1 for one player.
pub extern "C" fn tramp_um_send(
    engine: *mut u8,
    dest: i32,
    name_ptr: *const u8,
    name_len: usize,
    data_ptr: *const u8,
    data_len: usize,
) -> i32 {
    let name = read_str(name_ptr, name_len);
    let data = if data_ptr.is_null() || data_len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(data_ptr, data_len) }
    };
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let slot = if dest <= 0 {
        None
    } else {
        Some((dest - 1) as u32)
    };
    match host.um_send(&name, data, slot) {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("[um] send: {e}");
            -1
        }
    }
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:um:send",
        callback: tramp_um_send as *const (),
        flags: 0,
    }
}

/// `v1:um:hook(name, callback)` — client-side MsgFunc.
pub extern "C" fn tramp_um_hook(
    engine: *mut u8,
    name_ptr: *const u8,
    name_len: usize,
    callback: UmHookFn,
) -> i32 {
    let name = read_str(name_ptr, name_len);
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    match host.um.hook(&name, callback) {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("[um] hook: {e}");
            -1
        }
    }
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:um:hook",
        callback: tramp_um_hook as *const (),
        flags: 0,
    }
}

/// `v1:net:player_net_id(slot)` → net_id or 0.
pub extern "C" fn tramp_net_player_net_id(engine: *mut u8, slot: u32) -> u32 {
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    {
        let host = unsafe { &*(engine as *const crate::Host) };
        return host.player_net_id(slot).unwrap_or(0) as u32;
    }
    #[cfg(not(any(feature = "dedicated-server", feature = "client", feature = "full")))]
    {
        let _ = (engine, slot);
        0
    }
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:net:player_net_id",
        callback: tramp_net_player_net_id as *const (),
        flags: 0,
    }
}

/// `v1:client:set_view_net_id(net_id)`
pub extern "C" fn tramp_client_set_view_net_id(engine: *mut u8, net_id: u16) -> i32 {
    #[cfg(any(feature = "client", feature = "full"))]
    {
        let host = unsafe { &mut *(engine as *mut crate::Host) };
        host.client_set_view_net_id(net_id);
        return 0;
    }
    #[cfg(not(any(feature = "client", feature = "full")))]
    {
        let _ = (engine, net_id);
        -1
    }
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:client:set_view_net_id",
        callback: tramp_client_set_view_net_id as *const (),
        flags: 0,
    }
}

/// `v1:client:set_lightstyle(style, pattern)`
pub extern "C" fn tramp_client_set_lightstyle(
    engine: *mut u8,
    style: u8,
    pat_ptr: *const u8,
    pat_len: usize,
) -> i32 {
    #[cfg(any(feature = "client", feature = "full"))]
    {
        let pat = read_str(pat_ptr, pat_len);
        let host = unsafe { &mut *(engine as *mut crate::Host) };
        host.client_set_lightstyle(style, &pat);
        return 0;
    }
    #[cfg(not(any(feature = "client", feature = "full")))]
    {
        let _ = (engine, style, pat_ptr, pat_len);
        -1
    }
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:client:set_lightstyle",
        callback: tramp_client_set_lightstyle as *const (),
        flags: 0,
    }
}

