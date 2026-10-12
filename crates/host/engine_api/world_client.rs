//! Client `World` tramps (`v1:client_world:*`) — same ABI as server world API.

use crate::engine_api::engine_api::EngineApiMethod;

pub extern "C" fn tramp_cw_get(engine: *mut u8, entity: u64, comp: u64) -> *mut std::ffi::c_void {
    let host = unsafe { &*(engine as *const crate::Host) };
    let Some(world) = host.client_engine.as_ref().map(|c| &c.world) else {
        return std::ptr::null_mut();
    };
    let ptr = world.get_raw(entity, comp);
    if ptr.is_null() {
        std::ptr::null_mut()
    } else {
        ptr as *mut std::ffi::c_void
    }
}

pub extern "C" fn tramp_cw_set(
    engine: *mut u8,
    entity: u64,
    comp: u64,
    data: *const std::ffi::c_void,
    size: usize,
) {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    if let Some(ce) = host.client_engine.as_mut() {
        ce.world.set_raw(entity, comp, data, size);
    }
}

pub extern "C" fn tramp_cw_has(engine: *mut u8, entity: u64, comp: u64) -> bool {
    let host = unsafe { &*(engine as *const crate::Host) };
    host.client_engine
        .as_ref()
        .map(|c| c.world.has(entity, comp))
        .unwrap_or(false)
}

pub extern "C" fn tramp_cw_spawn(engine: *mut u8) -> u64 {
    let host = unsafe { &*(engine as *const crate::Host) };
    host.client_engine
        .as_ref()
        .map(|c| c.world.spawn())
        .unwrap_or(0)
}

pub extern "C" fn tramp_cw_component_init_raw(
    engine: *mut u8,
    name_ptr: *const u8,
    name_len: usize,
    size: usize,
    align: usize,
) -> u64 {
    let host = unsafe { &*(engine as *const crate::Host) };
    let Some(world) = host.client_engine.as_ref().map(|c| &c.world) else {
        return 0;
    };
    let name = unsafe {
        let bytes = std::slice::from_raw_parts(name_ptr, name_len);
        std::str::from_utf8_unchecked(bytes)
    };
    let desc = crate::world::ComponentDescriptor {
        name: name.to_string(),
        size,
        align,
    };
    world.component_init_raw(&desc)
}

inventory::submit! { EngineApiMethod { name: "v1:client_world:get", callback: tramp_cw_get as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:client_world:set", callback: tramp_cw_set as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:client_world:has", callback: tramp_cw_has as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:client_world:spawn", callback: tramp_cw_spawn as *const (), flags: 0 } }
inventory::submit! {
    EngineApiMethod {
        name: "v1:client_world:component_init_raw",
        callback: tramp_cw_component_init_raw as *const (),
        flags: 0,
    }
}
