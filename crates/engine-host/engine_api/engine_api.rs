use std::collections::HashMap;
use std::sync::atomic::{AtomicPtr, Ordering};

pub struct EngineApiMethod {
    pub name: &'static str,
    pub callback: *const (),
    pub flags: u32,
}

unsafe impl Send for EngineApiMethod {}
unsafe impl Sync for EngineApiMethod {}

inventory::collect!(EngineApiMethod);

pub struct MethodDB {
    pub hashmap: HashMap<&'static str, *const ()>,
}

static METHODDB_PTR: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());

#[derive(Clone, Copy, Debug)]
pub struct MethodEntry {
    pub name: &'static str,
    pub flags: u32,
}

impl MethodDB {
    /// All tramps registered via [`inventory::submit!`] (sorted by name).
    pub fn inventory_entries() -> Vec<MethodEntry> {
        let mut entries: Vec<MethodEntry> = inventory::iter::<EngineApiMethod>()
            .map(|m| MethodEntry {
                name: m.name,
                flags: m.flags,
            })
            .collect();
        entries.sort_by_key(|e| e.name);
        entries
    }

    pub fn new() -> Self {
        let mut hashmap = HashMap::new();
        for method in inventory::iter::<EngineApiMethod> {
            hashmap.insert(method.name, method.callback);
        }
        Self { hashmap }
    }

    pub fn into_resolver(self) -> extern "C" fn(*const u8, usize) -> *const () {
        let boxed = Box::into_raw(Box::new(self));
        METHODDB_PTR.store(boxed as *mut (), Ordering::Release);

        extern "C" fn resolve(name_ptr: *const u8, name_len: usize) -> *const () {
            let db = unsafe { &*(METHODDB_PTR.load(Ordering::Acquire) as *const MethodDB) };
            let name = unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len))
            };
            db.hashmap.get(name).copied().unwrap_or(std::ptr::null())
        }

        resolve
    }
}
