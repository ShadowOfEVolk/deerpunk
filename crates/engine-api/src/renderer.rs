use std::ffi::c_void;

use crate::resolve_fn;

pub struct RendererApiResolved {
    ctx: *mut u8,
    load_texture_fn: unsafe extern "C" fn(*mut u8, *const u8, usize) -> u32,
    upload_texture_raw_fn: unsafe extern "C" fn(*mut u8, *const u8, usize, u32, u32) -> u32,
    upload_lightmap_uri_fn: unsafe extern "C" fn(*mut u8, *const u8, usize) -> u32,
    upload_lightmap_raw_fn: unsafe extern "C" fn(
        *mut u8,
        *const u8, usize,
        *const u8, usize,
        *const u8, usize,
        *const u8, usize,
        u32,
        u32,
    ) -> u32,
    set_scene_lighting_fn: unsafe extern "C" fn(*mut u8, *const u8, usize, *const u8, usize) -> i32,
    set_sky_palette_fn: unsafe extern "C" fn(*mut u8, *const u8, usize) -> i32,
    set_procedural_sky_fn: unsafe extern "C" fn(*mut u8, i32) -> i32,
    set_gamelib_sky_fn: unsafe extern "C" fn(*mut u8, *const u8, usize) -> i32,
    load_shader_fn: unsafe extern "C" fn(*mut u8, *const u8, usize, *const u8, usize) -> u32,
    reload_shaders_fn: unsafe extern "C" fn(*mut u8) -> i32,
}

impl RendererApiResolved {
    pub fn new(ctx: *mut u8, resolver: crate::Resolver) -> Self {
        Self {
            ctx,
            load_texture_fn: unsafe {
                std::mem::transmute(resolve_fn(resolver, "v1:renderer:load_texture"))
            },
            upload_texture_raw_fn: unsafe {
                std::mem::transmute(resolve_fn(resolver, "v1:renderer:upload_texture_raw"))
            },
            upload_lightmap_uri_fn: unsafe {
                std::mem::transmute(resolve_fn(resolver, "v1:renderer:upload_lightmap_uri"))
            },
            upload_lightmap_raw_fn: unsafe {
                std::mem::transmute(resolve_fn(resolver, "v1:renderer:upload_lightmap_raw"))
            },
            set_scene_lighting_fn: unsafe {
                std::mem::transmute(resolve_fn(resolver, "v1:renderer:set_scene_lighting"))
            },
            set_sky_palette_fn: unsafe {
                std::mem::transmute(resolve_fn(resolver, "v1:renderer:set_sky_palette"))
            },
            set_procedural_sky_fn: unsafe {
                std::mem::transmute(resolve_fn(resolver, "v1:renderer:set_procedural_sky"))
            },
            set_gamelib_sky_fn: unsafe {
                std::mem::transmute(resolve_fn(resolver, "v1:renderer:set_gamelib_sky"))
            },
            load_shader_fn: unsafe {
                std::mem::transmute(resolve_fn(resolver, "v1:renderer:load_shader"))
            },
            reload_shaders_fn: unsafe {
                std::mem::transmute(resolve_fn(resolver, "v1:renderer:reload_shaders"))
            },
        }
    }

    pub fn load_texture(&self, path: &str) -> u32 {
        let bytes = path.as_bytes();
        unsafe { (self.load_texture_fn)(self.ctx, bytes.as_ptr(), bytes.len()) }
    }

    /// Upload a raw RGBA texture (atlas built in memory). Returns MeshTexture slot (0 = failed).
    pub fn upload_texture_raw(&self, pixels: &[u8], w: u32, h: u32) -> u32 {
        unsafe {
            (self.upload_texture_raw_fn)(self.ctx, pixels.as_ptr(), pixels.len(), w, h)
        }
    }

