use crate::components::{no_prefab_apply, ComponentRegisterDesc};
use crate::world::WorldProgressSystemFn;
use crate::Resolver;
use crate::{ComponentId, EntityId};

pub use crate::world::WorldProgressSystemFn as ClientWorldProgressSystemFn;

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
    query_fn:
        extern "C" fn(*mut u8, *const u64, usize, *const u64, usize, *mut u64, usize) -> usize,
    register_observer_fn: extern "C" fn(*mut u8, *mut u8, usize, u32, u64) -> i32,
    system_register_fn: extern "C" fn(
        *mut u8,
        *const u8,
        usize,
        *const u8,
        usize,
        WorldProgressSystemFn,
        *mut std::ffi::c_void,
        f32,
    ) -> u64,
    system_set_interval_fn: extern "C" fn(*mut u8, u64, f32) -> i32,
    system_set_stage_fn: extern "C" fn(*mut u8, u64, *const u8, usize) -> i32,
}

unsafe impl Send for ClientWorldApi {}
unsafe impl Sync for ClientWorldApi {}

impl ClientWorldApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            get_fn: unsafe { crate::fn_from_erased(resolve(r, "v1:client_world:get")) },
            set_fn: unsafe { crate::fn_from_erased(resolve(r, "v1:client_world:set")) },
            has_fn: unsafe { crate::fn_from_erased(resolve(r, "v1:client_world:has")) },
            spawn_fn: unsafe { crate::fn_from_erased(resolve(r, "v1:client_world:spawn")) },
            destroy_fn: unsafe { crate::fn_from_erased(resolve(r, "v1:client_world:destroy")) },
            remove_fn: unsafe { crate::fn_from_erased(resolve(r, "v1:client_world:remove")) },
            is_alive_fn: unsafe { crate::fn_from_erased(resolve(r, "v1:client_world:is_alive")) },
            component_register_fn: unsafe {
                crate::fn_from_erased(resolve(r, "v1:client_world:component_register"))
            },
            query_fn: unsafe { crate::fn_from_erased(resolve(r, "v1:client_world:query")) },
            register_observer_fn: unsafe {
                crate::fn_from_erased(resolve(r, "v1:client_world:register_observer"))
            },
            system_register_fn: unsafe {
                crate::fn_from_erased(resolve(r, "v1:client_world:system_register"))
            },
            system_set_interval_fn: unsafe {
                crate::fn_from_erased(resolve(r, "v1:client_world:system_set_interval"))
            },
            system_set_stage_fn: unsafe {
                crate::fn_from_erased(resolve(r, "v1:client_world:system_set_stage"))
            },
        }
    }

    /// Flecs system on the client world. `run` receives each matching table.
    /// Component columns are `fields[term]` in query order and live only for that call.
    pub fn system_register(
        &self,
        name: &str,
        query: &str,
        run: WorldProgressSystemFn,
        user_data: *mut std::ffi::c_void,
        interval_secs: f32,
    ) -> u64 {
        (self.system_register_fn)(
            self.ctx,
            name.as_ptr(),
            name.len(),
            query.as_ptr(),
            query.len(),
            run,
            user_data,
            interval_secs,
        )
    }

    pub fn system_set_interval(&self, system_id: u64, interval_secs: f32) {
        let _ = (self.system_set_interval_fn)(self.ctx, system_id, interval_secs);
    }

    /// Move a client system onto a named phase (`renderer.prepare` runs after extract).
    pub fn system_set_stage(&self, system_id: u64, stage: &str) -> i32 {
        (self.system_set_stage_fn)(self.ctx, system_id, stage.as_ptr(), stage.len())
    }

    /// Flecs observer on the client world. `events`: bit0 OnAdd, bit1 OnSet.
    pub fn register_observer(
        &self,
        query: &str,
        on_add: bool,
        on_set: bool,
        callback: extern "C" fn(u64),
    ) -> bool {
        let mut flags = 0u32;
        if on_add {
            flags |= 1;
        }
        if on_set {
            flags |= 2;
        }
        let cb_ptr: u64 = callback as u64;
        (self.register_observer_fn)(
            self.ctx,
            query.as_ptr() as *mut u8,
            query.len(),
            flags,
            cb_ptr,
        ) != 0
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

    pub fn set_raw(
        &self,
        entity: EntityId,
        comp: ComponentId,
        ptr: *const std::ffi::c_void,
        size: usize,
    ) {
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
        let desc =
            ComponentRegisterDesc::new(name, size, align, std::ptr::null_mut(), no_prefab_apply());
        self.component_register(&desc)
    }

    pub fn query(&self, require: &[ComponentId], exclude: &[ComponentId]) -> Vec<EntityId> {
        let max = 16384;
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
