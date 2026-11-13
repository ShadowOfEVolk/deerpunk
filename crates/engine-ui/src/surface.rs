//! Offscreen UI surfaces: extra RmlUi contexts rendered into their own
//! [`UiFrame`] stream (first use — inventory cursor item; later 3D panels).
//!
//! The geometry bridge is global (`rust_*` exports in `runtime.rs`), so
//! surfaces live in a process-static registry next to `GEOM`: the bridge
//! routes compile/draw/texture calls by [`GEOM_TARGET`] — `0` = main UI
//! builder, otherwise the surface id whose builder receives them. Each
//! surface owns a [`UiGeometryBuilder`], so mesh ids never cross streams;
//! texture ids come from the shared global counter (gamelib keeps one
//! texture table, surfaces just reference it).
//!
//! Engine clients reach the registry only through the [`UiRuntime`]
//! surface methods (`surface_create`, `surface_destroy`,
//! `surface_load_document`, `surface_data_model_open`,
//! `surface_data_model_apply`, `surface_frame`).
//!
//! Registry-lock discipline: never hold [`SURFACES`] across an FFI call
//! into RmlUi — render/texture callbacks re-enter Rust and re-lock it.

use std::collections::HashMap;
use std::ffi::CString;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex};

use deerpunk_engine_common::ui_draw::UiFrame;

use crate::assets;
use crate::draw_list::UiGeometryBuilder;
use crate::ffi;
use crate::runtime::{apply_data_model_ops, ui_elapsed_secs, DataModelOp, UiDocument};

/// Geometry-bridge target: `0` = main `GEOM`, otherwise the surface id
/// receiving compile/draw/texture callbacks. Set around surface update +
/// render windows (and surface document/data-model calls, which can resolve
/// textures); main rendering runs with the default `0`.
pub(crate) static GEOM_TARGET: AtomicU32 = AtomicU32::new(0);

/// Registry map. The raw shim handles (`rmlui_handle_t`) inside `UiSurface`
/// are only dereferenced on the UI thread that drives RmlUi; the mutex
/// exists so bridge callbacks (same thread) can route geometry by target —
/// which is also why it is never held across an FFI call. Sending the map
/// between threads is therefore sound.
struct SurfaceMap(HashMap<u32, UiSurface>);
unsafe impl Send for SurfaceMap {}

impl std::ops::Deref for SurfaceMap {
    type Target = HashMap<u32, UiSurface>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for SurfaceMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

static SURFACES: LazyLock<Mutex<SurfaceMap>> =
    LazyLock::new(|| Mutex::new(SurfaceMap(HashMap::new())));

/// RAII: restore the previous bridge target, also on unwind.
pub(crate) struct TargetGuard {
    prev: u32,
}

impl TargetGuard {
    pub(crate) fn new(id: u32) -> Self {
        Self {
            prev: GEOM_TARGET.swap(id, Ordering::Relaxed),
        }
    }
}

impl Drop for TargetGuard {
    fn drop(&mut self) {
        GEOM_TARGET.store(self.prev, Ordering::Relaxed);
    }
}

pub(crate) struct UiSurface {
    ctx: ffi::rmlui_handle_t,
    /// Loaded document (closed before the context on destroy).
    doc: Option<UiDocument>,
    builder: UiGeometryBuilder,
    /// Input, data-model write, or document load since the last `frame`.
    force: bool,
    /// `f64` bits, absolute [`ui_elapsed_secs`]. Infinity = idle until input
    /// or a data-model write; independent of the main UI throttle.
    next_update_at: u64,
}

impl UiSurface {
    pub(crate) fn new(ctx: ffi::rmlui_handle_t, w: u32, h: u32) -> Self {
        Self {
            ctx,
            doc: None,
            builder: UiGeometryBuilder::new(w, h),
            force: true,
            next_update_at: 0,
        }
    }

