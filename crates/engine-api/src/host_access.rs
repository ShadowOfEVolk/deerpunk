use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

pub struct HostAccessApi {
    ctx: *mut u8,
    flush_albedos_fn: extern "C" fn(*mut u8) -> i32,
    client_track_static_fn: extern "C" fn(*mut u8, u64) -> i32,
}

unsafe impl Send for HostAccessApi {}
unsafe impl Sync for HostAccessApi {}

impl HostAccessApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            flush_albedos_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:host:flush_pending_albedos"))
            },
            client_track_static_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:host:client_track_static"))
            },
        }
    }

    pub fn flush_pending_albedos(&self) {
        (self.flush_albedos_fn)(self.ctx);
    }

    pub fn client_track_static_entity(&self, entity: u64) {
        (self.client_track_static_fn)(self.ctx, entity);
    }
}
