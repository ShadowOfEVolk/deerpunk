use std::ffi::c_void;

use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

pub type LoadFn = extern "C" fn(
    user: *mut c_void,
    path_ptr: *const u8,
    path_len: usize,
    query_ptr: *const u8,
    query_len: usize,
) -> *mut c_void;

pub type DropFn = extern "C" fn(*mut c_void);

#[repr(C)]
pub struct ResourceLoaderDesc {
    pub id_ptr: *const u8,
    pub id_len: usize,
    pub extensions_ptr: *const u8,
    pub extensions_len: usize,
    pub type_tag_ptr: *const u8,
    pub type_tag_len: usize,
    pub user_data: *mut c_void,
    pub load: LoadFn,
    pub drop_fn: DropFn,
}

pub struct ResourcesApi {
    ctx: *mut u8,
    register_fn: extern "C" fn(*mut u8, *const ResourceLoaderDesc) -> i32,
}

unsafe impl Send for ResourcesApi {}
unsafe impl Sync for ResourcesApi {}

impl ResourcesApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            register_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:resources:register"))
            },
        }
    }

    /// Register a loader on the host [`ResourceManager`].
    ///
    /// `load` returns an opaque pointer owned by the gamelib; host calls `drop_fn` on unload.
    pub fn register(&self, desc: &ResourceLoaderDesc) -> Result<(), String> {
        if (self.register_fn)(self.ctx, desc) == 0 {
            Ok(())
        } else {
            Err("resources.register failed".into())
        }
    }

    pub fn register_loader(
        &self,
        id: &str,
        extensions: &str,
        type_tag: &str,
        user_data: *mut c_void,
        load: LoadFn,
        drop_fn: DropFn,
    ) -> Result<(), String> {
        let desc = ResourceLoaderDesc {
            id_ptr: id.as_ptr(),
            id_len: id.len(),
            extensions_ptr: extensions.as_ptr(),
            extensions_len: extensions.len(),
            type_tag_ptr: type_tag.as_ptr(),
            type_tag_len: type_tag.len(),
            user_data,
            load,
            drop_fn,
        };
        self.register(&desc)
    }
}
