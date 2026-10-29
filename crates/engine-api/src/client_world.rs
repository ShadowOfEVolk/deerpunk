use crate::components::{no_prefab_apply, ComponentRegisterDesc};
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
    destroy_fn: extern "C" fn(*mut u8, u64),
    remove_fn: extern "C" fn(*mut u8, u64, u64),
    is_alive_fn: extern "C" fn(*mut u8, u64) -> bool,
    component_register_fn: extern "C" fn(*mut u8, *const ComponentRegisterDesc) -> u64,
    query_fn: extern "C" fn(
        *mut u8,
        *const u64,
        usize,
        *const u64,
        usize,
        *mut u64,
        usize,
    ) -> usize,
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
            destroy_fn: unsafe { std::mem::transmute(resolve(r, "v1:client_world:destroy")) },
            remove_fn: unsafe { std::mem::transmute(resolve(r, "v1:client_world:remove")) },
            is_alive_fn: unsafe { std::mem::transmute(resolve(r, "v1:client_world:is_alive")) },
            component_register_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:client_world:component_register"))
            },
            query_fn: unsafe { std::mem::transmute(resolve(r, "v1:client_world:query")) },
        }
    }

    pub fn spawn(&self) -> EntityId {
        (self.spawn_fn)(self.ctx)
    }

    pub fn destroy(&self, entity: EntityId) {
        (self.destroy_fn)(self.ctx, entity);
    }

    pub fn remove(&self, entity: EntityId, comp: ComponentId) {
        deerpunk_engine_common::ecs_validate::assert_component_id(comp, "ClientWorldApi::remove");
        (self.remove_fn)(self.ctx, entity, comp);
    }

    pub fn is_alive(&self, entity: EntityId) -> bool {
        (self.is_alive_fn)(self.ctx, entity)
    }

    pub fn has(&self, entity: EntityId, comp: ComponentId) -> bool {
        deerpunk_engine_common::ecs_validate::assert_component_id(comp, "ClientWorldApi::has");
        (self.has_fn)(self.ctx, entity, comp)
    }

    pub fn get<T: Copy>(&self, entity: EntityId, comp: ComponentId) -> Option<T> {
        deerpunk_engine_common::ecs_validate::assert_component_id(comp, "ClientWorldApi::get");
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
        deerpunk_engine_common::ecs_validate::assert_component_id(comp, "ClientWorldApi::set_raw");
        deerpunk_engine_common::ecs_validate::assert_set_payload(size, "ClientWorldApi::set_raw");
        if ptr.is_null() {
            panic!("ClientWorldApi::set_raw: null data pointer");
        }
        (self.set_fn)(self.ctx, entity, comp, ptr, size);
    }

    pub fn set<T>(&self, entity: EntityId, comp: ComponentId, value: T) {
        deerpunk_engine_common::ecs_validate::assert_component_id(comp, "ClientWorldApi::set");
        let size = std::mem::size_of::<T>();
        deerpunk_engine_common::ecs_validate::assert_set_payload(size, "ClientWorldApi::set");
        let ptr = &value as *const T as *const std::ffi::c_void;
        let size = std::mem::size_of::<T>();
        self.set_raw(entity, comp, ptr, size);
        // Host `v1:client_world:set` clones `Arc` fields for heap-owning components; stack `value` drops here.
    }

    pub fn component_register(&self, desc: &ComponentRegisterDesc) -> Result<ComponentId, String> {
        let id = (self.component_register_fn)(self.ctx, desc);
        if id == 0 {
            Err("client component_register returned 0 (see host log / panic above)".into())
        } else {
            Ok(id)
        }
    }

    pub fn register_component(
        &self,
        name: &str,
        size: usize,
        align: usize,
    ) -> Result<ComponentId, String> {
        if size == 0 {
            return Err(format!(
                "register_component \"{name}\": size must be > 0 \
                 (zero-sized types need at least one byte, e.g. `_tag: u8`)"
            ));
        }
        let desc = ComponentRegisterDesc::new(
            name,
            size,
            align,
            std::ptr::null_mut(),
            no_prefab_apply(),
        );
        self.component_register(&desc)
    }

    pub fn query(&self, require: &[ComponentId], exclude: &[ComponentId]) -> Vec<EntityId> {
        let max = 4096;
        let mut buf = vec![0u64; max];
        let count = (self.query_fn)(
            self.ctx,
            require.as_ptr(),
            require.len(),
            exclude.as_ptr(),
            exclude.len(),
            buf.as_mut_ptr(),
            max,
        );
        buf.truncate(count);
        buf
    }
}
