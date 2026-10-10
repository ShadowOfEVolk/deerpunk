//! In-process net channel + server enable (`v1:net:*`, `v1:server:*`).

use crate::channel::ClientCmd;
use crate::engine_api::engine_api::EngineApiMethod;

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

/// `v1:server:is_enabled` → 1/0.
pub extern "C" fn tramp_server_is_enabled(engine: *mut u8) -> i32 {
    let host = unsafe { &*(engine as *const crate::Host) };
    if host.server_engine.enabled {
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
