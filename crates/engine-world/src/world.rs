use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::path::Path;
use std::sync::Mutex;

use flecs_ecs_sys::{self as sys};
use serde_json::Value;

use crate::component_inspect::{
    schema_cbor, ComponentInspectDumpFn, ComponentInspectRegistry, NativeInspectDumpFn,
};
use crate::layout_meta::{decode as decode_layout_meta, LayoutMetaRegistry};
use crate::prefab::value_to_cbor;

pub static SEPARATOR: &CStr = c".";
pub use deerpunk_engine_ffi::{ComponentId, EntityId, PrefabComponentApplyFn};

pub type SystemId = u64;

#[derive(Clone, Copy)]
struct StageSlot {
    phase: u64,
    /// Systems with `(DependsOn, phase)`.
    query: *mut sys::ecs_query_t,
}

struct StageTable {
    by_name: HashMap<String, StageSlot>,
    system_phase: HashMap<u64, u64>,
}
pub type QueryId = usize;

pub struct ComponentDescriptor {
    pub name: String,
    pub size: usize,
    pub align: usize,
    pub prefab_apply_user: *mut c_void,
    pub prefab_apply: Option<PrefabComponentApplyFn>,
    /// JSON Schema bytes (CBOR-encoded JSON).
    pub inspect_schema_cbor: Vec<u8>,
    pub inspect_dump_user: *mut c_void,
    pub inspect_dump_native: Option<NativeInspectDumpFn>,
    pub inspect_dump_ffi: Option<ComponentInspectDumpFn>,
    /// DPK1 layout metadata blob ([`crate::layout_meta::encode`]).
    pub layout_meta: Vec<u8>,
}

/// Host/engine prefab apply (JSON `Value` from prefab file).
pub type NativePrefabApplyFn = fn(
    &World,
    &mut deerpunk_engine_resource::MeshRegistry,
    &Path,
    &deerpunk_engine_resource::ResourceManager,
    EntityId,
    ComponentId,
    &Value,
) -> Result<(), String>;

#[derive(Clone, Copy)]
enum PrefabApplierKind {
    Native(NativePrefabApplyFn),
    Ffi {
        user: *mut c_void,
        apply: PrefabComponentApplyFn,
    },
}

#[derive(Clone, Copy)]
struct PrefabApplier {
    component_id: ComponentId,
    /// Flecs storage size (FFI gamelib prefab apply uses this for first add).
    component_size: usize,
    kind: PrefabApplierKind,
}

/// Callback for a system or observer. `entities` has `count` ids. `dt` is 0 for observers.
pub type EcsCallback =
    extern "C-unwind" fn(entities: *const EntityId, count: u32, dt: f32, ctx: *mut c_void);

#[derive(Clone, Copy)]
pub enum SystemSchedule {
    /// Not part of `ecs_progress`. Run with [`World::run_system`].
    Manual,
    /// Runs inside [`World::simulate`].
    OnUpdate,
}

#[derive(Clone, Copy)]
pub enum WatchEvent {
    OnAdd,
    OnSet,
    OnRemove,
}

struct CallbackSlot {
    _name: CString,
    _query: CString,
    cb: EcsCallback,
    ctx: *mut c_void,
}

unsafe extern "C-unwind" fn callback_bridge(it: *mut sys::ecs_iter_t) {
    unsafe {
        if it.is_null() {
            return;
        }
        let it_ref = &*it;
        if it_ref.ctx.is_null() || it_ref.count <= 0 {
            return;
        }
        let slot = &*(it_ref.ctx as *const CallbackSlot);
        (slot.cb)(
            it_ref.entities,
            it_ref.count as u32,
            it_ref.delta_time,
            slot.ctx,
        );
    }
}

