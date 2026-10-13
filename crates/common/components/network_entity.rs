//! Marks an entity for prefab-driven network replication.

use crate::network::field_path::FieldPathId;

pub const NET_PREFAB_MAX: usize = 64;
pub const NET_FIELDS_MAX: usize = 16;

/// Skip per-frame hot replication after initial `stream.in` (map geometry, static func_wall, …).
pub const NET_FLAG_DORMANT: u8 = 1;

/// Server-authored replication marker. Client spawns via prefab + hot/cold paths.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NetworkEntity {
    /// Assigned by server replication (`0` = not yet).
    pub net_id: u16,
    pub _pad: u16,
    /// Prefab name used at spawn (NUL-padded).
    pub prefab: [u8; NET_PREFAB_MAX],
    pub prefab_len: u8,
    pub hot_count: u8,
    pub cold_count: u8,
    pub flags: u8,
    pub hot: [u8; NET_FIELDS_MAX],
    pub cold: [u8; NET_FIELDS_MAX],
}

impl Default for NetworkEntity {
    fn default() -> Self {
        Self {
            net_id: 0,
            _pad: 0,
            prefab: [0; NET_PREFAB_MAX],
            prefab_len: 0,
            hot_count: 0,
            cold_count: 0,
            flags: 0,
            hot: [0; NET_FIELDS_MAX],
            cold: [0; NET_FIELDS_MAX],
        }
    }
}

impl NetworkEntity {
    pub fn from_paths(hot: &[FieldPathId], cold: &[FieldPathId]) -> Self {
        let mut e = Self::default();
        e.set_hot(hot);
        e.set_cold(cold);
        e
    }

    pub fn from_prefab_paths(hot: &[FieldPathId], cold: &[FieldPathId], dormant: bool) -> Self {
        let mut e = Self::from_paths(hot, cold);
        if dormant {
            e.flags |= NET_FLAG_DORMANT;
        }
        e
    }

    #[inline]
    pub fn is_dormant(&self) -> bool {
        self.flags & NET_FLAG_DORMANT != 0
    }

    pub fn set_prefab(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let n = bytes.len().min(NET_PREFAB_MAX);
        self.prefab[..n].copy_from_slice(&bytes[..n]);
        if n < NET_PREFAB_MAX {
            self.prefab[n..].fill(0);
        }
        self.prefab_len = n as u8;
    }

    pub fn prefab_str(&self) -> &str {
        let n = (self.prefab_len as usize).min(NET_PREFAB_MAX);
        std::str::from_utf8(&self.prefab[..n]).unwrap_or("")
    }

    pub fn set_hot(&mut self, paths: &[FieldPathId]) {
        let n = paths.len().min(NET_FIELDS_MAX);
        self.hot_count = n as u8;
        self.hot = [0; NET_FIELDS_MAX];
        for (i, p) in paths.iter().take(n).enumerate() {
            self.hot[i] = p.as_u8();
        }
    }

    pub fn set_cold(&mut self, paths: &[FieldPathId]) {
        let n = paths.len().min(NET_FIELDS_MAX);
        self.cold_count = n as u8;
        self.cold = [0; NET_FIELDS_MAX];
        for (i, p) in paths.iter().take(n).enumerate() {
            self.cold[i] = p.as_u8();
        }
    }

    pub fn hot_paths(&self) -> impl Iterator<Item = FieldPathId> + '_ {
        self.hot[..self.hot_count as usize]
            .iter()
            .filter_map(|id| FieldPathId::from_u8(*id))
    }

    pub fn cold_paths(&self) -> impl Iterator<Item = FieldPathId> + '_ {
        self.cold[..self.cold_count as usize]
            .iter()
            .filter_map(|id| FieldPathId::from_u8(*id))
    }

    /// Component keys allowed on the client for this entity.
    pub fn client_component_keys(&self) -> Vec<&'static str> {
        let mut keys = vec!["TransformComponent", "NetworkEntity"];
        for p in self.cold_paths() {
            if let Some(k) = p.component_key() {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        for p in self.hot_paths() {
            if let Some(k) = p.component_key() {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        keys
    }
}
