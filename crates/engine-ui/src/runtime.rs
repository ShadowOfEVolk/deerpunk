use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use deerpunk_engine_common::ui_draw::UiFrame;

use crate::assets::{self, UiAssetSource};
use crate::draw_list::UiGeometryBuilder;
use crate::ffi;
use crate::paths;
use crate::surface;

static GEOM: Mutex<Option<UiGeometryBuilder>> = Mutex::new(None);
static START: OnceLock<Instant> = OnceLock::new();
/// `f64` bits. `Context::GetNextUpdateDelay` — infinity means idle until input.
static NEXT_UPDATE_AT: AtomicU64 = AtomicU64::new(0);
/// Input, resize, or a data-model write happened since the last `frame`.
static UI_FORCE: AtomicBool = AtomicBool::new(true);
static DUMMY_RENDER_API: Mutex<Option<Box<ffi::rmlui_render_api_t>>> = Mutex::new(None);
/// Kept alive for RmlUi (`g_callbacks` stores a raw pointer).
static CALLBACKS: Mutex<Option<Box<ffi::rmlui_callbacks_t>>> = Mutex::new(None);
static UI_CMD_QUEUE: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static CONTENT_ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);
static FILES_LIST_JSON: Mutex<Option<CString>> = Mutex::new(None);
/// Listen-server / in-map: Lua `in_game()`.
static CLIENT_IN_GAME: AtomicBool = AtomicBool::new(false);
/// LAN browser JSON for `eng_get_files_list("lan://servers")`.
static LAN_SERVERS_JSON: Mutex<Option<CString>> = Mutex::new(None);

/// Gamedir root for `eng_get_files_list` (e.g. `…/content/darkdemo`).
pub fn set_content_root(path: impl Into<PathBuf>) {
    if let Ok(mut g) = CONTENT_ROOT.lock() {
        *g = Some(path.into());
    }
}

/// Serializes tests that boot the process-global RmlUi interpreter
/// (`rmlui_init` guards on a renderer singleton). Shared with `surface` tests.
#[cfg(test)]
pub(crate) static RML_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Drive Lua `in_game()` (host sets when server enable flips).
pub fn set_client_in_game(on: bool) {
    CLIENT_IN_GAME.store(on, Ordering::SeqCst);
}

/// Host pushes LAN server list JSON for the menu browser.
pub fn set_lan_servers_json(json: impl Into<String>) {
    let s = json.into();
    if let Ok(mut g) = LAN_SERVERS_JSON.lock() {
        *g = Some(CString::new(s).unwrap_or_else(|_| CString::new("[]").unwrap()));
    }
}

/// Run Lua in the RmlUi interpreter (gamelib / content scripts).
pub fn lua_eval(code: &str) {
    if code.is_empty() {
        return;
    }
    mark_ui_dirty();
    let c = CString::new(code).expect("ui lua_eval: code contains null byte");
    unsafe {
        ffi::rmlui_lua_do_string(c.as_ptr());
    }
}

/// Drain console commands queued by RmlUi Lua `cmd(...)`.
pub fn drain_commands() -> Vec<String> {
    let Ok(mut q) = UI_CMD_QUEUE.lock() else {
        return Vec::new();
    };
    q.drain(..).collect()
}

pub(crate) fn with_geom<R>(f: impl FnOnce(&mut UiGeometryBuilder) -> R) -> Option<R> {
    let mut g = GEOM.lock().ok()?;
    g.as_mut().map(f)
}

/// Route one geometry-bridge callback by [`surface::GEOM_TARGET`]: `0` = main
/// `GEOM`, otherwise the addressed surface's own builder.
fn with_geom_target<R>(f: impl FnOnce(&mut UiGeometryBuilder) -> R) -> Option<R> {
    let target = surface::GEOM_TARGET.load(Ordering::Relaxed);
    if target == 0 {
        with_geom(f)
    } else {
        surface::with_surface_geom(target, f)
    }
}

/// Loaded RML document handle (Panorama-style: document = UI unit).
pub struct UiDocument {
    handle: ffi::rmlui_handle_t,
    /// Last value written per field. Unchanged writes skip RmlUi (no dirty).
    written: RefCell<HashMap<String, DataModelValue>>,
}

/// Typed value for [`UiDocument::data_model_set`].
#[derive(Clone, Debug, PartialEq)]
pub enum DataModelValue {
    Bool(bool),
    Number(f64),
    String(String),
}

/// One batched write for [`UiDocument::data_model_apply`].
#[derive(Clone, Debug, PartialEq)]
pub struct DataModelOp {
    /// Dotted path (`health`, `rows.2.icon`); digit-only segments are 1-based
    /// Lua array indices, others are string keys. The root must be a variable
    /// name.
    pub path: String,
    pub value: DataModelOpValue,
}

/// Value payload for [`DataModelOp`].
#[derive(Clone, Debug, PartialEq)]
pub enum DataModelOpValue {
    Bool(bool),
    Number(f64),
    String(String),
    /// Set the length of the Lua array at the op path: growth fills empty
    /// tables, the tail of a shrink is dropped.
    Resize(usize),
}

/// Failure of [`UiDocument::data_model_apply`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataModelApplyError {
    /// The widget document handle is gone.
    NoDocument,
    /// The first failed op (0-based). A missing/unknown `data-model` on the
    /// document also surfaces here as `Op(0)`; the host log distinguishes.
    Op(usize),
}

impl std::fmt::Display for DataModelApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDocument => write!(f, "document gone"),
            Self::Op(i) => write!(f, "op {i} failed"),
        }
    }
}

impl UiDocument {
    pub fn data_model_set(&self, field: &str, value: DataModelValue) -> bool {
        match value {
            DataModelValue::Bool(v) => self.data_model_set_bool(field, v),
            DataModelValue::Number(v) => self.data_model_set_number(field, v),
            DataModelValue::String(v) => self.data_model_set_string(field, &v),
        }
    }

    pub fn show(&self) {
        if !self.handle.is_null() {
            mark_ui_dirty();
            unsafe { ffi::rmlui_document_show(self.handle) };
        }
    }

    pub fn hide(&self) {
        if !self.handle.is_null() {
            mark_ui_dirty();
            unsafe { ffi::rmlui_document_hide(self.handle) };
        }
    }

