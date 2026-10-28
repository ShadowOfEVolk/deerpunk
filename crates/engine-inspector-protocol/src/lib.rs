//! Wire protocol for `deerpunk-inspector` (JSON-RPC 2.0 over WebSocket).
//!
//! Component fields: JSON Schema + instance data via `component_register` (schema/dump as CBOR).

pub mod jsonrpc;
pub mod types;

pub use jsonrpc::{
    ERR_ENTITY_NOT_FOUND, ERR_INTERNAL, ERR_INVALID_PARAMS, ERR_INVALID_REQUEST, ERR_METHOD_NOT_FOUND,
    PROTO_VERSION, err, notify, ok,
};
pub use types::*;

use serde_json::{Value, json};

/// Built-in collections for a minimal engine stub (entities only).
pub fn default_collections() -> Vec<CollectionDescriptor> {
    vec![CollectionDescriptor {
        id: "entities".into(),
        label: "Entities".into(),
        group: CollectionGroup::Runtime,
        modes: vec![CollectionMode::List, CollectionMode::Get],
        id_field: "index".into(),
        nested_in: None,
    }]
}

pub fn initialize_result(
    server: &str,
    server_version: &str,
    collections: &[CollectionDescriptor],
) -> InitializeResult {
    let collections = if collections.is_empty() {
        default_collections()
    } else {
        collections.to_vec()
    };
    InitializeResult {
        proto: PROTO_VERSION,
        server: server.into(),
        server_version: Some(server_version.into()),
        collections,
        capabilities: Some(json!({
            "componentFields": true,
            "eventsStream": false,
        })),
    }
}

/// Fallback when no inspect bridge is registered for this component.
pub fn stub_component(slug: &str) -> ComponentSnapshot {
    ComponentSnapshot::Data {
        slug: slug.to_string(),
        schema: None,
        data: json!({
            "_stub": true,
            "_note": "no inspect dump for this component",
        }),
    }
}

pub fn component_data(slug: &str, schema: Option<Value>, data: Value) -> ComponentSnapshot {
    ComponentSnapshot::Data {
        slug: slug.to_string(),
        schema,
        data,
    }
}

/// Handle `inspector.initialize` / `inspector.initialized` and unknown methods.
/// Collection methods (`entities.*`) are left to the host.
pub fn handle_session_rpc(
    method: &str,
    id: Option<&Value>,
    params: &Value,
    server: &str,
    server_version: &str,
    collections: &[CollectionDescriptor],
) -> Option<String> {
    match method {
        "inspector.initialize" => {
            let Some(id) = id else {
                return Some(err(&Value::Null, ERR_INVALID_REQUEST, "initialize needs id"));
            };
            let proto = params.get("proto").and_then(|v| v.as_i64()).unwrap_or(0);
            if proto < 1 {
                return Some(err(id, ERR_INVALID_PARAMS, "unsupported protocol version"));
            }
            let result = initialize_result(server, server_version, collections);
            Some(ok(id, serde_json::to_value(result).unwrap_or(Value::Null)))
        }
        "inspector.initialized" => None, // notification ack: nothing to send
        _ => None,
    }
}
