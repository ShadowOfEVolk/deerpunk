//! Map swapchain formats to stable tokens for gamelib UI init.
//! Nuklear draw pointers target the host context (same thread, inside the UI frame).

use std::ffi::CStr;

use deerpunk_engine_ffi::{
    NuklearDrawHost, GAMELIB_SURFACE_BGRA8, GAMELIB_SURFACE_BGRA8_SRGB, GAMELIB_SURFACE_RGBA8,
    GAMELIB_SURFACE_RGBA8_SRGB,
};
use deerpunk_engine_renderer::host_ui;

pub fn nuklear_draw_host() -> NuklearDrawHost {
    NuklearDrawHost {
        begin_overlay: Some(nk_begin_overlay),
        end_window: Some(nk_end_window),
        label: Some(nk_label),
        separator: Some(nk_separator),
    }
}

unsafe extern "C" fn nk_begin_overlay(title: *const u8, x: f32, y: f32, w: f32, h: f32) -> i32 {
    if title.is_null() {
        return 0;
    }
    let title = CStr::from_ptr(title as *const i8).to_string_lossy();
    if host_ui::begin_overlay(&title, x, y, w, h) {
        1
    } else {
        0
    }
}

unsafe extern "C" fn nk_end_window() {
    host_ui::end_window();
}

unsafe extern "C" fn nk_label(text: *const u8) {
    if text.is_null() {
        return;
    }
    let text = CStr::from_ptr(text as *const i8).to_string_lossy();
    host_ui::label(&text);
}

unsafe extern "C" fn nk_separator() {
    host_ui::separator();
}

pub fn gamelib_ui_format_token(format: wgpu::TextureFormat) -> u32 {
    match format {
        wgpu::TextureFormat::Bgra8UnormSrgb => GAMELIB_SURFACE_BGRA8_SRGB,
        wgpu::TextureFormat::Rgba8UnormSrgb => GAMELIB_SURFACE_RGBA8_SRGB,
        wgpu::TextureFormat::Bgra8Unorm => GAMELIB_SURFACE_BGRA8,
        wgpu::TextureFormat::Rgba8Unorm => GAMELIB_SURFACE_RGBA8,
        _ => GAMELIB_SURFACE_BGRA8_SRGB,
    }
}
