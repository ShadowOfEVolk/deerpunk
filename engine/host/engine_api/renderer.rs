use crate::engine_api::engine_api::EngineApiMethod;

/// Trampoline: load_texture(path_ptr, path_len) -> u32 (MeshTexture slot).
pub extern "C" fn tramp_load_texture(engine: *mut u8, path_ptr: *const u8, path_len: usize) -> u32 {
    let host = unsafe { &mut *(engine as *mut crate::Host) };
    let path = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(path_ptr, path_len))
    };

    match image::open(path) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let pixels = rgba.into_raw();
            host.client_engine.as_mut().unwrap().renderer_mut().upload_texture(&pixels, w, h).0
        }
        Err(e) => {
            log::error!("[renderer] failed to load texture '{}': {}", path, e);
            0
        }
    }
}

inventory::submit! { EngineApiMethod { name: "v1:renderer:load_texture", callback: tramp_load_texture as *const (), flags: 0 } }
