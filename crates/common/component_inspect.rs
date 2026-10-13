//! Inspector: JSON Schema (CBOR at register) + field dump (CBOR on read).

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;

use serde_json::Value;

use crate::prefab::{value_from_cbor, value_to_cbor};
use crate::world::{ComponentId, EntityId, World};

pub use deerpunk_engine_ffi::ComponentInspectDumpFn;

pub type NativeInspectDumpFn =
    fn(&World, EntityId, ComponentId) -> Result<Value, String>;

#[derive(Clone)]
enum InspectDumpKind {
    Native(NativeInspectDumpFn),
    Ffi {
        user: *mut c_void,
        dump: ComponentInspectDumpFn,
    },
}

#[derive(Clone)]
pub struct ComponentInspectMeta {
    pub component_id: ComponentId,
    /// JSON Schema (decoded from registration CBOR).
    pub schema: Value,
    kind: InspectDumpKind,
}

pub struct ComponentInspectRegistry {
    by_name: Mutex<HashMap<String, ComponentInspectMeta>>,
}

impl ComponentInspectRegistry {
    pub fn new() -> Self {
        Self {
            by_name: Mutex::new(HashMap::new()),
        }
    }

    pub fn register_native(
        &self,
        name: &str,
        component_id: ComponentId,
        schema_cbor: &[u8],
        dump: NativeInspectDumpFn,
    ) -> Result<(), String> {
        let schema = if schema_cbor.is_empty() {
            Value::Null
        } else {
            value_from_cbor(schema_cbor)?
        };
        let mut map = self
            .by_name
            .lock()
            .map_err(|_| "inspect registry lock poisoned".to_string())?;
        map.insert(
            name.to_string(),
            ComponentInspectMeta {
                component_id,
                schema,
                kind: InspectDumpKind::Native(dump),
            },
        );
        Ok(())
    }

    pub fn register_ffi(
        &self,
        name: &str,
        component_id: ComponentId,
        schema_cbor: &[u8],
        user: *mut c_void,
        dump: ComponentInspectDumpFn,
    ) -> Result<(), String> {
        if dump as usize == 0 {
            return Ok(());
        }
        let schema = if schema_cbor.is_empty() {
            Value::Null
        } else {
            value_from_cbor(schema_cbor)?
        };
        let mut map = self
            .by_name
            .lock()
            .map_err(|_| "inspect registry lock poisoned".to_string())?;
        map.insert(
            name.to_string(),
            ComponentInspectMeta {
                component_id,
                schema,
                kind: InspectDumpKind::Ffi { user, dump },
            },
        );
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<ComponentInspectMeta> {
        let base = name.rsplit('.').next().unwrap_or(name);
        let map = self.by_name.lock().ok()?;
        map.get(base).cloned()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn dump(
        &self,
        world: &World,
        entity: EntityId,
        component_slug: &str,
    ) -> Option<(Value, Value)> {
        let meta = self.get(component_slug)?;
        if !world.has(entity, meta.component_id) {
            return None;
        }
        let data = match meta.kind {
            InspectDumpKind::Native(dump) => dump(world, entity, meta.component_id).ok()?,
            InspectDumpKind::Ffi { user, dump } => {
                let mut buf = vec![0u8; 65536];
                let n = dump(
                    user,
                    entity,
                    meta.component_id,
                    buf.as_mut_ptr(),
                    buf.len(),
                );
                if n < 0 {
                    return None;
                }
                buf.truncate(n as usize);
                value_from_cbor(&buf).ok()?
            }
        };
        Some((meta.schema.clone(), data))
    }
}

pub fn schema_cbor(schema: &Value) -> Result<Vec<u8>, String> {
    value_to_cbor(schema)
}
