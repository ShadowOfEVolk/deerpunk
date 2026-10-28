use std::ffi::c_void;

use crate::components::{no_prefab_apply, ComponentRegisterDesc};
use crate::spawn_prefab::SpawnPrefabOpts;
use crate::Resolver;

pub type EntityId = u64;
pub type ComponentId = u64;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

pub struct WorldApiResolved {
    ctx: *mut u8,
    get_fn: extern "C" fn(*mut u8, u64, u64) -> *mut c_void,
    set_fn: extern "C" fn(*mut u8, u64, u64, *const c_void, usize),
    has_fn: extern "C" fn(*mut u8, u64, u64) -> bool,
    remove_fn: extern "C" fn(*mut u8, u64, u64),
    spawn_fn: extern "C" fn(*mut u8) -> u64,
    spawn_bundle_fn: extern "C" fn(*mut u8, *const u64, *const *mut c_void, usize) -> u64,
    destroy_fn: extern "C" fn(*mut u8, u64),
    clear_fn: extern "C" fn(*mut u8),
    query_fn:
        extern "C" fn(*mut u8, *const u64, usize, *const u64, usize, *mut u64, usize) -> usize,
    query_count_fn: extern "C" fn(*mut u8, *const u64, usize, *const u64, usize) -> usize,
    query_matches_fn: extern "C" fn(*mut u8, u64, *const u64, usize, *const u64, usize) -> bool,
    query_prepare_fn: extern "C" fn(*mut u8, *const u8, usize) -> u32,
    query_execute_fn: extern "C" fn(
        *mut u8,
        u32,
        extern "C" fn(u64, *const *const c_void, usize, *mut c_void),
        *mut c_void,
    ),
    spawn_prefab_fn: extern "C" fn(*mut u8, *const u8, usize, *const SpawnPrefabOpts) -> u64,
    component_register_fn: extern "C" fn(*mut u8, *const ComponentRegisterDesc) -> u64,
}

unsafe impl Send for WorldApiResolved {}
unsafe impl Sync for WorldApiResolved {}