    /// Set a field on this document's body `data-model` (RmlUi Lua binding + dirty).
    pub fn data_model_set_bool(&self, field: &str, value: bool) -> bool {
        if self.handle.is_null() || field.is_empty() {
            return false;
        }
        if self.already_written(field, &DataModelValue::Bool(value)) {
            return true;
        }
        let field_c = match CString::new(field) {
            Ok(c) => c,
            Err(_) => return false,
        };
        let ok = unsafe {
            ffi::rmlui_document_data_model_set_bool(self.handle, field_c.as_ptr(), value as c_int)
                != 0
        };
        if ok {
            self.remember(field, DataModelValue::Bool(value));
            mark_ui_dirty();
        }
        ok
    }

    pub fn data_model_set_number(&self, field: &str, value: f64) -> bool {
        if self.handle.is_null() || field.is_empty() {
            return false;
        }
        if self.already_written(field, &DataModelValue::Number(value)) {
            return true;
        }
        let field_c = match CString::new(field) {
            Ok(c) => c,
            Err(_) => return false,
        };
        let ok = unsafe {
            ffi::rmlui_document_data_model_set_number(self.handle, field_c.as_ptr(), value) != 0
        };
        if ok {
            self.remember(field, DataModelValue::Number(value));
            mark_ui_dirty();
        }
        ok
    }

    pub fn data_model_set_string(&self, field: &str, value: &str) -> bool {
        if self.handle.is_null() || field.is_empty() {
            return false;
        }
        if matches!(self.written.borrow().get(field), Some(DataModelValue::String(prev)) if prev == value)
        {
            return true;
        }
        let field_c = match CString::new(field) {
            Ok(c) => c,
            Err(_) => return false,
        };
        let value_c = match CString::new(value) {
            Ok(c) => c,
            Err(_) => return false,
        };
        let ok = unsafe {
            ffi::rmlui_document_data_model_set_string(
                self.handle,
                field_c.as_ptr(),
                value_c.as_ptr(),
            ) != 0
        };
        if ok {
            self.remember(field, DataModelValue::String(value.to_string()));
            mark_ui_dirty();
        }
        ok
    }

    /// Apply a batch of path writes to this document's body `data-model`
    /// (RmlUi Lua binding + dirty). Ops apply in order; the first failed op
    /// stops the batch, earlier ops stay applied. Unlike the scalar setters
    /// there is no `written` dedup — paths vary per op and RmlUi only dirties
    /// variables whose value actually changed.
    pub fn data_model_apply(&self, ops: &[DataModelOp]) -> Result<(), DataModelApplyError> {
        apply_data_model_ops(self.handle, ops)
    }

    /// Wrap a raw shim document handle (surface registry owns these).
    pub(crate) fn from_handle(handle: ffi::rmlui_handle_t) -> Self {
        Self {
            handle,
            written: RefCell::new(HashMap::new()),
        }
    }

    /// Raw shim handle (surface routing applies ops through it).
    pub(crate) fn handle(&self) -> ffi::rmlui_handle_t {
        self.handle
    }
}

/// Batch-write ops through a raw document handle; shared by main documents
/// ([`UiDocument::data_model_apply`]) and surface documents. The C side
/// borrows the strings only for the duration of the call; boxes keep the
/// NUL buffers address-stable while the Vec grows.
pub(crate) fn apply_data_model_ops(
    handle: ffi::rmlui_handle_t,
    ops: &[DataModelOp],
) -> Result<(), DataModelApplyError> {
    if handle.is_null() {
        return Err(DataModelApplyError::NoDocument);
    }
    let mut strings: Vec<Box<CString>> = Vec::with_capacity(ops.len());
    let mut raw: Vec<ffi::rmlui_data_model_op_t> = Vec::with_capacity(ops.len());
    for (i, op) in ops.iter().enumerate() {
        let path_c = CString::new(op.path.as_str()).map_err(|_| DataModelApplyError::Op(i))?;
        strings.push(Box::new(path_c));
        let path_ptr = strings.last().unwrap().as_ptr();
        let (tag, value) = match &op.value {
            DataModelOpValue::Bool(v) => {
                let mut value: ffi::rmlui_data_model_op_s__bindgen_ty_1 =
                    unsafe { std::mem::zeroed() };
                value.b = *v as c_int;
                (ffi::RMLUI_DATA_MODEL_OP_BOOL as u8, value)
            }
            DataModelOpValue::Number(v) => {
                let mut value: ffi::rmlui_data_model_op_s__bindgen_ty_1 =
                    unsafe { std::mem::zeroed() };
                value.num = *v;
                (ffi::RMLUI_DATA_MODEL_OP_NUMBER as u8, value)
            }
            DataModelOpValue::String(v) => {
                let value_c = CString::new(v.as_str()).map_err(|_| DataModelApplyError::Op(i))?;
                strings.push(Box::new(value_c));
                let mut value: ffi::rmlui_data_model_op_s__bindgen_ty_1 =
                    unsafe { std::mem::zeroed() };
                value.str_ = strings.last().unwrap().as_ptr();
                (ffi::RMLUI_DATA_MODEL_OP_STRING as u8, value)
            }
            DataModelOpValue::Resize(len) => {
                let mut value: ffi::rmlui_data_model_op_s__bindgen_ty_1 =
                    unsafe { std::mem::zeroed() };
                value.len = *len as i64;
                (ffi::RMLUI_DATA_MODEL_OP_RESIZE as u8, value)
            }
        };
        raw.push(ffi::rmlui_data_model_op_t {
            path: path_ptr,
            tag,
            _pad: [0; 7],
            value,
        });
    }

    let rc = unsafe { ffi::rmlui_document_data_model_apply(handle, raw.as_ptr(), raw.len()) };
    // Non-empty batches (even partially failed ones) changed the model.
    if !ops.is_empty() {
        mark_ui_dirty();
    }
    if rc == 0 {
        Ok(())
    } else {
        Err(DataModelApplyError::Op((-rc - 1) as usize))
    }
}

impl UiDocument {
    /// Post-layout component tree as text (agent tooling): depth-first, one
    /// element per line — `TAG#id.class1.class2 [x,y w×h]`, with a quoted
    /// text snippet for `#text` nodes; 2-space indent per nesting level.
    /// `max_depth` caps the nesting levels shown (1 = document root only);
    /// `None` = unlimited. `None` if the runtime/document is gone.
    pub fn tree_text(&self, max_depth: Option<u32>) -> Option<String> {
        if self.handle.is_null() {
            return None;
        }
        let depth = i32::try_from(max_depth.unwrap_or(0)).unwrap_or(i32::MAX);
        let buffer = unsafe { ffi::rmlui_document_tree(self.handle, depth) };
        if buffer.is_null() {
            return None;
        }
        let text = unsafe { CStr::from_ptr(buffer) }
            .to_string_lossy()
            .into_owned();
        unsafe { ffi::rmlui_free(buffer.cast()) };
        Some(text)
    }