/// Gamelib callback for [`World::register_progress_system`].
///
/// Flecs invokes it once per matching table. `entities`, `fields`, and `sizes`
/// are valid only for that call. `fields[term]` is the component column for
/// query term `term` (the order of the query string). Row `i` is
/// `fields[term] + i * sizes[term]` when the column is an owned array.
pub type ProgressSystemFn = extern "C" fn(
    entities: *const EntityId,
    count: u32,
    fields: *const *const c_void,
    field_count: u32,
    sizes: *const usize,
    dt: f32,
    user_data: *mut c_void,
);

struct ProgressSystemSlot {
    run: ProgressSystemFn,
    user_data: *mut c_void,
}

unsafe extern "C-unwind" fn progress_system_bridge(it: *mut sys::ecs_iter_t) {
    unsafe {
        if it.is_null() {
            return;
        }
        let it_ref = &*it;
        if it_ref.ctx.is_null() {
            return;
        }
        let slot = &*(it_ref.ctx as *const ProgressSystemSlot);
        if it_ref.count <= 0 {
            return;
        }
        let field_count = it_ref.field_count.max(0) as usize;
        let mut fields = [std::ptr::null::<c_void>(); 32];
        let mut sizes = [0usize; 32];
        let n = field_count.min(fields.len());
        for i in 0..n {
            let index = i as i8;
            // `ecs_field_w_size` aborts when `size` is 0. Tags and unset fields stay null.
            let size = sys::ecs_field_size(it, index);
            sizes[i] = size;
            if size != 0 {
                fields[i] = sys::ecs_field_w_size(it, size, index);
            }
        }
        (slot.run)(
            it_ref.entities,
            it_ref.count as u32,
            fields.as_ptr(),
            n as u32,
            sizes.as_ptr(),
            it_ref.delta_time,
            slot.user_data,
        );
    }
}

pub struct World {
    inner: *mut sys::ecs_world_t,
    queries: Vec<*mut sys::ecs_query_t>,
    prefab_appliers: Mutex<HashMap<String, PrefabApplier>>,
    inspect: ComponentInspectRegistry,
    layout_meta: LayoutMetaRegistry,
    /// Entities to [`Self::destroy`] after each [`Self::simulate`] (safe from Flecs OnUpdate).
    deferred_destroy: Mutex<Vec<EntityId>>,
    /// Named Flecs phases (`renderer.prepare` and others) plus the phase last assigned to a system.
    stages: Mutex<StageTable>,
}