impl WorldApiResolved {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            get_fn: unsafe { std::mem::transmute(resolve(r, "v1:world:get")) },
            set_fn: unsafe { std::mem::transmute(resolve(r, "v1:world:set")) },
            has_fn: unsafe { std::mem::transmute(resolve(r, "v1:world:has")) },
            remove_fn: unsafe { std::mem::transmute(resolve(r, "v1:world:remove")) },
            spawn_fn: unsafe { std::mem::transmute(resolve(r, "v1:world:spawn")) },
            spawn_bundle_fn: unsafe { std::mem::transmute(resolve(r, "v1:world:spawn_bundle")) },
            destroy_fn: unsafe { std::mem::transmute(resolve(r, "v1:world:destroy")) },
            clear_fn: unsafe { std::mem::transmute(resolve(r, "v1:world:clear")) },
            query_fn: unsafe { std::mem::transmute(resolve(r, "v1:world:query")) },
            query_count_fn: unsafe { std::mem::transmute(resolve(r, "v1:world:query_count")) },
            query_matches_fn: unsafe { std::mem::transmute(resolve(r, "v1:world:query_matches")) },
            query_prepare_fn: unsafe { std::mem::transmute(resolve(r, "v1:world:query_prepare")) },
            query_execute_fn: unsafe { std::mem::transmute(resolve(r, "v1:world:query_execute")) },
            spawn_prefab_fn: unsafe { std::mem::transmute(resolve(r, "v1:world:spawn_prefab")) },
            component_register_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:world:component_register"))
            },
        }
    }

    pub fn get_raw(&self, entity: EntityId, comp: ComponentId) -> Option<*const c_void> {
        let ptr = (self.get_fn)(self.ctx, entity, comp);
        if ptr.is_null() {
            None
        } else {
            Some(ptr as *const c_void)
        }
    }

    pub fn get<T: Copy>(&self, entity: EntityId, comp: ComponentId) -> Option<T> {
        let ptr = (self.get_fn)(self.ctx, entity, comp);
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { std::ptr::read_unaligned(ptr as *const T) })
        }
    }

    pub fn set_raw(&self, entity: EntityId, comp: ComponentId, ptr: *const c_void, size: usize) {
        (self.set_fn)(self.ctx, entity, comp, ptr, size);
    }

    pub fn set<T>(&self, entity: EntityId, comp: ComponentId, value: T) {
        let ptr = &value as *const T as *const c_void;
        let size = std::mem::size_of::<T>();
        (self.set_fn)(self.ctx, entity, comp, ptr, size);
        std::mem::forget(value);
    }

    pub fn has(&self, entity: EntityId, comp: ComponentId) -> bool {
        (self.has_fn)(self.ctx, entity, comp)
    }

    pub fn remove(&self, entity: EntityId, comp: ComponentId) {
        (self.remove_fn)(self.ctx, entity, comp);
    }

    pub fn spawn(&self) -> EntityId {
        (self.spawn_fn)(self.ctx)
    }

    pub fn spawn_bundle_raw(&self, components: &[(ComponentId, *const c_void, usize)]) -> EntityId {
        let count = components.len();
        if count == 0 {
            return self.spawn();
        }
        let ids: Vec<u64> = components.iter().map(|(id, _, _)| *id).collect();
        let data: Vec<*mut c_void> = components
            .iter()
            .map(|(_, ptr, _)| *ptr as *mut c_void)
            .collect();
        (self.spawn_bundle_fn)(self.ctx, ids.as_ptr(), data.as_ptr(), count)
    }

    pub fn spawn_bundle(&self, components: Vec<(ComponentId, Box<dyn std::any::Any>)>) -> EntityId {
        let count = components.len();
        if count == 0 {
            return self.spawn();
        }
        let ids: Vec<u64> = components.iter().map(|(id, _)| *id).collect();
        let data: Vec<*mut c_void> = components
            .into_iter()
            .map(|(_, d)| Box::into_raw(d) as *const u8 as *mut c_void)
            .collect();
        (self.spawn_bundle_fn)(self.ctx, ids.as_ptr(), data.as_ptr(), count)
    }

    pub fn destroy(&self, entity: EntityId) {
        (self.destroy_fn)(self.ctx, entity);
    }

    pub fn clear(&self) {
        (self.clear_fn)(self.ctx);
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

    pub fn query_count(&self, require: &[ComponentId], exclude: &[ComponentId]) -> usize {
        (self.query_count_fn)(
            self.ctx,
            require.as_ptr(),
            require.len(),
            exclude.as_ptr(),
            exclude.len(),
        )
    }

    pub fn query_matches(
        &self,
        entity: EntityId,
        require: &[ComponentId],
        exclude: &[ComponentId],
    ) -> bool {
        (self.query_matches_fn)(
            self.ctx,
            entity,
            require.as_ptr(),
            require.len(),
            exclude.as_ptr(),
            exclude.len(),
        )
    }

    pub fn component_register(&self, desc: &ComponentRegisterDesc) -> Result<ComponentId, String> {
        let id = (self.component_register_fn)(self.ctx, desc);
        if id == 0 {
            Err("component_register failed".into())
        } else {
            Ok(id)
        }
    }

    /// Register component type (no prefab applier).
    pub fn register_component(
        &self,
        name: &str,
        size: usize,
        align: usize,
    ) -> Result<ComponentId, String> {
        let desc =
            ComponentRegisterDesc::new(name, size, align, std::ptr::null_mut(), no_prefab_apply());
        self.component_register(&desc)
    }

    pub fn query_prepare(&self, expression: &str) -> u32 {
        (self.query_prepare_fn)(self.ctx, expression.as_ptr(), expression.len())
    }

    pub fn query_execute(
        &self,
        query_id: u32,
        callback: extern "C" fn(u64, *const *const c_void, usize, *mut c_void),
        ctx: *mut c_void,
    ) {
        (self.query_execute_fn)(self.ctx, query_id, callback, ctx);
    }

    /// Trace a line against the server BSP world mesh.
    /// Returns `Some(result)` on hit (with hit point, fraction, texture name),
    /// `None` on miss.
    /// Spawn `{gamedir}/prefabs/{name}.prefab` into the server world.
    pub fn spawn_prefab(&self, name: &str, opts: &SpawnPrefabOpts) -> Result<EntityId, String> {
        let bytes = name.as_bytes();
        let id = (self.spawn_prefab_fn)(self.ctx, bytes.as_ptr(), bytes.len(), opts as *const _);
        if id == 0 {
            Err(format!("spawn_prefab failed: {name}"))
        } else {
            Ok(id)
        }
    }
}
