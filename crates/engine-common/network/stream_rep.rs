//! Entity streaming: visibility baseline (`stream.in`), leave (`stream.out`), batched hot updates.

use std::collections::HashMap;

use serde_json::Value;

use super::field_path::{Cursor, FieldPathId, FieldValue};
use super::prefab_rep::{decode_fields, encode_fields};
use super::protocol::SVC;

/// Client should create/refresh shadow entity from prefab + component overrides + hot fields.
#[derive(Clone, Debug)]
pub struct StreamInRpc {
    pub net_id: u16,
    /// Bumps when the same `net_id` is reused after `stream.out` (anti-ghost).
    pub generation: u8,
    pub prefab: String,
    /// Prefab-shaped component JSON (e.g. `Mesh3D.uri`, `Worldspawn.map`).
    pub components: HashMap<String, Value>,
    pub hot: Vec<(FieldPathId, FieldValue)>,
}

/// Client drops shadow; server entity may still exist.
#[derive(Clone, Debug)]
pub struct StreamOutRpc {
    pub net_id: u16,
}

/// One reliable packet with hot-field changes for multiple streamed entities.
#[derive(Clone, Debug)]
pub struct StreamUpdateBatchRpc {
    pub updates: Vec<StreamEntityUpdate>,
}

#[derive(Clone, Debug)]
pub struct StreamEntityUpdate {
    pub net_id: u16,
    pub fields: Vec<(FieldPathId, FieldValue)>,
}

fn encode_components(out: &mut Vec<u8>, components: &HashMap<String, Value>) {
    let json = serde_json::to_vec(components).unwrap_or_default();
    let n = json.len().min(65535);
    out.extend_from_slice(&(n as u16).to_le_bytes());
    out.extend_from_slice(&json[..n]);
}

fn decode_components(c: &mut Cursor<'_>) -> Option<HashMap<String, Value>> {
    let n = c.u16()? as usize;
    let bytes = c.take(n)?;
    let v: Value = serde_json::from_slice(bytes).ok()?;
    match v {
        Value::Object(map) => Some(map.into_iter().collect()),
        _ => Some(HashMap::new()),
    }
}

impl StreamInRpc {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(128);
        out.extend_from_slice(&self.net_id.to_le_bytes());
        out.push(self.generation);
        let name = self.prefab.as_bytes();
        let n = name.len().min(255);
        out.push(n as u8);
        out.extend_from_slice(&name[..n]);
        encode_components(&mut out, &self.components);
        encode_fields(&mut out, &self.hot);
        out
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        let mut c = Cursor::new(payload);
        let net_id = c.u16()?;
        let generation = c.u8()?;
        let prefab = c.string()?;
        let components = decode_components(&mut c)?;
        let hot = decode_fields(&mut c)?;
        Some(Self {
            net_id,
            generation,
            prefab,
            components,
            hot,
        })
    }

    pub fn to_packet(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(self.encode().len() + 1);
        p.push(SVC::StreamIn as u8);
        p.extend_from_slice(&self.encode());
        p
    }
}

impl StreamOutRpc {
    pub fn encode(&self) -> Vec<u8> {
        self.net_id.to_le_bytes().to_vec()
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        let mut c = Cursor::new(payload);
        Some(Self { net_id: c.u16()? })
    }

    pub fn to_packet(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(3);
        p.push(SVC::StreamOut as u8);
        p.extend_from_slice(&self.encode());
        p
    }
}

impl StreamUpdateBatchRpc {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        let n = self.updates.len().min(255);
        out.push(n as u8);
        for u in self.updates.iter().take(n) {
            out.extend_from_slice(&u.net_id.to_le_bytes());
            encode_fields(&mut out, &u.fields);
        }
        out
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        let mut c = Cursor::new(payload);
        let count = c.u8()? as usize;
        let mut updates = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            let net_id = c.u16()?;
            let fields = decode_fields(&mut c)?;
            updates.push(StreamEntityUpdate { net_id, fields });
        }
        Some(Self { updates })
    }

    pub fn to_packet(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(self.encode().len() + 1);
        p.push(SVC::StreamUpdate as u8);
        p.extend_from_slice(&self.encode());
        p
    }
}
