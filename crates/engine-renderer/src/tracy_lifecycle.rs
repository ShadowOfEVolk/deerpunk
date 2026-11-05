//! Explicit Tracy startup/shutdown when built with `manual-lifetime` (see `Host::shutdown`).

#[cfg(feature = "tracy")]
mod imp {
    use std::sync::Once;

    static START: Once = Once::new();

    pub fn ensure_started() {
        START.call_once(|| {
            let _ = tracy_client::Client::start();
        });
    }

    pub fn shutdown() {
        if unsafe { tracy_client::sys::___tracy_profiler_started() == 0 } {
            return;
        }
        unsafe {
            tracy_client::sys::___tracy_shutdown_profiler();
        }
    }
}

/// Start the Tracy client once per process (host executable).
pub fn tracy_ensure_started() {
    #[cfg(feature = "tracy")]
    imp::ensure_started();
}

/// True while the global Tracy profiler is active (`___tracy_profiler_started`).
pub fn tracy_active() -> bool {
    #[cfg(feature = "tracy")]
    {
        unsafe { tracy_client::sys::___tracy_profiler_started() != 0 }
    }
    #[cfg(not(feature = "tracy"))]
    {
        false
    }
}

/// Stop Tracy worker thread; call after render/client teardown, no zones active.
pub fn tracy_shutdown() {
    #[cfg(feature = "tracy")]
    imp::shutdown();
}
