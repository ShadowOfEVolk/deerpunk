//! `v1:materials:*` — material load/create/set via methoddb FFI.
//!
//! Colocated with the host material system (not in engine_methoddb.rs trampoline).

use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

pub struct MaterialsApi {
    ctx: *mut u8,
    /// Load .mat asset by path → MaterialHandle (u32). 0 = failed.
    load_fn: extern "C" fn(*mut u8, *const u8, usize) -> u32,
    /// Create material at runtime: shader_path + defines + props_json → MaterialHandle.
    create_fn: extern "C" fn(*mut u8, *const u8, usize, *const u8, usize, *const u8, usize) -> u32,
    /// Hot-reload material file → 0 on success, -1 on error.
    reload_fn: extern "C" fn(*mut u8, *const u8, usize) -> i32,
    /// Update a texture property on an existing material → 0 ok, -1 error.
    set_texture_fn: extern "C" fn(*mut u8, u32, *const u8, usize, u32) -> i32,
}

unsafe impl Send for MaterialsApi {}
unsafe impl Sync for MaterialsApi {}

impl MaterialsApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            load_fn: unsafe { std::mem::transmute(resolve(r, "v1:materials:load")) },
            create_fn: unsafe { std::mem::transmute(resolve(r, "v1:materials:create")) },
            reload_fn: unsafe { std::mem::transmute(resolve(r, "v1:materials:reload")) },
            set_texture_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:materials:set_texture"))
            },
        }
    }

    /// Load a `.mat` asset by path. Returns MaterialHandle (0 = failed).
    pub fn load(&self, path: &str) -> u32 {
        let bytes = path.as_bytes();
        (self.load_fn)(self.ctx, bytes.as_ptr(), bytes.len())
    }

    /// Create a material at runtime.
    ///
    /// `shader_path` — e.g. `"file://shaders/pbr.wgsl"`.
    /// `defines` — comma-joined preprocessor defines, e.g. `"ALPHA_MASK,DOUBLE_SIDED"`.
    /// `props_json` — JSON-encoded `Vec<(String, PropertyValueSer)>`.
    /// Texture values may use `"slot:N"` to reference a pre-uploaded texture slot.
    pub fn create(&self, shader_path: &str, defines: &str, props_json: &[u8]) -> u32 {
        let sp = shader_path.as_bytes();
        let df = defines.as_bytes();
        (self.create_fn)(
            self.ctx,
            sp.as_ptr(),
            sp.len(),
            df.as_ptr(),
            df.len(),
            props_json.as_ptr(),
            props_json.len(),
        )
    }

    /// Hot-reload a material file (dev). 0 = ok, -1 = error.
    pub fn reload(&self, path: &str) -> i32 {
        let bytes = path.as_bytes();
        (self.reload_fn)(self.ctx, bytes.as_ptr(), bytes.len())
    }

    /// Update a texture property on an existing material.
    ///
    /// `handle` — MaterialHandle from `load`/`create`.
    /// `prop_name` — `"_BaseMap"`, `"_NormalMap"`, or `"_ORMMap"`.
    /// `slot` — texture slot from `renderer.upload_texture_raw` or
    /// `host.flush_pending_albedos`. Invalidates the GPU cache so the next
    /// draw re-uploads. All entities sharing the handle pick up the change.
    /// Returns 0 on success, -1 on error.
    pub fn set_texture(&self, handle: u32, prop_name: &str, slot: u32) -> i32 {
        let bytes = prop_name.as_bytes();
        (self.set_texture_fn)(self.ctx, handle, bytes.as_ptr(), bytes.len(), slot)
    }
}