    /// Ordered teardown: close the document while its context still exists,
    /// then destroy the context. Called from `surface_destroy` and when the
    /// `UiRuntime` shuts down; the interpreter is alive at that point.
    fn destroy(&mut self) {
        self.doc = None;
        if !self.ctx.is_null() {
            unsafe { ffi::rmlui_context_destroy(self.ctx) };
            self.ctx = ptr::null_mut();
        }
    }
}

/// Route one bridge callback to the builder of [`GEOM_TARGET`].
/// `f` must not call back into RmlUi — the registry lock is held throughout.
pub(crate) fn with_surface_geom<R>(
    id: u32,
    f: impl FnOnce(&mut UiGeometryBuilder) -> R,
) -> Option<R> {
    let mut surfaces = SURFACES.lock().ok()?;
    surfaces.get_mut(&id).map(|s| f(&mut s.builder))
}

/// Create a surface context `surface_<id>` with its own geometry builder.
/// `dp_ratio` is the main UI's layout scale (`UiRuntime::set_dp_ratio`) —
/// new RmlUi contexts default to 1.0, which halves layouts on 2x screens.
pub(crate) fn create(id: u32, w: i32, h: i32, dp_ratio: f32) -> Result<(), String> {
    if id == 0 {
        return Err("surface id 0 is reserved for the main UI".into());
    }
    if w <= 0 || h <= 0 {
        return Err(format!("surface size must be positive, got {w}x{h}"));
    }
    let mut surfaces = SURFACES.lock().map_err(|_| "surface registry poisoned")?;
    if surfaces.contains_key(&id) {
        return Err(format!("surface id {id} already in use"));
    }
    let name = CString::new(format!("surface_{id}")).expect("surface name has no null");
    let ctx = unsafe { ffi::rmlui_context_create(name.as_ptr()) };
    if ctx.is_null() {
        return Err(format!("rmlui_context_create failed for surface_{id}"));
    }
    // New RmlUi contexts have zero dimensions — main gets its size from
    // `rmlui_init`, surfaces must set layout dimensions explicitly.
    unsafe { ffi::rmlui_context_set_dimensions(ctx, w, h) };
    unsafe { ffi::rmlui_context_set_dp_ratio(ctx, dp_ratio) };
    surfaces.insert(id, UiSurface::new(ctx, w as u32, h as u32));
    log::info!(target: "ui", "surface {id} created {w}x{h}");
    Ok(())
}

/// Destroy a surface (unknown id: no-op). The document closes before the
/// context, then both are gone.
pub(crate) fn destroy(id: u32) {
    if let Ok(mut surfaces) = SURFACES.lock() {
        if let Some(mut s) = surfaces.remove(&id) {
            s.destroy();
            log::info!(target: "ui", "surface {id} destroyed");
        }
    }
}

/// Teardown for `UiRuntime::drop`: surfaces must die before `rmlui_shutdown`.
pub(crate) fn destroy_all() {
    if let Ok(mut surfaces) = SURFACES.lock() {
        for (id, mut s) in surfaces.drain() {
            s.destroy();
            log::info!(target: "ui", "surface {id} destroyed (runtime shutdown)");
        }
    }
}

/// Load a document on the surface context. Path resolution matches main:
/// `rmlui_document_load` resolves through the `get_base_path` callback
/// (`assets::ui_base_path`), so `relative` is UI-base-relative.
pub(crate) fn load_document(id: u32, relative: &str) -> Result<(), String> {
    let path = CString::new(relative).map_err(|_| "document path contains null byte")?;
    let ctx = {
        let mut surfaces = SURFACES.lock().map_err(|_| "surface registry poisoned")?;
        let s = surfaces
            .get_mut(&id)
            .ok_or_else(|| format!("surface {id} not found"))?;
        if s.ctx.is_null() {
            return Err("surface context is gone".into());
        }
        s.ctx
    };
    let handle = {
        let _target = TargetGuard::new(id);
        unsafe { ffi::rmlui_document_load(ctx, path.as_ptr()) }
    };
    if handle.is_null() {
        return Err(format!("failed to load document: {relative}"));
    }
    if let Ok(mut surfaces) = SURFACES.lock() {
        if let Some(s) = surfaces.get_mut(&id) {
            s.doc = Some(UiDocument::from_handle(handle));
            s.force = true;
        }
    }
    // A surface renders its document unconditionally — show it (main keeps
    // manual `show`; the shim loads documents hidden). Outside the lock.
    unsafe { ffi::rmlui_document_show(handle) };
    Ok(())
}

/// Open a data model on the surface context (shared interpreter).
///
/// `lua_table_src` accepts either a Lua expression evaluating to the model
/// table (typically a constructor like `{ health = 0, rows = {} }`) or a
/// `.lua` path relative to the UI base whose chunk returns the model table.
/// The generated code keeps the table in a unique global
/// `surface_<id>_<model>` (the interpreter is shared — bare model names
/// would collide with main-UI globals) and opens it strictly on this
/// surface's context, never on main.
pub(crate) fn data_model_open(id: u32, model: &str, lua_table_src: &str) -> Result<(), String> {
    if model.is_empty()
        || !model
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(format!("invalid data-model name: {model:?}"));
    }
    let trimmed = lua_table_src.trim();
    let ctx_name = format!("surface_{id}");
    let global = format!("{ctx_name}_{model}");
    let table_assignment = if trimmed.ends_with(".lua") {
        let path = Path::new(trimmed);
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            assets::ui_base_path().join(path)
        };
        let src = std::fs::read_to_string(&abs)
            .map_err(|e| format!("lua source not found {}: {e}", abs.display()))?;
        if src.contains('\0') {
            return Err(format!("lua source {} contains null byte", abs.display()));
        }
        // Chunk form: the file must end with `return <model table>`.
        format!("{global} = (function()\n{src}\nend)()")
    } else {
        if lua_table_src.contains('\0') {
            return Err("lua source contains null byte".into());
        }
        // Expression form.
        format!("{global} = {lua_table_src}")
    };
    {
        let surfaces = SURFACES.lock().map_err(|_| "surface registry poisoned")?;
        let s = surfaces
            .get(&id)
            .ok_or_else(|| format!("surface {id} not found"))?;
        if s.ctx.is_null() {
            return Err("surface context is gone".into());
        }
    }
    let code = format!(
        "{table_assignment}\nassert(rmlui.contexts['{ctx_name}']:OpenDataModel('{model}', {global}))\n"
    );
    let c = CString::new(code).map_err(|_| "generated lua contains null byte")?;
    {
        let _target = TargetGuard::new(id);
        unsafe { ffi::rmlui_lua_do_string(c.as_ptr()) };
    }
    if let Ok(mut surfaces) = SURFACES.lock() {
        if let Some(s) = surfaces.get_mut(&id) {
            s.force = true;
        }
    }
    Ok(())
}

