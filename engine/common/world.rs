use std::ffi::{CStr, CString};

use flecs_ecs_sys::{self as sys};

use crate::util::short_type_name;

pub static SEPARATOR: &CStr = c".";
pub type EntityId = u64;
pub type SystemId = u64;
pub type ComponentId = u64;
pub type QueryId = usize;

pub struct ComponentDescriptor {
    pub name: String,
    pub size: usize,
    pub align: usize,
}

pub struct SystemDescriptor {
    pub name: &'static str,
    pub query: &'static str,
    pub callback: sys::ecs_iter_action_t,
    pub phase: u64,
}

pub struct World {
    inner: *mut sys::ecs_world_t,
    queries: Vec<*mut sys::ecs_query_t>,
}

unsafe impl Send for World {}
unsafe impl Sync for World {}

impl World {
    pub fn new() -> Self {
        let inner = unsafe { sys::ecs_init() };
        unsafe { sys::ecs_log_set_level(5) };
        Self {
            inner,
            queries: Vec::new(),
        }
    }

    pub fn as_ptr(&self) -> *mut sys::ecs_world_t {
        self.inner
    }

    // ── Component registration ─────────────────────────────────

    pub fn component_init<T: 'static>(&self) -> ComponentId {
        self.component_init_raw(&ComponentDescriptor {
            name: short_type_name::<T>().to_string(),
            size: std::mem::size_of::<T>(),
            align: std::mem::align_of::<T>(),
        })
    }

    pub fn component_init_raw(&self, desc: &ComponentDescriptor) -> ComponentId {
        log::debug!(
            "component register: name={}, size={}, align={}",
            desc.name,
            desc.size,
            desc.align
        );

        let c_name = CString::new(desc.name.as_str()).expect("component name contains null byte");
        let sep = c".";
        unsafe {
            let entity_desc = sys::ecs_entity_desc_t {
                _canary: 0,
                id: 0,
                parent: 0,
                name: c_name.as_ptr(),
                sep: sep.as_ptr(),
                root_sep: core::ptr::null(),
                symbol: c_name.as_ptr(),
                use_low_id: true,
                add: core::ptr::null(),
                add_expr: core::ptr::null(),
                set: core::ptr::null(),
            };
            let entity = sys::ecs_entity_init(self.inner, &entity_desc);
            let mut ti: sys::ecs_type_info_t = std::mem::zeroed();
            ti.size = desc.size as i32;
            ti.alignment = desc.align as i32;
            ti.name = c_name.as_ptr();
            sys::ecs_component_init(
                self.inner,
                &sys::ecs_component_desc_t {
                    _canary: 0,
                    entity,
                    type_: ti,
                },
            )
            .into()
        }
    }

    // ── Entity CRUD ────────────────────────────────────────────

    pub fn spawn(&self) -> EntityId {
        log::trace!("ecs: spawn entity");
        unsafe { sys::ecs_new(self.inner) }
    }

    pub fn get_raw(&self, entity: EntityId, comp: ComponentId) -> *const std::ffi::c_void {
        unsafe { sys::ecs_get_id(self.inner, entity, comp) }
    }

    pub fn get<T: Copy>(&self, entity: EntityId, comp: ComponentId) -> Option<T> {
        let ptr = self.get_raw(entity, comp);
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { std::ptr::read_unaligned(ptr as *const T) })
        }
    }

    pub fn set_raw(
        &self,
        entity: EntityId,
        comp: ComponentId,
        data: *const std::ffi::c_void,
        size: usize,
    ) {
        println!(
            "set_raw_pre: entity={}, comp={}, size={}",
            entity, comp, size
        );
        unsafe { sys::ecs_set_id(self.inner, entity, comp, size, data) }
        println!(
            "set_raw_post: entity={}, comp={}, size={}",
            entity, comp, size
        );
    }

    /// Store `value` in Flecs by bitwise copy.
    ///
    /// Flecs does not run Rust `Drop`. Callers used to `ecs_set_id` then drop the
    /// temporary — that freed `Vec`/heap inside components like [`Mesh3D`] while
    /// Flecs still held the same pointers (dangling → UB on extract/clone).
    ///
    /// We drop any previous value in place, then `ManuallyDrop` the new one so
    /// ownership of heap resources lives in Flecs storage until overwritten.
    pub fn set<T>(&self, entity: EntityId, comp: ComponentId, value: T) {
        unsafe {
            let existing = sys::ecs_get_id(self.inner, entity, comp) as *mut T;
            if !existing.is_null() {
                std::ptr::drop_in_place(existing);
            }
        }
        let value = std::mem::ManuallyDrop::new(value);
        self.set_raw(
            entity,
            comp,
            (&*value) as *const T as *const std::ffi::c_void,
            std::mem::size_of::<T>(),
        );
    }

    pub fn has(&self, entity: EntityId, comp: ComponentId) -> bool {
        unsafe { sys::ecs_has_id(self.inner, entity, comp) }
    }

    pub fn remove(&self, entity: EntityId, comp: ComponentId) {
        unsafe { sys::ecs_remove_id(self.inner, entity, comp) }
    }

    pub fn destroy(&self, entity: EntityId) {
        unsafe { sys::ecs_delete(self.inner, entity) }
    }

    // ── System ─────────────────────────────────────────────────

    pub fn system_init(&self, desc: &SystemDescriptor) -> SystemId {
        let c_name = CString::new(desc.name).expect("system name contains null byte");
        let c_query = CString::new(desc.query).expect("system query contains null byte");
        unsafe {
            let mut entity_desc: sys::ecs_entity_desc_t = std::mem::zeroed();
            entity_desc.id = 0;
            entity_desc.name = c_name.as_ptr();
            let mut flecs_desc: sys::ecs_system_desc_t = std::mem::zeroed();
            flecs_desc.entity = sys::ecs_entity_init(self.inner, &entity_desc);
            flecs_desc.query.expr = c_query.as_ptr();
            flecs_desc.callback = desc.callback;
            flecs_desc.phase = desc.phase;
            sys::ecs_system_init(self.inner, &flecs_desc).into()
        }
    }

    pub fn simulate(&self, dt: f32) {
        unsafe {
            sys::ecs_progress(self.inner, dt);
        }
    }

    // ── Debug ──────────────────────────────────────────────────

    pub fn debug_dump(&self, names: &[&str]) {
        for name in names {
            let c_name = CString::new(*name).unwrap();
            unsafe {
                let id = sys::ecs_lookup(self.inner, c_name.as_ptr());
                let ti = sys::ecs_get_type_info(self.inner, id);
                let count = sys::ecs_count_id(self.inner, id);
                log::info!(
                    "flecs debug: \"{}\" => id={}, type_info={:?}, count={}",
                    name,
                    id,
                    ti,
                    count
                );
            }
        }
    }

    // ── Generic queries ────────────────────────────────────────

    /// Prepare a query from a DSL expression. Returns QueryId for later execution.
    pub fn query_prepare(&mut self, expression: &str) -> QueryId {
        let c_expr = CString::new(expression).expect("query expression contains null byte");
        unsafe {
            let mut desc: sys::ecs_query_desc_t = std::mem::zeroed();
            desc.expr = c_expr.as_ptr();
            let q = sys::ecs_query_init(self.inner, &desc);
            if q.is_null() {
                panic!(
                    "ecs_query_init failed for expression: {}, code: {}",
                    expression,
                    sys::ecs_log_last_error()
                );
            }
            let id = self.queries.len();
            self.queries.push(q);
            id
        }
    }

    /// Prepare a query from a list of component IDs. Returns QueryId for later execution.
    pub fn query_prepare_ids(&mut self, component_ids: &[ComponentId]) -> QueryId {
        unsafe {
            let mut desc: sys::ecs_query_desc_t = std::mem::zeroed();
            for (i, &comp_id) in component_ids.iter().enumerate() {
                if i >= 32 {
                    break;
                }
                desc.terms[i].id = comp_id as u64;
                desc.terms[i].src.id = 0;
                desc.terms[i].src.name = core::ptr::null();
            }
            let q = sys::ecs_query_init(self.inner, &desc);
            if q.is_null() {
                panic!(
                    "ecs_query_init failed for component IDs: {:?}",
                    component_ids
                );
            }
            let id = self.queries.len();
            self.queries.push(q);
            id
        }
    }

    /// Release a previously prepared query.
    pub fn query_release(&mut self, query_id: QueryId) {
        if let Some(q) = self.queries.get_mut(query_id) {
            if !q.is_null() {
                unsafe {
                    sys::ecs_query_fini(*q);
                }
                *q = std::ptr::null_mut();
            }
        }
    }

    /// Returns an iterator over archetype batches for a prepared query.
    /// Each batch gives contiguous pointers to entity IDs and component columns.
    pub fn query_batches(&self, query_id: QueryId) -> Option<QueryBatches<'_>> {
        let q = self.queries.get(query_id)?;
        if q.is_null() {
            return None;
        }
        unsafe {
            let it = Box::new(sys::ecs_query_iter(self.inner, *q));
            Some(QueryBatches {
                it,
                done: false,
                _marker: std::marker::PhantomData,
            })
        }
    }
}