    /// Load `file://…#lightmap:…` via ResourceManager and upload. Returns GPU lightmap id.
    pub fn upload_lightmap_uri(&self, uri: &str) -> u32 {
        let bytes = uri.as_bytes();
        unsafe {
            (self.upload_lightmap_uri_fn)(self.ctx, bytes.as_ptr(), bytes.len())
        }
    }

    /// Upload a 4-layer lightmap atlas (RGBA8 each). Returns lightmap id (0 = failed).
    pub fn upload_lightmap_raw(
        &self,
        layers: [&[u8]; 4],
        w: u32,
        h: u32,
    ) -> u32 {
        unsafe {
            (self.upload_lightmap_raw_fn)(
                self.ctx,
                layers[0].as_ptr(),
                layers[0].len(),
                layers[1].as_ptr(),
                layers[1].len(),
                layers[2].as_ptr(),
                layers[2].len(),
                layers[3].as_ptr(),
                layers[3].len(),
                w,
                h,
            )
        }
    }

    /// Set scene-level lighting (sun direction + ambient color).
    /// `sun_dir` = [x, y, z, intensity] (intensity 0 = sun off). Passed as raw bytes.
    /// `ambient` = [r, g, b, _]. Passed as raw bytes.
    /// Fullscreen procedural sky (`gamedir/shaders/sky.wgsl`). Not a camera material.
    pub fn set_procedural_sky(&self, enable: bool) -> i32 {
        unsafe { (self.set_procedural_sky_fn)(self.ctx, if enable { 1 } else { 0 }) }
    }

    /// Hosek colors from gamelib: 156 bytes (39×f32 LE). Empty slice clears the payload.
    pub fn set_gamelib_sky(&self, bytes: &[u8]) -> i32 {
        unsafe {
            (self.set_gamelib_sky_fn)(self.ctx, bytes.as_ptr(), bytes.len())
        }
    }

    pub fn set_scene_lighting(&self, sun_dir: &[f32; 4], ambient: &[f32; 4]) -> i32 {
        // Cast f32 arrays to raw bytes (16 bytes each) without bytemuck dep.
        let sd_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(sun_dir.as_ptr() as *const u8, 16)
        };
        let am_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(ambient.as_ptr() as *const u8, 16)
        };
        unsafe {
            (self.set_scene_lighting_fn)(
                self.ctx,
                sd_bytes.as_ptr(),
                sd_bytes.len(),
                am_bytes.as_ptr(),
                am_bytes.len(),
            )
        }
    }

    /// Sky gradient palette (day_k + zenith rgb) — must match `ohmyblock_shared_net::daytime`.
    pub fn set_sky_palette(&self, day_k: f32, zenith: &[f32; 3]) -> i32 {
        let mut bytes = [0u8; 16];
        bytes[0..4].copy_from_slice(&day_k.to_le_bytes());
        bytes[4..8].copy_from_slice(&zenith[0].to_le_bytes());
        bytes[8..12].copy_from_slice(&zenith[1].to_le_bytes());
        bytes[12..16].copy_from_slice(&zenith[2].to_le_bytes());
        unsafe {
            (self.set_sky_palette_fn)(self.ctx, bytes.as_ptr(), bytes.len())
        }
    }

    /// Load a `.wgsl` shader into the ShaderRegistry. Returns ShaderId (0 = failed).
    ///
    /// `path` — `file://` URI cache key (e.g. `"file://shaders/mesh.wgsl"`).
    /// `defines` — comma-joined preprocessor defines (e.g. `"ALPHA_MASK"` or `""`).
    pub fn load_shader(&self, path: &str, defines: &str) -> u32 {
        let p = path.as_bytes();
        let d = defines.as_bytes();
        unsafe {
            (self.load_shader_fn)(self.ctx, p.as_ptr(), p.len(), d.as_ptr(), d.len())
        }
    }

    /// Hot-reload WGSL from gamedir (sky, postfx, materials already in the registry).
    pub fn reload_shaders(&self) -> i32 {
        unsafe { (self.reload_shaders_fn)(self.ctx) }
    }
}