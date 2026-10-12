use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use deerpunk_engine_common::ui_draw::UiFrame;

use crate::assets::{self, UiAssetSource};
use crate::draw_list::UiGeometryBuilder;
use crate::ffi;
use crate::paths;

static GEOM: Mutex<Option<UiGeometryBuilder>> = Mutex::new(None);
static START: Mutex<Option<Instant>> = Mutex::new(None);
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

/// Drain console commands queued by RmlUi Lua `cmd(...)`.
pub fn drain_commands() -> Vec<String> {
    let Ok(mut q) = UI_CMD_QUEUE.lock() else {
        return Vec::new();
    };
    q.drain(..).collect()
}

fn with_geom<R>(f: impl FnOnce(&mut UiGeometryBuilder) -> R) -> Option<R> {
    let mut g = GEOM.lock().ok()?;
    g.as_mut().map(f)
}

/// Loaded RML document handle (Panorama-style: document = UI unit).
pub struct UiDocument {
    handle: ffi::rmlui_handle_t,
}

impl UiDocument {
    pub fn show(&self) {
        if !self.handle.is_null() {
            unsafe { ffi::rmlui_document_show(self.handle) };
        }
    }

    pub fn hide(&self) {
        if !self.handle.is_null() {
            unsafe { ffi::rmlui_document_hide(self.handle) };
        }
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
    _assets: Box<dyn UiAssetSource>,
}

impl UiRuntime {
    pub fn new(assets: Box<dyn UiAssetSource>, screen: (u32, u32), context_name: &str) -> Self {
        assets::set_ui_base_path(assets.base().to_path_buf());
        *START.lock().unwrap() = Some(Instant::now());
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
            log::error!("[deerpunk-ui] rmlui_init failed");
            return Self {
                ctx: ptr::null_mut(),
                initialized: false,
                _assets: assets,
            };
        }

        if !load_default_fonts() {
            log::error!("[deerpunk-ui] no fonts under {}", assets.base().display());
            unsafe { ffi::rmlui_shutdown() };
            return Self {
                ctx: ptr::null_mut(),
                initialized: false,
                _assets: assets,
            };
        }

        let name = CString::new(context_name).unwrap_or_else(|_| CString::new("main").unwrap());
        let ctx = unsafe { ffi::rmlui_context_create(name.as_ptr()) };
        if ctx.is_null() {
            log::error!("[deerpunk-ui] context_create failed");
            unsafe { ffi::rmlui_shutdown() };
            return Self {
                ctx: ptr::null_mut(),
                initialized: false,
                _assets: assets,
            };
        }

        log::info!(
            "[deerpunk-ui] ready {}x{} base={}",
            w,
            h,
            assets.base().display()
        );
        Self {
            ctx,
            initialized: true,
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
            log::error!("[deerpunk-ui] failed to load document: {relative}");
            return None;
        }
        Some(UiDocument { handle })
    }

    pub fn resize(&self, w: u32, h: u32) {
        with_geom(|g| g.resize(w, h));
        unsafe {
            ffi::rmlui_set_viewport(w as i32, h as i32);
            if !self.ctx.is_null() {
                ffi::rmlui_context_set_dimensions(self.ctx, w as i32, h as i32);
            }
        }
    }

    pub fn mouse_move(&self, x: i32, y: i32) {
        if !self.ctx.is_null() {
            unsafe { ffi::rmlui_process_mouse_move(self.ctx, x, y) };
        }
    }

    pub fn mouse_button(&self, button: i32, down: bool) {
        if self.ctx.is_null() {
            return;
        }
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
        unsafe { ffi::rmlui_process_mouse_wheel(self.ctx, -delta) };
    }

    /// Update RmlUi and collect GPU geometry for the wgpu UI pass.
    pub fn frame(&self) -> Option<UiFrame> {
        if !self.is_ready() {
            return None;
        }
        with_geom(|g| {
            g.begin_frame();
            g.ensure_white_upload();
        });
        unsafe {
            ffi::rmlui_context_update(self.ctx);
            ffi::rmlui_context_render(self.ctx);
        }
        with_geom(|g| g.take_frame())
    }
}

impl Drop for UiRuntime {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe { ffi::rmlui_context_destroy(self.ctx) };
            self.ctx = ptr::null_mut();
        }
        if self.initialized {
            unsafe { ffi::rmlui_shutdown() };
            self.initialized = false;
            log::info!("[deerpunk-ui] Rml::Shutdown");
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
            log::info!("[deerpunk-ui] font loaded: {rel}");
            ok = true;
        }
    }
    ok
}

