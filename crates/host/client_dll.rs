//! Client gamelib DLL (`client_init` / `client_frame` / `client_imgui_frame` / `client_shutdown`).

pub type ResolverFn = extern "C" fn(*const u8, usize) -> *const ();

#[repr(C)]
pub struct ClientInitData {
    pub version: u32,
    pub ctx: *mut u8,
    pub resolver: ResolverFn,
}

const CLIENT_API_VERSION: u32 = 1;

type ClientInitFn = unsafe extern "C" fn(ClientInitData);
type ClientFrameFn = unsafe extern "C" fn(f32);
/// Called between host `imgui_begin_frame` and render (current ImGui context).
type ClientImguiFrameFn = unsafe extern "C" fn(*mut std::ffi::c_void);
type ClientShutdownFn = unsafe extern "C" fn();
type ClientAudioWorldSceneFn = unsafe extern "C" fn(u64);

pub struct ClientDll {
    _lib: libloading::Library,
    init_fn: ClientInitFn,
    frame_fn: ClientFrameFn,
    imgui_frame_fn: Option<ClientImguiFrameFn>,
    audio_world_scene_fn: Option<ClientAudioWorldSceneFn>,
    shutdown_fn: ClientShutdownFn,
}

impl ClientDll {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let lib = unsafe { libloading::Library::new(path) }?;
        let init_fn = unsafe { *lib.get::<ClientInitFn>(b"client_init")? };
        let frame_fn = unsafe { *lib.get::<ClientFrameFn>(b"client_frame")? };
        let imgui_frame_fn = unsafe {
            lib.get::<ClientImguiFrameFn>(b"client_imgui_frame")
                .ok()
                .map(|s| *s)
        };
        let audio_world_scene_fn = unsafe {
            lib.get::<ClientAudioWorldSceneFn>(b"client_audio_world_scene")
                .ok()
                .map(|s| *s)
        };
        let shutdown_fn = unsafe { *lib.get::<ClientShutdownFn>(b"client_shutdown")? };
        Ok(Self {
            _lib: lib,
            init_fn,
            frame_fn,
            imgui_frame_fn,
            audio_world_scene_fn,
            shutdown_fn,
        })
    }

    pub unsafe fn init(&self, ctx: *mut u8, resolver: ResolverFn) {
        let data = ClientInitData {
            version: CLIENT_API_VERSION,
            ctx,
            resolver,
        };
        (self.init_fn)(data);
    }

    pub unsafe fn frame(&self, dt: f32) {
        (self.frame_fn)(dt);
    }

    /// Gamelib ImGui overlays — only between host `imgui_begin_frame` and render.
    pub unsafe fn imgui_frame(&self, imgui_ctx: *mut std::ffi::c_void) {
        if let Some(f) = self.imgui_frame_fn {
            f(imgui_ctx);
        }
    }

    pub fn audio_world_scene_fn(&self) -> Option<ClientAudioWorldSceneFn> {
        self.audio_world_scene_fn
    }

    pub unsafe fn shutdown(&self) {
        (self.shutdown_fn)();
    }
}
