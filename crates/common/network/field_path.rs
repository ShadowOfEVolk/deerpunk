//! Prefab hot/cold field paths for network replication.

/// Stable wire id for a replicated field or whole cold component.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldPathId {
    TransformPosition = 1,
    TransformRotation = 2,
    TransformScale = 3,
    /// Whole cold component (client loads defaults from local prefab).
    CameraComponent = 10,
    CharacterController = 11,
    SoundEmitter = 12,
    Material = 13,
    Worldspawn = 14,
    Mesh3D = 15,
    /// Cold override: Worldspawn.map string.
    WorldspawnMap = 16,
    /// Cold/hot: `Mesh3D.mesh` URI string (`file://…#brush:N`, etc.).
    Mesh3DMesh = 18,
}

impl FieldPathId {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::TransformPosition,
            2 => Self::TransformRotation,
            3 => Self::TransformScale,
            10 => Self::CameraComponent,
            11 => Self::CharacterController,
            12 => Self::SoundEmitter,
            13 => Self::Material,
            14 => Self::Worldspawn,
            15 => Self::Mesh3D,
            16 => Self::WorldspawnMap,
            18 => Self::Mesh3DMesh,
            _ => return None,
        })
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Parse prefab path like `TransformComponent.position` or `CameraComponent`.
    pub fn parse(path: &str) -> Option<Self> {
        match path.trim() {
            "TransformComponent.position" => Some(Self::TransformPosition),
            "TransformComponent.rotation" => Some(Self::TransformRotation),
            "TransformComponent.scale" => Some(Self::TransformScale),
            "CameraComponent" => Some(Self::CameraComponent),
            "CharacterController" => Some(Self::CharacterController),
            "SoundEmitter" => Some(Self::SoundEmitter),
            "Material" => Some(Self::Material),
            "Worldspawn" => Some(Self::Worldspawn),
            "Mesh3D" => Some(Self::Mesh3D),
            "Worldspawn.map" => Some(Self::WorldspawnMap),
            "Mesh3D.mesh" => Some(Self::Mesh3DMesh),
            _ => None,
        }
    }

    /// Prefab component key to apply when this path is listed as cold.
    pub fn component_key(self) -> Option<&'static str> {
        match self {
            Self::TransformPosition | Self::TransformRotation | Self::TransformScale => {
                Some("TransformComponent")
            }
            Self::CameraComponent => Some("CameraComponent"),
            Self::CharacterController => Some("CharacterController"),
            Self::SoundEmitter => Some("SoundEmitter"),
            Self::Material => Some("Material"),
            Self::Worldspawn | Self::WorldspawnMap => Some("Worldspawn"),
            Self::Mesh3D | Self::Mesh3DMesh => Some("Mesh3D"),
        }
    }

    pub fn is_hot_scalar(self) -> bool {
        matches!(
            self,
            Self::TransformPosition | Self::TransformRotation | Self::TransformScale
        )
    }
}

/// Wire value for one field path.
#[derive(Clone, Debug)]
pub enum FieldValue {
    Vec3([f32; 3]),
    Quat([f32; 4]),
    /// UTF-8 string (Worldspawn.map, etc.), max 255 bytes on wire.
    Str(String),
    /// Marker: apply whole component from local prefab (no payload).
    PrefabDefault,
    /// `BspModel.index` and similar scalar hot fields.
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

    pub fn decode(path: FieldPathId, data: &mut Cursor<'_>) -> Option<Self> {
        match path {
            FieldPathId::TransformPosition | FieldPathId::TransformScale => {
                Some(Self::Vec3([data.f32()?, data.f32()?, data.f32()?]))
            }
            FieldPathId::TransformRotation => {
                Some(Self::Quat([data.f32()?, data.f32()?, data.f32()?, data.f32()?]))
            }
            FieldPathId::WorldspawnMap => Some(Self::Str(data.string()?)),
            FieldPathId::Mesh3DMesh => Some(Self::Str(data.string()?)),
            FieldPathId::CameraComponent
            | FieldPathId::CharacterController
            | FieldPathId::SoundEmitter
            | FieldPathId::Material
            | FieldPathId::Worldspawn
            | FieldPathId::Mesh3D => Some(Self::PrefabDefault),
        }
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
