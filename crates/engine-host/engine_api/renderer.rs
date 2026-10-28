//! `v1:renderer:*` — renderer FFI methods, colocated with the renderer bridge.
//!
//! Existing methods (load_texture) are in engine_methoddb.rs (to be moved in TASK-009).
//! New methods are colocated here.

use deerpunk_engine_macro as methoddb;

#[cfg(any(feature = "client", feature = "full"))]
#[methoddb::namespace(name = "v1:renderer", on_host, scope = ["server", "client"])]
impl crate::Host {
    /// Upload a raw RGBA texture (atlas built in memory by gamelib).
    /// Returns MeshTexture slot (0 = failed).
    #[methoddb::bind(name = "upload_texture_raw")]
    pub fn api_renderer_upload_texture_raw(&mut self, pixels: &[u8], w: u32, h: u32) -> u32 {
        let Some(ce) = self.client_engine.as_mut() else {
            return 0;
        };
        ce.renderer_mut().upload_texture(pixels, w, h).0
    }

    /// Load a lightmap atlas via [`ResourceManager`] and upload to the GPU.
    /// URI example: `file://maps/foo.bsp#lightmap:world` (gamelib BSP loader).
    #[methoddb::bind(name = "upload_lightmap_uri")]
    pub fn api_renderer_upload_lightmap_uri(&mut self, uri: &str) -> u32 {
        use deerpunk_engine_common::lightmap_atlas::LightmapAtlasLayers;
        let handle = self.resources.load::<LightmapAtlasLayers>(uri);
        let atlas = match handle.try_get() {
            Some(a) => a,
            None => {
                log::error!("[renderer] upload_lightmap_uri '{uri}': not loaded");
                return 0;
            }
        };
        let Some(ce) = self.client_engine.as_mut() else {
            return 0;
        };
        ce.renderer_mut().upload_lightmap_layers(
            atlas.width,
            atlas.height,
            [
                &atlas.layers[0].pixels,
                &atlas.layers[1].pixels,
                &atlas.layers[2].pixels,
                &atlas.layers[3].pixels,
            ],
        )
    }

    /// Upload a 4-layer lightmap atlas (RGBA8 each). Returns lightmap id (0 = failed).
    #[methoddb::bind(name = "upload_lightmap_raw")]
    pub fn api_renderer_upload_lightmap_raw(
        &mut self,
        layer0: &[u8],
        layer1: &[u8],
        layer2: &[u8],
        layer3: &[u8],
        w: u32,
        h: u32,
    ) -> u32 {
        let Some(ce) = self.client_engine.as_mut() else {
            return 0;
        };
        ce.renderer_mut()
            .upload_lightmap_layers(w, h, [layer0, layer1, layer2, layer3])
    }

    /// Set scene-level lighting (sun direction + ambient color).
    /// sun_dir_bytes = 16 bytes (4×f32: x, y, z, intensity; intensity 0 = sun off).
    /// ambient_bytes = 16 bytes (4×f32: r, g, b, _).
    #[methoddb::bind(name = "set_scene_lighting")]
    pub fn api_renderer_set_scene_lighting(
        &mut self,
        sun_dir_bytes: &[u8],
        ambient_bytes: &[u8],
    ) -> i32 {
        let Some(ce) = self.client_engine.as_mut() else {
            return -1;
        };
        if sun_dir_bytes.len() < 16 || ambient_bytes.len() < 16 {
            return -1;
        }
        let sd: [f32; 4] = unsafe { std::ptr::read(sun_dir_bytes.as_ptr() as *const [f32; 4]) };
        let am: [f32; 4] = unsafe { std::ptr::read(ambient_bytes.as_ptr() as *const [f32; 4]) };
        ce.renderer_mut().set_scene_lighting(sd, am);
        0
    }

    /// `bytes` = 16 bytes: day_k (f32), zenith rgb (3×f32).
    #[methoddb::bind(name = "set_sky_palette")]
    pub fn api_renderer_set_sky_palette(&mut self, bytes: &[u8]) -> i32 {
        let Some(ce) = self.client_engine.as_mut() else {
            return -1;
        };
        if bytes.len() < 16 {
            return -1;
        }
        let day_k = f32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let zx = f32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let zy = f32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let zz = f32::from_le_bytes(bytes[12..16].try_into().unwrap());
        ce.renderer_mut().set_sky_palette(day_k, [zx, zy, zz]);
        0
    }

    /// Enable procedural sky pass (`gamedir/shaders/sky.wgsl`).
    #[methoddb::bind(name = "set_procedural_sky")]
    pub fn api_renderer_set_procedural_sky(&mut self, enable: i32) -> i32 {
        let Some(ce) = self.client_engine.as_mut() else {
            return -1;
        };
        ce.set_procedural_sky(enable != 0);
        0
    }

