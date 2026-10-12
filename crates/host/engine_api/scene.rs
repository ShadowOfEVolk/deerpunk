use crate::engine_api::engine_api::EngineApiMethod;

/// `v1:scene:load` — load `{gamedir}/scenes/{name}.scene`. Returns 0 on success, -1 on error.
pub extern "C" fn tramp_scene_load(
    engine: *mut u8,
    name_ptr: *const u8,
    name_len: usize,
) -> i32 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let name = unsafe {
        let bytes = std::slice::from_raw_parts(name_ptr, name_len);
        std::str::from_utf8(bytes).unwrap_or("")
    };
    if name.is_empty() {
        log::error!("[scene] load: empty name");
        return -1;
    }
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    {
        match host.load_scene(name) {
            Ok(_) => 0,
            Err(e) => {
                log::error!("[scene] load {name}: {e}");
                -1
            }
        }
    }
    #[cfg(not(any(feature = "dedicated-server", feature = "client", feature = "full")))]
    {
        let _ = host;
        let _ = name;
        -1
    }
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:scene:load",
        callback: tramp_scene_load as *const (),
        flags: 0,
    }
}

/// `v1:scene:spawn_prefab` — spawn `{gamedir}/prefabs/{name}.prefab`. Returns entity id, or 0.
pub extern "C" fn tramp_scene_spawn_prefab(
    engine: *mut u8,
    name_ptr: *const u8,
    name_len: usize,
) -> u64 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let name = unsafe {
        let bytes = std::slice::from_raw_parts(name_ptr, name_len);
        std::str::from_utf8(bytes).unwrap_or("")
    };
    if name.is_empty() {
        return 0;
    }
    match host.spawn_prefab(name) {
        Ok(id) => id,
        Err(e) => {
            log::error!("[scene] spawn_prefab {name}: {e}");
            0
        }
    }
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:scene:spawn_prefab",
        callback: tramp_scene_spawn_prefab as *const (),
        flags: 0,
    }
}

