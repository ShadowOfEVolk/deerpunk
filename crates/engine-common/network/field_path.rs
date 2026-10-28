//! Wire values and decode cursor for network replication.

pub use super::net_field::{FieldPathId, NetFieldId};

/// Wire value for one replicated field.
#[derive(Clone, Debug)]
pub enum FieldValue {
    Vec3([f32; 3]),
    Quat([f32; 4]),
    /// UTF-8 string (Worldspawn.map, mesh URI, …), max 255 bytes on wire (except mesh tag encoding).
    Str(String),
    /// Marker: apply whole component from local prefab (no payload).
    PrefabDefault,
    U32(u32),
}

impl FieldValue {
    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Self::Vec3(v) => {
                for f in v {
                    out.extend_from_slice(&f.to_le_bytes());
                }
            }
            Self::Quat(v) => {
                for f in v {
                    out.extend_from_slice(&f.to_le_bytes());
                }
            }
            Self::Str(s) => {
                let b = s.as_bytes();
                let n = b.len().min(255);
                out.push(n as u8);
                out.extend_from_slice(&b[..n]);
            }
            Self::PrefabDefault => {}
            Self::U32(v) => out.extend_from_slice(&v.to_le_bytes()),
        }
    }

    pub fn decode(field: NetFieldId, data: &mut Cursor<'_>) -> Option<Self> {
        use super::net_field::{NetFieldKind, NetFieldRegistry};
        let reg = NetFieldRegistry::global()
            .lock()
            .ok()?;
        let desc = reg.get(field)?;
        match &desc.kind {
            NetFieldKind::WholeComponent { .. } => Some(Self::PrefabDefault),
            NetFieldKind::Mesh3dUri | NetFieldKind::DrawMaterialUri | NetFieldKind::MapKvStudioModel => {
                let tag = data.u8()?;
                if tag == 0 {
                    Some(Self::PrefabDefault)
                } else {
                    let bytes = data.take(tag as usize)?;
                    Some(Self::Str(String::from_utf8_lossy(bytes).into_owned()))
                }
            }
            NetFieldKind::WorldspawnMap => Some(Self::Str(data.string()?)),
            NetFieldKind::Memory { wire, .. } => decode_wire(*wire, data),
        }
    }
}

fn decode_wire(wire: crate::layout_meta::WireKind, data: &mut Cursor<'_>) -> Option<FieldValue> {
    use crate::layout_meta::WireKind;
    match wire {
        WireKind::Vec3 => Some(FieldValue::Vec3([data.f32()?, data.f32()?, data.f32()?])),
        WireKind::Quat => {
            Some(FieldValue::Quat([data.f32()?, data.f32()?, data.f32()?, data.f32()?]))
        }
        WireKind::U32 | WireKind::U16 | WireKind::U8 => {
            let s = data.take(4)?;
            Some(FieldValue::U32(u32::from_le_bytes([s[0], s[1], s[2], s[3]])))
        }
        WireKind::Indirect => Some(FieldValue::Str(data.string()?)),
        WireKind::Void => Some(FieldValue::PrefabDefault),
        _ => None,
    }
}

pub struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn u8(&mut self) -> Option<u8> {
        let b = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    pub fn u16(&mut self) -> Option<u16> {
        let s = self.take(2)?;
        Some(u16::from_le_bytes([s[0], s[1]]))
    }

    pub fn f32(&mut self) -> Option<f32> {
        let s = self.take(4)?;
        Some(f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let slice = self.data.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(slice)
    }

    pub fn string(&mut self) -> Option<String> {
        let len = self.u8()? as usize;
        let bytes = self.take(len)?;
        Some(String::from_utf8_lossy(bytes).into_owned())
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }
}
