//! Entity streaming: visibility baseline (`stream.in`), leave (`stream.out`), batched hot updates.

use super::field_path::{Cursor, FieldPathId, FieldValue};
use super::prefab_net_registry::PrefabNetId;
use super::prefab_rep::{decode_fields, encode_fields};
use super::protocol::SVC;

/// Client spawns from registered [`PrefabNetId`] + cold/hot net fields (no prefab JSON on wire).
#[derive(Clone, Debug)]
pub struct StreamInRpc {
    pub net_id: u16,
    /// Bumps when the same `net_id` is reused after `stream.out` (anti-ghost).
    pub generation: u8,
    pub prefab_id: PrefabNetId,
    /// Cold snapshot (whole components + rare fields from [`NetworkEntity::cold`]).
    pub cold: Vec<(FieldPathId, FieldValue)>,
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

impl StreamInRpc {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        out.extend_from_slice(&self.net_id.to_le_bytes());
        out.push(self.generation);
        out.extend_from_slice(&self.prefab_id.0.to_le_bytes());
        encode_fields(&mut out, &self.cold);
        encode_fields(&mut out, &self.hot);
        out
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        let mut c = Cursor::new(payload);
        let net_id = c.u16()?;
        let generation = c.u8()?;
        let prefab_id = PrefabNetId(c.u16()?);
        let cold = decode_fields(&mut c)?;
        let hot = decode_fields(&mut c)?;
        Some(Self {
            net_id,
            generation,
            prefab_id,
            cold,
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