/// A batch of entities sharing the same archetype table.
/// `entities` points to a contiguous array of `count` entity IDs.
/// `columns[f]` points to the start of field `f`'s data — a contiguous array of `count` elements.
/// Access as: `slice::from_raw_parts(columns[f] as *const T, count)`
pub struct QueryBatch {
    pub entities: *const EntityId,
    pub count: usize,
    pub columns: Vec<*const std::ffi::c_void>,
    pub sizes: Vec<usize>,
}

pub struct QueryBatches<'a> {
    it: Box<sys::ecs_iter_t>,
    done: bool,
    _marker: std::marker::PhantomData<&'a World>,
}

impl<'a> Iterator for QueryBatches<'a> {
    type Item = QueryBatch;

    fn next(&mut self) -> Option<QueryBatch> {
        if self.done {
            return None;
        }
        unsafe {
            if !sys::ecs_query_next(&mut *self.it) {
                self.done = true;
                return None;
            }
            let count = self.it.count as usize;
            let fields = self.it.field_count as usize;
            let mut columns = Vec::with_capacity(fields);
            let mut sizes = Vec::with_capacity(fields);
            for f in 0..fields {
                let size = *self.it.sizes.add(f) as usize;
                let col = sys::ecs_field_w_size(&*self.it, size, f as i8);
                columns.push(col as *const std::ffi::c_void);
                sizes.push(size);
            }
            Some(QueryBatch {
                entities: self.it.entities,
                count,
                columns,
                sizes,
            })
        }
    }
}