    fn already_written(&self, field: &str, value: &DataModelValue) -> bool {
        self.written
            .borrow()
            .get(field)
            .is_some_and(|prev| prev == value)
    }

    fn remember(&self, field: &str, value: DataModelValue) {
        self.written.borrow_mut().insert(field.to_string(), value);
    }
}

impl Drop for UiDocument {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { ffi::rmlui_document_close(self.handle) };
            self.handle = ptr::null_mut();
        }
    }
}

/// RmlUi runtime: one context, documents loaded from UI asset root.
pub struct UiRuntime {
    ctx: ffi::rmlui_handle_t,
    initialized: bool,
    /// Last layout scale set via [`UiRuntime::set_dp_ratio`] — new surfaces
    /// inherit it (main starts at 1.0).
    dp_ratio: Cell<f32>,
    _assets: Box<dyn UiAssetSource>,
}

impl UiRuntime {
    pub fn new(assets: Box<dyn UiAssetSource>, screen: (u32, u32), context_name: &str) -> Self {
        assets::set_ui_base_path(assets.base().to_path_buf());
        let _ = START.set(Instant::now());
        UI_FORCE.store(true, Ordering::Relaxed);
        NEXT_UPDATE_AT.store(0, Ordering::Relaxed);
        *GEOM.lock().unwrap() = Some(UiGeometryBuilder::new(screen.0, screen.1));

        let (w, h) = screen;
        let callbacks = Box::new(ffi::rmlui_callbacks_t {
            get_time: Some(cb_get_time),
            log: Some(cb_log),
            get_base_path: Some(cb_base_path),
            scissor: None,
            execute_command: Some(cb_execute_command),
            client_in_game: Some(cb_client_in_game),
            get_cvar_float: None,
            get_cvar_string: None,
            get_files_list: Some(cb_get_files_list),
            load_file: None,
        });
        let cb_ptr = &*callbacks as *const ffi::rmlui_callbacks_t;
        *CALLBACKS.lock().unwrap() = Some(callbacks);

        let render_api = make_dummy_render_api();
        *DUMMY_RENDER_API.lock().unwrap() = Some(render_api);
        let api_ptr = DUMMY_RENDER_API
            .lock()
            .unwrap()
            .as_ref()
            .map(|b| &**b as *const _)
            .unwrap_or(ptr::null());

        let ok = unsafe { ffi::rmlui_init(w as i32, h as i32, cb_ptr, api_ptr) };
        if ok == 0 {
            log::error!(target: "ui", "rmlui_init failed");
            return Self {
                ctx: ptr::null_mut(),
                initialized: false,
                dp_ratio: Cell::new(1.0),
                _assets: assets,
            };
        }

        if !load_default_fonts() {
            log::error!(target: "ui", "no fonts under {}", assets.base().display());
            unsafe { ffi::rmlui_shutdown() };
            return Self {
                ctx: ptr::null_mut(),
                initialized: false,
                dp_ratio: Cell::new(1.0),
                _assets: assets,
            };
        }

        let name = CString::new(context_name).unwrap_or_else(|_| CString::new("main").unwrap());
        let ctx = unsafe { ffi::rmlui_context_create(name.as_ptr()) };
        if ctx.is_null() {
            log::error!(target: "ui", "context_create failed");
            unsafe { ffi::rmlui_shutdown() };
            return Self {
                ctx: ptr::null_mut(),
                initialized: false,
                dp_ratio: Cell::new(1.0),
                _assets: assets,
            };
        }

        log::info!(
            target: "ui",
            "ready {}x{} base={}",
            w,
            h,
            assets.base().display()
        );
        Self {
            ctx,
            initialized: true,
            dp_ratio: Cell::new(1.0),
            _assets: assets,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.initialized && !self.ctx.is_null()
    }

    /// Load document relative to UI base, e.g. `widgets/hello.rml`.
    /// Does not show — caller uses [`UiDocument::show`].
    pub fn load_document(&self, relative: &str) -> Option<UiDocument> {
        if !self.is_ready() {
            return None;
        }
        let path = CString::new(relative).ok()?;
        let handle = unsafe { ffi::rmlui_document_load(self.ctx, path.as_ptr()) };
        if handle.is_null() {
            log::error!(target: "ui", "failed to load document: {relative}");
            return None;
        }
        mark_ui_dirty();
        Some(UiDocument {
            handle,
            written: RefCell::new(HashMap::new()),
        })
    }

    // ── UI Surfaces: offscreen RmlUi contexts (registry in `surface`) ──

    /// Create an offscreen surface: its own RmlUi context (`surface_<id>`)
    /// and geometry builder (own mesh-id space; texture ids shared with
    /// main; dp ratio inherited from the last [`UiRuntime::set_dp_ratio`]).
    /// `id` must be nonzero (0 is the main UI) and unique; `w`, `h`
    /// positive.
    pub fn surface_create(&mut self, id: u32, w: i32, h: i32) -> Result<(), String> {
        if !self.is_ready() {
            return Err("ui runtime not ready".into());
        }
        surface::create(id, w, h, self.dp_ratio.get())
    }

    /// Destroy a surface (document closes before its context); unknown ids
    /// are a no-op.
    pub fn surface_destroy(&mut self, id: u32) {
        surface::destroy(id);
    }

    /// Load a document on the surface. `path` is UI-base-relative and
    /// resolves through the same shim base-path callback as main.
    pub fn surface_load_document(&self, id: u32, path: &str) -> Result<(), String> {
        surface::load_document(id, path)
    }

    /// Open a data model on the surface context (shared interpreter).
    /// `lua_table_src` is a Lua expression evaluating to the model table
    /// (e.g. `{ item_id = 0, x = 0 }`) or a `.lua` path (UI-base-relative)
    /// whose chunk returns the model table. The table lands in the global
    /// `surface_<id>_<model>`; the model opens strictly on this surface's
    /// context, never on main.
    pub fn surface_data_model_open(
        &self,
        id: u32,
        model: &str,
        lua_table_src: &str,
    ) -> Result<(), String> {
        surface::data_model_open(id, model, lua_table_src)
    }

    /// Batch-write the model named by the surface document's `data-model`
    /// attribute (same op paths as [`UiDocument::data_model_apply`]).
    pub fn surface_data_model_apply(&self, id: u32, ops: &[DataModelOp]) -> Result<(), String> {
        surface::data_model_apply(id, ops)
    }

    /// Dirty-only frame from the surface's own RmlUi cycle — independent of
    /// the main UI's fps throttling. `None` while nothing changed.
    pub fn surface_frame(&self, id: u32) -> Option<UiFrame> {
        surface::frame(id)
    }

    pub fn resize(&self, w: u32, h: u32) {
        mark_ui_dirty();
        with_geom(|g| g.resize(w, h));
        unsafe {
            ffi::rmlui_set_viewport(w as i32, h as i32);
            if !self.ctx.is_null() {
                ffi::rmlui_context_set_dimensions(self.ctx, w as i32, h as i32);
            }
        }
    }

    /// Layout scale: CSS `px`/`dp` become `ratio` context pixels. Context
    /// size stays in framebuffer pixels. Remembered for surfaces created
    /// later; call before `surface_create` on fractional-DPI screens.
    pub fn set_dp_ratio(&self, ratio: f32) {
        if !(ratio > 0.0) {
            return;
        }
        self.dp_ratio.set(ratio);
        if self.ctx.is_null() {
            return;
        }
        mark_ui_dirty();
        unsafe { ffi::rmlui_context_set_dp_ratio(self.ctx, ratio) };
    }

    pub fn mouse_move(&self, x: i32, y: i32) {
        if !self.ctx.is_null() {
            mark_ui_dirty();
            unsafe { ffi::rmlui_process_mouse_move(self.ctx, x, y) };
        }
    }

    pub fn mouse_button(&self, button: i32, down: bool) {
        if self.ctx.is_null() {
            return;
        }
        mark_ui_dirty();
        unsafe {
            if down {
                ffi::rmlui_process_mouse_button_down(self.ctx, button);
            } else {
                ffi::rmlui_process_mouse_button_up(self.ctx, button);
            }
        }
    }

    /// `delta`: engine convention — positive = scroll up (away). RmlUi wants the opposite
    /// (positive = down); same as official SDL/GLFW backends (`-y`).
    pub fn mouse_wheel(&self, delta: f32) {
        if self.ctx.is_null() || delta == 0.0 {
            return;
        }
        mark_ui_dirty();
        unsafe { ffi::rmlui_process_mouse_wheel(self.ctx, -delta) };
    }

    pub fn key_down(&self, rml_key: i32) {
        if self.ctx.is_null() || rml_key == 0 {
            return;
        }
        mark_ui_dirty();
        unsafe { ffi::rmlui_process_key_down(self.ctx, rml_key) };
    }

    pub fn key_up(&self, rml_key: i32) {
        if self.ctx.is_null() || rml_key == 0 {
            return;
        }
        mark_ui_dirty();
        unsafe { ffi::rmlui_process_key_up(self.ctx, rml_key) };
    }

    pub fn text_input(&self, codepoint: u32) {
        if self.ctx.is_null() || codepoint == 0 {
            return;
        }
        mark_ui_dirty();
        unsafe { ffi::rmlui_process_text_input(self.ctx, codepoint) };
    }

    /// Update RmlUi and collect GPU commands for the wgpu UI pass.
    ///
    /// Skips the update when `GetNextUpdateDelay` has not elapsed and nothing
    /// (pointer, keys, data model, resize) requested a frame. A render whose
    /// draw list matches the previous one returns `None` so the GPU mesh stays.
    pub fn frame(&self) -> Option<UiFrame> {
        if !self.is_ready() {
            return None;
        }
        let now = ui_elapsed_secs();
        let next_at = f64::from_bits(NEXT_UPDATE_AT.load(Ordering::Relaxed));
        let force = UI_FORCE.swap(false, Ordering::Relaxed);
        if !force && now < next_at {
            return None;
        }
        unsafe { ffi::rmlui_context_update(self.ctx) };
        let delay = unsafe { ffi::rmlui_context_next_update_delay(self.ctx) };
        let next = if delay.is_finite() {
            now + delay
        } else {
            f64::INFINITY
        };
        NEXT_UPDATE_AT.store(next.to_bits(), Ordering::Relaxed);
        with_geom(|g| g.begin_frame());
        unsafe {
            ffi::rmlui_context_render(self.ctx);
        }
        with_geom(|g| g.take_frame_if_changed()).flatten()
    }
}

pub(crate) fn mark_ui_dirty() {
    UI_FORCE.store(true, Ordering::Relaxed);
}

pub(crate) fn ui_elapsed_secs() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

impl Drop for UiRuntime {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe { ffi::rmlui_context_destroy(self.ctx) };
            self.ctx = ptr::null_mut();
        }
        if self.initialized {
            // Surfaces own contexts/documents and must die first.
            surface::destroy_all();
            unsafe { ffi::rmlui_shutdown() };
            self.initialized = false;
            log::info!(target: "ui", "Rml::Shutdown");
        }
        *GEOM.lock().unwrap() = None;
        *DUMMY_RENDER_API.lock().unwrap() = None;
        *CALLBACKS.lock().unwrap() = None;
    }
}

