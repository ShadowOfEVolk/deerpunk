//! Client gamelib DLL (`client_init` / `client_frame` / …).

pub use deerpunk_engine_ffi::{ClientInitData, ResolverFn, GAMELIB_API_VERSION};

type ClientInitFn = unsafe extern "C" fn(ClientInitData);
type ClientFrameFn = unsafe extern "C" fn(f32);
type ClientPostFrameFn = unsafe extern "C" fn(f32);
type ClientPreRenderFn = unsafe extern "C" fn(f32);
type ClientGamelibUiBuildFn =
    unsafe extern "C" fn(*const deerpunk_engine_ffi::GamelibUiBuildArgs) -> i32;
type ClientGamelibUiRenderFn =
    unsafe extern "C" fn(*const deerpunk_engine_ffi::GamelibUiRenderArgs) -> i32;
type ClientShutdownFn = unsafe extern "C" fn();
type ClientAudioWorldSceneFn = unsafe extern "C" fn(u64);
type ClientSceneFlushedFn = unsafe extern "C" fn();
type ClientComposeSkyFn = unsafe extern "C" fn(*mut u8, *mut crate::scene::SkySceneCompose);

pub struct ClientDll {
    _lib: libloading::Library,
    init_fn: ClientInitFn,
    frame_fn: ClientFrameFn,
    post_frame_fn: Option<ClientPostFrameFn>,
    pre_render_fn: Option<ClientPreRenderFn>,
    gamelib_ui_build_fn: Option<ClientGamelibUiBuildFn>,
    gamelib_ui_render_fn: Option<ClientGamelibUiRenderFn>,
    audio_world_scene_fn: Option<ClientAudioWorldSceneFn>,
    scene_flushed_fn: Option<ClientSceneFlushedFn>,
    compose_sky_fn: Option<ClientComposeSkyFn>,
    shutdown_fn: ClientShutdownFn,
}

impl ClientDll {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let lib = unsafe { libloading::Library::new(path) }?;
        let init_fn = unsafe { *lib.get::<ClientInitFn>(b"client_init")? };
        let frame_fn = unsafe { *lib.get::<ClientFrameFn>(b"client_frame")? };
        let post_frame_fn = unsafe {
            lib.get::<ClientPostFrameFn>(b"client_post_frame")
                .ok()
                .map(|s| *s)
        };
        let pre_render_fn = unsafe {
            lib.get::<ClientPreRenderFn>(b"client_pre_render")
                .ok()
                .map(|s| *s)
        };
        let gamelib_ui_build_fn = unsafe {
            lib.get::<ClientGamelibUiBuildFn>(b"client_gamelib_ui_build")
                .ok()
                .map(|s| *s)
        };
        let gamelib_ui_render_fn = unsafe {
            lib.get::<ClientGamelibUiRenderFn>(b"client_gamelib_ui_render")
                .ok()
                .map(|s| *s)
        };
        let audio_world_scene_fn = unsafe {
            lib.get::<ClientAudioWorldSceneFn>(b"client_audio_world_scene")
                .ok()
                .map(|s| *s)
        };
        let scene_flushed_fn = unsafe {
            lib.get::<ClientSceneFlushedFn>(b"client_scene_flushed")
                .ok()
                .map(|s| *s)
        };
        let compose_sky_fn = unsafe {
            lib.get::<ClientComposeSkyFn>(b"client_compose_sky")
                .ok()
                .map(|s| *s)
        };
        let shutdown_fn = unsafe { *lib.get::<ClientShutdownFn>(b"client_shutdown")? };
        Ok(Self {
            _lib: lib,
            init_fn,
            frame_fn,
            post_frame_fn,
            pre_render_fn,
            gamelib_ui_build_fn,
            gamelib_ui_render_fn,
            audio_world_scene_fn,
            scene_flushed_fn,
            compose_sky_fn,
            shutdown_fn,
        })
    }

    pub unsafe fn init(&self, ctx: *mut u8, resolver: ResolverFn) {
        let data = ClientInitData {
            version: GAMELIB_API_VERSION,
            ctx,
            resolver,
        };
        (self.init_fn)(data);
    }

    pub unsafe fn frame(&self, dt: f32) {
        (self.frame_fn)(dt);
    }

    pub unsafe fn post_frame(&self, dt: f32) {
        if let Some(f) = self.post_frame_fn {
            f(dt);
        }
    }

    /// After server tick, immediately before scene extract / render (lighting, sky).
    pub unsafe fn pre_render(&self, dt: f32) {
        if let Some(f) = self.pre_render_fn {
            f(dt);
        }
    }

    pub unsafe fn gamelib_ui_build(&self, args: &deerpunk_engine_ffi::GamelibUiBuildArgs) -> i32 {
        if let Some(f) = self.gamelib_ui_build_fn {
            f(args)
        } else {
            0
        }
    }

    pub fn gamelib_ui_render_fn(&self) -> Option<ClientGamelibUiRenderFn> {
        self.gamelib_ui_render_fn
    }

    pub fn audio_world_scene_fn(&self) -> Option<ClientAudioWorldSceneFn> {
        self.audio_world_scene_fn
    }

    pub fn compose_sky_fn(&self) -> Option<ClientComposeSkyFn> {
        self.compose_sky_fn
    }

    pub unsafe fn scene_flushed(&self) {
        if let Some(f) = self.scene_flushed_fn {
            f();
        }
    }

    pub unsafe fn shutdown(&self) {
        (self.shutdown_fn)();
    }
}
