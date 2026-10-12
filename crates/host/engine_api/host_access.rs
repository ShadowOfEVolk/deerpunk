//! Gamelib access to server/client world + ResourceManager.

use std::ffi::c_void;

use crate::engine_api::engine_api::EngineApiMethod;

pub extern "C" fn tramp_host_with_server(
    engine: *mut u8,
    callback: extern "C" fn(*mut c_void, *mut c_void),
    user_data: *mut c_void,
) {
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    {
        let host = unsafe { &mut *(engine as *mut crate::Host) };
        let gamedir = host.fs.gamedir();
        let gamedir_bytes = gamedir.as_os_str().as_encoded_bytes();
        let mut access = deerpunk_engine_api::host_access::ServerHostAccess {
            world: &mut host.server_engine.world as *mut _ as *mut c_void,
            resources: &mut host.resources as *mut _ as *mut c_void,
            gamedir_ptr: gamedir_bytes.as_ptr(),
            gamedir_len: gamedir_bytes.len(),
        };
        callback(&mut access as *mut _ as *mut c_void, user_data);
    }
    #[cfg(not(any(feature = "dedicated-server", feature = "client", feature = "full")))]
    {
        let _ = (engine, callback, user_data);
    }
}

pub extern "C" fn tramp_host_with_client(
    engine: *mut u8,
    callback: extern "C" fn(*mut c_void, *mut c_void),
    user_data: *mut c_void,
) {
    #[cfg(any(feature = "client", feature = "full"))]
    {
        let host = unsafe { &mut *(engine as *mut crate::Host) };
        let Some(client) = host.client_engine.as_mut() else {
            return;
        };
        let gamedir = host.fs.gamedir();
        let gamedir_bytes = gamedir.as_os_str().as_encoded_bytes();
        let mut access = deerpunk_engine_api::host_access::ClientHostAccess {
            world: &mut client.world as *mut _ as *mut c_void,
            resources: &mut host.resources as *mut _ as *mut c_void,
            gamedir_ptr: gamedir_bytes.as_ptr(),
            gamedir_len: gamedir_bytes.len(),
        };
        callback(&mut access as *mut _ as *mut c_void, user_data);
    }
    #[cfg(not(any(feature = "client", feature = "full")))]
    {
        let _ = (engine, callback, user_data);
    }
}

pub extern "C" fn tramp_host_flush_pending_albedos(engine: *mut u8) -> i32 {
    #[cfg(any(feature = "client", feature = "full"))]
    {
        let host = unsafe { &mut *(engine as *mut crate::Host) };
        host.flush_pending_albedos();
        0
    }
    #[cfg(not(any(feature = "client", feature = "full")))]
    {
        let _ = engine;
        -1
    }
}

pub extern "C" fn tramp_host_client_track_static(engine: *mut u8, entity: u64) -> i32 {
    #[cfg(any(feature = "client", feature = "full"))]
    {
        let host = unsafe { &mut *(engine as *mut crate::Host) };
        host.client_rep.track_static_entity(entity);
        0
    }
    #[cfg(not(any(feature = "client", feature = "full")))]
    {
        let _ = (engine, entity);
        -1
    }
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:host:with_server",
        callback: tramp_host_with_server as *const (),
        flags: 0,
    }
}
inventory::submit! {
    EngineApiMethod {
        name: "v1:host:with_client",
        callback: tramp_host_with_client as *const (),
        flags: 0,
    }
}
inventory::submit! {
    EngineApiMethod {
        name: "v1:host:flush_pending_albedos",
        callback: tramp_host_flush_pending_albedos as *const (),
        flags: 0,
    }
}
inventory::submit! {
    EngineApiMethod {
        name: "v1:host:client_track_static",
        callback: tramp_host_client_track_static as *const (),
        flags: 0,
    }
}
