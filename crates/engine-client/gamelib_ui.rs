//! Map swapchain formats to stable tokens for gamelib UI init.

use deerpunk_engine_ffi::{
    GAMELIB_SURFACE_BGRA8, GAMELIB_SURFACE_BGRA8_SRGB, GAMELIB_SURFACE_RGBA8,
    GAMELIB_SURFACE_RGBA8_SRGB,
};

pub fn gamelib_ui_format_token(format: wgpu::TextureFormat) -> u32 {
    match format {
        wgpu::TextureFormat::Bgra8UnormSrgb => GAMELIB_SURFACE_BGRA8_SRGB,
        wgpu::TextureFormat::Rgba8UnormSrgb => GAMELIB_SURFACE_RGBA8_SRGB,
        wgpu::TextureFormat::Bgra8Unorm => GAMELIB_SURFACE_BGRA8,
        wgpu::TextureFormat::Rgba8Unorm => GAMELIB_SURFACE_RGBA8,
        _ => GAMELIB_SURFACE_BGRA8_SRGB,
    }
}
