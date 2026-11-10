//! `v1:world:spawn_prefab` — FFI opts parsing.

use std::collections::HashMap;

use deerpunk_engine_api::spawn_prefab::SpawnPrefabOpts;
use deerpunk_engine_common::scene::MapSpawnOpts;
use glam::Vec3;

use crate::engine_api::resources::read_str;

pub(crate) fn map_spawn_opts_from_ffi(
    opts: &SpawnPrefabOpts,
) -> Result<Option<MapSpawnOpts>, String> {
    let o = opts;
    let origin = if o.has_origin != 0 {
        Some(Vec3::from_array(o.origin))
    } else {
        None
    };
    let angles = if o.has_angles != 0 {
        Some(Vec3::from_array(o.angles))
    } else {
        None
    };
    let mut kv = HashMap::new();
    if !o.kv.is_null() && o.kv_count > 0 {
        let pairs = unsafe { std::slice::from_raw_parts(o.kv, o.kv_count as usize) };
        for p in pairs {
            let key = read_str(p.key_ptr, p.key_len);
            let val = read_str(p.val_ptr, p.val_len);
            if !key.is_empty() {
                kv.insert(key, val);
            }
        }
    }
    if origin.is_none() && angles.is_none() && kv.is_empty() {
        return Ok(None);
    }
    Ok(Some(MapSpawnOpts { origin, angles, kv }))
}

// `spawn_prefab` moved to `ServerThreadCtx` (the server world lives on the
// sim thread); only the FFI opts parsing stays here.
