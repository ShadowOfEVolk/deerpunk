//! Gamelib debug widgets drawn through the host Nuklear context.

use crate::NuklearDrawHost;

pub const GAMELIB_UI_API_VERSION: u32 = 2;

/// Swapchain color format token (see engine-host `gamelib_ui_format_token`).
pub const GAMELIB_SURFACE_BGRA8_SRGB: u32 = 1;
pub const GAMELIB_SURFACE_RGBA8_SRGB: u32 = 2;
pub const GAMELIB_SURFACE_BGRA8: u32 = 3;
pub const GAMELIB_SURFACE_RGBA8: u32 = 4;

#[repr(C)]
pub struct GamelibUiBuildArgs {
    pub version: u32,
    /// Legacy: host `Device` (gamelib must not create a second wgpu stack in the DLL).
    pub device: *mut std::ffi::c_void,
    pub queue: *mut std::ffi::c_void,
    pub surface_format: u32,
    pub width: f32,
    pub height: f32,
    pub nk: NuklearDrawHost,
}

#[repr(C)]
pub struct GamelibUiRenderArgs {
    pub version: u32,
    pub encoder: *mut std::ffi::c_void,
    pub color_view: *mut std::ffi::c_void,
    pub width: f32,
    pub height: f32,
}

impl GamelibUiBuildArgs {
    pub fn valid_gpu(&self) -> bool {
        self.version == GAMELIB_UI_API_VERSION
            && !self.device.is_null()
            && !self.queue.is_null()
            && self.width > 0.0
            && self.height > 0.0
    }

    pub fn valid(&self) -> bool {
        self.valid_gpu()
    }
}

impl GamelibUiRenderArgs {
    pub fn valid(&self) -> bool {
        self.version == GAMELIB_UI_API_VERSION
            && !self.encoder.is_null()
            && !self.color_view.is_null()
            && self.width > 0.0
            && self.height > 0.0
    }
}

pub type GamelibUiBuildFn = unsafe extern "C" fn(*const GamelibUiBuildArgs) -> i32;
pub type GamelibUiRenderFn = unsafe extern "C" fn(*const GamelibUiRenderArgs) -> i32;
