//! `v1:resources:load_material` / `is_loaded`.

use super::materials::materials_load_asset;

pub(crate) fn resources_load_material(host: &mut crate::Host, path: &str) -> u64 {
    materials_load_asset(host, path)
}

pub(crate) fn resources_is_loaded(host: &crate::Host, asset_id: u64) -> i32 {
    if asset_id == 0 {
        return 0;
    }
    if host
        .resources
        .is_loaded(deerpunk_engine_common::resource::AssetId(asset_id))
    {
        1
    } else {
        0
    }
}