/// Batch-write the model named by the loaded document's `data-model`
/// attribute (same path as main: through the surface document handle).
pub(crate) fn data_model_apply(id: u32, ops: &[DataModelOp]) -> Result<(), String> {
    let handle = {
        let surfaces = SURFACES.lock().map_err(|_| "surface registry poisoned")?;
        let s = surfaces
            .get(&id)
            .ok_or_else(|| format!("surface {id} not found"))?;
        let doc = s
            .doc
            .as_ref()
            .ok_or_else(|| format!("surface {id}: no document loaded"))?;
        doc.handle()
    };
    let result = {
        let _target = TargetGuard::new(id);
        apply_data_model_ops(handle, ops)
    };
    if let Ok(mut surfaces) = SURFACES.lock() {
        if let Some(s) = surfaces.get_mut(&id) {
            s.force = true;
        }
    }
    result.map_err(|e| e.to_string())
}

/// Update + render the surface on its own RmlUi cycle — no coupling to the
/// main UI's `r_ui_fps` throttling. Dirty-only: `None` when the context is
/// unknown/idle or the render produced identical draws (delta contract).
pub(crate) fn frame(id: u32) -> Option<UiFrame> {
    // Critical section 1: throttle decision (no lock held across FFI).
    let (ctx, due, now) = {
        let mut surfaces = SURFACES.lock().ok()?;
        let s = surfaces.get_mut(&id)?;
        if s.ctx.is_null() {
            return None;
        }
        let now = ui_elapsed_secs();
        let due = s.force || now >= f64::from_bits(s.next_update_at);
        (s.ctx, due, now)
    };
    if !due {
        return None;
    }
    let _target = TargetGuard::new(id);
    unsafe { ffi::rmlui_context_update(ctx) };
    let delay = unsafe { ffi::rmlui_context_next_update_delay(ctx) };

    // Critical section 2: consume the force flag, schedule the next update,
    // start the render window.
    {
        let mut surfaces = SURFACES.lock().ok()?;
        let s = surfaces.get_mut(&id)?;
        s.force = false;
        s.next_update_at = if delay.is_finite() {
            (now + delay).to_bits()
        } else {
            f64::INFINITY.to_bits()
        };
        s.builder.begin_frame();
    }
    unsafe { ffi::rmlui_context_render(ctx) };

    // Critical section 3: delta take.
    SURFACES
        .lock()
        .ok()?
        .get_mut(&id)?
        .builder
        .take_frame_if_changed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{make_dummy_render_api, test_callbacks, DataModelOp, DataModelOpValue};

    /// Surface RmlUi/C-side diagnostics in test output.
    struct TestLogger;
    impl log::Log for TestLogger {
        fn enabled(&self, meta: &log::Metadata) -> bool {
            meta.level() <= log::Level::Warn
        }
        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                eprintln!("[{}] {}", record.level(), record.args());
            }
        }
        fn flush(&self) {}
    }

    const CURSOR_RML: &str = r#"<rml>