/// Alive entity for inspector listing (component *names* only — no field values).
#[derive(Clone, Debug)]
pub struct InspectEntity {
    pub id: EntityId,
    pub name: String,
    pub components: Vec<String>,
}

/// Component type names registered by the engine / gamelib (not flecs builtins).
fn is_game_component_token(token: &str) -> bool {
    // Skip flecs pairs like `(Identifier,Name)` / `(ChildOf,flecs.core)`.
    if token.starts_with('(') {
        return false;
    }
    let base = token.rsplit('.').next().unwrap_or(token);
    matches!(
        base,
        "TransformComponent"
            | "Mesh3D"
            | "Material"
            | "CameraComponent"
            | "Worldspawn"
            | "PointLight"
            | "MapKv"
            | "UiDocumentComponent"
            | "SimpleEntity"
            | "Terrain"
            | "TerrainSplatMaterial"
            | "TerrainNeedRebuild"
            | "TerrainCollider"
            | "TerrainRenderer"
            | "MeshParts"
            | "PendingBspArrays"
            | "MeshRenderer"
            | "NetReplicatedComponent"
    )
}

impl World {
    /// Snapshot of **game** entities (skips flecs builtins/modules/component types).
    ///
    /// `ecs_get_entities` returns the entire flecs ID space (~200+ core entities).
    /// We only keep entities that own at least one engine/gamelib component.
    pub fn inspect_entities(&self) -> Vec<InspectEntity> {
        let mut out = Vec::new();
        unsafe {
            let entities = sys::ecs_get_entities(self.inner);
            // Flecs packs alive IDs in `[0 .. alive_count)`.
            let n = entities.alive_count.max(0) as usize;
            let ids = std::slice::from_raw_parts(entities.ids, n);
            for &entity in ids {
                if !sys::ecs_is_alive(self.inner, entity) {
                    continue;
                }

                let mut raw_parts: Vec<String> = Vec::new();
                let type_ptr = sys::ecs_get_type(self.inner, entity);
                if !type_ptr.is_null() {
                    let tp = sys::ecs_type_str(self.inner, type_ptr);
                    if !tp.is_null() {
                        let s = CStr::from_ptr(tp).to_string_lossy().into_owned();
                        if let Some(free_fn) = sys::ecs_os_api.free_ {
                            free_fn(tp as *mut std::ffi::c_void);
                        }
                        for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                            raw_parts.push(part.to_string());
                        }
                    }
                }

                let components: Vec<String> = raw_parts
                    .iter()
                    .filter(|p| is_game_component_token(p))
                    .cloned()
                    .collect();
                if components.is_empty() {
                    continue;
                }

                let name_ptr = sys::ecs_get_name(self.inner, entity);
                let name = if name_ptr.is_null() {
                    format!("0x{entity:x}")
                } else {
                    CStr::from_ptr(name_ptr).to_string_lossy().into_owned()
                };

                out.push(InspectEntity {
                    id: entity,
                    name,
                    components,
                });
            }
        }
        out
    }
}

