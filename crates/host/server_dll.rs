use std::ffi::c_void;

pub use deerpunk_engine_ffi::{GAMELIB_API_VERSION, ResolverFn, ServerInitData};

type ServerInitFn = unsafe extern "C" fn(ServerInitData);
type ServerFrameFn = unsafe extern "C" fn(f32);
type ServerShutdownFn = unsafe extern "C" fn();
type ServerPlayerConnectedFn = unsafe extern "C" fn(u32);
type ServerPlayerDisconnectedFn = unsafe extern "C" fn(u32);
/// `phase` 0 = after PrefabDef (send Map UM); 1 = after entity snapshot (LightStyle, SetView, …).
type ServerJoinInitFn = unsafe extern "C" fn(u32, u32);

pub struct ServerDll {
    _lib: libloading::Library,
    init_fn: ServerInitFn,
    frame_fn: ServerFrameFn,
    shutdown_fn: ServerShutdownFn,
    player_connected_fn: Option<ServerPlayerConnectedFn>,
    player_disconnected_fn: Option<ServerPlayerDisconnectedFn>,
    join_init_fn: Option<ServerJoinInitFn>,
}

impl ServerDll {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let lib = unsafe { libloading::Library::new(path) }?;
        let init_fn = unsafe { *lib.get::<ServerInitFn>(b"server_init")? };
        let frame_fn = unsafe { *lib.get::<ServerFrameFn>(b"server_frame")? };
        let shutdown_fn = unsafe { *lib.get::<ServerShutdownFn>(b"server_shutdown")? };
        let player_connected_fn = unsafe {
            lib.get::<ServerPlayerConnectedFn>(b"server_player_connected")
                .ok()
                .map(|s| *s)
        };
        let player_disconnected_fn = unsafe {
            lib.get::<ServerPlayerDisconnectedFn>(b"server_player_disconnected")
                .ok()
                .map(|s| *s)
        };
        let join_init_fn = unsafe {
            lib.get::<ServerJoinInitFn>(b"server_join_init")
                .ok()
                .map(|s| *s)
        };
        Ok(Self {
            _lib: lib,
            init_fn,
            frame_fn,
            shutdown_fn,
            player_connected_fn,
            player_disconnected_fn,
            join_init_fn,
        })
    }

    pub unsafe fn init(&self, ctx: *mut u8, resolver: ResolverFn) {
        let data = ServerInitData {
            version: GAMELIB_API_VERSION,
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

    pub unsafe fn player_connected(&self, slot: u32) {
        if let Some(f) = self.player_connected_fn {
            f(slot);
        } else {
            log::warn!("[host] server_player_connected not exported");
        }
    }

    pub unsafe fn join_init(&self, slot: u32, phase: u32) {
        if let Some(f) = self.join_init_fn {
            f(slot, phase);
        }
    }

    pub unsafe fn player_disconnected(&self, slot: u32) {
        if let Some(f) = self.player_disconnected_fn {
            f(slot);
        }
    }
}
