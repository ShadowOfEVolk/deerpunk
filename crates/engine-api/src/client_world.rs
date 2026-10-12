use crate::Resolver;
use crate::{ComponentId, EntityId};

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

/// Client render/prediction world (host `client_engine.world`).
pub struct ClientWorldApi {
    ctx: *mut u8,
    get_fn: extern "C" fn(*mut u8, u64, u64) -> *mut std::ffi::c_void,
    set_fn: extern "C" fn(*mut u8, u64, u64, *const std::ffi::c_void, usize),
    has_fn: extern "C" fn(*mut u8, u64, u64) -> bool,
    spawn_fn: extern "C" fn(*mut u8) -> u64,
    component_init_raw_fn: extern "C" fn(*mut u8, *const u8, usize, usize, usize) -> u64,
}

unsafe impl Send for ClientWorldApi {}
unsafe impl Sync for ClientWorldApi {}

impl ClientWorldApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            get_fn: unsafe { std::mem::transmute(resolve(r, "v1:client_world:get")) },
            set_fn: unsafe { std::mem::transmute(resolve(r, "v1:client_world:set")) },
            has_fn: unsafe { std::mem::transmute(resolve(r, "v1:client_world:has")) },
            spawn_fn: unsafe { std::mem::transmute(resolve(r, "v1:client_world:spawn")) },
            component_init_raw_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:client_world:component_init_raw"))
            },
        }
    }

    pub fn spawn(&self) -> EntityId {
        (self.spawn_fn)(self.ctx)
    }

    pub fn has(&self, entity: EntityId, comp: ComponentId) -> bool {
        (self.has_fn)(self.ctx, entity, comp)
    }

    pub fn get<T: Copy>(&self, entity: EntityId, comp: ComponentId) -> Option<T> {
        let ptr = (self.get_fn)(self.ctx, entity, comp);
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { std::ptr::read_unaligned(ptr as *const T) })
        }
    }

    pub fn get_raw(&self, entity: EntityId, comp: ComponentId) -> Option<*const std::ffi::c_void> {
        let ptr = (self.get_fn)(self.ctx, entity, comp);
        if ptr.is_null() {
            None
        } else {
            Some(ptr as *const std::ffi::c_void)
        }
    }

    pub fn set_raw(&self, entity: EntityId, comp: ComponentId, ptr: *const std::ffi::c_void, size: usize) {
        (self.set_fn)(self.ctx, entity, comp, ptr, size);
    }

    pub fn set<T>(&self, entity: EntityId, comp: ComponentId, value: T) {
        let ptr = &value as *const T as *const std::ffi::c_void;
        let size = std::mem::size_of::<T>();
        self.set_raw(entity, comp, ptr, size);
        std::mem::forget(value);
    }

    pub fn component_init_raw(&self, name: &str, size: usize, align: usize) -> ComponentId {
        (self.component_init_raw_fn)(self.ctx, name.as_ptr(), name.len(), size, align)
    }
}
