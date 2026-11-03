//! Stable [`PropertyId`] for layout-backed fields (replicate, save, prefab, inspect, …).

use crate::property::registry::PropertyRegistry;

/// Canonical property identifier (same id on disk, net packets, and ECS accessors).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PropertyId(pub u32);

impl PropertyId {
    pub const TRANSFORM_POSITION: Self = Self(1);
    pub const TRANSFORM_ROTATION: Self = Self(2);
    pub const TRANSFORM_SCALE: Self = Self(3);
    pub const CAMERA_COMPONENT: Self = Self(10);
    pub const CHARACTER_CONTROLLER: Self = Self(11);
    pub const SOUND_EMITTER: Self = Self(12);
    pub const MATERIAL: Self = Self(13);
    pub const WORLDSPAWN: Self = Self(14);
    pub const MESH3D: Self = Self(15);
    pub const WORLDSPAWN_MAP: Self = Self(16);
    pub const MESH3D_MESH: Self = Self(18);
    pub const STUDIO_MODEL_BODY: Self = Self(19);
    pub const STUDIO_MODEL_SKIN: Self = Self(20);
    pub const MAPKV_STUDIO_MODEL: Self = Self(21);
    pub const DRAW_MATERIAL_URI: Self = Self(22);

    #[inline]
    pub const fn from_u32(v: u32) -> Self {
        Self(v)
    }

    /// Lookup after decoding a packet or save blob.
    pub fn decode(v: u32) -> Option<Self> {
        let id = Self(v);
        PropertyRegistry::global()
            .lock()
            .ok()
            .and_then(|r| r.get(id).map(|_| id))
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }

    pub fn encode_le(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.0.to_le_bytes());
    }

    pub fn parse(path: &str) -> Option<Self> {
        PropertyRegistry::global()
            .lock()
            .ok()
            .and_then(|r| r.resolve_path(path))
    }
}
