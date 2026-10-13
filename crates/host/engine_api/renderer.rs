//! `v1:renderer:*` — renderer FFI methods, colocated with the renderer bridge.
//!
//! Existing methods (load_texture) are in engine_methoddb.rs (to be moved in TASK-009).
//! New methods are colocated here.

use deerpunk_macro as methoddb;

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

    /// Load a `.wgsl` shader into the ShaderRegistry. Returns ShaderId (0 = failed).
    ///
    /// `path` — `file://` URI used as cache key (e.g. `"file://shaders/mesh.wgsl"`).
    /// `defines` — comma-joined preprocessor defines (e.g. `"ALPHA_MASK"` or `""`).
    #[methoddb::bind(name = "load_shader")]
    pub fn api_renderer_load_shader(&mut self, path: &str, defines: &str) -> u32 {
        let Some(ce) = self.client_engine.as_mut() else {
            return 0;
        };
        let fs_path = path.strip_prefix("file://").unwrap_or(path);
        let resolved = self.fs.resolve_path(fs_path);
        let source = match std::fs::read_to_string(&resolved) {
            Ok(s) => s,
            Err(e) => {
                log::error!("[renderer] load_shader '{path}': {e}");
                return 0;
            }
        };
        let defines_vec: Vec<String> = if defines.is_empty() {
            Vec::new()
        } else {
            defines.split(',').map(|s| s.trim().to_string()).collect()
        };
        match ce.shader_registry.load(path, &defines_vec, &source) {
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

    #[methoddb::bind(name = "load_shader")]
    pub fn api_renderer_load_shader(&mut self, _path: &str, _defines: &str) -> u32 {
        0
    }
}