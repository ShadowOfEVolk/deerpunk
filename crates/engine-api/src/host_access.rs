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

/// Mutable server-side host state for gamelib (world + assets).
#[repr(C)]
pub struct ServerHostAccess {
    pub world: *mut c_void,
    pub resources: *mut c_void,
    pub gamedir_ptr: *const u8,
    pub gamedir_len: usize,
}

/// Client prediction / render world + shared resources.
#[repr(C)]
pub struct ClientHostAccess {
    pub world: *mut c_void,
    pub resources: *mut c_void,
    pub gamedir_ptr: *const u8,
    pub gamedir_len: usize,
}

pub type ServerHostFn = extern "C" fn(*mut ServerHostAccess, *mut c_void);
pub type ClientHostFn = extern "C" fn(*mut ClientHostAccess, *mut c_void);

pub struct HostAccessApi {
    ctx: *mut u8,
    with_server_fn: extern "C" fn(*mut u8, ServerHostFn, *mut c_void),
    with_client_fn: extern "C" fn(*mut u8, ClientHostFn, *mut c_void),
    flush_albedos_fn: extern "C" fn(*mut u8) -> i32,
    client_track_static_fn: extern "C" fn(*mut u8, u64) -> i32,
}

unsafe impl Send for HostAccessApi {}
unsafe impl Sync for HostAccessApi {}

impl HostAccessApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            with_server_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:host:with_server"))
            },
            with_client_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:host:with_client"))
            },
            flush_albedos_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:host:flush_pending_albedos"))
            },
            client_track_static_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:host:client_track_static"))
            },
        }
    }

    pub fn with_server(&self, f: ServerHostFn, user: *mut c_void) {
        (self.with_server_fn)(self.ctx, f, user);
    }

    pub fn with_client(&self, f: ClientHostFn, user: *mut c_void) {
        (self.with_client_fn)(self.ctx, f, user);
    }

    pub fn flush_pending_albedos(&self) {
        (self.flush_albedos_fn)(self.ctx);
    }

    pub fn client_track_static_entity(&self, entity: u64) {
        (self.client_track_static_fn)(self.ctx, entity);
    }
}