    /// Gamelib sky colors (39×f32 LE). Fewer than 156 bytes clears the payload.
    #[methoddb::bind(name = "set_gamelib_sky")]
    pub fn api_renderer_set_gamelib_sky(&mut self, bytes: &[u8]) -> i32 {
        let Some(ce) = self.client_engine.as_mut() else {
            return -1;
        };
        const NEED: usize = 39 * 4;
        if bytes.len() < NEED {
            ce.set_gamelib_sky(None);
            return 0;
        }
        let mut f = [0f32; 39];
        for i in 0..39 {
            f[i] = f32::from_le_bytes(bytes[i * 4..(i + 1) * 4].try_into().unwrap());
        }
        let mut params = [0f32; 32];
        params.copy_from_slice(&f[..32]);
        ce.set_gamelib_sky(Some(deerpunk_engine_client::client_engine::GamelibSkyPayload {
            params,
            horizon: [f[32], f[33], f[34]],
            zenith: [f[35], f[36], f[37]],
            sun_glow: f[38],
        }));
        0
    }

    /// Load a `.wgsl` shader into the ShaderRegistry. Returns ShaderId (0 = failed).
    ///
    /// Hot-reload all WGSL from gamedir (built-in passes + material shaders already in the registry).
    #[methoddb::bind(name = "reload_shaders")]
    pub fn api_renderer_reload_shaders(&mut self) -> i32 {
        let Some(ce) = self.client_engine.as_mut() else {
            return -1;
        };
        match ce.reload_shaders(&self.resources) {
            Ok(()) => 0,
            Err(e) => {
                log::error!("[renderer] reload_shaders: {e}");
                -1
            }
        }
    }

    /// `path` — `file://` URI used as cache key (e.g. `"file://shaders/mesh.wgsl"`).
    /// `defines` — comma-joined preprocessor defines (e.g. `"ALPHA_MASK"` or `""`).
    #[methoddb::bind(name = "load_shader")]
    pub fn api_renderer_load_shader(&mut self, path: &str, defines: &str) -> u32 {
        let Some(ce) = self.client_engine.as_mut() else {
            return 0;
        };
        use deerpunk_engine_common::resource::ShaderSource;
        let handle = self.resources.load::<ShaderSource>(path);
        let source = match handle.try_get() {
            Some(s) => s.wgsl,
            None => {
                log::error!(
                    "[renderer] load_shader '{path}': not loaded (asset={:?})",
                    handle.id()
                );
                return 0;
            }
        };
        let defines_vec: Vec<String> = if defines.is_empty() {
            Vec::new()
        } else {
            defines.split(',').map(|s| s.trim().to_string()).collect()
        };
        let content_root = self.resources.base_dir();
        match ce.shader_registry.load(path, &defines_vec, &source, &content_root) {
            Ok(id) => id,
            Err(e) => {
                log::error!("[renderer] load_shader '{path}': {e}");
                0
            }
        }
    }
}

#[cfg(not(any(feature = "client", feature = "full")))]
#[methoddb::namespace(name = "v1:renderer", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "upload_texture_raw")]
    pub fn api_renderer_upload_texture_raw(&mut self, _pixels: &[u8], _w: u32, _h: u32) -> u32 {
        0
    }

    #[methoddb::bind(name = "upload_lightmap_uri")]
    pub fn api_renderer_upload_lightmap_uri(&mut self, _uri: &str) -> u32 {
        0
    }

    #[methoddb::bind(name = "upload_lightmap_raw")]
    pub fn api_renderer_upload_lightmap_raw(
        &mut self,
        _l0: &[u8],
        _l1: &[u8],
        _l2: &[u8],
        _l3: &[u8],
        _w: u32,
        _h: u32,
    ) -> u32 {
        0
    }

    #[methoddb::bind(name = "set_scene_lighting")]
    pub fn api_renderer_set_scene_lighting(
        &mut self,
        _sun_dir_bytes: &[u8],
        _ambient_bytes: &[u8],
    ) -> i32 {
        0
    }

    #[methoddb::bind(name = "set_sky_palette")]
    pub fn api_renderer_set_sky_palette(&mut self, _bytes: &[u8]) -> i32 {
        0
    }

    #[methoddb::bind(name = "set_procedural_sky")]
    pub fn api_renderer_set_procedural_sky(&mut self, _enable: i32) -> i32 {
        -1
    }

    #[methoddb::bind(name = "set_gamelib_sky")]
    pub fn api_renderer_set_gamelib_sky(&mut self, _bytes: &[u8]) -> i32 {
        -1
    }

    #[methoddb::bind(name = "load_shader")]
    pub fn api_renderer_load_shader(&mut self, _path: &str, _defines: &str) -> u32 {
        0
    }

    #[methoddb::bind(name = "reload_shaders")]
    pub fn api_renderer_reload_shaders(&mut self) -> i32 {
        -1
    }
}