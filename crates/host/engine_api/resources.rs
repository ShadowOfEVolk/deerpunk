//! Resource / BSP asset API for gamelib (`v1:resources:*`).

use crate::engine_api::engine_api::EngineApiMethod;

pub(crate) fn read_str(ptr: *const u8, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    unsafe {
        let bytes = std::slice::from_raw_parts(ptr, len);
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// `v1:resources:load_bsp` — load BSP via ResourceManager.
/// Writes `BspMap { id }` into `out`. Returns 0 on success, -1 on error.
pub extern "C" fn tramp_resources_load_bsp(
    engine: *mut u8,
    path_ptr: *const u8,
    path_len: usize,
    out_id: *mut u64,
) -> i32 {
    if out_id.is_null() {
        return -1;
    }
    let path = read_str(path_ptr, path_len);
    if path.trim().is_empty() {
        log::error!("[resources] load_bsp: empty path");
        return -1;
    }
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    {
        let host = unsafe { &mut *(engine as *mut crate::Host) };
        match host.load_bsp(&path) {
            Ok(id) => {
                unsafe { *out_id = id };
                0
            }
            Err(e) => {
                log::error!("[resources] load_bsp {path}: {e}");
                -1
            }
        }
    }
    #[cfg(not(any(feature = "dedicated-server", feature = "client", feature = "full")))]
    {
        let _ = (engine, path, out_id);
        -1
    }
}

/// `v1:resources:place_bsp` — place loaded BSP into world.
/// Returns worldspawn entity id, or 0 on error.
pub extern "C" fn tramp_resources_place_bsp(
    engine: *mut u8,
    asset_id: u64,
    name_ptr: *const u8,
    name_len: usize,
) -> u64 {
    let map_name = read_str(name_ptr, name_len);
    let map_name = if map_name.trim().is_empty() {
        format!("{asset_id}")
    } else {
        map_name
    };
    #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
    {
        let host = unsafe { &mut *(engine as *mut crate::Host) };
        match host.place_bsp(asset_id, &map_name) {
            Ok(entity) => entity,
            Err(e) => {
                log::error!("[resources] place_bsp id={asset_id}: {e}");
                0
            }
        }
    }
    #[cfg(not(any(feature = "dedicated-server", feature = "client", feature = "full")))]
    {
        let _ = (engine, asset_id, map_name);
        0
    }
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:resources:load_bsp",
        callback: tramp_resources_load_bsp as *const (),
        flags: 0,
    }
}
inventory::submit! {
    EngineApiMethod {
        name: "v1:resources:place_bsp",
        callback: tramp_resources_place_bsp as *const (),
        flags: 0,
    }
}
