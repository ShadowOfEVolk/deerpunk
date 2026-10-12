use std::ffi::c_void;

pub use deerpunk_engine_common::component_inspect::ComponentInspectDumpFn;
pub use deerpunk_engine_common::world::PrefabComponentApplyFn;

/// No prefab applier (ECS type registration only).
pub fn no_prefab_apply() -> PrefabComponentApplyFn {
    unsafe { std::mem::transmute(0usize) }
}

/// No inspector dump (schema-only or no inspect).
pub fn no_inspect_dump() -> ComponentInspectDumpFn {
    unsafe { std::mem::transmute(0usize) }
}

#[repr(C)]
pub struct ComponentRegisterDesc {
    pub name_ptr: *const u8,
    pub name_len: usize,
    pub size: usize,
    pub align: usize,
    pub user_data: *mut c_void,
    /// Zero → no prefab applier (see [`no_prefab_apply`]).
    pub apply_fn: PrefabComponentApplyFn,
    /// CBOR blob of JSON Schema (empty ptr/len → none).
    pub inspect_schema_ptr: *const u8,
    pub inspect_schema_len: usize,
    pub inspect_dump_user: *mut c_void,
    /// Zero → no dump (see [`no_inspect_dump`]).
    pub inspect_dump_fn: ComponentInspectDumpFn,
}

impl ComponentRegisterDesc {
    pub fn new(
        name: &str,
        size: usize,
        align: usize,
        user_data: *mut c_void,
        apply_fn: PrefabComponentApplyFn,
    ) -> Self {
        Self {
            name_ptr: name.as_ptr(),
            name_len: name.len(),
            size,
            align,
            user_data,
            apply_fn,
            inspect_schema_ptr: std::ptr::null(),
            inspect_schema_len: 0,
            inspect_dump_user: std::ptr::null_mut(),
            inspect_dump_fn: no_inspect_dump(),
        }
    }
}