fn make_dummy_render_api() -> Box<ffi::rmlui_render_api_t> {
    unsafe extern "C" fn noop_set2d(_: c_int) {}
    unsafe extern "C" fn noop_i(_: c_int) {}
    unsafe extern "C" fn noop() {}
    unsafe extern "C" fn noop_color(_: u8, _: u8, _: u8, _: u8) {}
    unsafe extern "C" fn noop_uv(_: f32, _: f32) {}
    unsafe extern "C" fn noop_v(_: f32, _: f32, _: f32) {}
    unsafe extern "C" fn noop_bind(_: c_int, _: u32) {}
    unsafe extern "C" fn noop_load(
        _: *const c_char,
        _: *const u8,
        _: usize,
        _: c_int,
    ) -> c_int {
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
    START
        .lock()
        .ok()
        .and_then(|g| g.map(|t| t.elapsed().as_secs_f64()))
        .unwrap_or(0.0)
}

unsafe extern "C" fn cb_log(level: c_int, msg: *const c_char) {
    if msg.is_null() {
        return;
    }
    let s = CStr::from_ptr(msg).to_string_lossy();
    match level {
        0 => log::error!("[RmlUi] {s}"),
        1 => log::warn!("[RmlUi] {s}"),
        _ => log::info!("[RmlUi] {s}"),
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
        if !ext.is_empty() && !fname.to_ascii_lowercase().ends_with(&ext.to_ascii_lowercase()) {
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
    1 as *mut std::ffi::c_void
}

#[no_mangle]
pub unsafe extern "C" fn rust_destroy_renderer(_h: *mut std::ffi::c_void) {}

#[no_mangle]
pub unsafe extern "C" fn rust_render_set_viewport(_h: *mut std::ffi::c_void, w: c_int, h: c_int) {
    with_geom(|r| r.resize(w as u32, h as u32));
}

#[no_mangle]
pub unsafe extern "C" fn rust_render_begin_frame(_h: *mut std::ffi::c_void) {}

#[no_mangle]
pub unsafe extern "C" fn rust_render_end_frame(_h: *mut std::ffi::c_void) {}

#[no_mangle]
pub unsafe extern "C" fn rust_render_geometry(
    _h: *mut std::ffi::c_void,
    positions: *const f32,
    colours: *const u8,
    texcoords: *const f32,
    indices: *const i32,
    num_indices: c_int,
    tx: f32,
    ty: f32,
    texture: usize,
) {
    if positions.is_null() || colours.is_null() || indices.is_null() || num_indices <= 0 {
        return;
    }
    let n = num_indices as usize;
    // positions length unknown; use max index
    let idx = std::slice::from_raw_parts(indices, n);
    let max_v = idx.iter().copied().max().unwrap_or(0) as usize + 1;
    let pos = std::slice::from_raw_parts(positions, max_v * 2);
    let col = std::slice::from_raw_parts(colours, max_v * 4);
    let uv = if texcoords.is_null() {
        &[][..]
    } else {
        std::slice::from_raw_parts(texcoords, max_v * 2)
    };
    with_geom(|r| r.draw_indexed(pos, col, uv, idx, tx, ty, texture));
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
    let path = assets::ui_base_path().join(rel.as_ref());
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
            let id = with_geom(|r| r.load_texture_rgba(rgba.into_raw(), w, h)).unwrap_or(0);
            id as usize
        }
        Err(e) => {
            log::warn!("[deerpunk-ui] texture load failed {}: {e}", path.display());
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
    with_geom(|r| r.release_texture(tex as u32));
}

#[no_mangle]
pub unsafe extern "C" fn rust_scissor_enable(_h: *mut std::ffi::c_void, enable: c_int) {
    with_geom(|r| r.set_scissor(enable != 0));
}

#[no_mangle]
pub unsafe extern "C" fn rust_scissor_region(
    _h: *mut std::ffi::c_void,
    x: c_int,
    y: c_int,
    w: c_int,
    h: c_int,
) {
    with_geom(|r| r.set_scissor_region(x, y, w, h));
}

#[no_mangle]
pub unsafe extern "C" fn rust_create_system(
    _cb: *const ffi::rmlui_callbacks_t,
) -> *mut std::ffi::c_void {
    1 as *mut std::ffi::c_void
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
            log::warn!("[deerpunk-ui] file not found: {} ({e})", full.display());
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
pub unsafe extern "C" fn rust_file_seek(f: *mut std::ffi::c_void, offset: i64, origin: c_int) -> c_int {
    if f.is_null() {
        return -1;
    }
    let file = &mut *(f as *mut FileState);
    let new_pos = match origin {
        0 => offset as i64,
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