fn load_default_fonts() -> bool {
    let base = assets::ui_base_path();
    let mut ok = false;
    for name in [
        "LatoLatin-Regular.ttf",
        "LatoLatin-Bold.ttf",
        "VT323-Regular.ttf",
        "font.ttf",
    ] {
        let rel = format!("{}{name}", paths::FONTS);
        let abs = base.join(&rel);
        if !abs.is_file() {
            continue;
        }
        let Ok(c) = CString::new(rel.clone()) else {
            continue;
        };
        if unsafe { ffi::rmlui_load_font(c.as_ptr()) } != 0 {
            log::info!(target: "ui", "font loaded: {rel}");
            ok = true;
        }
    }
    ok
}

/// Same callback shape as `UiRuntime::new`; boxes leak — process-local
/// singletons the C side keeps pointing at until shutdown.
#[cfg(test)]
pub(crate) fn test_callbacks() -> Box<ffi::rmlui_callbacks_t> {
    Box::new(ffi::rmlui_callbacks_t {
        get_time: Some(cb_get_time),
        log: Some(cb_log),
        get_base_path: Some(cb_base_path),
        scissor: None,
        execute_command: Some(cb_execute_command),
        client_in_game: Some(cb_client_in_game),
        get_cvar_float: None,
        get_cvar_string: None,
        get_files_list: Some(cb_get_files_list),
        load_file: None,
    })
}

