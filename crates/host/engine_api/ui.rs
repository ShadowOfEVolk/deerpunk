use crate::engine_api::engine_api::EngineApiMethod;
use crate::engine_api::resources::read_str;

#[cfg(any(feature = "client", feature = "full"))]
use deerpunk_engine_client::client_engine::ClientEngine;

#[cfg(any(feature = "client", feature = "full"))]
fn with_client<F>(engine: *mut u8, f: F) -> i32
where
    F: FnOnce(&mut ClientEngine) -> Result<(), String>,
{
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let Some(client) = host.client_engine.as_mut() else {
        log::error!("[ui] no client engine");
        return -1;
    };
    match f(client) {
        Ok(()) => 0,
        Err(e) => {
            log::error!("[ui] {e}");
            -1
        }
    }
}

#[cfg(not(any(feature = "client", feature = "full")))]
fn with_client(_engine: *mut u8, _f: ()) -> i32 {
    -1
}

/// Load UI document asset into the client widget list (hidden).
pub extern "C" fn tramp_ui_load(engine: *mut u8, id_ptr: *const u8, id_len: usize) -> i32 {
    let id = read_str(id_ptr, id_len);
    with_client(engine, |c| c.ui_load(&id))
}

pub extern "C" fn tramp_ui_unload(engine: *mut u8, id_ptr: *const u8, id_len: usize) -> i32 {
    let id = read_str(id_ptr, id_len);
    with_client(engine, |c| c.ui_unload(&id))
}

pub extern "C" fn tramp_ui_show(engine: *mut u8, id_ptr: *const u8, id_len: usize) -> i32 {
    let id = read_str(id_ptr, id_len);
    with_client(engine, |c| c.ui_show(&id))
}

pub extern "C" fn tramp_ui_hide(engine: *mut u8, id_ptr: *const u8, id_len: usize) -> i32 {
    let id = read_str(id_ptr, id_len);
    with_client(engine, |c| c.ui_hide(&id))
}

inventory::submit! { EngineApiMethod { name: "v1:ui:load", callback: tramp_ui_load as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:ui:unload", callback: tramp_ui_unload as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:ui:show", callback: tramp_ui_show as *const (), flags: 0 } }
inventory::submit! { EngineApiMethod { name: "v1:ui:hide", callback: tramp_ui_hide as *const (), flags: 0 } }
