//! Prefab component payloads: JSON on disk → CBOR across gamelib FFI.

use serde_json::Value;

pub fn value_to_cbor(value: &Value) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    ciborium::into_writer(value, &mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

pub fn value_from_cbor(bytes: &[u8]) -> Result<Value, String> {
    ciborium::from_reader(bytes).map_err(|e| e.to_string())
}
