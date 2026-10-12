//! `v1:resources:*` — gamelib registers opaque loaders on the host [`ResourceManager`].

use deerpunk_engine_api::resources::ResourceLoaderDesc;

use crate::engine_api::engine_api::EngineApiMethod;

use super::callback_loader::{CallbackLoader, DropFn, LoadFn};

pub(crate) fn read_str(ptr: *const u8, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    unsafe {
        let bytes = std::slice::from_raw_parts(ptr, len);
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn parse_extensions(s: &str) -> Vec<String> {
    s.split([',', ';', ' '])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| p.trim_start_matches('.').to_ascii_lowercase())
        .collect()
}

/// `v1:resources:register`
pub extern "C" fn tramp_resources_register(engine: *mut u8, desc: *const ResourceLoaderDesc) -> i32 {
    if desc.is_null() {
        return -1;
    }
    let desc = unsafe { &*desc };
    if desc.load as usize == 0 || desc.drop_fn as usize == 0 {
        return -1;
    }
    let id = read_str(desc.id_ptr, desc.id_len);
    if id.is_empty() {
        return -1;
    }
    let ext_src = read_str(desc.extensions_ptr, desc.extensions_len);
    let extensions = parse_extensions(&ext_src);
    let type_tag = read_str(desc.type_tag_ptr, desc.type_tag_len);
    if type_tag.is_empty() {
        return -1;
    }

    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let loader = CallbackLoader::new(
        id.clone(),
        extensions,
        type_tag,
        desc.user_data,
        desc.load,
        desc.drop_fn,
    );
    host.resources.register_loader(Box::new(loader));
    log::info!("[resources] registered loader '{id}'");
    0
}

inventory::submit! {
    EngineApiMethod {
        name: "v1:resources:register",
        callback: tramp_resources_register as *const (),
        flags: 0,
    }
}