pub(crate) fn make_dummy_render_api() -> Box<ffi::rmlui_render_api_t> {
    unsafe extern "C" fn noop_set2d(_: c_int) {}
    unsafe extern "C" fn noop_i(_: c_int) {}
    unsafe extern "C" fn noop() {}
    unsafe extern "C" fn noop_color(_: u8, _: u8, _: u8, _: u8) {}
    unsafe extern "C" fn noop_uv(_: f32, _: f32) {}
    unsafe extern "C" fn noop_v(_: f32, _: f32, _: f32) {}
    unsafe extern "C" fn noop_bind(_: c_int, _: u32) {}
    unsafe extern "C" fn noop_load(_: *const c_char, _: *const u8, _: usize, _: c_int) -> c_int {
        0
    }
    unsafe extern "C" fn noop_create(
        _: *const c_char,
        _: c_int,
        _: c_int,
        _: *const std::ffi::c_void,
        _: u32,
    ) -> c_int {
        1
    }
    unsafe extern "C" fn noop_fill(
        _: c_int,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: u8,
        _: u8,
        _: u8,
        _: u8,
    ) {
    }
    unsafe extern "C" fn noop_stretch(
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: c_int,
    ) {
    }

    Box::new(ffi::rmlui_render_api_t {
        R_Set2DMode: Some(noop_set2d),
        TriRenderMode: Some(noop_i),
        Begin: Some(noop_i),
        End: Some(noop),
        Color4ub: Some(noop_color),
        TexCoord2f: Some(noop_uv),
        Vertex3f: Some(noop_v),
        GL_Bind: Some(noop_bind),
        GL_SetRenderMode: Some(noop_i),
        GL_LoadTexture: Some(noop_load),
        GL_CreateTexture: Some(noop_create),
        FillRGBA: Some(noop_fill),
        R_DrawStretchPic: Some(noop_stretch),
        white_texture: 1,
    })
}

static BASE_PATH_CACHED: std::sync::Mutex<Option<CString>> = std::sync::Mutex::new(None);

unsafe extern "C" fn cb_get_time() -> f64 {
    ui_elapsed_secs()
}

unsafe extern "C" fn cb_log(level: c_int, msg: *const c_char) {
    if msg.is_null() {
        return;
    }
    let s = CStr::from_ptr(msg).to_string_lossy();
    match level {
        0 => log::error!(target: "rmlui", "{s}"),
        1 => log::warn!(target: "rmlui", "{s}"),
        _ => log::info!(target: "rmlui", "{s}"),
    }
}

unsafe extern "C" fn cb_base_path() -> *const c_char {
    let path = assets::ui_base_path();
    let c = CString::new(path.to_string_lossy().as_ref()).unwrap_or_default();
    let mut slot = BASE_PATH_CACHED.lock().unwrap();
    *slot = Some(c);
    slot.as_ref().unwrap().as_ptr()
}

unsafe extern "C" fn cb_execute_command(cmd: *const c_char) {
    if cmd.is_null() {
        return;
    }
    let s = CStr::from_ptr(cmd).to_string_lossy().trim().to_string();
    if s.is_empty() {
        return;
    }
    if let Ok(mut q) = UI_CMD_QUEUE.lock() {
        q.push_back(s);
    }
}

unsafe extern "C" fn cb_client_in_game() -> c_int {
    if CLIENT_IN_GAME.load(Ordering::SeqCst) {
        1
    } else {
        0
    }
}

unsafe extern "C" fn cb_get_files_list(pattern: *const c_char) -> *const c_char {
    let empty = {
        let mut slot = FILES_LIST_JSON.lock().unwrap();
        *slot = Some(CString::new("[]").unwrap_or_default());
        slot.as_ref().unwrap().as_ptr()
    };
    if pattern.is_null() {
        return empty;
    }
    let pat = CStr::from_ptr(pattern).to_string_lossy();
    // Virtual: LAN browser list (JSON array of server objects).
    if pat == "lan://servers" {
        if let Ok(g) = LAN_SERVERS_JSON.lock() {
            if let Some(ref c) = *g {
                return c.as_ptr();
            }
        }
        return empty;
    }
    let Some(root) = CONTENT_ROOT.lock().ok().and_then(|g| g.clone()) else {
        return empty;
    };
    let json = list_files_json(&root, pat.as_ref());
    let mut slot = FILES_LIST_JSON.lock().unwrap();
    *slot = Some(CString::new(json).unwrap_or_else(|_| CString::new("[]").unwrap()));
    slot.as_ref().unwrap().as_ptr()
}

/// Support patterns like `maps/*.bsp` relative to content root.
fn list_files_json(root: &Path, pattern: &str) -> String {
    let pattern = pattern.trim().replace('\\', "/");
    let (dir_rel, suffix) = match pattern.rsplit_once('/') {
        Some((d, mask)) => (d, mask),
        None => ("", pattern.as_str()),
    };
    let ext = suffix.strip_prefix('*').unwrap_or(suffix);
    let dir = if dir_rel.is_empty() {
        root.to_path_buf()
    } else {
        root.join(dir_rel)
    };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return "[]".into();
    };
    let mut names: Vec<String> = Vec::new();
    for ent in rd.flatten() {
        let path = ent.path();
        if !path.is_file() {
            continue;
        }
        let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !ext.is_empty()
            && !fname
                .to_ascii_lowercase()
                .ends_with(&ext.to_ascii_lowercase())
        {
            continue;
        }
        let rel = if dir_rel.is_empty() {
            fname.to_string()
        } else {
            format!("{dir_rel}/{fname}")
        };
        names.push(rel.replace('\\', "/"));
    }
    names.sort();
    let mut out = String::from("[");
    for (i, n) in names.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&n.replace('"', ""));
        out.push('"');
    }
    out.push(']');
    out
}

// ── C exports expected by rmlui-sys rust_bridge ─────────────────

struct FileState {
    data: Vec<u8>,
    pos: usize,
}

#[no_mangle]
pub unsafe extern "C" fn rust_create_renderer(
    _api: *const ffi::rmlui_render_api_t,
    _cb: *const ffi::rmlui_callbacks_t,
) -> *mut std::ffi::c_void {
    // Soft renderer already created in UiRuntime::new
    std::ptr::dangling_mut::<std::ffi::c_void>()
}

#[no_mangle]
pub unsafe extern "C" fn rust_destroy_renderer(_h: *mut std::ffi::c_void) {}

#[no_mangle]
pub unsafe extern "C" fn rust_render_set_viewport(_h: *mut std::ffi::c_void, w: c_int, h: c_int) {
    with_geom_target(|r| r.resize(w as u32, h as u32));
}