unsafe impl Send for World {}
unsafe impl Sync for World {}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        let inner = unsafe { sys::ecs_init() };
        unsafe { sys::ecs_log_set_level(5) };
        Self {
            inner,
            queries: Vec::new(),
            prefab_appliers: Mutex::new(HashMap::new()),
            inspect: ComponentInspectRegistry::new(),
            layout_meta: LayoutMetaRegistry::new(),
            deferred_destroy: Mutex::new(Vec::new()),
            stages: Mutex::new(StageTable {
                by_name: HashMap::new(),
                system_phase: HashMap::new(),
            }),
        }
    }

    pub fn inspect_registry(&self) -> &ComponentInspectRegistry {
        &self.inspect
    }

    pub fn layout_registry(&self) -> &LayoutMetaRegistry {
        &self.layout_meta
    }

    /// Inspector field dump: `(schema, data)` JSON values.
    pub fn inspect_component_fields(
        &self,
        entity: EntityId,
        component_slug: &str,
    ) -> Option<(Value, Value)> {
        self.inspect.dump(self, entity, component_slug)
    }

    pub fn register_native_inspect(
        &self,
        name: &str,
        component_id: ComponentId,
        schema: Value,
        dump: NativeInspectDumpFn,
    ) -> Result<(), String> {
        let cbor = if schema.is_null() {
            Vec::new()
        } else {
            schema_cbor(&schema)?
        };
        self.inspect
            .register_native(name, component_id, &cbor, dump)
    }

    // ── Component registration ─────────────────────────────────

    pub fn component_init<T: 'static>(&self) -> ComponentId {
        self.component_init_raw(&ComponentDescriptor {
            name: short_type_name::<T>().to_string(),
            size: std::mem::size_of::<T>(),
            align: std::mem::align_of::<T>(),
            prefab_apply_user: std::ptr::null_mut(),
            prefab_apply: None,
            inspect_schema_cbor: Vec::new(),
            inspect_dump_user: std::ptr::null_mut(),
            inspect_dump_native: None,
            inspect_dump_ffi: None,
            layout_meta: Vec::new(),
        })
    }

    /// Apply a registered gamelib prefab applier. Returns `Ok(true)` if handled.
    pub fn try_prefab_apply_registered(
        &self,
        meshes: &mut deerpunk_engine_resource::MeshRegistry,
        gamedir: &Path,
        resources: &deerpunk_engine_resource::ResourceManager,
        entity: EntityId,
        name: &str,
        value: &Value,
    ) -> Result<bool, String> {
        let applier = {
            let map = self
                .prefab_appliers
                .lock()
                .map_err(|_| "prefab applier lock poisoned".to_string())?;
            map.get(name).cloned()
        };
        let Some(applier) = applier else {
            return Ok(false);
        };
        match applier.kind {
            PrefabApplierKind::Native(apply) => {
                apply(
                    self,
                    meshes,
                    gamedir,
                    resources,
                    entity,
                    applier.component_id,
                    value,
                )?;
            }
            PrefabApplierKind::Ffi { user, apply } => {
                if crate::prefab_layout_apply::try_apply_prefab_json(
                    self,
                    entity,
                    applier.component_id,
                    value,
                )
                .is_ok()
                {
                    return Ok(true);
                }
                if !self.has(entity, applier.component_id) && applier.component_size > 0 {
                    let buf = vec![0u8; applier.component_size];
                    self.set_raw(
                        entity,
                        applier.component_id,
                        buf.as_ptr() as *const std::ffi::c_void,
                        applier.component_size,
                    );
                }
                let cbor = value_to_cbor(value)?;
                let code = apply(
                    user,
                    entity,
                    applier.component_id,
                    cbor.as_ptr(),
                    cbor.len(),
                );
                if code != 0 {
                    return Err(format!("prefab apply '{name}' failed (code={code})"));
                }
            }
        }
        Ok(true)
    }

    /// Register prefab JSON key → native applier (engine / host).
    pub fn attach_native_prefab_applier(
        &self,
        prefab_key: &str,
        component_id: ComponentId,
        apply: NativePrefabApplyFn,
    ) {
        let mut map = self
            .prefab_appliers
            .lock()
            .expect("prefab applier lock poisoned");
        map.insert(
            prefab_key.to_string(),
            PrefabApplier {
                component_id,
                component_size: 0,
                kind: PrefabApplierKind::Native(apply),
            },
        );
    }

    pub fn component_init_raw(&self, desc: &ComponentDescriptor) -> ComponentId {
        if desc.size == 0 {
            panic!(
                "ECS component_init \"{}\": size must be > 0 \
                 (Flecs cannot store zero-sized types; add e.g. `_tag: u8` like MeshDrawHidden)",
                desc.name
            );
        }
        log::debug!(
            "component register: name={}, size={}, align={}",
            desc.name,
            desc.size,
            desc.align
        );

        let c_name = CString::new(desc.name.as_str()).expect("component name contains null byte");
        let sep = c".";
        let id = unsafe {
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
        };
        if let Some(apply) = desc.prefab_apply {
            if apply as usize != 0 {
                let mut map = self
                    .prefab_appliers
                    .lock()
                    .expect("prefab applier lock poisoned");
                map.insert(
                    desc.name.clone(),
                    PrefabApplier {
                        component_id: id,
                        component_size: desc.size,
                        kind: PrefabApplierKind::Ffi {
                            user: desc.prefab_apply_user,
                            apply,
                        },
                    },
                );
            }
        }
        if !desc.inspect_schema_cbor.is_empty()
            || desc.inspect_dump_native.is_some()
            || desc.inspect_dump_ffi.is_some()
        {
            if let Some(dump) = desc.inspect_dump_native {
                let _ =
                    self.inspect
                        .register_native(&desc.name, id, &desc.inspect_schema_cbor, dump);
            } else if let Some(dump) = desc.inspect_dump_ffi {
                let _ = self.inspect.register_ffi(
                    &desc.name,
                    id,
                    &desc.inspect_schema_cbor,
                    desc.inspect_dump_user,
                    dump,
                );
            }
        }
        if !desc.layout_meta.is_empty() {
            match decode_layout_meta(&desc.layout_meta) {
                Ok(layout) => {
                    let _ = self.layout_meta.register(id, layout.clone());
                    if let Some(hook) = layout_hook() {
                        hook(id, &layout);
                    }
                }
                Err(e) => {
                    log::error!("component '{}' layout meta: {e}", desc.name);
                }
            }
        }
        id
    }

    /// Resolve a registered component type id by Flecs symbol name (e.g. `ItemDropComponent`).
    pub fn component_lookup(&self, name: &str) -> Option<ComponentId> {
        let c_name = CString::new(name).ok()?;
        let id = unsafe { sys::ecs_lookup(self.inner, c_name.as_ptr()) };
        if id == 0 {
            None
        } else {
            Some(id)
        }
    }

    // ── Entity CRUD ────────────────────────────────────────────

    pub fn spawn(&self) -> EntityId {
        log::trace!("ecs: spawn entity");
        unsafe { sys::ecs_new(self.inner) }
    }

    pub fn get_raw(&self, entity: EntityId, comp: ComponentId) -> *const std::ffi::c_void {
        crate::ecs_validate::assert_component_id(comp, "get_raw");
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

    // Pointer is an FFI payload: null is rejected, Flecs reads `size` bytes. The
    // signature stays safe so host bindings can forward a checked C pointer.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn set_raw(
        &self,
        entity: EntityId,
        comp: ComponentId,
        data: *const std::ffi::c_void,
        size: usize,
    ) {
        crate::ecs_validate::assert_component_id(comp, "set_raw");
        crate::ecs_validate::assert_entity_alive(self, entity, "set_raw");
        crate::ecs_validate::assert_set_payload(size, "set_raw");
        if data.is_null() {
            panic!("ECS set_raw: null data pointer");
        }
        unsafe { sys::ecs_set_id(self.inner, entity, comp, size, data) }
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
        crate::ecs_validate::assert_component_id(comp, "has");
        unsafe { sys::ecs_has_id(self.inner, entity, comp) }
    }

    /// Alive entities with `comp` — no inspector name/type string work (hot paths).
    pub fn for_each_with_component(&self, comp: ComponentId, mut f: impl FnMut(EntityId)) {
        unsafe {
            let entities = sys::ecs_get_entities(self.inner);
            let n = entities.alive_count.max(0) as usize;
            let ids = std::slice::from_raw_parts(entities.ids, n);
            for &entity in ids {
                if !sys::ecs_is_alive(self.inner, entity) {
                    continue;
                }
                if self.has(entity, comp) {
                    f(entity);
                }
            }
        }
    }

    pub fn remove(&self, entity: EntityId, comp: ComponentId) {
        crate::ecs_validate::assert_component_id(comp, "remove");
        crate::ecs_validate::assert_entity_alive(self, entity, "remove");
        unsafe { sys::ecs_remove_id(self.inner, entity, comp) }
    }

    pub fn destroy(&self, entity: EntityId) {
        unsafe { sys::ecs_delete(self.inner, entity) }
    }

    /// Queue delete until after the current / next [`Self::simulate`] (`ecs_progress` completes first).
    /// Use from gamelib Flecs systems instead of [`Self::destroy`].
    pub fn destroy_deferred(&self, entity: EntityId) {
        if entity == 0 {
            return;
        }
        self.deferred_destroy.lock().unwrap().push(entity);
    }

    pub fn clear_deferred_destroy(&self) {
        self.deferred_destroy.lock().unwrap().clear();
    }

    /// Apply all [`Self::destroy_deferred`] queues (also called at end of [`Self::simulate`]).
    pub fn flush_deferred_destroys(&self) {
        let pending: Vec<EntityId> = {
            let mut q = self.deferred_destroy.lock().unwrap();
            std::mem::take(&mut *q)
        };

        for entity in pending {
            if entity != 0 && self.is_alive(entity) {
                self.destroy(entity);
            }
        }
    }

    pub fn is_alive(&self, entity: EntityId) -> bool {
        unsafe { sys::ecs_is_alive(self.inner, entity) }
    }

    /// Flecs entity name (`ecs_get_name` / inspector). Not replicated; gameplay ids use components.
    pub fn set_entity_name(&self, entity: EntityId, name: &str) {
        let c_name = CString::new(name).expect("entity name contains null byte");
        unsafe {
            sys::ecs_set_name(self.inner, entity, c_name.as_ptr());
        }
    }

    // ── System ─────────────────────────────────────────────────

    /// Register a system. Flecs types stay inside this crate; `cb` sees entity ids only.
    pub fn register_system(
        &self,
        name: &str,
        query: &str,
        schedule: SystemSchedule,
        ctx: *mut c_void,
        cb: EcsCallback,
    ) -> SystemId {
        let slot = Box::leak(Box::new(CallbackSlot {
            _name: CString::new(name).expect("system name contains null byte"),
            _query: CString::new(query).expect("system query contains null byte"),
            cb,
            ctx,
        }));
        unsafe {
            let mut entity_desc: sys::ecs_entity_desc_t = std::mem::zeroed();
            entity_desc.id = 0;
            entity_desc.name = slot._name.as_ptr();
            let mut flecs_desc: sys::ecs_system_desc_t = std::mem::zeroed();
            flecs_desc.entity = sys::ecs_entity_init(self.inner, &entity_desc);
            flecs_desc.query.expr = slot._query.as_ptr();
            flecs_desc.callback = Some(callback_bridge);
            flecs_desc.ctx = slot as *mut CallbackSlot as *mut c_void;
            flecs_desc.phase = match schedule {
                SystemSchedule::Manual => 0,
                SystemSchedule::OnUpdate => sys::EcsOnUpdate,
            };
            let id: SystemId = sys::ecs_system_init(self.inner, &flecs_desc);
            log::info!(target: "ecs", "registered system '{name}' query='{query}' id={id}");
            id
        }
    }

    /// Register an observer. `cb` is invoked once per event batch (`dt` is 0).
    pub fn register_observer(
        &self,
        query: &str,
        events: &[WatchEvent],
        ctx: *mut c_void,
        cb: EcsCallback,
    ) -> Result<(), String> {
        if events.is_empty() {
            return Err("register_observer: no events".into());
        }
        let slot = Box::leak(Box::new(CallbackSlot {
            _name: CString::new("observer").expect("name"),
            _query: CString::new(query).map_err(|e| e.to_string())?,
            cb,
            ctx,
        }));
        unsafe {
            let mut desc: sys::ecs_observer_desc_t = std::mem::zeroed();
            desc.query.expr = slot._query.as_ptr();
            desc.callback = Some(callback_bridge);
            desc.ctx = slot as *mut CallbackSlot as *mut c_void;
            for (i, ev) in events.iter().enumerate() {
                if i >= desc.events.len() {
                    break;
                }
                desc.events[i] = match ev {
                    WatchEvent::OnAdd => sys::EcsOnAdd,
                    WatchEvent::OnSet => sys::EcsOnSet,
                    WatchEvent::OnRemove => sys::EcsOnRemove,
                };
            }
            let ent = sys::ecs_observer_init(self.inner, &desc);
            if ent == 0 {
                return Err(format!("observer init failed for query '{query}'"));
            }
        }
        log::info!(target: "ecs", "observer '{query}'");
        Ok(())
    }

    /// Register a gamelib callback as a Flecs system (`EcsOnUpdate`, runs in [`Self::simulate`]).
    pub fn register_progress_system(
        &self,
        name: &str,
        query: &str,
        run: ProgressSystemFn,
        user_data: *mut std::ffi::c_void,
        interval_secs: f32,
    ) -> SystemId {
        let name_static: &'static str = Box::leak(name.to_string().into_boxed_str());
        let query_static: &'static str = Box::leak(query.to_string().into_boxed_str());
        let slot = Box::leak(Box::new(ProgressSystemSlot { run, user_data }));
        let c_name = CString::new(name_static).expect("system name contains null byte");
        let c_query = CString::new(query_static).expect("system query contains null byte");
        unsafe {
            let mut entity_desc: sys::ecs_entity_desc_t = std::mem::zeroed();
            entity_desc.id = 0;
            entity_desc.name = c_name.as_ptr();
            let mut flecs_desc: sys::ecs_system_desc_t = std::mem::zeroed();
            flecs_desc.entity = sys::ecs_entity_init(self.inner, &entity_desc);
            flecs_desc.query.expr = c_query.as_ptr();
            flecs_desc.callback = Some(progress_system_bridge);
            flecs_desc.ctx = slot as *mut ProgressSystemSlot as *mut c_void;
            flecs_desc.phase = sys::EcsOnUpdate;
            flecs_desc.interval = interval_secs.max(0.0);
            let id: SystemId = sys::ecs_system_init(self.inner, &flecs_desc);
            log::info!(
                target: "ecs",
                "registered progress system '{name_static}' query='{query_static}' id={id} interval={}s",
                flecs_desc.interval
            );
            id
        }
    }

    /// Move a progress system from `OnUpdate` onto a named Flecs phase.
    ///
    /// `stage` is a Flecs path (`renderer.prepare`). The phase is not part of the
    /// builtin pipeline; the host runs it with [`Self::run_stage`].
    /// Returns `0` on success, `-1` if `system_id` or `stage` is empty.
    pub fn set_system_stage(&self, system_id: SystemId, stage: &str) -> i32 {
        if system_id == 0 || stage.is_empty() || stage.contains('\0') {
            return -1;
        }
        let Some(phase) = self.ensure_stage(stage) else {
            return -1;
        };
        unsafe {
            let mut desc: sys::ecs_system_desc_t = std::mem::zeroed();
            desc.entity = system_id as sys::ecs_entity_t;
            desc.phase = phase;
            let updated = sys::ecs_system_update(self.inner, system_id as sys::ecs_entity_t, &desc);
            if updated == 0 {
                return -1;
            }
            // `ecs_system_update` keeps interval/ctx but does not move the phase.
            // The pipeline matches `(DependsOn, phase)`, so swap that pair directly.
            let on_update = sys::ecs_make_pair(sys::EcsDependsOn, sys::EcsOnUpdate);
            sys::ecs_remove_id(self.inner, system_id, on_update);
            sys::ecs_remove_id(self.inner, system_id, sys::EcsOnUpdate);
            if let Some(previous) = self
                .stages
                .lock()
                .unwrap()
                .system_phase
                .insert(system_id, phase)
            {
                if previous != phase {
                    sys::ecs_remove_id(
                        self.inner,
                        system_id,
                        sys::ecs_make_pair(sys::EcsDependsOn, previous),
                    );
                    sys::ecs_remove_id(self.inner, system_id, previous);
                }
            }
            sys::ecs_add_id(
                self.inner,
                system_id,
                sys::ecs_make_pair(sys::EcsDependsOn, phase),
            );
            sys::ecs_add_id(self.inner, system_id, phase);
            // Builtin `ecs_progress` still visits the system after the OnUpdate pair
            // is gone. Keep it disabled until [`Self::run_stage`].
            sys::ecs_enable(self.inner, system_id, false);
        }
        log::info!(target: "ecs", "system id={system_id} stage='{stage}'");
        0
    }

    /// Run every system currently assigned to `stage`. Unknown names are a no-op.
    pub fn run_stage(&self, stage: &str, dt: f32) {
        if stage.is_empty() {
            return;
        }
        let query = {
            let table = self.stages.lock().unwrap();
            table.by_name.get(stage).map(|slot| slot.query)
        };
        let Some(query) = query else {
            return;
        };
        if query.is_null() {
            return;
        }
        unsafe {
            let mut it = sys::ecs_query_iter(self.inner, query);
            while sys::ecs_query_next(&mut it) {
                for i in 0..it.count {
                    let system = *it.entities.add(i as usize);
                    sys::ecs_enable(self.inner, system, true);
                    sys::ecs_run(self.inner, system, dt, std::ptr::null_mut());
                    sys::ecs_enable(self.inner, system, false);
                }
            }
        }
    }

    fn ensure_stage(&self, name: &str) -> Option<u64> {
        let mut table = self.stages.lock().unwrap();
        if let Some(slot) = table.by_name.get(name) {
            return Some(slot.phase);
        }
        let c_name = CString::new(name).ok()?;
        unsafe {
            let existing = sys::ecs_lookup(self.inner, c_name.as_ptr());
            let phase = if existing != 0 {
                existing
            } else {
                let mut desc: sys::ecs_entity_desc_t = std::mem::zeroed();
                desc.name = c_name.as_ptr();
                desc.sep = SEPARATOR.as_ptr();
                sys::ecs_entity_init(self.inner, &desc)
            };
            if phase == 0 {
                return None;
            }
            sys::ecs_add_id(self.inner, phase, sys::EcsPhase);
            let mut qdesc: sys::ecs_query_desc_t = std::mem::zeroed();
            qdesc.terms[0].id = sys::ecs_make_pair(sys::EcsDependsOn, phase);
            qdesc.flags = sys::EcsQueryMatchDisabled;
            let query = sys::ecs_query_init(self.inner, &qdesc);
            if query.is_null() {
                return None;
            }
            table
                .by_name
                .insert(name.to_string(), StageSlot { phase, query });
            Some(phase)
        }
    }

    /// Change Flecs [`ecs_system_desc_t::interval`] for a gamelib progress system.
    pub fn set_progress_system_interval(&self, system_id: SystemId, interval_secs: f32) {
        if system_id == 0 {
            return;
        }
        unsafe {
            let mut desc: sys::ecs_system_desc_t = std::mem::zeroed();
            desc.entity = system_id as sys::ecs_entity_t;
            desc.interval = interval_secs.max(0.0);
            sys::ecs_system_update(self.inner, system_id as sys::ecs_entity_t, &desc);
            log::info!(
                target: "ecs",
                "progress system id={system_id} interval={}s",
                desc.interval
            );
        }
    }

    pub fn simulate(&self, dt: f32) {
        unsafe {
            sys::ecs_progress(self.inner, dt);
        }
        self.flush_deferred_destroys();
    }

    /// Run a Flecs system outside the default pipeline (e.g. physics sync systems).
    pub fn run_system(&self, system_id: SystemId, dt: f32) {
        if system_id == 0 {
            return;
        }
        unsafe {
            sys::ecs_run(
                self.inner,
                system_id as sys::ecs_entity_t,
                dt,
                std::ptr::null_mut(),
            );
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
                desc.terms[i].id = comp_id;
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
            | "DrawMaterial"
            | "CameraComponent"
            | "Worldspawn"
            | "PointLight"
            | "SoundEmitter"
            | "LuaScript"
            | "MapKv"
            | "UiDocumentComponent"
            | "SimpleEntity"
            | "Terrain"
            | "TerrainSplatMaterial"
            | "TerrainNeedRebuild"
            | "TerrainCollider"
            | "TerrainRenderer"
            | "PendingAlbedoAtlas"
            | "MeshRenderer"
            | "NetReplicatedComponent"
            | "NetworkEntity"
            | "CharacterController"
            | "PlayerMovement"
            | "PlayerUse"
            | "FallRescue"
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
                    .filter(|p| is_game_component_token(p) || self.inspect.contains(p))
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

pub fn install_panic_hook(world: &World) {
    use std::sync::Once;
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let ptr = world.inner as usize;
        std::panic::set_hook(Box::new(move |info| {
            let path = "crash_world_dump.txt";
            let w = ptr as *mut sys::ecs_world_t;
            dump_world_from_ptr(w, path);
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
            for slot in self.stages.lock().unwrap().by_name.values() {
                if !slot.query.is_null() {
                    sys::ecs_query_fini(slot.query);
                }
            }
            sys::ecs_fini(self.inner);
        }
    }
}

fn short_type_name<T: ?Sized>() -> &'static str {
    let full = std::any::type_name::<T>();
    full.split("::").last().unwrap_or(full)
}

type LayoutHook = fn(ComponentId, &crate::layout_meta::LayoutMeta);

fn layout_hook() -> Option<LayoutHook> {
    *LAYOUT_HOOK.lock().unwrap()
}

static LAYOUT_HOOK: Mutex<Option<LayoutHook>> = Mutex::new(None);

/// Engine-common property registry listens here. Not a Flecs type.
pub fn set_layout_hook(hook: LayoutHook) {
    *LAYOUT_HOOK.lock().unwrap() = Some(hook);
}

#[cfg(test)]
mod stage_tests {
    use super::*;

    extern "C" fn count_runs(
        _entities: *const EntityId,
        _count: u32,
        _fields: *const *const c_void,
        _field_count: u32,
        _sizes: *const usize,
        _dt: f32,
        user: *mut c_void,
    ) {
        unsafe { *(user as *mut u32) += 1 };
    }

    #[test]
    fn set_system_stage_replaces_on_update() {
        let world = World::new();
        unsafe {
            let mut tag: sys::ecs_entity_desc_t = std::mem::zeroed();
            tag.name = c"StageProbe".as_ptr();
            let tag_id = sys::ecs_entity_init(world.inner, &tag);
            assert_ne!(tag_id, 0);
            let entity = world.spawn();
            sys::ecs_add_id(world.inner, entity, tag_id);
        }
        let mut runs = 0u32;
        let id = world.register_progress_system(
            "prep",
            "StageProbe",
            count_runs,
            &mut runs as *mut u32 as *mut c_void,
            0.0,
        );
        assert_ne!(id, 0);
        assert_eq!(world.set_system_stage(id, "renderer.prepare"), 0);
        unsafe {
            let phase = sys::ecs_lookup(world.inner, c"renderer.prepare".as_ptr());
            assert_ne!(phase, 0);
            assert!(sys::ecs_has_id(
                world.inner,
                id,
                sys::ecs_make_pair(sys::EcsDependsOn, phase),
            ));
            assert!(!sys::ecs_has_id(
                world.inner,
                id,
                sys::ecs_make_pair(sys::EcsDependsOn, sys::EcsOnUpdate),
            ));
        }
        world.simulate(0.016);
        assert_eq!(runs, 0, "staged system must leave the OnUpdate pipeline");
        world.run_stage("renderer.prepare", 0.016);
        assert_eq!(runs, 1);
        assert_eq!(world.set_system_stage(0, "renderer.prepare"), -1);
        assert_eq!(world.set_system_stage(id, ""), -1);
    }
}
