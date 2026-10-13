//! Inspector JSON Schema helpers (`schemars` → CBOR for host registration).

use schemars::JsonSchema;
use serde_json::Value;

use crate::prefab::value_to_cbor;

/// Build CBOR-encoded JSON Schema for `T` (one-shot; cache with `LazyLock` at call site).
pub fn schema_cbor<T: JsonSchema>() -> Vec<u8> {
    let root = schemars::schema_for!(T);
    let value: Value = serde_json::to_value(&root).expect("JsonSchema JSON");
    value_to_cbor(&value).expect("inspect schema CBOR")
}
