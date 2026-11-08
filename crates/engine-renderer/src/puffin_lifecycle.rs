//! Puffin profiler: scopes in-process, stream frames over TCP (`puffin_http`).

#[cfg(feature = "puffin")]
mod imp {
    use std::sync::Once;

    static START: Once = Once::new();
    static SERVER: std::sync::OnceLock<puffin_http::Server> = std::sync::OnceLock::new();

    pub fn ensure_started() {
        START.call_once(|| {
            puffin::set_scopes_on(true);
            let listen = std::env::var("PUFFIN_LISTEN")
                .unwrap_or_else(|_| format!("0.0.0.0:{}", puffin_http::DEFAULT_PORT));
            match puffin_http::Server::new(&listen) {
                Ok(server) => {
                    let viewer = std::env::var("PUFFIN_VIEWER_URL")
                        .unwrap_or_else(|_| format!("127.0.0.1:{}", puffin_http::DEFAULT_PORT));
                    let _ = SERVER.set(server);
                    log::info!("Puffin: serving on {listen} — run: puffin_viewer --url {viewer}");
                }
                Err(err) => {
                    log::error!("Puffin: failed to start HTTP server on {listen}: {err}");
                }
            }
        });
    }

    pub fn active() -> bool {
        puffin::are_scopes_on() && SERVER.get().is_some()
    }

    /// Call once per host frame (before scoped work).
    pub fn begin_frame() {
        if puffin::are_scopes_on() {
            puffin::GlobalProfiler::lock().new_frame();
        }
    }
}

pub fn puffin_ensure_started() {
    #[cfg(feature = "puffin")]
    imp::ensure_started();
}

pub fn puffin_active() -> bool {
    #[cfg(feature = "puffin")]
    {
        return imp::active();
    }
    #[cfg(not(feature = "puffin"))]
    {
        false
    }
}

pub fn puffin_begin_frame() {
    #[cfg(feature = "puffin")]
    imp::begin_frame();
}