pub fn install_panic_hook(world_ptr: *mut sys::ecs_world_t) {
    use std::sync::Once;
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let ptr = world_ptr as usize;
        std::panic::set_hook(Box::new(move |info| {
            let path = "crash_world_dump.txt";
            unsafe {
                let w = ptr as *mut sys::ecs_world_t;
                dump_world_from_ptr(w, path);
            }
            log::error!("Panic occurred — world dumped to {}", path);
            if let Some(s) = info.payload().downcast_ref::<&str>() {
                log::error!("Panic: {}", s);
            } else if let Some(s) = info.payload().downcast_ref::<String>() {
                log::error!("Panic: {}", s);
            }
            if let Some(loc) = info.location() {
                log::error!("at {}:{}:{}", loc.file(), loc.line(), loc.column());
            }
        }));
    });
}

fn dump_world_from_ptr(world_ptr: *mut sys::ecs_world_t, path: &str) {
    let mut output = String::new();
    output.push_str("=== World Dump (panic) ===\n");

    unsafe {
        let entities = sys::ecs_get_entities(world_ptr);

        output.push_str(&format!(
            "Entity slots: count={}, alive={}\n",
            entities.count, entities.alive_count
        ));

        let ids = std::slice::from_raw_parts(entities.ids, entities.count as usize);
        let mut alive = 0;
        for &entity in ids {
            if !sys::ecs_is_alive(world_ptr, entity) {
                continue;
            }
            alive += 1;
            let name_ptr = sys::ecs_get_name(world_ptr, entity);
            let name = if name_ptr.is_null() {
                "(unnamed)".to_string()
            } else {
                CStr::from_ptr(name_ptr).to_string_lossy().into_owned()
            };

            let type_ptr = sys::ecs_get_type(world_ptr, entity);
            let type_str = if type_ptr.is_null() {
                "(no components)".to_string()
            } else {
                let tp = sys::ecs_type_str(world_ptr, type_ptr);
                if tp.is_null() {
                    "(null type)".to_string()
                } else {
                    let s = CStr::from_ptr(tp).to_string_lossy().into_owned();
                    if let Some(free_fn) = sys::ecs_os_api.free_ {
                        free_fn(tp as *mut std::ffi::c_void);
                    }
                    s
                }
            };

            output.push_str(&format!("  0x{:08x} '{}' : {}\n", entity, name, type_str));
        }
        output.insert_str(
            output.find('\n').unwrap() + 1,
            &format!("Actually alive (scanned): {}\n", alive),
        );
    }

    let _ = std::fs::write(path, &output);
}

impl Drop for World {
    fn drop(&mut self) {
        unsafe {
            for q in &self.queries {
                if !q.is_null() {
                    sys::ecs_query_fini(*q);
                }
            }
            sys::ecs_fini(self.inner);
        }
    }
}