<head>
<style>
	body { }
	#icon { display: block; width: 16px; height: 16px; }
	#icon.shown { background: #f00; }
</style>
</head>
<body data-model="cur">
	<div id="icon" data-class-shown="item_id > 0"></div>
</body>
</rml>
"#;

    /// Headless surface smoke: registry guards, per-surface RmlUi cycle,
    /// dirty-only delta contract, model opened on the surface context only.
    #[test]
    fn surface_frames_are_dirty_only_and_model_isolated() {
        let _rml = crate::runtime::RML_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let _ = log::set_boxed_logger(Box::new(TestLogger));
        log::set_max_level(log::LevelFilter::Info);

        let base = std::env::temp_dir().join(format!("deerpunk-ui-surface-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("cursor_test.rml"), CURSOR_RML).unwrap();
        assets::set_ui_base_path(base.clone());

        // Boot the interpreter directly (same shape as runtime.rs tests):
        // dummy renderer, no fonts — geometry binds, no text nodes.
        let callbacks: &'static ffi::rmlui_callbacks_t = Box::leak(test_callbacks());
        let render_api: &'static ffi::rmlui_render_api_t = Box::leak(make_dummy_render_api());
        assert_eq!(unsafe { ffi::rmlui_init(64, 64, callbacks, render_api) }, 1);
        let main_ctx = unsafe { ffi::rmlui_context_create(c"main".as_ptr()) };
        assert!(!main_ctx.is_null(), "main context_create failed");

        // Registry guards: id 0 reserved for main, duplicates rejected.
        assert!(create(0, 16, 16, 1.0).is_err(), "id 0 must be reserved");
        assert!(create(1, 32, 32, 1.0).is_ok(), "surface create failed");
        assert!(create(1, 32, 32, 1.0).is_err(), "duplicate id must fail");

        // Unknown ids error on ops and frame None.
        assert!(load_document(99, "cursor_test.rml").is_err());
        assert!(data_model_open(99, "cur", "{}").is_err());
        assert!(data_model_apply(99, &[]).is_err());
        assert!(frame(99).is_none());

        // Model opens on the surface CONTEXT first; the document binds it at
        // load time (same order as main: Lua model, then document).
        data_model_open(1, "cur", "{ item_id = 7 }").expect("open model on surface ctx");
        load_document(1, "cursor_test.rml").expect("surface document load");

        // Context + model global exist under the surface names.
        unsafe {
            ffi::rmlui_lua_do_string(
                c"assert(rmlui.contexts['surface_1'] ~= nil)
assert(surface_1_cur ~= nil and surface_1_cur.item_id == 7)"
                    .as_ptr(),
            );
        }

        // First frame: icon visible (item_id = 7) — mesh + white texture ship.
        let f1 = frame(1).expect("first surface frame");
        assert_eq!(f1.screen, [32, 32], "screen must be the surface size");
        assert!(!f1.meshes.is_empty(), "icon geometry compiled");
        assert!(!f1.draws.is_empty(), "icon draw recorded");
        assert!(f1.textures.contains_key(&1), "white fallback ships once");

        // Idle: no writes, delay infinite → no frame (delta contract).
        assert!(frame(1).is_none(), "idle surface must not emit frames");

        // Data-model write → next frame ships the delta (draws changed).
        let ops = vec![DataModelOp {
            path: "item_id".into(),
            value: DataModelOpValue::Number(0.0),
        }];
        data_model_apply(1, &ops).expect("apply op");
        let f2 = frame(1).expect("dirty surface ships a frame");
        assert!(
            !f2.draws.is_empty() || !f2.released_meshes.is_empty(),
            "hide delta must ship draws or released meshes"
        );
        assert!(frame(1).is_none(), "still idle after the delta");

        // The surface model never touched main: 'cur' is still free there.
        unsafe {
            ffi::rmlui_lua_do_string(
                c"assert(rmlui.contexts['main']:OpenDataModel('cur', { x = 1 }))".as_ptr(),
            );
        }

        destroy(1);
        unsafe {
            ffi::rmlui_context_destroy(main_ctx);
            ffi::rmlui_shutdown();
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
