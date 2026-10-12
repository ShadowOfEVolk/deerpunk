//! Prefab-driven replication wire packets (SVC payloads without command byte).

use super::field_path::{Cursor, FieldPathId, FieldValue};
use super::protocol::SVC;
use crate::scene::EntityDef;

/// Spawn a prefab on the client. `net_id == 0` → not a NetworkEntity (e.g. worldspawn).
#[derive(Clone, Debug)]
pub struct SpawnPrefabRpc {
    pub net_id: u16,
    pub prefab: String,
    /// Cold overrides / markers (whole components as PrefabDefault, or WorldspawnMap).
    pub cold: Vec<(FieldPathId, FieldValue)>,
    /// Hot snapshot so the entity appears in the correct pose immediately.
    pub hot: Vec<(FieldPathId, FieldValue)>,
}

/// Hot-field delta for an existing NetworkEntity.
#[derive(Clone, Debug)]
pub struct NetDeltaRpc {
    pub net_id: u16,
    pub fields: Vec<(FieldPathId, FieldValue)>,
}

/// Destroy a networked entity on the client.
#[derive(Clone, Debug)]
pub struct NetDestroyRpc {
    pub net_id: u16,
}

/// Server-authored prefab (`prefabs/{name}.prefab` body as JSON).
#[derive(Clone, Debug)]
pub struct PrefabDefRpc {
    pub name: String,
    pub def: EntityDef,
}

fn encode_fields(out: &mut Vec<u8>, fields: &[(FieldPathId, FieldValue)]) {
    out.push(fields.len().min(255) as u8);
    for (path, value) in fields.iter().take(255) {
        out.push(path.as_u8());
        value.encode(out);
    }
}

fn decode_fields(c: &mut Cursor<'_>) -> Option<Vec<(FieldPathId, FieldValue)>> {
    let count = c.u8()? as usize;
    let mut fields = Vec::with_capacity(count.min(32));
    for _ in 0..count {
        let path = FieldPathId::from_u8(c.u8()?)?;
        let value = FieldValue::decode(path, c)?;
        fields.push((path, value));
    }
    Some(fields)
}

impl SpawnPrefabRpc {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        out.extend_from_slice(&self.net_id.to_le_bytes());
        let name = self.prefab.as_bytes();
        let n = name.len().min(255);
        out.push(n as u8);
        out.extend_from_slice(&name[..n]);
        encode_fields(&mut out, &self.cold);
        encode_fields(&mut out, &self.hot);
        out
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        let mut c = Cursor::new(payload);
        let net_id = c.u16()?;
        let prefab = c.string()?;
        let cold = decode_fields(&mut c)?;
        let hot = decode_fields(&mut c)?;
        Some(Self {
            net_id,
            prefab,
            cold,
            hot,
        })
    }

    pub fn to_packet(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(64);
        p.push(SVC::SpawnPrefab as u8);
        p.extend_from_slice(&self.encode());
        p
    }
}

impl NetDeltaRpc {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32);
        out.extend_from_slice(&self.net_id.to_le_bytes());
        encode_fields(&mut out, &self.fields);
        out
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        let mut c = Cursor::new(payload);
        let net_id = c.u16()?;
        let fields = decode_fields(&mut c)?;
        Some(Self { net_id, fields })
    }

    pub fn to_packet(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(32);
        p.push(SVC::NetDelta as u8);
        p.extend_from_slice(&self.encode());
        p
    }
}

impl PrefabDefRpc {
    pub fn encode(&self) -> Vec<u8> {
        let json = serde_json::to_vec(&self.def).unwrap_or_default();
        let name = self.name.as_bytes();
        let n = name.len().min(255);
        let jlen = json.len().min(512 * 1024);
        let mut out = Vec::with_capacity(8 + n + jlen);
        out.push(n as u8);
        out.extend_from_slice(&name[..n]);
        out.extend_from_slice(&(jlen as u32).to_le_bytes());
        out.extend_from_slice(&json[..jlen]);
        out
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        let mut c = Cursor::new(payload);
        let name = c.string()?;
        let jlen = {
            let s = c.take(4)?;
            u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize
        };
        let json = c.take(jlen)?;
        let def: EntityDef = serde_json::from_slice(json).ok()?;
        Some(Self { name, def })
    }

    pub fn to_packet(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(64);
        p.push(SVC::PrefabDef as u8);
        p.extend_from_slice(&self.encode());
        p
    }
}

impl NetDestroyRpc {
    pub fn encode(&self) -> Vec<u8> {
        self.net_id.to_le_bytes().to_vec()
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        let mut c = Cursor::new(payload);
        Some(Self { net_id: c.u16()? })
    }

    pub fn to_packet(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(3);
        p.push(SVC::NetDestroy as u8);
        p.extend_from_slice(&self.encode());
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_roundtrip() {
        let rpc = SpawnPrefabRpc {
            net_id: 7,
            prefab: "player".into(),
            cold: vec![
                (FieldPathId::CameraComponent, FieldValue::PrefabDefault),
                (FieldPathId::CharacterController, FieldValue::PrefabDefault),
            ],
            hot: vec![(
                FieldPathId::TransformPosition,
                FieldValue::Vec3([1.0, 2.0, 3.0]),
            )],
        };
        let decoded = SpawnPrefabRpc::decode(&rpc.encode()).unwrap();
        assert_eq!(decoded.net_id, 7);
        assert_eq!(decoded.prefab, "player");
        assert_eq!(decoded.cold.len(), 2);
        assert_eq!(decoded.hot.len(), 1);
        match &decoded.hot[0].1 {
            FieldValue::Vec3(v) => assert_eq!(*v, [1.0, 2.0, 3.0]),
            _ => panic!("expected vec3"),
        }
    }

    #[test]
    fn delta_destroy_roundtrip() {
        let d = NetDeltaRpc {
            net_id: 3,
            fields: vec![(
                FieldPathId::TransformRotation,
                FieldValue::Quat([0.0, 0.0, 0.0, 1.0]),
            )],
        };
        let d2 = NetDeltaRpc::decode(&d.encode()).unwrap();
        assert_eq!(d2.net_id, 3);
        assert_eq!(d2.fields.len(), 1);

        let x = NetDestroyRpc { net_id: 9 };
        assert_eq!(NetDestroyRpc::decode(&x.encode()).unwrap().net_id, 9);
    }
}