#[no_mangle]
pub unsafe extern "C" fn rust_render_begin_frame(_h: *mut std::ffi::c_void) {}

#[no_mangle]
pub unsafe extern "C" fn rust_render_end_frame(_h: *mut std::ffi::c_void) {}

#[no_mangle]
pub unsafe extern "C" fn rust_compile_geometry(
    vertices: *const u8,
    stride: c_int,
    num_vertices: c_int,
    indices: *const i32,
    num_indices: c_int,
) -> u32 {
    if vertices.is_null() || stride <= 0 || num_vertices <= 0 {
        return 0;
    }
    let nv = num_vertices as usize;
    let stride = stride as usize;
    let bytes = std::slice::from_raw_parts(vertices, stride * nv);
    let idx = if indices.is_null() || num_indices <= 0 {
        &[][..]
    } else {
        std::slice::from_raw_parts(indices, num_indices as usize)
    };
    with_geom_target(|r| r.compile_geometry(bytes, stride, nv, idx)).unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn rust_release_geometry(id: u32) {
    with_geom_target(|r| r.release_geometry(id));
}

#[no_mangle]
pub unsafe extern "C" fn rust_draw_geometry(id: u32, tx: f32, ty: f32, texture: usize) {
    with_geom_target(|r| r.draw_geometry(id, tx, ty, texture));
}

#[no_mangle]
pub unsafe extern "C" fn rust_load_texture(
    _h: *mut std::ffi::c_void,
    source: *const c_char,
    out_w: *mut c_int,
    out_h: *mut c_int,
) -> usize {
    if source.is_null() {
        return 0;
    }
    let rel = CStr::from_ptr(source).to_string_lossy();
    let path = assets::resolve_texture_path(&rel).unwrap_or_else(|| {
        let s = rel.as_ref();
        let p = Path::new(s);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            assets::ui_base_path().join(s)
        }
    });
    match image::open(&path) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            if !out_w.is_null() {
                *out_w = w as i32;
            }
            if !out_h.is_null() {
                *out_h = h as i32;
            }
            let nearest = path
                .to_string_lossy()
                .replace('\\', "/")
                .contains("/textures/");
            let id = with_geom_target(|r| r.load_texture_rgba(rgba.into_raw(), w, h, nearest))
                .unwrap_or(0);
            id as usize
        }
        Err(e) => {
            log::warn!(target: "ui", "texture load failed {}: {e}", path.display());
            0
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rust_generate_texture(
    _h: *mut std::ffi::c_void,
    rgba: *const u8,
    width: c_int,
    height: c_int,
) -> usize {
    if rgba.is_null() || width <= 0 || height <= 0 {
        let id = with_geom(|r| r.white_id()).unwrap_or(1);
        return id as usize;
    }
    let n = (width * height * 4) as usize;
    let slice = std::slice::from_raw_parts(rgba, n);
    let id = with_geom(|r| r.generate_texture(slice, width, height)).unwrap_or(1);
    id as usize
}

#[no_mangle]
pub unsafe extern "C" fn rust_release_texture(_h: *mut std::ffi::c_void, tex: usize) {
    with_geom_target(|r| r.release_texture(tex as u32));
}

#[no_mangle]
pub unsafe extern "C" fn rust_scissor_enable(_h: *mut std::ffi::c_void, enable: c_int) {
    with_geom_target(|r| r.set_scissor(enable != 0));
}

#[no_mangle]
pub unsafe extern "C" fn rust_scissor_region(
    _h: *mut std::ffi::c_void,
    x: c_int,
    y: c_int,
    w: c_int,
    h: c_int,
) {
    with_geom_target(|r| r.set_scissor_region(x, y, w, h));
}

#[no_mangle]
pub unsafe extern "C" fn rust_create_system(
    _cb: *const ffi::rmlui_callbacks_t,
) -> *mut std::ffi::c_void {
    std::ptr::dangling_mut::<std::ffi::c_void>()
}

#[no_mangle]
pub unsafe extern "C" fn rust_destroy_system(_h: *mut std::ffi::c_void) {}

#[no_mangle]
pub unsafe extern "C" fn rust_system_get_elapsed_time(_h: *mut std::ffi::c_void) -> f64 {
    cb_get_time()
}

#[no_mangle]
pub unsafe extern "C" fn rust_system_log_message(
    _h: *mut std::ffi::c_void,
    level: c_int,
    msg: *const c_char,
) -> c_int {
    cb_log(level, msg);
    1
}

#[no_mangle]
pub unsafe extern "C" fn rust_file_open(path: *const c_char) -> *mut std::ffi::c_void {
    if path.is_null() {
        return ptr::null_mut();
    }
    let rel = CStr::from_ptr(path).to_string_lossy();
    let full = assets::ui_base_path().join(rel.as_ref());
    match std::fs::read(&full) {
        Ok(data) => Box::into_raw(Box::new(FileState { data, pos: 0 })) as *mut _,
        Err(e) => {
            log::warn!(target: "ui", "file not found: {} ({e})", full.display());
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rust_file_close(f: *mut std::ffi::c_void) {
    if !f.is_null() {
        drop(Box::from_raw(f as *mut FileState));
    }
}

#[no_mangle]
pub unsafe extern "C" fn rust_file_read(
    f: *mut std::ffi::c_void,
    buf: *mut std::ffi::c_void,
    size: usize,
) -> usize {
    if f.is_null() || buf.is_null() {
        return 0;
    }
    let file = &mut *(f as *mut FileState);
    let out = std::slice::from_raw_parts_mut(buf as *mut u8, size);
    let remain = file.data.len().saturating_sub(file.pos);
    let n = remain.min(size);
    out[..n].copy_from_slice(&file.data[file.pos..file.pos + n]);
    file.pos += n;
    n
}

#[no_mangle]
pub unsafe extern "C" fn rust_file_seek(
    f: *mut std::ffi::c_void,
    offset: i64,
    origin: c_int,
) -> c_int {
    if f.is_null() {
        return -1;
    }
    let file = &mut *(f as *mut FileState);
    let new_pos = match origin {
        0 => offset,
        1 => file.pos as i64 + offset,
        2 => file.data.len() as i64 + offset,
        _ => return -1,
    };
    if new_pos < 0 || new_pos as usize > file.data.len() {
        return -1;
    }
    file.pos = new_pos as usize;
    0
}

#[no_mangle]
pub unsafe extern "C" fn rust_file_tell(f: *mut std::ffi::c_void) -> usize {
    if f.is_null() {
        return 0;
    }
    (*(f as *mut FileState)).pos
}

#[cfg(test)]
mod data_model_apply_tests {
    use super::*;

    /// RmlUi state is process-global (`rmlui_init` guards on a renderer
    /// singleton); these tests boot full interpreter+context runs, so they
    /// must not race each other in the default parallel test harness.
    /// Serialization uses the crate-level [`RML_TEST_LOCK`].

    /// Surface RmlUi/C-side diagnostics (model lookup, op errors) in test output.
    struct StderrLogger;
    impl log::Log for StderrLogger {
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

    const DEMO_RML: &str = r#"<rml>
<head>
<style>
	body { }
	div { }
</style>
</head>
<body data-model="demo">
	<div id="flat" data-class-five="health == 5"></div>
	<div id="lab" data-class-z="label == 'z'"></div>
	<div data-for="rows">
		<div class="rowitem" data-class-a="it.icon == 'a'"></div>
	</div>
</body>
</rml>
"#;

    /// Headless end-to-end: boots the RmlUi interpreter + context (dummy
    /// renderer, no fonts — the document binds data, no text nodes), opens a
    /// Lua `demo` model exactly like game UI scripts do, and checks batched
    /// writes through the live data bindings.
    #[test]
    fn data_model_apply_batch_writes_and_resize() {
        let _rml = RML_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _ = log::set_boxed_logger(Box::new(StderrLogger));
        log::set_max_level(log::LevelFilter::Warn);

        let base = std::env::temp_dir().join(format!("deerpunk-ui-apply-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("demo_test.rml"), DEMO_RML).unwrap();
        assets::set_ui_base_path(base.clone());

        // Same callback shape as UiRuntime::new; boxes leak — process-local
        // singletons the C side keeps pointing at until shutdown.
        let callbacks: &'static ffi::rmlui_callbacks_t =
            Box::leak(Box::new(ffi::rmlui_callbacks_t {
                get_time: Some(cb_get_time),
                log: Some(cb_log),
                get_base_path: Some(cb_base_path),
                scissor: None,
                execute_command: Some(cb_execute_command),
                client_in_game: Some(cb_client_in_game),
                get_cvar_float: None,
                get_cvar_string: None,
                get_files_list: Some(cb_get_files_list),
                load_file: None,
            }));
        let render_api: &'static ffi::rmlui_render_api_t = Box::leak(make_dummy_render_api());
        assert_eq!(
            unsafe { ffi::rmlui_init(64, 64, callbacks, render_api) },
            1,
            "rmlui_init failed"
        );
        let ctx = unsafe { ffi::rmlui_context_create(c"main".as_ptr()) };
        assert!(!ctx.is_null(), "context_create failed");

        // Open the model from Lua (same idiom as `ui/scripts/init.lua`).
        unsafe {
            ffi::rmlui_lua_do_string(
                c"demo = { health = 0, label = 'x', rows = { { icon = 'a' }, { icon = 'b' } } }
                assert(rmlui.contexts['main']:OpenDataModel('demo', demo))
                assert(demo.rows[2].icon == 'b')"
                    .as_ptr(),
            );
        }

        let doc = unsafe { ffi::rmlui_document_load(ctx, c"demo_test.rml".as_ptr()) };
        assert!(!doc.is_null(), "document load failed");
        let document = UiDocument {
            handle: doc,
            written: RefCell::new(HashMap::new()),
        };

        // Flat, string-key, and nested array-element writes in one batch.
        let ops = vec![
            DataModelOp {
                path: "health".into(),
                value: DataModelOpValue::Number(5.0),
            },
            DataModelOp {
                path: "label".into(),
                value: DataModelOpValue::String("z".into()),
            },
            DataModelOp {
                path: "rows.2.icon".into(),
                value: DataModelOpValue::String("c".into()),
            },
        ];
        assert_eq!(document.data_model_apply(&ops), Ok(()));

        // Resize up: 2 → 4 rows (new rows are empty tables).
        let ops = vec![DataModelOp {
            path: "rows".into(),
            value: DataModelOpValue::Resize(4),
        }];
        assert_eq!(document.data_model_apply(&ops), Ok(()));

        unsafe {
            ffi::rmlui_context_update(ctx);
        }
        let tree = document.tree_text(None).expect("tree dump");
        assert!(
            tree.contains("#flat.five"),
            "health == 5 not bound:\n{tree}"
        );
        assert!(tree.contains("#lab.z"), "label == 'z' not bound:\n{tree}");
        assert_eq!(
            tree.matches("rowitem").count(),
            4,
            "rows resize to 4 not bound:\n{tree}"
        );

        // Row 1 kept its pre-resize element binding (icon still 'a').
        assert!(
            tree.contains("rowitem.a"),
            "row 1 icon binding lost:\n{tree}"
        );

        // Writes into the rebuilt array flip bindings on the next update.
        let ops = vec![DataModelOp {
            path: "rows.2.icon".into(),
            value: DataModelOpValue::String("a".into()),
        }];
        assert_eq!(document.data_model_apply(&ops), Ok(()));
        unsafe {
            ffi::rmlui_context_update(ctx);
        }
        let tree = document.tree_text(None).expect("tree dump");
        assert_eq!(
            tree.matches("rowitem.a").count(),
            2,
            "rows.2.icon write not bound after resize:\n{tree}"
        );

        // Resize down: 4 → 1 row.
        let ops = vec![DataModelOp {
            path: "rows".into(),
            value: DataModelOpValue::Resize(1),
        }];
        assert_eq!(document.data_model_apply(&ops), Ok(()));
        unsafe {
            ffi::rmlui_context_update(ctx);
        }
        let tree = document.tree_text(None).expect("tree dump");
        assert_eq!(
            tree.matches("rowitem").count(),
            1,
            "rows resize to 1 not bound:\n{tree}"
        );

        // A bad path fails the batch at its own op index.
        let ops = vec![
            DataModelOp {
                path: "health".into(),
                value: DataModelOpValue::Number(6.0),
            },
            DataModelOp {
                path: "rows.9.icon".into(),
                value: DataModelOpValue::String("x".into()),
            },
        ];
        assert_eq!(
            document.data_model_apply(&ops),
            Err(DataModelApplyError::Op(1))
        );

        // Empty batches succeed; a numeric root or empty path fails.
        assert_eq!(document.data_model_apply(&[]), Ok(()));
        let ops = vec![DataModelOp {
            path: "1.foo".into(),
            value: DataModelOpValue::Number(1.0),
        }];
        assert_eq!(
            document.data_model_apply(&ops),
            Err(DataModelApplyError::Op(0))
        );
        let ops = vec![DataModelOp {
            path: String::new(),
            value: DataModelOpValue::Number(1.0),
        }];
        assert_eq!(
            document.data_model_apply(&ops),
            Err(DataModelApplyError::Op(0))
        );

        drop(document);
        unsafe {
            ffi::rmlui_context_destroy(ctx);
            ffi::rmlui_shutdown();
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Probe: Lua-backed array lifecycle — wholesale replacement from Lua
    /// (`m.palette = out`, the game's refresh_palette) vs the engine resize
    /// op, observed through live data-class bindings and RmlUi warnings.
    #[test]
    fn data_model_lua_array_replacement_probe() {
        let _rml = RML_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _ = log::set_boxed_logger(Box::new(StderrLogger));
        log::set_max_level(log::LevelFilter::Warn);

        const PAL_RML: &str = r#"<rml>
<head><style> body { } div { } </style></head>
<body data-model="pal_test">
	<div data-for="slot : palette">
		<div class="s" data-class-selected="slot.selected" data-class-it="slot.item_id == 40"></div>
	</div>
</body>
</rml>
"#;

        let base =
            std::env::temp_dir().join(format!("deerpunk-ui-palprobe-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("pal_test.rml"), PAL_RML).unwrap();
        assets::set_ui_base_path(base.clone());

        let callbacks: &'static ffi::rmlui_callbacks_t =
            Box::leak(Box::new(ffi::rmlui_callbacks_t {
                get_time: Some(cb_get_time),
                log: Some(cb_log),
                get_base_path: Some(cb_base_path),
                scissor: None,
                execute_command: Some(cb_execute_command),
                client_in_game: Some(cb_client_in_game),
                get_cvar_float: None,
                get_cvar_string: None,
                get_files_list: Some(cb_get_files_list),
                load_file: None,
            }));
        let render_api: &'static ffi::rmlui_render_api_t = Box::leak(make_dummy_render_api());
        assert_eq!(unsafe { ffi::rmlui_init(64, 64, callbacks, render_api) }, 1);
        let ctx = unsafe { ffi::rmlui_context_create(c"main".as_ptr()) };
        assert!(!ctx.is_null());

        // Same shape as inventory.lua: model userdata is kept, the plain
        // table handed to OpenDataModel is abandoned; refill replaces the
        // whole `palette` array with a fresh table.
        unsafe {
            ffi::rmlui_lua_do_string(
                c"pal = { palette = {}, show_grid = false }
pal_model = rmlui.contexts['main']:OpenDataModel('pal_test', pal)
function pal_refill(sel_id, n)
    local out = {}
    for i = 1, n do
        out[i] = { item_id = i, selected = (i == sel_id), icon = '',
            icon_block = false, show_icon = false, show_count = false,
            count = '', show_dur = false, dur_width = '0%',
            tip_title = '', tip_body = '' }
    end
    pal_model.palette = out
end
pal_refill(40, 90)"
                    .as_ptr(),
            );
        }

        let doc = unsafe { ffi::rmlui_document_load(ctx, c"pal_test.rml".as_ptr()) };
        assert!(!doc.is_null(), "document load failed");
        let document = UiDocument {
            handle: doc,
            written: RefCell::new(HashMap::new()),
        };

        let dump = |what: &str| {
            let cmds = drain_commands();
            for c in &cmds {
                eprintln!("LUA {c}");
            }
            unsafe { ffi::rmlui_context_update(ctx) };
            let tree = document.tree_text(None).expect("tree dump");
            let total = tree.matches("div.s").count();
            let sel = tree.matches("div.s.selected").count();
            let it40 = tree.matches("div.s.it").count();
            eprintln!("PROBE {what}: total={total} selected={sel} it40={it40}");
            (total, sel, it40)
        };

        unsafe {
            ffi::rmlui_lua_do_string(
                c"cmd('STATE model=' .. tostring(pal_model ~= nil) .. ' p1=' .. tostring(pal_model.palette and pal_model.palette[1].item_id or 'nil'))"
                    .as_ptr(),
            );
        }
        dump("initial fill 90 sel=40");
        {
            let tree = document.tree_text(None).expect("tree dump");
            let lines: Vec<&str> = tree.lines().filter(|l| l.contains(".it")).collect();
            eprintln!("TREE IT LINES phase1: {:?}", lines);
        }
        unsafe {
            ffi::rmlui_lua_do_string(
                c"pal_refill(40, 90) cmd('REFILL 90/' .. tostring(pal_model.palette[1].item_id))"
                    .as_ptr(),
            );
        }
        dump("refill same 90 sel=40");
        // Move the selection: stale caches would keep item 40 selected.
        unsafe {
            ffi::rmlui_lua_do_string(
                c"pal_refill(5, 90) cmd('REFILL 90/' .. tostring(pal_model.palette[1].item_id))"
                    .as_ptr(),
            );
        }
        dump("refill 90 sel=5");
        // Shrink then grow — the observed warning shape.
        unsafe {
            ffi::rmlui_lua_do_string(c"pal_refill(5, 22) cmd('REFILL 22/has40=' .. tostring(pal_model.palette[40] ~= nil) .. ' has22=' .. tostring(pal_model.palette[22] ~= nil))".as_ptr());
        }
        let (_, _, it40_after_shrink) = dump("refill 22 sel=5");
        unsafe {
            ffi::rmlui_lua_do_string(
                c"pal_refill(5, 90) cmd('REFILL 90B/' .. tostring(pal_model.palette[1].item_id))"
                    .as_ptr(),
            );
        }
        let (total_grow, sel_grow, _) = dump("refill 90 sel=5 (grow)");

        // Same model, engine-op path: does the binding follow Resize?
        let ops = vec![DataModelOp {
            path: "palette".into(),
            value: DataModelOpValue::Resize(22),
        }];
        assert_eq!(document.data_model_apply(&ops), Ok(()));
        let (total_engine, _, it40_engine) = dump("engine resize 90->22");

        drop(document);
        unsafe {
            ffi::rmlui_context_destroy(ctx);
            ffi::rmlui_shutdown();
        }
        let _ = std::fs::remove_dir_all(&base);
        // The shrink must have removed item 40 from the model view.
        assert_eq!(
            it40_after_shrink, 0,
            "shrink to 22 kept a stale item-40 binding"
        );
        assert_eq!(
            total_engine, 22,
            "engine resize on the same model shrank the view"
        );
        let _ = (total_grow, sel_grow, it40_engine);
    }
}
