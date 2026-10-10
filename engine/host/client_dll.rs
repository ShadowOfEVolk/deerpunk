//! Client gamelib DLL (`client_init` / `client_frame` / `client_shutdown`).

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
type ClientShutdownFn = unsafe extern "C" fn();

pub struct ClientDll {
    _lib: libloading::Library,
    init_fn: ClientInitFn,
    frame_fn: ClientFrameFn,
    shutdown_fn: ClientShutdownFn,
}

impl ClientDll {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let lib = unsafe { libloading::Library::new(path) }?;
        let init_fn = unsafe { *lib.get::<ClientInitFn>(b"client_init")? };
        let frame_fn = unsafe { *lib.get::<ClientFrameFn>(b"client_frame")? };
        let shutdown_fn = unsafe { *lib.get::<ClientShutdownFn>(b"client_shutdown")? };
        Ok(Self {
            _lib: lib,
            init_fn,
            frame_fn,
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

    pub unsafe fn shutdown(&self) {
        (self.shutdown_fn)();
    }
}
