//! [`r_log_uploads`] — log GPU **resource** uploads (mesh / material / shader / texture), not per-frame uniforms.

use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Synced from [`r_log_uploads`] each frame via client `apply_renderer_cvars`.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn material(path_or_handle: &str, gpu_row: u32) {
    if !enabled() {
        return;
    }
    log::info!("[gpu upload] material `{path_or_handle}` ssbo_row={gpu_row}");
}

pub fn material_created(path: &str, handle: u32) {
    if !enabled() {
        return;
    }
    log::info!("[gpu upload] material store `{path}` handle={handle}");
}

pub fn shader(path: &str, defines: &str) {
    if !enabled() {
        return;
    }
    if defines.is_empty() {
        log::info!("[gpu upload] shader `{path}`");
    } else {
        log::info!("[gpu upload] shader `{path}` defines=[{defines}]");
    }
}

pub fn texture(kind: &str, w: u32, h: u32, bytes: usize) {
    if !enabled() {
        return;
    }
    log::info!("[gpu upload] texture {kind} {w}x{h} bytes={bytes}");
}
