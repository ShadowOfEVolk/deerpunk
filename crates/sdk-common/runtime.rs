//! Process-wide gamelib handle to [`EngineApi`] (one per loaded client/server DLL).
//!
//! Call [`init_engine`] once from `client_init` / `server_init`, then use [`engine`]
//! from prefab apply, hooks, and game code — no `user_data` / host `ctx` in FFI callbacks.

use std::sync::OnceLock;

use deerpunk_engine_api::EngineApi;

static ENGINE: OnceLock<&'static EngineApi> = OnceLock::new();

/// Register the leaked [`EngineApi`] for this gamelib DLL. Idempotent (second call is ignored).
pub fn init_engine(api: &'static EngineApi) {
    let _ = ENGINE.set(api);
}

/// Gamelib engine facade (`engine().cmd`, `engine().world`, …).
#[inline]
pub fn engine() -> &'static EngineApi {
    ENGINE
        .get()
        .copied()
        .expect("gamelib engine not initialized — call init_engine() from *_init")
}

/// Same as [`engine`], but returns `None` before init (prefab apply returns `-1`).
#[inline]
pub fn try_engine() -> Option<&'static EngineApi> {
    ENGINE.get().copied()
}

/// Host `ctx` passed into [`EngineApi::new`] (methoddb). Prefer typed [`engine`] APIs.
#[inline]
pub fn raw_ctx() -> *mut u8 {
    engine().ctx
}

/// Build a leaked [`EngineApi`] and [`init_engine`]. Typical `*_init` entry.
#[inline]
pub fn install_engine(ctx: *mut u8, resolver: deerpunk_engine_api::Resolver) -> &'static EngineApi {
    let api = Box::leak(Box::new(EngineApi::new(ctx, resolver)));
    init_engine(api);
    api
}
